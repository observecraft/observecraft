//! The internal listener (`OC_LOGPROC_ADDR`): the log indexer, and profile uploads from the intake
//! (`/ingest/profiles`, see `routes::profiles`) and feature flag evaluation counts
//! (`/ingest/feature-flag-evaluations`, see `routes::feature_flags`), and the server's own metrics
//! (`/metrics`, Prometheus text; see `cache`).
//!
//! The log indexer: Vector sends every normalized log here (NDJSON, internal listener on
//! `OC_LOGPROC_ADDR`) after writing it to `oc.logs_raw`. Each log goes to the first of its org's
//! indexes whose filter matches; that index's exclusion filters may drop it (sampled
//! deterministically by log id) and its daily quota may cap it. Kept logs are written to `oc.logs`
//! with their index name and retention, which the table's TTL uses.
//!
//! Logs no index keeps are not lost: live tail and archives get every log (they read before this step).
//!
//! Before routing, the org's log pipelines (see `pipelines`) process each log, so index filters,
//! exclusion filters and the indexed row see the parsed and remapped log.
//!
//! After the pipelines, the org's standard attributes (see `standard_attributes`) copy alias values
//! to their standard names (`status_code` → `http.status_code`), so generated metrics and routing see them.
//!
//! After the pipelines and before routing, every log is counted in the org's generated metrics (see
//! `log_metrics`), so logs an index excludes still count; their buckets are flushed every 10 s.
//!
//! Index rules, pipelines, standard attributes and generated metrics are reloaded from Postgres every 10 s; quota counters are local and
//! re-synced from ClickHouse every 30 s, so several servers sharing the work stay close to the quota.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use axum::body::Bytes;
use axum::extract::State;
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::Router;
use oc_query::{matches, parse_query, Fields, Term};
use serde::Deserialize;
use serde_json::Value;

use crate::log_metrics::{self, Aggregator, Definition, OrgMetrics, Retention};
use crate::pipelines::{self, Pipeline};
use crate::standard_attributes::{self, Override, Remapper, StoredRow};
use crate::state::AppState;

const DAY_MS: i64 = 86_400_000;

struct Exclusion {
    terms: Vec<Term>,
    /// Share of matching logs dropped, 0–100.
    rate: f64,
}

struct IndexRule {
    name: String,
    terms: Vec<Term>,
    retention_days: u16,
    daily_quota: Option<u64>,
    reset_hour: i64,
    exclusions: Vec<Exclusion>,
}

impl IndexRule {
    /// Start (epoch ms) of the quota day containing `now`.
    fn day_start(&self, now: i64) -> i64 {
        let offset = self.reset_hour * 3_600_000;
        (now - offset).div_euclid(DAY_MS) * DAY_MS + offset
    }
}

#[derive(Default)]
struct Counter {
    day_start: i64,
    n: u64,
}

#[derive(Default)]
pub struct Indexer {
    rules: RwLock<HashMap<String, Arc<Vec<IndexRule>>>>,
    counters: Mutex<HashMap<(String, String), Counter>>,
    /// Each org's enabled log pipelines, in order.
    pipelines: RwLock<HashMap<String, Arc<Vec<Pipeline>>>>,
    /// The standard attributes of each org that changed the defaults; others use the defaults.
    standard: RwLock<HashMap<String, Arc<Remapper>>>,
    /// Each org's generated metrics, and their buckets not flushed yet.
    log_metrics: RwLock<HashMap<String, Arc<OrgMetrics>>>,
    metric_buckets: Mutex<Aggregator>,
}

#[derive(sqlx::FromRow)]
struct LogMetricRow {
    org_id: String,
    name: String,
    query: String,
    aggregation: String,
    path: Option<String>,
    group_by: Value,
    raw: i32,
    rollup_5m: i32,
    rollup_1h: i32,
}

#[derive(sqlx::FromRow)]
struct PipelineRow {
    org_id: String,
    id: String,
    name: String,
    filter: String,
    processors: Value,
}

#[derive(Deserialize, sqlx::FromRow)]
struct RuleRow {
    org_id: String,
    name: String,
    filter: String,
    retention_days: i32,
    daily_quota: Option<i64>,
    quota_reset_hour: i16,
    exclusion_filters: Value,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ExclusionDef {
    query: String,
    sample_rate: f64,
    #[serde(default = "yes")]
    enabled: bool,
}

fn yes() -> bool {
    true
}

fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis() as i64
}

impl Indexer {
    async fn reload(&self, st: &AppState) -> Result<(), sqlx::Error> {
        let rows: Vec<RuleRow> = sqlx::query_as(
            "SELECT i.org_id, i.name, i.filter, i.retention_days, i.daily_quota, i.quota_reset_hour, i.exclusion_filters \
             FROM log_indexes i JOIN orgs o ON o.id = i.org_id WHERE o.status = 'active' ORDER BY i.org_id, i.position, i.created_at",
        )
        .fetch_all(&st.postgres)
        .await?;
        let mut by_org: HashMap<String, Vec<IndexRule>> = HashMap::new();
        for r in rows {
            let exclusions = serde_json::from_value::<Vec<ExclusionDef>>(r.exclusion_filters)
                .unwrap_or_default()
                .into_iter()
                .filter(|e| e.enabled)
                .map(|e| Exclusion { terms: parse_query(&e.query), rate: e.sample_rate.clamp(0.0, 100.0) })
                .collect();
            by_org.entry(r.org_id).or_default().push(IndexRule {
                name: r.name,
                terms: parse_query(&r.filter),
                retention_days: r.retention_days.clamp(1, u16::MAX as i32) as u16,
                daily_quota: r.daily_quota.map(|q| q as u64),
                reset_hour: r.quota_reset_hour as i64,
                exclusions,
            });
        }
        *self.rules.write().unwrap() = by_org.into_iter().map(|(k, v)| (k, Arc::new(v))).collect();
        self.reload_pipelines(st).await
    }

    async fn reload_pipelines(&self, st: &AppState) -> Result<(), sqlx::Error> {
        let rows: Vec<PipelineRow> = sqlx::query_as(
            "SELECT p.org_id, p.id::text AS id, p.name, p.filter, p.processors FROM log_pipelines p JOIN orgs o ON o.id = p.org_id \
             WHERE o.status = 'active' AND p.enabled ORDER BY p.org_id, p.position, p.created_at",
        )
        .fetch_all(&st.postgres)
        .await?;
        let mut by_org: HashMap<String, Vec<Pipeline>> = HashMap::new();
        for r in rows {
            let processors = r.processors.as_array().map(Vec::as_slice).unwrap_or_default();
            let (p, errors) = Pipeline::compile(&r.name, &r.filter, processors);
            // Saved pipelines were checked by the API; a processor that stopped compiling is skipped.
            if !errors.is_empty() {
                tracing::warn!(org = %r.org_id, pipeline = %r.id, errors = %errors.join("; "), "log pipeline processors skipped");
            }
            by_org.entry(r.org_id).or_default().push(p);
        }
        *self.pipelines.write().unwrap() = by_org.into_iter().map(|(k, v)| (k, Arc::new(v))).collect();
        self.reload_standard_attributes(st).await?;
        self.reload_log_metrics(st).await
    }

    async fn reload_standard_attributes(&self, st: &AppState) -> Result<(), sqlx::Error> {
        let rows: Vec<StoredRow> = sqlx::query_as(
            "SELECT a.org_id, a.path, a.hidden, a.type, a.description, a.aliases, a.preserve_source FROM log_standard_attributes a \
             JOIN orgs o ON o.id = a.org_id WHERE o.status = 'active' ORDER BY a.org_id, a.path",
        )
        .fetch_all(&st.postgres)
        .await?;
        let mut by_org: HashMap<String, Vec<Override>> = HashMap::new();
        for r in rows {
            by_org.entry(r.org_id.clone()).or_default().push(Override::from(r));
        }
        *self.standard.write().unwrap() = by_org.into_iter().map(|(org, o)| (org, Arc::new(Remapper::compile(&standard_attributes::merge(&o))))).collect();
        Ok(())
    }

    fn standard_for(&self, org: &str) -> Arc<Remapper> {
        self.standard.read().unwrap().get(org).cloned().unwrap_or_else(standard_attributes::default_remapper)
    }

    async fn reload_log_metrics(&self, st: &AppState) -> Result<(), sqlx::Error> {
        let rows: Vec<LogMetricRow> = sqlx::query_as(
            "SELECT m.org_id, m.name, m.query, m.aggregation, m.path, m.group_by, COALESCE(s.metrics_retention_days, 0) AS raw, \
             COALESCE(s.metrics_5m_days, 0) AS rollup_5m, COALESCE(s.metrics_1h_days, 0) AS rollup_1h \
             FROM log_metrics m JOIN orgs o ON o.id = m.org_id LEFT JOIN org_data_settings s ON s.org_id = m.org_id \
             WHERE o.status = 'active' ORDER BY m.org_id, m.name",
        )
        .fetch_all(&st.postgres)
        .await?;
        let mut by_org: HashMap<String, OrgMetrics> = HashMap::new();
        for r in rows {
            let retention = Retention { raw: r.raw, rollup_5m: r.rollup_5m, rollup_1h: r.rollup_1h };
            let definition = Definition::stored(&r.name, &r.query, &r.aggregation, r.path.as_deref(), &r.group_by);
            by_org.entry(r.org_id).or_insert_with(|| OrgMetrics { definitions: Vec::new(), retention }).definitions.push(definition);
        }
        *self.log_metrics.write().unwrap() = by_org.into_iter().map(|(k, v)| (k, Arc::new(v))).collect();
        Ok(())
    }

    /// Writes the generated metrics' buckets; a failed insert is logged and its points dropped.
    async fn flush_metrics(&self, st: &AppState) {
        let (points, dropped) = self.metric_buckets.lock().unwrap().drain();
        for ((org, metric), logs) in dropped {
            tracing::warn!(%org, %metric, logs, max_series = log_metrics::MAX_SERIES_PER_METRIC, "generated metric has too many series: logs of new series not counted");
        }
        let (sums, histograms) = log_metrics::rows(&points);
        for (table, rows) in [("otel.metrics_ingest_sum", sums), ("otel.metrics_ingest_exponential_histogram", histograms)] {
            if rows.is_empty() {
                continue;
            }
            if let Err(e) = insert_rows(st, table, rows).await {
                tracing::warn!(%table, error = %e, "writing generated metrics failed");
            }
        }
    }

    /// Brings quota counters up to what ClickHouse holds (other servers index too).
    async fn sync_counts(&self, st: &AppState) -> Result<(), clickhouse::error::Error> {
        #[derive(clickhouse::Row, Deserialize)]
        struct HourCount {
            org_id: String,
            index_name: String,
            hour: i64,
            n: u64,
        }
        let rows = st
            .clickhouse
            .query(
                "SELECT org_id, toString(index_name) AS index_name, toInt64(toUnixTimestamp(toStartOfHour(timestamp))) * 1000 AS hour, count() AS n \
                 FROM logs WHERE timestamp >= now() - INTERVAL 25 HOUR GROUP BY org_id, index_name, hour",
            )
            .fetch_all::<HourCount>()
            .await?;
        let now = now_ms();
        let rules = self.rules.read().unwrap().clone();
        let mut totals: HashMap<(String, String), (i64, u64)> = HashMap::new();
        for r in rows {
            let Some(rule) = rules.get(&r.org_id).and_then(|rs| rs.iter().find(|x| x.name == r.index_name)).filter(|x| x.daily_quota.is_some()) else { continue };
            let start = rule.day_start(now);
            if r.hour >= start {
                let e = totals.entry((r.org_id, r.index_name)).or_insert((start, 0));
                e.1 += r.n;
            }
        }
        let mut counters = self.counters.lock().unwrap();
        for (key, (start, n)) in totals {
            let c = counters.entry(key).or_default();
            if c.day_start != start {
                *c = Counter { day_start: start, n: 0 };
            }
            c.n = c.n.max(n);
        }
        Ok(())
    }

    /// The index that keeps this log (and its retention), or None when it isn't indexed.
    fn route(&self, org: &str, id: &str, f: &Fields<'_>, now: i64) -> Option<(String, u16)> {
        let rules = self.rules.read().unwrap().get(org).cloned()?;
        let rule = rules.iter().find(|r| matches(&r.terms, f))?;
        if let Some(ex) = rule.exclusions.iter().find(|e| matches(&e.terms, f)) {
            let mut h = std::collections::hash_map::DefaultHasher::new();
            id.hash(&mut h);
            if (h.finish() % 10_000) as f64 / 100.0 < ex.rate {
                return None;
            }
        }
        if let Some(quota) = rule.daily_quota {
            let start = rule.day_start(now);
            let mut counters = self.counters.lock().unwrap();
            let c = counters.entry((org.to_string(), rule.name.clone())).or_default();
            if c.day_start != start {
                *c = Counter { day_start: start, n: 0 };
            }
            if c.n >= quota {
                return None;
            }
            c.n += 1;
        }
        Some((rule.name.clone(), rule.retention_days))
    }
}

fn string_map(v: Option<&Value>) -> HashMap<String, String> {
    v.and_then(Value::as_object).map(|o| o.iter().map(|(k, v)| (k.clone(), v.as_str().map(str::to_string).unwrap_or_else(|| v.to_string()))).collect()).unwrap_or_default()
}

/// Routes one NDJSON batch; returns the kept rows as NDJSON for `INSERT … FORMAT JSONEachRow`.
fn route_batch(ix: &Indexer, body: &[u8]) -> (Vec<u8>, usize, usize) {
    let now = now_ms();
    let (mut out, mut seen, mut kept) = (Vec::with_capacity(body.len()), 0, 0);
    for line in body.split(|b| *b == b'\n').filter(|l| !l.is_empty()) {
        let Ok(mut ev) = serde_json::from_slice::<Value>(line) else { continue };
        seen += 1;
        let org_id = ev.get("org_id").and_then(Value::as_str).unwrap_or("").to_string();
        let org_pipelines = ix.pipelines.read().unwrap().get(&org_id).cloned().unwrap_or_default();
        pipelines::process_row(&org_pipelines, &ix.standard_for(&org_id), &mut ev, now);
        let s = |k: &str| ev.get(k).and_then(Value::as_str).unwrap_or("").to_string();
        let (org, id) = (s("org_id"), s("id"));
        let (service, status, host, source, message) = (s("service"), s("status"), s("host"), s("source"), s("message"));
        let (tags, attr) = (string_map(ev.get("tags")), string_map(ev.get("attr")));
        let f = Fields { service: &service, status: &status, host: &host, source: &source, message: &message, tags: &tags, attr: &attr, index: None };
        let org_metrics = ix.log_metrics.read().unwrap().get(&org).cloned();
        if let Some(m) = org_metrics {
            let ts = ev.get("ts_ms").and_then(Value::as_i64).unwrap_or(now);
            ix.metric_buckets.lock().unwrap().record(&org, &m, &f, ts, now);
        }
        let Some((index, retention)) = ix.route(&org, &id, &f, now) else { continue };
        if let Some(o) = ev.as_object_mut() {
            o.insert("index_name".into(), Value::String(index));
            o.insert("retention_days".into(), Value::from(retention));
        }
        if serde_json::to_writer(&mut out, &ev).is_ok() {
            out.push(b'\n');
            kept += 1;
        }
    }
    (out, seen, kept)
}

async fn ingest(State((st, ix)): State<(AppState, Arc<Indexer>)>, headers: HeaderMap, body: Bytes) -> StatusCode {
    if !internal_authorized(st.config.internal_token.as_deref(), &headers) {
        return StatusCode::UNAUTHORIZED;
    }
    let (rows, _seen, kept) = route_batch(&ix, &body);
    if kept == 0 {
        return StatusCode::OK;
    }
    match insert_rows(&st, "oc.logs", rows).await {
        Ok(()) => StatusCode::OK,
        Err(e) => {
            tracing::warn!(error = %e, "indexing logs failed");
            // Vector retries (and buffers on disk meanwhile).
            StatusCode::SERVICE_UNAVAILABLE
        }
    }
}

/// `INSERT INTO <table> … FORMAT JSONEachRow` of NDJSON rows.
pub(crate) async fn insert_rows(st: &AppState, table: &str, rows: Vec<u8>) -> Result<(), String> {
    let query = format!("INSERT INTO {table} SETTINGS input_format_skip_unknown_fields = 1, date_time_input_format = 'best_effort' FORMAT JSONEachRow");
    let res = reqwest::Client::new()
        .post(&st.config.clickhouse_url)
        .basic_auth(&st.config.clickhouse_user, Some(&st.config.clickhouse_password))
        .query(&[("query", query)])
        .body(rows)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if res.status().is_success() {
        return Ok(());
    }
    let status = res.status();
    let body: String = res.text().await.unwrap_or_default().chars().take(300).collect();
    Err(format!("{status}: {body}"))
}

/// Requests to the internal listener carry `Authorization: Bearer <OC_INTERNAL_TOKEN>` when it is set.
fn internal_authorized(token: Option<&str>, headers: &HeaderMap) -> bool {
    token.is_none_or(|token| headers.get("authorization").and_then(|v| v.to_str().ok()).and_then(|v| v.strip_prefix("Bearer ")) == Some(token))
}

/// The server's own counters (shared cache: `oc_cache_*`) in Prometheus text format, for the
/// Datadog Agent's OpenMetrics check or a Collector's `prometheus` receiver.
async fn internal_metrics(State(st): State<AppState>, headers: HeaderMap) -> Response {
    if !internal_authorized(st.config.internal_token.as_deref(), &headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    ([(header::CONTENT_TYPE, "text/plain; version=0.0.4")], st.cache.render_metrics()).into_response()
}

/// Starts the internal indexer listener and its refresh loops.
pub async fn serve(st: AppState) -> anyhow::Result<()> {
    let ix = Arc::new(Indexer::default());
    ix.reload(&st).await?;
    let (st2, ix2) = (st.clone(), ix.clone());
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(10));
        let mut n = 0u64;
        loop {
            tick.tick().await;
            if let Err(e) = ix2.reload(&st2).await {
                tracing::warn!(error = %e, "reloading log indexes failed");
            }
            if n.is_multiple_of(3) {
                if let Err(e) = ix2.sync_counts(&st2).await {
                    tracing::warn!(error = %e, "syncing index quotas failed");
                }
            }
            n += 1;
        }
    });
    let (st3, ix3) = (st.clone(), ix.clone());
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_millis(log_metrics::BUCKET_MS as u64));
        loop {
            tick.tick().await;
            ix3.flush_metrics(&st3).await;
        }
    });
    let addr = st.config.logproc_addr.clone();
    let profiles = Router::new()
        .route("/ingest/profiles", post(crate::routes::profiles::ingest))
        .route("/ingest/feature-flag-evaluations", post(crate::routes::feature_flags::ingest_evaluations))
        .route("/ingest/apm-telemetry", post(crate::routes::code_security::ingest_telemetry))
        .route("/ingest/code-scans", post(crate::routes::code_security::ingest_sarif))
        .route("/metrics", get(internal_metrics))
        .with_state(st.clone());
    let app = Router::new()
        .route("/ingest/logs", post(ingest))
        .with_state((st, ix))
        .merge(profiles)
        .layer(axum::extract::DefaultBodyLimit::max(64 * 1024 * 1024));
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    tracing::info!(%addr, "log indexer listening");
    tokio::spawn(async move {
        if let Err(e) = axum::serve(listener, app).await {
            tracing::error!(error = %e, "log indexer stopped");
        }
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::log_metrics::{Aggregation, PointValue};

    fn indexer(rules: Vec<IndexRule>) -> Indexer {
        let ix = Indexer::default();
        ix.rules.write().unwrap().insert("acme".into(), Arc::new(rules));
        ix
    }

    fn rule(name: &str, filter: &str, quota: Option<u64>, exclusions: Vec<Exclusion>) -> IndexRule {
        IndexRule { name: name.into(), terms: parse_query(filter), retention_days: 7, daily_quota: quota, reset_hour: 0, exclusions }
    }

    fn line(org: &str, id: &str, service: &str, status: &str) -> String {
        format!("{{\"org_id\":\"{org}\",\"id\":\"{id}\",\"service\":\"{service}\",\"status\":\"{status}\",\"message\":\"m\",\"tags\":{{\"env\":\"prod\"}},\"attr\":{{}}}}\n")
    }

    #[test]
    fn internal_requests_need_the_token_when_one_is_set() {
        let mut h = HeaderMap::new();
        assert!(internal_authorized(None, &h));
        assert!(!internal_authorized(Some("s3cret"), &h));
        h.insert("authorization", "Bearer wrong".parse().unwrap());
        assert!(!internal_authorized(Some("s3cret"), &h));
        h.insert("authorization", "Bearer s3cret".parse().unwrap());
        assert!(internal_authorized(Some("s3cret"), &h));
    }

    #[test]
    fn first_matching_index_wins_and_unmatched_logs_are_not_indexed() {
        let ix = indexer(vec![rule("errors", "status:error", None, vec![]), rule("prod", "env:prod -service:noisy", None, vec![])]);
        let body = [line("acme", "1", "api", "error"), line("acme", "2", "api", "info"), line("acme", "3", "noisy", "info"), line("other", "4", "api", "error")].concat();
        let (out, seen, kept) = route_batch(&ix, body.as_bytes());
        assert_eq!((seen, kept), (4, 2));
        let rows: Vec<Value> = out.split(|b| *b == b'\n').filter(|l| !l.is_empty()).map(|l| serde_json::from_slice(l).unwrap()).collect();
        assert_eq!(rows[0]["index_name"], "errors");
        assert_eq!(rows[1]["index_name"], "prod");
        assert_eq!(rows[0]["retention_days"], 7);
    }

    #[test]
    fn daily_quota_caps_an_index() {
        let ix = indexer(vec![rule("main", "", Some(2), vec![])]);
        let body = (0..5).map(|i| line("acme", &i.to_string(), "api", "info")).collect::<String>();
        assert_eq!(route_batch(&ix, body.as_bytes()).2, 2);
    }

    #[test]
    fn exclusion_filters_sample_matching_logs() {
        let ix = indexer(vec![rule("main", "", None, vec![Exclusion { terms: parse_query("status:info"), rate: 100.0 }])]);
        let body = [line("acme", "1", "api", "info"), line("acme", "2", "api", "error")].concat();
        assert_eq!(route_batch(&ix, body.as_bytes()).2, 1);
        let half = indexer(vec![rule("main", "", None, vec![Exclusion { terms: parse_query(""), rate: 50.0 }])]);
        let body = (0..2000).map(|i| line("acme", &format!("id-{i}"), "api", "info")).collect::<String>();
        let kept = route_batch(&half, body.as_bytes()).2;
        assert!((800..1200).contains(&kept), "about half kept, got {kept}");
    }

    #[test]
    fn pipelines_run_before_index_routing() {
        let ix = indexer(vec![rule("server-errors", "@http.status_code:5* status:error", None, vec![]), rule("main", "", None, vec![])]);
        let processors = serde_json::json!([
            {"type": "grok-parser", "grok": {"matchRules": "access %{word:http.method} %{notSpace:http.url} %{integer:http.status_code}"}},
            {"type": "category-processor", "target": "level", "categories": [{"filter": {"query": "@http.status_code:[500 TO 599]"}, "name": "error"}]},
            {"type": "status-remapper", "sources": ["level"]}
        ]);
        let (p, errors) = Pipeline::compile("access logs", "service:api", processors.as_array().unwrap());
        assert!(errors.is_empty());
        ix.pipelines.write().unwrap().insert("acme".into(), Arc::new(vec![p]));
        let access = |id: &str, msg: &str| format!("{{\"org_id\":\"acme\",\"id\":\"{id}\",\"service\":\"api\",\"status\":\"info\",\"message\":\"{msg}\",\"tags\":{{}},\"attr\":{{}},\"attributes\":\"{{}}\"}}\n");
        let body = [access("1", "GET /cart 503"), access("2", "GET /cart 200"), line("other", "3", "api", "info")].concat();
        let (out, seen, kept) = route_batch(&ix, body.as_bytes());
        assert_eq!((seen, kept), (3, 2), "the other org has no index");
        let rows: Vec<Value> = out.split(|b| *b == b'\n').filter(|l| !l.is_empty()).map(|l| serde_json::from_slice(l).unwrap()).collect();
        assert_eq!((rows[0]["index_name"].as_str(), rows[0]["status"].as_str()), (Some("server-errors"), Some("error")));
        assert_eq!(rows[0]["attr"]["http.url"], "/cart");
        assert_eq!(serde_json::from_str::<Value>(rows[0]["attributes"].as_str().unwrap()).unwrap()["http"]["status_code"], 503);
        assert_eq!((rows[1]["index_name"].as_str(), rows[1]["status"].as_str()), (Some("main"), Some("info")));
    }

    #[test]
    fn generated_metrics_see_processed_logs_including_excluded_ones() {
        let ix = indexer(vec![rule("main", "", None, vec![Exclusion { terms: parse_query("status:info"), rate: 100.0 }])]);
        let (p, errors) = Pipeline::compile("teams", "service:api", serde_json::json!([{"type": "string-builder-processor", "template": "checkout", "target": "team"}]).as_array().unwrap());
        assert!(errors.is_empty());
        ix.pipelines.write().unwrap().insert("acme".into(), Arc::new(vec![p]));
        let group_by = [("status".to_string(), "status".to_string()), ("@team".to_string(), "team".to_string())];
        let definitions = vec![Definition::new("logs.api", "service:api", Aggregation::Count, &group_by)];
        ix.log_metrics.write().unwrap().insert("acme".into(), Arc::new(OrgMetrics { definitions, retention: Retention::default() }));
        let body = [line("acme", "1", "api", "info"), line("acme", "2", "api", "error"), line("acme", "3", "web", "error"), line("other", "4", "api", "error")].concat();
        let (_, seen, kept) = route_batch(&ix, body.as_bytes());
        assert_eq!((seen, kept), (4, 2), "the info log is excluded from the index");
        let (points, _) = ix.metric_buckets.lock().unwrap().drain();
        let mut got: Vec<_> = points.iter().map(|p| (p.org.as_str(), p.tags.clone(), p.value.clone())).collect();
        got.sort_by(|a, b| a.1.cmp(&b.1));
        let tags = |status: &str| vec![("status".to_string(), status.to_string()), ("team".to_string(), "checkout".to_string())];
        assert_eq!(got, vec![("acme", tags("error"), PointValue::Count(1)), ("acme", tags("info"), PointValue::Count(1))]);
    }

    #[test]
    fn standard_attributes_are_remapped_before_routing_and_generated_metrics() {
        let ix = indexer(vec![rule("server-errors", "@http.status_code:5*", None, vec![]), rule("main", "", None, vec![])]);
        let definitions = vec![Definition::new("logs.api.codes", "", Aggregation::Count, &[("@http.status_code".to_string(), "code".to_string())])];
        ix.log_metrics.write().unwrap().insert("acme".into(), Arc::new(OrgMetrics { definitions, retention: Retention::default() }));
        let access = |id: &str, attrs: &str| {
            let attr: Value = serde_json::from_str(attrs).unwrap();
            let flat: serde_json::Map<String, Value> = crate::pipelines::log::flatten(attr.as_object().unwrap()).into_iter().map(|(k, v)| (k, Value::from(v))).collect();
            format!(
                "{}\n",
                serde_json::json!({"org_id": "acme", "id": id, "service": "api", "status": "info", "message": "m", "tags": {}, "attr": flat, "attributes": attrs})
            )
        };
        // The defaults apply to an org that changed nothing; an existing standard value stays.
        let body = [access("1", r#"{"status_code":"503"}"#), access("2", r#"{"http":{"status_code":200},"status_code":"503"}"#), access("3", r#"{"team":"checkout"}"#)].concat();
        let (out, seen, kept) = route_batch(&ix, body.as_bytes());
        assert_eq!((seen, kept), (3, 3));
        let rows: Vec<Value> = out.split(|b| *b == b'\n').filter(|l| !l.is_empty()).map(|l| serde_json::from_slice(l).unwrap()).collect();
        assert_eq!((rows[0]["index_name"].as_str(), rows[0]["attr"]["http.status_code"].as_str(), rows[0]["attr"]["status_code"].as_str()), (Some("server-errors"), Some("503"), Some("503")));
        assert_eq!(serde_json::from_str::<Value>(rows[0]["attributes"].as_str().unwrap()).unwrap()["http"]["status_code"], 503, "cast to the integer type");
        assert_eq!((rows[1]["index_name"].as_str(), rows[1]["attr"]["http.status_code"].as_str()), (Some("main"), Some("200")));
        assert_eq!(rows[2]["attributes"], r#"{"team":"checkout"}"#, "a log without aliases isn't rewritten");
        let (points, _) = ix.metric_buckets.lock().unwrap().drain();
        let mut codes: Vec<_> = points.iter().map(|p| p.tags.clone()).collect();
        codes.sort();
        assert!(codes.contains(&vec![("code".to_string(), "503".to_string())]), "generated metrics see the standard attribute: {codes:?}");
        // An org's own list replaces the defaults.
        let own = crate::standard_attributes::merge(&[crate::standard_attributes::Override { path: "http.status_code".into(), definition: None }]);
        ix.standard.write().unwrap().insert("acme".into(), Arc::new(Remapper::compile(&own)));
        let (out, _, _) = route_batch(&ix, access("4", r#"{"status_code":"503"}"#).as_bytes());
        let row: Value = serde_json::from_slice(out.trim_ascii_end()).unwrap();
        assert_eq!((row["index_name"].as_str(), row["attr"].get("http.status_code")), (Some("main"), None));
    }

    #[test]
    fn quota_days_start_at_the_reset_hour() {
        let r = IndexRule { reset_hour: 14, ..rule("main", "", Some(1), vec![]) };
        // 2026-10-25T13:00Z belongs to the day that started 2026-10-24T14:00Z.
        assert_eq!(r.day_start(1_792_933_200_000), 1_792_850_400_000);
    }
}
