//! Every integration pipeline against its upstream test samples: integrations-core's and integrations-extras'
//! `assets/logs/<pipeline>_tests.yaml` (BSD-3-Clause), the logs and the results Datadog's engine gives for them.
//! They are not part of this repository: `scripts/integration-log-samples/fetch.sh` (`make integration-log-samples`)
//! downloads them at the revisions the pipelines were imported from, and the test is skipped without them.
//!
//! Each sample goes the way an Agent's log does: the intake's envelope, `log_normalize`, then the pipeline. The log
//! passes when its message, status, date, service, trace id, pipeline tags and attributes are the upstream result's.
//! `OC_LOG_SAMPLES_REPORT=<file>` writes every failing sample's differences there.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde_json::{Map, Value};

use super::library::{all, IntegrationPipeline};
use super::{Log, Pipeline};

/// The revisions in `fetch.sh`.
const CORE: &str = "1e7420ec635aafdd33db6a1e69586de7d2e24c21";
const EXTRAS: &str = "20d3ceeaf342b8c9f6211dc9df9ea3d382dbb692";

/// Samples that pass today: the test fails when fewer do (raise it as the engine catches up).
const PASSING: usize = 1299;

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.is_empty())
}

/// Where `fetch.sh` puts the samples, or None (skipped) when they aren't there; `OC_REQUIRE_LOG_SAMPLES=1` panics instead.
fn samples_dir() -> Option<PathBuf> {
    let dir = env("OC_INTEGRATION_LOG_SAMPLES").map(PathBuf::from).unwrap_or_else(|| {
        let cache = env("XDG_CACHE_HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(env("HOME").unwrap_or_default()).join(".cache"));
        cache.join("observecraft/integration-log-samples").join(format!("{CORE}-{EXTRAS}"))
    });
    if dir.join(".complete").is_file() {
        return Some(dir);
    }
    let hint = format!("integration log samples not found in {} (fetch them with `make integration-log-samples`)", dir.display());
    assert!(env("OC_REQUIRE_LOG_SAMPLES").as_deref() != Some("1"), "{hint}; OC_REQUIRE_LOG_SAMPLES=1 requires them");
    eprintln!("{hint}: skipping");
    None
}

/// The source the pipeline's filter selects (`source:nginx`, `source:(a OR b)` → its first), else the integration.
fn source_of(p: &IntegrationPipeline) -> String {
    let f = &p.filter;
    let Some(i) = f.find("source:") else { return p.id.clone() };
    f[i + 7..].trim_start_matches(['(', '"']).split(|c: char| !(c.is_alphanumeric() || "_-./".contains(c))).next().unwrap_or_default().to_string()
}

/// The sample as the Agent forwards it, normalized by the log ingester, then through the pipeline.
fn run(pipeline: &Pipeline, source: &str, test: &Value) -> Log {
    let sample = match &test["sample"] {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    };
    let mut envelope = serde_json::json!({"message": sample, "ddsource": source, "source_type": "kafka", "_oc_kafka_headers": {"x-oc-org": "example-org"}});
    if let Some(service) = test["service"].as_str() {
        envelope["service"] = service.into();
    }
    if let Some(tags) = test["tags"].as_array() {
        envelope["ddtags"] = tags.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(",").into();
    }
    let fields = envelope.as_object().unwrap().clone();
    // A row's date is the log's own when it isn't the ingest time.
    let ingested = chrono::DateTime::from_timestamp_millis(1).unwrap();
    let mut row = crate::log_normalize::normalize(crate::log_normalize::LogEvent { fields, timestamp: None }, ingested).unwrap();
    let mut log = Log::default();
    (log.message, log.status, log.service, log.host, log.source, log.trace_id) = (row.message, row.status, row.service, row.host, row.source, row.trace_id);
    log.tags = row.tags.into_iter().collect();
    log.attrs = std::mem::take(&mut row.attrs);
    pipeline.apply(&mut log, chrono::Utc::now().timestamp_millis());
    log.timestamp_ms = log.timestamp_ms.or((row.ts_ms != 1).then_some(row.ts_ms));
    log
}

/// Upstream's results come from a run in early 1971: a date without a year is that year's. Same month, day and time.
fn same_but_year(got: i64, want: i64) -> bool {
    use chrono::{DateTime, Datelike, Timelike};
    let (Some(g), Some(w)) = (DateTime::from_timestamp_millis(got), DateTime::from_timestamp_millis(want)) else { return false };
    want < 100_000_000_000 && (g.month(), g.day(), g.num_seconds_from_midnight(), g.timestamp_subsec_millis()) == (w.month(), w.day(), w.num_seconds_from_midnight(), w.timestamp_subsec_millis())
}

/// Numbers compare by value (`1` and `1.0`), as upstream's YAML writes them.
fn same(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => x.as_f64() == y.as_f64() || x.as_i64().zip(y.as_i64()).is_some_and(|(g, w)| same_but_year(g, w)),
        (Value::Object(x), Value::Object(y)) => x.len() == y.len() && x.iter().all(|(k, v)| y.get(k).is_some_and(|w| same(v, w))),
        (Value::Array(x), Value::Array(y)) => x.len() == y.len() && x.iter().zip(y).all(|(v, w)| same(v, w)),
        _ => a == b,
    }
}

/// The attributes' differences, as `path: got → want`.
fn diff(path: &str, got: Option<&Value>, want: Option<&Value>, out: &mut Vec<String>) {
    match (got, want) {
        (Some(Value::Object(g)), Some(Value::Object(w))) => {
            let keys: std::collections::BTreeSet<&String> = g.keys().chain(w.keys()).collect();
            for k in keys {
                diff(&if path.is_empty() { k.clone() } else { format!("{path}.{k}") }, g.get(k), w.get(k), out);
            }
        }
        (Some(g), Some(w)) if same(g, w) => {}
        (None, None) => {}
        (g, w) => out.push(format!("@{path}: {} → {}", g.map_or("-".into(), short), w.map_or("-".into(), short))),
    }
}

fn short(v: &Value) -> String {
    let s = v.to_string();
    if s.len() > 80 {
        format!("{}…", &s[..s.char_indices().nth(80).map_or(s.len(), |(i, _)| i)])
    } else {
        s
    }
}

/// What differs between the processed log and upstream's result (empty when it passes).
fn compare(log: &Log, test: &Value) -> Vec<String> {
    let want = &test["result"];
    let mut out = Vec::new();
    let mut field = |name: &str, got: &str, want: &Value| {
        if let Some(w) = want.as_str() {
            // A JSON log without a message is its JSON, which upstream prints again.
            let json = |s: &str| serde_json::from_str::<Value>(s).ok().filter(Value::is_object);
            if got != w && !json(got).is_some_and(|g| json(w).is_some_and(|w| same(&g, &w))) {
                out.push(format!("{name}: {} → {}", short(&got.into()), short(&w.into())));
            }
        }
    };
    field("message", &log.message, &want["message"]);
    // ObserveCraft's four statuses: Datadog's ok and notice are info; critical, alert and emergency are error.
    let status = want["status"].as_str().map(|s| match s {
        "ok" | "notice" => "info",
        "critical" | "alert" | "emergency" => "error",
        s => s,
    });
    field("status", &log.status, &status.into());
    field("service", &log.service, &want["service"]);
    // Trace ids are stored as 32 hex digits.
    field("trace_id", &log.trace_id, &want["trace_id"].as_str().and_then(super::processors::trace_id).into());
    // Upstream's `1` is "no date of the log's own".
    match (want["timestamp"].as_i64(), log.timestamp_ms) {
        (Some(1) | None, _) => {}
        (Some(w), Some(g)) if same_but_year(g, w) => {}
        (Some(w), g) if g != Some(w) => out.push(format!("timestamp: {} → {w}", g.map_or("-".into(), |g| g.to_string()))),
        _ => {}
    }
    // The tags the pipeline adds (upstream lists the sample's source and input tags too).
    let given: Vec<&str> = test["tags"].as_array().map(|t| t.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
    for tag in want["tags"].as_array().into_iter().flatten().filter_map(Value::as_str) {
        if tag.starts_with("source:") || given.contains(&tag) {
            continue;
        }
        let (k, v) = tag.split_once(':').unwrap_or((tag, ""));
        if log.tags.get(k).map(String::as_str) != Some(v) {
            out.push(format!("tag {tag} missing"));
        }
    }
    let empty = Map::new();
    let custom = want["custom"].as_object().unwrap_or(&empty);
    diff("", Some(&Value::Object(log.attrs.clone())), Some(&Value::Object(custom.clone())), &mut out);
    out
}

#[test]
fn integration_pipelines_give_upstreams_results_for_its_samples() {
    let Some(dir) = samples_dir() else { return };
    let (mut total, mut passed) = (0, 0);
    let mut report = String::new();
    let mut per: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    for p in all().iter().filter(|p| p.source.license == "BSD-3-Clause") {
        let repo = p.source.repository.trim_start_matches("DataDog/");
        let tests = dir.join(repo).join(p.source.path.replace(".yaml", "_tests.yaml"));
        let Ok(text) = std::fs::read_to_string(&tests) else { continue };
        let doc: Value = serde_yaml_ng::from_str(&text).unwrap_or_else(|e| panic!("{}: {e}", tests.display()));
        let (pipeline, errors) = Pipeline::compile(&p.name, &p.filter, &p.processors);
        let source = source_of(p);
        let entry = per.entry(p.id.clone()).or_default();
        for (i, test) in doc["tests"].as_array().into_iter().flatten().enumerate() {
            let log = run(&pipeline, &source, test);
            let diffs = compare(&log, test);
            total += 1;
            entry.1 += 1;
            if diffs.is_empty() {
                passed += 1;
                entry.0 += 1;
            } else {
                report += &format!("{} #{}{}\n  {}\n", p.id, i + 1, if errors.is_empty() { String::new() } else { format!(" (compile: {errors:?})") }, diffs.join("\n  "));
            }
        }
    }
    let full = per.values().filter(|(ok, n)| ok == n).count();
    let summary = format!("{passed} of {total} samples pass; {full} of {} pipelines pass all theirs", per.len());
    eprintln!("{summary}");
    if let Some(path) = env("OC_LOG_SAMPLES_REPORT") {
        let lines: String = per.iter().map(|(id, (ok, n))| format!("{id} {ok}/{n}\n")).collect();
        std::fs::write(path, format!("{summary}\n\n{lines}\n{report}")).unwrap();
    }
    assert!(passed >= PASSING, "{summary} (was {PASSING})");
}
