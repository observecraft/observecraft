//! Continuous profiles from Datadog's profilers (dd-trace-* with profiling on): the Agent forwards
//! each upload (multipart: `event.json` and gzip pprof files) to the intake, which passes it to the
//! server's internal listener; [`ingest`] stores one `otel.profiles` row per pprof file.
//!
//! The API lists profiles and merges a service's profiles over a time range into a flame graph
//! for one sample type (cpu-time, wall-time, alloc-space, …).

use std::collections::HashMap;
use std::io::Read;

use axum::extract::{Multipart, State};
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use clickhouse::Row;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;

use super::logs::TimeRange;
use crate::auth::Writer;
use crate::error::{ApiError, ApiResult};
use crate::state::{AppState, OrgId};

// ================================================================ pprof

/// perftools.profiles (pprof's profile.proto), the fields a flame graph needs.
pub mod pprof {
    #[derive(Clone, PartialEq, prost::Message)]
    pub struct Profile {
        #[prost(message, repeated, tag = "1")]
        pub sample_type: Vec<ValueType>,
        #[prost(message, repeated, tag = "2")]
        pub sample: Vec<Sample>,
        #[prost(message, repeated, tag = "4")]
        pub location: Vec<Location>,
        #[prost(message, repeated, tag = "5")]
        pub function: Vec<Function>,
        #[prost(string, repeated, tag = "6")]
        pub string_table: Vec<String>,
    }
    #[derive(Clone, PartialEq, prost::Message)]
    pub struct ValueType {
        #[prost(int64, tag = "1")]
        pub r#type: i64,
        #[prost(int64, tag = "2")]
        pub unit: i64,
    }
    #[derive(Clone, PartialEq, prost::Message)]
    pub struct Sample {
        #[prost(uint64, repeated, tag = "1")]
        pub location_id: Vec<u64>,
        #[prost(int64, repeated, tag = "2")]
        pub value: Vec<i64>,
        #[prost(message, repeated, tag = "3")]
        pub label: Vec<Label>,
    }
    #[derive(Clone, PartialEq, prost::Message)]
    pub struct Label {
        #[prost(int64, tag = "1")]
        pub key: i64,
        #[prost(int64, tag = "2")]
        pub str: i64,
        #[prost(int64, tag = "3")]
        pub num: i64,
    }
    #[derive(Clone, PartialEq, prost::Message)]
    pub struct Location {
        #[prost(uint64, tag = "1")]
        pub id: u64,
        #[prost(message, repeated, tag = "4")]
        pub line: Vec<Line>,
    }
    #[derive(Clone, PartialEq, prost::Message)]
    pub struct Line {
        #[prost(uint64, tag = "1")]
        pub function_id: u64,
        #[prost(int64, tag = "2")]
        pub line: i64,
    }
    #[derive(Clone, PartialEq, prost::Message)]
    pub struct Function {
        #[prost(uint64, tag = "1")]
        pub id: u64,
        #[prost(int64, tag = "2")]
        pub name: i64,
        #[prost(int64, tag = "4")]
        pub filename: i64,
    }
}

/// A pprof file as uploaded: gzip-compressed or plain.
pub fn decode_pprof(data: &[u8]) -> Result<pprof::Profile, String> {
    use prost::Message;
    let raw = if data.starts_with(&[0x1f, 0x8b]) {
        let mut out = Vec::new();
        flate2::read::GzDecoder::new(data).take(256 * 1024 * 1024).read_to_end(&mut out).map_err(|e| format!("gzip: {e}"))?;
        out
    } else {
        data.to_vec()
    };
    pprof::Profile::decode(raw.as_slice()).map_err(|e| format!("pprof: {e}"))
}

#[derive(Serialize, ToSchema, Clone, PartialEq, Debug)]
pub struct SampleType {
    /// e.g. cpu-time, wall-time, alloc-space, inuse-space, lock-acquire-wait
    pub r#type: String,
    /// e.g. nanoseconds, bytes, count
    pub unit: String,
}

/// How a sample type reads in the UI and how it aggregates over time.
#[derive(Serialize, ToSchema, Clone, PartialEq, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ProfileMetric {
    /// The profiler's name: cpu-time, wall-time, alloc-space, heap-space, …
    pub r#type: String,
    /// nanoseconds, bytes or count.
    pub unit: String,
    /// Datadog's name: CPU Time, Wall Time, Heap Live Size, …
    pub label: String,
    /// EXECUTION, ELAPSED TIME, MEMORY, LOCK, EXCEPTIONS, THREADS or OTHER.
    pub category: String,
    /// Gauges (heap live size, goroutines) are averaged; the other types add up over time.
    pub gauge: bool,
}

/// The profilers' spellings of one sample type: dd-trace-py/-java/-dotnet use the first name, Go's
/// runtime profiles the others. The UI and API use the first.
const ALIASES: &[&[&str]] = &[
    &["cpu-time", "cpu"],
    &["cpu-samples", "samples"],
    &["wall-time", "wall"],
    &["alloc-space", "alloc_space", "alloc-size"],
    &["alloc-samples", "alloc_objects"],
    &["heap-space", "inuse-space", "inuse_space", "heap-live-size"],
    &["heap-live-samples", "inuse_objects", "inuse-objects"],
];

/// The canonical name of a sample type.
pub fn canonical(t: &str) -> &str {
    ALIASES.iter().find(|a| a.contains(&t)).map(|a| a[0]).unwrap_or(t)
}

/// Every spelling of a canonical sample type.
fn spellings(t: &str) -> Vec<String> {
    ALIASES.iter().find(|a| a[0] == t).map(|a| a.iter().map(|s| s.to_string()).collect()).unwrap_or_else(|| vec![t.to_string()])
}

/// Datadog's name, category and aggregation of a sample type (any spelling; see [`ALIASES`]).
pub fn metric(t: &str, unit: &str) -> ProfileMetric {
    let t = canonical(t);
    let (label, category, gauge) = match t {
        "cpu-time" | "cpu" => ("CPU Time", "EXECUTION", false),
        "cpu-samples" | "samples" => ("CPU Samples", "EXECUTION", false),
        "wall-time" | "wall" => ("Wall Time", "ELAPSED TIME", false),
        "wall-samples" => ("Wall Samples", "ELAPSED TIME", false),
        "alloc-space" | "alloc_space" | "alloc-size" => ("Allocated Memory", "MEMORY", false),
        "alloc-samples" | "alloc_objects" => ("Allocations", "MEMORY", false),
        "heap-space" | "inuse-space" | "inuse_space" | "heap-live-size" => ("Heap Live Size", "MEMORY", true),
        "inuse_objects" | "inuse-objects" | "heap-live-samples" => ("Heap Live Objects", "MEMORY", true),
        "lock-acquire-wait" => ("Lock Wait Time", "LOCK", false),
        "lock-acquire" => ("Lock Acquires", "LOCK", false),
        "lock-release-hold" => ("Lock Hold Time", "LOCK", false),
        "lock-release" => ("Lock Releases", "LOCK", false),
        "contentions" => ("Lock Contentions", "LOCK", false),
        "delay" => ("Lock Delay", "LOCK", false),
        "exception-samples" => ("Thrown Exceptions", "EXCEPTIONS", false),
        "goroutines" => ("Goroutines", "THREADS", true),
        _ => (t, "OTHER", false),
    };
    let unit = if !unit.is_empty() {
        unit
    } else if t.contains("time") || t.contains("wait") || t.contains("hold") || t == "cpu" || t == "wall" || t == "delay" {
        "nanoseconds"
    } else if t.contains("space") || t.contains("size") {
        "bytes"
    } else {
        "count"
    };
    ProfileMetric { r#type: t.into(), unit: unit.into(), label: label.into(), category: category.into(), gauge }
}

/// The default sample type: CPU time.
const DEFAULT_TYPE: &str = "cpu-time";

/// A frame: function and source file.
type Frame = (String, String);

#[derive(Serialize, ToSchema, Clone, Debug)]
#[schema(no_recursion)]
pub struct FlameNode {
    /// Function name (the root is "all").
    pub name: String,
    /// Source file, when the profiler reports one.
    pub file: String,
    pub value: i64,
    pub children: Vec<FlameNode>,
}

/// A flame graph being built: frames under each node.
#[derive(Default)]
struct Builder {
    value: i64,
    children: HashMap<Frame, Builder>,
}

impl Builder {
    fn add(&mut self, stack: &[Frame], value: i64) {
        self.value += value;
        if let Some((first, rest)) = stack.split_first() {
            self.children.entry(first.clone()).or_default().add(rest, value);
        }
    }

    /// The tree, dropping frames below `min` (so the graph stays readable).
    fn finish(self, (name, file): Frame, min: i64) -> FlameNode {
        let mut children: Vec<FlameNode> = self.children.into_iter().filter(|(_, c)| c.value >= min).map(|(f, c)| c.finish(f, min)).collect();
        children.sort_by_key(|c| std::cmp::Reverse(c.value));
        FlameNode { name, file, value: self.value, children }
    }
}

/// The sample types of a profile.
pub fn sample_types(p: &pprof::Profile) -> Vec<SampleType> {
    let s = |i: i64| p.string_table.get(i as usize).cloned().unwrap_or_default();
    p.sample_type.iter().map(|t| SampleType { r#type: s(t.r#type), unit: s(t.unit) }).collect()
}

/// The sum of each sample type over a profile's samples.
pub fn totals(p: &pprof::Profile) -> Vec<(String, i64)> {
    sample_types(p).into_iter().enumerate().map(|(i, t)| (t.r#type, p.sample.iter().map(|s| s.value.get(i).copied().unwrap_or(0).max(0)).sum())).collect()
}

/// The pprof label Datadog's profilers put on samples taken inside a traced request.
const ENDPOINT_LABEL: &str = "trace endpoint";

/// Profiles merged for one sample type: the flame graph, self time per function and the split by
/// endpoint.
#[derive(Default)]
struct Merge {
    tree: Builder,
    functions: HashMap<Frame, i64>,
    /// "" = samples outside any traced endpoint.
    endpoints: HashMap<String, i64>,
}

/// Adds a profile's samples of `kind`; with `endpoint`, only that endpoint's samples go into the
/// tree and function list (the endpoint split always covers every sample).
fn add_profile(m: &mut Merge, p: &pprof::Profile, kind: &str, endpoint: Option<&str>) {
    let Some(idx) = sample_types(p).iter().position(|t| canonical(&t.r#type) == kind) else {
        return;
    };
    let s = |i: i64| p.string_table.get(i as usize).map(String::as_str).unwrap_or("");
    let functions: HashMap<u64, Frame> = p.function.iter().map(|f| (f.id, (s(f.name).to_string(), s(f.filename).to_string()))).collect();
    // A location can hold inlined frames: callee first.
    let unknown: Frame = ("?".into(), String::new());
    let locations: HashMap<u64, Vec<Frame>> = p.location.iter().map(|l| (l.id, l.line.iter().map(|ln| functions.get(&ln.function_id).cloned().unwrap_or_else(|| unknown.clone())).collect())).collect();
    for sample in &p.sample {
        let value = sample.value.get(idx).copied().unwrap_or(0);
        if value <= 0 {
            continue;
        }
        let ep = sample.label.iter().find(|l| s(l.key) == ENDPOINT_LABEL).map(|l| s(l.str)).unwrap_or("");
        *m.endpoints.entry(ep.to_string()).or_default() += value;
        if endpoint.is_some_and(|e| e != ep) {
            continue;
        }
        // location_id is leaf first; a flame graph reads root first.
        let mut stack: Vec<Frame> = sample.location_id.iter().flat_map(|id| locations.get(id).cloned().unwrap_or_default()).collect();
        if let Some(leaf) = stack.first() {
            *m.functions.entry(leaf.clone()).or_default() += value;
        }
        stack.reverse();
        m.tree.add(&stack, value);
    }
}

/// The library a frame belongs to, for the "Package" grouping: the Python distribution (or
/// `stdlib`), the Java/.NET namespace, the Go import path. Empty for application code.
pub fn package(name: &str, file: &str) -> String {
    for marker in ["site-packages/", "dist-packages/"] {
        if let Some(i) = file.find(marker) {
            let rest = &file[i + marker.len()..];
            let first = rest.split('/').next().unwrap_or(rest);
            return first.trim_end_matches(".py").to_string();
        }
    }
    if file.contains("/lib/python") || file.starts_with("<frozen") {
        return "stdlib".into();
    }
    if file.ends_with(".py") || file.ends_with(".rb") || file.ends_with(".js") || file.ends_with(".php") {
        return String::new();
    }
    // Go: github.com/org/repo/pkg.(*Type).Method → github.com/org/repo/pkg
    if name.contains('/') {
        let (dir, last) = name.rsplit_once('/').unwrap_or(("", name));
        let pkg = last.split('.').next().unwrap_or(last);
        return format!("{dir}/{pkg}");
    }
    // Java / .NET: com.example.shop.Cart.add → com.example.shop
    let parts: Vec<&str> = name.split('(').next().unwrap_or(name).split('.').collect();
    if parts.len() > 2 {
        return parts[..parts.len() - 2].join(".");
    }
    // Rust / C++ / PHP: shop::cart::add → shop
    match name.split_once("::") {
        Some((first, _)) => first.to_string(),
        None => String::new(),
    }
}

// ================================================================ ingestion (internal)

#[derive(Row, Serialize)]
struct ProfileRow {
    #[serde(rename = "OrgId")]
    org_id: String,
    #[serde(rename = "Start")]
    start: i64,
    #[serde(rename = "End")]
    end: i64,
    #[serde(rename = "Service")]
    service: String,
    #[serde(rename = "Env")]
    env: String,
    #[serde(rename = "Version")]
    version: String,
    #[serde(rename = "Host")]
    host: String,
    #[serde(rename = "Language")]
    language: String,
    #[serde(rename = "RuntimeId")]
    runtime_id: String,
    #[serde(rename = "Attachment")]
    attachment: String,
    #[serde(rename = "Tags")]
    tags: Vec<(String, String)>,
    #[serde(rename = "SampleTotals")]
    sample_totals: Vec<(String, i64)>,
    #[serde(rename = "Pprof", with = "serde_bytes")]
    pprof: Vec<u8>,
}

/// RFC 3339 → epoch nanoseconds.
fn nanos(ts: &str) -> Option<i64> {
    // e.g. 2026-10-26T09:31:20.123456Z: parse with ClickHouse-free arithmetic.
    let (date, time) = ts.trim_end_matches('Z').split_once('T')?;
    let mut d = date.split('-').map(|x| x.parse::<i64>().ok());
    let (y, m, day) = (d.next()??, d.next()??, d.next()??);
    let (hms, frac) = time.split_once('.').unwrap_or((time, "0"));
    let mut t = hms.split(':').map(|x| x.parse::<i64>().ok());
    let (h, mi, sec) = (t.next()??, t.next()??, t.next()??);
    let frac_ns: i64 = format!("{:0<9}", &frac.chars().take(9).collect::<String>()).parse().ok()?;
    // Days from the civil date (Howard Hinnant's algorithm).
    let (y2, m2) = if m <= 2 { (y - 1, m + 9) } else { (y, m - 3) };
    let era = y2.div_euclid(400);
    let yoe = y2 - era * 400;
    let doy = (153 * m2 + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some(((days * 86_400 + h * 3600 + mi * 60 + sec) * 1_000_000_000) + frac_ns)
}

/// `key:value,key2:value2` → pairs.
fn tag_pairs(s: &str) -> Vec<(String, String)> {
    s.split(',').filter_map(|t| t.split_once(':')).map(|(k, v)| (k.trim().to_string(), v.trim().to_string())).collect()
}

/// `POST /ingest/profiles` on the internal listener: one profiler upload, from the intake (which
/// sets `x-oc-org` after checking the API key).
pub async fn ingest(State(st): State<AppState>, headers: HeaderMap, mut form: Multipart) -> StatusCode {
    if let Some(token) = &st.config.internal_token {
        let ok = headers.get("authorization").and_then(|v| v.to_str().ok()).and_then(|v| v.strip_prefix("Bearer ")) == Some(token.as_str());
        if !ok {
            return StatusCode::UNAUTHORIZED;
        }
    }
    let Some(org) = headers.get("x-oc-org").and_then(|v| v.to_str().ok()).filter(|o| !o.is_empty()).map(str::to_string) else {
        return StatusCode::BAD_REQUEST;
    };
    let mut event = Value::Null;
    let mut files: Vec<(String, Vec<u8>)> = Vec::new();
    loop {
        match form.next_field().await {
            Ok(Some(field)) => {
                let file = field.file_name().unwrap_or_default().to_string();
                let name = field.name().unwrap_or_default().to_string();
                let Ok(data) = field.bytes().await else {
                    return StatusCode::BAD_REQUEST;
                };
                if name == "event" {
                    event = serde_json::from_slice(&data).unwrap_or(Value::Null);
                } else if file.ends_with(".pprof") {
                    files.push((file, data.to_vec()));
                }
            }
            Ok(None) => break,
            Err(_) => return StatusCode::BAD_REQUEST,
        }
    }
    // Tags: the profiler's (service, env, version, host, runtime-id, …) plus the Agent's.
    let mut tags = tag_pairs(event.get("tags_profiler").and_then(Value::as_str).unwrap_or(""));
    tags.extend(tag_pairs(headers.get("x-datadog-additional-tags").and_then(|v| v.to_str().ok()).unwrap_or("")));
    let tag = |k: &str| tags.iter().find(|(key, _)| key == k).map(|(_, v)| v.clone()).unwrap_or_default();
    let start = event.get("start").and_then(Value::as_str).and_then(nanos).unwrap_or(0);
    let end = event.get("end").and_then(Value::as_str).and_then(nanos).unwrap_or(start);
    let (service, env, version, host, runtime_id) = (tag("service"), tag("env"), tag("version"), tag("host"), tag("runtime-id"));
    let language = event.get("family").and_then(Value::as_str).map(str::to_string).unwrap_or_else(|| tag("language"));
    if files.is_empty() || start == 0 {
        return StatusCode::BAD_REQUEST;
    }
    let result = async {
        let mut insert = st.clickhouse.insert("otel.profiles")?;
        for (attachment, pprof) in files {
            insert
                .write(&ProfileRow {
                    org_id: org.clone(),
                    start,
                    end,
                    service: service.clone(),
                    env: env.clone(),
                    version: version.clone(),
                    host: host.clone(),
                    language: language.clone(),
                    runtime_id: runtime_id.clone(),
                    attachment,
                    tags: tags.clone(),
                    sample_totals: decode_pprof(&pprof).map(|p| totals(&p)).unwrap_or_default(),
                    pprof,
                })
                .await?;
        }
        insert.end().await
    }
    .await;
    match result {
        Ok(()) => StatusCode::ACCEPTED,
        Err(e) => {
            tracing::warn!(error = %e, "storing profile failed");
            StatusCode::SERVICE_UNAVAILABLE
        }
    }
}

// ================================================================ API

/// Profile filters from a Datadog-style query (`service:checkout env:prod -version:1.2 team:web`):
/// core keys map to columns, other keys to the profiler's tags. Values are bound, never inlined.
struct Filter {
    sql: String,
    binds: Vec<String>,
}

fn filter(query: &str, skip: Option<&str>) -> Filter {
    let mut groups: std::collections::BTreeMap<(String, bool), Vec<String>> = Default::default();
    for term in oc_query::parse_query(query) {
        // Free text has nothing to match in a profile.
        let oc_query::Term::Tag { key, values, negate } = term else { continue };
        if key.is_empty() || Some(key.as_str()) == skip {
            continue;
        }
        groups.entry((key, negate)).or_default().extend(values.into_iter().filter(|v| !v.is_empty()));
    }
    groups.retain(|_, v| !v.is_empty());
    let mut f = Filter { sql: String::new(), binds: Vec::new() };
    for ((key, negate), values) in groups {
        let column = match key.as_str() {
            "service" => Some("toString(Service)"),
            "env" => Some("toString(Env)"),
            "version" => Some("Version"),
            "host" => Some("Host"),
            "language" => Some("toString(Language)"),
            "runtime-id" | "runtime_id" => Some("RuntimeId"),
            _ => None,
        };
        let expr = match column {
            Some(c) => c.to_string(),
            None => {
                f.binds.push(key);
                "Tags[?]".to_string()
            }
        };
        let list = vec!["?"; values.len()].join(", ");
        f.sql.push_str(&format!(" AND {expr} {}IN ({list})", if negate { "NOT " } else { "" }));
        f.binds.extend(values);
    }
    f
}

/// The legacy `service`/`env` fields as query terms.
fn with_fields(query: &str, service: &str, env: &str) -> String {
    let mut q = query.to_string();
    for (k, v) in [("service", service), ("env", env)] {
        if !v.is_empty() {
            q.push_str(&format!(" {k}:{v}"));
        }
    }
    q
}

fn bind_all(mut q: clickhouse::query::Query, binds: &[String]) -> clickhouse::query::Query {
    for b in binds {
        q = q.bind(b);
    }
    q
}

/// Profiles in the range (bind: from, to); queries put the org filter before it.
const IN_RANGE: &str = "Start >= fromUnixTimestamp64Milli(?) AND Start <= fromUnixTimestamp64Milli(?)";

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProfileSearchRequest {
    pub range: TimeRange,
    /// Filters: `service:x env:prod version:1.2 host:h team:web`.
    #[serde(default)]
    pub query: String,
    /// Only this service ("" = all).
    #[serde(default)]
    pub service: String,
    #[serde(default)]
    pub env: String,
    pub limit: usize,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProfileSummary {
    /// Epoch milliseconds.
    pub start: i64,
    pub end: i64,
    pub service: String,
    pub env: String,
    pub version: String,
    pub host: String,
    pub language: String,
    pub runtime_id: String,
    pub attachment: String,
    /// Total per sample type over the profile.
    pub totals: HashMap<String, i64>,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProfileList {
    pub items: Vec<ProfileSummary>,
    /// Services with profiles in the range.
    pub services: Vec<String>,
}

/// Profiles in the range, newest first (Profile List).
#[utoipa::path(post, path = "/api/v1/profiles/search", operation_id = "search_profiles", request_body = ProfileSearchRequest, responses((status = 200, body = ProfileList)), tag = "profiles")]
pub async fn search(State(st): State<AppState>, OrgId(org): OrgId, Json(req): Json<ProfileSearchRequest>) -> ApiResult<ProfileList> {
    req.range.validate()?;
    #[derive(Row, Deserialize)]
    struct Item {
        start: i64,
        end: i64,
        service: String,
        env: String,
        version: String,
        host: String,
        language: String,
        runtime_id: String,
        attachment: String,
        totals: Vec<(String, i64)>,
    }
    let f = filter(&with_fields(&req.query, &req.service, &req.env), None);
    let ch = st.ch(&org);
    let sql = format!(
        "SELECT toInt64(toUnixTimestamp64Milli(Start)) AS start, toInt64(toUnixTimestamp64Milli(End)) AS end, toString(Service) AS service, toString(Env) AS env, \
         Version AS version, Host AS host, toString(Language) AS language, RuntimeId AS runtime_id, toString(Attachment) AS attachment, \
         CAST(SampleTotals, 'Array(Tuple(String, Int64))') AS totals \
         FROM otel.profiles WHERE OrgId = {{oc_org:String}} AND {IN_RANGE}{} ORDER BY Start DESC LIMIT ?",
        f.sql
    );
    let items = bind_all(ch.query(&sql).bind(req.range.from).bind(req.range.to), &f.binds).bind(req.limit.clamp(1, 500) as u64).fetch_all::<Item>();
    let services = ch.query(&format!("SELECT DISTINCT toString(Service) FROM otel.profiles WHERE OrgId = {{oc_org:String}} AND {IN_RANGE} ORDER BY 1")).bind(req.range.from).bind(req.range.to).fetch_all::<String>();
    let (items, services) = tokio::try_join!(items, services)?;
    Ok(Json(ProfileList {
        items: items
            .into_iter()
            .map(|i| ProfileSummary {
                start: i.start,
                end: i.end,
                service: i.service,
                env: i.env,
                version: i.version,
                host: i.host,
                language: i.language,
                runtime_id: i.runtime_id,
                attachment: i.attachment,
                totals: i.totals.into_iter().collect(),
            })
            .collect(),
        services,
    }))
}

// ---------------------------------------------------------------- overview (explorer chart, services, facets)

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProfileOverviewRequest {
    pub range: TimeRange,
    #[serde(default)]
    pub query: String,
    /// The sample type charted; default: CPU time, else the first type present.
    #[serde(default)]
    pub sample_type: String,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProfileSeries {
    pub service: String,
    /// `[epoch ms, value]`: units per second for summed types (CPU nanoseconds per second = cores),
    /// the average for gauges.
    pub points: Vec<(i64, f64)>,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProfileServiceTotal {
    pub service: String,
    pub language: String,
    /// Over the whole range: units per second for summed types, the average for gauges.
    pub value: f64,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProfileFacetValue {
    pub value: String,
    pub count: u64,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProfileFacet {
    /// env, service, version or host.
    pub key: String,
    pub values: Vec<ProfileFacetValue>,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProfileOverview {
    /// Sample types present in the range.
    pub metrics: Vec<ProfileMetric>,
    /// The one charted (absent when there are no profiles).
    pub metric: Option<ProfileMetric>,
    pub step_ms: i64,
    pub series: Vec<ProfileSeries>,
    pub services: Vec<ProfileServiceTotal>,
    pub facets: Vec<ProfileFacet>,
}

const FACETS: [(&str, &str); 4] = [("env", "toString(Env)"), ("service", "toString(Service)"), ("version", "Version"), ("host", "Host")];

/// Explorer: the metric over time per service, each service's total and the facets.
#[utoipa::path(post, path = "/api/v1/profiles/overview", operation_id = "profile_overview", request_body = ProfileOverviewRequest, responses((status = 200, body = ProfileOverview)), tag = "profiles")]
pub async fn overview(State(st): State<AppState>, OrgId(org): OrgId, Json(req): Json<ProfileOverviewRequest>) -> ApiResult<ProfileOverview> {
    req.range.validate()?;
    let ch = st.ch(&org);
    let f = filter(&req.query, None);
    let types =
        bind_all(ch.query(&format!("SELECT DISTINCT arrayJoin(mapKeys(SampleTotals)) AS t FROM otel.profiles WHERE OrgId = {{oc_org:String}} AND {IN_RANGE}{} ORDER BY t", f.sql)).bind(req.range.from).bind(req.range.to), &f.binds)
            .fetch_all::<String>();
    let facets =
        futures::future::try_join_all(FACETS.iter().map(|(key, column)| {
            let f = filter(&req.query, Some(key));
            let sql = format!("SELECT {column} AS v, count() AS n FROM otel.profiles WHERE OrgId = {{oc_org:String}} AND {IN_RANGE}{} AND v != '' GROUP BY v ORDER BY n DESC, v LIMIT 100", f.sql);
            let q = bind_all(ch.query(&sql).bind(req.range.from).bind(req.range.to), &f.binds);
            async move {
                q.fetch_all::<(String, u64)>().await.map(|rows| ProfileFacet { key: key.to_string(), values: rows.into_iter().map(|(value, count)| ProfileFacetValue { value, count }).collect() })
            }
        }));
    let (types, facets) = tokio::try_join!(types, facets)?;
    let mut metrics: Vec<ProfileMetric> = Vec::new();
    for m in types.iter().map(|t| metric(t, "")) {
        if !metrics.iter().any(|x| x.r#type == m.r#type) {
            metrics.push(m);
        }
    }
    metrics.sort_by_key(|m| (["EXECUTION", "MEMORY", "ELAPSED TIME", "LOCK", "EXCEPTIONS", "THREADS"].iter().position(|c| *c == m.category).unwrap_or(9), m.label.clone()));
    let selected = if req.sample_type.is_empty() {
        metrics.iter().find(|m| m.r#type == DEFAULT_TYPE).or(metrics.first()).cloned()
    } else {
        metrics.iter().find(|m| m.r#type == canonical(&req.sample_type)).cloned()
    };
    let span_ms = (req.range.to - req.range.from).max(1);
    // About 60 points, never under one profile period (60 s).
    let step_ms = ((span_ms / 60).max(60_000) / 60_000) * 60_000;
    let Some(m) = selected else {
        return Ok(Json(ProfileOverview { metrics, metric: None, step_ms, series: vec![], services: vec![], facets }));
    };

    #[derive(Row, Deserialize)]
    struct Bucket {
        service: String,
        language: String,
        t: i64,
        v: f64,
        n: u64,
        runtimes: u64,
    }
    let sql = format!(
        "SELECT toString(Service) AS service, toString(any(Language)) AS language, toInt64(toUnixTimestamp(toStartOfInterval(Start, toIntervalSecond(?)))) * 1000 AS t, \
         toFloat64(sum(arraySum(arrayMap(k -> SampleTotals[k], ?)))) AS v, count() AS n, uniqExact(RuntimeId) AS runtimes \
         FROM otel.profiles WHERE OrgId = {{oc_org:String}} AND {IN_RANGE}{} AND hasAny(mapKeys(SampleTotals), ?) GROUP BY service, t ORDER BY t",
        f.sql
    );
    let names = spellings(&m.r#type);
    let rows = bind_all(ch.query(&sql).bind(step_ms / 1000).bind(&names).bind(req.range.from).bind(req.range.to), &f.binds).bind(&names).fetch_all::<Bucket>().await?;
    let step_s = step_ms as f64 / 1000.0;
    let value = |b: &Bucket| {
        if m.gauge {
            b.v * b.runtimes.max(1) as f64 / b.n.max(1) as f64
        } else {
            b.v / step_s
        }
    };
    let mut series: Vec<ProfileSeries> = Vec::new();
    let mut totals: HashMap<String, (String, f64, usize)> = HashMap::new();
    for b in &rows {
        let v = value(b);
        match series.iter_mut().find(|s| s.service == b.service) {
            Some(s) => s.points.push((b.t, v)),
            None => series.push(ProfileSeries { service: b.service.clone(), points: vec![(b.t, v)] }),
        }
        let e = totals.entry(b.service.clone()).or_insert((b.language.clone(), 0.0, 0));
        e.1 += if m.gauge { v } else { b.v };
        e.2 += 1;
    }
    let span_s = span_ms as f64 / 1000.0;
    let mut services: Vec<ProfileServiceTotal> =
        totals.into_iter().map(|(service, (language, sum, buckets))| ProfileServiceTotal { service, language, value: if m.gauge { sum / buckets.max(1) as f64 } else { sum / span_s } }).collect();
    services.sort_by(|a, b| b.value.total_cmp(&a.value).then(a.service.cmp(&b.service)));
    Ok(Json(ProfileOverview { metrics, metric: Some(m), step_ms, series, services, facets }))
}

// ---------------------------------------------------------------- flame graph

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct FlameGraphRequest {
    pub range: TimeRange,
    /// Filters: `service:x env:prod version:1.2 host:h`.
    #[serde(default)]
    pub query: String,
    #[serde(default)]
    pub service: String,
    #[serde(default)]
    pub env: String,
    /// Restrict to one profiler process (a profile's runtime id), or all of the service's.
    #[serde(default)]
    pub runtime_id: String,
    /// Sample type (cpu-time, wall-time, alloc-space, …); default: CPU time, else the first one.
    #[serde(default)]
    pub sample_type: String,
    /// Only samples taken in this endpoint ("" = samples outside any endpoint); absent = all.
    #[serde(default)]
    pub endpoint: Option<String>,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProfileCard {
    pub metric: ProfileMetric,
    /// Per minute for summed types, the average for gauges.
    pub value: f64,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProfileEndpoint {
    /// "" = activity outside a traced endpoint.
    pub name: String,
    pub value: i64,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProfileFunction {
    pub name: String,
    pub file: String,
    /// Python distribution, Java/.NET namespace, Go import path; "" for application code.
    pub package: String,
    /// Self value (the function at the top of the stack).
    pub value: i64,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct FlameGraph {
    pub sample_types: Vec<SampleType>,
    pub sample_type: String,
    pub unit: String,
    pub metric: Option<ProfileMetric>,
    /// Profiles merged.
    pub profiles: u64,
    /// Multiply a frame's value by this for Datadog's "Average …, per minute" (summed types) or
    /// "Average …" (gauges).
    pub scale: f64,
    /// One card per sample type: its per-minute value or average.
    pub cards: Vec<ProfileCard>,
    pub endpoints: Vec<ProfileEndpoint>,
    /// Self value per function, largest first.
    pub functions: Vec<ProfileFunction>,
    pub root: FlameNode,
}

/// Most profiles merged into one flame graph.
const MAX_MERGED: u64 = 300;
/// Functions listed beside the flame graph.
const MAX_FUNCTIONS: usize = 500;

/// Per-type accumulators for the cards and the scale.
#[derive(Default)]
struct TypeStats {
    total: i64,
    profiles: u64,
    minutes: f64,
    runtimes: std::collections::HashSet<String>,
}

impl TypeStats {
    /// Per minute across the service's instances (summed types: profile minutes / instances);
    /// the average profile times the instances (gauges).
    fn scale(&self, gauge: bool) -> f64 {
        let runtimes = self.runtimes.len().max(1) as f64;
        if gauge {
            runtimes / self.profiles.max(1) as f64
        } else {
            runtimes / self.minutes.max(1.0 / 60.0)
        }
    }
}

/// The matching profiles over the range, merged into one flame graph.
#[utoipa::path(post, path = "/api/v1/profiles/flamegraph", operation_id = "profile_flamegraph", request_body = FlameGraphRequest, responses((status = 200, body = FlameGraph)), tag = "profiles")]
pub async fn flamegraph(State(st): State<AppState>, OrgId(org): OrgId, Json(req): Json<FlameGraphRequest>) -> ApiResult<FlameGraph> {
    req.range.validate()?;
    #[derive(Row, Deserialize)]
    struct Blob {
        #[serde(with = "serde_bytes")]
        pprof: Vec<u8>,
        duration_ms: i64,
        runtime_id: String,
    }
    let mut query = with_fields(&req.query, &req.service, &req.env);
    if !req.runtime_id.is_empty() {
        query.push_str(&format!(" runtime-id:{}", req.runtime_id));
    }
    let f = filter(&query, None);
    let sql = format!(
        "SELECT Pprof AS pprof, toInt64(dateDiff('millisecond', Start, End)) AS duration_ms, RuntimeId AS runtime_id FROM otel.profiles \
         WHERE OrgId = {{oc_org:String}} AND {IN_RANGE}{} ORDER BY Start DESC LIMIT ?",
        f.sql
    );
    // Every attachment (Go splits CPU and heap into two files): the cards cover all sample types.
    let blobs = bind_all(st.ch(&org).query(&sql).bind(req.range.from).bind(req.range.to), &f.binds).bind(MAX_MERGED).fetch_all::<Blob>().await?;
    let profiles: Vec<(pprof::Profile, &Blob)> = blobs.iter().filter_map(|b| decode_pprof(&b.pprof).ok().map(|p| (p, b))).collect();
    // Sample types across the profiles, in first-seen order, with their totals.
    let mut types: Vec<SampleType> = Vec::new();
    let mut stats: HashMap<String, TypeStats> = HashMap::new();
    for (p, b) in &profiles {
        for t in sample_types(p) {
            let t = SampleType { r#type: canonical(&t.r#type).to_string(), unit: t.unit };
            if !types.iter().any(|x| x.r#type == t.r#type) {
                types.push(t);
            }
        }
        for (t, total) in totals(p) {
            let s = stats.entry(canonical(&t).to_string()).or_default();
            s.total += total;
            s.profiles += 1;
            s.minutes += b.duration_ms.max(1) as f64 / 60_000.0;
            s.runtimes.insert(b.runtime_id.clone());
        }
    }
    let selected = if req.sample_type.is_empty() {
        types.iter().find(|t| t.r#type == DEFAULT_TYPE).or(types.first()).cloned()
    } else {
        types.iter().find(|t| t.r#type == canonical(&req.sample_type)).cloned()
    };
    let cards: Vec<ProfileCard> = types
        .iter()
        .map(|t| {
            let m = metric(&t.r#type, &t.unit);
            let s = &stats[&t.r#type];
            ProfileCard { value: s.total as f64 * s.scale(m.gauge), metric: m }
        })
        .collect();
    let Some(selected) = selected else {
        if profiles.is_empty() {
            return Ok(Json(FlameGraph {
                sample_types: types,
                sample_type: String::new(),
                unit: String::new(),
                metric: None,
                profiles: 0,
                scale: 1.0,
                cards,
                endpoints: vec![],
                functions: vec![],
                root: FlameNode { name: "all".into(), file: String::new(), value: 0, children: vec![] },
            }));
        }
        return Err(ApiError::BadRequest(format!("no {} samples in these profiles", req.sample_type)));
    };
    let m = metric(&selected.r#type, &selected.unit);
    let mut merge = Merge::default();
    for (p, _) in &profiles {
        add_profile(&mut merge, p, &selected.r#type, req.endpoint.as_deref());
    }
    let scale = stats.get(&selected.r#type).map(|s| s.scale(m.gauge)).unwrap_or(1.0);
    let mut endpoints: Vec<ProfileEndpoint> = merge.endpoints.into_iter().map(|(name, value)| ProfileEndpoint { name, value }).collect();
    endpoints.sort_by(|a, b| b.value.cmp(&a.value).then(a.name.cmp(&b.name)));
    let mut functions: Vec<ProfileFunction> = merge.functions.into_iter().map(|((name, file), value)| ProfileFunction { package: package(&name, &file), name, file, value }).collect();
    functions.sort_by(|a, b| b.value.cmp(&a.value).then(a.name.cmp(&b.name)));
    functions.truncate(MAX_FUNCTIONS);
    // Frames under 0.1 % of the total are dropped.
    let min = (merge.tree.value / 1000).max(1);
    let root = merge.tree.finish(("all".into(), String::new()), min);
    Ok(Json(FlameGraph { sample_types: types, sample_type: selected.r#type, unit: selected.unit, metric: Some(m), profiles: profiles.len() as u64, scale, cards, endpoints, functions, root }))
}

// ---------------------------------------------------------------- insights

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProfileInsightRequest {
    pub range: TimeRange,
    #[serde(default)]
    pub env: String,
    #[serde(default)]
    pub service: String,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProfileInsight {
    /// `<kind>:<service>:<env>`: what review states are stored under.
    pub key: String,
    pub title: String,
    /// LOW, MEDIUM or HIGH.
    pub severity: String,
    /// CPU, MEMORY, LOCK, …
    pub category: String,
    pub service: String,
    pub env: String,
    pub language: String,
    pub description: String,
    /// for_review, reviewed, resolved or ignored.
    pub status: String,
    /// When the pattern peaked (epoch ms).
    pub detected_at: i64,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProfileInsightList {
    pub items: Vec<ProfileInsight>,
}

/// A CPU burst: the busiest minute at least this many times the median minute…
const BURST_RATIO: f64 = 2.0;
/// …and at least this many cores above it (so idle services don't qualify).
const BURST_MIN_CORES: f64 = 0.05;
/// Minutes of data needed to tell a burst from the usual level.
const BURST_MIN_MINUTES: usize = 10;

/// One minute of a service's CPU: (service, env, language, epoch ms, CPU nanoseconds).
type CpuMinute = (String, String, String, i64, f64);

/// CPU bursts per service and env: minutes using far more CPU than usual.
fn cpu_bursts(rows: &[CpuMinute]) -> Vec<(String, String, String, i64, f64, f64)> {
    // (service, env) → (language, cores per minute)
    type Minutes<'a> = (&'a str, Vec<(i64, f64)>);
    let mut by: HashMap<(&str, &str), Minutes> = HashMap::new();
    for (service, env, language, t, ns) in rows {
        by.entry((service, env)).or_insert((language, vec![])).1.push((*t, ns / 60e9));
    }
    let mut out = Vec::new();
    for ((service, env), (language, minutes)) in by {
        if minutes.len() < BURST_MIN_MINUTES {
            continue;
        }
        let mut sorted: Vec<f64> = minutes.iter().map(|(_, c)| *c).collect();
        sorted.sort_by(f64::total_cmp);
        let median = sorted[sorted.len() / 2];
        let Some(&(peak_t, peak)) = minutes.iter().max_by(|a, b| a.1.total_cmp(&b.1)) else {
            continue;
        };
        if peak >= median * BURST_RATIO && peak - median >= BURST_MIN_CORES {
            out.push((service.to_string(), env.to_string(), language.to_string(), peak_t, peak, median));
        }
    }
    out.sort_by(|a, b| (&a.0, &a.1).cmp(&(&b.0, &b.1)));
    out
}

/// Patterns detected in the profiles of the range (Profiles → Insights), with their review state.
#[utoipa::path(post, path = "/api/v1/profiles/insights", operation_id = "profile_insights", request_body = ProfileInsightRequest, responses((status = 200, body = ProfileInsightList)), tag = "profiles")]
pub async fn insights(State(st): State<AppState>, OrgId(org): OrgId, Json(req): Json<ProfileInsightRequest>) -> ApiResult<ProfileInsightList> {
    req.range.validate()?;
    let f = filter(&with_fields("", &req.service, &req.env), None);
    let sql = format!(
        "SELECT toString(Service) AS service, toString(Env) AS env, toString(any(Language)) AS language, toInt64(toUnixTimestamp(toStartOfMinute(Start))) * 1000 AS t, \
         toFloat64(sum(SampleTotals['cpu-time']) + sum(SampleTotals['cpu'])) AS ns \
         FROM otel.profiles WHERE OrgId = {{oc_org:String}} AND {IN_RANGE}{} AND (mapContains(SampleTotals, 'cpu-time') OR mapContains(SampleTotals, 'cpu')) GROUP BY service, env, t ORDER BY t",
        f.sql
    );
    let rows = bind_all(st.ch(&org).query(&sql).bind(req.range.from).bind(req.range.to), &f.binds).fetch_all::<CpuMinute>();
    let states = sqlx::query_as::<_, (String, String)>("SELECT insight_key, status FROM profile_insight_states WHERE org_id = $1").bind(&org).fetch_all(&st.postgres);
    let (rows, states) = tokio::try_join!(async { rows.await.map_err(ApiError::from) }, async { states.await.map_err(ApiError::from) })?;
    let states: HashMap<String, String> = states.into_iter().collect();
    let items = cpu_bursts(&rows)
        .into_iter()
        .map(|(service, env, language, t, peak, median)| {
            let key = format!("cpu-burst:{service}:{env}");
            ProfileInsight {
                status: states.get(&key).cloned().unwrap_or_else(|| "for_review".into()),
                key,
                title: "Burst of High CPU Usage".into(),
                severity: "MEDIUM".into(),
                category: "CPU".into(),
                description: format!(
                    "CPU usage rose to {peak:.2} cores against a usual {median:.2} cores. Short bursts like this suggest the service could use more CPU, or that a hot code path is worth optimizing."
                ),
                service,
                env,
                language,
                detected_at: t,
            }
        })
        .collect();
    Ok(Json(ProfileInsightList { items }))
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProfileInsightStateUpdate {
    pub key: String,
    /// for_review, reviewed, resolved or ignored.
    pub status: String,
}

/// Marks an insight reviewed, resolved, ignored or back for review.
#[utoipa::path(post, path = "/api/v1/profiles/insights/state", operation_id = "set_profile_insight_state", request_body = ProfileInsightStateUpdate, responses((status = 204)), tag = "profiles")]
pub async fn set_insight_state(State(st): State<AppState>, Writer(ctx): Writer, Json(u): Json<ProfileInsightStateUpdate>) -> Result<StatusCode, ApiError> {
    if !["for_review", "reviewed", "resolved", "ignored"].contains(&u.status.as_str()) {
        return Err(ApiError::BadRequest(format!("unknown status {}", u.status)));
    }
    if u.key.is_empty() || u.key.len() > 512 {
        return Err(ApiError::BadRequest("invalid insight key".into()));
    }
    sqlx::query(
        "INSERT INTO profile_insight_states (org_id, insight_key, status, updated_by, updated_at) VALUES ($1, $2, $3, $4::uuid, now()) \
         ON CONFLICT (org_id, insight_key) DO UPDATE SET status = EXCLUDED.status, updated_by = EXCLUDED.updated_by, updated_at = now()",
    )
    .bind(&ctx.org_id)
    .bind(&u.key)
    .bind(&u.status)
    .bind(&ctx.user_id)
    .execute(&st.postgres)
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;
    use prost::Message;

    fn profile() -> pprof::Profile {
        let strings = ["", "cpu-time", "nanoseconds", "wall-time", "main", "handle_request", "query_db", "render", "trace endpoint", "GET /cart"];
        pprof::Profile {
            sample_type: vec![pprof::ValueType { r#type: 1, unit: 2 }, pprof::ValueType { r#type: 3, unit: 2 }],
            sample: vec![
                // main → handle_request → query_db (leaf first)
                pprof::Sample { location_id: vec![3, 2, 1], value: vec![70, 100], label: vec![pprof::Label { key: 8, str: 9, num: 0 }] },
                pprof::Sample { location_id: vec![4, 2, 1], value: vec![30, 40], label: vec![] },
            ],
            location: (1..=4).map(|i| pprof::Location { id: i, line: vec![pprof::Line { function_id: i, line: 1 }] }).collect(),
            function: (1..=4).map(|i| pprof::Function { id: i, name: (i + 3) as i64, filename: 0 }).collect(),
            string_table: strings.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn flame_graph_from_pprof() {
        let p = profile();
        let gz = {
            use std::io::Write;
            let mut e = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
            e.write_all(&p.encode_to_vec()).unwrap();
            e.finish().unwrap()
        };
        let p = decode_pprof(&gz).unwrap();
        assert_eq!(sample_types(&p)[1], SampleType { r#type: "wall-time".into(), unit: "nanoseconds".into() });
        let mut m = Merge::default();
        add_profile(&mut m, &p, "cpu-time", None);
        add_profile(&mut m, &p, "cpu-time", None);
        assert_eq!(m.functions[&("query_db".to_string(), String::new())], 140);
        assert_eq!((m.endpoints["GET /cart"], m.endpoints[""]), (140, 60));
        let mut only = Merge::default();
        add_profile(&mut only, &p, "cpu-time", Some("GET /cart"));
        assert_eq!(only.tree.value, 70);
        assert_eq!(totals(&p), vec![("cpu-time".to_string(), 100), ("wall-time".to_string(), 140)]);
        let root = m.tree.finish(("all".into(), String::new()), 1);
        assert_eq!(root.value, 200);
        let main = &root.children[0];
        assert_eq!((main.name.as_str(), main.value), ("main", 200));
        let handler = &main.children[0];
        assert_eq!(handler.name, "handle_request");
        assert_eq!(handler.children.iter().map(|c| (c.name.as_str(), c.value)).collect::<Vec<_>>(), vec![("query_db", 140), ("render", 60)]);
    }

    #[test]
    fn packages_and_filters() {
        assert_eq!(package("read", "/usr/local/lib/python3.12/site-packages/pandas/io/parsers.py"), "pandas");
        assert_eq!(package("loads", "/usr/local/lib/python3.12/json/__init__.py"), "stdlib");
        assert_eq!(package("get_cart", "/app/cart.py"), "");
        assert_eq!(package("com.example.shop.Cart.add", "Cart.java"), "com.example.shop");
        assert_eq!(package("github.com/example-org/shop/cart.(*Cart).Add", "cart.go"), "github.com/example-org/shop/cart");
        assert_eq!(metric("heap-space", "bytes").label, "Heap Live Size");
        assert!(metric("inuse_space", "").gauge);
        assert_eq!(metric("cpu", "nanoseconds").r#type, "cpu-time");
        assert_eq!(spellings("heap-space"), ["heap-space", "inuse-space", "inuse_space", "heap-live-size"]);
        let f = filter("service:(checkout OR billing) -env:dev team:web free-text", None);
        assert_eq!(f.sql, " AND toString(Env) NOT IN (?) AND toString(Service) IN (?, ?) AND Tags[?] IN (?)");
        assert_eq!(f.binds, ["dev", "checkout", "billing", "team", "web"]);
        assert_eq!(filter("service:checkout env:prod", Some("service")).binds, ["prod"]);
    }

    #[test]
    fn bursts() {
        let mut rows: Vec<CpuMinute> = (0..20).map(|i| ("checkout".into(), "prod".into(), "python".into(), i * 60_000, 6e9)).collect();
        rows[7].4 = 30e9; // 0.5 cores against 0.1
        let found = cpu_bursts(&rows);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].3, 7 * 60_000);
        rows[7].4 = 9e9;
        assert!(cpu_bursts(&rows).is_empty());
    }

    #[test]
    fn rfc3339_nanos() {
        assert_eq!(nanos("1970-01-01T00:00:01Z"), Some(1_000_000_000));
        assert_eq!(nanos("2026-10-26T09:31:20.5Z"), Some(1_793_007_080_500_000_000));
        assert_eq!(tag_pairs("service:dd-checkout,env:dev"), vec![("service".into(), "dd-checkout".into()), ("env".into(), "dev".into())]);
    }
}
