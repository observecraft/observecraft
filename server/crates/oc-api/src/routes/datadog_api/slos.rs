//! Datadog-compatible service level objectives (`/api/v1/slo`), for the Terraform provider's
//! `datadog_service_level_objective` resource and `datadog_service_level_objective(s)` data sources and
//! Datadog's API clients.
//!
//! The documents translate onto our SLOs (`routes::slos`, the same `slos` table, so they show in the
//! UI): metric SLOs (`query.numerator` / `denominator`, or `sli_specification.count` with named metric
//! queries and good/total or good/bad formulas), monitor SLOs (`monitor_ids`) and time-slice SLOs
//! (`sli_specification.time_slice`, 1- or 5-minute slices); thresholds per `7d`, `30d` and `90d`
//! timeframe, the primary `timeframe`, `tags` and `description`.
//!
//! The document is stored as received (`datadog_slo_documents`) and returned on reads with the
//! server-owned fields (`id`: our UUID as 32 hex characters, `type_id`, `created_at`, `modified_at`,
//! `creator`). When the SLO was changed elsewhere (the UI or our own API), the parts of the document
//! our model holds are taken from the SLO instead, so a Terraform plan shows the drift.
//!
//! Not translated: `custom` timeframes, non-metric data sources and `.as_rate()` in queries (rejected
//! with 400); `groups` of monitor SLOs (stored and returned, the SLO counts every group). Metric
//! queries' `.as_count()` is how our metric SLOs count anyway, and a query without an aggregator
//! (`trace.servlet.request{env:prod}`) is averaged, as Datadog does.

use std::collections::HashMap;

use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::routing::get;
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use utoipa::{IntoParams, ToSchema};

use super::{DdAuth, DdError};
use crate::auth::{Access, AuthContext};
use crate::routes::monitor_formula::parse_formula;
use crate::routes::slos::{self as model, SloComparator, SloInput, SloStatus, SloTarget, SloType, SloWindow};
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new().route("/api/v1/slo", get(list).post(create)).route("/api/v1/slo/search", get(search)).route("/api/v1/slo/:id", get(get_one).put(update).delete(delete))
}

/// Datadog's timeframes we measure (`custom` is not).
const TIMEFRAMES: [SloWindow; 3] = [SloWindow::SevenDays, SloWindow::ThirtyDays, SloWindow::NinetyDays];
/// Fields the server owns: never stored, always filled in on reads.
const SERVER_FIELDS: [&str; 6] = ["id", "type_id", "created_at", "modified_at", "creator", "monitor_tags"];

// ================================================================ ids

/// Datadog's form of an SLO id: our UUID as 32 lowercase hex characters.
pub fn hex_id(uuid: &str) -> String {
    uuid.replace('-', "").to_lowercase()
}

/// Our UUID (with dashes) of a Datadog SLO id (32 hex characters, or a UUID), or None.
pub fn uuid_of(id: &str) -> Option<String> {
    let hex: String = id.trim().chars().filter(|c| *c != '-').collect::<String>().to_lowercase();
    if hex.len() != 32 || !hex.chars().all(|c| c.is_ascii_hexdigit()) || (id.contains('-') && id.trim().len() != 36) {
        return None;
    }
    Some(format!("{}-{}-{}-{}-{}", &hex[0..8], &hex[8..12], &hex[12..16], &hex[16..20], &hex[20..]))
}

// ================================================================ document → our model

fn invalid(message: impl std::fmt::Display) -> String {
    format!("Invalid payload: {message}")
}

fn window_of(timeframe: &str, path: &str) -> Result<SloWindow, String> {
    match timeframe {
        "custom" => Err(invalid(format!("{path}: custom timeframes are not supported (7d, 30d and 90d are)"))),
        t => SloWindow::parse(t).filter(|w| TIMEFRAMES.contains(w)).ok_or_else(|| invalid(format!("{path}: unknown timeframe '{t}' (7d, 30d and 90d are supported)"))),
    }
}

fn number(v: &Value, path: &str) -> Result<Option<f64>, String> {
    match v {
        Value::Null => Ok(None),
        Value::Number(n) => Ok(n.as_f64()),
        _ => Err(invalid(format!("{path} must be a number"))),
    }
}

fn string_list(v: &Value, path: &str) -> Result<Vec<String>, String> {
    match v {
        Value::Null => Ok(vec![]),
        Value::Array(items) => items.iter().map(|i| i.as_str().map(str::to_string).ok_or_else(|| invalid(format!("{path} must be a list of strings")))).collect(),
        _ => Err(invalid(format!("{path} must be a list of strings"))),
    }
}

/// A metric query as our metric SLOs read it: `.as_count()` is how they count already, a query
/// without an aggregator is averaged (Datadog's default), `.as_rate()` would change the meaning.
fn metric_query(q: &str, path: &str) -> Result<String, String> {
    if q.contains(".as_rate()") {
        return Err(invalid(format!("{path}: .as_rate() is not supported in SLO queries (count events with .as_count())")));
    }
    let q = q.replace(".as_count()", "");
    let q = q.trim();
    let has_agg = q.find('{').is_some_and(|brace| q[..brace].contains(':'));
    Ok(if has_agg || q.is_empty() { q.to_string() } else { format!("avg:{q}") })
}

/// The named metric queries of an `sli_specification` (`[{data_source, name, query}]`).
fn named_queries(v: &Value, path: &str) -> Result<Vec<(String, String)>, String> {
    let Some(items) = v.as_array().filter(|i| !i.is_empty()) else { return Err(invalid(format!("{path} must list at least one query"))) };
    items
        .iter()
        .enumerate()
        .map(|(i, q)| {
            let at = format!("{path}[{i}]");
            let source = q.get("data_source").and_then(Value::as_str).unwrap_or("metrics");
            if source != "metrics" {
                return Err(invalid(format!("{at}: only metrics queries are supported (data_source '{source}')")));
            }
            let name = q.get("name").and_then(Value::as_str).filter(|n| !n.is_empty()).ok_or_else(|| invalid(format!("{at}.name is required")))?;
            let query = q.get("query").and_then(Value::as_str).ok_or_else(|| invalid(format!("{at}.query is required")))?;
            Ok((name.to_string(), metric_query(query, &at)?))
        })
        .collect()
}

/// A formula over named queries with each name replaced by its query.
fn substitute(formula: &str, queries: &[(String, String)]) -> String {
    let mut out = String::new();
    let mut word = String::new();
    let flush = |word: &mut String, out: &mut String| {
        match queries.iter().find(|(name, _)| name == word) {
            Some((_, q)) => out.push_str(q),
            None => out.push_str(word),
        }
        word.clear();
    };
    for c in formula.trim().chars() {
        if c.is_ascii_alphanumeric() || c == '_' {
            word.push(c);
        } else {
            flush(&mut word, &mut out);
            out.push(c);
        }
    }
    flush(&mut word, &mut out);
    out
}

fn formula_text(v: &Value, path: &str) -> Result<Option<String>, String> {
    match v {
        Value::Null => Ok(None),
        f => f.get("formula").and_then(Value::as_str).map(|s| Some(s.to_string())).ok_or_else(|| invalid(format!("{path}.formula is required"))),
    }
}

/// Our SLO input from a Datadog SLO document (not yet validated by [`model::validate`]).
pub fn to_input(doc: &Value) -> Result<SloInput, String> {
    let Some(obj) = doc.as_object() else { return Err(invalid("the body must be a JSON object")) };
    let get = |k: &str| obj.get(k).unwrap_or(&Value::Null);
    let name = get("name").as_str().ok_or_else(|| invalid("name is required"))?.to_string();
    let kind = match get("type").as_str() {
        Some("metric") => SloType::Metric,
        Some("monitor") => SloType::Monitor,
        Some("time_slice") => SloType::TimeSlice,
        Some(other) => return Err(invalid(format!("unknown SLO type '{other}' (metric, monitor and time_slice are supported)"))),
        None => return Err(invalid("type is required")),
    };
    let description = match get("description") {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        _ => return Err(invalid("description must be a string")),
    };
    let tags = string_list(get("tags"), "tags")?;

    let Some(thresholds) = get("thresholds").as_array().filter(|t| !t.is_empty()) else { return Err(invalid("thresholds must list at least one threshold")) };
    let mut targets = Vec::new();
    for (i, t) in thresholds.iter().enumerate() {
        let at = format!("thresholds[{i}]");
        let window = window_of(t.get("timeframe").and_then(Value::as_str).ok_or_else(|| invalid(format!("{at}.timeframe is required")))?, &format!("{at}.timeframe"))?;
        let target = number(t.get("target").unwrap_or(&Value::Null), &format!("{at}.target"))?.ok_or_else(|| invalid(format!("{at}.target is required")))?;
        let warning = number(t.get("warning").unwrap_or(&Value::Null), &format!("{at}.warning"))?;
        targets.push(SloTarget { window, target, warning });
    }
    let primary = match get("timeframe") {
        Value::Null => None,
        Value::String(t) => Some(window_of(t, "timeframe")?),
        _ => return Err(invalid("timeframe must be a string")),
    };
    let primary_target = targets.iter().find(|t| Some(t.window) == primary).or(targets.first()).copied().unwrap_or(targets[0]);
    if primary.is_some_and(|p| p != primary_target.window) {
        return Err(invalid("timeframe must be the timeframe of one of the thresholds"));
    }
    let close = |a: f64, b: f64| (a - b).abs() < 1e-9;
    if let Some(t) = number(get("target_threshold"), "target_threshold")? {
        if !close(t, primary_target.target) {
            return Err(invalid(format!("target_threshold must match the target of the {} threshold", primary_target.window.as_str())));
        }
    }
    if let Some(w) = number(get("warning_threshold"), "warning_threshold")? {
        if !primary_target.warning.is_some_and(|pw| close(pw, w)) {
            return Err(invalid(format!("warning_threshold must match the warning of the {} threshold", primary_target.window.as_str())));
        }
    }

    let mut input = SloInput {
        name,
        description,
        r#type: kind,
        good_query: None,
        total_query: None,
        monitor_ids: vec![],
        slice_query: None,
        comparator: None,
        threshold: None,
        slice_minutes: None,
        targets,
        primary_window: primary,
        tags,
        teams: vec![],
    };
    let spec = get("sli_specification");
    match kind {
        SloType::Metric if !spec.is_null() => {
            let count = spec.get("count").ok_or_else(|| invalid("the sli_specification of a metric SLO is a count"))?;
            let queries = named_queries(count.get("queries").unwrap_or(&Value::Null), "sli_specification.count.queries")?;
            let good = formula_text(count.get("good_events_formula").unwrap_or(&Value::Null), "sli_specification.count.good_events_formula")?
                .ok_or_else(|| invalid("sli_specification.count.good_events_formula is required"))?;
            let good = substitute(&good, &queries);
            let total = formula_text(count.get("total_events_formula").unwrap_or(&Value::Null), "sli_specification.count.total_events_formula")?;
            let bad = formula_text(count.get("bad_events_formula").unwrap_or(&Value::Null), "sli_specification.count.bad_events_formula")?;
            input.total_query = Some(match (total, bad) {
                (Some(total), None) => substitute(&total, &queries),
                (None, Some(bad)) => format!("({good}) + ({})", substitute(&bad, &queries)),
                _ => return Err(invalid("sli_specification.count needs either total_events_formula or bad_events_formula")),
            });
            input.good_query = Some(good);
        }
        SloType::Metric => {
            let q = get("query");
            let (Some(numerator), Some(denominator)) = (q.get("numerator").and_then(Value::as_str), q.get("denominator").and_then(Value::as_str)) else {
                return Err(invalid("must specify the query for count types"));
            };
            input.good_query = Some(metric_query(numerator, "query.numerator")?);
            input.total_query = Some(metric_query(denominator, "query.denominator")?);
        }
        SloType::Monitor => {
            let ids = get("monitor_ids").as_array().filter(|i| !i.is_empty()).ok_or_else(|| invalid("must specify monitor_ids"))?;
            input.monitor_ids = ids.iter().map(|i| i.as_i64().ok_or_else(|| invalid("monitor_ids must be integers"))).collect::<Result<_, _>>()?;
        }
        SloType::TimeSlice => {
            let slice = spec.get("time_slice").ok_or_else(|| invalid("a time_slice SLO needs sli_specification.time_slice"))?;
            let query = slice.get("query").unwrap_or(&Value::Null);
            let queries = named_queries(query.get("queries").unwrap_or(&Value::Null), "sli_specification.time_slice.query.queries")?;
            let formulas =
                query.get("formulas").and_then(Value::as_array).filter(|f| f.len() == 1).ok_or_else(|| invalid("sli_specification.time_slice.query.formulas must hold exactly one formula"))?;
            let formula = formula_text(&formulas[0], "sli_specification.time_slice.query.formulas[0]")?.ok_or_else(|| invalid("sli_specification.time_slice.query.formulas[0].formula is required"))?;
            input.slice_query = Some(substitute(&formula, &queries));
            let comparator = slice.get("comparator").and_then(Value::as_str).unwrap_or_default();
            input.comparator = Some(SloComparator::parse(comparator).ok_or_else(|| invalid(format!("unknown comparator '{comparator}' (<, <=, > and >= are supported)")))?);
            input.threshold =
                Some(number(slice.get("threshold").unwrap_or(&Value::Null), "sli_specification.time_slice.threshold")?.ok_or_else(|| invalid("sli_specification.time_slice.threshold is required"))?);
            input.slice_minutes = Some(match slice.get("query_interval_seconds").and_then(Value::as_i64).unwrap_or(300) {
                60 => 1,
                300 => 5,
                other => return Err(invalid(format!("query_interval_seconds is 60 or 300, not {other}"))),
            });
        }
    }
    Ok(input)
}

/// [`to_input`] and our SLO validation; errors are Datadog's 400.
pub fn checked_input(doc: &Value) -> Result<SloInput, String> {
    let mut input = to_input(doc)?;
    model::validate(&mut input).map_err(invalid)?;
    Ok(input)
}

// ================================================================ our model → document

fn display(v: f64) -> String {
    format!("{v}")
}

/// Datadog thresholds of our targets (without our 1-day window, which Datadog doesn't have).
fn thresholds_of(input: &SloInput) -> Vec<Value> {
    input
        .targets
        .iter()
        .filter(|t| TIMEFRAMES.contains(&t.window))
        .map(|t| {
            let mut v = json!({ "timeframe": t.window.as_str(), "target": t.target });
            if let Some(w) = t.warning {
                v["warning"] = json!(w);
            }
            v
        })
        .collect()
}

/// A named-query form (`query1`, `query2`, … and a formula over them) of one of our query texts.
fn named_form(text: &str) -> (String, Vec<Value>) {
    let Ok(f) = parse_formula(text) else { return ("query1".into(), vec![json!({ "data_source": "metrics", "name": "query1", "query": text })]) };
    let trimmed = text.trim();
    let mut formula = String::new();
    let mut at = 0;
    for (i, (start, end)) in f.spans.iter().enumerate() {
        formula.push_str(&trimmed[at..*start]);
        formula.push_str(&format!("query{}", i + 1));
        at = *end;
    }
    formula.push_str(&trimmed[at..]);
    let queries = f.queries.iter().enumerate().map(|(i, q)| json!({ "data_source": "metrics", "name": format!("query{}", i + 1), "query": q })).collect();
    (formula, queries)
}

/// The definition fields of a document (`query`, `monitor_ids` or `sli_specification`) for our SLO.
fn definition_of(input: &SloInput) -> Map<String, Value> {
    let mut m = Map::new();
    match input.r#type {
        SloType::Metric => {
            m.insert("query".into(), json!({ "numerator": input.good_query, "denominator": input.total_query }));
        }
        SloType::Monitor => {
            m.insert("monitor_ids".into(), json!(input.monitor_ids));
        }
        SloType::TimeSlice => {
            let (formula, queries) = named_form(input.slice_query.as_deref().unwrap_or_default());
            m.insert(
                "sli_specification".into(),
                json!({ "time_slice": {
                    "comparator": input.comparator.map(SloComparator::as_str),
                    "threshold": input.threshold,
                    "query": { "formulas": [{ "formula": formula }], "queries": queries },
                    "query_interval_seconds": input.slice_minutes.unwrap_or(5) * 60,
                }}),
            );
        }
    }
    m
}

/// The primary-timeframe fields of a document for our SLO (none when its primary window is 1 day).
fn primary_of(input: &SloInput) -> Map<String, Value> {
    let mut m = Map::new();
    if let Some(t) = input.primary_window.filter(|w| TIMEFRAMES.contains(w)).and_then(|w| input.targets.iter().find(|t| t.window == w)) {
        m.insert("timeframe".into(), json!(t.window.as_str()));
        m.insert("target_threshold".into(), json!(t.target));
        if let Some(w) = t.warning {
            m.insert("warning_threshold".into(), json!(w));
        }
    }
    m
}

/// A Datadog document for one of our SLOs.
pub fn from_input(input: &SloInput) -> Value {
    let mut doc = Map::new();
    doc.insert("name".into(), json!(input.name));
    doc.insert("description".into(), json!(input.description));
    doc.insert("type".into(), json!(input.r#type.as_str()));
    doc.insert("tags".into(), json!(input.tags));
    doc.insert("thresholds".into(), Value::Array(thresholds_of(input)));
    doc.extend(primary_of(input));
    doc.extend(definition_of(input));
    Value::Object(doc)
}

fn same_tags(a: &[String], b: &[String]) -> bool {
    let mut a = a.to_vec();
    let mut b = b.to_vec();
    a.sort();
    b.sort();
    a == b
}

fn same_definition(a: &SloInput, b: &SloInput) -> bool {
    a.r#type == b.r#type
        && a.good_query == b.good_query
        && a.total_query == b.total_query
        && a.monitor_ids == b.monitor_ids
        && a.slice_query == b.slice_query
        && a.comparator == b.comparator
        && a.threshold == b.threshold
        && a.slice_minutes == b.slice_minutes
}

/// The document to return for an SLO: the stored one, with the parts our model holds taken from the
/// SLO where it changed since (an edit in the UI); a derived one when there is none.
pub fn reconcile(stored: Option<&Value>, current: &SloInput) -> Value {
    let Some(stored) = stored.filter(|d| d.is_object()) else { return from_input(current) };
    let Ok(expected) = checked_input(stored) else { return from_input(current) };
    let mut doc = stored.as_object().cloned().unwrap_or_default();
    if expected.name != current.name {
        doc.insert("name".into(), json!(current.name));
    }
    if expected.description != current.description {
        doc.insert("description".into(), json!(current.description));
    }
    if !same_tags(&expected.tags, &current.tags) {
        doc.insert("tags".into(), json!(current.tags));
    }
    if expected.targets != current.targets || expected.primary_window != current.primary_window {
        for k in ["timeframe", "target_threshold", "warning_threshold"] {
            doc.remove(k);
        }
        doc.insert("thresholds".into(), Value::Array(thresholds_of(current)));
        doc.extend(primary_of(current));
    }
    if !same_definition(&expected, current) {
        for k in ["query", "monitor_ids", "sli_specification"] {
            doc.remove(k);
        }
        doc.insert("type".into(), json!(current.r#type.as_str()));
        doc.extend(definition_of(current));
    }
    Value::Object(doc)
}

/// What the server adds to a document on reads.
pub struct SloServerFields {
    pub id: String,
    pub created_at: i64,
    pub modified_at: i64,
    /// (name, email) of the creator.
    pub creator: Option<(String, String)>,
}

/// A document as Datadog returns it: server-owned fields, the defaults Datadog fills in
/// (`description`, `tags`, display strings, the primary timeframe's thresholds) and, for count
/// specifications, the numerator and denominator they amount to.
pub fn respond(doc: &Value, current: &SloInput, server: &SloServerFields) -> Value {
    let mut d = doc.as_object().cloned().unwrap_or_default();
    let type_id = match current.r#type {
        SloType::Monitor => 0,
        SloType::Metric => 1,
        SloType::TimeSlice => 2,
    };
    d.insert("id".into(), json!(server.id));
    d.insert("type_id".into(), json!(type_id));
    d.insert("created_at".into(), json!(server.created_at));
    d.insert("modified_at".into(), json!(server.modified_at));
    d.insert("creator".into(), server.creator.as_ref().map_or(Value::Null, |(name, email)| json!({ "name": name, "handle": email, "email": email })));
    d.insert("monitor_tags".into(), json!([]));
    for (k, empty) in [("description", json!("")), ("tags", json!([]))] {
        if d.get(k).is_none_or(Value::is_null) {
            d.insert(k.into(), empty);
        }
    }
    if let Some(Value::Array(thresholds)) = d.get_mut("thresholds") {
        for t in thresholds.iter_mut().filter_map(Value::as_object_mut) {
            for (value, shown) in [("target", "target_display"), ("warning", "warning_display")] {
                if let Some(v) = t.get(value).and_then(Value::as_f64) {
                    t.entry(shown).or_insert_with(|| json!(display(v)));
                }
            }
        }
    }
    if !d.contains_key("timeframe") {
        d.extend(primary_of(current));
    } else if !d.contains_key("target_threshold") {
        for (k, v) in primary_of(current) {
            d.entry(k).or_insert(v);
        }
    }
    if current.r#type == SloType::Metric && !d.contains_key("query") {
        d.insert("query".into(), json!({ "numerator": current.good_query, "denominator": current.total_query }));
    }
    if let Some(slice) = d.get_mut("sli_specification").and_then(|s| s.get_mut("time_slice")).and_then(Value::as_object_mut) {
        slice.entry("query_interval_seconds").or_insert(json!(current.slice_minutes.unwrap_or(5) * 60));
    }
    Value::Object(d)
}

/// The document to store: as received, without server-owned fields.
pub fn to_store(doc: &Value) -> Value {
    let mut d = doc.as_object().cloned().unwrap_or_default();
    for k in SERVER_FIELDS {
        d.remove(k);
    }
    Value::Object(d)
}

// ================================================================ storage

#[derive(sqlx::FromRow)]
struct Stored {
    id: String,
    name: String,
    description: String,
    kind: String,
    good_query: Option<String>,
    total_query: Option<String>,
    monitor_ids: Vec<i64>,
    slice_query: Option<String>,
    comparator: Option<String>,
    threshold: Option<f64>,
    slice_minutes: Option<i32>,
    targets: Value,
    primary_window: String,
    tags: Vec<String>,
    teams: Vec<String>,
    created_at: i64,
    updated_at: i64,
    creator_name: Option<String>,
    creator_email: Option<String>,
    document: Option<Value>,
}

const SELECT: &str = "SELECT s.id::text AS id, s.name, s.description, s.type AS kind, s.good_query, s.total_query, s.monitor_ids, s.slice_query, s.comparator, \
    s.threshold, s.slice_minutes, s.targets, s.primary_window, s.tags, s.teams, \
    extract(epoch FROM s.created_at)::bigint AS created_at, extract(epoch FROM s.updated_at)::bigint AS updated_at, \
    u.name AS creator_name, u.email AS creator_email, d.document \
    FROM slos s LEFT JOIN users u ON u.id = s.created_by AND u.org_id = s.org_id \
    LEFT JOIN datadog_slo_documents d ON d.org_id = s.org_id AND d.slo_id = s.id";

impl Stored {
    fn input(&self) -> SloInput {
        SloInput {
            name: self.name.clone(),
            description: self.description.clone(),
            r#type: SloType::parse(&self.kind),
            good_query: self.good_query.clone(),
            total_query: self.total_query.clone(),
            monitor_ids: self.monitor_ids.clone(),
            slice_query: self.slice_query.clone(),
            comparator: self.comparator.as_deref().and_then(SloComparator::parse),
            threshold: self.threshold,
            slice_minutes: self.slice_minutes,
            targets: serde_json::from_value(self.targets.clone()).unwrap_or_default(),
            primary_window: SloWindow::parse(&self.primary_window),
            tags: self.tags.clone(),
            teams: self.teams.clone(),
        }
    }

    fn server(&self) -> SloServerFields {
        SloServerFields {
            id: hex_id(&self.id),
            created_at: self.created_at,
            modified_at: self.updated_at,
            creator: self.creator_email.clone().map(|e| (self.creator_name.clone().unwrap_or_default(), e)),
        }
    }

    /// The document as Datadog returns it.
    fn document(&self) -> Value {
        let current = self.input();
        respond(&reconcile(self.document.as_ref(), &current), &current, &self.server())
    }
}

fn not_found(id: &str) -> DdError {
    DdError::new(StatusCode::NOT_FOUND, format!("SLO not found: {id} not found"))
}

async fn load(st: &AppState, org: &str, id: &str) -> Result<Stored, DdError> {
    let Some(uuid) = uuid_of(id) else { return Err(not_found(id)) };
    sqlx::query_as::<_, Stored>(&format!("{SELECT} WHERE s.org_id = $1 AND s.id = $2::uuid")).bind(org).bind(&uuid).fetch_optional(&st.postgres).await?.ok_or_else(|| not_found(id))
}

async fn load_all(st: &AppState, org: &str) -> Result<Vec<Stored>, DdError> {
    Ok(sqlx::query_as::<_, Stored>(&format!("{SELECT} WHERE s.org_id = $1 ORDER BY s.created_at, s.id")).bind(org).fetch_all(&st.postgres).await?)
}

async fn store_document(conn: &mut sqlx::PgConnection, org: &str, id: &str, doc: &Value) -> Result<(), DdError> {
    sqlx::query("INSERT INTO datadog_slo_documents (org_id, slo_id, document) VALUES ($1, $2::uuid, $3) ON CONFLICT (org_id, slo_id) DO UPDATE SET document = EXCLUDED.document")
        .bind(org)
        .bind(id)
        .bind(to_store(doc))
        .execute(conn)
        .await?;
    Ok(())
}

/// Writes need the Standard or Admin role, as in our API.
pub(crate) fn writer(ctx: &AuthContext) -> Result<(), DdError> {
    if ctx.access < Access::Standard {
        return Err(DdError::forbidden());
    }
    Ok(())
}

/// A JSON body, with Datadog's 400 when it isn't one.
pub(crate) fn json_body(body: &Bytes) -> Result<Value, DdError> {
    serde_json::from_slice(body).map_err(|e| DdError::new(StatusCode::BAD_REQUEST, format!("Invalid payload: the body is not valid JSON ({e})")))
}

// ================================================================ handlers

/// `{"data": [SLO], "error": null}` (create, update, list) or `{"data": SLO, "error": null}` (get).
#[derive(Serialize, ToSchema)]
pub struct DatadogSloResponse {
    pub data: Value,
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Value>,
}

fn respond_with(data: Value) -> Json<DatadogSloResponse> {
    Json(DatadogSloResponse { data, error: None, metadata: None })
}

/// Create an SLO (Datadog-compatible).
#[utoipa::path(post, path = "/api/v1/slo", operation_id = "datadog_create_slo", request_body = Value,
    responses((status = 200, body = DatadogSloResponse), (status = 400, body = super::DatadogErrors), (status = 403, body = super::DatadogErrors)), tag = "datadog-api")]
pub async fn create(State(st): State<AppState>, DdAuth(ctx): DdAuth, body: Bytes) -> Result<Json<DatadogSloResponse>, DdError> {
    writer(&ctx)?;
    let doc = json_body(&body)?;
    let input = checked_input(&doc).map_err(|e| DdError::new(StatusCode::BAD_REQUEST, e))?;
    let input = model::checked(&st, &ctx.org_id, input).await?;
    let mut tx = st.postgres.begin().await?;
    let id = model::insert_slo(&mut tx, &ctx.org_id, &ctx.user_id, &input).await?;
    store_document(&mut tx, &ctx.org_id, &id, &doc).await?;
    tx.commit().await?;
    model::forget(&ctx.org_id);
    ctx.audit(&st, "slo.create", &format!("slo:{id}"), &input.name).await;
    let stored = load(&st, &ctx.org_id, &id).await?;
    Ok(respond_with(json!([stored.document()])))
}

/// Get an SLO (Datadog-compatible).
#[utoipa::path(get, path = "/api/v1/slo/{id}", operation_id = "datadog_get_slo", params(("id" = String, Path)),
    responses((status = 200, body = DatadogSloResponse), (status = 404, body = super::DatadogErrors)), tag = "datadog-api")]
pub async fn get_one(State(st): State<AppState>, DdAuth(ctx): DdAuth, Path(id): Path<String>) -> Result<Json<DatadogSloResponse>, DdError> {
    Ok(respond_with(load(&st, &ctx.org_id, &id).await?.document()))
}

/// Replace an SLO (Datadog-compatible). Its type can't change, as in Datadog.
#[utoipa::path(put, path = "/api/v1/slo/{id}", operation_id = "datadog_update_slo", params(("id" = String, Path)), request_body = Value,
    responses((status = 200, body = DatadogSloResponse), (status = 400, body = super::DatadogErrors), (status = 404, body = super::DatadogErrors)), tag = "datadog-api")]
pub async fn update(State(st): State<AppState>, DdAuth(ctx): DdAuth, Path(id): Path<String>, body: Bytes) -> Result<Json<DatadogSloResponse>, DdError> {
    writer(&ctx)?;
    let existing = load(&st, &ctx.org_id, &id).await?;
    let doc = json_body(&body)?;
    let mut input = checked_input(&doc).map_err(|e| DdError::new(StatusCode::BAD_REQUEST, e))?;
    if input.r#type.as_str() != existing.kind {
        return Err(DdError::new(StatusCode::BAD_REQUEST, invalid(format!("the type of an SLO can't change (it is {})", existing.kind))));
    }
    // Teams are ours (not in Datadog's document): kept.
    input.teams = existing.teams.clone();
    let input = model::checked(&st, &ctx.org_id, input).await?;
    let mut tx = st.postgres.begin().await?;
    if !model::update_slo(&mut tx, &ctx.org_id, &existing.id, &input).await? {
        return Err(not_found(&id));
    }
    store_document(&mut tx, &ctx.org_id, &existing.id, &doc).await?;
    tx.commit().await?;
    model::forget(&ctx.org_id);
    ctx.audit(&st, "slo.update", &format!("slo:{}", existing.id), &input.name).await;
    let stored = load(&st, &ctx.org_id, &existing.id).await?;
    Ok(respond_with(json!([stored.document()])))
}

#[derive(Deserialize, IntoParams)]
pub struct DatadogDeleteSloParams {
    /// Accepted for compatibility: nothing here blocks deleting an SLO.
    #[allow(dead_code)]
    pub force: Option<String>,
}

/// Delete an SLO and its corrections (Datadog-compatible).
#[utoipa::path(delete, path = "/api/v1/slo/{id}", operation_id = "datadog_delete_slo", params(("id" = String, Path), DatadogDeleteSloParams),
    responses((status = 200, body = DatadogSloResponse), (status = 404, body = super::DatadogErrors)), tag = "datadog-api")]
pub async fn delete(State(st): State<AppState>, DdAuth(ctx): DdAuth, Path(id): Path<String>, Query(_params): Query<DatadogDeleteSloParams>) -> Result<Json<DatadogSloResponse>, DdError> {
    writer(&ctx)?;
    let Some(uuid) = uuid_of(&id) else { return Err(not_found(&id)) };
    let name: Option<(String,)> = sqlx::query_as("DELETE FROM slos WHERE org_id = $1 AND id = $2::uuid RETURNING name").bind(&ctx.org_id).bind(&uuid).fetch_optional(&st.postgres).await?;
    let Some((name,)) = name else { return Err(not_found(&id)) };
    model::forget(&ctx.org_id);
    ctx.audit(&st, "slo.delete", &format!("slo:{uuid}"), &name).await;
    Ok(respond_with(json!([hex_id(&uuid)])))
}

#[derive(Deserialize, IntoParams, Default)]
#[serde(default)]
pub struct DatadogListSlosParams {
    /// Comma-separated SLO ids.
    pub ids: Option<String>,
    /// Part of the name.
    pub query: Option<String>,
    /// A tag the SLO has (`env:prod`).
    pub tags_query: Option<String>,
    /// Part of a metric SLO's numerator or denominator.
    pub metrics_query: Option<String>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

fn list_matches(doc: &Value, p: &DatadogListSlosParams, ids: &[String]) -> bool {
    let text = |k: &str| doc.get(k).and_then(Value::as_str).unwrap_or_default().to_lowercase();
    let has = |want: &Option<String>| want.as_deref().map(str::trim).filter(|w| !w.is_empty()).map(str::to_lowercase);
    (ids.is_empty() || ids.iter().any(|i| *i == text("id")))
        && has(&p.query).is_none_or(|q| text("name").contains(&q))
        && has(&p.tags_query).is_none_or(|t| doc.get("tags").and_then(Value::as_array).is_some_and(|tags| tags.iter().any(|x| x.as_str().is_some_and(|x| x.to_lowercase() == t))))
        && has(&p.metrics_query).is_none_or(|m| {
            let q = doc.get("query").cloned().unwrap_or_default();
            ["numerator", "denominator"].iter().any(|k| q.get(*k).and_then(Value::as_str).is_some_and(|v| v.to_lowercase().contains(&m)))
        })
}

/// List SLOs (Datadog-compatible), as the data sources filter them.
#[utoipa::path(get, path = "/api/v1/slo", operation_id = "datadog_list_slos", params(DatadogListSlosParams),
    responses((status = 200, body = DatadogSloResponse)), tag = "datadog-api")]
pub async fn list(State(st): State<AppState>, DdAuth(ctx): DdAuth, Query(p): Query<DatadogListSlosParams>) -> Result<Json<DatadogSloResponse>, DdError> {
    let all = load_all(&st, &ctx.org_id).await?;
    let total = all.len();
    let ids: Vec<String> = p.ids.as_deref().unwrap_or_default().split(',').map(str::trim).filter(|s| !s.is_empty()).map(|i| uuid_of(i).map(|u| hex_id(&u)).unwrap_or_else(|| i.to_string())).collect();
    let docs: Vec<Value> = all.iter().map(Stored::document).filter(|d| list_matches(d, &p, &ids)).collect();
    let limit = p.limit.filter(|l| *l > 0).unwrap_or(1000).min(1000) as usize;
    let page: Vec<Value> = docs.into_iter().skip(p.offset.unwrap_or(0).max(0) as usize).take(limit).collect();
    let metadata = json!({ "page": { "total_count": total, "total_filtered_count": page.len() } });
    Ok(Json(DatadogSloResponse { data: Value::Array(page), error: None, metadata: Some(metadata) }))
}

#[derive(Deserialize, IntoParams, Default)]
#[serde(default)]
pub struct DatadogSearchSlosParams {
    /// Words match name, description and tags; `key:value` words match tags.
    pub query: Option<String>,
    #[serde(rename = "page[size]")]
    #[param(rename = "page[size]")]
    pub page_size: Option<i64>,
    #[serde(rename = "page[number]")]
    #[param(rename = "page[number]")]
    pub page_number: Option<i64>,
    /// Accepted; facets are not returned.
    pub include_facets: Option<bool>,
}

/// `{"data": {"type": "service_level_objective_search_results", "attributes": {"slos": […]}}, "meta": {"pagination": …}, "links": …}`.
#[derive(Serialize, ToSchema)]
pub struct DatadogSloSearchResponse {
    pub data: Value,
    pub meta: Value,
    pub links: Value,
}

fn tag_values(tags: &[String], key: &str) -> Vec<String> {
    tags.iter().filter(|t| t.starts_with(&format!("{key}:"))).cloned().collect()
}

/// A search result: the SLO's document fields under Datadog's search names, with its status.
fn search_hit(doc: &Value, status: Option<&SloStatus>) -> Value {
    let tags = string_list(doc.get("tags").unwrap_or(&Value::Null), "tags").unwrap_or_default();
    let state = status.map_or("no_data", |s| match s.state {
        model::SloState::Ok => "ok",
        model::SloState::Warning => "warning",
        model::SloState::Breached => "breached",
        model::SloState::NoData => "no_data",
    });
    let error = (state == "no_data").then_some("No data in the time frame");
    let sli = status.and_then(|s| s.sli);
    let budget = status.and_then(|s| s.budget_remaining);
    let timeframe = doc.get("timeframe").cloned().unwrap_or(Value::Null);
    let target = doc.get("target_threshold").cloned().unwrap_or(Value::Null);
    let creator = doc.get("creator").filter(|c| c.is_object()).map(|c| {
        // Datadog's search has numeric user ids; ours are UUIDs, so this one is derived from the email.
        let email = c.get("email").and_then(Value::as_str).unwrap_or_default();
        let id = email.bytes().fold(0i64, |h, b| (h.wrapping_mul(31).wrapping_add(b as i64)) & 0x7fff_ffff);
        json!({ "name": c.get("name"), "email": email, "id": id })
    });
    let query = doc.get("query").map(|q| json!({ "numerator": q.get("numerator"), "denominator": q.get("denominator"), "metrics": null }));
    let status_doc = json!({ "state": state, "sli": sli, "error_budget_remaining": budget, "raw_error_budget_remaining": null,
        "indexed_at": doc.get("modified_at"), "span_precision": null, "calculation_error": error });
    json!({ "data": {
        "id": doc.get("id"),
        "type": "slo",
        "attributes": {
            "name": doc.get("name"),
            "description": doc.get("description"),
            "slo_type": doc.get("type"),
            "all_tags": tags,
            "env_tags": tag_values(&tags, "env"),
            "service_tags": tag_values(&tags, "service"),
            "team_tags": tag_values(&tags, "team"),
            "thresholds": doc.get("thresholds"),
            "timeframe": timeframe,
            "target_threshold": target,
            "warning_threshold": doc.get("warning_threshold"),
            "query": query,
            "monitor_ids": doc.get("monitor_ids"),
            "groups": doc.get("groups"),
            "creator": creator,
            "created_at": doc.get("created_at"),
            "modified_at": doc.get("modified_at"),
            "status": status_doc,
            "overall_status": [{ "state": state, "status": sli, "error_budget_remaining": budget, "raw_error_budget_remaining": null,
                "indexed_at": doc.get("modified_at"), "span_precision": null, "error": error, "target": target, "timeframe": timeframe }],
        }
    }})
}

/// Search SLOs (Datadog-compatible), as the `datadog_service_level_objectives` data source does with `query`.
#[utoipa::path(get, path = "/api/v1/slo/search", operation_id = "datadog_search_slos", params(DatadogSearchSlosParams),
    responses((status = 200, body = DatadogSloSearchResponse)), tag = "datadog-api")]
pub async fn search(State(st): State<AppState>, DdAuth(ctx): DdAuth, Query(p): Query<DatadogSearchSlosParams>) -> Result<Json<DatadogSloSearchResponse>, DdError> {
    let all = load_all(&st, &ctx.org_id).await?;
    let query = p.query.clone().unwrap_or_default();
    let matched: Vec<&Stored> = all.iter().filter(|s| model::query_matches(&query, &s.name, &s.description, &s.tags)).collect();
    let size = p.page_size.filter(|s| *s > 0).unwrap_or(10).min(1000);
    let number = p.page_number.unwrap_or(0).max(0);
    let total = matched.len() as i64;
    let page: Vec<&Stored> = matched.into_iter().skip((number * size) as usize).take(size as usize).collect();
    let statuses: HashMap<String, SloStatus> = if page.is_empty() { HashMap::new() } else { model::primary_statuses(&st, &ctx.org_id).await? };
    let slos: Vec<Value> = page.iter().map(|s| search_hit(&s.document(), statuses.get(&s.id))).collect();
    let last = if total == 0 { 0 } else { (total - 1) / size };
    let link = |n: i64| format!("/api/v1/slo/search?query={}&page[number]={n}&page[size]={size}", url::form_urlencoded::byte_serialize(query.as_bytes()).collect::<String>());
    Ok(Json(DatadogSloSearchResponse {
        data: json!({ "type": "service_level_objective_search_results", "attributes": { "slos": slos } }),
        meta: json!({ "pagination": { "number": number, "first_number": 0, "prev_number": (number - 1).max(0), "next_number": number + 1, "last_number": last, "size": size, "type": "number_size", "total": total } }),
        links: json!({ "self": link(number), "first": link(0), "next": link(number + 1), "last": link(last), "prev": if number > 0 { json!(link(number - 1)) } else { Value::Null } }),
    }))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    fn metric_doc() -> Value {
        json!({"type":"metric","description":"Checkout availability","name":"Checkout","groups":["env:test"],"monitor_ids":[],
            "query":{"denominator":"sum:trace.http.request.hits{service:order-service}.as_count()","numerator":"sum:trace.http.request.hits{service:order-service,!http.status_code:5xx}.as_count()"},
            "tags":["env:prod","app:core"],"thresholds":[{"target":97.0,"target_display":"97.0","timeframe":"7d","warning":98},{"target":99,"timeframe":"30d"}],
            "timeframe":"7d","target_threshold":97.0,"warning_threshold":98})
    }

    #[test]
    fn ids_are_uuids_as_hex() {
        assert_eq!(hex_id("26D1F492-d25b-598f-b6ef-1a0405faa153"), "26d1f492d25b598fb6ef1a0405faa153");
        assert_eq!(uuid_of("26d1f492d25b598fb6ef1a0405faa153").as_deref(), Some("26d1f492-d25b-598f-b6ef-1a0405faa153"));
        assert_eq!(uuid_of("26d1f492-d25b-598f-b6ef-1a0405faa153").as_deref(), Some("26d1f492-d25b-598f-b6ef-1a0405faa153"));
        assert_eq!(uuid_of("testdeleteansloreturnsnotfoundresponse1704322485"), None);
        assert_eq!(uuid_of("26d1f492d25b598fb6ef1a0405faa15z"), None);
        assert_eq!(uuid_of("2-6d1f492d25b598fb6ef1a0405faa153"), None);
    }

    #[test]
    fn a_metric_slo_maps_onto_our_model() {
        let i = checked_input(&metric_doc()).unwrap();
        assert_eq!((i.name.as_str(), i.description.as_str(), i.r#type), ("Checkout", "Checkout availability", SloType::Metric));
        // `.as_count()` is how our metric SLOs count.
        assert_eq!(i.good_query.as_deref(), Some("sum:trace.http.request.hits{service:order-service,!http.status_code:5xx}"));
        assert_eq!(i.total_query.as_deref(), Some("sum:trace.http.request.hits{service:order-service}"));
        assert!(i.monitor_ids.is_empty());
        assert_eq!(i.primary_window, Some(SloWindow::SevenDays));
        assert_eq!(i.targets, vec![SloTarget { window: SloWindow::SevenDays, target: 97.0, warning: Some(98.0) }, SloTarget { window: SloWindow::ThirtyDays, target: 99.0, warning: None }]);
        assert_eq!(i.tags, vec!["env:prod", "app:core"]);
    }

    #[test]
    fn the_primary_timeframe_defaults_to_the_first_threshold() {
        let mut d = metric_doc();
        for k in ["timeframe", "target_threshold", "warning_threshold"] {
            d.as_object_mut().unwrap().remove(k);
        }
        d["thresholds"] = json!([{"target":99,"timeframe":"30d"},{"target":97,"timeframe":"7d"}]);
        assert_eq!(checked_input(&d).unwrap().primary_window, Some(SloWindow::ThirtyDays));
    }

    #[test]
    fn count_specifications_substitute_named_queries() {
        let queries =
            json!([{"data_source":"metrics","name":"query1","query":"sum:httpservice.hits{*}.as_count()"},{"data_source":"metrics","name":"query2","query":"sum:httpservice.errors{*}.as_count()"}]);
        let total = json!({"type":"metric","name":"n","thresholds":[{"target":99.0,"timeframe":"7d"}],
            "sli_specification":{"count":{"good_events_formula":{"formula":"query1 - query2"},"total_events_formula":{"formula":"query1"},"queries":queries}}});
        let i = checked_input(&total).unwrap();
        assert_eq!(i.good_query.as_deref(), Some("sum:httpservice.hits{*} - sum:httpservice.errors{*}"));
        assert_eq!(i.total_query.as_deref(), Some("sum:httpservice.hits{*}"));
        let mut bad = total.clone();
        bad["sli_specification"]["count"] = json!({"good_events_formula":{"formula":"query1 - query2"},"bad_events_formula":{"formula":"query2"},"queries":queries});
        let i = checked_input(&bad).unwrap();
        assert_eq!(i.total_query.as_deref(), Some("(sum:httpservice.hits{*} - sum:httpservice.errors{*}) + (sum:httpservice.errors{*})"));
    }

    #[test]
    fn time_slice_and_monitor_slos_map_onto_our_model() {
        let d = json!({"type":"time_slice","name":"Latency","thresholds":[{"target":97.0,"timeframe":"7d","warning":98}],
            "sli_specification":{"time_slice":{"comparator":">","threshold":5,"query":{"formulas":[{"formula":"query1"}],"queries":[{"data_source":"metrics","name":"query1","query":"trace.servlet.request{env:prod}"}]}}}});
        let i = checked_input(&d).unwrap();
        assert_eq!(i.slice_query.as_deref(), Some("avg:trace.servlet.request{env:prod}"));
        assert_eq!((i.comparator, i.threshold, i.slice_minutes), (Some(SloComparator::Gt), Some(5.0), Some(5)));
        let mut one = d.clone();
        one["sli_specification"]["time_slice"]["query_interval_seconds"] = json!(60);
        assert_eq!(checked_input(&one).unwrap().slice_minutes, Some(1));

        let m = json!({"type":"monitor","name":"Up","monitor_ids":[12, 7],"groups":["env:prod"],"thresholds":[{"target":99.9,"timeframe":"30d"}]});
        let i = checked_input(&m).unwrap();
        assert_eq!((i.r#type, i.monitor_ids.clone()), (SloType::Monitor, vec![7, 12]));
    }

    #[test]
    fn unsupported_documents_are_rejected_precisely() {
        let err = |f: &dyn Fn(&mut Value)| {
            let mut d = metric_doc();
            f(&mut d);
            checked_input(&d).unwrap_err()
        };
        assert_eq!(err(&|d| d["thresholds"][0]["timeframe"] = json!("custom")), "Invalid payload: thresholds[0].timeframe: custom timeframes are not supported (7d, 30d and 90d are)");
        assert!(err(&|d| d["thresholds"][0]["timeframe"] = json!("1d")).contains("unknown timeframe '1d'"));
        assert!(err(&|d| d["type"] = json!("synthetic")).contains("unknown SLO type 'synthetic'"));
        assert_eq!(err(&|d| d["query"] = Value::Null), "Invalid payload: must specify the query for count types");
        assert!(err(&|d| d["query"]["numerator"] = json!("sum:a{*}.as_rate()")).contains(".as_rate()"));
        assert!(err(&|d| d["target_threshold"] = json!(95)).contains("target_threshold must match the target of the 7d threshold"));
        assert!(err(&|d| d["timeframe"] = json!("90d")).contains("timeframe must be the timeframe of one of the thresholds"));
        assert!(err(&|d| d["thresholds"] = json!([])).contains("thresholds"));
        assert!(err(&|d| {
            d["thresholds"][0]["target"] = json!(100);
            d["target_threshold"] = json!(100);
        })
        .contains("below 100"));
        assert_eq!(err(&|d| d["type"] = json!("monitor")), "Invalid payload: must specify monitor_ids");
        let ts = json!({"type":"time_slice","name":"x","thresholds":[{"target":97.0,"timeframe":"7d"}],
            "sli_specification":{"time_slice":{"comparator":">","threshold":5,"query":{"formulas":[{"formula":"query1"}],"queries":[{"data_source":"cloud_cost","name":"query1","query":"sum:aws.cost{*}"}]}}}});
        assert!(checked_input(&ts).unwrap_err().contains("only metrics queries are supported (data_source 'cloud_cost')"));
    }

    #[test]
    fn reads_return_the_stored_document_until_the_slo_changes_elsewhere() {
        let doc = metric_doc();
        let current = checked_input(&doc).unwrap();
        assert_eq!(reconcile(Some(&doc), &current), doc);

        // Renamed and retargeted in the UI: those parts come from the SLO, the rest stays as sent.
        let mut edited = current.clone();
        edited.name = "Checkout (renamed)".into();
        edited.targets = vec![SloTarget { window: SloWindow::ThirtyDays, target: 99.5, warning: None }];
        edited.primary_window = Some(SloWindow::ThirtyDays);
        let r = reconcile(Some(&doc), &edited);
        assert_eq!(r["name"], "Checkout (renamed)");
        assert_eq!(r["thresholds"], json!([{"timeframe":"30d","target":99.5}]));
        assert_eq!((r["timeframe"].as_str(), r["target_threshold"].as_f64(), r.get("warning_threshold")), (Some("30d"), Some(99.5), None));
        assert_eq!(r["query"], doc["query"]);
        assert_eq!(r["groups"], doc["groups"]);
        assert_eq!(r["description"], doc["description"]);

        // A new definition replaces the document's.
        let mut redefined = current.clone();
        redefined.total_query = Some("sum:trace.http.request.hits{*}".into());
        let r = reconcile(Some(&doc), &redefined);
        assert_eq!(r["query"]["denominator"], "sum:trace.http.request.hits{*}");
        // Tags compare as sets.
        let mut reordered = current.clone();
        reordered.tags.reverse();
        assert_eq!(reconcile(Some(&doc), &reordered)["tags"], doc["tags"]);
    }

    #[test]
    fn slos_made_in_the_ui_get_a_derived_document() {
        let ui = SloInput {
            name: "Search latency".into(),
            description: String::new(),
            r#type: SloType::TimeSlice,
            good_query: None,
            total_query: None,
            monitor_ids: vec![],
            slice_query: Some("avg:trace.http.request.duration{service:catalog-service} / 1000".into()),
            comparator: Some(SloComparator::Lt),
            threshold: Some(0.5),
            slice_minutes: Some(1),
            targets: vec![SloTarget { window: SloWindow::OneDay, target: 99.0, warning: None }, SloTarget { window: SloWindow::SevenDays, target: 99.5, warning: Some(99.7) }],
            primary_window: Some(SloWindow::OneDay),
            tags: vec!["service:catalog-service".into()],
            teams: vec!["search".into()],
        };
        let d = reconcile(None, &ui);
        assert_eq!(d["type"], "time_slice");
        // Datadog has no 1-day timeframe: only the 7d threshold, and no primary one.
        assert_eq!(d["thresholds"], json!([{"timeframe":"7d","target":99.5,"warning":99.7}]));
        assert!(d.get("timeframe").is_none());
        let slice = &d["sli_specification"]["time_slice"];
        assert_eq!(slice["query"]["formulas"], json!([{"formula":"query1 / 1000"}]));
        assert_eq!(slice["query"]["queries"], json!([{"data_source":"metrics","name":"query1","query":"avg:trace.http.request.duration{service:catalog-service}"}]));
        assert_eq!((slice["comparator"].as_str(), slice["query_interval_seconds"].as_i64()), (Some("<"), Some(60)));
        // And it maps back onto the same definition.
        let mut back = d.clone();
        back["thresholds"] = json!([{"timeframe":"7d","target":99.5,"warning":99.7}]);
        let i = checked_input(&back).unwrap();
        assert_eq!((i.slice_query, i.comparator, i.threshold, i.slice_minutes), (ui.slice_query, ui.comparator, ui.threshold, ui.slice_minutes));
    }

    #[test]
    fn responses_carry_server_fields_and_datadogs_defaults() {
        let doc = json!({"type":"metric","name":"n","thresholds":[{"target":95,"timeframe":"7d","warning":98}],
            "sli_specification":{"count":{"good_events_formula":{"formula":"query1"},"total_events_formula":{"formula":"query2"},
            "queries":[{"data_source":"metrics","name":"query1","query":"sum:a{ok:true}"},{"data_source":"metrics","name":"query2","query":"sum:a{*}"}]}}});
        let current = checked_input(&doc).unwrap();
        let server = SloServerFields { id: "ab".into(), created_at: 10, modified_at: 20, creator: Some(("Ada".into(), "ada@example.com".into())) };
        let r = respond(&reconcile(Some(&doc), &current), &current, &server);
        assert_eq!((r["id"].as_str(), r["type_id"].as_i64(), r["created_at"].as_i64(), r["modified_at"].as_i64()), (Some("ab"), Some(1), Some(10), Some(20)));
        assert_eq!(r["creator"], json!({"name":"Ada","handle":"ada@example.com","email":"ada@example.com"}));
        assert_eq!((r["description"].as_str(), r["tags"].clone(), r["monitor_tags"].clone()), (Some(""), json!([]), json!([])));
        assert_eq!(r["thresholds"][0]["target_display"], "95");
        assert_eq!(r["thresholds"][0]["warning_display"], "98");
        assert_eq!((r["timeframe"].as_str(), r["target_threshold"].as_f64(), r["warning_threshold"].as_f64()), (Some("7d"), Some(95.0), Some(98.0)));
        assert_eq!(r["query"], json!({"numerator":"sum:a{ok:true}","denominator":"sum:a{*}"}));
        assert_eq!(r["sli_specification"], doc["sli_specification"]);
        // Server fields sent back by a client are not stored.
        assert_eq!(to_store(&r).get("id"), None);
        assert_eq!(to_store(&r).get("creator"), None);
    }

    // ============================================================ live (OC_TEST_POSTGRES_URL / OC_TEST_CLICKHOUSE_URL)

    use super::super::replay::{new_org_with_roles, user_with_app_key, Feature, TestServer};

    pub(crate) async fn call(method: &str, url: &str, key: &str, body: Option<Value>) -> (u16, Value) {
        let mut req = reqwest::Client::new().request(reqwest::Method::from_bytes(method.as_bytes()).unwrap(), url);
        req = if key.starts_with("Bearer ") { req.header("Authorization", key) } else { req.header("DD-APPLICATION-KEY", key) };
        if let Some(b) = body {
            req = req.json(&b);
        }
        let res = req.send().await.unwrap();
        let status = res.status().as_u16();
        (status, res.json().await.unwrap_or(Value::Null))
    }

    /// Everything a client sent comes back as sent (the server may add fields, e.g. `target_display`).
    pub(crate) fn sent_fields(sent: &Value, got: &Value) {
        fn within(sent: &Value, got: &Value, path: &str) {
            match (sent, got) {
                (Value::Object(s), Value::Object(g)) => s.iter().for_each(|(k, v)| within(v, g.get(k).unwrap_or(&Value::Null), &format!("{path}.{k}"))),
                (Value::Array(s), Value::Array(g)) if s.len() == g.len() => s.iter().zip(g).enumerate().for_each(|(i, (s, g))| within(s, g, &format!("{path}[{i}]"))),
                _ => assert_eq!(got, sent, "{path}"),
            }
        }
        within(sent, got, "");
    }

    #[tokio::test]
    async fn an_slo_made_through_the_compat_api_is_one_of_ours() {
        let Some(s) = TestServer::start("dd-slo").await else { return };
        let ours = format!("Bearer {}", s.app_key);
        let doc = metric_doc();
        let (status, created) = call("POST", &format!("{}/api/v1/slo", s.base), &s.app_key, Some(doc.clone())).await;
        assert_eq!(status, 200, "{created}");
        let slo = &created["data"][0];
        let id = slo["id"].as_str().unwrap().to_string();
        assert_eq!(id.len(), 32);
        assert!(id.chars().all(|c| c.is_ascii_hexdigit()));
        sent_fields(&doc, slo);
        assert_eq!((slo["type_id"].as_i64(), slo["creator"]["name"].as_str()), (Some(1), Some("Terraform Test")));
        assert!(slo["created_at"].is_i64() && slo["modified_at"].is_i64());

        let (status, got) = call("GET", &format!("{}/api/v1/slo/{id}", s.base), &s.app_key, None).await;
        assert_eq!(status, 200);
        sent_fields(&doc, &got["data"]);
        assert_eq!(got["error"], Value::Null);

        // Our own API sees it: the same SLO, translated.
        let uuid = uuid_of(&id).unwrap();
        let (status, mine) = call("GET", &format!("{}/api/v1/slos/{uuid}", s.base), &ours, None).await;
        assert_eq!(status, 200, "{mine}");
        assert_eq!(mine["slo"]["goodQuery"], "sum:trace.http.request.hits{service:order-service,!http.status_code:5xx}");
        assert_eq!(mine["slo"]["primaryWindow"], "7d");
        assert_eq!(mine["slo"]["targets"].as_array().unwrap().len(), 2);

        // Edited in the UI: the compat read shows the change, the rest as sent.
        let mut input = mine["slo"].clone();
        input["name"] = json!("Checkout (edited in the UI)");
        input["targets"] = json!([{"window": "30d", "target": 99.5, "warning": null}]);
        input["primaryWindow"] = json!("30d");
        input["teams"] = json!(["checkout"]);
        let (status, body) = call("PUT", &format!("{}/api/v1/slos/{uuid}", s.base), &ours, Some(input)).await;
        assert_eq!(status, 200, "{body}");
        let (_, got) = call("GET", &format!("{}/api/v1/slo/{id}", s.base), &s.app_key, None).await;
        let d = &got["data"];
        assert_eq!(d["name"], "Checkout (edited in the UI)");
        assert_eq!(d["thresholds"], json!([{"timeframe":"30d","target":99.5,"target_display":"99.5"}]));
        assert_eq!((d["timeframe"].as_str(), d["target_threshold"].as_f64(), d.get("warning_threshold")), (Some("30d"), Some(99.5), None));
        assert_eq!(d["query"], doc["query"]);
        assert_eq!(d["groups"], doc["groups"]);

        // Replaced through the compat API again: the document wins, our teams stay.
        let mut update = doc.clone();
        update["description"] = json!("Updated by Terraform");
        let (status, body) = call("PUT", &format!("{}/api/v1/slo/{id}", s.base), &s.app_key, Some(update.clone())).await;
        assert_eq!(status, 200, "{body}");
        sent_fields(&update, &body["data"][0]);
        let (_, mine) = call("GET", &format!("{}/api/v1/slos/{uuid}", s.base), &ours, None).await;
        assert_eq!((mine["slo"]["name"].as_str(), mine["slo"]["description"].as_str()), (Some("Checkout"), Some("Updated by Terraform")));
        assert_eq!(mine["slo"]["teams"], json!(["checkout"]));
        let mut retyped = update.clone();
        retyped["type"] = json!("monitor");
        retyped["monitor_ids"] = json!([1]);
        let (status, body) = call("PUT", &format!("{}/api/v1/slo/{id}", s.base), &s.app_key, Some(retyped)).await;
        assert_eq!((status, body["errors"][0].as_str()), (400, Some("Invalid payload: the type of an SLO can't change (it is metric)")));

        let (status, body) = call("DELETE", &format!("{}/api/v1/slo/{id}?force=true", s.base), &s.app_key, None).await;
        assert_eq!((status, body), (200, json!({"data": [id], "error": null})));
        assert_eq!(call("GET", &format!("{}/api/v1/slo/{id}", s.base), &s.app_key, None).await.0, 404);
        assert_eq!(call("GET", &format!("{}/api/v1/slos/{uuid}", s.base), &ours, None).await.0, 404);
        let (status, body) = call("DELETE", &format!("{}/api/v1/slo/{id}", s.base), &s.app_key, None).await;
        assert_eq!((status, body), (404, json!({"errors": [format!("SLO not found: {id} not found")]})));
    }

    #[tokio::test]
    async fn slos_made_in_the_ui_read_as_datadog_documents() {
        let Some(s) = TestServer::start("dd-slo-ui").await else { return };
        let input = json!({"name": "Catalog availability", "type": "metric", "goodQuery": "sum:trace.http.request.hits{service:catalog-service,!http.status_code:5xx}",
            "totalQuery": "sum:trace.http.request.hits{service:catalog-service}", "targets": [{"window": "30d", "target": 99.9, "warning": 99.95}], "tags": ["service:catalog-service"]});
        let (status, made) = call("POST", &format!("{}/api/v1/slos", s.base), &format!("Bearer {}", s.app_key), Some(input)).await;
        assert_eq!(status, 200, "{made}");
        let id = hex_id(made["id"].as_str().unwrap());
        let (status, got) = call("GET", &format!("{}/api/v1/slo/{id}", s.base), &s.app_key, None).await;
        assert_eq!(status, 200);
        let d = &got["data"];
        assert_eq!((d["name"].as_str(), d["type"].as_str(), d["timeframe"].as_str(), d["target_threshold"].as_f64()), (Some("Catalog availability"), Some("metric"), Some("30d"), Some(99.9)));
        assert_eq!(d["query"]["denominator"], "sum:trace.http.request.hits{service:catalog-service}");
        assert_eq!(d["tags"], json!(["service:catalog-service"]));
    }

    #[tokio::test]
    async fn monitor_slos_need_monitors_of_the_org() {
        let Some(s) = TestServer::start("dd-slo-monitor").await else { return };
        let (monitor,): (i64,) =
            sqlx::query_as("INSERT INTO monitors (org_id, name, query, definition) VALUES ($1, 'Checkout errors', 'avg:trace.http.request.errors{service:order-service}', '{}') RETURNING id")
                .bind(&s.org)
                .fetch_one(&s.st.postgres)
                .await
                .unwrap();
        let doc = json!({"type":"monitor","name":"Checkout up","monitor_ids":[monitor],"groups":["env:prod"],"thresholds":[{"timeframe":"30d","target":99.9,"warning":99.95}]});
        let (status, created) = call("POST", &format!("{}/api/v1/slo", s.base), &s.app_key, Some(doc.clone())).await;
        assert_eq!(status, 200, "{created}");
        sent_fields(&doc, &created["data"][0]);
        assert_eq!(created["data"][0]["type_id"], 0);
        let mut missing = doc.clone();
        missing["monitor_ids"] = json!([monitor + 1000]);
        let (status, body) = call("POST", &format!("{}/api/v1/slo", s.base), &s.app_key, Some(missing)).await;
        assert_eq!((status, body["errors"][0].as_str()), (400, Some(format!("Monitor {} not found", monitor + 1000).as_str())));
    }

    #[tokio::test]
    async fn slos_follow_the_keys_org_and_role() {
        let Some(s) = TestServer::start("dd-slo-tenancy").await else { return };
        let (_, created) = call("POST", &format!("{}/api/v1/slo", s.base), &s.app_key, Some(metric_doc())).await;
        let id = created["data"][0]["id"].as_str().unwrap().to_string();
        let (_, reader) = user_with_app_key(&s.st, &s.org, "readonly").await;
        assert_eq!(call("GET", &format!("{}/api/v1/slo/{id}", s.base), &reader, None).await.0, 200);
        let forbidden = (403, json!({"errors": ["Forbidden"]}));
        assert_eq!(call("POST", &format!("{}/api/v1/slo", s.base), &reader, Some(metric_doc())).await, forbidden);
        assert_eq!(call("PUT", &format!("{}/api/v1/slo/{id}", s.base), &reader, Some(metric_doc())).await, forbidden);
        assert_eq!(call("DELETE", &format!("{}/api/v1/slo/{id}", s.base), &reader, None).await, forbidden);

        let other = new_org_with_roles(&s.st, "dd-slo-other").await;
        let (_, stranger) = user_with_app_key(&s.st, &other, "admin").await;
        assert_eq!(call("GET", &format!("{}/api/v1/slo/{id}", s.base), &stranger, None).await.0, 404);
        assert_eq!(call("PUT", &format!("{}/api/v1/slo/{id}", s.base), &stranger, Some(metric_doc())).await.0, 404);
        assert_eq!(call("DELETE", &format!("{}/api/v1/slo/{id}", s.base), &stranger, None).await.0, 404);
        let (_, list) = call("GET", &format!("{}/api/v1/slo", s.base), &stranger, None).await;
        assert_eq!(list["data"], json!([]));
        assert_eq!(call("GET", &format!("{}/api/v1/slo/{id}", s.base), "", None).await, forbidden);
    }

    #[tokio::test]
    async fn the_data_sources_find_slos_by_id_name_tag_and_query() {
        let Some(s) = TestServer::start("dd-slo-list").await else { return };
        let mut ids = vec![];
        for (name, tag) in [("Checkout availability", "env:prod"), ("Checkout latency", "env:staging"), ("Billing availability", "env:prod")] {
            let mut d = metric_doc();
            d["name"] = json!(name);
            d["tags"] = json!([tag]);
            if name.starts_with("Billing") {
                d["query"] =
                    json!({"numerator": "sum:trace.http.request.hits{service:billing,!http.status_code:5xx}.as_count()", "denominator": "sum:trace.http.request.hits{service:billing}.as_count()"});
            }
            let (_, c) = call("POST", &format!("{}/api/v1/slo", s.base), &s.app_key, Some(d)).await;
            ids.push(c["data"][0]["id"].as_str().unwrap().to_string());
        }
        let names = |v: &Value| v["data"].as_array().unwrap().iter().map(|d| d["name"].as_str().unwrap().to_string()).collect::<Vec<_>>();
        let list = |q: String| {
            let url = format!("{}/api/v1/slo?{q}", s.base);
            let key = s.app_key.clone();
            async move { call("GET", &url, &key, None).await.1 }
        };
        assert_eq!(names(&list(format!("ids={},{}", ids[0], ids[2])).await), vec!["Checkout availability", "Billing availability"]);
        assert_eq!(names(&list("query=checkout".into()).await), vec!["Checkout availability", "Checkout latency"]);
        assert_eq!(names(&list("tags_query=env:prod".into()).await), vec!["Checkout availability", "Billing availability"]);
        assert_eq!(names(&list("metrics_query=service:billing".into()).await), vec!["Billing availability"]);
        let page = list("limit=2&offset=2".into()).await;
        assert_eq!((names(&page), page["metadata"]["page"]["total_count"].as_i64()), (vec!["Billing availability".to_string()], Some(3)));

        let (status, found) = call("GET", &format!("{}/api/v1/slo/search?query=availability%20env:prod&page[size]=1&page[number]=1", s.base), &s.app_key, None).await;
        assert_eq!(status, 200, "{found}");
        let hits = found["data"]["attributes"]["slos"].as_array().unwrap();
        assert_eq!(hits.len(), 1);
        let hit = &hits[0]["data"];
        assert_eq!((hit["id"].as_str(), hit["type"].as_str()), (Some(ids[2].as_str()), Some("slo")));
        assert_eq!((hit["attributes"]["name"].as_str(), hit["attributes"]["slo_type"].as_str()), (Some("Billing availability"), Some("metric")));
        assert_eq!(hit["attributes"]["status"]["state"], "no_data");
        assert_eq!((found["meta"]["pagination"]["total"].as_i64(), found["meta"]["pagination"]["last_number"].as_i64()), (Some(2), Some(1)));
    }

    /// Datadog's recorded Service Level Objectives scenarios.
    #[tokio::test]
    async fn service_level_objective_cassettes_replay() {
        let Some(feature) = Feature::load("v1/service_level_objectives") else { return };
        let Some(s) = TestServer::start("dd-slo-cassettes").await else { return };
        // Scenario → why it isn't replayed.
        let unsupported: HashMap<&str, &str> =
            HashMap::from([("Get an SLO's history returns \"OK\" response", "SLO history (the computed SLI series) is not part of the compat API: no client in scope reads it")]);
        let mut replayed = 0;
        for scenario in &feature.scenarios {
            if feature.cassette(scenario).is_none() || unsupported.contains_key(scenario.name.as_str()) {
                continue;
            }
            let (exchanges, problems) = s.replayer().replay(&feature, scenario).await;
            assert!(problems.is_empty(), "{}: {problems:#?}", scenario.name);
            assert!(!exchanges.is_empty());
            replayed += 1;
        }
        assert_eq!(replayed, 14);
    }
}
