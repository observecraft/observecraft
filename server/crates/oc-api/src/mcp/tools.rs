//! The MCP server's tools (ADR-0009): read-only calls of the handlers the UI and Assist use, as the
//! application key's user. The org comes from the key only; telemetry goes through `AppState::ch(org)`
//! and logs through the user's restriction queries. Results are a line a person can read, then
//! compact JSON, capped in rows, text and time range.
use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use axum::extract::{Path, State};
use axum::Json;
use serde_json::{json, Map, Value};

use super::{ToolOutcome, Toolbox};
use crate::assist::tools::{err, headline as assist_headline, limit, parse_range, req, spec, OrgTools};
use crate::auth::AuthContext;
use crate::routes::{apm, cloud_security, code_security, dashboards, events, hosts, log_analytics, logs, metrics};
use crate::state::{AppState, CurrentUser, OrgId};

/// The longest time range a tool reads.
pub const MAX_RANGE_MS: i64 = 31 * 86_400_000;
/// A result longer than this many characters is cut.
pub const MAX_TEXT: usize = 50_000;
/// A tool call is abandoned after this long.
const TIMEOUT: Duration = Duration::from_secs(30);
/// Tool calls a user may make per window on one server replica.
const BURST_CALLS: usize = 50;
const BURST_WINDOW: Duration = Duration::from_secs(10);
const MESSAGE_CHARS: usize = 500;
const VALUE_CHARS: usize = 200;
const ATTRIBUTES: usize = 30;
const TRACE_SPANS: usize = 200;
const SERIES: usize = 40;
const WIDGETS: usize = 100;

const RANGE_HELP: &str = "Relative to now (15m, 1h, 4h, 1d, 7d) or epoch milliseconds; at most 31 days.";
const SEARCH_SYNTAX: &str = "Search syntax: `service:checkout status:error env:prod @http.status_code:>=500 \"timed out\" -host:web-1`.";

/// What a tool does.
#[derive(Clone, Copy)]
enum Run {
    /// Assist's tool of that name (description and schema too).
    Assist(&'static str),
    Own(&'static str, fn() -> Value),
}

struct Def {
    name: &'static str,
    title: &'static str,
    run: Run,
}

fn range_props() -> Value {
    json!({
        "from": {"type": "string", "description": format!("Start. {RANGE_HELP} Default 1h.")},
        "to": {"type": "string", "description": format!("End. {RANGE_HELP} Default now.")},
    })
}

fn object(mut props: Value, required: &[&str], ranged: bool) -> Value {
    if let (true, Some(p), Value::Object(r)) = (ranged, props.as_object_mut(), range_props()) {
        p.extend(r);
    }
    json!({"type": "object", "properties": props, "required": required, "additionalProperties": false})
}

fn limit_prop(default: u64, max: u64) -> Value {
    json!({"type": "integer", "minimum": 1, "maximum": max, "description": format!("Rows returned, default {default}.")})
}

const DEFS: &[Def] = &[
    Def {
        name: "search_logs",
        title: "Search logs",
        run: Run::Own("Search log events, newest first: time, service, status, host, message (cut at 500 characters), tags and attributes. Page with `cursor` from the previous result's nextCursor.", || {
            object(json!({"query": {"type": "string", "description": SEARCH_SYNTAX}, "limit": limit_prop(25, 100), "cursor": {"type": "string"}}), &["query"], true)
        }),
    },
    Def {
        name: "aggregate_logs",
        title: "Aggregate logs",
        run: Run::Own("Count logs (or compute a measure) grouped by facets, e.g. errors by service: query `status:error`, group_by [\"service\"]. Facets are tags (service, status, host, env) or attributes (@http.status_code).", || {
            object(
                json!({
                    "query": {"type": "string", "description": SEARCH_SYNTAX},
                    "group_by": {"type": "array", "items": {"type": "string"}, "maxItems": 3, "description": "Facets to split by, outermost first."},
                    "aggregation": {"type": "string", "enum": ["count", "cardinality", "sum", "min", "max", "avg", "median", "pc75", "pc90", "pc95", "pc99"], "description": "Default count."},
                    "metric": {"type": "string", "description": "The facet or measure for aggregations other than count, e.g. @duration."},
                    "limit": {"type": "integer", "minimum": 1, "maximum": 50, "description": "Groups kept per facet, default 10."},
                }),
                &[],
                true,
            )
        }),
    },
    Def {
        name: "query_metrics",
        title: "Query metrics",
        run: Run::Own("Query metric timeseries with the metric query syntax, e.g. `avg:system.cpu.user{env:prod} by {host}` or `sum:trace.http.request.errors{service:checkout}.as_count()`. Returns per series its unit, last, min, max and average, and up to `points` points. Find names with search_metrics.", || {
            object(
                json!({
                    "queries": {"type": "array", "items": {"type": "string"}, "minItems": 1, "maxItems": 5},
                    "points": {"type": "integer", "minimum": 0, "maximum": 120, "description": "Points per series, default 30; 0 for summaries only."},
                }),
                &["queries"],
                true,
            )
        }),
    },
    Def {
        name: "search_metrics",
        title: "Search metrics",
        run: Run::Own("Metrics reported in the last day (and span-derived trace.* metrics) whose name contains the words of `query`, with their kind, unit, description and tag keys.", || {
            object(json!({"query": {"type": "string", "description": "Words of the name, e.g. `kafka lag` or `system.cpu`."}, "tag": {"type": "string", "description": "Only metrics carrying this tag key, e.g. kube_namespace."}, "limit": limit_prop(50, 200)}), &[], false)
        }),
    },
    Def { name: "search_spans", title: "Search spans", run: Run::Assist("search_traces") },
    Def {
        name: "get_trace",
        title: "Get a trace",
        run: Run::Own("One trace's spans (up to 200, in start order): service, operation, parent, start offset and duration in ms, status, attributes.", || object(json!({"trace_id": {"type": "string", "description": "Hex trace id, from search_spans or a log's trace_id."}}), &["trace_id"], false)),
    },
    Def { name: "list_services", title: "List APM services", run: Run::Assist("list_services") },
    Def { name: "search_monitors", title: "Search monitors", run: Run::Assist("search_monitors") },
    Def { name: "get_monitor", title: "Get a monitor", run: Run::Assist("get_monitor") },
    Def { name: "search_incidents", title: "Search incidents", run: Run::Assist("search_incidents") },
    Def { name: "get_incident", title: "Get an incident", run: Run::Assist("get_incident") },
    Def {
        name: "search_events",
        title: "Search events",
        run: Run::Own("Events (deployments, alerts, Agent and API events), newest first: time, title, text, alert type, source, host, tags.", || {
            object(json!({"query": {"type": "string", "description": "e.g. `source:kubernetes status:error deploy`."}, "limit": limit_prop(20, 50)}), &[], true)
        }),
    },
    Def {
        name: "search_hosts",
        title: "Search hosts",
        run: Run::Own("Hosts seen in the range with cloud, OS, env, CPU and memory use, monitors and APM errors. Query by `cloud:aws os:linux env:prod team:x` and text on the name.", || {
            object(json!({"query": {"type": "string"}, "limit": limit_prop(25, 100)}), &[], true)
        }),
    },
    Def { name: "search_kubernetes", title: "Search Kubernetes", run: Run::Assist("search_kubernetes") },
    Def {
        name: "search_dashboards",
        title: "Search dashboards",
        run: Run::Own("Dashboards whose title or author matches the text.", || object(json!({"query": {"type": "string"}, "limit": limit_prop(20, 50)}), &[], false)),
    },
    Def {
        name: "get_dashboard",
        title: "Get a dashboard",
        run: Run::Own("A dashboard's widgets (type, title, queries) and template variables, by id (from search_dashboards).", || object(json!({"id": {"type": "string"}}), &["id"], false)),
    },
    Def { name: "search_error_issues", title: "Search error issues", run: Run::Assist("search_error_issues") },
    Def { name: "search_security_signals", title: "Search security signals", run: Run::Assist("search_security_signals") },
    Def { name: "get_security_signal", title: "Get a security signal", run: Run::Assist("get_security_signal") },
    Def {
        name: "search_vulnerabilities",
        title: "Search vulnerabilities",
        run: Run::Own("Cloud Security vulnerabilities of hosts and container images (from SBOMs): advisory, CVE, severity, package and fix, resource, whether it runs and whether an exploit is known.", || {
            object(
                json!({
                    "query": {"type": "string", "description": "Words of the CVE, advisory, package, resource or host."},
                    "severity": {"type": "array", "items": {"type": "string", "enum": ["critical", "high", "medium", "low", "unknown"]}},
                    "status": {"type": "string", "enum": ["open", "muted", "all"], "description": "Default open."},
                    "limit": limit_prop(20, 50),
                }),
                &[],
                false,
            )
        }),
    },
    Def {
        name: "search_code_security_findings",
        title: "Search code security findings",
        run: Run::Own("Static code findings (SAST, secrets, infrastructure as code) of the org's repositories: rule, severity, message, repository, file and line.", || {
            object(
                json!({
                    "kind": {"type": "string", "enum": ["sast", "secret", "iac"], "description": "All kinds when absent."},
                    "query": {"type": "string", "description": "Words of the rule, file or message."},
                    "severity": {"type": "array", "items": {"type": "string", "enum": ["critical", "high", "medium", "low", "info"]}},
                    "repository": {"type": "string"},
                    "status": {"type": "string", "enum": ["open", "muted", "resolved", "all"], "description": "Default open."},
                    "limit": limit_prop(20, 50),
                }),
                &[],
                false,
            )
        }),
    },
];

/// A tool's MCP definition.
fn definition(d: &Def) -> Value {
    let (description, schema) = match d.run {
        Run::Assist(name) => {
            let s = spec(name).expect("an Assist tool");
            (s.description, s.schema)
        }
        Run::Own(description, schema) => (description.to_string(), schema()),
    };
    json!({
        "name": d.name,
        "title": d.title,
        "description": description,
        "inputSchema": schema,
        "annotations": {"title": d.title, "readOnlyHint": true, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false},
    })
}

// ================================================================ limits

/// The call's range, refused beyond [`MAX_RANGE_MS`].
pub fn range(input: &Value, now: i64, default_from: &str) -> Result<logs::TimeRange, String> {
    let r = parse_range(input, now, default_from)?;
    if r.to - r.from > MAX_RANGE_MS {
        return Err("The time range may cover at most 31 days: narrow `from` / `to`".into());
    }
    Ok(r)
}

/// At most `max` characters, with an ellipsis when cut.
pub fn cut(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max).collect();
    out.push('…');
    out
}

/// A result's text, cut at [`MAX_TEXT`] with a note.
pub fn capped(text: String) -> String {
    let total = text.chars().count();
    if total <= MAX_TEXT {
        return text;
    }
    let mut out: String = text.chars().take(MAX_TEXT).collect();
    out.push_str(&format!("\n… (truncated: {} more characters; narrow the query, the time range or the limit)", total - MAX_TEXT));
    out
}

/// Up to [`ATTRIBUTES`] leaves of a JSON object as `a.b.c: value`, values cut.
fn flatten(v: &Value) -> Map<String, Value> {
    fn walk(prefix: &str, v: &Value, out: &mut Map<String, Value>) {
        if out.len() >= ATTRIBUTES {
            return;
        }
        match v {
            Value::Object(m) => {
                for (k, child) in m {
                    walk(&if prefix.is_empty() { k.clone() } else { format!("{prefix}.{k}") }, child, out);
                }
            }
            Value::String(s) => {
                out.insert(prefix.to_string(), json!(cut(s, VALUE_CHARS)));
            }
            Value::Null => {}
            other => {
                let text = other.to_string();
                out.insert(prefix.to_string(), if text.chars().count() > VALUE_CHARS { json!(cut(&text, VALUE_CHARS)) } else { other.clone() });
            }
        }
    }
    let mut out = Map::new();
    walk("", v, &mut out);
    out
}

fn iso(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms).map(|t| t.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string()).unwrap_or_default()
}

/// A log for the model.
pub fn log_json(l: &logs::LogEvent) -> Value {
    let mut v = json!({"timestamp": iso(l.timestamp), "service": l.service, "status": l.status, "host": l.host, "message": cut(&l.message, MESSAGE_CHARS)});
    if !l.source.is_empty() {
        v["source"] = json!(l.source);
    }
    if let Some(t) = &l.trace_id {
        v["trace_id"] = json!(t);
    }
    if !l.tags.is_empty() {
        let mut tags: Vec<_> = l.tags.iter().collect();
        tags.sort();
        v["tags"] = json!(tags.into_iter().take(ATTRIBUTES).map(|(k, val)| (k.clone(), json!(cut(val, VALUE_CHARS)))).collect::<Map<_, _>>());
    }
    let attributes = flatten(&l.attributes);
    if !attributes.is_empty() {
        v["attributes"] = Value::Object(attributes);
    }
    v
}

/// A metric series' summary and up to `points` points (evenly picked, the last one kept).
pub fn series_json(s: &metrics::MetricSeries, points: usize) -> Value {
    let finite: Vec<(i64, f64)> = s.points.iter().copied().filter(|p| p.1.is_finite()).collect();
    let values: Vec<f64> = finite.iter().map(|p| p.1).collect();
    let avg = (!values.is_empty()).then(|| values.iter().sum::<f64>() / values.len() as f64);
    let mut v = json!({"series": s.name, "unit": s.unit, "last": values.last(), "min": values.iter().cloned().reduce(f64::min), "max": values.iter().cloned().reduce(f64::max), "avg": avg, "count": values.len()});
    if points > 0 && !finite.is_empty() {
        let step = finite.len().div_ceil(points);
        let mut picked: Vec<(i64, f64)> = finite.iter().rev().step_by(step).copied().collect();
        picked.reverse();
        v["points"] = json!(picked.into_iter().map(|(t, x)| json!([iso(t), x])).collect::<Vec<_>>());
    }
    v
}

/// A trace's spans in start order, capped, attributes cut.
pub fn trace_json(t: &apm::TraceDetail) -> Value {
    let mut spans: Vec<&apm::TraceSpan> = t.spans.iter().collect();
    spans.sort_by(|a, b| a.start_us.total_cmp(&b.start_us));
    let shown: Vec<Value> = spans
        .iter()
        .take(TRACE_SPANS)
        .map(|s| {
            let mut attrs: Vec<_> = s.attributes.iter().collect();
            attrs.sort();
            json!({
                "span_id": s.span_id, "parent_span_id": s.parent_span_id, "service": s.service, "name": s.name, "kind": s.kind,
                "start_ms": (s.start_us / 1000.0 * 1000.0).round() / 1000.0, "duration_ms": (s.duration_us / 1000.0 * 1000.0).round() / 1000.0,
                "status": s.status, "status_message": cut(&s.status_message, VALUE_CHARS),
                "attributes": attrs.into_iter().take(20).map(|(k, v)| (k.clone(), json!(cut(v, VALUE_CHARS)))).collect::<Map<_, _>>(),
            })
        })
        .collect();
    json!({"trace_id": t.trace_id, "start": iso(t.start), "duration_ms": t.duration_ms, "total": t.spans.len(), "spans": shown})
}

/// Widgets as (group, type, title, queries), groups' widgets included, capped.
pub fn widgets_json(widgets: &[Value]) -> Vec<Value> {
    fn queries(v: &Value, out: &mut Vec<String>) {
        match v {
            Value::Object(m) => {
                for (k, child) in m {
                    match (k.as_str(), child) {
                        ("widgets", _) => {}
                        ("query" | "q" | "formula", Value::String(s)) if !s.is_empty() && out.len() < 10 => out.push(cut(s, VALUE_CHARS)),
                        _ => queries(child, out),
                    }
                }
            }
            Value::Array(a) => a.iter().for_each(|c| queries(c, out)),
            _ => {}
        }
    }
    fn walk(widgets: &[Value], group: Option<&str>, out: &mut Vec<Value>) {
        for w in widgets {
            if out.len() >= WIDGETS {
                return;
            }
            let d = &w["definition"];
            let mut q = Vec::new();
            queries(d, &mut q);
            let mut item = json!({"type": d["type"], "title": d["title"].as_str().unwrap_or("")});
            if let Some(g) = group {
                item["group"] = json!(g);
            }
            if !q.is_empty() {
                item["queries"] = json!(q);
            }
            out.push(item);
            if let Some(children) = d["widgets"].as_array() {
                walk(children, d["title"].as_str().or(Some("")), out);
            }
        }
    }
    let mut out = Vec::new();
    walk(widgets, None, &mut out);
    out
}

/// Calls per user within a sliding window, on this replica.
pub struct Burst {
    max: usize,
    window: Duration,
    hits: Mutex<HashMap<String, VecDeque<Instant>>>,
}

impl Burst {
    pub fn new(max: usize, window: Duration) -> Self {
        Burst { max, window, hits: Mutex::new(HashMap::new()) }
    }

    /// Records a call at `now`; false when the user already made `max` in the window.
    pub fn allow(&self, user: &str, now: Instant) -> bool {
        let mut hits = self.hits.lock().unwrap_or_else(|e| e.into_inner());
        if hits.len() > 10_000 {
            hits.retain(|_, q| q.back().is_some_and(|t| now.duration_since(*t) < self.window));
        }
        let q = hits.entry(user.to_string()).or_default();
        while q.front().is_some_and(|t| now.duration_since(*t) >= self.window) {
            q.pop_front();
        }
        if q.len() >= self.max {
            return false;
        }
        q.push_back(now);
        true
    }
}

static BURST: std::sync::LazyLock<Burst> = std::sync::LazyLock::new(|| Burst::new(BURST_CALLS, BURST_WINDOW));

// ================================================================ the tools

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// `total` matching and `shown` returned.
fn page_line(total: usize, shown: usize, one: &str, many: &str, none: &str) -> String {
    match (total, shown) {
        (0, _) => none.to_string(),
        (t, s) if s < t => format!("{} (showing {s})", plural(t, one, many)),
        (t, _) => plural(t, one, many),
    }
}

fn strings(v: &Value) -> Vec<String> {
    v.as_array().into_iter().flatten().filter_map(Value::as_str).map(str::to_string).collect()
}

/// The tools of one application key's user.
pub struct McpTools {
    pub st: AppState,
    pub ctx: AuthContext,
}

impl McpTools {
    fn org(&self) -> OrgId {
        OrgId(self.ctx.org_id.clone())
    }

    /// Runs a tool: (summary line, result) or a message for the model.
    async fn run(&self, def: &Def, input: &Value) -> Result<(String, Value), String> {
        let st = || State(self.st.clone());
        let now = metrics::now_ms();
        let ser = |v: Result<Value, serde_json::Error>| v.map_err(|e| e.to_string());
        match def.run {
            Run::Assist(name) => {
                if input.get("from").is_some() || input.get("to").is_some() {
                    range(input, now, "1h")?;
                }
                let assist = OrgTools { st: self.st.clone(), ctx: self.ctx.clone(), disabled: vec![] };
                let v = assist.call(name, input).await?;
                Ok((assist_headline(name, &v), v))
            }
            Run::Own(..) => match def.name {
                "search_logs" => {
                    let access = crate::log_access::for_user(&self.st, &self.ctx).await.map_err(err)?;
                    let r: logs::LogSearchRequest = req(json!({"range": range(input, now, "1h")?, "query": input["query"], "limit": limit(input, 25, 100), "cursor": input.get("cursor")}))?;
                    let page = logs::search(st(), self.org(), access, Json(r)).await.map_err(err)?.0;
                    let line = page_line(page.total as usize, page.logs.len(), "log", "logs", "No logs found");
                    Ok((line, json!({"total": page.total, "restricted": page.restricted, "nextCursor": page.next_cursor, "logs": page.logs.iter().map(log_json).collect::<Vec<_>>()})))
                }
                "aggregate_logs" => {
                    let access = crate::log_access::for_user(&self.st, &self.ctx).await.map_err(err)?;
                    let r = range(input, now, "1h")?;
                    let facets = strings(&input["group_by"]);
                    if facets.len() > 3 {
                        return Err("Group by at most 3 facets".into());
                    }
                    let aggregation = input["aggregation"].as_str().unwrap_or("count");
                    if aggregation != "count" && input["metric"].as_str().is_none() {
                        return Err(format!("{aggregation} needs a `metric` (a facet or measure such as @duration)"));
                    }
                    let per = limit(input, 10, 50);
                    let body: log_analytics::AggregateRequest = req(json!({
                        "compute": [{"aggregation": aggregation, "metric": input.get("metric"), "type": "total"}],
                        "filter": {"query": input["query"].as_str().unwrap_or(""), "from": r.from, "to": r.to},
                        "group_by": facets.iter().map(|f| json!({"facet": f, "limit": per})).collect::<Vec<_>>(),
                    }))?;
                    let out = log_analytics::aggregate(st(), self.org(), access, Json(body)).await.map_err(err)?.0;
                    let buckets: Vec<Value> = out.data.buckets.iter().take(200).map(|b| json!({"by": b.by, "value": b.computes.get("c0")})).collect();
                    Ok((plural(out.data.buckets.len(), "group", "groups"), json!({"aggregation": aggregation, "buckets": buckets})))
                }
                "query_metrics" => {
                    let queries: Vec<String> = strings(&input["queries"]);
                    if queries.is_empty() || queries.len() > 5 {
                        return Err("Give 1 to 5 queries".into());
                    }
                    let points = input["points"].as_u64().unwrap_or(30).min(120) as usize;
                    let r: metrics::TimeseriesRequest = req(json!({"queries": queries, "range": range(input, now, "1h")?, "points": 120}))?;
                    let series = metrics::timeseries(st(), self.org(), Json(r)).await.map_err(err)?.0;
                    let line = page_line(series.len(), series.len().min(SERIES), "series", "series", "No data");
                    Ok((line, json!({"total": series.len(), "series": series.iter().take(SERIES).map(|s| series_json(s, points)).collect::<Vec<_>>()})))
                }
                "search_metrics" => {
                    let words: Vec<String> = input["query"].as_str().unwrap_or("").split_whitespace().map(str::to_lowercase).collect();
                    let tag = input["tag"].as_str().filter(|t| !t.is_empty());
                    let all = metrics::catalog(st(), self.org()).await.map_err(err)?.0;
                    let hits: Vec<&metrics::MetricCatalogEntry> = all.iter().filter(|m| words.iter().all(|w| m.name.to_lowercase().contains(w)) && tag.is_none_or(|t| m.tags.iter().any(|x| x == t))).collect();
                    let shown: Vec<Value> = hits
                        .iter()
                        .take(limit(input, 50, 200) as usize)
                        .map(|m| json!({"name": m.name, "kind": m.kind, "unit": m.unit, "description": cut(&m.description, VALUE_CHARS), "tags": m.tags.iter().take(ATTRIBUTES).collect::<Vec<_>>()}))
                        .collect();
                    Ok((page_line(hits.len(), shown.len(), "metric", "metrics", "No metrics found"), json!({"total": hits.len(), "metrics": shown})))
                }
                "get_trace" => {
                    let id = input["trace_id"].as_str().map(str::trim).filter(|t| !t.is_empty() && t.len() <= 64 && t.chars().all(|c| c.is_ascii_hexdigit())).ok_or("Give the trace id: 1 to 64 hex characters")?;
                    let t = apm::trace(st(), self.org(), Path(id.to_lowercase())).await.map_err(err)?.0;
                    let v = trace_json(&t);
                    Ok((page_line(t.spans.len(), t.spans.len().min(TRACE_SPANS), "span", "spans", "No spans"), v))
                }
                "search_events" => {
                    let r: events::EventSearchRequest = req(json!({"range": range(input, now, "1d")?, "query": input["query"].as_str().unwrap_or(""), "offset": 0, "limit": limit(input, 20, 50)}))?;
                    let page = events::search(st(), self.org(), Json(r)).await.map_err(err)?.0;
                    let items: Vec<Value> = page
                        .items
                        .iter()
                        .map(|e| json!({"timestamp": iso(e.timestamp), "title": cut(&e.title, VALUE_CHARS), "text": cut(&e.text, MESSAGE_CHARS), "alert_type": e.alert_type, "priority": e.priority, "source": e.source, "host": e.host, "tags": e.tags.iter().take(ATTRIBUTES).collect::<Vec<_>>()}))
                        .collect();
                    Ok((page_line(page.total as usize, items.len(), "event", "events", "No events found"), json!({"total": page.total, "events": items})))
                }
                "search_hosts" => {
                    let r: hosts::HostSearchRequest = req(json!({"range": range(input, now, "1h")?, "query": input["query"].as_str().unwrap_or(""), "metricFilters": [], "groupBy": null, "sort": {"field": "name", "dir": "asc"}, "offset": 0, "limit": limit(input, 25, 100)}))?;
                    let page = hosts::search(st(), self.org(), Json(r)).await.map_err(err)?.0;
                    let line = page_line(page.total, page.hosts.len(), "host", "hosts", "No hosts found");
                    Ok((line, json!({"total": page.total, "hosts": ser(serde_json::to_value(&page.hosts))?})))
                }
                "search_dashboards" => {
                    let r: dashboards::DashboardSearchRequest = req(json!({"query": input["query"].as_str().unwrap_or(""), "preset": "all", "sort": {"field": "title", "dir": "asc"}, "offset": 0, "limit": limit(input, 20, 50)}))?;
                    let page = dashboards::search(st(), self.org(), CurrentUser(self.ctx.user_id.clone()), Json(r)).await.map_err(err)?.0;
                    let items: Vec<Value> = page
                        .dashboards
                        .iter()
                        .map(|d| json!({"id": d.id, "title": d.title, "description": d.description.as_deref().map(|t| cut(t, VALUE_CHARS)), "kind": d.kind, "teams": d.teams, "modified": iso(d.modified_at)}))
                        .collect();
                    Ok((page_line(page.total as usize, items.len(), "dashboard", "dashboards", "No dashboards found"), json!({"total": page.total, "dashboards": items})))
                }
                "get_dashboard" => {
                    let id = input["id"].as_str().map(str::trim).filter(|i| !i.is_empty() && i.len() <= 200).ok_or("Give the dashboard's id (from search_dashboards)")?;
                    let d = dashboards::load(&self.st, &self.ctx.org_id, &self.ctx.user_id, id).await.map_err(err)?;
                    let widgets = widgets_json(&d.widgets);
                    let line = format!("{}, {}", d.title, plural(widgets.len(), "widget", "widgets"));
                    Ok((line, json!({"id": d.id, "title": d.title, "description": d.description, "template_variables": ser(serde_json::to_value(&d.template_variables))?, "widgets": widgets})))
                }
                "search_vulnerabilities" => {
                    let words: Vec<String> = input["query"].as_str().unwrap_or("").split_whitespace().map(str::to_lowercase).collect();
                    let severities = strings(&input["severity"]);
                    let status = input["status"].as_str().unwrap_or("open");
                    let all = cloud_security::infra_vulnerabilities(&self.st, &self.ctx.org_id).await.map_err(err)?;
                    let hits: Vec<&cloud_security::InfraVulnerability> = all
                        .iter()
                        .filter(|v| status == "all" || v.status == status)
                        .filter(|v| severities.is_empty() || severities.iter().any(|s| s.eq_ignore_ascii_case(&v.severity)))
                        .filter(|v| {
                            let hay = [v.advisory_id.as_str(), v.cve.as_deref().unwrap_or(""), &v.title, &v.package, &v.resource_name, &v.host].join(" ").to_lowercase();
                            words.iter().all(|w| hay.contains(w))
                        })
                        .collect();
                    let shown: Vec<Value> = hits
                        .iter()
                        .take(limit(input, 20, 50) as usize)
                        .map(|v| {
                            json!({"advisory_id": v.advisory_id, "cve": v.cve, "title": cut(&v.title, VALUE_CHARS), "severity": v.severity, "score": v.score, "package": v.package, "version": v.version, "fixed": v.fixed,
                                "resource": v.resource_name, "resource_kind": v.resource_kind, "host": v.host, "env": v.env, "running": v.running, "exploit_available": v.exploit_available, "status": v.status, "last_seen": iso(v.last_seen)})
                        })
                        .collect();
                    Ok((page_line(hits.len(), shown.len(), "vulnerability", "vulnerabilities", "No vulnerabilities found"), json!({"total": hits.len(), "vulnerabilities": shown})))
                }
                "search_code_security_findings" => {
                    let f: code_security::FindingFilter = req(json!({
                        "kind": input["kind"].as_str().unwrap_or(""), "query": input["query"].as_str().unwrap_or(""), "severity": strings(&input["severity"]),
                        "repository": input["repository"].as_str().unwrap_or(""), "status": input["status"].as_str().unwrap_or("open"), "limit": limit(input, 20, 50),
                    }))?;
                    let page = code_security::list_findings(st(), self.org(), Json(f)).await.map_err(err)?.0;
                    let items: Vec<Value> = page
                        .findings
                        .iter()
                        .map(|f| json!({"id": f.id, "kind": f.kind, "tool": f.tool, "rule_id": f.rule_id, "rule": f.rule_name, "severity": f.severity, "message": cut(&f.message, 300), "repository": f.repository, "branch": f.branch, "file": f.file, "line": f.line, "status": f.status, "last_seen": iso(f.last_seen)}))
                        .collect();
                    Ok((page_line(page.total, items.len(), "finding", "findings", "No findings found"), json!({"total": page.total, "findings": items})))
                }
                other => Err(format!("Unknown tool {other}")),
            },
        }
    }
}

impl Toolbox for McpTools {
    fn tools(&self) -> Vec<Value> {
        DEFS.iter().map(definition).collect()
    }

    async fn call(&self, name: &str, args: &Value) -> Option<ToolOutcome> {
        let def = DEFS.iter().find(|d| d.name == name)?;
        let started = Instant::now();
        let result = if !BURST.allow(&self.ctx.user_id, started) {
            Err(format!("Too many tool calls: at most {BURST_CALLS} in {} seconds. Wait a few seconds and retry.", BURST_WINDOW.as_secs()))
        } else {
            match tokio::time::timeout(TIMEOUT, self.run(def, args)).await {
                Ok(r) => r,
                Err(_) => Err(format!("The query took longer than {} seconds: narrow the time range or the query", TIMEOUT.as_secs())),
            }
        };
        let ms = started.elapsed().as_millis() as u64;
        tracing::info!(tool = name, org = %self.ctx.org_id, user = %self.ctx.user_id, ms, ok = result.is_ok(), "mcp tool call");
        self.ctx.audit(&self.st, "mcp.tool_call", name, &cut(&args.to_string(), 500)).await;
        Some(match result {
            Ok((line, v)) => ToolOutcome { text: capped(format!("{line}\n{v}")), is_error: false },
            Err(message) => ToolOutcome { text: message, is_error: true },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_800_000_000_000;

    #[test]
    fn every_tool_has_a_read_only_object_schema() {
        let mut names = std::collections::HashSet::new();
        for d in DEFS {
            assert!(names.insert(d.name), "{} twice", d.name);
            assert!(!d.name.contains("datadog"), "{}: our own tool names", d.name);
            let t = definition(d);
            assert_eq!(t["inputSchema"]["type"], "object", "{}", d.name);
            assert!(t["description"].as_str().is_some_and(|s| s.len() > 20), "{}", d.name);
            assert_eq!(t["annotations"]["readOnlyHint"], true);
            for r in t["inputSchema"]["required"].as_array().unwrap() {
                assert!(t["inputSchema"]["properties"].get(r.as_str().unwrap()).is_some(), "{}: required {r} has no property", d.name);
            }
        }
        // Assist's tools that change something are not offered.
        for d in DEFS {
            if let Run::Assist(name) = d.run {
                assert!(crate::assist::tools::catalog().iter().any(|c| c.0 == name && !c.2), "{name} must be a read-only Assist tool");
            }
        }
        for expected in ["search_logs", "aggregate_logs", "query_metrics", "search_metrics", "search_spans", "get_trace", "list_services", "search_monitors", "get_incident", "search_events", "search_hosts", "get_dashboard", "search_vulnerabilities"] {
            assert!(names.contains(expected), "{expected}");
        }
    }

    #[test]
    fn ranges_are_capped_at_31_days() {
        assert!(range(&json!({"from": "31d"}), NOW, "1h").is_ok());
        assert_eq!(range(&json!({"from": "32d"}), NOW, "1h").unwrap_err(), "The time range may cover at most 31 days: narrow `from` / `to`");
        assert!(range(&json!({"from": "yesterday"}), NOW, "1h").is_err());
        let r = range(&json!({}), NOW, "1h").unwrap();
        assert_eq!((r.from, r.to), (NOW - 3_600_000, NOW));
    }

    #[test]
    fn limits_are_capped() {
        assert_eq!(limit(&json!({"limit": 10_000}), 25, 100), 100);
        assert_eq!(limit(&json!({"limit": 0}), 25, 100), 1);
        assert_eq!(limit(&json!({}), 25, 100), 25);
    }

    #[test]
    fn long_text_is_cut_with_a_note() {
        assert_eq!(cut("héllo", 10), "héllo");
        assert_eq!(cut("héllo wörld", 5), "héllo…");
        let short = "a".repeat(MAX_TEXT);
        assert_eq!(capped(short.clone()), short);
        let long = capped("é".repeat(MAX_TEXT + 7));
        assert!(long.starts_with(&"é".repeat(MAX_TEXT)));
        assert!(long.ends_with("(truncated: 7 more characters; narrow the query, the time range or the limit)"));
    }

    #[test]
    fn logs_keep_key_fields_and_cut_the_rest() {
        let l = logs::LogEvent {
            id: "1".into(),
            timestamp: NOW,
            status: "error".into(),
            service: "order-service".into(),
            host: "i-0a1b2c3d4e5f60001".into(),
            source: "java".into(),
            message: "x".repeat(2_000),
            tags: [("env".to_string(), "prod".to_string())].into(),
            attributes: json!({"http": {"status_code": 502, "url": "/orders"}, "big": "y".repeat(1_000), "none": null}),
            trace_id: Some("abc123".into()),
            span_id: None,
        };
        let v = log_json(&l);
        assert_eq!(v["timestamp"], "2027-01-15T08:00:00.000Z");
        assert_eq!((v["service"].as_str(), v["host"].as_str(), v["trace_id"].as_str()), (Some("order-service"), Some("i-0a1b2c3d4e5f60001"), Some("abc123")));
        assert_eq!(v["message"].as_str().unwrap().chars().count(), MESSAGE_CHARS + 1);
        assert_eq!(v["tags"]["env"], "prod");
        assert_eq!(v["attributes"]["http.status_code"], 502);
        assert_eq!(v["attributes"]["http.url"], "/orders");
        assert_eq!(v["attributes"]["big"].as_str().unwrap().chars().count(), VALUE_CHARS + 1);
        assert!(v["attributes"].get("none").is_none());
        let many: Map<String, Value> = (0..100).map(|i| (format!("k{i:03}"), json!(i))).collect();
        assert_eq!(flatten(&Value::Object(many)).len(), ATTRIBUTES);
    }

    #[test]
    fn metric_series_are_summarized_with_capped_points() {
        let s = metrics::MetricSeries { name: "host:web-1".into(), points: (0..120).map(|i| (NOW + i * 60_000, i as f64)).chain([(NOW + 200 * 60_000, f64::NAN)]).collect(), unit: Some("percent".into()) };
        let v = series_json(&s, 30);
        assert_eq!((v["last"].as_f64(), v["min"].as_f64(), v["max"].as_f64(), v["avg"].as_f64(), v["count"].as_u64()), (Some(119.0), Some(0.0), Some(119.0), Some(59.5), Some(120)));
        let points = v["points"].as_array().unwrap();
        assert_eq!(points.len(), 30);
        assert_eq!(points.last().unwrap()[1], 119.0);
        assert!(series_json(&s, 0).get("points").is_none());
    }

    #[test]
    fn traces_are_ordered_and_capped() {
        let span = |i: usize| apm::TraceSpan {
            span_id: format!("{i:016x}"),
            parent_span_id: String::new(),
            service: "storefront-api".into(),
            name: "GET /cart".into(),
            kind: "server".into(),
            start_us: (300 - i) as f64 * 1000.0,
            duration_us: 1500.0,
            status: "ok".into(),
            status_message: String::new(),
            attributes: [("http.method".to_string(), "GET".to_string())].into(),
            resource: HashMap::new(),
            events: vec![],
        };
        let t = apm::TraceDetail { trace_id: "abc".into(), start: NOW, duration_ms: 300.0, spans: (0..300).map(span).collect() };
        let v = trace_json(&t);
        assert_eq!(v["total"], 300);
        let spans = v["spans"].as_array().unwrap();
        assert_eq!(spans.len(), TRACE_SPANS);
        assert_eq!(spans[0]["start_ms"], 1.0);
        assert_eq!(spans[0]["duration_ms"], 1.5);
        assert_eq!(spans[0]["attributes"]["http.method"], "GET");
    }

    #[test]
    fn dashboard_widgets_are_listed_with_their_queries() {
        let widgets = vec![
            json!({"id": "a", "definition": {"type": "timeseries", "title": "CPU", "requests": [{"query": "avg:system.cpu.user{*} by {host}"}]}}),
            json!({"id": "b", "definition": {"type": "group", "title": "Logs", "widgets": [{"id": "c", "definition": {"type": "log_stream", "title": "Errors", "query": "status:error"}}]}}),
        ];
        let v = widgets_json(&widgets);
        assert_eq!(v.len(), 3);
        assert_eq!(v[0], json!({"type": "timeseries", "title": "CPU", "queries": ["avg:system.cpu.user{*} by {host}"]}));
        assert_eq!(v[1], json!({"type": "group", "title": "Logs"}));
        assert_eq!(v[2], json!({"type": "log_stream", "title": "Errors", "group": "Logs", "queries": ["status:error"]}));
        let many: Vec<Value> = (0..500).map(|_| json!({"definition": {"type": "note"}})).collect();
        assert_eq!(widgets_json(&many).len(), WIDGETS);
    }

    #[test]
    fn bursts_are_limited_per_user() {
        let b = Burst::new(3, Duration::from_secs(10));
        let t0 = Instant::now();
        assert!((0..3).all(|_| b.allow("u1", t0)));
        assert!(!b.allow("u1", t0 + Duration::from_secs(1)));
        assert!(b.allow("u2", t0 + Duration::from_secs(1)), "another user has their own budget");
        assert!(b.allow("u1", t0 + Duration::from_secs(10)), "the window slides");
    }

    #[test]
    fn summary_lines() {
        assert_eq!(page_line(612, 25, "log", "logs", "No logs"), "612 logs (showing 25)");
        assert_eq!(page_line(1, 1, "log", "logs", "No logs"), "1 log");
        assert_eq!(page_line(0, 0, "log", "logs", "No logs"), "No logs");
    }
}
