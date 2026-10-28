//! On-Call schedules: who is on call when (pure functions, no I/O).
//!
//! A schedule has layers, each a rotation of responders in its own time zone (the schedule's):
//! - the first shift starts at `effectiveFrom` (local wall-clock time), every shift lasts
//!   `rotationHours` *local* hours, so a daily 09:00 handoff stays at 09:00 across DST changes;
//!   a local time that doesn't exist (spring forward) moves to the instant after the gap, an
//!   ambiguous one (fall back) takes the earlier instant;
//! - responders take turns in order, from the first;
//! - optional restrictions (weekly windows such as Mon 09:00 → Fri 17:00) keep only the parts
//!   of each shift inside a window; outside them the layer has nobody;
//! - an optional `effectiveUntil` ends the layer.
//!
//! The final schedule merges the layers: at any instant the responders are those of every layer
//! that has someone, later layers first (a layer added below the others takes precedence), cut to
//! `maxConcurrent` when set (1: only the last layer with someone). Overrides replace the final
//! schedule for their time.

use chrono::{DateTime, Datelike, Duration, LocalResult, NaiveDateTime, NaiveTime, TimeZone, Utc};
use chrono_tz::Tz;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Local date-time format of `effectiveFrom` / `effectiveUntil`.
pub const LOCAL_FORMAT: &str = "%Y-%m-%dT%H:%M";

#[derive(Serialize, Deserialize, ToSchema, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum OnCallWeekday {
    Mon,
    Tue,
    Wed,
    Thu,
    Fri,
    Sat,
    Sun,
}

impl OnCallWeekday {
    fn days_from_monday(self) -> i64 {
        self as i64
    }
}

/// A weekly window in the schedule's time zone: `startDay startTime` → `endDay endTime` (wrapping past Sunday when the end comes first).
#[derive(Serialize, Deserialize, ToSchema, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct OnCallRestriction {
    pub start_day: OnCallWeekday,
    /// "HH:MM"
    pub start_time: String,
    pub end_day: OnCallWeekday,
    /// "HH:MM"
    pub end_time: String,
}

#[derive(Serialize, Deserialize, ToSchema, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct OnCallLayer {
    pub id: String,
    pub name: String,
    /// User ids in rotation order.
    pub members: Vec<String>,
    /// Length of a shift: 24 (daily), 168 (weekly) or any number of hours.
    pub rotation_hours: i32,
    /// First handoff in the schedule's time zone, "YYYY-MM-DDTHH:MM".
    pub effective_from: String,
    /// The layer ends here ("YYYY-MM-DDTHH:MM", schedule time zone).
    #[serde(default)]
    pub effective_until: Option<String>,
    /// Only on call inside these windows (none: always).
    #[serde(default)]
    pub restrictions: Vec<OnCallRestriction>,
}

#[derive(Serialize, Deserialize, ToSchema, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct OnCallShift {
    pub user_id: String,
    /// Unix ms.
    pub start: i64,
    pub end: i64,
}

/// One stretch of the final schedule.
#[derive(Serialize, Deserialize, ToSchema, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct OnCallFinalShift {
    pub user_ids: Vec<String>,
    pub start: i64,
    pub end: i64,
    /// An override replaces the layers here.
    pub is_override: bool,
}

/// An override as the computation needs it.
#[derive(Clone, Debug, PartialEq)]
pub struct OverrideSpan {
    pub user_id: String,
    pub start: i64,
    pub end: i64,
}

pub fn parse_tz(name: &str) -> Option<Tz> {
    name.parse::<Tz>().ok()
}

pub fn parse_local(s: &str) -> Option<NaiveDateTime> {
    NaiveDateTime::parse_from_str(s.trim(), LOCAL_FORMAT).ok()
}

pub fn parse_time(s: &str) -> Option<NaiveTime> {
    NaiveTime::parse_from_str(s.trim(), "%H:%M").ok()
}

/// The instant of a local wall-clock time: the earlier one when ambiguous, the one after the gap when it doesn't exist.
pub fn local_to_utc(tz: Tz, local: NaiveDateTime) -> i64 {
    match tz.from_local_datetime(&local) {
        LocalResult::Single(t) => t.timestamp_millis(),
        LocalResult::Ambiguous(a, _) => a.timestamp_millis(),
        // In a gap: read it with the offset in force just before the gap (02:30 → 03:30 when clocks jump 02:00 → 03:00).
        LocalResult::None => {
            let before = tz.from_local_datetime(&(local - Duration::hours(3))).earliest().map(|b| b.offset_ms()).unwrap_or(0);
            Utc.from_utc_datetime(&local).timestamp_millis() - before
        }
    }
}

trait OffsetMs {
    fn offset_ms(&self) -> i64;
}

impl OffsetMs for DateTime<Tz> {
    fn offset_ms(&self) -> i64 {
        use chrono::Offset;
        self.offset().fix().local_minus_utc() as i64 * 1000
    }
}

pub fn utc_to_local(tz: Tz, ms: i64) -> NaiveDateTime {
    let utc = DateTime::<Utc>::from_timestamp_millis(ms).unwrap_or_default();
    utc.with_timezone(&tz).naive_local()
}

/// Problems with a layer, for validation messages.
pub fn layer_problem(layer: &OnCallLayer) -> Option<String> {
    if !(1..=8760).contains(&layer.rotation_hours) {
        return Some(format!("{}: shifts last 1–8760 hours", layer.name));
    }
    let Some(from) = parse_local(&layer.effective_from) else { return Some(format!("{}: start must be YYYY-MM-DDTHH:MM", layer.name)) };
    if let Some(until) = &layer.effective_until {
        match parse_local(until) {
            None => return Some(format!("{}: end must be YYYY-MM-DDTHH:MM", layer.name)),
            Some(u) if u <= from => return Some(format!("{}: the end must come after the start", layer.name)),
            _ => {}
        }
    }
    for r in &layer.restrictions {
        let (Some(s), Some(e)) = (parse_time(&r.start_time), parse_time(&r.end_time)) else { return Some(format!("{}: restriction times are HH:MM", layer.name)) };
        if r.start_day == r.end_day && s == e {
            return Some(format!("{}: a restriction can't start and end at the same time", layer.name));
        }
    }
    None
}

/// The restriction windows (UTC ms) overlapping [from, to).
fn restriction_windows(tz: Tz, restrictions: &[OnCallRestriction], from: i64, to: i64) -> Vec<(i64, i64)> {
    let first = utc_to_local(tz, from).date();
    let monday = first - Duration::days(first.weekday().num_days_from_monday() as i64 + 7);
    let mut out = Vec::new();
    let mut week = monday;
    loop {
        let week_start = local_to_utc(tz, week.and_time(NaiveTime::MIN));
        if week_start >= to {
            break;
        }
        for r in restrictions {
            let (Some(st), Some(et)) = (parse_time(&r.start_time), parse_time(&r.end_time)) else { continue };
            let start_local = (week + Duration::days(r.start_day.days_from_monday())).and_time(st);
            let mut end_local = (week + Duration::days(r.end_day.days_from_monday())).and_time(et);
            if end_local <= start_local {
                end_local += Duration::days(7);
            }
            let (s, e) = (local_to_utc(tz, start_local), local_to_utc(tz, end_local));
            if e > from && s < to {
                out.push((s.max(from), e.min(to)));
            }
        }
        week += Duration::days(7);
    }
    merge(out)
}

fn merge(mut spans: Vec<(i64, i64)>) -> Vec<(i64, i64)> {
    spans.sort();
    let mut out: Vec<(i64, i64)> = Vec::new();
    for (s, e) in spans {
        match out.last_mut() {
            Some(last) if s <= last.1 => last.1 = last.1.max(e),
            _ => out.push((s, e)),
        }
    }
    out
}

/// The shifts of one layer overlapping [from, to), clipped to it.
pub fn layer_shifts(layer: &OnCallLayer, tz: Tz, from: i64, to: i64) -> Vec<OnCallShift> {
    let (Some(start_local), true, true) = (parse_local(&layer.effective_from), !layer.members.is_empty(), layer.rotation_hours > 0) else { return Vec::new() };
    if from >= to {
        return Vec::new();
    }
    let until = layer.effective_until.as_deref().and_then(parse_local).map(|u| local_to_utc(tz, u)).unwrap_or(i64::MAX);
    let to = to.min(until);
    let len = Duration::hours(layer.rotation_hours as i64);
    let boundary = |k: i64| local_to_utc(tz, start_local + len * k as i32);
    // The rotation containing `from`, counted in local time (one step back covers DST offsets).
    let from_local = utc_to_local(tz, from);
    let elapsed_h = (from_local - start_local).num_hours();
    let mut k = (elapsed_h.div_euclid(layer.rotation_hours as i64) - 1).max(0);
    let windows = (!layer.restrictions.is_empty()).then(|| restriction_windows(tz, &layer.restrictions, from, to));
    let mut out = Vec::new();
    let n = layer.members.len() as i64;
    loop {
        let (s, e) = (boundary(k), boundary(k + 1));
        if s >= to {
            break;
        }
        let user = &layer.members[k.rem_euclid(n) as usize];
        let (s, e) = (s.max(from), e.min(to));
        if e > s {
            match &windows {
                None => out.push(OnCallShift { user_id: user.clone(), start: s, end: e }),
                Some(ws) => {
                    for &(ws, we) in ws {
                        let (a, b) = (s.max(ws), e.min(we));
                        if b > a {
                            out.push(OnCallShift { user_id: user.clone(), start: a, end: b });
                        }
                    }
                }
            }
        }
        k += 1;
    }
    out
}

/// The final schedule over [from, to): layers merged (later layers first), overrides applied.
pub fn final_schedule(layers: &[OnCallLayer], overrides: &[OverrideSpan], max_concurrent: Option<usize>, tz: Tz, from: i64, to: i64) -> Vec<OnCallFinalShift> {
    let per_layer: Vec<Vec<OnCallShift>> = layers.iter().map(|l| layer_shifts(l, tz, from, to)).collect();
    let ovs: Vec<&OverrideSpan> = overrides.iter().filter(|o| o.end > from && o.start < to && o.end > o.start).collect();
    let mut points: Vec<i64> = vec![from, to];
    for s in per_layer.iter().flatten() {
        points.push(s.start);
        points.push(s.end);
    }
    for o in &ovs {
        points.push(o.start.max(from));
        points.push(o.end.min(to));
    }
    points.sort_unstable();
    points.dedup();
    let mut out: Vec<OnCallFinalShift> = Vec::new();
    let mut cursor = vec![0usize; per_layer.len()];
    for w in points.windows(2) {
        let (a, b) = (w[0], w[1]);
        if b <= a {
            continue;
        }
        let mut users: Vec<String> = Vec::new();
        let covering: Vec<&&OverrideSpan> = ovs.iter().filter(|o| o.start <= a && o.end >= b).collect();
        let is_override = !covering.is_empty();
        if is_override {
            for o in covering {
                if !users.contains(&o.user_id) {
                    users.push(o.user_id.clone());
                }
            }
        } else {
            // Each layer's shifts are sorted and don't overlap: one cursor per layer walks them along the segments.
            for (i, shifts) in per_layer.iter().enumerate().rev() {
                while cursor[i] < shifts.len() && shifts[cursor[i]].end <= a {
                    cursor[i] += 1;
                }
                if let Some(s) = shifts.get(cursor[i]).filter(|s| s.start <= a && s.end >= b) {
                    if !users.contains(&s.user_id) {
                        users.push(s.user_id.clone());
                    }
                }
            }
            if let Some(max) = max_concurrent {
                users.truncate(max.max(1));
            }
        }
        if users.is_empty() {
            continue;
        }
        match out.last_mut() {
            Some(last) if last.end == a && last.user_ids == users && last.is_override == is_override => last.end = b,
            _ => out.push(OnCallFinalShift { user_ids: users, start: a, end: b, is_override }),
        }
    }
    out
}

/// Who is on call at `at`.
pub fn on_call_at(layers: &[OnCallLayer], overrides: &[OverrideSpan], max_concurrent: Option<usize>, tz: Tz, at: i64) -> Vec<String> {
    final_schedule(layers, overrides, max_concurrent, tz, at, at + 1).into_iter().next().map(|s| s.user_ids).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    const fn hours_ms(h: i64) -> i64 {
        h * 3_600_000
    }

    fn layer(members: &[&str], hours: i32, from: &str) -> OnCallLayer {
        OnCallLayer {
            id: "l1".into(),
            name: "Layer 1".into(),
            members: members.iter().map(|s| s.to_string()).collect(),
            rotation_hours: hours,
            effective_from: from.into(),
            effective_until: None,
            restrictions: vec![],
        }
    }

    fn utc(s: &str) -> i64 {
        Utc.from_utc_datetime(&parse_local(s).unwrap()).timestamp_millis()
    }

    fn at(tz: Tz, s: &str) -> i64 {
        local_to_utc(tz, parse_local(s).unwrap())
    }

    const UTC: Tz = chrono_tz::UTC;
    const BERLIN: Tz = chrono_tz::Europe::Berlin;
    const NEW_YORK: Tz = chrono_tz::America::New_York;

    #[test]
    fn daily_rotation_takes_turns_in_order() {
        let l = layer(&["ana", "ben", "cy"], 24, "2026-10-01T09:00");
        let shifts = layer_shifts(&l, UTC, utc("2026-10-01T09:00"), utc("2026-10-05T09:00"));
        let who: Vec<&str> = shifts.iter().map(|s| s.user_id.as_str()).collect();
        assert_eq!(who, ["ana", "ben", "cy", "ana"]);
        assert_eq!(shifts[1].start, utc("2026-10-02T09:00"));
        assert_eq!(shifts[1].end, utc("2026-10-03T09:00"));
    }

    #[test]
    fn nobody_before_the_layer_starts_and_shifts_are_clipped_to_the_range() {
        let l = layer(&["ana", "ben"], 24, "2026-10-01T09:00");
        assert!(layer_shifts(&l, UTC, utc("2026-09-20T00:00"), utc("2026-10-01T09:00")).is_empty());
        let shifts = layer_shifts(&l, UTC, utc("2026-10-01T12:00"), utc("2026-10-02T10:00"));
        assert_eq!(
            shifts,
            vec![
                OnCallShift { user_id: "ana".into(), start: utc("2026-10-01T12:00"), end: utc("2026-10-02T09:00") },
                OnCallShift { user_id: "ben".into(), start: utc("2026-10-02T09:00"), end: utc("2026-10-02T10:00") }
            ]
        );
    }

    #[test]
    fn handoff_boundaries_are_half_open() {
        let l = layer(&["ana", "ben"], 168, "2026-10-05T09:00");
        let tz = UTC;
        assert_eq!(on_call_at(std::slice::from_ref(&l), &[], None, tz, utc("2026-10-12T08:59") + 59_999), vec!["ana"]);
        assert_eq!(on_call_at(std::slice::from_ref(&l), &[], None, tz, utc("2026-10-12T09:00")), vec!["ben"]);
        assert_eq!(on_call_at(&[l], &[], None, tz, utc("2026-10-19T09:00")), vec!["ana"]);
    }

    #[test]
    fn far_future_lookups_land_in_the_right_rotation() {
        let l = layer(&["ana", "ben", "cy"], 24, "2026-01-01T00:00");
        // 2027-01-01 is day 365 → 365 % 3 == 2.
        assert_eq!(on_call_at(&[l], &[], None, UTC, utc("2027-01-01T12:00")), vec!["cy"]);
    }

    #[test]
    fn daily_handoff_stays_at_local_time_across_dst() {
        // Berlin leaves summer time on 2026-10-25 (03:00 CEST → 02:00 CET).
        let l = layer(&["ana", "ben"], 24, "2026-10-24T09:00");
        let shifts = layer_shifts(&l, BERLIN, at(BERLIN, "2026-10-24T09:00"), at(BERLIN, "2026-10-26T09:00"));
        assert_eq!(shifts.len(), 2);
        assert_eq!(shifts[0].start, utc("2026-10-24T07:00"));
        assert_eq!(shifts[0].end, utc("2026-10-25T08:00"));
        // The shift over the change lasts 25 hours.
        assert_eq!(shifts[0].end - shifts[0].start, hours_ms(25));
        assert_eq!(shifts[1].end, utc("2026-10-26T08:00"));
    }

    #[test]
    fn spring_forward_gap_and_fall_back_ambiguity() {
        // New York: 2026-03-08 02:00 → 03:00 (02:30 doesn't exist); 2026-11-01 02:00 → 01:00 (01:30 happens twice).
        assert_eq!(at(NEW_YORK, "2026-03-08T02:30"), utc("2026-03-08T07:30"));
        assert_eq!(at(NEW_YORK, "2026-11-01T01:30"), utc("2026-11-01T05:30"));
        let l = layer(&["ana", "ben"], 24, "2026-03-07T02:30");
        let shifts = layer_shifts(&l, NEW_YORK, utc("2026-03-07T00:00"), utc("2026-03-10T00:00"));
        assert_eq!(shifts[0].end, utc("2026-03-08T07:30"));
        assert_eq!(shifts[1].start, utc("2026-03-08T07:30"));
        assert_eq!(shifts[1].end, utc("2026-03-09T06:30"));
    }

    #[test]
    fn custom_twelve_hour_rotation() {
        let l = layer(&["ana", "ben"], 12, "2026-10-01T08:00");
        assert_eq!(on_call_at(std::slice::from_ref(&l), &[], None, UTC, utc("2026-10-01T19:59")), vec!["ana"]);
        assert_eq!(on_call_at(std::slice::from_ref(&l), &[], None, UTC, utc("2026-10-01T20:00")), vec!["ben"]);
        assert_eq!(on_call_at(&[l], &[], None, UTC, utc("2026-10-02T08:00")), vec!["ana"]);
    }

    #[test]
    fn restrictions_keep_only_business_hours() {
        let mut l = layer(&["ana", "ben"], 168, "2026-10-05T00:00"); // Monday
        l.restrictions = vec![OnCallRestriction { start_day: OnCallWeekday::Mon, start_time: "09:00".into(), end_day: OnCallWeekday::Fri, end_time: "17:00".into() }];
        let tz = BERLIN;
        assert!(on_call_at(std::slice::from_ref(&l), &[], None, tz, at(tz, "2026-10-05T08:00")).is_empty());
        assert_eq!(on_call_at(std::slice::from_ref(&l), &[], None, tz, at(tz, "2026-10-07T03:00")), vec!["ana"]);
        assert!(on_call_at(std::slice::from_ref(&l), &[], None, tz, at(tz, "2026-10-10T12:00")).is_empty());
        assert_eq!(on_call_at(std::slice::from_ref(&l), &[], None, tz, at(tz, "2026-10-12T09:00")), vec!["ben"]);
        let shifts = layer_shifts(&l, tz, at(tz, "2026-10-05T00:00"), at(tz, "2026-10-12T00:00"));
        assert_eq!(shifts, vec![OnCallShift { user_id: "ana".into(), start: at(tz, "2026-10-05T09:00"), end: at(tz, "2026-10-09T17:00") }]);
    }

    #[test]
    fn daily_restrictions_and_windows_wrapping_the_week() {
        let mut l = layer(&["ana"], 24, "2026-10-01T00:00");
        let day = |d| OnCallRestriction { start_day: d, start_time: "18:00".into(), end_day: d, end_time: "08:00".into() };
        // An end before the start wraps into the next week: Sun 18:00 → Sun 08:00 is all week but Sunday daytime.
        l.restrictions = vec![day(OnCallWeekday::Sun)];
        assert_eq!(on_call_at(std::slice::from_ref(&l), &[], None, UTC, utc("2026-10-04T23:00")), vec!["ana"]); // Sunday night
        assert_eq!(on_call_at(std::slice::from_ref(&l), &[], None, UTC, utc("2026-10-07T12:00")), vec!["ana"]); // Wednesday
        assert!(on_call_at(std::slice::from_ref(&l), &[], None, UTC, utc("2026-10-11T12:00")).is_empty()); // Sunday noon
        l.restrictions = vec![OnCallRestriction { start_day: OnCallWeekday::Fri, start_time: "18:00".into(), end_day: OnCallWeekday::Mon, end_time: "08:00".into() }];
        assert_eq!(on_call_at(std::slice::from_ref(&l), &[], None, UTC, utc("2026-10-05T07:00")), vec!["ana"]); // Monday morning
        assert!(on_call_at(&[l], &[], None, UTC, utc("2026-10-05T09:00")).is_empty());
    }

    #[test]
    fn effective_until_ends_the_layer() {
        let mut l = layer(&["ana"], 24, "2026-10-01T00:00");
        l.effective_until = Some("2026-10-03T12:00".into());
        assert_eq!(on_call_at(std::slice::from_ref(&l), &[], None, UTC, utc("2026-10-03T11:00")), vec!["ana"]);
        assert!(on_call_at(&[l], &[], None, UTC, utc("2026-10-03T12:00")).is_empty());
    }

    #[test]
    fn later_layers_come_first_and_max_concurrent_cuts() {
        let base = layer(&["ana", "ben"], 168, "2026-10-05T00:00");
        let mut weekend = layer(&["cy"], 168, "2026-10-05T00:00");
        weekend.id = "l2".into();
        weekend.restrictions = vec![OnCallRestriction { start_day: OnCallWeekday::Sat, start_time: "00:00".into(), end_day: OnCallWeekday::Mon, end_time: "00:00".into() }];
        let layers = [base, weekend];
        assert_eq!(on_call_at(&layers, &[], None, UTC, utc("2026-10-07T12:00")), vec!["ana"]);
        assert_eq!(on_call_at(&layers, &[], None, UTC, utc("2026-10-10T12:00")), vec!["cy", "ana"]);
        assert_eq!(on_call_at(&layers, &[], Some(1), UTC, utc("2026-10-10T12:00")), vec!["cy"]);
        let fin = final_schedule(&layers, &[], Some(1), UTC, utc("2026-10-05T00:00"), utc("2026-10-12T00:00"));
        let shape: Vec<(&str, i64, i64)> = fin.iter().map(|s| (s.user_ids[0].as_str(), s.start, s.end)).collect();
        assert_eq!(shape, vec![("ana", utc("2026-10-05T00:00"), utc("2026-10-10T00:00")), ("cy", utc("2026-10-10T00:00"), utc("2026-10-12T00:00"))]);
    }

    #[test]
    fn overrides_replace_the_final_schedule_and_adjacent_stretches_merge() {
        let l = layer(&["ana", "ben"], 24, "2026-10-01T00:00");
        let ov = [OverrideSpan { user_id: "dee".into(), start: utc("2026-10-01T10:00"), end: utc("2026-10-01T14:00") }];
        let fin = final_schedule(std::slice::from_ref(&l), &ov, None, UTC, utc("2026-10-01T00:00"), utc("2026-10-03T00:00"));
        assert_eq!(
            fin,
            vec![
                OnCallFinalShift { user_ids: vec!["ana".into()], start: utc("2026-10-01T00:00"), end: utc("2026-10-01T10:00"), is_override: false },
                OnCallFinalShift { user_ids: vec!["dee".into()], start: utc("2026-10-01T10:00"), end: utc("2026-10-01T14:00"), is_override: true },
                OnCallFinalShift { user_ids: vec!["ana".into()], start: utc("2026-10-01T14:00"), end: utc("2026-10-02T00:00"), is_override: false },
                OnCallFinalShift { user_ids: vec!["ben".into()], start: utc("2026-10-02T00:00"), end: utc("2026-10-03T00:00"), is_override: false },
            ]
        );
        // An override fills gaps too.
        let gap = [OverrideSpan { user_id: "dee".into(), start: utc("2026-09-30T00:00"), end: utc("2026-09-30T06:00") }];
        assert_eq!(on_call_at(&[l], &gap, None, UTC, utc("2026-09-30T01:00")), vec!["dee"]);
    }

    #[test]
    fn empty_layers_and_bad_input_yield_nobody() {
        assert!(layer_shifts(&layer(&[], 24, "2026-10-01T00:00"), UTC, 0, i64::MAX / 4).is_empty());
        assert!(layer_shifts(&layer(&["ana"], 24, "garbage"), UTC, 0, 10).is_empty());
        assert!(final_schedule(&[], &[], None, UTC, 0, 1000).is_empty());
    }

    #[test]
    fn layer_validation() {
        assert!(layer_problem(&layer(&["a"], 24, "2026-10-01T09:00")).is_none());
        assert!(layer_problem(&layer(&["a"], 0, "2026-10-01T09:00")).is_some());
        assert!(layer_problem(&layer(&["a"], 24, "2026-10-01")).is_some());
        let mut l = layer(&["a"], 24, "2026-10-01T09:00");
        l.effective_until = Some("2026-09-01T09:00".into());
        assert!(layer_problem(&l).is_some());
        l.effective_until = None;
        l.restrictions = vec![OnCallRestriction { start_day: OnCallWeekday::Mon, start_time: "25:00".into(), end_day: OnCallWeekday::Mon, end_time: "10:00".into() }];
        assert!(layer_problem(&l).is_some());
        assert!(parse_tz("Europe/Berlin").is_some() && parse_tz("Mars/Olympus").is_none());
    }
}
