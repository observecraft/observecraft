//! Durable, bounded retry spool for generated metrics. Acknowledging a source request requires
//! committing its aggregated points here. Export is at least once: a crash after ClickHouse has
//! accepted an insert but before this transaction commits can duplicate that insert.

use std::collections::BTreeMap;
use std::time::Duration;

use futures::{stream, StreamExt};

use crate::log_metrics::{self, Point};
use crate::state::AppState;

const MAX_BYTES_PER_ORG: i64 = 64 * 1024 * 1024;
const MAX_BATCHES_PER_ORG: i64 = 4096;
const EXPORT_TIMEOUT: Duration = Duration::from_secs(30);

/// One transaction covers all orgs in an internal request; a full spool fails the entire request.
/// Locks are acquired in org order. Only configured log metrics put traffic through this spool.
pub async fn enqueue(st: &AppState, points: Vec<Point>) -> Result<(), String> {
    enqueue_with_limit(st, points, MAX_BYTES_PER_ORG, MAX_BATCHES_PER_ORG).await
}

async fn enqueue_with_limit(st: &AppState, points: Vec<Point>, max_bytes: i64, max_batches: i64) -> Result<(), String> {
    let mut orgs: BTreeMap<String, Vec<Point>> = BTreeMap::new();
    for point in points {
        orgs.entry(point.org.clone()).or_default().push(point);
    }
    if orgs.is_empty() {
        return Ok(());
    }
    let mut tx = st.postgres.begin().await.map_err(|e| e.to_string())?;
    for (org, points) in orgs {
        let (sums, histograms) = log_metrics::rows(&points);
        let bytes = (sums.len() + histograms.len()) as i64;
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 724914))")
            .bind(&org)
            .execute(&mut *tx)
            .await
            .map_err(|e| e.to_string())?;
        let (pending, batches): (i64, i64) =
            sqlx::query_as("SELECT COALESCE(sum(octet_length(sums)::bigint + octet_length(histograms)), 0)::bigint, count(*) FROM log_metric_outbox WHERE org_id = $1")
                .bind(&org)
                .fetch_one(&mut *tx)
                .await
                .map_err(|e| e.to_string())?;
        if bytes > max_bytes - pending || batches >= max_batches {
            return Err(format!("generated metric spool is full for organization {org}"));
        }
        sqlx::query("INSERT INTO log_metric_outbox (org_id, sums, histograms) VALUES ($1, $2, $3)")
            .bind(&org)
            .bind(sums)
            .bind(histograms)
            .execute(&mut *tx)
            .await
            .map_err(|e| e.to_string())?;
    }
    tx.commit().await.map_err(|e| e.to_string())
}

/// A worker holds a row lock during bounded export. Other replicas skip that row, and restart
/// releases the lock without deleting its payload. Commit successful sinks even if another fails.
async fn export_one(st: &AppState, org: &str) -> Result<bool, String> {
    let mut tx = st.postgres.begin().await.map_err(|e| e.to_string())?;
    let batch: Option<(String, Vec<u8>, Vec<u8>)> = sqlx::query_as("SELECT id::text, sums, histograms FROM log_metric_outbox WHERE org_id = $1 ORDER BY created_at, id FOR UPDATE SKIP LOCKED LIMIT 1")
        .bind(org)
        .fetch_optional(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;
    let Some((id, mut sums, mut histograms)) = batch else { return Ok(false) };
    let mut failure = None;
    let mut changed = false;
    for (table, rows) in [("otel.metrics_log_ingest_sum", &mut sums), ("otel.metrics_log_ingest_exponential_histogram", &mut histograms)] {
        if rows.is_empty() {
            continue;
        }
        let result = tokio::time::timeout(EXPORT_TIMEOUT, crate::logproc::insert_rows(st, table, rows.clone())).await;
        match result {
            Ok(Ok(())) => {
                rows.clear();
                changed = true;
            }
            other => {
                failure = Some(match other {
                    Ok(Err(e)) => e,
                    Err(_) => "generated metric export timed out; delivery outcome is unknown".into(),
                    Ok(Ok(())) => unreachable!(),
                });
                break;
            }
        }
    }
    if sums.is_empty() && histograms.is_empty() {
        sqlx::query("DELETE FROM log_metric_outbox WHERE org_id = $1 AND id = $2::uuid")
            .bind(org)
            .bind(&id)
            .execute(&mut *tx)
            .await
            .map_err(|e| e.to_string())?;
    } else if changed {
        // Keep failed payload columns untouched: retrying an outage must not rewrite their TOAST/WAL.
        sqlx::query("UPDATE log_metric_outbox SET sums = CASE WHEN $3 THEN ''::bytea ELSE sums END, histograms = CASE WHEN $4 THEN ''::bytea ELSE histograms END WHERE org_id = $1 AND id = $2::uuid")
            .bind(org)
            .bind(&id)
            .bind(sums.is_empty())
            .bind(histograms.is_empty())
            .execute(&mut *tx)
            .await
            .map_err(|e| e.to_string())?;
    }
    tx.commit().await.map_err(|e| e.to_string())?;
    if let Some(error) = failure {
        return Err(error);
    }
    Ok(true)
}

pub async fn run(st: AppState) {
    let mut tick = tokio::time::interval(Duration::from_secs(2));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tick.tick().await;
        // Metadata discovery only; each payload read/write below is scoped to one active org.
        let orgs = sqlx::query_scalar::<_, String>("SELECT id FROM orgs WHERE status = 'active' AND EXISTS (SELECT 1 FROM log_metric_outbox WHERE org_id = orgs.id) ORDER BY id")
            .fetch_all(&st.postgres)
            .await;
        let orgs = match orgs {
            Ok(orgs) => orgs,
            Err(e) => {
                tracing::warn!(error = %e, "discovering generated metric exports failed");
                continue;
            }
        };
        stream::iter(orgs)
            .map(|org| {
                let st = &st;
                async move {
                    for _ in 0..64 {
                        match export_one(st, &org).await {
                            Ok(true) => {}
                            Ok(false) => break,
                            Err(e) => {
                                tracing::warn!(%org, error = %e, "generated metric export deferred");
                                break;
                            }
                        }
                    }
                }
            })
            .buffer_unordered(4)
            .collect::<Vec<_>>()
            .await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::log_metrics::{Histogram, PointValue, Retention};
    use axum::{extract::Query, http::StatusCode, routing::post, Router};
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };

    fn points(org: &str) -> Vec<Point> {
        let count = Point {
            event_id: String::new(),
            org: org.into(),
            metric: "logs.checkout".into(),
            description: "Log count".into(),
            retention: Retention::default(),
            tags: vec![],
            time_ms: 1_793_455_200_000,
            value: PointValue::Count(2),
        };
        let mut histogram = Histogram::default();
        histogram.add(12.0);
        let dist = Point {
            metric: "logs.duration".into(),
            value: PointValue::Distribution(histogram),
            ..count.clone()
        };
        vec![count, dist]
    }

    #[tokio::test]
    async fn replay_and_lost_export_acknowledgements_preserve_counts_distributions_and_rollups() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let Some((_, backend)) = crate::test_support::replicas(1).await else { return };
        let Some((_, workers)) = crate::test_support::replicas_with(2, |cfg| cfg.clickhouse_url = url.clone()).await else {
            return;
        };
        let st = &backend[0];
        let org = crate::test_support::new_org(st, "metric-identity").await;
        let other = crate::test_support::new_org(st, "metric-identity-other").await;
        let backend_state = st.clone();
        let lost = Arc::new(AtomicUsize::new(0));
        let server = tokio::spawn(async move {
            let app = Router::new().route(
                "/",
                post(move |Query(q): Query<std::collections::HashMap<String, String>>, body: axum::body::Bytes| {
                    let st = backend_state.clone();
                    let lost = lost.clone();
                    async move {
                        let table = if q["query"].contains("metrics_log_ingest_sum") {
                            "otel.metrics_log_ingest_sum"
                        } else {
                            "otel.metrics_log_ingest_exponential_histogram"
                        };
                        crate::logproc::insert_rows(&st, table, body.to_vec()).await.unwrap();
                        if lost.fetch_add(1, Ordering::SeqCst) == 0 {
                            (StatusCode::SERVICE_UNAVAILABLE, "response lost after durable write")
                        } else {
                            (StatusCode::OK, "")
                        }
                    }
                }),
            );
            axum::serve(listener, app).await.unwrap();
        });
        for table in ["metrics_log_samples", "metrics_log_dist_samples"] {
            let sql = format!("SYSTEM STOP MERGES ${{ON_CLUSTER}} otel.{table}${{LOCAL}}");
            for sql in oc_store::render(&sql, st.tables.cluster.as_deref()) {
                st.clickhouse.query(&sql).execute().await.unwrap();
            }
        }
        let at = (crate::routes::metrics::now_ms() - 15 * 60_000).div_euclid(300_000) * 300_000 + 10_000;
        let event = |org: &str, id: &str, value: f64| {
            let mut points = points(org);
            let mut histogram = Histogram::default();
            histogram.add(value);
            points[0].value = PointValue::Count(1);
            points[1].value = PointValue::Distribution(histogram);
            for point in &mut points {
                point.event_id = id.to_string();
                point.time_ms = at;
            }
            points
        };
        let initial = [event(&org, "event-a", 10.0), event(&org, "event-b", 20.0)].concat();
        enqueue(st, initial).await.unwrap();
        assert!(export_one(&workers[0], &org).await.is_err(), "the first insert committed, but its acknowledgement was lost");
        assert!(export_one(&workers[1], &org).await.unwrap(), "a replacement worker repeats that committed insert");
        let regrouped = [event(&org, "event-b", 20.0), event(&org, "event-c", -5.0), event(&org, "event-a", 10.0)].concat();
        enqueue(st, regrouped).await.unwrap();
        export_one(&workers[0], &org).await.unwrap();
        enqueue(st, event(&other, "event-a", 10.0)).await.unwrap();
        export_one(&workers[1], &other).await.unwrap();
        assert_eq!(
            st.ch(&org)
                .query("SELECT sum(Value) FROM otel.metrics_samples_read WHERE OrgId = {oc_org:String}")
                .fetch_one::<f64>()
                .await
                .unwrap(),
            3.0
        );
        assert_eq!(
            st.ch(&other)
                .query("SELECT sum(Value) FROM otel.metrics_samples_read WHERE OrgId = {oc_org:String}")
                .fetch_one::<f64>()
                .await
                .unwrap(),
            1.0
        );
        #[derive(clickhouse::Row, serde::Deserialize)]
        struct Distribution {
            n: u64,
            sum: f64,
            min: f64,
            max: f64,
            positive: u64,
            negative: u64,
            offset: i32,
            buckets: Vec<u64>,
        }
        let distribution = st.ch(&org).query("SELECT Count AS n, Sum AS sum, Min AS min, Max AS max, arraySum(PositiveBucketCounts) AS positive, arraySum(NegativeBucketCounts) AS negative, PositiveOffset AS offset, PositiveBucketCounts AS buckets FROM otel.metrics_dist_samples_read WHERE OrgId = {oc_org:String}")
            .fetch_one::<Distribution>().await.unwrap();
        assert_eq!(
            (distribution.n, distribution.sum, distribution.min, distribution.max, distribution.positive, distribution.negative),
            (3, 25.0, -5.0, 20.0, 2, 1)
        );
        assert_eq!(distribution.offset, crate::log_metrics::bucket_index(10.0));
        assert_eq!(distribution.buckets.len(), 65, "histogram bucket indexes must be aligned, not summed by array position");
        assert_eq!((distribution.buckets[0], distribution.buckets[64]), (1, 1));
        assert!(
            st.ch(&org)
                .query("SELECT count() FROM otel.metrics_arrivals WHERE OrgId = {oc_org:String}")
                .fetch_one::<u64>()
                .await
                .unwrap()
                > 0
        );
        let physical = st
            .ch(&org)
            .query("SELECT count() FROM otel.metrics_log_samples WHERE OrgId = {oc_org:String}")
            .fetch_one::<u64>()
            .await
            .unwrap();
        assert!(physical > 3, "the test leaves physical duplicates: correctness must not wait for merges");
        let range = crate::routes::logs::TimeRange {
            from: at.div_euclid(300_000) * 300_000,
            to: at.div_euclid(300_000) * 300_000 + 300_000,
        };
        let query = oc_query::parse_metric_query("sum:logs.checkout{*}").unwrap();
        let (groups, step) = crate::routes::metrics::evaluate_query_stepped(st, &org, &query, range, 1).await.unwrap();
        assert_eq!(
            groups.iter().flat_map(|g| &g.points).map(|p| p.1).sum::<f64>() * step as f64 / 1000.0,
            3.0,
            "the ordinary metrics API reads the canonical contributions"
        );
        for _ in 0..2 {
            crate::metric_rollups::roll(&st.ch(&org), st.tables.cluster.is_some(), crate::metric_rollups::Tier::FiveMinutes, range.from, range.to)
                .await
                .unwrap();
            assert_eq!(
                st.ch(&org)
                    .query("SELECT sum(Sum) FROM otel.metrics_rollup_5m FINAL WHERE OrgId = {oc_org:String}")
                    .fetch_one::<f64>()
                    .await
                    .unwrap(),
                3.0
            );
            assert_eq!(
                st.ch(&org)
                    .query("SELECT sum(CountTotal) FROM otel.metrics_dist_rollup_5m FINAL WHERE OrgId = {oc_org:String}")
                    .fetch_one::<u64>()
                    .await
                    .unwrap(),
                3
            );
        }
        assert_eq!(
            st.ch(&org)
                .query("SELECT uniqExact(SeriesId) FROM otel.metrics_series WHERE OrgId = {oc_org:String} AND MetricName = 'logs.checkout'")
                .fetch_one::<u64>()
                .await
                .unwrap(),
            1
        );
        let legacy = crate::test_support::new_org(st, "metric-legacy-payload").await;
        let mut legacy_points = points(&legacy);
        for point in &mut legacy_points {
            point.time_ms = at;
        }
        enqueue(st, legacy_points).await.unwrap();
        export_one(&workers[0], &legacy).await.unwrap();
        assert_eq!(
            st.ch(&legacy)
                .query("SELECT sum(Value) FROM otel.metrics_samples WHERE OrgId = {oc_org:String}")
                .fetch_one::<f64>()
                .await
                .unwrap(),
            2.0,
            "older queued aggregates keep their ordinary storage path"
        );
        for table in ["metrics_log_samples", "metrics_log_dist_samples"] {
            let sql = format!("SYSTEM START MERGES ${{ON_CLUSTER}} otel.{table}${{LOCAL}}");
            for sql in oc_store::render(&sql, st.tables.cluster.as_deref()) {
                st.clickhouse.query(&sql).execute().await.unwrap();
            }
        }
        server.abort();
    }

    #[tokio::test]
    async fn retry_after_restart_keeps_only_the_failed_sink() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let Some((_, states)) = crate::test_support::replicas_with(2, |cfg| cfg.clickhouse_url = url.clone()).await else {
            return;
        };
        let org = crate::test_support::new_org(&states[0], "outbox-partial").await;
        let sums = Arc::new(AtomicUsize::new(0));
        let hist = Arc::new(AtomicUsize::new(0));
        let (sum_calls, hist_calls) = (sums.clone(), hist.clone());
        let server = tokio::spawn(async move {
            let app = Router::new().route(
                "/",
                post(move |Query(q): Query<std::collections::HashMap<String, String>>| {
                    let (sums, hist) = (sum_calls.clone(), hist_calls.clone());
                    async move {
                        if q["query"].contains("metrics_log_ingest_sum") {
                            sums.fetch_add(1, Ordering::SeqCst);
                            (StatusCode::OK, "")
                        } else if hist.fetch_add(1, Ordering::SeqCst) == 0 {
                            (StatusCode::SERVICE_UNAVAILABLE, "unavailable")
                        } else {
                            (StatusCode::OK, "")
                        }
                    }
                }),
            );
            axum::serve(listener, app).await.unwrap();
        });
        enqueue(&states[0], points(&org)).await.unwrap();
        assert!(export_one(&states[0], &org).await.is_err());
        let pending: (i32, i32) = sqlx::query_as("SELECT octet_length(sums), octet_length(histograms) FROM log_metric_outbox WHERE org_id = $1")
            .bind(&org)
            .fetch_one(&states[0].postgres)
            .await
            .unwrap();
        assert_eq!(pending.0, 0);
        assert!(pending.1 > 0);
        assert!(export_one(&states[1], &org).await.unwrap(), "another replica recovers using only durable payloads");
        assert!(!export_one(&states[0], &org).await.unwrap());
        assert_eq!(sums.load(Ordering::SeqCst), 1);
        assert_eq!(hist.load(Ordering::SeqCst), 2);
        server.abort();
    }

    #[tokio::test]
    async fn concurrent_workers_skip_claimed_rows_and_cancellation_retains_payload() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let Some((_, states)) = crate::test_support::replicas_with(2, |cfg| cfg.clickhouse_url = url.clone()).await else {
            return;
        };
        let org = crate::test_support::new_org(&states[0], "outbox-cancel").await;
        let arrived = Arc::new(tokio::sync::Notify::new());
        let calls = Arc::new(AtomicUsize::new(0));
        let notified = arrived.clone();
        let server = tokio::spawn(async move {
            let app = Router::new().route(
                "/",
                post(move || {
                    let arrived = notified.clone();
                    let calls = calls.clone();
                    async move {
                        if calls.fetch_add(1, Ordering::SeqCst) == 0 {
                            arrived.notify_one();
                            std::future::pending::<()>().await;
                        }
                        ""
                    }
                }),
            );
            axum::serve(listener, app).await.unwrap();
        });
        enqueue(&states[0], points(&org)).await.unwrap();
        let st = states[0].clone();
        let worker_org = org.clone();
        let worker = tokio::spawn(async move { export_one(&st, &worker_org).await });
        tokio::time::timeout(Duration::from_secs(5), arrived.notified()).await.unwrap();
        assert!(!export_one(&states[1], &org).await.unwrap());
        worker.abort();
        assert!(worker.await.unwrap_err().is_cancelled());
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if export_one(&states[1], &org).await.unwrap() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        assert!(!export_one(&states[0], &org).await.unwrap());
        server.abort();
    }

    #[tokio::test]
    async fn bounded_spool_rejects_atomically_and_isolates_organizations() {
        let Some((_, states)) = crate::test_support::replicas(1).await else { return };
        let st = &states[0];
        let first = crate::test_support::new_org(st, "outbox-cap-a").await;
        let second = crate::test_support::new_org(st, "outbox-cap-b").await;
        enqueue_with_limit(st, points(&second), 10_000, 1).await.unwrap();
        let both = [points(&first), points(&second)].concat();
        assert!(enqueue_with_limit(st, both, 10_000, 1).await.is_err());
        let n: i64 = sqlx::query_scalar("SELECT count(*) FROM log_metric_outbox WHERE org_id = $1")
            .bind(&first)
            .fetch_one(&st.postgres)
            .await
            .unwrap();
        assert_eq!(n, 0, "failure rolls back the earlier organization's insert too");
        assert!(enqueue_with_limit(st, points(&first), 1, 1).await.is_err(), "byte cap applies to a new batch too");
        enqueue_with_limit(st, points(&first), 10_000, 1).await.unwrap();
        assert!(!export_one(st, &crate::test_support::org("other-outbox")).await.unwrap());
    }
}
