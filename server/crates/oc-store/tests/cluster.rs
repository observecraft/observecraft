//! Applies the schema to a real ClickHouse cluster and checks that sharded writes land on the
//! shards, read back through the Distributed tables and reach every replica. Skipped unless
//! OC_TEST_CLICKHOUSE_CLUSTER_URL points at a node of a cluster named by OC_TEST_CLICKHOUSE_CLUSTER
//! (default `oc`) with Keeper and two shards, e.g. `make cluster-up` (2 shards x 2 replicas):
//! `OC_TEST_CLICKHOUSE_CLUSTER_URL=http://localhost:18161 cargo test -p oc-store --test cluster`.
//! Each run writes its own organizations, so it can run again on the same cluster.

use oc_store::{migrate_clickhouse, ClickHouseConfig};

#[derive(clickhouse::Row, serde::Deserialize)]
struct Count {
    n: u64,
}

#[derive(clickhouse::Row, serde::Deserialize)]
struct ReplicaCount {
    shard: String,
    replica: String,
    n: u64,
}

#[tokio::test]
async fn schema_on_a_cluster() {
    let Ok(url) = std::env::var("OC_TEST_CLICKHOUSE_CLUSTER_URL") else {
        eprintln!("OC_TEST_CLICKHOUSE_CLUSTER_URL unset: skipping");
        return;
    };
    let cluster = std::env::var("OC_TEST_CLICKHOUSE_CLUSTER").unwrap_or_else(|_| "oc".into());
    let cfg = ClickHouseConfig { url, user: "default".into(), password: String::new(), database: "oc".into(), cluster: Some(cluster.clone()) };
    migrate_clickhouse(&cfg, false).await.expect("migrate");
    // Idempotent: a second run (another API replica, an upgrade) is a no-op.
    migrate_clickhouse(&cfg, false).await.expect("migrate again");

    let run = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis();
    let (acme, globex) = (format!("acme-{run}"), format!("globex-{run}"));
    // Inserts wait for the shards; reads and inserts go to the first replica of each shard. The
    // other replicas fetch the parts in the background (checked at the end).
    let ch = clickhouse::Client::default()
        .with_url(&cfg.url)
        .with_user("default")
        .with_option("insert_distributed_sync", "1")
        .with_option("load_balancing", "in_order")
        .with_option("param_org", &acme)
        .with_option("param_other", &globex);
    let count = |sql: &str| {
        let (ch, sql) = (ch.clone(), sql.to_string());
        async move { ch.query(&sql).fetch_one::<Count>().await.expect(&sql).n }
    };
    // Spans of two orgs, 200 traces: spread over both shards by TraceId.
    ch.query(
        "INSERT INTO otel.otel_traces (Timestamp, TraceId, SpanId, ServiceName, SpanName, ResourceAttributes) \
         SELECT now64(9), lower(hex(cityHash64(number, {org:String}))) || lower(hex(cityHash64(number + 1))), toString(number), 'checkout', 'GET /', \
                map('oc.org_id', if(number % 2 = 0, {org:String}, {other:String})) FROM numbers(200)",
    )
    .execute()
    .await
    .expect("insert spans");
    assert_eq!(count("SELECT count() AS n FROM otel.otel_traces WHERE OrgId IN ({org:String}, {other:String})").await, 200);
    assert_eq!(count("SELECT count() AS n FROM otel.otel_traces WHERE OrgId = {org:String}").await, 100);
    let shards = count("SELECT uniqExact(_shard_num) AS n FROM otel.otel_traces WHERE OrgId = {org:String}").await;
    assert_eq!(shards, 2, "spans are spread over both shards");
    // The trace-id lookup is filled by the shard-local materialized view, with the org.
    assert_eq!(count("SELECT count() AS n FROM otel.otel_traces_trace_id_ts WHERE OrgId = {other:String}").await, 100);
    // Metrics: 100 series, 3 points each, inserted through the ingest table. A series' samples,
    // series row and rollups live on one shard, and the series spread over both.
    ch.query(
        "INSERT INTO otel.metrics_ingest_gauge (ResourceAttributes, MetricName, Attributes, TimeUnix, Value) \
         SELECT map('oc.org_id', {org:String}, 'host.name', concat('i-0a1b2c3d4e5f6', toString(number % 100))), 'system.cpu.utilization', \
                map('state', 'user'), now64(9) - toIntervalSecond(intDiv(number, 100) * 10), number FROM numbers(300)",
    )
    .execute()
    .await
    .expect("insert metrics");
    assert_eq!(count("SELECT count() AS n FROM otel.metrics_samples WHERE OrgId = {org:String}").await, 300);
    assert_eq!(count("SELECT uniqExact(SeriesId) AS n FROM otel.metrics_series WHERE OrgId = {org:String}").await, 100);
    assert_eq!(count("SELECT uniqExact(_shard_num) AS n FROM otel.metrics_samples WHERE OrgId = {org:String}").await, 2, "series are spread over both shards");
    // A histogram's samples and rollups too.
    ch.query(
        "INSERT INTO otel.metrics_ingest_histogram (ResourceAttributes, MetricName, Attributes, TimeUnix, Count, Sum, BucketCounts, ExplicitBounds, AggregationTemporality) \
         SELECT map('oc.org_id', {org:String}, 'host.name', concat('i-0a1b2c3d4e5f6', toString(number % 100))), 'http.server.duration', map('route', '/cart'), \
                now64(9) - toIntervalSecond(intDiv(number, 100) * 10), number, number * 2.5, [number, number], [100.], 2 FROM numbers(300)",
    )
    .execute()
    .await
    .expect("insert histograms");
    assert_eq!(count("SELECT count() AS n FROM otel.metrics_dist_samples WHERE OrgId = {org:String}").await, 300);
    // The tag index rows of a series are on its shard (the series view writes them shard-locally).
    assert!(count("SELECT count() AS n FROM otel.metrics_tags WHERE OrgId = {org:String}").await > 0);
    let apart = count(
        "SELECT count() AS n FROM (SELECT SeriesId, any(_shard_num) AS shard FROM otel.metrics_tags WHERE OrgId = {org:String} GROUP BY SeriesId) t \
         INNER JOIN (SELECT SeriesId, any(_shard_num) AS shard FROM otel.metrics_series WHERE OrgId = {org:String} GROUP BY SeriesId) r USING SeriesId WHERE t.shard != r.shard",
    )
    .await;
    assert_eq!(apart, 0, "a series' index rows and its series row are on different shards");
    // Its values, through the Distributed table, by (org, key).
    for _ in 0..30 {
        if count("SELECT count() AS n FROM otel.metrics_tag_values WHERE OrgId = {org:String} AND Key = 'host.name'").await == 100 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
    assert_eq!(count("SELECT count() AS n FROM otel.metrics_tag_values FINAL WHERE OrgId = {org:String} AND Key = 'host.name'").await, 100);
    for table in ["metrics_samples", "metrics_series", "metrics_rollup_5m", "metrics_rollup_1h", "metrics_dist_samples", "metrics_dist_rollup_5m", "metrics_dist_rollup_1h"] {
        let split = count(&format!("SELECT count() AS n FROM (SELECT SeriesId FROM otel.{table} WHERE OrgId = {{org:String}} GROUP BY SeriesId HAVING uniqExact(_shard_num) > 1)")).await;
        assert_eq!(split, 0, "a series is on more than one shard in {table}");
    }
    let apart = count(
        "SELECT count() AS n FROM (SELECT SeriesId, any(_shard_num) AS shard FROM otel.metrics_samples WHERE OrgId = {org:String} GROUP BY SeriesId) s \
         INNER JOIN (SELECT SeriesId, any(_shard_num) AS shard FROM otel.metrics_series WHERE OrgId = {org:String} GROUP BY SeriesId) r USING SeriesId WHERE s.shard != r.shard",
    )
    .await;
    assert_eq!(apart, 0, "a series' samples and its labels are on different shards");
    // The server's client joins Distributed tables and filters them with subqueries of other
    // Distributed tables (series → samples for hosts and pods, the tag index), which ClickHouse
    // refuses by default ("Double-distributed IN/JOIN subqueries is denied").
    let server = oc_store::clickhouse(&cfg).with_option("load_balancing", "in_order").with_option("param_org", &acme);
    let joined = server
        .query(
            "SELECT count() AS n FROM otel.metrics_samples AS x INNER JOIN (SELECT SeriesId, any(Attributes) AS Attributes FROM otel.metrics_series WHERE OrgId = {org:String} GROUP BY SeriesId) AS s \
             ON x.SeriesId = s.SeriesId WHERE x.OrgId = {org:String} AND s.Attributes['state'] = 'user'",
        )
        .fetch_one::<Count>()
        .await
        .expect("series → samples join");
    assert_eq!(joined.n, 300);
    let filtered = server
        .query("SELECT count() AS n FROM otel.metrics_samples WHERE OrgId = {org:String} AND SeriesId IN (SELECT SeriesId FROM otel.metrics_tags WHERE OrgId = {org:String} AND Key = 'host.name')")
        .fetch_one::<Count>()
        .await
        .expect("samples of the tag index's series");
    assert_eq!(filtered.n, 300);
    // Logs land on both shards; each shard's views fill its error groups and facet counts, read
    // back whole through the Distributed tables.
    ch.query(
        "INSERT INTO oc.logs (org_id, timestamp, status, service, host, message, tags) \
         SELECT {org:String}, now64(3), if(number % 2 = 0, 'error', 'info'), 'billing', 'i-0a1b2c3d4e5f60001', 'ValueError: order 7 not found', map('env', 'prod') FROM numbers(200)",
    )
    .execute()
    .await
    .expect("insert logs");
    assert_eq!(count("SELECT uniqExact(_shard_num) AS n FROM oc.logs WHERE org_id = {org:String}").await, 2, "logs are spread over both shards");
    assert_eq!(count("SELECT toUInt64(sum(n)) AS n FROM oc.error_groups_1m WHERE org_id = {org:String}").await, 100);
    assert_eq!(count("SELECT toUInt64(sum(n)) AS n FROM oc.log_facets_1m WHERE org_id = {org:String} AND env = 'prod'").await, 200);
    // Kubernetes objects: an object's inventory row is on the shard of its snapshots and full body.
    // Each object arrives twice (the collector resends it); the later snapshot wins in FINAL.
    for phase in ["Pending", "Running"] {
        ch.query(
            "INSERT INTO otel.otel_k8s_objects (Timestamp, Body, ResourceAttributes) \
             SELECT now64(9), concat('{\"kind\":\"Pod\",\"metadata\":{\"name\":\"storefront-api-', toString(number), '\",\"namespace\":\"storefront\",\"uid\":\"', {org:String}, '-pod-', toString(number), \
                    '\"},\"status\":{\"phase\":\"', ?, '\"}}'), map('oc.org_id', {org:String}, 'k8s.cluster.name', 'shop-prod') FROM numbers(100)",
        )
        .bind(phase)
        .execute()
        .await
        .expect("insert kubernetes objects");
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert_eq!(count("SELECT count() AS n FROM otel.k8s_inventory FINAL WHERE OrgId = {org:String} AND Kind = 'Pod'").await, 100);
    assert_eq!(count("SELECT count() AS n FROM otel.k8s_inventory FINAL WHERE OrgId = {org:String} AND Kind = 'Pod' AND Phase = 'Running'").await, 100, "the latest snapshot wins");
    assert_eq!(count("SELECT uniqExact(_shard_num) AS n FROM otel.k8s_inventory WHERE OrgId = {org:String}").await, 2, "objects are spread over both shards");
    let apart = count(
        "SELECT count() AS n FROM (SELECT Uid, any(_shard_num) AS shard FROM otel.k8s_inventory WHERE OrgId = {org:String} GROUP BY Uid) i \
         INNER JOIN (SELECT Uid, any(_shard_num) AS shard FROM otel.k8s_resources WHERE OrgId = {org:String} GROUP BY Uid) r USING Uid WHERE i.shard != r.shard",
    )
    .await;
    assert_eq!(apart, 0, "an object's inventory row and its body are on different shards");

    // Replication: once the replicas caught up, every replica of a shard holds the same rows of
    // every table, materialized views' targets included.
    let tables: Vec<(String, String)> = ch
        .query("SELECT database, table FROM system.replicas WHERE database IN ('oc', 'otel') ORDER BY database, table")
        .fetch_all()
        .await
        .expect("replicated tables");
    assert!(tables.len() > 20, "the shard-local tables are replicated: {tables:?}");
    for (db, table) in &tables {
        ch.query(&format!("SYSTEM SYNC REPLICA ON CLUSTER '{cluster}' {db}.{table}")).execute().await.unwrap_or_else(|e| panic!("sync {db}.{table}: {e}"));
    }
    for (db, table, org_column) in [
        ("otel", "otel_traces_local", "OrgId"),
        ("otel", "metrics_samples_local", "OrgId"),
        ("otel", "metrics_series_local", "OrgId"),
        ("otel", "metrics_tags_local", "OrgId"),
        ("otel", "metrics_dist_samples_local", "OrgId"),
        ("otel", "k8s_inventory_local", "OrgId"),
        ("oc", "logs_local", "org_id"),
        ("oc", "error_groups_1m_local", "org_id"),
        ("oc", "log_facets_1m_local", "org_id"),
    ] {
        let rows: Vec<ReplicaCount> = ch
            .query(&format!(
                "SELECT getMacro('shard') AS shard, getMacro('replica') AS replica, count() AS n FROM clusterAllReplicas('{cluster}', {db}.{table}) \
                 WHERE {org_column} = {{org:String}} GROUP BY shard, replica ORDER BY shard, replica"
            ))
            .fetch_all()
            .await
            .unwrap_or_else(|e| panic!("{db}.{table}: {e}"));
        let mut by_shard: std::collections::BTreeMap<&str, Vec<(&str, u64)>> = Default::default();
        for r in &rows {
            by_shard.entry(&r.shard).or_default().push((&r.replica, r.n));
        }
        assert_eq!(by_shard.len(), 2, "{db}.{table}: rows on both shards: {by_shard:?}");
        for (shard, replicas) in by_shard {
            assert!(replicas.iter().all(|(_, n)| *n == replicas[0].1 && *n > 0), "{db}.{table} shard {shard}: replicas differ {replicas:?}");
        }
    }
    // Retention changes reach every shard's local table.
    oc_store::apply_retention(&cfg, oc_store::RetentionDays { traces: 7, traces_max: 30, metrics: 30, metrics_5m: 90, metrics_1h: 395, ci: 30, raw_logs_hours: 72, metrics_cold_after_days: 0 }).await.expect("retention");
}
