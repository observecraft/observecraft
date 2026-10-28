//! Service level indicator math, free of storage: good and total amounts per bucket for metric,
//! monitor and time-slice SLOs, corrections excluded, and from them the SLI, error budget, burn rate
//! and state. Times are epoch milliseconds; spans are half-open `[from, to)`.

use std::collections::BTreeMap;

/// A time span `[from, to)`.
pub(crate) type Span = (i64, i64);

/// Good and total amounts: events for metric SLOs, milliseconds for monitor and time-slice SLOs.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub(crate) struct Measure {
    pub good: f64,
    pub total: f64,
}

impl Measure {
    fn add(&mut self, other: Measure) {
        self.good += other.good;
        self.total += other.total;
    }
}

/// Sorted, non-overlapping spans covering the same time as `spans`.
pub(crate) fn merge(mut spans: Vec<Span>) -> Vec<Span> {
    spans.retain(|s| s.1 > s.0);
    spans.sort_unstable();
    let mut out: Vec<Span> = Vec::with_capacity(spans.len());
    for s in spans {
        match out.last_mut() {
            Some(last) if s.0 <= last.1 => last.1 = last.1.max(s.1),
            _ => out.push(s),
        }
    }
    out
}

/// Milliseconds of `[from, to)` covered by `merged` (the output of [`merge`]).
pub(crate) fn covered(from: i64, to: i64, merged: &[Span]) -> i64 {
    merged.iter().map(|s| (s.1.min(to) - s.0.max(from)).max(0)).sum()
}

/// Share of the bucket `[t, t + step)` that counts: inside `[from, to)` and outside the corrections.
fn counted_share(t: i64, step: i64, from: i64, to: i64, corrections: &[Span]) -> f64 {
    let (start, end) = (t.max(from), (t + step).min(to));
    if end <= start || step <= 0 {
        return 0.0;
    }
    (end - start - covered(start, end, corrections)) as f64 / step as f64
}

/// Values of every series added up per timestamp.
pub(crate) fn sum_series<'a>(series: impl IntoIterator<Item = &'a [(i64, f64)]>) -> BTreeMap<i64, f64> {
    let mut out = BTreeMap::new();
    for points in series {
        for (t, v) in points {
            if v.is_finite() {
                *out.entry(*t).or_insert(0.0) += v;
            }
        }
    }
    out
}

/// Metric SLO: good and total events per bucket. Values are per-second rates over buckets of
/// `step` ms (what the metric engine returns), so a bucket holds `rate × step / 1000` events; a
/// bucket partly outside the window or under a correction counts for its remaining share.
pub(crate) fn metric_measures(good: &BTreeMap<i64, f64>, total: &BTreeMap<i64, f64>, step: i64, from: i64, to: i64, corrections: &[Span]) -> Vec<(i64, Measure)> {
    let per_bucket = step as f64 / 1000.0;
    total
        .iter()
        .filter_map(|(t, all)| {
            let share = counted_share(*t, step, from, to, corrections);
            if share <= 0.0 {
                return None;
            }
            let all = all.max(0.0) * per_bucket * share;
            // More good than total events (separate queries, late points) is capped.
            let ok = (good.get(t).copied().unwrap_or(0.0).max(0.0) * per_bucket * share).min(all);
            Some((*t, Measure { good: ok, total: all }))
        })
        .collect()
}

/// Comparison a time slice's value must pass to be good.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Cmp {
    Lt,
    Le,
    Gt,
    Ge,
}

impl Cmp {
    pub(crate) fn holds(self, value: f64, threshold: f64) -> bool {
        match self {
            Cmp::Lt => value < threshold,
            Cmp::Le => value <= threshold,
            Cmp::Gt => value > threshold,
            Cmp::Ge => value >= threshold,
        }
    }
}

/// Time-slice SLO: each slice of `step` ms with a value is good time when the value passes the
/// condition; slices without data don't count.
pub(crate) fn slice_measures(points: &BTreeMap<i64, f64>, step: i64, cmp: Cmp, threshold: f64, from: i64, to: i64, corrections: &[Span]) -> Vec<(i64, Measure)> {
    points
        .iter()
        .filter_map(|(t, v)| {
            let ms = counted_share(*t, step, from, to, corrections) * step as f64;
            (ms > 0.0).then(|| (*t, Measure { good: if cmp.holds(*v, threshold) { ms } else { 0.0 }, total: ms }))
        })
        .collect()
}

/// A monitor group's state transition, as stored in `monitor_events`.
#[derive(Debug, Clone)]
pub(crate) struct Transition {
    pub group: String,
    pub at: i64,
    pub from_alert: bool,
    pub to_alert: bool,
}

/// When a monitor was in alert within `[from, to)`: per group, from its transitions (sorted by time)
/// and, for groups without transitions in the range, whether `alerting_now` lists it.
pub(crate) fn alert_spans(transitions: &[Transition], alerting_now: &[String], from: i64, to: i64) -> Vec<Span> {
    let mut groups: BTreeMap<&str, Vec<&Transition>> = BTreeMap::new();
    for t in transitions {
        groups.entry(t.group.as_str()).or_default().push(t);
    }
    let mut out = Vec::new();
    for list in groups.values() {
        // The state before the first transition is what it left.
        let mut alerting = list.first().map(|t| t.from_alert).unwrap_or(false);
        let mut since = from;
        for t in list {
            if alerting && t.at > since {
                out.push((since, t.at.min(to)));
            }
            alerting = t.to_alert;
            since = t.at.max(from);
        }
        if alerting && to > since {
            out.push((since, to));
        }
    }
    for group in alerting_now {
        if !groups.contains_key(group.as_str()) {
            out.push((from, to));
        }
    }
    merge(out)
}

/// Monitor SLO: up and observed time per bucket of `bucket` ms from `from`, where down is when any
/// of the monitors alerted (`down`, merged), time outside `observed` (before the monitors existed)
/// and corrections excluded.
pub(crate) fn uptime_measures(down: &[Span], observed: Span, corrections: &[Span], from: i64, to: i64, bucket: i64) -> Vec<(i64, Measure)> {
    let mut out = Vec::new();
    if bucket <= 0 {
        return out;
    }
    let excluded = merge(corrections.iter().copied().chain([(i64::MIN, observed.0), (observed.1, i64::MAX)]).collect());
    let mut t = from;
    while t < to {
        let end = (t + bucket).min(to);
        let total = end - t - covered(t, end, &excluded);
        if total > 0 {
            // Down time that is neither excluded nor outside the bucket.
            let down_here: i64 = down.iter().map(|d| (d.0.max(t), d.1.min(end))).filter(|d| d.1 > d.0).map(|d| d.1 - d.0 - covered(d.0, d.1, &excluded)).sum();
            out.push((t, Measure { good: (total - down_here) as f64, total: total as f64 }));
        }
        t = end;
    }
    out
}

pub(crate) fn total(measures: &[(i64, Measure)]) -> Measure {
    let mut m = Measure::default();
    for (_, x) in measures {
        m.add(*x);
    }
    m
}

/// Measures from `since` on.
pub(crate) fn total_since(measures: &[(i64, Measure)], since: i64) -> Measure {
    let mut m = Measure::default();
    for (_, x) in measures.iter().filter(|(t, _)| *t >= since) {
        m.add(*x);
    }
    m
}

/// Measures added up into buckets of `bucket` ms aligned on `from`.
pub(crate) fn rebucket(measures: &[(i64, Measure)], from: i64, bucket: i64) -> Vec<(i64, Measure)> {
    let mut out: BTreeMap<i64, Measure> = BTreeMap::new();
    for (t, m) in measures {
        let start = from + (t - from).max(0) / bucket.max(1) * bucket.max(1);
        out.entry(start).or_default().add(*m);
    }
    out.into_iter().collect()
}

/// SLI in percent; none without data.
pub(crate) fn sli(m: Measure) -> Option<f64> {
    (m.total > 0.0).then(|| (m.good / m.total * 100.0).clamp(0.0, 100.0))
}

/// Error budget for a target in percent (0 < target < 100).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Budget {
    /// Bad amount the target allows over the window.
    pub allowed: f64,
    pub used: f64,
    /// Share of the allowed amount left, in percent (negative once overspent).
    pub remaining_pct: f64,
}

pub(crate) fn budget(m: Measure, target: f64) -> Option<Budget> {
    if m.total <= 0.0 {
        return None;
    }
    let allowed = m.total * (1.0 - target / 100.0);
    let used = (m.total - m.good).max(0.0);
    let remaining_pct = if allowed > 0.0 { (allowed - used) / allowed * 100.0 } else { 0.0 };
    Some(Budget { allowed, used, remaining_pct })
}

/// How fast the budget burns: the bad share over the bad share the target allows (1 = exactly
/// spends the budget over the window).
pub(crate) fn burn_rate(m: Measure, target: f64) -> Option<f64> {
    let allowed_share = 1.0 - target / 100.0;
    (m.total > 0.0 && allowed_share > 0.0).then(|| ((m.total - m.good).max(0.0) / m.total) / allowed_share)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum State {
    Ok,
    Warning,
    Breached,
    NoData,
}

pub(crate) fn state(sli: Option<f64>, target: f64, warning: Option<f64>) -> State {
    match sli {
        None => State::NoData,
        Some(s) if s < target => State::Breached,
        Some(s) if warning.is_some_and(|w| s < w) => State::Warning,
        Some(_) => State::Ok,
    }
}

/// One point of the SLI and error budget charts.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct HistoryPoint {
    pub t: i64,
    /// SLI of this bucket alone.
    pub bucket_sli: Option<f64>,
    /// SLI from the window's start to the end of this bucket.
    pub sli: Option<f64>,
    /// Error budget left at the end of this bucket, as a share of the whole window's budget: the
    /// budget is what the target allows over the full window's total.
    pub budget_remaining: Option<f64>,
}

pub(crate) fn history(buckets: &[(i64, Measure)], target: f64) -> Vec<HistoryPoint> {
    let window = total(buckets);
    let allowed = window.total * (1.0 - target / 100.0);
    let mut sofar = Measure::default();
    buckets
        .iter()
        .map(|(t, m)| {
            sofar.add(*m);
            let used = (sofar.total - sofar.good).max(0.0);
            HistoryPoint { t: *t, bucket_sli: sli(*m), sli: sli(sofar), budget_remaining: (allowed > 0.0).then(|| (allowed - used) / allowed * 100.0) }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIN: i64 = 60_000;
    const HOUR: i64 = 60 * MIN;

    fn series(points: &[(i64, f64)]) -> BTreeMap<i64, f64> {
        points.iter().copied().collect()
    }

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    #[test]
    fn merge_joins_overlapping_and_touching_spans() {
        assert_eq!(merge(vec![(5, 10), (0, 3), (3, 4), (8, 12), (20, 20)]), vec![(0, 4), (5, 12)]);
        assert_eq!(covered(2, 9, &[(0, 4), (5, 12)]), 2 + 4);
    }

    #[test]
    fn metric_sli_is_good_over_total_events() {
        // Rates per second over 1-minute buckets: 10/s total, 9.9/s good.
        let total = series(&[(0, 10.0), (MIN, 10.0)]);
        let good = series(&[(0, 9.9), (MIN, 10.0)]);
        let m = super::total(&metric_measures(&good, &total, MIN, 0, 2 * MIN, &[]));
        assert!(close(m.total, 1200.0));
        assert!(close(m.good, 594.0 + 600.0));
        assert!(close(sli(m).unwrap(), 99.5));
    }

    #[test]
    fn metric_buckets_without_good_events_are_all_bad_and_good_is_capped() {
        let total = series(&[(0, 1.0), (MIN, 1.0)]);
        let good = series(&[(MIN, 2.0)]);
        let m = super::total(&metric_measures(&good, &total, MIN, 0, 2 * MIN, &[]));
        assert!(close(m.good, 60.0));
        assert!(close(m.total, 120.0));
    }

    #[test]
    fn corrections_are_excluded_from_metric_slis() {
        // The second minute was all errors, but it was under maintenance.
        let total = series(&[(0, 1.0), (MIN, 1.0), (2 * MIN, 1.0)]);
        let good = series(&[(0, 1.0), (MIN, 0.0), (2 * MIN, 1.0)]);
        let corrections = merge(vec![(MIN, 2 * MIN)]);
        let m = super::total(&metric_measures(&good, &total, MIN, 0, 3 * MIN, &corrections));
        assert_eq!(sli(m), Some(100.0));
        assert!(close(m.total, 120.0));
        // Half a bucket corrected counts half of it.
        let m = super::total(&metric_measures(&good, &total, MIN, 0, 3 * MIN, &merge(vec![(MIN, MIN + 30_000)])));
        assert!(close(m.total, 150.0));
        assert!(close(m.good, 120.0));
    }

    #[test]
    fn buckets_partly_outside_the_window_count_their_share() {
        let total = series(&[(0, 1.0)]);
        let m = super::total(&metric_measures(&total, &total, MIN, 30_000, HOUR, &[]));
        assert!(close(m.total, 30.0));
    }

    #[test]
    fn monitor_uptime_follows_alert_transitions() {
        let tr = |group: &str, at: i64, from_alert: bool, to_alert: bool| Transition { group: group.into(), at, from_alert, to_alert };
        // host:a alerts from 10 to 20 minutes, host:b from 15 to 30: down 10–30.
        let events = vec![tr("host:a", 10 * MIN, false, true), tr("host:b", 15 * MIN, false, true), tr("host:a", 20 * MIN, true, false), tr("host:b", 30 * MIN, true, false)];
        let down = alert_spans(&events, &[], 0, HOUR);
        assert_eq!(down, vec![(10 * MIN, 30 * MIN)]);
        let m = super::total(&uptime_measures(&down, (0, HOUR), &[], 0, HOUR, 15 * MIN));
        assert!(close(m.total, HOUR as f64));
        assert!(close(m.good, 40.0 * MIN as f64));
        assert!(close(sli(m).unwrap(), 200.0 / 3.0));
    }

    #[test]
    fn a_group_alerting_since_before_the_range_is_down_from_the_start() {
        let events = vec![Transition { group: "*".into(), at: 20 * MIN, from_alert: true, to_alert: false }];
        assert_eq!(alert_spans(&events, &[], 0, HOUR), vec![(0, 20 * MIN)]);
        // No transitions in range: the current state holds over it.
        assert_eq!(alert_spans(&[], &["*".into()], 0, HOUR), vec![(0, HOUR)]);
        assert_eq!(alert_spans(&[], &[], 0, HOUR), vec![]);
        // Still alerting at the end.
        let events = vec![Transition { group: "*".into(), at: 50 * MIN, from_alert: false, to_alert: true }];
        assert_eq!(alert_spans(&events, &[], 0, HOUR), vec![(50 * MIN, HOUR)]);
    }

    #[test]
    fn uptime_excludes_corrections_and_time_before_the_monitor_existed() {
        let down = vec![(10 * MIN, 20 * MIN)];
        // Observed from minute 5; maintenance 10–15 (its down time doesn't count).
        let m = super::total(&uptime_measures(&down, (5 * MIN, HOUR), &merge(vec![(10 * MIN, 15 * MIN)]), 0, HOUR, 10 * MIN));
        assert!(close(m.total, 50.0 * MIN as f64));
        assert!(close(m.good, 45.0 * MIN as f64));
        // Nothing observed: no data.
        let m = super::total(&uptime_measures(&down, (HOUR, HOUR), &[], 0, HOUR, 10 * MIN));
        assert_eq!(sli(m), None);
    }

    #[test]
    fn time_slices_are_good_when_their_value_passes() {
        // p95 latency per 5-minute slice must stay under 0.5 s; one slice has no data.
        let points = series(&[(0, 0.2), (5 * MIN, 0.7), (10 * MIN, 0.4)]);
        let m = super::total(&slice_measures(&points, 5 * MIN, Cmp::Lt, 0.5, 0, 20 * MIN, &[]));
        assert!(close(m.total, 15.0 * MIN as f64));
        assert!(close(m.good, 10.0 * MIN as f64));
        let m = super::total(&slice_measures(&points, 5 * MIN, Cmp::Lt, 0.5, 0, 20 * MIN, &merge(vec![(5 * MIN, 10 * MIN)])));
        assert_eq!(sli(m), Some(100.0));
        assert!(Cmp::Ge.holds(1.0, 1.0) && !Cmp::Gt.holds(1.0, 1.0) && Cmp::Le.holds(1.0, 1.0));
    }

    #[test]
    fn error_budget_counts_allowed_and_used_bad_amounts() {
        // 99.9% of 100,000 events allows 100 bad ones; 40 used leaves 60%.
        let b = budget(Measure { good: 99_960.0, total: 100_000.0 }, 99.9).unwrap();
        assert!(close(b.allowed, 100.0));
        assert!(close(b.used, 40.0));
        assert!(close(b.remaining_pct, 60.0));
        // Overspent budgets go negative.
        let b = budget(Measure { good: 99_700.0, total: 100_000.0 }, 99.9).unwrap();
        assert!(close(b.remaining_pct, -200.0));
        assert_eq!(budget(Measure::default(), 99.9), None);
    }

    #[test]
    fn burn_rate_is_the_bad_share_over_the_allowed_share() {
        // 1% errors against a 99.9% target burns 10x.
        assert!(close(burn_rate(Measure { good: 99.0, total: 100.0 }, 99.9).unwrap(), 10.0));
        assert!(close(burn_rate(Measure { good: 100.0, total: 100.0 }, 99.0).unwrap(), 0.0));
        assert_eq!(burn_rate(Measure::default(), 99.0), None);
        let m = vec![(0, Measure { good: 0.0, total: 10.0 }), (HOUR, Measure { good: 10.0, total: 10.0 })];
        assert_eq!(total_since(&m, HOUR), Measure { good: 10.0, total: 10.0 });
    }

    #[test]
    fn state_compares_the_sli_with_target_and_warning() {
        assert_eq!(state(Some(99.0), 99.5, Some(99.8)), State::Breached);
        assert_eq!(state(Some(99.6), 99.5, Some(99.8)), State::Warning);
        assert_eq!(state(Some(99.9), 99.5, Some(99.8)), State::Ok);
        assert_eq!(state(Some(99.6), 99.5, None), State::Ok);
        assert_eq!(state(Some(99.5), 99.5, None), State::Ok);
        assert_eq!(state(None, 99.5, Some(99.8)), State::NoData);
    }

    #[test]
    fn history_tracks_the_running_sli_and_the_budget_burn_down() {
        let buckets = vec![(0, Measure { good: 1000.0, total: 1000.0 }), (HOUR, Measure { good: 990.0, total: 1000.0 }), (2 * HOUR, Measure::default())];
        let h = history(&buckets, 99.0);
        // The window allows 20 bad events; 10 were spent in the second hour.
        assert_eq!(h[0].budget_remaining, Some(100.0));
        assert_eq!(h[0].bucket_sli, Some(100.0));
        assert_eq!(h[1].bucket_sli, Some(99.0));
        assert!(close(h[1].sli.unwrap(), 99.5));
        assert!(close(h[1].budget_remaining.unwrap(), 50.0));
        assert_eq!(h[2].bucket_sli, None);
        assert!(close(h[2].budget_remaining.unwrap(), 50.0));
        let r = rebucket(&[(0, Measure { good: 1.0, total: 1.0 }), (MIN, Measure { good: 0.0, total: 1.0 }), (HOUR, Measure { good: 1.0, total: 1.0 })], 0, HOUR);
        assert_eq!(r, vec![(0, Measure { good: 1.0, total: 2.0 }), (HOUR, Measure { good: 1.0, total: 1.0 })]);
    }

    #[test]
    fn sum_series_adds_groups_per_timestamp() {
        let a = [(0, 1.0), (MIN, 2.0)];
        let b = [(0, 3.0), (MIN, f64::NAN)];
        assert_eq!(sum_series([&a[..], &b[..]]), series(&[(0, 4.0), (MIN, 2.0)]));
    }
}
