//! The internal listener (`OC_LOGPROC_ADDR`): the log indexer, and profile uploads from the intake
//! (`/ingest/profiles`, see `routes::profiles`).
//!
//! The log indexer: Vector sends every normalized log here (NDJSON, internal listener on
//! `OC_LOGPROC_ADDR`) after writing it to `oc.logs_raw`. Each log goes to the first of its org's
//! indexes whose filter matches; that index's exclusion filters may drop it (sampled
//! deterministically by log id) and its daily quota may cap it. Kept logs are written to `oc.logs`
//! with their index name and retention, which the table's TTL uses.
//!
//! Logs no index keeps are not lost: live tail and archives get every log (they read before this step).
//!
//! Index rules are reloaded from Postgres every 10 s; quota counters are local and re-synced from
//! ClickHouse every 30 s, so several servers sharing the work stay close to the quota.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::routing::post;
use axum::Router;
use oc_query::{matches, parse_query, Fields, Term};
use serde::Deserialize;
use serde_json::Value;

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
        Ok(())
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
        let s = |k: &str| ev.get(k).and_then(Value::as_str).unwrap_or("").to_string();
        let (org, id) = (s("org_id"), s("id"));
        let (service, status, host, source, message) = (s("service"), s("status"), s("host"), s("source"), s("message"));
        let (tags, attr) = (string_map(ev.get("tags")), string_map(ev.get("attr")));
        let f = Fields { service: &service, status: &status, host: &host, source: &source, message: &message, tags: &tags, attr: &attr, index: None };
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
    if let Some(token) = &st.config.internal_token {
        let ok = headers.get("authorization").and_then(|v| v.to_str().ok()).and_then(|v| v.strip_prefix("Bearer ")) == Some(token.as_str());
        if !ok {
            return StatusCode::UNAUTHORIZED;
        }
    }
    let (rows, _seen, kept) = route_batch(&ix, &body);
    if kept == 0 {
        return StatusCode::OK;
    }
    let res = reqwest::Client::new()
        .post(&st.config.clickhouse_url)
        .basic_auth(&st.config.clickhouse_user, Some(&st.config.clickhouse_password))
        .query(&[(
            "query",
            "INSERT INTO oc.logs SETTINGS input_format_skip_unknown_fields = 1, date_time_input_format = 'best_effort' FORMAT JSONEachRow",
        )])
        .body(rows)
        .send()
        .await;
    match res {
        Ok(r) if r.status().is_success() => StatusCode::OK,
        Ok(r) => {
            let status = r.status();
            let body: String = r.text().await.unwrap_or_default().chars().take(300).collect();
            tracing::warn!(%status, %body, "indexing logs failed");
            // Vector retries (and buffers on disk meanwhile).
            StatusCode::SERVICE_UNAVAILABLE
        }
        Err(e) => {
            tracing::warn!(error = %e, "indexing logs failed");
            StatusCode::SERVICE_UNAVAILABLE
        }
    }
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
    let addr = st.config.logproc_addr.clone();
    let profiles = Router::new()
        .route("/ingest/profiles", post(crate::routes::profiles::ingest))
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
    fn quota_days_start_at_the_reset_hour() {
        let r = IndexRule { reset_hour: 14, ..rule("main", "", Some(1), vec![]) };
        // 2026-10-25T13:00Z belongs to the day that started 2026-10-24T14:00Z.
        assert_eq!(r.day_start(1_792_933_200_000), 1_792_850_400_000);
    }
}
