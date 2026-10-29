//! Downtimes in Datadog's shapes, on our downtimes: `/api/v2/downtime` (the `datadog_downtime_schedule`
//! resource; ids are UUIDs) and the legacy `/api/v1/downtime` (`datadog_downtime`; integer ids).
//!
//! Both write the same rows the UI lists and the evaluator reads. The document as received is kept
//! (`datadog_document`, with the API version that wrote it) and returned on reads with the
//! server-owned fields (status, cancellation, ids, timestamps) from the row; a downtime written
//! through one version, or in the UI, reads through the other as derived from the row.
//!
//! Mapping:
//! - v2 `monitor_identifier` (`monitor_id` or `monitor_tags`, `["*"]` = all monitors) and v1
//!   `monitor_id` / `monitor_tags`;
//! - scope: v2's search string (`*`, or `key:value` terms joined by AND) and v1's tag list;
//! - one-time `start` / `end`; recurrences repeating every day or every week (v1 `days` / `weeks`
//!   with period 1, v2 `FREQ=DAILY` / `FREQ=WEEKLY`), limited to weekdays (v1 `week_days`, v2
//!   `BYDAY`) and ending at a date or after a number of occurrences; recurrence starts in the
//!   downtime's timezone;
//! - `message`.
//!
//! `display_timezone`, `mute_first_recovery_notification`, `notify_end_states` and `notify_end_types`
//! are stored and returned, not acted on. Other recurrences (monthly, yearly, every N days, several
//! v2 recurrences, other RRULE parts) and scopes with OR / NOT / wildcards are rejected with a 400.

use std::collections::HashMap;

use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use chrono::{Datelike, NaiveDateTime, TimeZone};
use serde_json::{json, Map, Value};

use super::monitors::{iso, now_ms, org_number, user_number};
use super::{DdAuth, DdError};
use crate::auth::{Access, AuthContext};
use crate::routes::monitor_engine::{DowntimeWindow, DAY_MS};
use crate::state::AppState;

const WEEK_MS: i64 = 7 * DAY_MS;
const DAY_NAMES: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
const RRULE_DAYS: [&str; 7] = ["MO", "TU", "WE", "TH", "FR", "SA", "SU"];
const V2_ATTRIBUTES: &[&str] = &["display_timezone", "message", "monitor_identifier", "mute_first_recovery_notification", "notify_end_states", "notify_end_types", "schedule", "scope"];

fn default_end_states() -> Value {
    json!(["alert", "no data", "warn"])
}

// ================================================================ our model

/// What a Datadog downtime means for our downtime row. Times in epoch ms.
#[derive(Debug, Clone, PartialEq)]
pub struct Plan {
    pub monitor_id: Option<i64>,
    pub monitor_tags: Vec<String>,
    /// `key:value` tags; empty = every group.
    pub scope: Vec<String>,
    pub message: String,
    pub start: i64,
    pub end: Option<i64>,
    /// `daily` or `weekly`.
    pub recurrence: Option<&'static str>,
    pub recur_until: Option<i64>,
    /// With a daily recurrence: only these UTC weekdays (0 = Monday).
    pub weekdays: Vec<i16>,
}

fn timezone(name: Option<&str>) -> Result<chrono_tz::Tz, String> {
    match name.map(str::trim) {
        None | Some("") => Ok(chrono_tz::UTC),
        Some(n) => n.parse().map_err(|_| format!("Invalid timezone '{n}'")),
    }
}

/// UTC weekday (0 = Monday) of `ms`.
fn utc_weekday(ms: i64) -> i16 {
    ((ms.div_euclid(DAY_MS) + 3) % 7) as i16
}

/// Weekdays in `tz` → the UTC weekdays of occurrences starting at `start`'s time of day.
fn utc_weekdays(local: &[i16], start: i64, tz: chrono_tz::Tz) -> Vec<i16> {
    let local_day = chrono::DateTime::from_timestamp_millis(start).map(|t| t.with_timezone(&tz).weekday().num_days_from_monday() as i16).unwrap_or(0);
    let shift = utc_weekday(start) - local_day;
    let mut out: Vec<i16> = local.iter().map(|d| (d + shift).rem_euclid(7)).collect();
    out.sort();
    out.dedup();
    out
}

/// The start of the `n`-th occurrence (1-based).
fn nth_occurrence(start: i64, period: i64, weekdays: &[i16], n: i64) -> i64 {
    if weekdays.is_empty() {
        return start + (n.max(1) - 1) * period;
    }
    let mut seen = 0;
    let mut at = start;
    for _ in 0..(n.max(1) * 7 + 7) {
        if weekdays.contains(&utc_weekday(at)) {
            seen += 1;
            if seen == n.max(1) {
                return at;
            }
        }
        at += DAY_MS;
    }
    at
}

fn check_times(plan: &Plan) -> Result<(), String> {
    if let Some(end) = plan.end {
        if end <= plan.start {
            return Err("the downtime must end after it starts".into());
        }
    }
    if let Some(r) = plan.recurrence {
        let end = plan.end.ok_or("a recurring downtime needs an end (or a duration)")?;
        let period = if r == "daily" { DAY_MS } else { WEEK_MS };
        if end - plan.start >= period {
            return Err("each occurrence must be shorter than the repeat period".into());
        }
    }
    Ok(())
}

// ================================================================ v1 documents

fn int_field(obj: &Map<String, Value>, key: &str) -> Result<Option<i64>, String> {
    match obj.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => v.as_i64().map(Some).ok_or_else(|| format!("{key} must be an integer")),
    }
}

fn str_list(v: Option<&Value>, key: &str) -> Result<Vec<String>, String> {
    match v {
        None | Some(Value::Null) => Ok(vec![]),
        Some(Value::Array(items)) => items.iter().map(|i| i.as_str().map(|s| s.trim().to_string()).ok_or_else(|| format!("{key} must be a list of strings"))).collect(),
        Some(_) => Err(format!("{key} must be a list of strings")),
    }
}

fn tag_scope(entries: Vec<String>) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    for s in entries.into_iter().filter(|s| !s.is_empty() && s != "*") {
        if !s.contains(':') {
            return Err(format!("scope entries are '*' or key:value tags, not '{s}'"));
        }
        out.push(s);
    }
    Ok(out)
}

fn monitor_tags(tags: Vec<String>) -> Vec<String> {
    tags.into_iter().filter(|t| !t.is_empty() && t != "*").collect()
}

/// Our downtime from a v1 document. `start` defaults to `default_start` (now, or the stored start).
pub fn v1_plan(doc: &Value, default_start: i64) -> Result<Plan, String> {
    let obj = doc.as_object().ok_or("the downtime must be a JSON object")?;
    let scope = match obj.get("scope") {
        Some(Value::Array(a)) if !a.is_empty() => tag_scope(str_list(obj.get("scope"), "scope")?)?,
        Some(Value::String(s)) if !s.trim().is_empty() => tag_scope(s.split(',').map(|p| p.trim().to_string()).collect())?,
        _ => return Err("Invalid scope parameter".into()),
    };
    let tz = timezone(obj.get("timezone").and_then(Value::as_str))?;
    let start = int_field(obj, "start")?.map_or(default_start, |s| s * 1000);
    let end = int_field(obj, "end")?.map(|e| e * 1000);
    let monitor_id = int_field(obj, "monitor_id")?;
    let message = obj.get("message").and_then(Value::as_str).unwrap_or_default().trim().to_string();
    let mut plan = Plan {
        monitor_id,
        monitor_tags: if monitor_id.is_some() { vec![] } else { monitor_tags(str_list(obj.get("monitor_tags"), "monitor_tags")?) },
        scope,
        message,
        start,
        end,
        recurrence: None,
        recur_until: None,
        weekdays: vec![],
    };
    if let Some(r) = obj.get("recurrence").filter(|r| !r.is_null()) {
        let r = r.as_object().ok_or("recurrence must be an object")?;
        let kind = r.get("type").and_then(Value::as_str).unwrap_or_default();
        if !["days", "weeks", "months", "years", "rrule"].contains(&kind) {
            return Err("Invalid recurrence type".into());
        }
        let days = str_list(r.get("week_days"), "week_days")?;
        let local_days: Vec<i16> =
            days.iter().map(|d| DAY_NAMES.iter().position(|n| n == d).map(|i| i as i16)).collect::<Option<_>>().ok_or("Invalid week_days: use Mon, Tue, Wed, Thu, Fri, Sat and Sun")?;
        let (until_date, until_occurrences) = (int_field(r, "until_date")?, int_field(r, "until_occurrences")?);
        if until_date.is_some() && until_occurrences.is_some() {
            return Err("Provide only one of until_occurrences and until_date".into());
        }
        if kind != "days" && kind != "weeks" {
            return Err(format!("recurrence type '{kind}' is not supported by ObserveCraft yet (repeat every day or every week)"));
        }
        let period = int_field(r, "period")?.unwrap_or(1);
        if period != 1 {
            return Err(format!("a recurrence period of {period} is not supported by ObserveCraft yet (repeat every day or every week)"));
        }
        if kind == "days" && !local_days.is_empty() {
            return Err("week_days apply to weekly recurrences".into());
        }
        plan.weekdays = utc_weekdays(&local_days, start, tz);
        plan.recurrence = Some(if kind == "days" || !plan.weekdays.is_empty() { "daily" } else { "weekly" });
        let period_ms = if plan.recurrence == Some("daily") { DAY_MS } else { WEEK_MS };
        plan.recur_until = until_date.map(|u| u * 1000).or_else(|| until_occurrences.map(|n| nth_occurrence(start, period_ms, &plan.weekdays, n)));
    }
    check_times(&plan)?;
    Ok(plan)
}

/// A v1 document as sent (or merged onto the stored one), with Datadog's defaults.
pub fn v1_normalize(body: Value, stored: Option<Value>) -> Result<Value, String> {
    let Value::Object(mut incoming) = body else { return Err("the downtime must be a JSON object".into()) };
    for key in
        ["id", "org_id", "created", "modified", "canceled", "active", "disabled", "status", "creator_id", "updater_id", "parent_id", "child_id", "downtime_type", "uuid", "creator", "active_child"]
    {
        incoming.remove(key);
    }
    let mut doc = match stored {
        Some(Value::Object(mut s)) => {
            s.extend(incoming);
            s
        }
        _ => incoming,
    };
    for (key, value) in [
        ("message", Value::Null),
        ("monitor_id", Value::Null),
        ("monitor_tags", json!(["*"])),
        ("mute_first_recovery_notification", json!(false)),
        ("notify_end_states", default_end_states()),
        ("notify_end_types", json!(["expired"])),
        ("recurrence", Value::Null),
        ("end", Value::Null),
        ("timezone", json!("UTC")),
    ] {
        doc.entry(key).or_insert(value);
    }
    if let Some(r) = doc.get_mut("recurrence").and_then(Value::as_object_mut) {
        for key in ["week_days", "until_occurrences", "until_date"] {
            r.entry(key).or_insert(Value::Null);
        }
        r.entry("period").or_insert(json!(1));
    }
    Ok(Value::Object(doc))
}

// ================================================================ v2 documents

/// A v2 scope (`*`, or `key:value` terms joined with AND) → our tag scope.
pub fn parse_scope(scope: &str) -> Result<Vec<String>, String> {
    let scope = scope.trim();
    if scope.is_empty() {
        return Err("scope is required".into());
    }
    let unsupported = || format!("scope '{scope}' is not supported by ObserveCraft yet: use '*' or key:value terms joined with AND");
    let mut out = Vec::new();
    let mut rest = scope;
    while !rest.is_empty() {
        let mut term = String::new();
        let mut in_quotes = false;
        let mut end = rest.len();
        for (i, c) in rest.char_indices() {
            match c {
                '"' => in_quotes = !in_quotes,
                c if c.is_whitespace() && !in_quotes => {
                    end = i;
                    break;
                }
                c => term.push(c),
            }
        }
        rest = rest[end..].trim_start();
        match term.as_str() {
            "AND" => continue,
            "*" => continue,
            "OR" | "NOT" => return Err(unsupported()),
            t if t.starts_with(['-', '(', '!']) || t.ends_with(')') => return Err(unsupported()),
            _ => {}
        }
        let (key, value) = term.split_once(':').ok_or_else(|| format!("All values must have a key: '{term}' (use key:value)"))?;
        if key.is_empty() || value.is_empty() || value.contains('*') {
            return Err(unsupported());
        }
        out.push(format!("{key}:{value}"));
    }
    Ok(out)
}

fn scope_text(scope: &[String]) -> String {
    if scope.is_empty() {
        return "*".into();
    }
    scope
        .iter()
        .map(|t| match t.split_once(':') {
            Some((k, v)) if v.contains(char::is_whitespace) => format!("{k}:\"{v}\""),
            _ => t.clone(),
        })
        .collect::<Vec<_>>()
        .join(" AND ")
}

/// `1h`, `30m`, `2d`, `1w` → ms.
fn duration_ms(s: &str) -> Result<i64, String> {
    let s = s.trim();
    let (n, unit) = s.split_at(s.find(|c: char| !c.is_ascii_digit()).unwrap_or(s.len()));
    let n: i64 = n.parse().map_err(|_| format!("Invalid duration '{s}' (e.g. 30m, 1h, 1d, 1w)"))?;
    let unit_ms = match unit {
        "m" => 60_000,
        "h" => 3_600_000,
        "d" => DAY_MS,
        "w" => WEEK_MS,
        _ => return Err(format!("Invalid duration '{s}' (e.g. 30m, 1h, 1d, 1w)")),
    };
    Ok(n * unit_ms)
}

fn duration_text(ms: i64) -> String {
    match ms {
        m if m % WEEK_MS == 0 => format!("{}w", m / WEEK_MS),
        m if m % DAY_MS == 0 => format!("{}d", m / DAY_MS),
        m if m % 3_600_000 == 0 => format!("{}h", m / 3_600_000),
        m => format!("{}m", (m / 60_000).max(1)),
    }
}

/// A parsed RRULE: (recurrence, local weekdays, UNTIL in ms, COUNT).
type Rrule = (&'static str, Vec<i16>, Option<i64>, Option<i64>);

/// An RRULE → [`Rrule`].
fn parse_rrule(rule: &str) -> Result<Rrule, String> {
    let unsupported = |part: &str| format!("RRULE part '{part}' is not supported by ObserveCraft yet (FREQ=DAILY or FREQ=WEEKLY with BYDAY, UNTIL, COUNT)");
    let (mut freq, mut days, mut until, mut count) = (None, Vec::new(), None, None);
    for part in rule.trim().trim_start_matches("RRULE:").split(';').map(str::trim).filter(|p| !p.is_empty()) {
        let (key, value) = part.split_once('=').ok_or_else(|| format!("Invalid RRULE '{rule}'"))?;
        match key.to_uppercase().as_str() {
            "FREQ" => {
                freq = Some(match value.to_uppercase().as_str() {
                    "DAILY" => "daily",
                    "WEEKLY" => "weekly",
                    _ => return Err(unsupported(part)),
                })
            }
            "INTERVAL" if value == "1" => {}
            "BYDAY" => {
                for d in value.split(',') {
                    days.push(RRULE_DAYS.iter().position(|n| n.eq_ignore_ascii_case(d.trim())).ok_or_else(|| unsupported(part))? as i16);
                }
            }
            "UNTIL" => {
                let v = value.trim_end_matches('Z');
                let t = NaiveDateTime::parse_from_str(v, "%Y%m%dT%H%M%S")
                    .or_else(|_| chrono::NaiveDate::parse_from_str(v, "%Y%m%d").map(|d| d.and_hms_opt(23, 59, 59).unwrap_or_default()))
                    .map_err(|_| format!("Invalid RRULE UNTIL '{value}'"))?;
                until = Some(t.and_utc().timestamp_millis());
            }
            "COUNT" => count = Some(value.parse::<i64>().map_err(|_| format!("Invalid RRULE COUNT '{value}'"))?),
            "WKST" => {}
            _ => return Err(unsupported(part)),
        }
    }
    let freq = freq.ok_or("RRULE needs FREQ")?;
    if freq == "daily" && !days.is_empty() {
        return Err(unsupported("BYDAY with FREQ=DAILY"));
    }
    Ok((freq, days, until, count))
}

fn rrule_text(plan: &Plan) -> String {
    let mut rule = match (plan.recurrence, plan.weekdays.is_empty()) {
        (Some("weekly"), _) => "FREQ=WEEKLY".to_string(),
        (_, false) => format!("FREQ=WEEKLY;BYDAY={}", plan.weekdays.iter().map(|d| RRULE_DAYS[*d as usize]).collect::<Vec<_>>().join(",")),
        _ => "FREQ=DAILY".to_string(),
    };
    if let Some(u) = plan.recur_until.and_then(chrono::DateTime::from_timestamp_millis) {
        rule.push_str(&format!(";UNTIL={}", u.format("%Y%m%dT%H%M%SZ")));
    }
    rule
}

fn parse_time(v: &Value, key: &str) -> Result<Option<i64>, String> {
    match v {
        Value::Null => Ok(None),
        Value::String(s) => chrono::DateTime::parse_from_rfc3339(s).map(|t| Some(t.timestamp_millis())).map_err(|_| format!("{key} must be an ISO-8601 date-time with a UTC offset")),
        _ => Err(format!("{key} must be an ISO-8601 date-time")),
    }
}

fn local_start(s: &str, tz: chrono_tz::Tz) -> Result<i64, String> {
    let t = NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S")
        .or_else(|_| NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M"))
        .map_err(|_| format!("Invalid recurrence start '{s}' (YYYY-MM-DDTHH:MM:SS, local time)"))?;
    tz.from_local_datetime(&t).earliest().map(|t| t.timestamp_millis()).ok_or_else(|| format!("recurrence start '{s}' doesn't exist in {tz}"))
}

/// Our downtime from v2 attributes (normalized: schedule starts filled in).
pub fn v2_plan(attrs: &Value) -> Result<Plan, String> {
    let obj = attrs.as_object().ok_or("attributes must be an object")?;
    let scope = parse_scope(obj.get("scope").and_then(Value::as_str).ok_or("scope is required")?)?;
    let ident = obj.get("monitor_identifier").and_then(Value::as_object).ok_or("monitor_identifier is required (monitor_id or monitor_tags)")?;
    let monitor_id = int_field(ident, "monitor_id")?;
    let tags = monitor_tags(str_list(ident.get("monitor_tags"), "monitor_identifier.monitor_tags")?);
    if monitor_id.is_none() && ident.get("monitor_tags").is_none() {
        return Err("monitor_identifier needs monitor_id or monitor_tags".into());
    }
    let message = obj.get("message").and_then(Value::as_str).unwrap_or_default().trim().to_string();
    let mut plan = Plan { monitor_id, monitor_tags: if monitor_id.is_some() { vec![] } else { tags }, scope, message, start: 0, end: None, recurrence: None, recur_until: None, weekdays: vec![] };
    let schedule = obj.get("schedule").ok_or("schedule is required")?;
    match schedule.get("recurrences") {
        Some(recurrences) => {
            let list = recurrences.as_array().ok_or("schedule.recurrences must be a list")?;
            let [r] = list.as_slice() else { return Err("only one recurrence per downtime is supported by ObserveCraft yet".into()) };
            let tz = timezone(schedule.get("timezone").and_then(Value::as_str))?;
            let start = local_start(r.get("start").and_then(Value::as_str).ok_or("recurrence start is required")?, tz)?;
            let duration = duration_ms(r.get("duration").and_then(Value::as_str).ok_or("recurrence duration is required")?)?;
            let (freq, local_days, until, count) = parse_rrule(r.get("rrule").and_then(Value::as_str).ok_or("recurrence rrule is required")?)?;
            plan.start = start;
            plan.end = Some(start + duration);
            plan.weekdays = utc_weekdays(&local_days, start, tz);
            plan.recurrence = Some(if freq == "daily" || !plan.weekdays.is_empty() { "daily" } else { "weekly" });
            let period = if plan.recurrence == Some("daily") { DAY_MS } else { WEEK_MS };
            plan.recur_until = until.or_else(|| count.map(|n| nth_occurrence(start, period, &plan.weekdays, n)));
        }
        None => {
            plan.start = parse_time(schedule.get("start").unwrap_or(&Value::Null), "schedule.start")?.ok_or("schedule.start is required")?;
            plan.end = parse_time(schedule.get("end").unwrap_or(&Value::Null), "schedule.end")?;
        }
    }
    check_times(&plan)?;
    Ok(plan)
}

/// v2 attributes as sent (or merged onto the stored ones), with Datadog's defaults and the
/// schedule's starts filled in (`start: null` keeps the stored start, or is now).
pub fn v2_normalize(attrs: &Value, stored: Option<&Value>, now: i64) -> Result<Value, String> {
    let incoming = attrs.as_object().ok_or("attributes must be an object")?;
    if let Some(unexpected) = incoming.keys().find(|k| !V2_ATTRIBUTES.contains(&k.as_str())) {
        return Err(format!("Additional properties are not allowed ('{unexpected}' was unexpected)"));
    }
    let stored = stored.and_then(Value::as_object);
    let mut doc = stored.cloned().unwrap_or_default();
    for (k, v) in incoming {
        doc.insert(k.clone(), v.clone());
    }
    for (key, value) in [
        ("display_timezone", json!("UTC")),
        ("message", Value::Null),
        ("mute_first_recovery_notification", json!(false)),
        ("notify_end_states", default_end_states()),
        ("notify_end_types", json!(["expired"])),
    ] {
        doc.entry(key).or_insert(value);
    }
    let stored_schedule = stored.and_then(|s| s.get("schedule"));
    let schedule = match doc.get("schedule") {
        None | Some(Value::Null) => json!({ "start": null, "end": null }),
        Some(s) => s.clone(),
    };
    let schedule = match schedule.get("recurrences") {
        Some(recurrences) => {
            let tz = timezone(schedule.get("timezone").and_then(Value::as_str))?;
            let stored_start = stored_schedule.and_then(|s| s["recurrences"][0]["start"].as_str()).map(str::to_string);
            let mut recurrences = recurrences.clone();
            if let Some(list) = recurrences.as_array_mut() {
                for r in list.iter_mut().filter(|r| r.is_object()) {
                    if r.get("start").and_then(Value::as_str).is_none() {
                        let start =
                            stored_start.clone().unwrap_or_else(|| chrono::DateTime::from_timestamp_millis(now).map(|t| t.with_timezone(&tz).format("%Y-%m-%dT%H:%M").to_string()).unwrap_or_default());
                        r["start"] = json!(start);
                    }
                }
            }
            json!({ "recurrences": recurrences, "timezone": schedule.get("timezone").and_then(Value::as_str).filter(|t| !t.is_empty()).unwrap_or("UTC") })
        }
        None => {
            let start = match schedule.get("start") {
                Some(Value::String(s)) => json!(s),
                _ => stored_schedule.and_then(|s| s.get("start")).filter(|s| s.is_string()).cloned().unwrap_or_else(|| json!(iso(now))),
            };
            json!({ "start": start, "end": schedule.get("end").cloned().unwrap_or(Value::Null) })
        }
    };
    doc.insert("schedule".into(), schedule);
    Ok(Value::Object(doc))
}

// ================================================================ storage

#[derive(sqlx::FromRow, Clone)]
pub(crate) struct Row {
    id: i64,
    uuid: String,
    monitor_id: Option<i64>,
    monitor_tags: Vec<String>,
    scope: Vec<String>,
    message: String,
    start_at: i64,
    end_at: Option<i64>,
    recurrence: Option<String>,
    recur_until: Option<i64>,
    weekdays: Vec<i16>,
    creator_id: Option<String>,
    creator_email: Option<String>,
    creator_name: Option<String>,
    creator_created_at: Option<i64>,
    updater_id: Option<String>,
    created_at: i64,
    modified_at: i64,
    canceled_at: Option<i64>,
    datadog_document: Option<Value>,
    datadog_version: Option<String>,
}

const SELECT: &str = "SELECT d.id, d.uuid::text AS uuid, d.monitor_id, d.monitor_tags, d.scope, d.message, \
    (extract(epoch FROM d.start_at) * 1000)::bigint AS start_at, (extract(epoch FROM d.end_at) * 1000)::bigint AS end_at, d.recurrence, \
    (extract(epoch FROM d.recur_until) * 1000)::bigint AS recur_until, d.weekdays, d.created_by::text AS creator_id, u.email AS creator_email, \
    u.name AS creator_name, (extract(epoch FROM u.created_at) * 1000)::bigint AS creator_created_at, d.modified_by::text AS updater_id, \
    (extract(epoch FROM d.created_at) * 1000)::bigint AS created_at, (extract(epoch FROM d.modified_at) * 1000)::bigint AS modified_at, \
    (extract(epoch FROM d.canceled_at) * 1000)::bigint AS canceled_at, d.datadog_document, d.datadog_version \
    FROM downtimes d LEFT JOIN users u ON u.id = d.created_by";

impl Row {
    fn window(&self) -> DowntimeWindow {
        DowntimeWindow {
            start: self.start_at,
            end: self.end_at,
            period: match self.recurrence.as_deref() {
                Some("daily") => Some(DAY_MS),
                Some("weekly") => Some(WEEK_MS),
                _ => None,
            },
            until: self.recur_until,
            canceled: self.canceled_at,
            weekdays: self.weekdays.clone(),
        }
    }

    pub(crate) fn applies_to(&self, monitor_id: i64, tags: &[String]) -> bool {
        match self.monitor_id {
            Some(id) => id == monitor_id,
            None => self.monitor_tags.iter().all(|t| tags.contains(t)),
        }
    }

    fn status(&self, now: i64) -> &'static str {
        let w = self.window();
        if self.canceled_at.is_some() {
            "canceled"
        } else if w.active_at(now).is_some() {
            "active"
        } else if w.finished_at(now) {
            "ended"
        } else {
            "scheduled"
        }
    }

    fn plan(&self) -> Plan {
        Plan {
            monitor_id: self.monitor_id,
            monitor_tags: self.monitor_tags.clone(),
            scope: self.scope.clone(),
            message: self.message.clone(),
            start: self.start_at,
            end: self.end_at,
            recurrence: match self.recurrence.as_deref() {
                Some("daily") => Some("daily"),
                Some("weekly") => Some("weekly"),
                _ => None,
            },
            recur_until: self.recur_until,
            weekdays: self.weekdays.clone(),
        }
    }

    fn stored(&self, version: &str) -> Option<&Value> {
        self.datadog_document.as_ref().filter(|_| self.datadog_version.as_deref() == Some(version))
    }

    /// A monitor's `matching_downtimes` entry, unless the downtime has ended.
    pub(crate) fn matching(&self, now: i64) -> Option<Value> {
        let status = self.status(now);
        (status == "active" || status == "scheduled").then(|| {
            json!({ "id": self.id, "active": status == "active", "monitor_id": self.monitor_id, "start": self.start_at / 1000, "end": self.end_at.map(|e| e / 1000),
                "scope": if self.scope.is_empty() { vec!["*".to_string()] } else { self.scope.clone() }, "groups": Vec::<String>::new() })
        })
    }

    /// The v1 document: as written through v1, else derived from the row.
    pub(crate) fn v1_document(&self, org: &str, now: i64, with_creator: bool) -> Value {
        let mut doc = match self.stored("v1") {
            Some(d) => d.clone(),
            None => {
                let recurrence = self.recurrence.as_ref().map(|r| {
                    let week_days: Value = if self.weekdays.is_empty() { Value::Null } else { json!(self.weekdays.iter().map(|d| DAY_NAMES[*d as usize]).collect::<Vec<_>>()) };
                    json!({ "type": if r == "weekly" || !self.weekdays.is_empty() { "weeks" } else { "days" }, "period": 1, "week_days": week_days,
                        "until_date": self.recur_until.map(|u| u / 1000), "until_occurrences": null })
                });
                json!({
                    "message": if self.message.is_empty() { Value::Null } else { json!(self.message) },
                    "monitor_id": self.monitor_id,
                    "monitor_tags": if self.monitor_tags.is_empty() { json!(["*"]) } else { json!(self.monitor_tags) },
                    "scope": if self.scope.is_empty() { json!(["*"]) } else { json!(self.scope) },
                    "recurrence": recurrence,
                    "timezone": "UTC",
                    "mute_first_recovery_notification": false,
                    "notify_end_states": default_end_states(),
                    "notify_end_types": ["expired"],
                })
            }
        };
        let status = self.status(now);
        doc["id"] = json!(self.id);
        doc["uuid"] = json!(self.uuid);
        doc["org_id"] = json!(org_number(org));
        doc["start"] = json!(self.start_at / 1000);
        doc["end"] = json!(self.end_at.map(|e| e / 1000));
        doc["created"] = json!(self.created_at / 1000);
        doc["modified"] = json!(self.modified_at / 1000);
        doc["canceled"] = json!(self.canceled_at.map(|c| c / 1000));
        doc["active"] = json!(status == "active");
        doc["disabled"] = json!(self.canceled_at.is_some());
        doc["status"] = json!(status);
        doc["parent_id"] = Value::Null;
        doc["child_id"] = Value::Null;
        doc["creator_id"] = json!(self.creator_id.as_deref().map(user_number));
        doc["updater_id"] = json!(self.updater_id.as_deref().map(user_number));
        doc["downtime_type"] = json!(if self.monitor_id.is_some() || !self.monitor_tags.is_empty() { 0 } else { 2 });
        if with_creator {
            doc["creator"] = match &self.creator_id {
                Some(id) => json!({ "id": user_number(id), "email": self.creator_email, "handle": self.creator_email, "name": self.creator_name }),
                None => Value::Null,
            };
        }
        doc
    }

    /// The v2 attributes: as written through v2, else derived from the row.
    fn v2_attributes(&self) -> Value {
        if let Some(d) = self.stored("v2") {
            return d.clone();
        }
        let schedule = match &self.recurrence {
            Some(_) => json!({
                "recurrences": [{ "duration": duration_text(self.end_at.unwrap_or(self.start_at) - self.start_at), "rrule": rrule_text(&self.plan()),
                    "start": chrono::DateTime::from_timestamp_millis(self.start_at).map(|t| t.format("%Y-%m-%dT%H:%M").to_string()) }],
                "timezone": "UTC",
            }),
            None => json!({ "start": iso(self.start_at), "end": self.end_at.map(iso) }),
        };
        json!({
            "scope": scope_text(&self.scope),
            "monitor_identifier": match self.monitor_id {
                Some(id) => json!({ "monitor_id": id }),
                None => json!({ "monitor_tags": if self.monitor_tags.is_empty() { vec!["*".to_string()] } else { self.monitor_tags.clone() } }),
            },
            "message": if self.message.is_empty() { Value::Null } else { json!(self.message) },
            "schedule": schedule,
            "display_timezone": "UTC",
            "mute_first_recovery_notification": false,
            "notify_end_states": default_end_states(),
            "notify_end_types": ["expired"],
        })
    }

    /// The v2 `data` object.
    fn v2_data(&self, now: i64) -> Value {
        let mut attrs = self.v2_attributes();
        attrs["status"] = json!(self.status(now));
        attrs["created"] = json!(iso(self.created_at));
        attrs["modified"] = json!(iso(self.modified_at));
        attrs["canceled"] = json!(self.canceled_at.map(iso));
        if attrs["schedule"].get("recurrences").is_some() {
            if let Some((s, e)) = self.window().active_at(now) {
                attrs["schedule"]["current_downtime"] = json!({ "start": iso(s), "end": e.map(iso) });
            }
        } else {
            attrs["schedule"] = json!({ "start": iso(self.start_at), "end": self.end_at.map(iso) });
        }
        json!({
            "type": "downtime",
            "id": self.uuid,
            "attributes": attrs,
            "relationships": {
                "created_by": { "data": self.creator_id.as_ref().map(|id| json!({ "type": "users", "id": id })) },
                "monitor": { "data": self.monitor_id.map(|id| json!({ "type": "monitors", "id": id.to_string() })) },
            },
        })
    }

    /// The creator, as a v2 `users` resource for `included`.
    fn v2_creator(&self) -> Option<Value> {
        let id = self.creator_id.as_ref()?;
        let created = self.creator_created_at.map(iso);
        Some(json!({
            "type": "users",
            "id": id,
            "attributes": {
                "name": self.creator_name, "handle": self.creator_email, "email": self.creator_email, "created_at": created, "modified_at": created,
                "icon": "", "title": null, "verified": true, "service_account": false, "disabled": false, "allowed_login_methods": [], "status": "Active",
            },
        }))
    }

    fn v2_response(&self, now: i64) -> Value {
        json!({ "data": self.v2_data(now), "included": self.v2_creator().into_iter().collect::<Vec<_>>() })
    }
}

/// The org's downtimes (with canceled ones of the last 30 days when `include_canceled`).
pub(crate) async fn load_rows(st: &AppState, org: &str, include_canceled: bool) -> Result<Vec<Row>, DdError> {
    Ok(sqlx::query_as(&format!("{SELECT} WHERE d.org_id = $1 AND (d.canceled_at IS NULL OR ($2 AND d.canceled_at > now() - interval '30 days')) ORDER BY d.id"))
        .bind(org)
        .bind(include_canceled)
        .fetch_all(&st.postgres)
        .await?)
}

async fn load_by_id(st: &AppState, org: &str, id: i64) -> Result<Option<Row>, DdError> {
    Ok(sqlx::query_as(&format!("{SELECT} WHERE d.org_id = $1 AND d.id = $2")).bind(org).bind(id).fetch_optional(&st.postgres).await?)
}

async fn load_by_uuid(st: &AppState, org: &str, uuid: &str) -> Result<Option<Row>, DdError> {
    Ok(sqlx::query_as(&format!("{SELECT} WHERE d.org_id = $1 AND d.uuid = $2::uuid")).bind(org).bind(uuid).fetch_optional(&st.postgres).await?)
}

fn bad_request(message: impl Into<String>) -> DdError {
    DdError::new(StatusCode::BAD_REQUEST, message)
}

fn writer(ctx: &AuthContext) -> Result<(), DdError> {
    if ctx.access < Access::Standard {
        return Err(DdError::new(StatusCode::FORBIDDEN, "Forbidden: your role is read-only"));
    }
    Ok(())
}

fn json_body(body: &Bytes) -> Result<Value, DdError> {
    serde_json::from_slice(body).map_err(|e| bad_request(format!("Invalid JSON: {e}")))
}

fn is_uuid(s: &str) -> bool {
    s.len() == 36 && s.char_indices().all(|(i, c)| if [8, 13, 18, 23].contains(&i) { c == '-' } else { c.is_ascii_hexdigit() })
}

async fn check_monitor(st: &AppState, org: &str, plan: &Plan) -> Result<(), DdError> {
    if let Some(id) = plan.monitor_id {
        let exists: Option<(i64,)> = sqlx::query_as("SELECT id FROM monitors WHERE org_id = $1 AND id = $2").bind(org).bind(id).fetch_optional(&st.postgres).await?;
        if exists.is_none() {
            return Err(DdError::new(StatusCode::NOT_FOUND, format!("Monitor with id {id} not found")));
        }
    }
    Ok(())
}

fn seconds(ms: Option<i64>) -> Option<f64> {
    ms.map(|m| m as f64 / 1000.0)
}

async fn insert(st: &AppState, ctx: &AuthContext, plan: &Plan, doc: &Value, version: &str) -> Result<i64, DdError> {
    check_monitor(st, &ctx.org_id, plan).await?;
    let (id,): (i64,) = sqlx::query_as(
        "INSERT INTO downtimes (org_id, monitor_id, monitor_tags, scope, message, start_at, end_at, recurrence, recur_until, weekdays, created_by, datadog_document, datadog_version) \
         VALUES ($1, $2, $3, $4, $5, to_timestamp($6), to_timestamp($7), $8, to_timestamp($9), $10, $11::uuid, $12, $13) RETURNING id",
    )
    .bind(&ctx.org_id)
    .bind(plan.monitor_id)
    .bind(&plan.monitor_tags)
    .bind(&plan.scope)
    .bind(&plan.message)
    .bind(plan.start as f64 / 1000.0)
    .bind(seconds(plan.end))
    .bind(plan.recurrence)
    .bind(seconds(plan.recur_until))
    .bind(&plan.weekdays)
    .bind(&ctx.user_id)
    .bind(doc)
    .bind(version)
    .fetch_one(&st.postgres)
    .await?;
    ctx.audit(st, "downtime.create", &format!("downtime:{id}"), &plan.message).await;
    Ok(id)
}

async fn store_update(st: &AppState, ctx: &AuthContext, id: i64, plan: &Plan, doc: &Value, version: &str) -> Result<(), DdError> {
    check_monitor(st, &ctx.org_id, plan).await?;
    sqlx::query(
        "UPDATE downtimes SET monitor_id = $3, monitor_tags = $4, scope = $5, message = $6, start_at = to_timestamp($7), end_at = to_timestamp($8), recurrence = $9, \
         recur_until = to_timestamp($10), weekdays = $11, datadog_document = $12, datadog_version = $13, modified_at = now(), modified_by = $14::uuid \
         WHERE org_id = $1 AND id = $2",
    )
    .bind(&ctx.org_id)
    .bind(id)
    .bind(plan.monitor_id)
    .bind(&plan.monitor_tags)
    .bind(&plan.scope)
    .bind(&plan.message)
    .bind(plan.start as f64 / 1000.0)
    .bind(seconds(plan.end))
    .bind(plan.recurrence)
    .bind(seconds(plan.recur_until))
    .bind(&plan.weekdays)
    .bind(doc)
    .bind(version)
    .bind(&ctx.user_id)
    .execute(&st.postgres)
    .await?;
    ctx.audit(st, "downtime.update", &format!("downtime:{id}"), &plan.message).await;
    Ok(())
}

/// Cancels a downtime (again: no-op).
async fn cancel(st: &AppState, ctx: &AuthContext, id: i64) -> Result<(), DdError> {
    let r = sqlx::query("UPDATE downtimes SET canceled_at = now(), modified_at = now(), modified_by = $3::uuid WHERE org_id = $1 AND id = $2 AND canceled_at IS NULL")
        .bind(&ctx.org_id)
        .bind(id)
        .bind(&ctx.user_id)
        .execute(&st.postgres)
        .await?;
    if r.rows_affected() > 0 {
        ctx.audit(st, "downtime.cancel", &format!("downtime:{id}"), "").await;
    }
    Ok(())
}

// ================================================================ v1 handlers

fn v1_not_found() -> DdError {
    DdError::new(StatusCode::NOT_FOUND, "Downtime not found")
}

fn v1_id(s: &str) -> Result<i64, DdError> {
    s.parse().map_err(|_| v1_not_found())
}

/// Schedule a downtime (Datadog v1).
#[utoipa::path(post, path = "/api/v1/downtime", operation_id = "datadog_create_downtime_v1", request_body = Object,
    responses((status = 200, body = Object), (status = 400, body = super::DatadogErrors)), tag = "datadog-api")]
pub async fn v1_create(State(st): State<AppState>, DdAuth(ctx): DdAuth, body: Bytes) -> Result<Json<Value>, DdError> {
    writer(&ctx)?;
    let now = now_ms();
    let doc = v1_normalize(json_body(&body)?, None).map_err(bad_request)?;
    let plan = v1_plan(&doc, now).map_err(bad_request)?;
    let id = insert(&st, &ctx, &plan, &doc, "v1").await?;
    let row = load_by_id(&st, &ctx.org_id, id).await?.ok_or_else(v1_not_found)?;
    Ok(Json(row.v1_document(&ctx.org_id, now_ms(), false)))
}

/// A downtime (Datadog v1).
#[utoipa::path(get, path = "/api/v1/downtime/{downtime_id}", operation_id = "datadog_get_downtime_v1", params(("downtime_id" = i64, Path)),
    responses((status = 200, body = Object), (status = 404, body = super::DatadogErrors)), tag = "datadog-api")]
pub async fn v1_get(State(st): State<AppState>, DdAuth(ctx): DdAuth, Path(id): Path<String>) -> Result<Json<Value>, DdError> {
    let row = load_by_id(&st, &ctx.org_id, v1_id(&id)?).await?.ok_or_else(v1_not_found)?;
    Ok(Json(row.v1_document(&ctx.org_id, now_ms(), false)))
}

/// The org's downtimes (Datadog v1): `current_only`, `with_creator`.
#[utoipa::path(get, path = "/api/v1/downtime", operation_id = "datadog_list_downtimes_v1", params(("current_only" = Option<bool>, Query), ("with_creator" = Option<bool>, Query)),
    responses((status = 200, body = Vec<Object>)), tag = "datadog-api")]
pub async fn v1_list(State(st): State<AppState>, DdAuth(ctx): DdAuth, Query(params): Query<HashMap<String, String>>) -> Result<Json<Vec<Value>>, DdError> {
    let now = now_ms();
    let current_only = params.get("current_only").is_some_and(|v| v == "true");
    let with_creator = params.get("with_creator").is_some_and(|v| v == "true");
    let rows = load_rows(&st, &ctx.org_id, false).await?;
    Ok(Json(rows.iter().filter(|r| !current_only || r.status(now) == "active").map(|r| r.v1_document(&ctx.org_id, now, with_creator)).collect()))
}

/// Update a downtime (Datadog v1): the fields sent replace the stored ones.
#[utoipa::path(put, path = "/api/v1/downtime/{downtime_id}", operation_id = "datadog_update_downtime_v1", params(("downtime_id" = i64, Path)), request_body = Object,
    responses((status = 200, body = Object), (status = 400, body = super::DatadogErrors), (status = 404, body = super::DatadogErrors)), tag = "datadog-api")]
pub async fn v1_update(State(st): State<AppState>, DdAuth(ctx): DdAuth, Path(id): Path<String>, body: Bytes) -> Result<Json<Value>, DdError> {
    writer(&ctx)?;
    let id = v1_id(&id)?;
    let row = load_by_id(&st, &ctx.org_id, id).await?.ok_or_else(v1_not_found)?;
    let stored = row.v1_document(&ctx.org_id, now_ms(), false);
    let doc = v1_normalize(json_body(&body)?, Some(stored)).map_err(bad_request)?;
    let plan = v1_plan(&doc, row.start_at).map_err(bad_request)?;
    let doc = v1_normalize(doc, None).map_err(bad_request)?;
    store_update(&st, &ctx, id, &plan, &doc, "v1").await?;
    let row = load_by_id(&st, &ctx.org_id, id).await?.ok_or_else(v1_not_found)?;
    Ok(Json(row.v1_document(&ctx.org_id, now_ms(), false)))
}

/// Cancel a downtime (Datadog v1).
#[utoipa::path(delete, path = "/api/v1/downtime/{downtime_id}", operation_id = "datadog_cancel_downtime_v1", params(("downtime_id" = i64, Path)),
    responses((status = 204), (status = 404, body = super::DatadogErrors)), tag = "datadog-api")]
pub async fn v1_cancel(State(st): State<AppState>, DdAuth(ctx): DdAuth, Path(id): Path<String>) -> Result<StatusCode, DdError> {
    writer(&ctx)?;
    let id = v1_id(&id)?;
    load_by_id(&st, &ctx.org_id, id).await?.ok_or_else(v1_not_found)?;
    cancel(&st, &ctx, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Cancel every downtime with exactly this scope (Datadog v1).
#[utoipa::path(post, path = "/api/v1/downtime/cancel/by_scope", operation_id = "datadog_cancel_downtimes_by_scope_v1", request_body = Object,
    responses((status = 200, body = Object), (status = 404, body = super::DatadogErrors)), tag = "datadog-api")]
pub async fn v1_cancel_by_scope(State(st): State<AppState>, DdAuth(ctx): DdAuth, body: Bytes) -> Result<Json<Value>, DdError> {
    writer(&ctx)?;
    let body = json_body(&body)?;
    let scope = body.get("scope").and_then(Value::as_str).ok_or_else(|| bad_request("scope is required"))?;
    let wanted = tag_scope(scope.split(',').map(|s| s.trim().to_string()).collect()).map_err(bad_request)?;
    let rows = load_rows(&st, &ctx.org_id, false).await?;
    let mut canceled = Vec::new();
    for r in rows.iter().filter(|r| r.scope == wanted) {
        cancel(&st, &ctx, r.id).await?;
        canceled.push(r.id);
    }
    if canceled.is_empty() {
        return Err(DdError::new(StatusCode::NOT_FOUND, "No downtimes found"));
    }
    Ok(Json(json!({ "cancelled_ids": canceled })))
}

// ================================================================ v2 handlers

fn v2_not_found(id: &str) -> DdError {
    DdError::new(StatusCode::NOT_FOUND, format!("Downtime {id} not found"))
}

async fn v2_row(st: &AppState, org: &str, id: &str) -> Result<Row, DdError> {
    if !is_uuid(id) {
        return Err(bad_request("Invalid URL param: downtime_id must be an uuid"));
    }
    load_by_uuid(st, org, id).await?.ok_or_else(|| v2_not_found(id))
}

fn v2_attributes_of(body: &Value) -> Result<&Value, DdError> {
    let data = body.get("data").filter(|d| d.is_object()).ok_or_else(|| bad_request("data is required"))?;
    if data.get("type").and_then(Value::as_str).is_some_and(|t| t != "downtime") {
        return Err(bad_request("data.type must be 'downtime'"));
    }
    data.get("attributes").filter(|a| a.is_object()).ok_or_else(|| bad_request("data.attributes is required"))
}

/// Schedule a downtime (Datadog v2).
#[utoipa::path(post, path = "/api/v2/downtime", operation_id = "datadog_create_downtime", request_body = Object,
    responses((status = 200, body = Object), (status = 400, body = super::DatadogErrors)), tag = "datadog-api")]
pub async fn v2_create(State(st): State<AppState>, DdAuth(ctx): DdAuth, body: Bytes) -> Result<Json<Value>, DdError> {
    writer(&ctx)?;
    let body = json_body(&body)?;
    let now = now_ms();
    let doc = v2_normalize(v2_attributes_of(&body)?, None, now).map_err(bad_request)?;
    let plan = v2_plan(&doc).map_err(bad_request)?;
    let id = insert(&st, &ctx, &plan, &doc, "v2").await?;
    let row = load_by_id(&st, &ctx.org_id, id).await?.ok_or_else(v1_not_found)?;
    Ok(Json(row.v2_response(now_ms())))
}

/// A downtime (Datadog v2).
#[utoipa::path(get, path = "/api/v2/downtime/{downtime_id}", operation_id = "datadog_get_downtime", params(("downtime_id" = String, Path)),
    responses((status = 200, body = Object), (status = 400, body = super::DatadogErrors), (status = 404, body = super::DatadogErrors)), tag = "datadog-api")]
pub async fn v2_get(State(st): State<AppState>, DdAuth(ctx): DdAuth, Path(id): Path<String>) -> Result<Json<Value>, DdError> {
    Ok(Json(v2_row(&st, &ctx.org_id, &id).await?.v2_response(now_ms())))
}

/// The org's downtimes (Datadog v2): `current_only`, `page[offset]`, `page[limit]`.
#[utoipa::path(get, path = "/api/v2/downtime", operation_id = "datadog_list_downtimes",
    params(("current_only" = Option<bool>, Query), ("page[offset]" = Option<i64>, Query), ("page[limit]" = Option<i64>, Query)),
    responses((status = 200, body = Object)), tag = "datadog-api")]
pub async fn v2_list(State(st): State<AppState>, DdAuth(ctx): DdAuth, Query(params): Query<HashMap<String, String>>) -> Result<Json<Value>, DdError> {
    let now = now_ms();
    let current_only = params.get("current_only").is_some_and(|v| v == "true");
    let offset = params.get("page[offset]").and_then(|v| v.parse::<usize>().ok()).unwrap_or(0);
    let limit = params.get("page[limit]").and_then(|v| v.parse::<usize>().ok()).unwrap_or(30).clamp(1, 1000);
    let rows = load_rows(&st, &ctx.org_id, false).await?;
    let matched: Vec<&Row> = rows.iter().filter(|r| !current_only || r.status(now) == "active").collect();
    let data: Vec<Value> = matched.iter().skip(offset).take(limit).map(|r| r.v2_data(now)).collect();
    Ok(Json(json!({ "data": data, "meta": { "page": { "total_filtered_count": matched.len() } } })))
}

/// Update a downtime (Datadog v2): the attributes sent replace the stored ones.
#[utoipa::path(patch, path = "/api/v2/downtime/{downtime_id}", operation_id = "datadog_update_downtime", params(("downtime_id" = String, Path)), request_body = Object,
    responses((status = 200, body = Object), (status = 400, body = super::DatadogErrors), (status = 404, body = super::DatadogErrors)), tag = "datadog-api")]
pub async fn v2_update(State(st): State<AppState>, DdAuth(ctx): DdAuth, Path(id): Path<String>, body: Bytes) -> Result<Json<Value>, DdError> {
    writer(&ctx)?;
    let row = v2_row(&st, &ctx.org_id, &id).await?;
    let body = json_body(&body)?;
    let mut stored = row.v2_attributes();
    if stored["schedule"].get("recurrences").is_none() {
        stored["schedule"] = json!({ "start": iso(row.start_at), "end": row.end_at.map(iso) });
    }
    let doc = v2_normalize(v2_attributes_of(&body)?, Some(&stored), now_ms()).map_err(bad_request)?;
    let plan = v2_plan(&doc).map_err(bad_request)?;
    store_update(&st, &ctx, row.id, &plan, &doc, "v2").await?;
    let row = load_by_id(&st, &ctx.org_id, row.id).await?.ok_or_else(|| v2_not_found(&id))?;
    Ok(Json(row.v2_response(now_ms())))
}

/// Cancel a downtime (Datadog v2).
#[utoipa::path(delete, path = "/api/v2/downtime/{downtime_id}", operation_id = "datadog_cancel_downtime", params(("downtime_id" = String, Path)),
    responses((status = 204), (status = 400, body = super::DatadogErrors), (status = 404, body = super::DatadogErrors)), tag = "datadog-api")]
pub async fn v2_cancel(State(st): State<AppState>, DdAuth(ctx): DdAuth, Path(id): Path<String>) -> Result<Response, DdError> {
    writer(&ctx)?;
    let row = v2_row(&st, &ctx.org_id, &id).await?;
    cancel(&st, &ctx, row.id).await?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

/// The active downtimes that cover a monitor (Datadog v2).
#[utoipa::path(get, path = "/api/v2/monitor/{monitor_id}/downtime_matches", operation_id = "datadog_list_monitor_downtimes", params(("monitor_id" = i64, Path)),
    responses((status = 200, body = Object), (status = 404, body = super::DatadogErrors)), tag = "datadog-api")]
pub async fn v2_monitor_matches(State(st): State<AppState>, DdAuth(ctx): DdAuth, Path(id): Path<String>) -> Result<Json<Value>, DdError> {
    let not_found = || DdError::new(StatusCode::NOT_FOUND, format!("Monitor with id {id} not found"));
    let monitor_id: i64 = id.parse().map_err(|_| not_found())?;
    let tags: Option<(Vec<String>,)> = sqlx::query_as("SELECT tags FROM monitors WHERE org_id = $1 AND id = $2").bind(&ctx.org_id).bind(monitor_id).fetch_optional(&st.postgres).await?;
    let (tags,) = tags.ok_or_else(not_found)?;
    let now = now_ms();
    let rows = load_rows(&st, &ctx.org_id, false).await?;
    let data: Vec<Value> = rows
        .iter()
        .filter(|r| r.applies_to(monitor_id, &tags))
        .filter_map(|r| r.window().active_at(now).map(|(s, e)| (r, s, e)))
        .map(|(r, s, e)| json!({ "type": "downtime_match", "id": r.uuid, "attributes": { "groups": ["*"], "scope": scope_text(&r.scope), "start": iso(s), "end": e.map(iso) } }))
        .collect();
    Ok(Json(json!({ "data": data, "meta": { "page": { "total_filtered_count": data.len() } } })))
}

#[cfg(test)]
mod tests {
    use super::*;

    const START: i64 = 1_767_225_600_000; // 2026-01-01T00:00:00Z, a Thursday

    #[test]
    fn v1_documents_map_onto_downtimes() {
        let doc = v1_normalize(
            json!({ "scope": ["host:i-0a1b2c3d4e5f60001", "env:prod"], "monitor_tags": ["service:billing"], "start": START / 1000, "end": START / 1000 + 3600, "message": " Deploy @slack-ops " }),
            None,
        )
        .unwrap();
        let p = v1_plan(&doc, 0).unwrap();
        assert_eq!(p.scope, vec!["host:i-0a1b2c3d4e5f60001", "env:prod"]);
        assert_eq!(p.monitor_tags, vec!["service:billing"]);
        assert_eq!((p.start, p.end, p.message.as_str(), p.recurrence), (START, Some(START + 3_600_000), "Deploy @slack-ops", None));
        assert_eq!(doc["timezone"], "UTC");
        assert_eq!(doc["notify_end_types"], json!(["expired"]));

        let all = v1_plan(&v1_normalize(json!({ "scope": ["*"], "monitor_id": 7 }), None).unwrap(), START).unwrap();
        assert_eq!((all.scope.len(), all.monitor_id, all.start), (0, Some(7), START));

        // Weekdays every week: a daily recurrence on those days.
        let weekly = json!({ "scope": ["*"], "start": START / 1000, "end": START / 1000 + 3600, "recurrence": { "type": "weeks", "period": 1, "week_days": ["Mon", "Tue", "Wed", "Thu", "Fri"], "until_occurrences": 3 } });
        let p = v1_plan(&weekly, 0).unwrap();
        assert_eq!((p.recurrence, p.weekdays.clone()), (Some("daily"), vec![0, 1, 2, 3, 4]));
        assert_eq!(p.recur_until, Some(START + 4 * DAY_MS), "Thu, Fri, then Monday");
        let p = v1_plan(&json!({ "scope": ["*"], "start": START / 1000, "end": START / 1000 + 3600, "recurrence": { "type": "weeks", "period": 1, "until_date": START / 1000 + 86_400 * 30 } }), 0)
            .unwrap();
        assert_eq!((p.recurrence, p.recur_until), (Some("weekly"), Some(START + 30 * DAY_MS)));
        // A weekday in another timezone: Monday 23:00 in New York is Tuesday 04:00 UTC.
        let ny = json!({ "scope": ["*"], "timezone": "America/New_York", "start": (START + 5 * DAY_MS + 4 * 3_600_000) / 1000, "end": (START + 5 * DAY_MS + 5 * 3_600_000) / 1000,
            "recurrence": { "type": "weeks", "week_days": ["Mon"] } });
        assert_eq!(v1_plan(&ny, 0).unwrap().weekdays, vec![1]);
    }

    #[test]
    fn v1_documents_we_cant_represent_are_rejected() {
        let reject = |v: Value| v1_plan(&v1_normalize(v, None).unwrap(), START).unwrap_err();
        assert_eq!(reject(json!({ "monitor_tags": ["a"] })), "Invalid scope parameter");
        let base = |r: Value| json!({ "scope": ["*"], "start": START / 1000, "end": START / 1000 + 3600, "recurrence": r });
        assert_eq!(reject(base(json!({ "type": "hours", "period": 1 }))), "Invalid recurrence type");
        assert!(reject(base(json!({ "type": "years", "period": 1 }))).contains("'years' is not supported"));
        assert!(reject(base(json!({ "type": "days", "period": 2 }))).contains("period of 2"));
        assert!(reject(base(json!({ "type": "weeks", "week_days": ["mon"] }))).starts_with("Invalid week_days"));
        assert!(reject(base(json!({ "type": "weeks", "until_date": 1, "until_occurrences": 3 }))).contains("only one of"));
        assert!(reject(json!({ "scope": ["*"], "start": START / 1000, "recurrence": { "type": "days" } })).contains("needs an end"));
        assert!(reject(json!({ "scope": ["host"] })).contains("key:value"));
    }

    #[test]
    fn v2_scopes() {
        assert_eq!(parse_scope("*").unwrap(), Vec::<String>::new());
        assert_eq!(parse_scope("env:prod AND host:\"web store 1\"").unwrap(), vec!["env:prod", "host:web store 1"]);
        assert_eq!(parse_scope("env:prod service:billing").unwrap(), vec!["env:prod", "service:billing"]);
        assert!(parse_scope("BAD_SCOPE_MISSING_KEY_VALUE_FORMAT").unwrap_err().starts_with("All values must have a key"));
        for s in ["env:prod OR env:staging", "NOT env:prod", "-env:prod", "host:web-*", "(env:prod)"] {
            assert!(parse_scope(s).unwrap_err().contains("not supported"), "{s}");
        }
        assert_eq!(scope_text(&["env:prod".into(), "host:web store 1".into()]), "env:prod AND host:\"web store 1\"");
        assert_eq!(parse_scope(&scope_text(&["env:prod".into(), "host:web store 1".into()])).unwrap(), vec!["env:prod", "host:web store 1"]);
    }

    #[test]
    fn v2_documents_map_onto_downtimes() {
        let now = START;
        let attrs = v2_normalize(&json!({ "scope": "env:prod", "monitor_identifier": { "monitor_tags": ["*"] }, "schedule": { "start": null } }), None, now).unwrap();
        assert_eq!(attrs["schedule"], json!({ "start": iso(now), "end": null }));
        assert_eq!((attrs["display_timezone"].as_str(), attrs["mute_first_recovery_notification"].as_bool()), (Some("UTC"), Some(false)));
        let p = v2_plan(&attrs).unwrap();
        assert_eq!((p.scope.clone(), p.monitor_tags.len(), p.start, p.end), (vec!["env:prod".to_string()], 0, now, None));

        let one = json!({ "scope": "*", "monitor_identifier": { "monitor_id": 12 }, "message": "maintenance", "schedule": { "start": "2026-01-01T00:00:00Z", "end": "2026-01-01T02:00:00Z" } });
        let p = v2_plan(&v2_normalize(&one, None, now).unwrap()).unwrap();
        assert_eq!((p.monitor_id, p.start, p.end, p.message.as_str()), (Some(12), START, Some(START + 7_200_000), "maintenance"));

        let rec = json!({ "scope": "*", "monitor_identifier": { "monitor_tags": ["team:payments"] },
            "schedule": { "timezone": "Europe/Paris", "recurrences": [{ "duration": "2h", "rrule": "FREQ=WEEKLY;BYDAY=MO,TU,WE,TH,FR;COUNT=2", "start": "2026-01-01T01:00:00" }] } });
        let p = v2_plan(&v2_normalize(&rec, None, now).unwrap()).unwrap();
        // 01:00 in Paris is midnight UTC in winter.
        assert_eq!((p.start, p.end, p.recurrence, p.weekdays.clone()), (START, Some(START + 7_200_000), Some("daily"), vec![0, 1, 2, 3, 4]));
        assert_eq!(p.recur_until, Some(START + DAY_MS), "Thursday and Friday");
        let p = v2_plan(
            &v2_normalize(
                &json!({ "scope": "*", "monitor_identifier": { "monitor_tags": ["*"] }, "schedule": { "recurrences": [{ "duration": "1h", "rrule": "FREQ=DAILY;UNTIL=20260110T000000Z" }] } }),
                None,
                now,
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!((p.start, p.recurrence, p.recur_until), (START, Some("daily"), Some(START + 9 * DAY_MS)));

        // An update keeps the stored recurrence start when the client sends none.
        let stored = v2_normalize(&rec, None, now).unwrap();
        let updated =
            v2_normalize(&json!({ "schedule": { "timezone": "Europe/Paris", "recurrences": [{ "duration": "3h", "rrule": "FREQ=DAILY", "start": null }] } }), Some(&stored), now + DAY_MS).unwrap();
        assert_eq!(updated["schedule"]["recurrences"][0]["start"], "2026-01-01T01:00:00");
        assert_eq!(updated["monitor_identifier"], json!({ "monitor_tags": ["team:payments"] }));
    }

    #[test]
    fn v2_documents_we_cant_represent_are_rejected() {
        let reject = |v: Value| v2_normalize(&v, None, START).and_then(|d| v2_plan(&d)).unwrap_err();
        assert_eq!(reject(json!({ "invalid_field": "x" })), "Additional properties are not allowed ('invalid_field' was unexpected)");
        let rec = |rrule: &str| json!({ "scope": "*", "monitor_identifier": { "monitor_tags": ["*"] }, "schedule": { "recurrences": [{ "duration": "1h", "rrule": rrule }] } });
        assert!(reject(rec("FREQ=MONTHLY;BYMONTHDAY=1")).contains("FREQ=MONTHLY"));
        assert!(reject(rec("FREQ=DAILY;INTERVAL=2")).contains("INTERVAL=2"));
        assert!(reject(rec("FREQ=WEEKLY;BYHOUR=3")).contains("BYHOUR"));
        assert!(reject(json!({ "scope": "*", "monitor_identifier": { "monitor_tags": ["*"] }, "schedule": { "recurrences": [{ "duration": "1h", "rrule": "FREQ=DAILY" }, { "duration": "1h", "rrule": "FREQ=WEEKLY" }] } })).contains("only one recurrence"));
        assert!(reject(json!({ "scope": "*", "monitor_identifier": { "monitor_tags": ["*"] }, "schedule": { "recurrences": [{ "duration": "25h", "rrule": "FREQ=DAILY" }] } }))
            .contains("shorter than the repeat period"));
        assert!(reject(json!({ "scope": "*", "monitor_identifier": {} })).contains("monitor_id or monitor_tags"));
    }

    #[test]
    fn rows_read_back_in_both_versions() {
        let row = Row {
            id: 3,
            uuid: "0a1b2c3d-0000-4000-8000-000000000003".into(),
            monitor_id: None,
            monitor_tags: vec!["team:payments".into()],
            scope: vec!["env:prod".into()],
            message: "Weekday maintenance".into(),
            start_at: START,
            end_at: Some(START + 7_200_000),
            recurrence: Some("daily".into()),
            recur_until: None,
            weekdays: vec![0, 1, 2, 3, 4],
            creator_id: None,
            creator_email: None,
            creator_name: None,
            creator_created_at: None,
            updater_id: None,
            created_at: START,
            modified_at: START,
            canceled_at: None,
            datadog_document: None,
            datadog_version: None,
        };
        let v1 = row.v1_document("default", START + 60_000, false);
        assert_eq!(v1["recurrence"]["type"], "weeks");
        assert_eq!(v1["recurrence"]["week_days"], json!(["Mon", "Tue", "Wed", "Thu", "Fri"]));
        assert_eq!((v1["status"].as_str(), v1["active"].as_bool()), (Some("active"), Some(true)));
        assert_eq!(v1_plan(&v1_normalize(v1, None).unwrap(), 0).unwrap(), row.plan());
        let v2 = row.v2_data(START + 60_000);
        assert_eq!(v2["attributes"]["schedule"]["recurrences"][0], json!({ "duration": "2h", "rrule": "FREQ=WEEKLY;BYDAY=MO,TU,WE,TH,FR", "start": "2026-01-01T00:00" }));
        assert_eq!(v2["attributes"]["schedule"]["current_downtime"]["start"], iso(START));
        let attrs: Map<String, Value> = v2["attributes"].as_object().unwrap().iter().filter(|(k, _)| V2_ATTRIBUTES.contains(&k.as_str())).map(|(k, v)| (k.clone(), v.clone())).collect();
        assert_eq!(v2_plan(&Value::Object(attrs)).unwrap(), row.plan());
        let canceled = Row { canceled_at: Some(START), ..row };
        assert_eq!(canceled.v2_data(START)["attributes"]["status"], "canceled");
    }
}

/// Against the live test databases (skipped without them).
#[cfg(test)]
mod live_tests {
    use super::super::monitors::live_tests::Client;
    use super::super::monitors::replay_feature;
    use super::super::replay::{new_org_with_roles, user_with_app_key, TestServer};
    use reqwest::{Method, StatusCode};
    use serde_json::{json, Value};

    #[tokio::test]
    async fn v1_downtimes_cassettes_replay() {
        let unsupported = [
            ("Schedule a downtime once a year", "yearly recurrences are not supported"),
            ("Get active downtimes for a monitor returns \"OK\" response", "reads a monitor that only exists in the recording's org"),
        ];
        let Some(replayed) = replay_feature("dd-downtimes-v1-cassettes", "v1/downtimes", &unsupported).await else { return };
        assert_eq!(replayed.len(), 16, "{replayed:#?}");
    }

    #[tokio::test]
    async fn v2_downtimes_cassettes_replay() {
        let unsupported = [("Get active downtimes for a monitor returns \"OK\" response", "reads a monitor that only exists in the recording's org")];
        let Some(replayed) = replay_feature("dd-downtimes-v2-cassettes", "v2/downtimes", &unsupported).await else { return };
        assert_eq!(replayed.len(), 13, "{replayed:#?}");
    }

    /// A downtime as the provider's `datadog_downtime_schedule` sends it.
    fn schedule(monitor_id: i64) -> Value {
        json!({ "data": { "type": "downtime", "attributes": {
            "scope": "env:prod AND host:i-0a1b2c3d4e5f60001",
            "monitor_identifier": { "monitor_id": monitor_id },
            "message": "Deploying storefront-api @slack-ops",
            "display_timezone": "",
            "mute_first_recovery_notification": false,
            "schedule": { "start": "2030-01-01T00:00:00Z", "end": "2030-01-01T02:00:00Z" },
        } } })
    }

    #[tokio::test]
    async fn downtimes_land_in_our_downtimes_and_read_in_both_versions() {
        let Some(s) = TestServer::start("dd-downtimes").await else { return };
        let c = Client::new(&s, &s.app_key);
        let monitor = json!({ "name": "CPU", "type": "query alert", "query": "avg(last_5m):avg:system.cpu.user{*} by {host} > 90" });
        let (_, m) = c.send(Method::POST, "/api/v1/monitor", Some(monitor)).await;
        let monitor_id = m["id"].as_i64().unwrap();

        let (status, created) = c.send(Method::POST, "/api/v2/downtime", Some(schedule(monitor_id))).await;
        assert_eq!(status, StatusCode::OK, "{created}");
        let uuid = created["data"]["id"].as_str().unwrap().to_string();
        let a = &created["data"]["attributes"];
        assert_eq!((a["scope"].as_str(), a["message"].as_str(), a["status"].as_str()), (Some("env:prod AND host:i-0a1b2c3d4e5f60001"), Some("Deploying storefront-api @slack-ops"), Some("scheduled")));
        assert_eq!(a["monitor_identifier"], json!({ "monitor_id": monitor_id }));
        assert_eq!(a["schedule"], json!({ "start": "2030-01-01T00:00:00.000000+00:00", "end": "2030-01-01T02:00:00.000000+00:00" }));
        assert_eq!((a["display_timezone"].as_str(), a["notify_end_types"].clone()), (Some(""), json!(["expired"])));
        assert_eq!(created["data"]["relationships"]["monitor"]["data"], json!({ "type": "monitors", "id": monitor_id.to_string() }));
        assert_eq!(c.send(Method::GET, &format!("/api/v2/downtime/{uuid}"), None).await.1["data"]["attributes"], created["data"]["attributes"]);

        // Our own API lists it, mapped.
        let (_, ours) = c.ours(Method::POST, "/api/v1/downtimes/search", Some(json!({}))).await;
        let d = &ours["downtimes"][0];
        assert_eq!((d["monitorId"].as_i64(), d["scope"].clone(), d["status"].as_str()), (Some(monitor_id), json!(["env:prod", "host:i-0a1b2c3d4e5f60001"]), Some("scheduled")));
        assert_eq!(d["start"].as_i64(), Some(1_893_456_000_000));
        let v1_id = d["id"].as_i64().unwrap();

        // The same downtime through v1.
        let (_, v1) = c.send(Method::GET, &format!("/api/v1/downtime/{v1_id}"), None).await;
        assert_eq!((v1["monitor_id"].as_i64(), v1["start"].as_i64(), v1["end"].as_i64()), (Some(monitor_id), Some(1_893_456_000), Some(1_893_463_200)));
        assert_eq!(v1["scope"], json!(["env:prod", "host:i-0a1b2c3d4e5f60001"]));

        // Updates: a recurrence on weekdays.
        let patch = json!({ "data": { "type": "downtime", "id": uuid, "attributes": { "message": null,
            "schedule": { "timezone": "UTC", "recurrences": [{ "duration": "1h", "rrule": "FREQ=WEEKLY;BYDAY=MO,TU,WE,TH,FR", "start": "2030-01-01T22:00:00" }] } } } });
        let (status, updated) = c.send(Method::PATCH, &format!("/api/v2/downtime/{uuid}"), Some(patch)).await;
        assert_eq!(status, StatusCode::OK, "{updated}");
        assert_eq!(updated["data"]["attributes"]["schedule"]["recurrences"][0]["start"], "2030-01-01T22:00:00");
        assert!(updated["data"]["attributes"]["message"].is_null());
        let (_, v1) = c.send(Method::GET, &format!("/api/v1/downtime/{v1_id}"), None).await;
        assert_eq!(v1["recurrence"]["week_days"], json!(["Mon", "Tue", "Wed", "Thu", "Fri"]));

        // Read-only users and other orgs.
        let (_, reader_key) = user_with_app_key(&s.st, &s.org, "readonly").await;
        assert_eq!(Client::new(&s, &reader_key).send(Method::DELETE, &format!("/api/v2/downtime/{uuid}"), None).await.0, StatusCode::FORBIDDEN);
        let other_org = new_org_with_roles(&s.st, "dd-downtimes-other").await;
        let (_, other_key) = user_with_app_key(&s.st, &other_org, "admin").await;
        let other = Client::new(&s, &other_key);
        assert_eq!(other.send(Method::GET, &format!("/api/v2/downtime/{uuid}"), None).await.0, StatusCode::NOT_FOUND);
        assert_eq!(other.send(Method::DELETE, &format!("/api/v1/downtime/{v1_id}"), None).await.0, StatusCode::NOT_FOUND);
        assert_eq!(other.send(Method::POST, "/api/v2/downtime", Some(schedule(monitor_id))).await.0, StatusCode::NOT_FOUND, "another org's monitor");

        // Cancel (twice is fine), as the provider's destroy does.
        assert_eq!(c.send(Method::DELETE, &format!("/api/v2/downtime/{uuid}"), None).await.0, StatusCode::NO_CONTENT);
        assert_eq!(c.send(Method::DELETE, &format!("/api/v2/downtime/{uuid}"), None).await.0, StatusCode::NO_CONTENT);
        let (_, canceled) = c.send(Method::GET, &format!("/api/v2/downtime/{uuid}"), None).await;
        assert_eq!(canceled["data"]["attributes"]["status"], "canceled");
        assert!(canceled["data"]["attributes"]["canceled"].is_string());
        let (_, ours) = c.ours(Method::POST, "/api/v1/downtimes/search", Some(json!({}))).await;
        assert_eq!(ours["total"], 0);
    }
}
