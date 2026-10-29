//! Monitor evaluation, independent of storage: definitions, threshold/change classification,
//! backtesting over a series, downtime windows, message templates and recipient extraction.
//!
//! A monitor evaluates every minute. Per `by` group, the query's points inside the evaluation window
//! are reduced with the time aggregator (avg/sum/min/max/last); a change monitor compares that value
//! with the same window shifted back by `changeMinutes`. The value is then compared with the alert and
//! warning thresholds; recovery thresholds add hysteresis (a group stays in Alert until the value no
//! longer crosses the recovery threshold). A monitor's overall status is its worst group.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use super::metrics::{Group, TimeAggregator};

#[derive(Serialize, Deserialize, ToSchema, Clone, Copy, PartialEq, Eq, Debug, Hash)]
#[serde(rename_all = "lowercase")]
pub enum MonitorStatus {
    Ok,
    Warn,
    Alert,
    #[serde(rename = "nodata")]
    NoData,
}

impl MonitorStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            MonitorStatus::Ok => "ok",
            MonitorStatus::Warn => "warn",
            MonitorStatus::Alert => "alert",
            MonitorStatus::NoData => "nodata",
        }
    }
    pub fn parse(s: &str) -> Self {
        match s {
            "ok" => MonitorStatus::Ok,
            "warn" => MonitorStatus::Warn,
            "alert" => MonitorStatus::Alert,
            _ => MonitorStatus::NoData,
        }
    }
    /// Worst-first ordering used for a monitor's overall status: Alert > Warn > No Data > OK.
    pub fn severity(self) -> u8 {
        match self {
            MonitorStatus::Alert => 3,
            MonitorStatus::Warn => 2,
            MonitorStatus::NoData => 1,
            MonitorStatus::Ok => 0,
        }
    }
}

pub fn worst(statuses: impl IntoIterator<Item = MonitorStatus>) -> Option<MonitorStatus> {
    statuses.into_iter().max_by_key(|s| s.severity())
}

#[derive(Serialize, Deserialize, ToSchema, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum Detection {
    Threshold,
    Change,
}

#[derive(Serialize, Deserialize, ToSchema, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum Comparator {
    Above,
    AboveOrEqual,
    Below,
    BelowOrEqual,
}

impl Comparator {
    pub fn crosses(self, value: f64, threshold: f64) -> bool {
        match self {
            Comparator::Above => value > threshold,
            Comparator::AboveOrEqual => value >= threshold,
            Comparator::Below => value < threshold,
            Comparator::BelowOrEqual => value <= threshold,
        }
    }
    pub fn symbol(self) -> &'static str {
        match self {
            Comparator::Above => ">",
            Comparator::AboveOrEqual => ">=",
            Comparator::Below => "<",
            Comparator::BelowOrEqual => "<=",
        }
    }
    fn upward(self) -> bool {
        matches!(self, Comparator::Above | Comparator::AboveOrEqual)
    }
}

#[derive(Serialize, Deserialize, ToSchema, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum ChangeMode {
    /// Absolute difference.
    Change,
    /// Difference as a percentage of the earlier value.
    PctChange,
}

#[derive(Serialize, Deserialize, ToSchema, Clone, PartialEq, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Thresholds {
    pub critical: f64,
    pub warning: Option<f64>,
    /// While in Alert, the group recovers only once the value no longer crosses this.
    pub critical_recovery: Option<f64>,
    pub warning_recovery: Option<f64>,
}

fn default_no_data_minutes() -> i64 {
    10
}

/// Evaluation settings stored with a monitor (the `definition` column).
#[derive(Serialize, Deserialize, ToSchema, Clone, PartialEq, Debug)]
#[serde(rename_all = "camelCase")]
pub struct MonitorOptions {
    pub detection: Detection,
    pub time_aggregation: TimeAggregator,
    /// Evaluation window, 1–1440 minutes.
    pub window_minutes: i64,
    /// Change detection only.
    pub change_mode: Option<ChangeMode>,
    /// Change detection only: compare with the value this many minutes earlier.
    pub change_minutes: Option<i64>,
    pub comparator: Comparator,
    pub thresholds: Thresholds,
    /// Go to No Data when a group stops reporting for `noDataMinutes`.
    #[serde(default)]
    pub notify_no_data: bool,
    #[serde(default = "default_no_data_minutes")]
    pub no_data_minutes: i64,
}

impl MonitorOptions {
    pub fn validate(&self) -> Result<(), String> {
        if !(1..=1440).contains(&self.window_minutes) {
            return Err("the evaluation window must be between 1 minute and 1 day".into());
        }
        if self.detection == Detection::Change && !self.change_minutes.is_some_and(|m| (1..=10_080).contains(&m)) {
            return Err("change monitors need a comparison period between 1 minute and 1 week".into());
        }
        if !(1..=1440).contains(&self.no_data_minutes) {
            return Err("the no-data timeframe must be between 1 minute and 1 day".into());
        }
        let t = &self.thresholds;
        let finite = [Some(t.critical), t.warning, t.critical_recovery, t.warning_recovery];
        if finite.iter().flatten().any(|v| !v.is_finite()) {
            return Err("thresholds must be numbers".into());
        }
        // Thresholds must be ordered in the alerting direction: warning before alert, recovery before its threshold.
        let up = self.comparator.upward();
        let before = |a: f64, b: f64| if up { a < b } else { a > b };
        if let Some(w) = t.warning {
            if !before(w, t.critical) {
                return Err(format!("the warning threshold must be {} the alert threshold", if up { "below" } else { "above" }));
            }
        }
        if let Some(r) = t.critical_recovery {
            if !before(r, t.critical) {
                return Err(format!("the alert recovery threshold must be {} the alert threshold", if up { "below" } else { "above" }));
            }
        }
        if let (Some(r), Some(w)) = (t.warning_recovery, t.warning) {
            if !before(r, w) {
                return Err(format!("the warning recovery threshold must be {} the warning threshold", if up { "below" } else { "above" }));
            }
        }
        if t.warning_recovery.is_some() && t.warning.is_none() {
            return Err("a warning recovery threshold needs a warning threshold".into());
        }
        Ok(())
    }

    /// Longest look-back an evaluation needs, in milliseconds.
    pub fn lookback_ms(&self) -> i64 {
        (self.window_minutes + self.change_minutes.filter(|_| self.detection == Detection::Change).unwrap_or(0)) * 60_000
    }

    /// Datadog-style full query: `avg(last_5m):avg:system.load.1{*} by {host} > 2`.
    pub fn full_query(&self, metric_query: &str) -> String {
        let agg = match self.time_aggregation {
            TimeAggregator::Avg => "avg",
            TimeAggregator::Sum => "sum",
            TimeAggregator::Min => "min",
            TimeAggregator::Max => "max",
            TimeAggregator::Last => "last",
        };
        let reducer = format!("{agg}(last_{})", minutes_label(self.window_minutes));
        let head = match (self.detection, self.change_mode) {
            (Detection::Change, mode) => format!(
                "{}({reducer},last_{})",
                if mode == Some(ChangeMode::PctChange) { "pct_change" } else { "change" },
                minutes_label(self.change_minutes.unwrap_or(60))
            ),
            _ => reducer,
        };
        format!("{head}:{} {} {}", metric_query.trim(), self.comparator.symbol(), fmt_num(self.thresholds.critical))
    }

    /// Next status for one group given its evaluated value (`None` = no data in the window).
    pub fn classify(&self, value: Option<f64>, prev: MonitorStatus) -> MonitorStatus {
        let Some(v) = value else {
            return if self.notify_no_data { MonitorStatus::NoData } else { prev };
        };
        let c = self.comparator;
        let t = &self.thresholds;
        if c.crosses(v, t.critical) {
            return MonitorStatus::Alert;
        }
        if prev == MonitorStatus::Alert {
            if let Some(r) = t.critical_recovery {
                if c.crosses(v, r) {
                    return MonitorStatus::Alert;
                }
            }
        }
        if let Some(w) = t.warning {
            if c.crosses(v, w) {
                return MonitorStatus::Warn;
            }
            if prev == MonitorStatus::Warn {
                if let Some(r) = t.warning_recovery {
                    if c.crosses(v, r) {
                        return MonitorStatus::Warn;
                    }
                }
            }
        }
        MonitorStatus::Ok
    }

    /// The group's value at `at`: points within the window ending at `at` reduced with the time
    /// aggregator; for change monitors, the difference with the window ending `changeMinutes` earlier.
    pub fn value_at(&self, points: &[(i64, f64)], at: i64) -> Option<f64> {
        let w = self.window_minutes * 60_000;
        let now = reduce_window(points, at - w, at, self.time_aggregation)?;
        if self.detection != Detection::Change {
            return Some(now);
        }
        let shift = self.change_minutes.unwrap_or(60) * 60_000;
        let before = reduce_window(points, at - shift - w, at - shift, self.time_aggregation)?;
        Some(match self.change_mode.unwrap_or(ChangeMode::Change) {
            ChangeMode::Change => now - before,
            ChangeMode::PctChange if before == 0.0 => return None,
            ChangeMode::PctChange => (now - before) / before.abs() * 100.0,
        })
    }
}

fn reduce_window(points: &[(i64, f64)], from: i64, to: i64, agg: TimeAggregator) -> Option<f64> {
    let start = points.partition_point(|p| p.0 <= from);
    let end = points.partition_point(|p| p.0 <= to);
    let slice = &points[start..end];
    if slice.is_empty() {
        return None;
    }
    let vs = slice.iter().map(|p| p.1);
    Some(match agg {
        TimeAggregator::Last => slice[slice.len() - 1].1,
        TimeAggregator::Sum => vs.sum(),
        TimeAggregator::Min => vs.fold(f64::INFINITY, f64::min),
        TimeAggregator::Max => vs.fold(f64::NEG_INFINITY, f64::max),
        TimeAggregator::Avg => vs.sum::<f64>() / slice.len() as f64,
    })
}

pub fn minutes_label(m: i64) -> String {
    match m {
        m if m % 1440 == 0 => format!("{}d", m / 1440),
        m if m % 60 == 0 => format!("{}h", m / 60),
        m => format!("{m}m"),
    }
}

pub fn fmt_num(v: f64) -> String {
    if v.fract() == 0.0 && v.abs() < 1e15 {
        format!("{}", v as i64)
    } else {
        let s = format!("{v:.4}");
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    }
}

/// `host:a,service:b`, or `*` for an ungrouped query.
pub fn group_key(tags: &BTreeMap<String, String>) -> String {
    if tags.is_empty() {
        "*".into()
    } else {
        tags.iter().map(|(k, v)| format!("{k}:{v}")).collect::<Vec<_>>().join(",")
    }
}

pub fn parse_group_key(key: &str) -> BTreeMap<String, String> {
    if key == "*" {
        return BTreeMap::new();
    }
    key.split(',').filter_map(|p| p.split_once(':')).map(|(k, v)| (k.to_string(), v.to_string())).collect()
}

// ================================================================ backtest

pub struct Transition {
    pub at: i64,
    pub group: String,
    pub from: MonitorStatus,
    pub to: MonitorStatus,
    pub value: Option<f64>,
}

pub struct Backtest {
    pub transitions: Vec<Transition>,
    /// Per group: (key, evaluated value at each evaluation time).
    pub evaluated: Vec<(String, Vec<(i64, Option<f64>)>)>,
    /// Status of each group after the last evaluation.
    pub last: Vec<(String, MonitorStatus, Option<f64>)>,
}

/// Replays the monitor over `[from, to]`, evaluating every `step_ms` like the live evaluator does.
/// Every group starts OK.
pub fn backtest(opts: &MonitorOptions, groups: &[Group], from: i64, to: i64, step_ms: i64) -> Backtest {
    let step = step_ms.max(10_000);
    let mut transitions = Vec::new();
    let mut evaluated = Vec::new();
    let mut last = Vec::new();
    // First evaluation once a full window is available (or right away for short previews).
    let first = (from + opts.window_minutes * 60_000).min(to);
    for g in groups {
        let key = group_key(&g.tags);
        let mut status = MonitorStatus::Ok;
        let mut values = Vec::new();
        let mut value = None;
        let mut at = first - (first - from) % step;
        // A group doesn't exist before its first point: no evaluations (and no No Data) before it.
        let born = g.points.first().map_or(i64::MAX, |p| p.0);
        while at <= to {
            if at < born {
                at += step;
                continue;
            }
            value = opts.value_at(&g.points, at);
            let next = opts.classify(value, status);
            if next != status {
                transitions.push(Transition { at, group: key.clone(), from: status, to: next, value });
                status = next;
            }
            values.push((at, value));
            at += step;
        }
        evaluated.push((key.clone(), values));
        last.push((key, status, value));
    }
    transitions.sort_by_key(|t| t.at);
    Backtest { transitions, evaluated, last }
}

// ================================================================ downtimes

#[derive(Clone, Debug)]
pub struct DowntimeWindow {
    pub start: i64,
    pub end: Option<i64>,
    /// Period of a recurring downtime in ms.
    pub period: Option<i64>,
    pub until: Option<i64>,
    pub canceled: Option<i64>,
    /// A daily recurring downtime only on these days of the week (0 = Monday … 6 = Sunday, UTC);
    /// empty = every occurrence.
    pub weekdays: Vec<i16>,
}

pub const DAY_MS: i64 = 86_400_000;

impl DowntimeWindow {
    /// The occurrence active at `now`, as (start, end): `None` when the downtime isn't active.
    pub fn active_at(&self, now: i64) -> Option<(i64, Option<i64>)> {
        if self.canceled.is_some_and(|c| c <= now) || now < self.start {
            return None;
        }
        let (Some(end), Some(period)) = (self.end, self.period) else {
            return match self.end {
                Some(end) if now >= end => None,
                end => Some((self.start, end)),
            };
        };
        let k = (now - self.start) / period;
        let (s, e) = (self.start + k * period, end + k * period);
        if self.until.is_some_and(|u| s > u) || now >= e || !self.on_weekday(s) {
            return None;
        }
        Some((s, Some(e)))
    }

    fn on_weekday(&self, at: i64) -> bool {
        // 1970-01-01 was a Thursday.
        self.weekdays.is_empty() || self.weekdays.contains(&(((at.div_euclid(DAY_MS) + 3) % 7) as i16))
    }

    /// Whether the downtime has ended for good (one-off past its end, recurring past its last occurrence).
    pub fn finished_at(&self, now: i64) -> bool {
        if self.canceled.is_some_and(|c| c <= now) {
            return true;
        }
        match (self.end, self.period) {
            (Some(end), None) => now >= end,
            (Some(end), Some(period)) => self.until.is_some_and(|u| {
                let k = (u - self.start).div_euclid(period);
                now >= end + k * period
            }),
            _ => false,
        }
    }
}

/// Whether a downtime scope (`[]`/`["*"]` = everything, else every `key:value` must match) covers a group.
pub fn scope_covers(scope: &[String], group: &BTreeMap<String, String>) -> bool {
    scope.iter().filter(|s| s.as_str() != "*").all(|s| match s.split_once(':') {
        Some((k, v)) => group.get(k).is_some_and(|gv| gv == v),
        None => false,
    })
}

// ================================================================ messages

/// Template variables available in a monitor message.
pub struct MessageContext<'a> {
    pub status: MonitorStatus,
    pub previous: MonitorStatus,
    pub value: Option<f64>,
    pub opts: &'a MonitorOptions,
    pub monitor_name: &'a str,
    pub group: &'a BTreeMap<String, String>,
}

/// Renders `{{#is_alert}}…{{/is_alert}}` / `{{^is_alert}}…{{/is_alert}}` blocks (is_alert, is_warning,
/// is_no_data, is_recovery, is_alert_recovery, is_warning_recovery) and `{{value}}`, `{{threshold}}`,
/// `{{warn_threshold}}`, `{{monitor.name}}` and `{{<tag>.name}}` variables. Unknown variables are kept.
pub fn render_message(template: &str, ctx: &MessageContext) -> String {
    let recovery = ctx.status == MonitorStatus::Ok && ctx.previous != MonitorStatus::Ok;
    let flag = |name: &str| -> Option<bool> {
        Some(match name {
            "is_alert" => ctx.status == MonitorStatus::Alert,
            "is_warning" => ctx.status == MonitorStatus::Warn,
            "is_no_data" => ctx.status == MonitorStatus::NoData,
            "is_recovery" => recovery,
            "is_alert_recovery" => recovery && ctx.previous == MonitorStatus::Alert,
            "is_warning_recovery" => recovery && ctx.previous == MonitorStatus::Warn,
            _ => return None,
        })
    };
    let mut out = String::new();
    let mut rest = template;
    // Stack of "is this section shown" for nested blocks.
    let mut shown: Vec<bool> = Vec::new();
    let visible = |s: &Vec<bool>| s.iter().all(|v| *v);
    while let Some(open) = rest.find("{{") {
        if visible(&shown) {
            out.push_str(&rest[..open]);
        }
        let Some(close) = rest[open..].find("}}").map(|i| open + i) else {
            rest = &rest[open..];
            break;
        };
        let tag = rest[open + 2..close].trim();
        let raw = &rest[open..close + 2];
        rest = &rest[close + 2..];
        if let Some(name) = tag.strip_prefix('#').or_else(|| tag.strip_prefix('^')) {
            match flag(name.trim()) {
                Some(f) => shown.push(if tag.starts_with('#') { f } else { !f }),
                None if visible(&shown) => out.push_str(raw),
                None => {}
            }
            continue;
        }
        if let Some(name) = tag.strip_prefix('/') {
            if flag(name.trim()).is_some() {
                shown.pop();
            } else if visible(&shown) {
                out.push_str(raw);
            }
            continue;
        }
        if !visible(&shown) {
            continue;
        }
        let value = match tag {
            "value" => ctx.value.map(fmt_num),
            "threshold" => Some(fmt_num(ctx.opts.thresholds.critical)),
            "warn_threshold" => ctx.opts.thresholds.warning.map(fmt_num),
            "monitor.name" => Some(ctx.monitor_name.to_string()),
            t => t.strip_suffix(".name").and_then(|k| ctx.group.get(k).cloned()),
        };
        out.push_str(&value.unwrap_or_else(|| raw.to_string()));
    }
    if visible(&shown) {
        out.push_str(rest);
    }
    out
}

/// `@handles` in a message: emails, `@slack-…`, `@webhook-…`, `@pagerduty-…`, `@team-…`, user handles.
pub fn recipients(message: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let chars: Vec<char> = message.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let boundary = i == 0 || !(chars[i - 1].is_alphanumeric() || matches!(chars[i - 1], '.' | '_' | '-' | '+'));
        if chars[i] == '@' && boundary {
            let mut j = i + 1;
            while j < chars.len() && (chars[j].is_alphanumeric() || matches!(chars[j], '.' | '_' | '-' | '+' | '@')) {
                j += 1;
            }
            let handle: String = chars[i + 1..j].iter().collect::<String>().trim_end_matches(['.', '-', '@']).to_string();
            if !handle.is_empty() && !out.contains(&handle) {
                out.push(handle);
            }
            i = j;
        } else {
            i += 1;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts() -> MonitorOptions {
        MonitorOptions {
            detection: Detection::Threshold,
            time_aggregation: TimeAggregator::Avg,
            window_minutes: 5,
            change_mode: None,
            change_minutes: None,
            comparator: Comparator::Above,
            thresholds: Thresholds { critical: 90.0, warning: Some(80.0), critical_recovery: Some(85.0), warning_recovery: None },
            notify_no_data: true,
            no_data_minutes: 10,
        }
    }

    #[test]
    fn classify_uses_recovery_hysteresis() {
        let o = opts();
        assert_eq!(o.classify(Some(95.0), MonitorStatus::Ok), MonitorStatus::Alert);
        assert_eq!(o.classify(Some(87.0), MonitorStatus::Alert), MonitorStatus::Alert, "above the recovery threshold stays in alert");
        assert_eq!(o.classify(Some(87.0), MonitorStatus::Ok), MonitorStatus::Warn);
        assert_eq!(o.classify(Some(84.0), MonitorStatus::Alert), MonitorStatus::Warn);
        assert_eq!(o.classify(Some(10.0), MonitorStatus::Warn), MonitorStatus::Ok);
        assert_eq!(o.classify(None, MonitorStatus::Ok), MonitorStatus::NoData);
        let quiet = MonitorOptions { notify_no_data: false, ..o };
        assert_eq!(quiet.classify(None, MonitorStatus::Warn), MonitorStatus::Warn);
    }

    #[test]
    fn validate_checks_threshold_order() {
        assert!(opts().validate().is_ok());
        let mut o = opts();
        o.thresholds.warning = Some(95.0);
        assert!(o.validate().is_err());
        o = opts();
        o.comparator = Comparator::Below;
        assert!(o.validate().is_err(), "below: warning must be above the alert threshold");
    }

    #[test]
    fn change_value_compares_shifted_windows() {
        let mut o = opts();
        o.detection = Detection::Change;
        o.change_mode = Some(ChangeMode::PctChange);
        o.change_minutes = Some(60);
        o.window_minutes = 1;
        let points = vec![(0, 10.0), (60 * 60_000, 15.0)];
        assert_eq!(o.value_at(&points, 60 * 60_000), Some(50.0));
        assert_eq!(o.full_query("avg:x{*}"), "pct_change(avg(last_1m),last_1h):avg:x{*} > 90");
    }

    #[test]
    fn backtest_records_transitions() {
        let o = MonitorOptions { window_minutes: 1, ..opts() };
        let points: Vec<(i64, f64)> = (0..10).map(|i| (i * 60_000, if (3..6).contains(&i) { 95.0 } else { 50.0 })).collect();
        let g = Group { tags: BTreeMap::new(), points };
        let b = backtest(&o, &[g], 0, 9 * 60_000, 60_000);
        let seq: Vec<_> = b.transitions.iter().map(|t| (t.from, t.to)).collect();
        assert_eq!(seq, vec![(MonitorStatus::Ok, MonitorStatus::Alert), (MonitorStatus::Alert, MonitorStatus::Ok)]);
    }

    #[test]
    fn recurring_downtime_windows() {
        let d = DowntimeWindow { start: 0, end: Some(3_600_000), period: Some(DAY_MS), until: Some(2 * DAY_MS), canceled: None, weekdays: vec![] };
        assert_eq!(d.active_at(DAY_MS + 60_000), Some((DAY_MS, Some(DAY_MS + 3_600_000))));
        assert_eq!(d.active_at(DAY_MS + 7_200_000), None);
        assert!(d.active_at(3 * DAY_MS + 60_000).is_none(), "past recur_until");
        assert!(d.finished_at(2 * DAY_MS + 3_600_000));
        let forever = DowntimeWindow { start: 0, end: None, period: None, until: None, canceled: None, weekdays: vec![] };
        assert_eq!(forever.active_at(10 * DAY_MS), Some((0, None)));
    }

    #[test]
    fn daily_downtimes_limited_to_weekdays() {
        // 1970-01-01 was a Thursday (3); Monday to Friday, one hour from midnight UTC.
        let d = DowntimeWindow { start: 0, end: Some(3_600_000), period: Some(DAY_MS), until: None, canceled: None, weekdays: vec![0, 1, 2, 3, 4] };
        assert_eq!(d.active_at(60_000), Some((0, Some(3_600_000))), "Thursday");
        assert_eq!(d.active_at(DAY_MS + 60_000), Some((DAY_MS, Some(DAY_MS + 3_600_000))), "Friday");
        assert_eq!(d.active_at(2 * DAY_MS + 60_000), None, "Saturday");
        assert_eq!(d.active_at(3 * DAY_MS + 60_000), None, "Sunday");
        assert!(d.active_at(4 * DAY_MS + 60_000).is_some(), "Monday");
    }

    #[test]
    fn renders_conditional_blocks_and_recipients() {
        let o = opts();
        let group: BTreeMap<String, String> = [("host".to_string(), "web-1".to_string())].into();
        let ctx = MessageContext { status: MonitorStatus::Alert, previous: MonitorStatus::Ok, value: Some(97.5), opts: &o, monitor_name: "CPU", group: &group };
        let msg = "CPU {{value}}% on {{host.name}} {{#is_alert}}@pagerduty-checkout{{/is_alert}}{{#is_recovery}}@slack-ops{{/is_recovery}} @oncall@example.com.";
        let out = render_message(msg, &ctx);
        assert_eq!(out, "CPU 97.5% on web-1 @pagerduty-checkout @oncall@example.com.");
        assert_eq!(recipients(&out), vec!["pagerduty-checkout", "oncall@example.com"]);
        assert!(recipients("mail me at a@b.com").is_empty(), "an address inside text is not a mention");
    }
}
