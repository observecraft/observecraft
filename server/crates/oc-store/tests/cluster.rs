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
    let count = |sql: &'static str| {
        let ch = ch.clone();
        async move { ch.query(sql).fetch_one::<Count>().await.expect(sql).n }
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
    // Retention changes reach every shard's local table.
    oc_store::apply_retention(&cfg, oc_store::RetentionDays { traces: 7, traces_max: 30, metrics: 30, ci: 30, raw_logs_hours: 72 }).await.expect("retention");
}
