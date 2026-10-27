//! Metrics storage (ADR-0001): rows the collector inserts into the `otel.metrics_ingest_*` tables
//! land as series, samples and rollups. Skipped unless OC_TEST_CLICKHOUSE_URL points at a single
//! ClickHouse server (user `default`, or OC_TEST_CLICKHOUSE_USER / OC_TEST_CLICKHOUSE_PASSWORD).

use oc_store::{migrate_clickhouse, ClickHouseConfig};

#[derive(clickhouse::Row, serde::Deserialize)]
struct Count {
    n: u64,
}

#[derive(clickhouse::Row, serde::Deserialize, Debug, PartialEq)]
struct Rollup {
    min: f64,
    max: f64,
    sum: f64,
    count: u64,
    first: f64,
    last: f64,
}

#[tokio::test]
async fn ingested_points_become_series_samples_and_rollups() {
    let Ok(url) = std::env::var("OC_TEST_CLICKHOUSE_URL") else {
        eprintln!("OC_TEST_CLICKHOUSE_URL unset: skipping");
        return;
    };
    let user = std::env::var("OC_TEST_CLICKHOUSE_USER").unwrap_or_else(|_| "default".into());
    let password = std::env::var("OC_TEST_CLICKHOUSE_PASSWORD").unwrap_or_default();
    let cfg = ClickHouseConfig { url: url.clone(), user: user.clone(), password: password.clone(), database: "oc".into(), cluster: None };
    migrate_clickhouse(&cfg, false).await.expect("migrate");
    migrate_clickhouse(&cfg, false).await.expect("migrate again");

    let ch = clickhouse::Client::default().with_url(&url).with_user(&user).with_password(&password);
    let org = format!("metrics-test-{}", std::process::id());
    let count = |sql: String| {
        let ch = ch.clone();
        async move { ch.query(&sql).fetch_one::<Count>().await.unwrap_or_else(|e| panic!("{sql}: {e}")).n }
    };

    // One gauge series sent with its labels in two different orders, 3 points in one 5-minute
    // bucket, plus a second series.
    ch.query(&format!(
        "INSERT INTO otel.metrics_ingest_gauge (ResourceAttributes, ServiceName, MetricName, MetricUnit, Attributes, TimeUnix, Value) VALUES \
         (map('oc.org_id', '{org}', 'k8s.pod.name', 'storefront-7d9', 'host.name', 'i-0a1b2c3d4e5f60001'), 'storefront-api', 'system.cpu.utilization', '1', map('state', 'user', 'cpu', '0'), '2026-10-27 10:00:10', 0.25), \
         (map('host.name', 'i-0a1b2c3d4e5f60001', 'oc.org_id', '{org}', 'k8s.pod.name', 'storefront-7d9'), 'storefront-api', 'system.cpu.utilization', '1', map('cpu', '0', 'state', 'user'), '2026-10-27 10:01:10', 0.75), \
         (map('oc.org_id', '{org}', 'k8s.pod.name', 'storefront-7d9', 'host.name', 'i-0a1b2c3d4e5f60001'), 'storefront-api', 'system.cpu.utilization', '1', map('state', 'user', 'cpu', '0'), '2026-10-27 10:02:10', 0.5), \
         (map('oc.org_id', '{org}', 'host.name', 'i-0a1b2c3d4e5f60002'), 'catalog-service', 'system.cpu.utilization', '1', map('state', 'user', 'cpu', '0'), '2026-10-27 10:00:10', 0.1)"
    ))
    .execute()
    .await
    .expect("insert gauge points");

    // A cumulative, monotonic counter.
    ch.query(&format!(
        "INSERT INTO otel.metrics_ingest_sum (ResourceAttributes, ServiceName, MetricName, Attributes, TimeUnix, Value, AggregationTemporality, IsMonotonic) VALUES \
         (map('oc.org_id', '{org}'), 'order-service', 'http.server.requests', map('route', '/cart'), '2026-10-27 10:00:00', 10, 2, true), \
         (map('oc.org_id', '{org}'), 'order-service', 'http.server.requests', map('route', '/cart'), '2026-10-27 10:00:30', 25, 2, true)"
    ))
    .execute()
    .await
    .expect("insert sum points");

    // A gauge and a counter that share a name and labels are two series.
    ch.query(&format!(
        "INSERT INTO otel.metrics_ingest_gauge (ResourceAttributes, MetricName, Attributes, TimeUnix, Value) VALUES \
         (map('oc.org_id', '{org}'), 'queue.depth', map('queue', 'orders'), '2026-10-27 10:00:00', 3)"
    ))
    .execute()
    .await
    .expect("insert gauge with a shared name");
    ch.query(&format!(
        "INSERT INTO otel.metrics_ingest_sum (ResourceAttributes, MetricName, Attributes, TimeUnix, Value, AggregationTemporality, IsMonotonic) VALUES \
         (map('oc.org_id', '{org}'), 'queue.depth', map('queue', 'orders'), '2026-10-27 10:00:00', 7, 1, false)"
    ))
    .execute()
    .await
    .expect("insert sum with a shared name");

    let o = format!("OrgId = '{org}'");
    assert_eq!(count(format!("SELECT uniqExact(SeriesId) AS n FROM otel.metrics_series WHERE {o} AND MetricName = 'queue.depth'")).await, 2);
    assert_eq!(count(format!("SELECT uniqExact(MetricType) AS n FROM otel.metrics_series WHERE {o} AND MetricName = 'queue.depth'")).await, 2);
    // The ingest tables store nothing.
    assert_eq!(count("SELECT count() AS n FROM otel.metrics_ingest_gauge".to_string()).await, 0);
    // Same labels in another order are the same series.
    assert_eq!(count(format!("SELECT uniqExact(SeriesId) AS n FROM otel.metrics_series WHERE {o} AND MetricName = 'system.cpu.utilization'")).await, 2);
    assert_eq!(count(format!("SELECT uniqExact(SeriesId) AS n FROM otel.metrics_samples WHERE {o} AND MetricName = 'system.cpu.utilization'")).await, 2);
    assert_eq!(count(format!("SELECT count() AS n FROM otel.metrics_samples WHERE {o}")).await, 8, "every point is a sample");
    // Series keep their labels (sorted), type and temporality.
    assert_eq!(
        count(format!(
            "SELECT count() AS n FROM otel.metrics_series FINAL WHERE {o} AND MetricName = 'system.cpu.utilization' AND ServiceName = 'storefront-api' \
             AND ResourceAttributes['k8s.pod.name'] = 'storefront-7d9' AND Attributes['state'] = 'user' AND MetricType = 'gauge' AND MetricUnit = '1' \
             AND mapKeys(Attributes) = ['cpu', 'state']"
        ))
        .await,
        1
    );
    assert_eq!(
        count(format!("SELECT count() AS n FROM otel.metrics_series FINAL WHERE {o} AND MetricName = 'http.server.requests' AND MetricType = 'sum' AND AggregationTemporality = 2 AND IsMonotonic")).await,
        1
    );
    // The org comes from the resource, as for every other telemetry table.
    assert_eq!(count(format!("SELECT count() AS n FROM otel.metrics_samples WHERE {o} AND MetricName = 'http.server.requests'")).await, 2);

    // Rollups hold the same aggregates as the samples, at 5 minutes and 1 hour.
    for table in ["metrics_rollup_5m", "metrics_rollup_1h"] {
        let r = ch
            .query(&format!(
                "SELECT min(Min) AS min, max(Max) AS max, sum(Sum) AS sum, sum(Count) AS count, argMinMerge(First) AS first, argMaxMerge(Last) AS last \
                 FROM otel.{table} WHERE {o} AND MetricName = 'system.cpu.utilization' AND SeriesId = \
                 (SELECT any(SeriesId) FROM otel.metrics_series WHERE {o} AND ServiceName = 'storefront-api')"
            ))
            .fetch_one::<Rollup>()
            .await
            .unwrap_or_else(|e| panic!("{table}: {e}"));
        assert_eq!(r, Rollup { min: 0.25, max: 0.75, sum: 1.5, count: 3, first: 0.25, last: 0.5 }, "{table}");
    }
}
