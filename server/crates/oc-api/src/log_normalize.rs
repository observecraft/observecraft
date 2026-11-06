//! Log normalization: an OTLP log record or a JSON log, as the Kafka ingester decodes it, into the indexed
//! row (`oc.logs`), exactly as `deploy/vector/normalize.vrl` does in Vector.
//!
//! Mirrors Datadog's preprocessing:
//! - JSON bodies and lines are parsed and their fields become attributes;
//! - reserved attributes are read from Datadog's default names (message, status, service, host, date, trace and
//!   span ids, `ddsource`, `ddtags`), and plain-text lines carry their level (klog, logfmt, dashed, a leading
//!   word; the Datadog Agent's stream status only when the line has none);
//! - Kubernetes, ECS and cloud resource attributes become Datadog's tags, and service, env and version follow
//!   unified service tagging.
//!
//! The values follow VRL's: `to_string` of a float is its shortest form (`100`, `0.000001`), objects and arrays
//! are encoded as JSON with sorted keys, `flatten` joins nested keys with `.` and drops empty objects.
//! `log_normalize/cases.ndjson` holds the rows Vector made from the same inputs (`scripts/log-normalize-golden.mjs`).

use std::collections::{BTreeMap, HashMap};
use std::sync::LazyLock;

use chrono::{DateTime, SecondsFormat, TimeZone, Utc};
use regex::Regex;
use serde::Serialize;
use serde_json::{Map, Value};

/// A log as the Kafka ingester decodes it, shaped as a Vector event: `source_type` (`opentelemetry`, `kafka`),
/// OTLP's `message`, `attributes`, `resources`, `severity_text`, `severity_number`, `trace_id`, `span_id` and
/// `scope`, or a JSON log's own fields, with `_oc_kafka_headers` and `_oc_ingest`. `timestamp` is the event time
/// a source sets (OTLP's, the Kafka record's); a JSON log's own `timestamp` field stays in `fields`.
#[derive(Debug, Clone, Default)]
pub struct LogEvent {
    pub fields: Map<String, Value>,
    pub timestamp: Option<DateTime<Utc>>,
}

/// An indexed row (`oc.logs` columns, and `ts_ms` for live tail).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Row {
    pub org_id: String,
    pub id: String,
    #[serde(serialize_with = "rfc3339")]
    pub timestamp: DateTime<Utc>,
    pub ts_ms: i64,
    pub status: String,
    pub service: String,
    pub host: String,
    pub source: String,
    pub message: String,
    #[serde(serialize_with = "sorted")]
    pub tags: HashMap<String, String>,
    #[serde(serialize_with = "sorted")]
    pub attr: HashMap<String, String>,
    /// The attributes as typed JSON (Vector's `attributes`), for the indexer's pipelines. Not stored: `attr`
    /// holds them, and readers nest it again (`pipelines::log::attributes_json`).
    #[serde(skip)]
    pub attrs: Map<String, Value>,
    /// Set by the indexer once an index keeps the log.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub index_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retention_days: Option<u16>,
    pub trace_id: String,
    pub span_id: String,
    pub severity_number: i64,
    pub scope: String,
}

/// A map in key order: ClickHouse stores a `Map` as arrays in the order given, and the same order in every row
/// compresses and merges far better.
fn sorted<S: serde::Serializer>(m: &HashMap<String, String>, s: S) -> Result<S::Ok, S::Error> {
    let mut entries: Vec<(&String, &String)> = m.iter().collect();
    entries.sort_unstable_by(|a, b| a.0.cmp(b.0));
    s.collect_map(entries)
}

fn rfc3339<S: serde::Serializer>(t: &DateTime<Utc>, s: S) -> Result<S::Ok, S::Error> {
    s.serialize_str(&t.to_rfc3339_opts(SecondsFormat::AutoSi, true))
}

// ---------------------------------------------------------------- VRL's value functions

/// VRL's `to_string`: None for objects and arrays (VRL fails, and the script then encodes them).
fn to_string(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.clone()),
        Value::Null => Some(String::new()),
        Value::Bool(b) => Some(b.to_string()),
        Value::Number(n) => Some(match (n.as_i64(), n.as_u64()) {
            (Some(i), _) => i.to_string(),
            (_, Some(u)) => u.to_string(),
            _ => n.as_f64().unwrap_or_default().to_string(),
        }),
        Value::Object(_) | Value::Array(_) => None,
    }
}

/// `to_string(v) ?? encode_json(v)`.
fn text(v: &Value) -> String {
    to_string(v).unwrap_or_else(|| encode_json(v))
}

/// `to_string(get(map, [key]) ?? null) ?? ""`.
fn text_of(map: &Map<String, Value>, key: &str) -> String {
    map.get(key).and_then(to_string).unwrap_or_default()
}

/// VRL's `encode_json`: object keys sorted (serde_json's `Map` is ordered by key).
fn encode_json(v: &Value) -> String {
    serde_json::to_string(v).unwrap_or_default()
}

/// VRL's `flatten`: nested objects' keys joined with `.`; empty objects disappear, arrays stay.
fn flatten(m: &Map<String, Value>) -> Map<String, Value> {
    fn walk(prefix: &str, m: &Map<String, Value>, out: &mut Map<String, Value>) {
        for (k, v) in m {
            let key = if prefix.is_empty() { k.clone() } else { format!("{prefix}.{k}") };
            match v {
                Value::Object(inner) => walk(&key, inner, out),
                other => {
                    out.insert(key, other.clone());
                }
            }
        }
    }
    let mut out = Map::new();
    walk("", m, &mut out);
    out
}

/// `get(map, [key]) ?? null`, with null as absent.
fn present<'a>(m: &'a Map<String, Value>, key: &str) -> Option<&'a Value> {
    m.get(key).filter(|v| !v.is_null())
}

/// `merge(a, b)`: b's keys win.
fn merge(mut a: Map<String, Value>, b: Map<String, Value>) -> Map<String, Value> {
    for (k, v) in b {
        a.insert(k, v);
    }
    a
}

/// `parse_json(s)` when it is an object.
fn json_object(s: &str) -> Option<Map<String, Value>> {
    match serde_json::from_str::<Value>(s).ok()? {
        Value::Object(m) => Some(m),
        _ => None,
    }
}

/// `parse_timestamp(s, "%+")`.
fn parse_timestamp(s: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_str(s, "%+").or_else(|_| DateTime::parse_from_rfc3339(s)).ok().map(|t| t.with_timezone(&Utc))
}

/// VRL's `to_int`: integers, floats truncated, numeric strings, booleans; null is 0.
fn to_int(v: &Value) -> Option<i64> {
    match v {
        Value::Number(n) => n.as_i64().or_else(|| n.as_f64().map(|f| f as i64)),
        Value::String(s) => s.trim().parse().ok(),
        Value::Bool(b) => Some(*b as i64),
        Value::Null => Some(0),
        _ => None,
    }
}

/// Seconds, milliseconds or nanoseconds since the epoch, as the script tells them apart.
fn from_unix(n: i64) -> Option<DateTime<Utc>> {
    if n > 100_000_000_000_000_000 {
        Some(Utc.timestamp_nanos(n))
    } else if n > 100_000_000_000 {
        Utc.timestamp_millis_opt(n).single()
    } else {
        Utc.timestamp_opt(n, 0).single()
    }
}

/// UUIDv7 of `at`: its milliseconds, then random bits, with the version and variant set.
fn uuid_v7(at: DateTime<Utc>) -> String {
    uuid_v7_seeded(at, rand::random::<u128>())
}

/// UUIDv7 of `at` whose other bits come from `seed`: the same seed, the same id.
pub fn uuid_v7_seeded(at: DateTime<Utc>, seed: u128) -> String {
    let mut bits = ((at.timestamp_millis() as u128 & 0xffff_ffff_ffff) << 80) | (seed & 0xffff_ffff_ffff_ffff_ffff);
    bits = (bits & !(0xf << 76)) | (7 << 76);
    bits = (bits & !(3 << 62)) | (2 << 62);
    let hex = format!("{bits:032x}");
    format!("{}-{}-{}-{}-{}", &hex[..8], &hex[8..12], &hex[12..16], &hex[16..20], &hex[20..])
}

fn re(pattern: &str) -> Regex {
    Regex::new(pattern).expect("valid pattern")
}

static KLOG: LazyLock<Regex> = LazyLock::new(|| re(r"^(?P<l>[IWEF])\d{4} \d{2}:\d{2}:\d{2}"));
static FIELD: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)(?:^|\s)(?:level|lvl|severity)=(?P<l>[a-z]+)"));
static DASHED: LazyLock<Regex> = LazyLock::new(|| re(r" [-|] +(?P<l>TRACE|DEBUG|INFO|WARNING|WARN|ERROR|CRITICAL|FATAL) +[-|] "));
static WORD: LazyLock<Regex> =
    LazyLock::new(|| re(r"(?i)^\s*(?:\S+\s+){0,2}?[\[<]?(?P<l>trace|debug|info|notice|warn|warning|error|err|fatal|critical|panic)[\]>]?(?::|\s|$)"));
static TRACE_ID: LazyLock<Regex> = LazyLock::new(|| re(r"^[0-9a-f]{32}$"));
static SPAN_ID: LazyLock<Regex> = LazyLock::new(|| re(r"^[0-9a-f]{16}$"));
static V7: LazyLock<Regex> = LazyLock::new(|| re(r"^[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$"));

/// Kubernetes, ECS and cloud resource attributes → Datadog's tag names.
const RESOURCE_TAGS: &[(&str, &str)] = &[
    ("k8s.namespace.name", "kube_namespace"),
    ("k8s.pod.name", "pod_name"),
    ("k8s.pod.uid", "pod_uid"),
    ("k8s.container.name", "kube_container_name"),
    ("k8s.deployment.name", "kube_deployment"),
    ("k8s.replicaset.name", "kube_replica_set"),
    ("k8s.daemonset.name", "kube_daemon_set"),
    ("k8s.statefulset.name", "kube_stateful_set"),
    ("k8s.job.name", "kube_job"),
    ("k8s.cronjob.name", "kube_cronjob"),
    ("k8s.node.name", "kube_node"),
    ("k8s.cluster.name", "kube_cluster_name"),
    ("container.id", "container_id"),
    ("container.name", "container_name"),
    ("container.image.name", "image_name"),
    ("container.image.tag", "image_tag"),
    ("k8s.pod.labels.app.kubernetes.io/name", "kube_app_name"),
    ("k8s.pod.labels.app.kubernetes.io/instance", "kube_app_instance"),
    ("k8s.pod.labels.app.kubernetes.io/version", "kube_app_version"),
    ("k8s.pod.labels.app.kubernetes.io/component", "kube_app_component"),
    ("k8s.pod.labels.app.kubernetes.io/part-of", "kube_app_part_of"),
    ("k8s.pod.labels.app.kubernetes.io/managed-by", "kube_app_managed_by"),
    ("aws.ecs.task.family", "task_family"),
    ("aws.ecs.task.revision", "task_version"),
    ("aws.ecs.task.arn", "task_arn"),
    ("aws.ecs.launchtype", "task_launch_type"),
    ("aws.ecs.container.arn", "container_arn"),
    ("cloud.region", "region"),
    ("cloud.availability_zone", "availability-zone"),
    ("cloud.account.id", "aws_account"),
    ("host.name", "host"),
];

/// Resource attributes naming the service, in order, when the log names none.
const RESOURCE_SERVICES: &[&str] = &[
    "k8s.pod.labels.tags.datadoghq.com/service",
    "service.name",
    "k8s.pod.labels.app.kubernetes.io/name",
    "k8s.deployment.name",
    "k8s.statefulset.name",
    "k8s.daemonset.name",
    "k8s.cronjob.name",
    "k8s.job.name",
    "k8s.container.name",
    "aws.ecs.task.family",
];

/// The first of `keys` with a value in `flat` (recorded in `used`).
fn first<'a>(flat: &'a Map<String, Value>, keys: &[&'static str], used: &mut Vec<&'static str>, accept: impl Fn(&Value) -> bool) -> Option<&'a Value> {
    keys.iter().find_map(|k| {
        let v = present(flat, k).filter(|v| accept(v))?;
        used.push(k);
        Some(v)
    })
}

fn nonempty_string(v: &Value) -> bool {
    v.as_str().is_some_and(|s| !s.is_empty())
}

/// The indexed row, or None for a log without an organization.
pub fn normalize(event: LogEvent, now: DateTime<Utc>) -> Option<Row> {
    let LogEvent { mut fields, timestamp } = event;
    let kind = fields.get("source_type").and_then(to_string).unwrap_or_default();
    let otlp = kind == "opentelemetry";
    let mut res = match fields.remove("resources") { Some(Value::Object(m)) => m, _ => Map::new() };
    let mut attrs = match fields.remove("attributes") { Some(Value::Object(m)) => m, _ => Map::new() };
    let root_tags = match fields.remove("tags") { Some(Value::Object(m)) => m, _ => Map::new() };
    // A source sets the event time; a JSON log's own `timestamp` field otherwise.
    let event_time: Option<Value> = match timestamp {
        Some(_) => None,
        None => fields.get("timestamp").cloned(),
    };

    // Reserved metadata the intake assigned before Kafka.
    let (ingest_id, received_at) = {
        let identity: Map<String, Value> = if kind == "kafka" || kind == "http_server" {
            fields.get("_oc_ingest").and_then(Value::as_object).cloned().unwrap_or_default()
        } else if otlp {
            let mut m = Map::new();
            m.insert("id".into(), attrs.get("oc.ingest_id").cloned().unwrap_or(Value::Null));
            m.insert("received_at".into(), attrs.get("oc.ingest_received_at").cloned().unwrap_or(Value::Null));
            m
        } else {
            Map::new()
        };
        (text_of(&identity, "id"), parse_timestamp(&text_of(&identity, "received_at")))
    };
    attrs.remove("oc.ingest_id");
    attrs.remove("oc.ingest_received_at");
    fields.remove("_oc_ingest");

    // Organization: the intake's record header for Kafka JSON logs, OTLP's resource, a trusted agent's field.
    if kind == "kafka" {
        let headers = fields.get("_oc_kafka_headers").and_then(Value::as_object).cloned().unwrap_or_default();
        fields.insert("x-oc-org".into(), Value::String(text_of(&headers, "x-oc-org")));
        for k in ["_oc_kafka_headers", "_oc_kafka_key", "_oc_kafka_topic", "_oc_kafka_partition", "_oc_kafka_offset"] {
            fields.remove(k);
        }
    }
    let mut org = text_of(&fields, "x-oc-org");
    if org.is_empty() {
        org = text_of(&res, "oc.org_id");
    }
    if org.is_empty() {
        org = text_of(&fields, "org_id");
    }
    if org.is_empty() {
        return None;
    }
    res.remove("oc.org_id");
    res.remove("oc.retention_days");

    // Attributes: a JSON log's own fields, then OTLP's attributes and object or JSON body.
    if !otlp {
        const ENVELOPE: &[&str] = &["org_id", "x-oc-org", "tags", "attributes", "resources", "source_type", "timestamp", "path"];
        let extra: Map<String, Value> = fields.iter().filter(|(k, _)| !ENVELOPE.contains(&k.as_str())).map(|(k, v)| (k.clone(), v.clone())).collect();
        attrs = merge(extra, attrs);
    }
    let mut body = Value::Null;
    if otlp {
        body = fields.get("message").cloned().unwrap_or(Value::Null);
        match &body {
            Value::Object(m) => {
                attrs = merge(attrs, m.clone());
                body = Value::Null;
            }
            Value::String(s) => {
                let b = s.trim();
                if b.starts_with('{') {
                    if let Some(parsed) = json_object(b) {
                        attrs = merge(attrs, parsed);
                        body = Value::Null;
                    }
                }
            }
            _ => {}
        }
    }

    let mut used: Vec<&'static str> = vec![];
    let mut flat = flatten(&attrs);
    const MESSAGE: &[&str] = &["message", "msg", "log"];
    let mut msg = first(&flat, MESSAGE, &mut used, |_| true).cloned();
    // A message that is itself JSON (container stdout through Fluent Bit or the Collector).
    if let Some(Value::String(s)) = &msg {
        let m = s.trim().to_string();
        if m.starts_with('{') {
            if let Some(parsed) = json_object(&m) {
                attrs.retain(|k, _| !used.contains(&k.as_str()));
                attrs = merge(attrs, parsed);
                used.clear();
                flat = flatten(&attrs);
                msg = first(&flat, MESSAGE, &mut used, |_| true).cloned();
                if msg.is_none() {
                    msg = Some(Value::String(m));
                }
            }
        }
    }
    let msg = match msg.unwrap_or(body) {
        Value::String(s) => s,
        Value::Null => String::new(),
        other => encode_json(&other),
    };

    // Level.
    let mut lvl: Option<Value> = None;
    if otlp {
        lvl = fields.get("severity_text").cloned().filter(|v| !v.is_null() && v.as_str() != Some(""));
    }
    let agent = !otlp && fields.contains_key("ddsource");
    for k in ["status", "severity", "level", "syslog.severity"] {
        if lvl.is_none() && !(agent && k == "status") {
            if let Some(v) = present(&flat, k) {
                lvl = Some(v.clone());
                used.push(k);
            }
        }
    }
    if lvl.is_none() && !msg.is_empty() {
        let mut end = msg.len().min(160);
        while !msg.is_char_boundary(end) {
            end -= 1;
        }
        let head = &msg[..end];
        let caught = KLOG.captures(head).or_else(|| FIELD.captures(head)).or_else(|| DASHED.captures(head)).or_else(|| WORD.captures(head));
        lvl = caught.and_then(|c| c.name("l")).map(|l| Value::String(l.as_str().to_string()));
    }
    if agent {
        if let Some(stream) = present(&flat, "status") {
            used.push("status");
            if lvl.is_none() {
                lvl = Some(stream.clone());
            }
        }
    }
    let mut sev = fields.get("severity_number").map_or(Some(0), to_int).unwrap_or(0);
    if !(0..=24).contains(&sev) {
        sev = 0;
    }
    if lvl.is_none() && sev > 0 {
        lvl = Some(Value::String(if sev >= 17 { "error" } else if sev >= 13 { "warn" } else if sev >= 9 { "info" } else { "debug" }.into()));
    }
    let lvl = lvl.map_or(Some(String::new()), |l| to_string(&l)).unwrap_or_else(|| "info".into()).to_lowercase();
    let status = if lvl.starts_with("err") || ["fatal", "critical", "crit", "alert", "emerg", "emergency", "panic", "e", "f"].contains(&lvl.as_str()) {
        "error"
    } else if lvl.starts_with("warn") || lvl == "w" {
        "warn"
    } else if ["debug", "trace", "d", "verbose"].contains(&lvl.as_str()) {
        "debug"
    } else {
        "info"
    };

    // Event time.
    let mut ts: Option<DateTime<Utc>> = None;
    for k in ["@timestamp", "timestamp", "_timestamp", "Timestamp", "eventTime", "date", "published_date", "syslog.timestamp"] {
        if ts.is_some() {
            break;
        }
        ts = match present(&flat, k) {
            Some(Value::String(s)) => parse_timestamp(s),
            Some(v @ Value::Number(_)) => to_int(v).and_then(from_unix),
            _ => None,
        };
        if ts.is_some() {
            used.push(k);
        }
    }
    if ts.is_none() {
        if let Some(Value::String(s)) = &event_time {
            ts = parse_timestamp(s);
        }
    }
    if ts.is_none() && otlp {
        ts = timestamp;
    }
    let ts = ts.or(received_at).or(timestamp).unwrap_or(now);

    // Trace and span ids.
    let mut trace_id = fields.get("trace_id").and_then(to_string).unwrap_or_default();
    let mut span_id = fields.get("span_id").and_then(to_string).unwrap_or_default();
    for k in ["dd.trace_id", "contextMap.dd.trace_id", "named_tags.dd.trace_id", "trace_id"] {
        if trace_id.is_empty() {
            trace_id = text_of(&flat, k);
        }
    }
    for k in ["dd.span_id", "contextMap.dd.span_id", "named_tags.dd.span_id", "span_id"] {
        if span_id.is_empty() {
            span_id = text_of(&flat, k);
        }
    }
    let mut trace_id = trace_id.to_lowercase();
    let mut span_id = span_id.to_lowercase();
    if !TRACE_ID.is_match(&trace_id) || trace_id == "00000000000000000000000000000000" {
        trace_id.clear();
    }
    if !SPAN_ID.is_match(&span_id) {
        span_id.clear();
    }

    // Datadog tags from resources.
    let r: Map<String, Value> = res.iter().map(|(k, v)| (k.clone(), Value::String(text(v)))).collect();
    let mut dd: BTreeMap<String, String> = BTreeMap::new();
    if !r.is_empty() {
        for (from, to) in RESOURCE_TAGS {
            let v = text_of(&r, from);
            if !v.is_empty() {
                dd.insert((*to).into(), v);
            }
        }
        let cluster_arn = text_of(&r, "aws.ecs.cluster.arn");
        if !cluster_arn.is_empty() {
            dd.insert("ecs_cluster".into(), after_last_slash(&cluster_arn));
        }
    }
    // FireLens (Fluent Bit) puts ECS metadata on the record itself.
    let fl_cluster = text_of(&attrs, "ecs_cluster");
    if !fl_cluster.is_empty() {
        dd.insert("ecs_cluster".into(), after_last_slash(&fl_cluster));
    }
    let fl_task = text_of(&attrs, "ecs_task_arn");
    if !fl_task.is_empty() {
        dd.insert("task_arn".into(), fl_task);
    }
    let fl_def = text_of(&attrs, "ecs_task_definition");
    if !fl_def.is_empty() {
        let parts: Vec<&str> = fl_def.split(':').collect();
        dd.insert("task_family".into(), parts[0].into());
        if parts.len() > 1 {
            dd.insert("task_version".into(), parts[1].into());
        }
    }
    let fl_container = text_of(&attrs, "container_name");
    if !fl_container.is_empty() {
        dd.insert("container_name".into(), fl_container.strip_prefix('/').unwrap_or(&fl_container).to_string());
    }
    if let Some(image) = dd.get("image_name").cloned() {
        let short = after_last_slash(&image);
        let short = short.find([':', '@']).map_or(short.as_str(), |i| &short[..i]).to_string();
        dd.insert("short_image".into(), short);
    }
    if let Some(container) = dd.get("kube_container_name").cloned() {
        dd.insert("display_container_name".into(), container);
    }

    // Unified service tagging.
    if !r.is_empty() {
        let mut env = text_of(&r, "deployment.environment.name");
        if env.is_empty() {
            env = text_of(&r, "deployment.environment");
        }
        let label_env = text_of(&r, "k8s.pod.labels.tags.datadoghq.com/env");
        if !label_env.is_empty() {
            env = label_env;
        }
        if !env.is_empty() && !root_tags.contains_key("env") {
            dd.insert("env".into(), env);
        }
        let mut version = text_of(&r, "service.version");
        let label_version = text_of(&r, "k8s.pod.labels.tags.datadoghq.com/version");
        if !label_version.is_empty() {
            version = label_version;
        }
        if !version.is_empty() {
            dd.insert("version".into(), version);
        }
    }

    let mut service = first(&flat, &["service", "syslog.appname", "dd.service"], &mut used, nonempty_string).and_then(Value::as_str).map(str::to_string);
    if service.is_none() && !r.is_empty() {
        service = RESOURCE_SERVICES.iter().map(|k| text_of(&r, k)).find(|v| !v.is_empty() && !v.starts_with("unknown_service"));
    }
    let service = service
        .or_else(|| dd.get("task_family").cloned())
        .or_else(|| dd.get("container_name").cloned())
        .or_else(|| dd.get("short_image").cloned())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown".into());

    let host = first(&flat, &["host", "hostname", "syslog.hostname"], &mut used, nonempty_string)
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| r.get("host.name").and_then(to_string))
        .or_else(|| r.get("k8s.node.name").and_then(to_string))
        .unwrap_or_default();

    let mut source = text_of(&flat, "ddsource");
    if !source.is_empty() {
        used.push("ddsource");
    }
    if source.is_empty() {
        source = dd.get("short_image").cloned().unwrap_or_default();
    }
    if source.is_empty() {
        source = service.clone();
    }

    // ddtags: "env:prod,team:payments".
    let ddtags = text_of(&flat, "ddtags");
    if !ddtags.is_empty() {
        used.push("ddtags");
        for kv in ddtags.split(',') {
            if let Some((k, v)) = kv.trim().split_once(':') {
                if !k.is_empty() {
                    dd.insert(k.into(), v.into());
                }
            }
        }
    }

    let mut tags: HashMap<String, String> = root_tags.iter().map(|(k, v)| (k.clone(), text(v))).collect();
    tags.extend(r.iter().map(|(k, v)| (k.clone(), text(v))));
    tags.extend(dd);

    attrs.retain(|k, _| !used.contains(&k.as_str()));
    // A plain-text stack trace (lines the collector joined) gets Datadog's error attributes, unless set (log_stack).
    let mut attr: HashMap<String, String> = flatten(&attrs).iter().map(|(k, v)| (k.clone(), text(v))).collect();
    if !attr.contains_key("error.stack") && !attr.contains_key("exception.stacktrace") {
        if let Some(st) = crate::log_stack::extract(&msg) {
            let mut error = match attrs.remove("error") {
                Some(Value::Object(m)) => m,
                Some(other) => Map::from_iter([("message".to_string(), other)]),
                None => Map::new(),
            };
            for (k, v) in [("stack", st.stack), ("kind", st.kind), ("message", st.message)] {
                if !v.is_empty() && !error.contains_key(k) {
                    attr.insert(format!("error.{k}"), v.clone());
                    error.insert(k.into(), Value::String(v));
                }
            }
            attrs.insert("error".into(), Value::Object(error));
        }
    }
    let scope = fields.get("scope").and_then(Value::as_object).and_then(|s| s.get("name")).and_then(to_string).unwrap_or_default();
    let id = if V7.is_match(&ingest_id) { ingest_id } else { uuid_v7(ts) };

    Some(Row {
        org_id: org,
        id,
        timestamp: ts,
        ts_ms: ts.timestamp_millis(),
        status: status.into(),
        service,
        host,
        source,
        message: msg,
        tags,
        attr,
        attrs,
        index_name: None,
        retention_days: None,
        trace_id,
        span_id,
        severity_number: sev,
        scope,
    })
}

/// A fresh UUIDv7, for other modules' tests.
#[cfg(test)]
pub fn tests_uuid_v7() -> String {
    uuid_v7(Utc::now())
}

/// `replace(s, r'^.*/', "")`.
fn after_last_slash(s: &str) -> String {
    s.rsplit('/').next().unwrap_or(s).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v7(id: &str) -> bool {
        regex::Regex::new(r"^[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$").unwrap().is_match(id)
    }

    /// Every case of `log_normalize/cases.ndjson`: the row Vector made from it (`scripts/log-normalize-golden.mjs`).
    #[test]
    fn rows_are_vectors_rows() {
        let now = chrono::DateTime::parse_from_rfc3339("2026-11-05T00:00:00Z").unwrap().with_timezone(&chrono::Utc);
        let mut failures = vec![];
        for line in include_str!("log_normalize/cases.ndjson").lines() {
            let case: Value = serde_json::from_str(line).unwrap();
            let name = case["name"].as_str().unwrap();
            let mut fields: Map<String, Value> = case["event"].as_object().unwrap().clone();
            let timestamp = fields.remove("_t").and_then(|t| chrono::DateTime::parse_from_rfc3339(t.as_str()?).ok()).map(|t| t.with_timezone(&chrono::Utc));
            // The typed attributes aren't stored, but they are still Vector's `attributes`.
            let row = normalize(LogEvent { fields, timestamp }, now).map(|r| {
                let mut v = serde_json::to_value(&r).unwrap();
                v["attributes"] = Value::String(encode_json(&Value::Object(r.attrs)));
                v
            });
            let mut expected = case["expected"].clone();
            let (Some(mut got), false) = (row.clone(), expected.is_null()) else {
                if row.is_some() != !expected.is_null() {
                    failures.push(format!("{name}: row {row:?}, Vector's {expected}"));
                }
                continue;
            };
            // Generated ids are random: both UUIDv7. An accepted log's id is kept as it came.
            let (want, have) = (expected["id"].as_str().unwrap_or_default().to_string(), got["id"].as_str().unwrap_or_default().to_string());
            if want != have && v7(&want) && v7(&have) && !case["event"].to_string().contains(&want) {
                expected["id"] = Value::Null;
                got["id"] = Value::Null;
            }
            if got != expected {
                let diff: Vec<String> = expected.as_object().unwrap().iter().filter(|(k, v)| got.get(*k) != Some(*v)).map(|(k, v)| format!("{k}: want {v}, got {}", got.get(k).unwrap_or(&Value::Null))).collect();
                failures.push(format!("{name}:\n  {}", diff.join("\n  ")));
            }
        }
        assert!(failures.is_empty(), "{} of the cases differ from Vector:\n{}", failures.len(), failures.join("\n"));
    }

    /// A generated id is a UUIDv7 of the log's time, so ids sort as the logs do.
    /// Maps are written in key order, the same in every row: ClickHouse stores a `Map` as arrays in the order given.
    #[test]
    fn rows_serialize_maps_in_key_order() {
        let fields = serde_json::json!({ "source_type": "kafka", "_oc_kafka_headers": { "x-oc-org": "org-1" }, "message": "hi", "zeta": 1, "alpha": 2, "mid": 3, "ddtags": "z:1,a:2,m:3" });
        let row = normalize(LogEvent { fields: fields.as_object().unwrap().clone(), timestamp: None }, chrono::Utc::now()).unwrap();
        let json = serde_json::to_string(&row).unwrap();
        let (a, m, z) = (json.find("\"alpha\"").unwrap(), json.find("\"mid\"").unwrap(), json.find("\"zeta\"").unwrap());
        assert!(a < m && m < z, "{json}");
        let tags = &json[json.find("\"tags\"").unwrap()..json.find("\"attr\"").unwrap()];
        let (ta, tm, tz) = (tags.find("\"a\"").unwrap(), tags.find("\"m\"").unwrap(), tags.find("\"z\"").unwrap());
        assert!(ta < tm && tm < tz, "{tags}");
    }

    #[test]
    fn generated_ids_carry_the_logs_millisecond() {
        let now = chrono::Utc::now();
        let event = serde_json::json!({ "source_type": "kafka", "_oc_kafka_headers": { "x-oc-org": "org-1" }, "message": "hi", "timestamp": "2026-11-05T10:20:30.456Z" });
        let row = normalize(LogEvent { fields: event.as_object().unwrap().clone(), timestamp: None }, now).unwrap();
        assert!(v7(&row.id), "{}", row.id);
        assert_eq!(i64::from_str_radix(&row.id.replace('-', "")[..12], 16).unwrap(), row.ts_ms);
        assert_eq!(row.ts_ms, chrono::DateTime::parse_from_rfc3339("2026-11-05T10:20:30.456Z").unwrap().timestamp_millis());
    }
}
