//! Applies the schema to a real ClickHouse cluster and checks that sharded writes land on the
//! shards and read back through the Distributed tables. Skipped unless OC_TEST_CLICKHOUSE_CLUSTER_URL
//! points at a node of a cluster named by OC_TEST_CLICKHOUSE_CLUSTER (default `oc`) with Keeper.

use oc_store::{migrate_clickhouse, ClickHouseConfig};

#[derive(clickhouse::Row, serde::Deserialize)]
struct Count {
    n: u64,
}

#[tokio::test]
async fn schema_on_a_cluster() {
    let Ok(url) = std::env::var("OC_TEST_CLICKHOUSE_CLUSTER_URL") else {
        eprintln!("OC_TEST_CLICKHOUSE_CLUSTER_URL unset: skipping");
        return;
    };
    let cluster = std::env::var("OC_TEST_CLICKHOUSE_CLUSTER").unwrap_or_else(|_| "oc".into());
    let cfg = ClickHouseConfig { url, user: "default".into(), password: String::new(), database: "oc".into(), cluster: Some(cluster) };
    migrate_clickhouse(&cfg, false).await.expect("migrate");
    // Idempotent: a second run (another API replica, an upgrade) is a no-op.
    migrate_clickhouse(&cfg, false).await.expect("migrate again");

    let ch = clickhouse::Client::default().with_url(&cfg.url).with_user("default").with_option("insert_distributed_sync", "1");
    let count = |sql: &str| {
        let (ch, sql) = (ch.clone(), sql.to_string());
        async move { ch.query(&sql).fetch_one::<Count>().await.expect(&sql).n }
    };
    // Spans of two orgs, 200 traces: spread over both shards by TraceId.
    ch.query(
        "INSERT INTO otel.otel_traces (Timestamp, TraceId, SpanId, ServiceName, SpanName, ResourceAttributes) \
         SELECT now64(9), lower(hex(cityHash64(number))) || lower(hex(cityHash64(number + 1))), toString(number), 'checkout', 'GET /', \
                map('oc.org_id', if(number % 2 = 0, 'acme', 'globex')) FROM numbers(200)",
    )
    .execute()
    .await
    .expect("insert spans");
    assert_eq!(count("SELECT count() AS n FROM otel.otel_traces").await, 200);
    assert_eq!(count("SELECT count() AS n FROM otel.otel_traces WHERE OrgId = 'acme'").await, 100);
    let shards = count("SELECT uniqExact(_shard_num) AS n FROM otel.otel_traces").await;
    assert_eq!(shards, 2, "spans are spread over both shards");
    // The trace-id lookup is filled by the shard-local materialized view, with the org.
    assert_eq!(count("SELECT count() AS n FROM otel.otel_traces_trace_id_ts WHERE OrgId = 'globex'").await, 100);
    // Metrics: 100 series, 3 points each, inserted through the ingest table. A series' samples,
    // series row and rollups live on one shard, and the series spread over both.
    ch.query(
        "INSERT INTO otel.metrics_ingest_gauge (ResourceAttributes, MetricName, Attributes, TimeUnix, Value) \
         SELECT map('oc.org_id', 'acme', 'host.name', concat('i-0a1b2c3d4e5f6', toString(number % 100))), 'system.cpu.utilization', \
                map('state', 'user'), now64(9) - toIntervalSecond(intDiv(number, 100) * 10), number FROM numbers(300)",
    )
    .execute()
    .await
    .expect("insert metrics");
    assert_eq!(count("SELECT count() AS n FROM otel.metrics_samples WHERE OrgId = 'acme'").await, 300);
    assert_eq!(count("SELECT uniqExact(SeriesId) AS n FROM otel.metrics_series WHERE OrgId = 'acme'").await, 100);
    assert_eq!(count("SELECT uniqExact(_shard_num) AS n FROM otel.metrics_samples").await, 2, "series are spread over both shards");
    // A histogram's samples and rollups too.
    ch.query(
        "INSERT INTO otel.metrics_ingest_histogram (ResourceAttributes, MetricName, Attributes, TimeUnix, Count, Sum, BucketCounts, ExplicitBounds, AggregationTemporality) \
         SELECT map('oc.org_id', 'acme', 'host.name', concat('i-0a1b2c3d4e5f6', toString(number % 100))), 'http.server.duration', map('route', '/cart'), \
                now64(9) - toIntervalSecond(intDiv(number, 100) * 10), number, number * 2.5, [number, number], [100.], 2 FROM numbers(300)",
    )
    .execute()
    .await
    .expect("insert histograms");
    assert_eq!(count("SELECT count() AS n FROM otel.metrics_dist_samples WHERE OrgId = 'acme'").await, 300);
    // The tag index rows of a series are on its shard (the series view writes them shard-locally).
    assert!(count("SELECT count() AS n FROM otel.metrics_tags WHERE OrgId = 'acme'").await > 0);
    let apart = count(
        "SELECT count() AS n FROM (SELECT SeriesId, any(_shard_num) AS shard FROM otel.metrics_tags GROUP BY SeriesId) t \
         INNER JOIN (SELECT SeriesId, any(_shard_num) AS shard FROM otel.metrics_series GROUP BY SeriesId) r USING SeriesId WHERE t.shard != r.shard",
    )
    .await;
    assert_eq!(apart, 0, "a series' index rows and its series row are on different shards");
    // Its values, through the Distributed table, by (org, key).
    for _ in 0..30 {
        if count("SELECT count() AS n FROM otel.metrics_tag_values WHERE OrgId = 'acme' AND Key = 'host.name'").await == 100 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
    assert_eq!(count("SELECT count() AS n FROM otel.metrics_tag_values FINAL WHERE OrgId = 'acme' AND Key = 'host.name'").await, 100);
    for table in ["metrics_samples", "metrics_series", "metrics_rollup_5m", "metrics_rollup_1h", "metrics_dist_samples", "metrics_dist_rollup_5m", "metrics_dist_rollup_1h"] {
        let split = count(&format!("SELECT count() AS n FROM (SELECT SeriesId FROM otel.{table} GROUP BY SeriesId HAVING uniqExact(_shard_num) > 1)")).await;
        assert_eq!(split, 0, "a series is on more than one shard in {table}");
    }
    let apart = count(
        "SELECT count() AS n FROM (SELECT SeriesId, any(_shard_num) AS shard FROM otel.metrics_samples GROUP BY SeriesId) s \
         INNER JOIN (SELECT SeriesId, any(_shard_num) AS shard FROM otel.metrics_series GROUP BY SeriesId) r USING SeriesId WHERE s.shard != r.shard",
    )
    .await;
    assert_eq!(apart, 0, "a series' samples and its labels are on different shards");
    // Logs land on both shards; each shard's views fill its error groups and facet counts, read
    // back whole through the Distributed tables.
    ch.query(
        "INSERT INTO oc.logs (org_id, timestamp, status, service, host, message, tags) \
         SELECT 'acme', now64(3), if(number % 2 = 0, 'error', 'info'), 'billing', 'i-0a1b2c3d4e5f60001', 'ValueError: order 7 not found', map('env', 'prod') FROM numbers(200)",
    )
    .execute()
    .await
    .expect("insert logs");
    assert_eq!(count("SELECT toUInt64(sum(n)) AS n FROM oc.error_groups_1m WHERE org_id = 'acme'").await, 100);
    assert_eq!(count("SELECT toUInt64(sum(n)) AS n FROM oc.log_facets_1m WHERE org_id = 'acme' AND env = 'prod'").await, 200);
    // Kubernetes objects: an object's inventory row is on the shard of its snapshots and full body.
    ch.query(
        "INSERT INTO otel.otel_k8s_objects (Timestamp, Body, ResourceAttributes) \
         SELECT now64(9), concat('{\"kind\":\"Pod\",\"metadata\":{\"name\":\"storefront-api-', toString(number), '\",\"namespace\":\"storefront\",\"uid\":\"pod-', toString(number), '\"}}'), \
                map('oc.org_id', 'acme', 'k8s.cluster.name', 'shop-prod') FROM numbers(100)",
    )
    .execute()
    .await
    .expect("insert kubernetes objects");
    assert_eq!(count("SELECT count() AS n FROM otel.k8s_inventory FINAL WHERE OrgId = 'acme' AND Kind = 'Pod'").await, 100);
    assert_eq!(count("SELECT uniqExact(_shard_num) AS n FROM otel.k8s_inventory").await, 2, "objects are spread over both shards");
    let apart = count(
        "SELECT count() AS n FROM (SELECT Uid, any(_shard_num) AS shard FROM otel.k8s_inventory GROUP BY Uid) i \
         INNER JOIN (SELECT Uid, any(_shard_num) AS shard FROM otel.k8s_resources GROUP BY Uid) r USING Uid WHERE i.shard != r.shard",
    )
    .await;
    assert_eq!(apart, 0, "an object's inventory row and its body are on different shards");
    // Retention changes reach every shard's local table.
    oc_store::apply_retention(&cfg, oc_store::RetentionDays { traces: 7, traces_max: 30, metrics: 30, metrics_5m: 90, metrics_1h: 395, ci: 30, raw_logs_hours: 72, metrics_cold_after_days: 0 }).await.expect("retention");
}
