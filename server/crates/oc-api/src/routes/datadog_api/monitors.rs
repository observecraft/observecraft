//! `/api/v1/monitor`: Datadog's monitor documents on our monitors (the `datadog_monitor` and
//! `datadog_monitor_json` resources, the `datadog_monitor(s)` data sources and the API clients).
//!
//! A monitor written here is stored twice in the same row: our definition (type, query, options,
//! message, priority, tags, draft), which the evaluator runs and the UI edits, and the Datadog
//! document as received (`datadog_document`), so reads return what was sent. Reads put our row's
//! values back into the document wherever they differ from what the document maps to, so an edit
//! in the UI shows up in the document (and in `terraform plan`) as it would on Datadog.
//!
//! Mapping ([`from_datadog`] and [`to_datadog`]):
//! - `metric alert` / `query alert` → a metric monitor (`apm` when every metric is `trace.*`):
//!   `avg|sum|min|max|last(last_5m)`, `change(…,last_1h)` and `pct_change(…)` heads, arithmetic
//!   between metric queries, `>`, `>=`, `<`, `<=`.
//! - `log alert` → a log monitor: `logs("<search>")[.index(…)].rollup("count")[.by("a,b")].last("5m")`.
//! - thresholds `critical` / `warning` / `critical_recovery` / `warning_recovery`, `notify_no_data`
//!   (or `on_missing_data: show_and_notify_no_data`), `no_data_timeframe`, `name`, `message` (its
//!   `@handles` are the recipients), `priority`, `tags`, `draft_status`.
//! - Every other option is stored and returned but not acted on (listed in `docs/datadog-api.md`).
//!   Other monitor types are rejected with a 400 naming the type.

use std::collections::{BTreeMap, HashMap};

use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use oc_query::parse_metric_query;
use serde_json::{json, Map, Value};

use super::{DdAuth, DdError};
use crate::auth::{Access, AuthContext};
use crate::error::ApiError;
use crate::routes::downtimes::{load_downtimes, Downtime};
use crate::routes::logs::parse_log_monitor;
use crate::routes::metrics::TimeAggregator;
use crate::routes::monitor_engine::{fmt_num, minutes_label, ChangeMode, Comparator, Detection, MonitorOptions, MonitorStatus, Thresholds};
use crate::routes::monitor_formula::parse_formula;
use crate::routes::monitors::{self, MonitorInput, MonitorType, SYNTHETICS_MANAGED};
use crate::state::AppState;

const SUPPORTED_TYPES: &str = "metric alert, query alert, log alert";

/// Datadog monitor types we know but can't evaluate yet.
const UNSUPPORTED_TYPES: &[&str] = &[
    "composite",
    "service check",
    "event alert",
    "event-v2 alert",
    "process alert",
    "rum alert",
    "trace-analytics alert",
    "slo alert",
    "audit alert",
    "ci-pipelines alert",
    "ci-tests alert",
    "error-tracking alert",
    "database-monitoring alert",
    "network-performance alert",
    "network-path alert",
    "cost alert",
    "data-quality alert",
    "data-jobs alert",
    "llm-observability alert",
];

/// Fields the server owns: dropped from what clients send, filled in on every answer.
const SERVER_OWNED: &[&str] = &["id", "org_id", "created", "created_at", "modified", "creator", "deleted", "overall_state", "overall_state_modified", "multi", "matching_downtimes", "state"];

// ================================================================ mapping

/// What a Datadog monitor document means for our monitor.
#[derive(Debug, Clone, PartialEq)]
pub struct Mapped {
    pub kind: MonitorType,
    pub query: String,
    pub options: MonitorOptions,
    pub name: String,
    pub message: String,
    pub priority: Option<i16>,
    pub tags: Vec<String>,
    pub draft: bool,
}

impl Mapped {
    fn input(&self, teams: Vec<String>) -> MonitorInput {
        MonitorInput {
            name: self.name.clone(),
            r#type: self.kind,
            query: self.query.clone(),
            options: self.options.clone(),
            message: self.message.clone(),
            priority: self.priority,
            tags: self.tags.clone(),
            teams,
            draft: self.draft,
        }
    }

    fn family(&self) -> &'static str {
        family(self.kind)
    }
}

fn family(kind: MonitorType) -> &'static str {
    match kind {
        MonitorType::Metric | MonitorType::Apm => "metric",
        MonitorType::Log => "log",
        MonitorType::Synthetics => "synthetics",
    }
}

fn datadog_family(datadog_type: &str) -> &'static str {
    match datadog_type {
        "metric alert" | "query alert" => "metric",
        "log alert" => "log",
        "synthetics alert" => "synthetics",
        _ => "other",
    }
}

fn invalid_query(detail: &str) -> String {
    format!("The value provided for parameter 'query' is invalid: {detail}")
}

fn unsupported_type(t: &str) -> String {
    if t == "synthetics alert" {
        SYNTHETICS_MANAGED.to_string()
    } else if UNSUPPORTED_TYPES.contains(&t) {
        format!("monitor type '{t}' is not supported by ObserveCraft yet (supported: {SUPPORTED_TYPES})")
    } else {
        format!("Invalid monitor type '{t}' (supported: {SUPPORTED_TYPES})")
    }
}

fn api_message(e: ApiError) -> String {
    match e {
        ApiError::BadRequest(m) | ApiError::NotFound(m) => m,
        other => other.to_string(),
    }
}

/// Our monitor from a Datadog document; Err is the 400 message.
pub fn from_datadog(doc: &Value) -> Result<Mapped, String> {
    let obj = doc.as_object().ok_or("the monitor must be a JSON object")?;
    let datadog_type = obj.get("type").and_then(Value::as_str).ok_or("the monitor type is required")?;
    let query = obj.get("query").and_then(Value::as_str).ok_or("the monitor query is required")?;
    let empty = Map::new();
    let options = obj.get("options").and_then(Value::as_object).unwrap_or(&empty);
    if options.get("variables").is_some_and(|v| v.as_array().map_or(!v.is_null(), |a| !a.is_empty())) {
        return Err("options.variables (formula and function queries) are not supported by ObserveCraft yet".into());
    }
    let (kind, query, mut opts) = match datadog_type {
        "metric alert" | "query alert" => parse_metric_alert(query)?,
        "log alert" => parse_log_alert(query)?,
        other => return Err(unsupported_type(other)),
    };

    let thresholds = options.get("thresholds").and_then(Value::as_object).unwrap_or(&empty);
    let threshold = |key: &str| -> Result<Option<f64>, String> {
        match thresholds.get(key) {
            None | Some(Value::Null) => Ok(None),
            Some(v) => v.as_f64().map(Some).ok_or_else(|| format!("options.thresholds.{key} must be a number")),
        }
    };
    if let Some(critical) = threshold("critical")? {
        if critical != opts.thresholds.critical {
            return Err(invalid_query(&format!("alert threshold ({}) does not match that used in the query ({})", fmt_num(critical), fmt_num(opts.thresholds.critical))));
        }
    }
    opts.thresholds.warning = threshold("warning")?;
    opts.thresholds.critical_recovery = threshold("critical_recovery")?;
    opts.thresholds.warning_recovery = threshold("warning_recovery")?;
    let notify_no_data = match options.get("notify_no_data") {
        None | Some(Value::Null) => false,
        Some(v) => v.as_bool().ok_or("options.notify_no_data must be a boolean")?,
    };
    opts.notify_no_data = notify_no_data || options.get("on_missing_data").and_then(Value::as_str) == Some("show_and_notify_no_data");
    opts.no_data_minutes = match options.get("no_data_timeframe") {
        None | Some(Value::Null) => (2 * opts.window_minutes).clamp(1, 1440),
        Some(v) => v.as_i64().ok_or("options.no_data_timeframe must be a number of minutes")?,
    };

    let text = |key: &str| -> Result<String, String> {
        match obj.get(key) {
            None | Some(Value::Null) => Ok(String::new()),
            Some(Value::String(s)) => Ok(s.clone()),
            Some(_) => Err(format!("{key} must be a string")),
        }
    };
    let priority = match obj.get("priority") {
        None | Some(Value::Null) => None,
        Some(v) => Some(v.as_i64().filter(|p| (1..=5).contains(p)).ok_or("priority must be an integer from 1 to 5")? as i16),
    };
    let tags = match obj.get("tags") {
        None | Some(Value::Null) => vec![],
        Some(Value::Array(items)) => items.iter().map(|t| t.as_str().map(str::to_string).ok_or("tags must be strings")).collect::<Result<_, _>>()?,
        Some(_) => return Err("tags must be a list of strings".into()),
    };
    let draft = match obj.get("draft_status").and_then(Value::as_str) {
        None | Some("published") => false,
        Some("draft") => true,
        Some(other) => return Err(format!("draft_status must be 'draft' or 'published', not '{other}'")),
    };
    let mapped = Mapped { kind, query, options: opts, name: text("name")?, message: text("message")?, priority, tags, draft };
    let mut input = mapped.input(vec![]);
    monitors::validate(&mut input).map_err(api_message)?;
    Ok(Mapped {
        kind: input.r#type,
        query: input.query.trim().to_string(),
        options: input.options,
        name: input.name,
        message: input.message,
        priority: input.priority,
        tags: input.tags,
        draft: input.draft,
    })
}

/// `q` split into the part before the comparator, the comparator and the threshold.
fn split_threshold(q: &str) -> Result<(&str, Comparator, f64), String> {
    let q = q.trim();
    let start = q.rfind(|c: char| !(c.is_ascii_digit() || matches!(c, '.' | '-' | '+' | 'e' | 'E'))).map_or(0, |i| i + 1);
    let lhs = q[..start].trim_end();
    let comparator = [(">=", Comparator::AboveOrEqual), ("<=", Comparator::BelowOrEqual), (">", Comparator::Above), ("<", Comparator::Below)]
        .into_iter()
        .find(|(op, _)| lhs.ends_with(op))
        .map(|(op, c)| (op.len(), c));
    let Some((op_len, comparator)) = comparator else {
        return Err(invalid_query("invalid operator specified (end the query with >, >=, < or <= and a threshold)"));
    };
    let threshold: f64 = q[start..].parse().map_err(|_| invalid_query(&format!("invalid threshold '{}'", &q[start..])))?;
    Ok((lhs[..lhs.len() - op_len].trim_end(), comparator, threshold))
}

/// `5m`, `1h`, `2d`, `1w` → minutes.
fn duration_minutes(s: &str) -> Result<i64, String> {
    let s = s.trim().trim_matches('"');
    let (n, unit) = s.split_at(s.find(|c: char| !c.is_ascii_digit()).unwrap_or(s.len()));
    let n: i64 = n.parse().map_err(|_| invalid_query(&format!("invalid time window '{s}'")))?;
    let factor = match unit {
        "m" => 1,
        "h" => 60,
        "d" => 1440,
        "w" => 10_080,
        _ => return Err(invalid_query(&format!("invalid time window '{s}' (use minutes, hours, days or weeks: 5m, 1h, 1d, 1w)"))),
    };
    Ok(n * factor)
}

/// `last_5m` → 5.
fn window_minutes(s: &str) -> Result<i64, String> {
    let s = s.trim();
    if s.starts_with("current_") {
        return Err(invalid_query(&format!("calendar-aligned evaluation windows ({s}) are not supported by ObserveCraft yet; use last_<n><m|h|d|w>")));
    }
    duration_minutes(s.strip_prefix("last_").ok_or_else(|| invalid_query(&format!("invalid time window '{s}'")))?)
}

/// Byte index of the `)` closing the `(` at `open`.
fn closing_paren(s: &str, open: usize) -> Result<usize, String> {
    let mut depth = 0;
    for (i, c) in s[open..].char_indices() {
        match c {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return Ok(open + i);
                }
            }
            _ => {}
        }
    }
    Err(invalid_query("unbalanced parentheses"))
}

/// `avg(last_5m)` → (aggregator, minutes).
fn reducer(s: &str) -> Result<(TimeAggregator, i64), String> {
    let s = s.trim();
    let open = s.find('(').ok_or_else(|| invalid_query(&format!("expected a time aggregation such as avg(last_5m), got '{s}'")))?;
    let agg = match &s[..open] {
        "avg" => TimeAggregator::Avg,
        "sum" => TimeAggregator::Sum,
        "min" => TimeAggregator::Min,
        "max" => TimeAggregator::Max,
        "last" => TimeAggregator::Last,
        other => return Err(invalid_query(&format!("time aggregation '{other}' is not supported by ObserveCraft yet (avg, sum, min, max, last)"))),
    };
    let inner = s[open + 1..].strip_suffix(')').ok_or_else(|| invalid_query(&format!("invalid time aggregation '{s}'")))?;
    Ok((agg, window_minutes(inner)?))
}

/// `avg(last_5m):avg:system.load.1{*} by {host} > 2`, `change(avg(last_5m),last_1h):… > 10`.
fn parse_metric_alert(q: &str) -> Result<(MonitorType, String, MonitorOptions), String> {
    let (lhs, comparator, critical) = split_threshold(q)?;
    let open = lhs.find('(').ok_or_else(|| invalid_query("expected a time aggregation such as avg(last_5m): before the metric query"))?;
    let close = closing_paren(lhs, open)?;
    let metric_query = lhs[close + 1..].trim_start().strip_prefix(':').ok_or_else(|| invalid_query("expected ':' after the time aggregation"))?.trim();
    let head = &lhs[..=close];
    let (detection, change_mode, change_minutes, (agg, window)) = match lhs[..open].trim() {
        f @ ("change" | "pct_change") => {
            let inner = &head[open + 1..close];
            let inner_open = inner.find('(').ok_or_else(|| invalid_query(&format!("invalid {f}() arguments")))?;
            let inner_close = closing_paren(inner, inner_open)?;
            let shift = inner[inner_close + 1..].trim_start().strip_prefix(',').ok_or_else(|| invalid_query(&format!("{f}() needs a comparison window, e.g. {f}(avg(last_5m),last_1h)")))?;
            let mode = if f == "pct_change" { ChangeMode::PctChange } else { ChangeMode::Change };
            (Detection::Change, Some(mode), Some(window_minutes(shift)?), reducer(&inner[..=inner_close])?)
        }
        _ => (Detection::Threshold, None, None, reducer(head)?),
    };
    let apm = parse_formula(metric_query).is_ok_and(|f| !f.queries.is_empty() && f.queries.iter().all(|q| parse_metric_query(q).is_ok_and(|q| q.metric.starts_with("trace."))));
    let options = MonitorOptions {
        detection,
        time_aggregation: agg,
        window_minutes: window,
        change_mode,
        change_minutes,
        comparator,
        thresholds: Thresholds { critical, warning: None, critical_recovery: None, warning_recovery: None },
        notify_no_data: false,
        no_data_minutes: 10,
    };
    Ok((if apm { MonitorType::Apm } else { MonitorType::Metric }, metric_query.to_string(), options))
}

/// A double-quoted string at the start of `s` (backslash escapes): (its text, the rest after it).
fn quoted(s: &str) -> Result<(String, &str), String> {
    let rest = s.trim_start().strip_prefix('"').ok_or_else(|| invalid_query("expected a quoted string"))?;
    let mut out = String::new();
    let mut chars = rest.char_indices();
    while let Some((i, c)) = chars.next() {
        match c {
            '\\' => {
                if let Some((_, n)) = chars.next() {
                    out.push(n);
                }
            }
            '"' => return Ok((out, &rest[i + 1..])),
            c => out.push(c),
        }
    }
    Err(invalid_query("unterminated string"))
}

fn quote(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

/// `logs("service:web").index("main").rollup("count").by("source").last("5m") > 2`.
fn parse_log_alert(q: &str) -> Result<(MonitorType, String, MonitorOptions), String> {
    let usage = "log alert queries look like logs(\"<search>\").index(\"*\").rollup(\"count\").by(\"<facet>\").last(\"5m\") > <threshold>";
    let (lhs, comparator, critical) = split_threshold(q)?;
    let rest = lhs.trim().strip_prefix("logs(").ok_or_else(|| invalid_query(usage))?;
    let (search, rest) = quoted(rest)?;
    let mut rest = rest.trim_start().strip_prefix(')').ok_or_else(|| invalid_query(usage))?;
    let mut by: Vec<String> = Vec::new();
    let mut window = None;
    while !rest.trim().is_empty() {
        let call = rest.trim_start().strip_prefix('.').ok_or_else(|| invalid_query(usage))?;
        let open = call.find('(').ok_or_else(|| invalid_query(usage))?;
        let name = call[..open].trim();
        let mut args = Vec::new();
        let mut after = call[open + 1..].trim_start();
        while !after.starts_with(')') {
            let (arg, tail) = if after.starts_with('"') {
                quoted(after)?
            } else {
                let end = after.find([',', ')']).ok_or_else(|| invalid_query(usage))?;
                (after[..end].trim().to_string(), &after[end..])
            };
            args.push(arg);
            after = tail.trim_start();
            after = after.strip_prefix(',').map(str::trim_start).unwrap_or(after);
            if after.is_empty() {
                return Err(invalid_query(usage));
            }
        }
        rest = &after[1..];
        match name {
            "index" => {}
            "rollup" if args.len() == 1 && args[0] == "count" => {}
            "rollup" => return Err(invalid_query("log alerts support only rollup(\"count\") in ObserveCraft for now")),
            "by" => by.extend(args.iter().flat_map(|a| a.split(',')).map(|s| s.trim().to_string()).filter(|s| !s.is_empty())),
            "last" if args.len() == 1 => window = Some(duration_minutes(&args[0])?),
            other => return Err(invalid_query(&format!("unsupported log query method .{other}()"))),
        }
    }
    let window = window.ok_or_else(|| invalid_query("log alert queries need an evaluation window, e.g. .last(\"5m\")"))?;
    let query = if by.is_empty() { format!("logs({})", quote(&search)) } else { format!("logs({}) by {{{}}}", quote(&search), by.join(",")) };
    let options = MonitorOptions {
        detection: Detection::Threshold,
        time_aggregation: TimeAggregator::Sum,
        window_minutes: window,
        change_mode: None,
        change_minutes: None,
        comparator,
        thresholds: Thresholds { critical, warning: None, critical_recovery: None, warning_recovery: None },
        notify_no_data: false,
        no_data_minutes: 10,
    };
    Ok((MonitorType::Log, query, options))
}

/// Datadog's type and query for one of our monitors.
fn datadog_query(m: &Mapped) -> (&'static str, String) {
    let o = &m.options;
    match m.kind {
        MonitorType::Log => {
            let (search, by) = parse_log_monitor(&m.query).unwrap_or_default();
            let by = if by.is_empty() { String::new() } else { format!(".by({})", quote(&by.join(","))) };
            let q = format!("logs({}).index(\"*\").rollup(\"count\"){by}.last(\"{}\") {} {}", quote(&search), minutes_label(o.window_minutes), o.comparator.symbol(), fmt_num(o.thresholds.critical));
            ("log alert", q)
        }
        MonitorType::Synthetics => ("synthetics alert", "no_query".into()),
        MonitorType::Metric | MonitorType::Apm => ("query alert", o.full_query(&m.query)),
    }
}

fn thresholds_document(t: &Thresholds) -> Value {
    let mut out = Map::new();
    out.insert("critical".into(), json!(t.critical));
    for (key, value) in [("warning", t.warning), ("critical_recovery", t.critical_recovery), ("warning_recovery", t.warning_recovery)] {
        if let Some(v) = value {
            out.insert(key.into(), json!(v));
        }
    }
    Value::Object(out)
}

/// The Datadog document of a monitor never written through this API.
pub fn to_datadog(m: &Mapped) -> Value {
    let (t, query) = datadog_query(m);
    let o = &m.options;
    let mut options = json!({
        "thresholds": thresholds_document(&o.thresholds),
        "notify_no_data": o.notify_no_data,
        "no_data_timeframe": o.no_data_minutes,
        "notify_audit": false,
        "include_tags": true,
        "new_host_delay": 300,
        "require_full_window": false,
    });
    if m.kind == MonitorType::Log {
        options["groupby_simple_monitor"] = json!(false);
    }
    json!({
        "name": m.name,
        "type": t,
        "query": query,
        "message": m.message,
        "tags": m.tags,
        "priority": m.priority,
        "restricted_roles": null,
        "draft_status": if m.draft { "draft" } else { "published" },
        "options": options,
    })
}

/// Puts `current` (our row) into `doc` wherever the document no longer maps to it (edited in our UI).
pub fn reconcile(doc: &mut Value, current: &Mapped) {
    let derived = match from_datadog(doc) {
        Ok(d) => d,
        Err(_) => {
            *doc = to_datadog(current);
            return;
        }
    };
    if derived == *current {
        return;
    }
    let fresh = to_datadog(current);
    if derived.name != current.name {
        doc["name"] = fresh["name"].clone();
    }
    if derived.message != current.message {
        doc["message"] = fresh["message"].clone();
    }
    if derived.priority != current.priority {
        doc["priority"] = fresh["priority"].clone();
    }
    if derived.tags != current.tags {
        doc["tags"] = fresh["tags"].clone();
    }
    if derived.draft != current.draft {
        doc["draft_status"] = fresh["draft_status"].clone();
    }
    if derived.kind != current.kind || derived.query != current.query || derived.options != current.options {
        doc["query"] = fresh["query"].clone();
        if derived.family() != current.family() {
            doc["type"] = fresh["type"].clone();
        }
        if !doc.get("options").is_some_and(Value::is_object) {
            doc["options"] = json!({});
        }
        let options = &mut doc["options"];
        let (d, c) = (&derived.options, &current.options);
        if d.thresholds != c.thresholds {
            if !options.get("thresholds").is_some_and(Value::is_object) {
                options["thresholds"] = json!({});
            }
            let thresholds = options["thresholds"].as_object_mut().expect("object");
            if thresholds.contains_key("critical") && d.thresholds.critical != c.thresholds.critical {
                thresholds.insert("critical".into(), json!(c.thresholds.critical));
            }
            let pairs = [
                ("warning", d.thresholds.warning, c.thresholds.warning),
                ("critical_recovery", d.thresholds.critical_recovery, c.thresholds.critical_recovery),
                ("warning_recovery", d.thresholds.warning_recovery, c.thresholds.warning_recovery),
            ];
            for (key, _, now) in pairs.into_iter().filter(|(_, was, now)| was != now) {
                match now {
                    Some(v) => thresholds.insert(key.into(), json!(v)),
                    None => thresholds.remove(key),
                };
            }
        }
        if d.notify_no_data != c.notify_no_data {
            options["notify_no_data"] = json!(c.notify_no_data);
            if !c.notify_no_data && options.get("on_missing_data").and_then(Value::as_str) == Some("show_and_notify_no_data") {
                options["on_missing_data"] = json!("show_no_data");
            }
        }
        if d.no_data_minutes != c.no_data_minutes {
            options["no_data_timeframe"] = json!(c.no_data_minutes);
        }
    }
}

/// A document as a client sent it (create) or merged onto the stored one (update: top-level fields
/// replace the stored ones, `options` as a whole), without server-owned fields and with Datadog's
/// defaults for what's left out.
pub fn normalize(body: Value, stored: Option<Value>) -> Result<Value, String> {
    let Value::Object(mut incoming) = body else { return Err("the monitor must be a JSON object".into()) };
    for key in SERVER_OWNED {
        incoming.remove(*key);
    }
    let mut doc = match stored {
        Some(Value::Object(mut stored)) => {
            stored.extend(incoming);
            stored
        }
        _ => incoming,
    };
    if !doc.get("options").is_some_and(Value::is_object) {
        doc.insert("options".into(), json!({}));
    }
    let log = doc.get("type").and_then(Value::as_str) == Some("log alert");
    let options = doc["options"].as_object_mut().expect("object");
    options.remove("silenced");
    let mut defaults = vec![("notify_no_data", json!(false)), ("notify_audit", json!(false)), ("new_host_delay", json!(300)), ("include_tags", json!(true))];
    if log {
        defaults.push(("groupby_simple_monitor", json!(false)));
    }
    for (key, value) in defaults {
        options.entry(key).or_insert(value);
    }
    for (key, value) in [("message", json!("")), ("tags", json!([])), ("priority", Value::Null), ("restricted_roles", Value::Null), ("draft_status", json!("published"))] {
        doc.entry(key).or_insert(value);
    }
    Ok(Value::Object(doc))
}

// ================================================================ ids and timestamps

/// Datadog's numeric org id for one of our orgs (ours are text): a stable 47-bit hash.
pub fn org_number(org: &str) -> i64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in org.bytes() {
        h = (h ^ b as u64).wrapping_mul(0x0100_0000_01b3);
    }
    (h & 0x7fff_ffff_ffff) as i64
}

/// Datadog's numeric (v1) user id for one of our user UUIDs: its first 48 bits.
pub fn user_number(user_id: &str) -> i64 {
    let hex: String = user_id.chars().filter(char::is_ascii_hexdigit).take(12).collect();
    i64::from_str_radix(&hex, 16).unwrap_or(0)
}

/// `2026-10-29T12:00:00.000000+00:00`, as Datadog writes timestamps.
pub fn iso(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms).map(|t| t.format("%Y-%m-%dT%H:%M:%S%.6f+00:00").to_string()).unwrap_or_default()
}

pub fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

// ================================================================ storage

#[derive(sqlx::FromRow)]
struct Row {
    id: i64,
    kind: String,
    name: String,
    query: String,
    definition: Value,
    status: String,
    message: String,
    priority: Option<i16>,
    tags: Vec<String>,
    teams: Vec<String>,
    draft: bool,
    datadog_document: Option<Value>,
    creator_id: Option<String>,
    creator_email: Option<String>,
    creator_name: Option<String>,
    created_at: i64,
    modified_at: i64,
    state_changed_at: i64,
}

const SELECT: &str = "SELECT m.id, m.type AS kind, m.name, m.query, m.definition, m.status, m.message, m.priority, m.tags, m.teams, m.draft, m.datadog_document, \
    m.created_by::text AS creator_id, u.email AS creator_email, u.name AS creator_name, \
    (extract(epoch FROM m.created_at) * 1000)::bigint AS created_at, (extract(epoch FROM m.modified_at) * 1000)::bigint AS modified_at, \
    (extract(epoch FROM m.state_changed_at) * 1000)::bigint AS state_changed_at \
    FROM monitors m LEFT JOIN users u ON u.id = m.created_by";

impl Row {
    fn kind(&self) -> MonitorType {
        MonitorType::parse(&self.kind)
    }

    /// Our definition as a [`Mapped`] (None for synthetic test monitors).
    fn current(&self) -> Option<Mapped> {
        if self.kind() == MonitorType::Synthetics {
            return None;
        }
        Some(Mapped {
            kind: self.kind(),
            query: self.query.clone(),
            options: serde_json::from_value(self.definition.clone()).ok()?,
            name: self.name.clone(),
            message: self.message.clone(),
            priority: self.priority,
            tags: self.tags.clone(),
            draft: self.draft,
        })
    }

    /// The Datadog document, in sync with the row.
    fn document(&self) -> Value {
        let Some(current) = self.current() else {
            return json!({
                "name": self.name, "type": "synthetics alert", "query": "no_query", "message": self.message, "tags": self.tags,
                "priority": self.priority, "restricted_roles": null, "draft_status": if self.draft { "draft" } else { "published" },
                "options": { "notify_audit": false, "include_tags": true, "new_host_delay": 300, "notify_no_data": false,
                    "synthetics_check_id": self.definition.get("syntheticTestId") },
            });
        };
        match self.datadog_document.clone() {
            Some(mut doc) => {
                reconcile(&mut doc, &current);
                doc
            }
            None => to_datadog(&current),
        }
    }

    fn multi(&self) -> bool {
        match self.kind() {
            MonitorType::Log => parse_log_monitor(&self.query).is_ok_and(|(_, by)| !by.is_empty()),
            MonitorType::Synthetics => false,
            _ => self.query.contains(" by "),
        }
    }

    fn applies(&self, d: &Downtime) -> bool {
        d.applies_to(self.id, &self.tags)
    }
}

async fn load(st: &AppState, org: &str, id: i64) -> Result<Option<Row>, DdError> {
    Ok(sqlx::query_as(&format!("{SELECT} WHERE m.org_id = $1 AND m.id = $2")).bind(org).bind(id).fetch_optional(&st.postgres).await?)
}

async fn load_all(st: &AppState, org: &str) -> Result<Vec<Row>, DdError> {
    Ok(sqlx::query_as(&format!("{SELECT} WHERE m.org_id = $1 ORDER BY m.id")).bind(org).fetch_all(&st.postgres).await?)
}

fn not_found() -> DdError {
    DdError::new(StatusCode::NOT_FOUND, "Monitor not found")
}

fn bad_request(message: impl Into<String>) -> DdError {
    DdError::new(StatusCode::BAD_REQUEST, message)
}

fn monitor_id(s: &str) -> Result<i64, DdError> {
    s.parse().map_err(|_| not_found())
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

fn overall_state(status: &str) -> &'static str {
    match MonitorStatus::parse(status) {
        MonitorStatus::Ok => "OK",
        MonitorStatus::Warn => "Warn",
        MonitorStatus::Alert => "Alert",
        MonitorStatus::NoData => "No Data",
    }
}

/// `options.silenced`: scope → end (epoch seconds, null = no end) of the active downtimes.
fn silenced(row: &Row, downtimes: &[Downtime], now: i64) -> Value {
    let mut out = Map::new();
    for d in downtimes.iter().filter(|d| row.applies(d)) {
        if let Some((_, end)) = d.window().active_at(now) {
            let scope = if d.scope.is_empty() { "*".to_string() } else { d.scope.join(",") };
            out.insert(scope, end.map_or(Value::Null, |e| json!(e / 1000)));
        }
    }
    Value::Object(out)
}

/// A monitor as Datadog answers it.
fn render(org: &str, row: &Row, downtimes: &[Downtime], matching: Option<Vec<Value>>) -> Value {
    let mut doc = row.document();
    let now = now_ms();
    doc["id"] = json!(row.id);
    doc["org_id"] = json!(org_number(org));
    doc["created"] = json!(iso(row.created_at));
    doc["created_at"] = json!(row.created_at);
    doc["modified"] = json!(iso(row.modified_at));
    doc["deleted"] = Value::Null;
    doc["multi"] = json!(row.multi());
    doc["overall_state"] = json!(overall_state(&row.status));
    doc["overall_state_modified"] = json!(iso(row.state_changed_at));
    doc["creator"] = match &row.creator_id {
        Some(id) => json!({ "id": user_number(id), "email": row.creator_email, "handle": row.creator_email, "name": row.creator_name }),
        None => Value::Null,
    };
    doc["options"]["silenced"] = silenced(row, downtimes, now);
    if doc.get("assets").is_none() {
        doc["assets"] = json!([]);
    }
    if let Some(m) = matching {
        doc["matching_downtimes"] = Value::Array(m);
    }
    doc
}

/// The v1 documents of the downtimes that apply to each monitor (`with_downtimes`).
async fn matching_downtimes(st: &AppState, org: &str, rows: &[&Row]) -> Result<HashMap<i64, Vec<Value>>, DdError> {
    let downtimes = super::downtimes::load_rows(st, org, false).await?;
    let now = now_ms();
    let mut out: HashMap<i64, Vec<Value>> = HashMap::new();
    for row in rows {
        let docs = downtimes.iter().filter(|d| d.applies_to(row.id, &row.tags)).filter_map(|d| d.matching(now)).collect();
        out.insert(row.id, docs);
    }
    Ok(out)
}

fn truthy(params: &HashMap<String, String>, key: &str, default: bool) -> bool {
    params.get(key).map_or(default, |v| v == "true")
}

// ================================================================ handlers

/// Create a monitor from a Datadog monitor document.
#[utoipa::path(post, path = "/api/v1/monitor", operation_id = "datadog_create_monitor", request_body = Object,
    responses((status = 200, body = Object), (status = 400, body = super::DatadogErrors), (status = 403, body = super::DatadogErrors)), tag = "datadog-api")]
pub async fn create(State(st): State<AppState>, DdAuth(ctx): DdAuth, body: Bytes) -> Result<Json<Value>, DdError> {
    writer(&ctx)?;
    let doc = normalize(json_body(&body)?, None).map_err(bad_request)?;
    let mapped = from_datadog(&doc).map_err(bad_request)?;
    let id = monitors::store_new(&st, &ctx, &mapped.input(vec![]), Some(&doc)).await?;
    let row = load(&st, &ctx.org_id, id).await?.ok_or_else(not_found)?;
    let downtimes = load_downtimes(&st, &ctx.org_id).await?;
    Ok(Json(render(&ctx.org_id, &row, &downtimes, None)))
}

/// A monitor as a Datadog monitor document.
#[utoipa::path(get, path = "/api/v1/monitor/{monitor_id}", operation_id = "datadog_get_monitor", params(("monitor_id" = i64, Path), ("with_downtimes" = Option<bool>, Query)),
    responses((status = 200, body = Object), (status = 404, body = super::DatadogErrors)), tag = "datadog-api")]
pub async fn get_one(State(st): State<AppState>, DdAuth(ctx): DdAuth, Path(id): Path<String>, Query(params): Query<HashMap<String, String>>) -> Result<Json<Value>, DdError> {
    let id = monitor_id(&id)?;
    let row = load(&st, &ctx.org_id, id).await?.ok_or_else(not_found)?;
    check_group_states(&params)?;
    let downtimes = load_downtimes(&st, &ctx.org_id).await?;
    let matching = if truthy(&params, "with_downtimes", false) { matching_downtimes(&st, &ctx.org_id, &[&row]).await?.remove(&row.id) } else { None };
    Ok(Json(render(&ctx.org_id, &row, &downtimes, matching)))
}

fn check_group_states(params: &HashMap<String, String>) -> Result<(), DdError> {
    if let Some(states) = params.get("group_states") {
        if states.split(',').map(str::trim).any(|s| !["all", "alert", "warn", "no data", "ok", ""].contains(&s)) {
            return Err(bad_request("Invalid group_state filters."));
        }
    }
    Ok(())
}

/// The org's monitors, filtered as the `datadog_monitor(s)` data sources ask (`name`, `tags`,
/// `monitor_tags`), paged with `page` / `page_size` or `id_offset`.
#[utoipa::path(get, path = "/api/v1/monitor", operation_id = "datadog_list_monitors",
    params(("name" = Option<String>, Query), ("tags" = Option<String>, Query), ("monitor_tags" = Option<String>, Query), ("group_states" = Option<String>, Query),
        ("with_downtimes" = Option<bool>, Query), ("id_offset" = Option<i64>, Query), ("page" = Option<i64>, Query), ("page_size" = Option<i64>, Query)),
    responses((status = 200, body = Vec<Object>), (status = 400, body = super::DatadogErrors)), tag = "datadog-api")]
pub async fn list(State(st): State<AppState>, DdAuth(ctx): DdAuth, Query(params): Query<HashMap<String, String>>) -> Result<Json<Vec<Value>>, DdError> {
    check_group_states(&params)?;
    let list_param = |key: &str| -> Vec<String> { params.get(key).map(|v| v.split(',').map(|s| s.trim().to_lowercase()).filter(|s| !s.is_empty()).collect()).unwrap_or_default() };
    let int_param = |key: &str| -> Result<Option<i64>, DdError> { params.get(key).map(|v| v.parse::<i64>().map_err(|_| bad_request(format!("Invalid {key}")))).transpose() };
    let name = params.get("name").map(|n| n.to_lowercase());
    let (scope_tags, monitor_tags) = (list_param("tags"), list_param("monitor_tags"));
    let id_offset = int_param("id_offset")?;
    let rows = load_all(&st, &ctx.org_id).await?;
    let matched: Vec<&Row> = rows
        .iter()
        .filter(|r| name.as_ref().is_none_or(|n| r.name.to_lowercase().contains(n)))
        .filter(|r| {
            let q = r.document()["query"].as_str().unwrap_or_default().to_lowercase();
            scope_tags.iter().all(|t| q.contains(t))
        })
        .filter(|r| monitor_tags.iter().all(|t| r.tags.iter().any(|mt| mt.to_lowercase() == *t)))
        .filter(|r| id_offset.is_none_or(|o| r.id > o))
        .collect();
    let page: Vec<&Row> = match int_param("page")? {
        Some(p) => {
            let size = int_param("page_size")?.unwrap_or(100).clamp(1, 1000);
            matched.into_iter().skip((p.max(0) * size) as usize).take(size as usize).collect()
        }
        None => match int_param("page_size")? {
            Some(size) => matched.into_iter().take(size.clamp(1, 1000) as usize).collect(),
            None => matched,
        },
    };
    let downtimes = load_downtimes(&st, &ctx.org_id).await?;
    let mut matching = if truthy(&params, "with_downtimes", true) { Some(matching_downtimes(&st, &ctx.org_id, &page).await?) } else { None };
    Ok(Json(page.iter().map(|r| render(&ctx.org_id, r, &downtimes, matching.as_mut().map(|m| m.remove(&r.id).unwrap_or_default()))).collect()))
}

/// Replace a monitor's fields with those sent (Datadog's monitor update).
#[utoipa::path(put, path = "/api/v1/monitor/{monitor_id}", operation_id = "datadog_update_monitor", params(("monitor_id" = i64, Path)), request_body = Object,
    responses((status = 200, body = Object), (status = 400, body = super::DatadogErrors), (status = 404, body = super::DatadogErrors)), tag = "datadog-api")]
pub async fn update(State(st): State<AppState>, DdAuth(ctx): DdAuth, Path(id): Path<String>, body: Bytes) -> Result<Json<Value>, DdError> {
    writer(&ctx)?;
    let id = monitor_id(&id)?;
    let row = load(&st, &ctx.org_id, id).await?.ok_or_else(not_found)?;
    let (doc, mapped) = merged(&row, json_body(&body)?)?;
    monitors::store_update(&st, &ctx, id, &mapped.input(row.teams.clone()), Some(&doc)).await?;
    let row = load(&st, &ctx.org_id, id).await?.ok_or_else(not_found)?;
    let downtimes = load_downtimes(&st, &ctx.org_id).await?;
    Ok(Json(render(&ctx.org_id, &row, &downtimes, None)))
}

/// The stored document with an update applied, and what it maps to.
fn merged(row: &Row, body: Value) -> Result<(Value, Mapped), DdError> {
    if row.kind() == MonitorType::Synthetics {
        return Err(bad_request(SYNTHETICS_MANAGED));
    }
    let doc = normalize(body, Some(row.document())).map_err(bad_request)?;
    let new_type = doc.get("type").and_then(Value::as_str).unwrap_or_default();
    if datadog_family(new_type) != family(row.kind()) && datadog_family(new_type) != "other" {
        return Err(bad_request(format!("The monitor type cannot be changed (from '{}' to '{new_type}')", row.document()["type"].as_str().unwrap_or_default())));
    }
    let mapped = from_datadog(&doc).map_err(bad_request)?;
    Ok((doc, mapped))
}

/// Delete a monitor. Monitors an SLO uses need `force=true`.
#[utoipa::path(delete, path = "/api/v1/monitor/{monitor_id}", operation_id = "datadog_delete_monitor", params(("monitor_id" = i64, Path), ("force" = Option<String>, Query)),
    responses((status = 200, body = Object), (status = 400, body = super::DatadogErrors), (status = 404, body = super::DatadogErrors)), tag = "datadog-api")]
pub async fn delete(State(st): State<AppState>, DdAuth(ctx): DdAuth, Path(id): Path<String>, Query(params): Query<HashMap<String, String>>) -> Result<Json<Value>, DdError> {
    writer(&ctx)?;
    let id = monitor_id(&id)?;
    let row = load(&st, &ctx.org_id, id).await?.ok_or_else(not_found)?;
    if row.kind() == MonitorType::Synthetics {
        return Err(bad_request(SYNTHETICS_MANAGED));
    }
    if params.get("force").map(String::as_str) != Some("true") {
        let slos = slos_using(&st, &ctx.org_id, id).await?;
        if !slos.is_empty() {
            return Err(bad_request(format!("monitor [{id},{}] is referenced in slos: {}", row.name, slos.join(", "))));
        }
    }
    sqlx::query("DELETE FROM monitors WHERE org_id = $1 AND id = $2").bind(&ctx.org_id).bind(id).execute(&st.postgres).await?;
    ctx.audit(&st, "monitor.delete", &format!("monitor:{id}"), &row.name).await;
    Ok(Json(json!({ "deleted_monitor_id": id })))
}

async fn slos_using(st: &AppState, org: &str, id: i64) -> Result<Vec<String>, DdError> {
    Ok(sqlx::query_scalar("SELECT name FROM slos WHERE org_id = $1 AND $2 = ANY(monitor_ids) ORDER BY name").bind(org).bind(id).fetch_all(&st.postgres).await?)
}

/// Check a new monitor document without saving it.
#[utoipa::path(post, path = "/api/v1/monitor/validate", operation_id = "datadog_validate_monitor", request_body = Object,
    responses((status = 200, body = Object), (status = 400, body = super::DatadogErrors)), tag = "datadog-api")]
pub async fn validate(_auth: DdAuth, body: Bytes) -> Result<Json<Value>, DdError> {
    let doc = normalize(json_body(&body)?, None).map_err(bad_request)?;
    from_datadog(&doc).map_err(bad_request)?;
    Ok(Json(json!({})))
}

/// Check an update of an existing monitor without saving it.
#[utoipa::path(post, path = "/api/v1/monitor/{monitor_id}/validate", operation_id = "datadog_validate_existing_monitor", params(("monitor_id" = i64, Path)), request_body = Object,
    responses((status = 200, body = Object), (status = 400, body = super::DatadogErrors), (status = 404, body = super::DatadogErrors)), tag = "datadog-api")]
pub async fn validate_existing(State(st): State<AppState>, DdAuth(ctx): DdAuth, Path(id): Path<String>, body: Bytes) -> Result<Json<Value>, DdError> {
    let id = monitor_id(&id)?;
    let row = load(&st, &ctx.org_id, id).await?.ok_or_else(not_found)?;
    merged(&row, json_body(&body)?)?;
    Ok(Json(json!({})))
}

/// Which of `monitor_ids` can be deleted without `force` (not used by an SLO).
#[utoipa::path(get, path = "/api/v1/monitor/can_delete", operation_id = "datadog_can_delete_monitors", params(("monitor_ids" = String, Query)),
    responses((status = 200, body = Object), (status = 409, body = Object), (status = 400, body = super::DatadogErrors)), tag = "datadog-api")]
pub async fn can_delete(State(st): State<AppState>, DdAuth(ctx): DdAuth, Query(params): Query<HashMap<String, String>>) -> Result<Response, DdError> {
    let ids: Vec<i64> = params
        .get("monitor_ids")
        .ok_or_else(|| bad_request("monitor_ids is required"))?
        .split(',')
        .map(|s| s.trim().parse::<i64>().map_err(|_| bad_request(format!("Invalid monitor id '{s}'"))))
        .collect::<Result<_, _>>()?;
    let mut ok = Vec::new();
    let mut errors = Map::new();
    for id in ids {
        let problem = match load(&st, &ctx.org_id, id).await? {
            None => Some("Monitor not found".to_string()),
            Some(r) if r.kind() == MonitorType::Synthetics => Some(SYNTHETICS_MANAGED.to_string()),
            Some(r) => {
                let slos = slos_using(&st, &ctx.org_id, id).await?;
                (!slos.is_empty()).then(|| format!("monitor [{id},{}] is referenced in slos: {}", r.name, slos.join(", ")))
            }
        };
        match problem {
            Some(p) => {
                errors.insert(id.to_string(), json!([p]));
            }
            None => ok.push(id),
        }
    }
    let status = if errors.is_empty() { StatusCode::OK } else { StatusCode::CONFLICT };
    let errors = if errors.is_empty() { Value::Null } else { Value::Object(errors) };
    Ok((status, Json(json!({ "data": { "ok": ok }, "errors": errors }))).into_response())
}

/// Search monitors: `status:alert`, `type:log`, `muted:true`, `tag:"env:prod"`, `priority:p1`
/// terms and free text on the name, with counts per status, muted, tag and type.
#[utoipa::path(get, path = "/api/v1/monitor/search", operation_id = "datadog_search_monitors",
    params(("query" = Option<String>, Query), ("page" = Option<i64>, Query), ("per_page" = Option<i64>, Query), ("sort" = Option<String>, Query)),
    responses((status = 200, body = Object), (status = 400, body = super::DatadogErrors)), tag = "datadog-api")]
pub async fn search(State(st): State<AppState>, DdAuth(ctx): DdAuth, Query(params): Query<HashMap<String, String>>) -> Result<Json<Value>, DdError> {
    let terms = search_terms(params.get("query").map(String::as_str).unwrap_or(""))?;
    let rows = load_all(&st, &ctx.org_id).await?;
    let downtimes = load_downtimes(&st, &ctx.org_id).await?;
    let now = now_ms();
    struct Hit<'a> {
        row: &'a Row,
        doc: Value,
        state: &'static str,
        muted: Option<Option<i64>>,
    }
    let hits: Vec<Hit> = rows
        .iter()
        .map(|row| {
            let mut muted: Option<Option<i64>> = None;
            for d in downtimes.iter().filter(|d| row.applies(d) && d.scope.is_empty()) {
                if let Some((_, end)) = d.window().active_at(now) {
                    muted = Some(match (muted, end) {
                        (Some(None), _) | (_, None) => None,
                        (Some(Some(a)), Some(b)) => Some(a.max(b)),
                        (None, Some(b)) => Some(b),
                    });
                }
            }
            Hit { doc: row.document(), state: overall_state(&row.status), muted, row }
        })
        .filter(|h| {
            terms.iter().all(|t| match t {
                SearchTerm::Status(s) => h.state.eq_ignore_ascii_case(s),
                SearchTerm::Type(t) => h.doc["type"].as_str().is_some_and(|dt| dt.starts_with(t.as_str())) || family(h.row.kind()) == t.as_str(),
                SearchTerm::Muted(m) => h.muted.is_some() == *m,
                SearchTerm::Tag(t) => h.row.tags.iter().any(|rt| rt.eq_ignore_ascii_case(t)),
                SearchTerm::Priority(p) => h.row.priority == Some(*p),
                SearchTerm::Text(w) => h.row.name.to_lowercase().contains(w) || h.row.query.to_lowercase().contains(w),
            })
        })
        .collect();
    let count = |values: Vec<Value>| -> Value {
        let mut map: BTreeMap<String, (Value, i64)> = BTreeMap::new();
        for v in values {
            map.entry(v.to_string()).or_insert((v, 0)).1 += 1;
        }
        let mut out: Vec<(Value, i64)> = map.into_values().collect();
        out.sort_by(|a, b| b.1.cmp(&a.1));
        Value::Array(out.into_iter().map(|(name, count)| json!({ "name": name, "count": count })).collect())
    };
    let counts = json!({
        "status": count(hits.iter().map(|h| json!(h.state)).collect()),
        "muted": count(hits.iter().map(|h| json!(h.muted.is_some())).collect()),
        "tag": count(hits.iter().flat_map(|h| h.row.tags.iter().map(|t| json!(t))).collect()),
        "type": count(hits.iter().map(|h| json!(family(h.row.kind()))).collect()),
    });
    let per_page = params.get("per_page").and_then(|v| v.parse::<i64>().ok()).unwrap_or(30).clamp(1, 1000);
    let page = params.get("page").and_then(|v| v.parse::<i64>().ok()).unwrap_or(0).max(0);
    let total = hits.len() as i64;
    let monitors: Vec<Value> = hits
        .iter()
        .skip((page * per_page) as usize)
        .take(per_page as usize)
        .map(|h| {
            let r = h.row;
            let metrics: Vec<String> = match r.kind() {
                MonitorType::Metric | MonitorType::Apm => parse_formula(&r.query).map(|f| f.queries.iter().filter_map(|q| parse_metric_query(q).ok().map(|q| q.metric)).collect()).unwrap_or_default(),
                _ => vec![],
            };
            let creator = r.creator_id.as_ref().map(|id| json!({ "id": user_number(id), "handle": r.creator_email, "name": r.creator_name }));
            json!({
                "id": r.id,
                "name": r.name,
                "type": h.doc["type"],
                "query": h.doc["query"],
                "status": h.state,
                "overall_state_modified": iso(r.state_changed_at),
                "tags": r.tags,
                "classification": family(r.kind()),
                "creator": creator,
                "metrics": metrics,
                "notifications": crate::routes::monitor_engine::recipients(&r.message).iter().map(|h| json!({ "handle": h, "name": h })).collect::<Vec<_>>(),
                "muted_until_ts": h.muted.flatten().map(|t| t / 1000),
                "last_triggered_ts": Value::Null,
                "scopes": Vec::<String>::new(),
                "org_id": org_number(&ctx.org_id),
                "priority": r.priority,
                "restricted_roles": h.doc.get("restricted_roles").cloned().unwrap_or(Value::Null),
            })
        })
        .collect();
    let page_count = (total + per_page - 1) / per_page;
    Ok(Json(json!({ "monitors": monitors, "counts": counts, "metadata": { "page": page, "per_page": per_page, "page_count": page_count, "total_count": total } })))
}

#[derive(Debug, PartialEq)]
enum SearchTerm {
    Status(String),
    Type(String),
    Muted(bool),
    Tag(String),
    Priority(i16),
    Text(String),
}

fn search_terms(query: &str) -> Result<Vec<SearchTerm>, DdError> {
    let mut terms = Vec::new();
    let mut rest = query.trim();
    while !rest.is_empty() {
        let (word, tail) = match rest.find(|c: char| c.is_whitespace() || c == '"') {
            Some(i) if rest.as_bytes()[i] == b'"' => {
                let (q, tail) = quoted(&rest[i..]).map_err(|_| bad_request("Query parsing error: unterminated quote"))?;
                (format!("{}{q}", &rest[..i]), tail)
            }
            Some(i) => (rest[..i].to_string(), &rest[i..]),
            None => (rest.to_string(), ""),
        };
        rest = tail.trim_start();
        let term = match word.split_once(':') {
            Some(("status", s)) => {
                let s = s.to_lowercase();
                if !["alert", "warn", "no data", "ok"].contains(&s.as_str()) {
                    return Err(bad_request(format!("Query parsing error: '{s}' is not a valid monitor status")));
                }
                SearchTerm::Status(if s == "ok" { "OK".into() } else { s })
            }
            Some(("type", t)) => SearchTerm::Type(t.to_lowercase()),
            Some(("muted", m)) => SearchTerm::Muted(m == "true"),
            Some(("tag", t)) => SearchTerm::Tag(t.to_string()),
            Some(("priority", p)) => SearchTerm::Priority(p.trim_start_matches(['p', 'P']).parse().map_err(|_| bad_request(format!("Query parsing error: '{p}' is not a valid priority")))?),
            _ => SearchTerm::Text(word.to_lowercase()),
        };
        terms.push(term);
    }
    Ok(terms)
}

/// Replays a vendored feature's recorded scenarios against a test server, except `unsupported`
/// (scenario → why); returns the scenarios replayed, or None without the live test databases.
#[cfg(test)]
pub(crate) async fn replay_feature(prefix: &str, path: &str, unsupported: &[(&str, &str)]) -> Option<Vec<String>> {
    use super::replay::{Feature, TestServer};
    let s = TestServer::start(prefix).await?;
    let feature = Feature::load(path);
    for (name, _) in unsupported {
        feature.scenario(name);
    }
    let mut replayed = Vec::new();
    for scenario in &feature.scenarios {
        if unsupported.iter().any(|(n, _)| *n == scenario.name) || feature.cassette(scenario).is_none() {
            continue;
        }
        let (_, problems) = s.replayer().replay(&feature, scenario).await;
        assert!(problems.is_empty(), "{}: {problems:#?}", scenario.name);
        replayed.push(scenario.name.clone());
    }
    Some(replayed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(v: Value) -> Value {
        normalize(v, None).unwrap()
    }

    fn metric(query: &str) -> Value {
        doc(json!({ "name": "CPU on web hosts", "type": "metric alert", "query": query, "message": "CPU high @slack-ops @oncall@example.com", "tags": ["service:storefront-api", "env:prod"] }))
    }

    #[test]
    fn metric_alerts_map_onto_metric_monitors() {
        let m = from_datadog(&metric("avg(last_5m):avg:system.cpu.user{env:prod} by {host} > 90")).unwrap();
        assert_eq!(m.kind, MonitorType::Metric);
        assert_eq!(m.query, "avg:system.cpu.user{env:prod} by {host}");
        let o = &m.options;
        assert_eq!((o.detection, o.time_aggregation, o.window_minutes, o.comparator), (Detection::Threshold, TimeAggregator::Avg, 5, Comparator::Above));
        assert_eq!(o.thresholds, Thresholds { critical: 90.0, warning: None, critical_recovery: None, warning_recovery: None });
        assert_eq!((o.notify_no_data, o.no_data_minutes), (false, 10));
        assert_eq!(m.name, "CPU on web hosts");
        assert_eq!(m.tags, vec!["service:storefront-api", "env:prod"]);
        assert_eq!((m.priority, m.draft), (None, false));

        let m = from_datadog(&metric("max(last_1h):sum:trace.http.request.errors{service:billing} >= 5")).unwrap();
        assert_eq!((m.kind, m.options.time_aggregation, m.options.window_minutes, m.options.comparator), (MonitorType::Apm, TimeAggregator::Max, 60, Comparator::AboveOrEqual));

        let m = from_datadog(&metric("pct_change(avg(last_5m),last_1d):avg:system.load.1{*} < -50")).unwrap();
        let o = &m.options;
        assert_eq!(
            (o.detection, o.change_mode, o.change_minutes, o.window_minutes, o.comparator, o.thresholds.critical),
            (Detection::Change, Some(ChangeMode::PctChange), Some(1440), 5, Comparator::Below, -50.0)
        );

        let m = from_datadog(&metric("sum(last_10m):sum:http.errors{*} / sum:http.requests{*} * 100 > 5")).unwrap();
        assert_eq!(m.query, "sum:http.errors{*} / sum:http.requests{*} * 100");
    }

    #[test]
    fn options_map_thresholds_and_no_data() {
        let mut d = metric("avg(last_5m):avg:system.load.1{*} > 4");
        d["priority"] = json!(2);
        d["draft_status"] = json!("draft");
        d["options"] =
            json!({ "thresholds": { "critical": 4, "warning": 3, "critical_recovery": 3.5, "warning_recovery": 2.5 }, "notify_no_data": true, "no_data_timeframe": 30, "renotify_interval": 60 });
        let m = from_datadog(&d).unwrap();
        assert_eq!(m.options.thresholds, Thresholds { critical: 4.0, warning: Some(3.0), critical_recovery: Some(3.5), warning_recovery: Some(2.5) });
        assert_eq!((m.options.notify_no_data, m.options.no_data_minutes), (true, 30));
        assert_eq!((m.priority, m.draft), (Some(2), true));

        // Without a timeframe, no data means twice the evaluation window, as on Datadog.
        d["options"] = json!({ "on_missing_data": "show_and_notify_no_data" });
        let m = from_datadog(&d).unwrap();
        assert_eq!((m.options.notify_no_data, m.options.no_data_minutes), (true, 10));

        d["options"] = json!({ "thresholds": { "critical": 5 } });
        let e = from_datadog(&d).unwrap_err();
        assert!(e.contains("alert threshold (5) does not match that used in the query (4)"), "{e}");
        d["options"] = json!({ "thresholds": { "critical": 4, "warning": 6 } });
        assert!(from_datadog(&d).unwrap_err().contains("warning threshold must be below"));
    }

    #[test]
    fn log_alerts_map_onto_log_monitors() {
        let d = doc(
            json!({ "name": "Checkout errors", "type": "log alert", "query": "logs(\"service:checkout AND status:error\").index(\"main\").rollup(\"count\").by(\"source,host\").last(\"15m\") > 2" }),
        );
        let m = from_datadog(&d).unwrap();
        assert_eq!(m.kind, MonitorType::Log);
        assert_eq!(m.query, "logs(\"service:checkout AND status:error\") by {source,host}");
        assert_eq!((m.options.time_aggregation, m.options.window_minutes, m.options.comparator, m.options.thresholds.critical), (TimeAggregator::Sum, 15, Comparator::Above, 2.0));
        let d = doc(json!({ "name": "Quiet", "type": "log alert", "query": "logs(\"service:\\\"web store\\\"\").index(\"*\").rollup(\"count\").last(\"1h\") < 1" }));
        assert_eq!(from_datadog(&d).unwrap().query, "logs(\"service:\\\"web store\\\"\")");
        let d = doc(json!({ "name": "x", "type": "log alert", "query": "logs(\"*\").index(\"*\").rollup(\"avg\", \"@duration\").last(\"5m\") > 1" }));
        assert!(from_datadog(&d).unwrap_err().contains("only rollup"));
    }

    #[test]
    fn unsupported_documents_are_rejected_precisely() {
        let reject = |t: &str, q: &str| from_datadog(&doc(json!({ "name": "x", "type": t, "query": q }))).unwrap_err();
        assert_eq!(reject("composite", "1 && 2"), "monitor type 'composite' is not supported by ObserveCraft yet (supported: metric alert, query alert, log alert)");
        assert!(reject("service check", "\"http.can_connect\".over(\"*\").by(\"host\").last(2).count_by_status()").contains("'service check' is not supported"));
        assert!(reject("synthetics alert", "no_query").contains("synthetic test monitors"));
        assert!(reject("weird", "x").starts_with("Invalid monitor type 'weird'"));
        assert!(reject("log alert", "query").starts_with("The value provided for parameter 'query' is invalid: invalid operator specified"));
        assert!(reject("metric alert", "avg(current_1mo):avg:system.load.5{*} > 0.5").contains("calendar-aligned"));
        assert!(reject("metric alert", "percentile(last_5m):p95:latency{*} > 1").contains("time aggregation 'percentile'"));
        assert!(reject("query alert", "avg(last_5m):anomalies(avg:system.load.1{*}, 'basic', 2) >= 1").contains("invalid query"));
        let mut d = metric("avg(last_5m):avg:system.load.1{*} > 1");
        d["options"]["variables"] = json!([{ "name": "query1", "data_source": "metrics", "query": "avg:system.load.1{*}" }]);
        assert!(from_datadog(&d).unwrap_err().contains("options.variables"));
    }

    #[test]
    fn our_monitors_round_trip_through_datadog_documents() {
        for d in [
            metric("avg(last_5m):avg:system.cpu.user{env:prod} by {host} > 90"),
            metric("change(max(last_1h),last_1d):avg:system.load.1{*} >= 3"),
            metric("min(last_15m):avg:trace.http.request.duration{service:billing} < 0.5"),
            doc(
                json!({ "name": "Errors", "type": "log alert", "query": "logs(\"status:error\").index(\"*\").rollup(\"count\").by(\"service\").last(\"10m\") > 100", "options": { "thresholds": { "critical": 100, "warning": 50 }, "notify_no_data": true, "no_data_timeframe": 20 } }),
            ),
        ] {
            let m = from_datadog(&d).unwrap();
            assert_eq!(from_datadog(&to_datadog(&m)).unwrap(), m, "{d}");
        }
    }

    #[test]
    fn documents_keep_what_was_sent_and_get_datadogs_defaults() {
        let d = doc(
            json!({ "id": 5, "creator": {}, "overall_state": "OK", "name": "n", "type": "log alert", "query": "q", "options": { "silenced": { "*": null }, "renotify_interval": 60 }, "custom": 1 }),
        );
        assert_eq!(
            d,
            json!({ "name": "n", "type": "log alert", "query": "q", "custom": 1, "message": "", "tags": [], "priority": null, "restricted_roles": null, "draft_status": "published",
                "options": { "renotify_interval": 60, "notify_no_data": false, "notify_audit": false, "new_host_delay": 300, "include_tags": true, "groupby_simple_monitor": false } })
        );
        // An update replaces top-level fields and the options as a whole; the rest stays.
        let updated = normalize(json!({ "name": "renamed", "options": { "thresholds": { "critical": 2 }, "new_host_delay": null } }), Some(d)).unwrap();
        assert_eq!(updated["name"], "renamed");
        assert_eq!(updated["custom"], 1);
        assert_eq!(
            updated["options"],
            json!({ "thresholds": { "critical": 2 }, "new_host_delay": null, "notify_no_data": false, "notify_audit": false, "include_tags": true, "groupby_simple_monitor": false })
        );
    }

    #[test]
    fn edits_made_elsewhere_show_in_the_document() {
        let sent = metric("avg(last_5m):avg:system.cpu.user{env:prod} by {host} > 90.0");
        let mut d = sent.clone();
        d["options"]["thresholds"] = json!({ "critical": 90.0, "warning": 80 });
        d["options"]["renotify_interval"] = json!(30);
        let mut current = from_datadog(&d).unwrap();
        // Nothing changed: the document stays exactly as sent (90.0 is not rewritten as 90).
        let mut same = d.clone();
        reconcile(&mut same, &current);
        assert_eq!(same, d);

        current.name = "CPU (edited)".into();
        current.tags = vec!["env:prod".into()];
        current.options.thresholds.critical = 95.0;
        current.options.thresholds.warning = None;
        current.options.notify_no_data = true;
        current.priority = Some(1);
        let mut drifted = d.clone();
        reconcile(&mut drifted, &current);
        assert_eq!(drifted["name"], "CPU (edited)");
        assert_eq!(drifted["tags"], json!(["env:prod"]));
        assert_eq!(drifted["priority"], 1);
        assert_eq!(drifted["query"], "avg(last_5m):avg:system.cpu.user{env:prod} by {host} > 95");
        assert_eq!(drifted["type"], "metric alert", "same family: the type as sent");
        assert_eq!(drifted["options"]["thresholds"], json!({ "critical": 95.0 }));
        assert_eq!(drifted["options"]["notify_no_data"], true);
        assert_eq!(drifted["options"]["renotify_interval"], 30, "options we don't map stay");
        assert_eq!(from_datadog(&drifted).unwrap(), current);
    }

    #[test]
    fn numeric_ids_are_stable() {
        assert_eq!(org_number("default"), org_number("default"));
        assert_ne!(org_number("default"), org_number("acme"));
        assert!(org_number("acme") > 0 && org_number("acme") < 1 << 47);
        assert_eq!(user_number("0a1b2c3d-4e5f-6000-8000-000000000001"), 0x0a1b2c3d4e5f);
        assert_eq!(iso(0), "1970-01-01T00:00:00.000000+00:00");
    }

    #[test]
    fn search_queries() {
        assert_eq!(
            search_terms("status:alert type:log tag:\"env:prod\" checkout").unwrap(),
            vec![SearchTerm::Status("alert".into()), SearchTerm::Type("log".into()), SearchTerm::Tag("env:prod".into()), SearchTerm::Text("checkout".into())]
        );
        assert_eq!(search_terms("status:\"no data\"").unwrap(), vec![SearchTerm::Status("no data".into())]);
        assert_eq!(search_terms("status:notastatus").unwrap_err().errors, vec!["Query parsing error: 'notastatus' is not a valid monitor status"]);
    }
}

/// Against the live test databases (skipped without them).
#[cfg(test)]
pub(crate) mod live_tests {
    use super::super::replay::{new_org_with_roles, user_with_app_key, TestServer};
    use super::replay_feature;
    use reqwest::{Method, StatusCode};
    use serde_json::{json, Value};

    /// The Monitors scenarios we don't replay, and why.
    const UNSUPPORTED: &[(&str, &str)] = &[
        ("Create a Cost Monitor returns \"OK\" response", "cost alert monitors are not supported"),
        ("Create a Data Jobs monitor returns \"OK\" response", "data-jobs alert monitors are not supported"),
        ("Create a Data Quality monitor returns \"OK\" response", "data-quality alert monitors are not supported"),
        ("Create a Data Quality monitor with sensitivity returns \"OK\" response", "data-quality alert monitors are not supported"),
        ("Create a RUM formula and functions monitor returns \"OK\" response", "rum alert monitors are not supported"),
        ("Create a ci-pipelines formula and functions monitor returns \"OK\" response", "ci-pipelines alert monitors are not supported"),
        ("Create a ci-pipelines monitor returns \"OK\" response", "ci-pipelines alert monitors are not supported"),
        ("Create a ci-tests formula and functions monitor returns \"OK\" response", "ci-tests alert monitors are not supported"),
        ("Create a ci-tests monitor returns \"OK\" response", "ci-tests alert monitors are not supported"),
        ("Create an Error Tracking monitor returns \"OK\" response", "error-tracking alert monitors are not supported"),
        ("Create an LLM Observability monitor returns \"OK\" response", "llm-observability alert monitors are not supported"),
        ("Create a metric monitor returns \"OK\" response", "calendar-aligned windows (current_1mo) are not supported"),
        ("Create a metric monitor with a custom schedule returns \"OK\" response", "calendar-aligned windows (current_1mo) and custom schedules are not supported"),
        ("Create a monitor with assets returns \"OK\" response", "calendar-aligned windows (current_1mo) are not supported"),
        ("Create a monitor with aggregate augmented query variables returns \"OK\" response", "formula and function queries (options.variables) are not supported"),
        ("Create a monitor with aggregate filtered query variables returns \"OK\" response", "formula and function queries (options.variables) are not supported"),
        ("Create a monitor returns \"OK\" response", "creates a role first through /api/v2/roles (users and roles stage)"),
        ("Get a synthetics monitor's details", "creates a synthetic test through /api/v1/synthetics (not compatible yet)"),
        ("Get a monitor's details with downtime returns \"OK\" response", "its downtime ended in May 2023 (recording time), so it no longer silences the monitor"),
        ("Monitors group search returns \"OK\" response", "/api/v1/monitor/groups/search is not implemented (no provider resource uses it)"),
        ("Monitors group search returns \"Bad Request\" response", "/api/v1/monitor/groups/search is not implemented (no provider resource uses it)"),
    ];

    #[tokio::test]
    async fn monitors_cassettes_replay() {
        let Some(replayed) = replay_feature("dd-monitors-cassettes", "v1/monitors", UNSUPPORTED).await else { return };
        for name in [
            "Create a monitor returns \"Bad Request\" response",
            "Edit a monitor returns \"OK\" response",
            "Check if a monitor can be deleted returns \"OK\" response",
            "Validate an existing monitor returns \"OK\" response",
            "Monitors search returns \"OK\" response",
            "Get all monitors returns \"OK\" response with pagination",
        ] {
            assert!(replayed.iter().any(|r| r == name), "{name} not replayed: {replayed:#?}");
        }
        assert_eq!(replayed.len(), 17, "{replayed:#?}");
    }

    pub(crate) struct Client {
        base: String,
        key: String,
        http: reqwest::Client,
    }

    impl Client {
        pub(crate) fn new(s: &TestServer, key: &str) -> Self {
            Client { base: s.base.clone(), key: key.into(), http: reqwest::Client::new() }
        }

        async fn request(&self, req: reqwest::RequestBuilder, body: Option<Value>) -> (StatusCode, Value) {
            let req = match body {
                Some(b) => req.json(&b),
                None => req,
            };
            let res = req.send().await.expect("request");
            (res.status(), res.json().await.unwrap_or(Value::Null))
        }

        /// The Datadog-compatible API.
        pub(crate) async fn send(&self, method: Method, path: &str, body: Option<Value>) -> (StatusCode, Value) {
            self.request(self.http.request(method, format!("{}{path}", self.base)).header("DD-APPLICATION-KEY", &self.key), body).await
        }

        /// Our own API, with the same key as a bearer token.
        pub(crate) async fn ours(&self, method: Method, path: &str, body: Option<Value>) -> (StatusCode, Value) {
            self.request(self.http.request(method, format!("{}{path}", self.base)).bearer_auth(&self.key), body).await
        }
    }

    /// A monitor as the Terraform provider's `datadog_monitor` sends it.
    fn provider_monitor() -> Value {
        json!({
            "name": "Storefront API CPU",
            "type": "metric alert",
            "query": "avg(last_5m):avg:system.cpu.user{service:storefront-api} by {host} > 90",
            "message": "CPU is high on {{host.name}} @slack-ops @oncall@example.com",
            "tags": ["env:prod", "service:storefront-api"],
            "priority": 2,
            "restricted_roles": null,
            "draft_status": "published",
            "options": {
                "thresholds": { "critical": 90, "warning": 80 },
                "include_tags": true, "notify_no_data": true, "no_data_timeframe": 20, "require_full_window": false, "new_host_delay": 300,
                "renotify_interval": 60, "renotify_statuses": null, "escalation_message": "still high", "evaluation_delay": 60, "notify_by": ["host"],
            },
        })
    }

    #[tokio::test]
    async fn monitors_read_back_what_was_written_and_land_in_our_monitors() {
        let Some(s) = TestServer::start("dd-monitors").await else { return };
        let c = Client::new(&s, &s.app_key);
        let (status, created) = c.send(Method::POST, "/api/v1/monitor", Some(provider_monitor())).await;
        assert_eq!(status, StatusCode::OK, "{created}");
        let id = created["id"].as_i64().unwrap();
        let (_, read) = c.send(Method::GET, &format!("/api/v1/monitor/{id}"), None).await;
        for key in ["name", "type", "query", "message", "tags", "priority", "draft_status"] {
            assert_eq!(read[key], provider_monitor()[key], "{key}");
        }
        for (k, v) in provider_monitor()["options"].as_object().unwrap() {
            assert_eq!(&read["options"][k], v, "options.{k}");
        }
        assert_eq!((read["overall_state"].as_str(), read["multi"].as_bool(), read["deleted"].is_null()), (Some("No Data"), Some(true), true));
        assert_eq!(read["creator"]["email"].as_str().map(|e| e.ends_with("@example.com")), Some(true));
        assert_eq!(read["options"]["silenced"], json!({}));

        // Our own API sees the same monitor, mapped.
        let (_, ours) = c.ours(Method::GET, &format!("/api/v1/monitors/{id}"), None).await;
        assert_eq!((ours["type"].as_str(), ours["metricQuery"].as_str()), (Some("metric"), Some("avg:system.cpu.user{service:storefront-api} by {host}")));
        assert_eq!(ours["options"]["thresholds"], json!({ "critical": 90.0, "warning": 80.0, "criticalRecovery": null, "warningRecovery": null }));
        assert_eq!((ours["options"]["notifyNoData"].as_bool(), ours["options"]["noDataMinutes"].as_i64()), (Some(true), Some(20)));
        assert_eq!(ours["recipients"], json!(["slack-ops", "oncall@example.com"]));
        assert_eq!(ours["priority"], 2);

        // An edit in the UI shows in the Datadog document (Terraform sees the drift).
        let mut input = json!({ "name": "Storefront API CPU (edited)", "type": ours["type"], "query": ours["metricQuery"], "options": ours["options"], "message": ours["message"],
            "priority": 1, "tags": ours["tags"], "teams": ours["teams"], "draft": false });
        input["options"]["thresholds"]["critical"] = json!(95);
        let (status, _) = c.ours(Method::PUT, &format!("/api/v1/monitors/{id}"), Some(input)).await;
        assert_eq!(status, StatusCode::OK);
        let (_, drifted) = c.send(Method::GET, &format!("/api/v1/monitor/{id}"), None).await;
        assert_eq!(drifted["name"], "Storefront API CPU (edited)");
        assert_eq!(drifted["priority"], 1);
        assert_eq!(drifted["query"], "avg(last_5m):avg:system.cpu.user{service:storefront-api} by {host} > 95");
        assert_eq!(drifted["options"]["thresholds"], json!({ "critical": 95.0, "warning": 80 }));
        assert_eq!(drifted["options"]["escalation_message"], "still high", "options we don't map are kept");

        // Terraform puts its configuration back.
        let (status, updated) = c.send(Method::PUT, &format!("/api/v1/monitor/{id}"), Some(provider_monitor())).await;
        assert_eq!(status, StatusCode::OK, "{updated}");
        assert_eq!((updated["name"].as_str(), updated["query"].as_str()), (provider_monitor()["name"].as_str(), provider_monitor()["query"].as_str()));
        let (_, ours) = c.ours(Method::GET, &format!("/api/v1/monitors/{id}"), None).await;
        assert_eq!((ours["name"].as_str(), ours["options"]["thresholds"]["critical"].as_f64()), (Some("Storefront API CPU"), Some(90.0)));

        // A monitor made in the UI reads as a Datadog document.
        let ui = json!({ "name": "Checkout errors", "type": "log", "query": "logs(\"service:checkout status:error\") by {service}", "message": "@slack-checkout", "tags": ["team:payments"],
            "options": { "detection": "threshold", "timeAggregation": "sum", "windowMinutes": 10, "comparator": "above", "thresholds": { "critical": 50 }, "notifyNoData": false, "noDataMinutes": 10 } });
        let (status, ui_monitor) = c.ours(Method::POST, "/api/v1/monitors", Some(ui)).await;
        assert_eq!(status, StatusCode::OK, "{ui_monitor}");
        let (_, doc) = c.send(Method::GET, &format!("/api/v1/monitor/{}", ui_monitor["id"]), None).await;
        assert_eq!(
            (doc["type"].as_str(), doc["query"].as_str()),
            (Some("log alert"), Some("logs(\"service:checkout status:error\").index(\"*\").rollup(\"count\").by(\"service\").last(\"10m\") > 50"))
        );

        // Lists, as the data sources filter them.
        let (_, all) = c.send(Method::GET, "/api/v1/monitor", None).await;
        assert_eq!(all.as_array().map(Vec::len), Some(2));
        assert!(all[0]["matching_downtimes"].is_array(), "with_downtimes is on by default");
        let (_, named) = c.send(Method::GET, "/api/v1/monitor?name=storefront", None).await;
        assert_eq!(named.as_array().map(|a| a.iter().map(|m| m["id"].as_i64()).collect::<Vec<_>>()), Some(vec![Some(id)]));
        let (_, tagged) = c.send(Method::GET, "/api/v1/monitor?monitor_tags=team:payments", None).await;
        assert_eq!(tagged.as_array().map(Vec::len), Some(1));
        let (_, scoped) = c.send(Method::GET, "/api/v1/monitor?tags=service:storefront-api", None).await;
        assert_eq!(scoped[0]["id"], id);

        let (status, gone) = c.send(Method::DELETE, &format!("/api/v1/monitor/{id}"), None).await;
        assert_eq!((status, gone), (StatusCode::OK, json!({ "deleted_monitor_id": id })));
        assert_eq!(c.send(Method::GET, &format!("/api/v1/monitor/{id}"), None).await, (StatusCode::NOT_FOUND, json!({ "errors": ["Monitor not found"] })));
    }

    #[tokio::test]
    async fn monitors_we_cant_represent_are_rejected_and_roles_and_orgs_are_enforced() {
        let Some(s) = TestServer::start("dd-monitors-rules").await else { return };
        let c = Client::new(&s, &s.app_key);
        let composite = json!({ "name": "both", "type": "composite", "query": "1 && 2" });
        assert_eq!(
            c.send(Method::POST, "/api/v1/monitor", Some(composite.clone())).await,
            (StatusCode::BAD_REQUEST, json!({ "errors": ["monitor type 'composite' is not supported by ObserveCraft yet (supported: metric alert, query alert, log alert)"] }))
        );
        assert_eq!(c.send(Method::POST, "/api/v1/monitor/validate", Some(composite)).await.0, StatusCode::BAD_REQUEST);
        assert_eq!(c.send(Method::POST, "/api/v1/monitor/validate", Some(provider_monitor())).await, (StatusCode::OK, json!({})));
        let (status, body) = c.send(Method::POST, "/api/v1/monitor", None).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(body["errors"][0].as_str().unwrap().starts_with("Invalid JSON"), "{body}");

        let (_, created) = c.send(Method::POST, "/api/v1/monitor", Some(provider_monitor())).await;
        let id = created["id"].as_i64().unwrap();
        let mut log = provider_monitor();
        log["type"] = json!("log alert");
        assert!(c.send(Method::PUT, &format!("/api/v1/monitor/{id}"), Some(log)).await.1["errors"][0].as_str().unwrap().contains("type cannot be changed"));

        // Read-only users read but don't write.
        let (_, reader_key) = user_with_app_key(&s.st, &s.org, "readonly").await;
        let reader = Client::new(&s, &reader_key);
        assert_eq!(reader.send(Method::GET, &format!("/api/v1/monitor/{id}"), None).await.0, StatusCode::OK);
        assert_eq!(reader.send(Method::POST, "/api/v1/monitor", Some(provider_monitor())).await.0, StatusCode::FORBIDDEN);
        assert_eq!(reader.send(Method::DELETE, &format!("/api/v1/monitor/{id}"), None).await.0, StatusCode::FORBIDDEN);

        // Another org's key sees nothing.
        let other_org = new_org_with_roles(&s.st, "dd-monitors-other").await;
        let (_, other_key) = user_with_app_key(&s.st, &other_org, "admin").await;
        let other = Client::new(&s, &other_key);
        assert_eq!(other.send(Method::GET, &format!("/api/v1/monitor/{id}"), None).await.0, StatusCode::NOT_FOUND);
        assert_eq!(other.send(Method::DELETE, &format!("/api/v1/monitor/{id}"), None).await.0, StatusCode::NOT_FOUND);
        assert_eq!(other.send(Method::GET, "/api/v1/monitor", None).await.1, json!([]));

        // A monitor an SLO uses needs force to go.
        sqlx::query("INSERT INTO slos (org_id, name, type, monitor_ids, targets, primary_window) VALUES ($1, 'Checkout availability', 'monitor', $2, '[]', '30d')")
            .bind(&s.org)
            .bind(vec![id])
            .execute(&s.st.postgres)
            .await
            .unwrap();
        let (status, body) = c.send(Method::GET, &format!("/api/v1/monitor/can_delete?monitor_ids={id}"), None).await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(body["data"]["ok"], json!([]));
        assert!(body["errors"][id.to_string()][0].as_str().unwrap().contains("Checkout availability"));
        assert_eq!(c.send(Method::DELETE, &format!("/api/v1/monitor/{id}"), None).await.0, StatusCode::BAD_REQUEST);
        assert_eq!(c.send(Method::DELETE, &format!("/api/v1/monitor/{id}?force=true"), None).await.0, StatusCode::OK);
    }
}
