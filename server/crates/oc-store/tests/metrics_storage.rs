//! Metrics storage (ADR-0001): rows the collector inserts into the `otel.metrics_ingest_*` tables
//! land as series, samples and rollups. Skipped unless OC_TEST_CLICKHOUSE_URL points at a single
//! ClickHouse server (user `default`, or OC_TEST_CLICKHOUSE_USER / OC_TEST_CLICKHOUSE_PASSWORD).

use oc_store::{migrate_clickhouse, ClickHouseConfig};

#[derive(clickhouse::Row, serde::Deserialize)]
struct Count {
    n: u64,
}

#[tokio::test]
async fn ingested_points_become_series_samples_and_rollups() {
    let Ok(url) = std::env::var("OC_TEST_CLICKHOUSE_URL") else {
        eprintln!("OC_TEST_CLICKHOUSE_URL unset: skipping");
        return;
    };
    if std::env::var_os("OC_TEST_CLICKHOUSE_CLUSTER").is_some() {
        eprintln!("a single-server test (the cluster's are in tests/cluster.rs): skipping");
        return;
    }
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

    // Rollups are the rollup job's (observecraft-server), not written per insert.
    for table in ["metrics_rollup_5m", "metrics_rollup_1h", "metrics_dist_rollup_5m", "metrics_dist_rollup_1h"] {
        assert_eq!(count(format!("SELECT count() AS n FROM otel.{table} WHERE {o}")).await, 0, "{table}");
    }
}

/// Points the intake marked as of known series (`oc.series_known = 1`) are samples like any other,
/// but write no series row; samples carry every retention tier (the rollup job reads them).
#[tokio::test]
async fn points_of_known_series_write_samples_but_no_series_rows() {
    let Ok(url) = std::env::var("OC_TEST_CLICKHOUSE_URL") else {
        eprintln!("OC_TEST_CLICKHOUSE_URL unset: skipping");
        return;
    };
    if std::env::var_os("OC_TEST_CLICKHOUSE_CLUSTER").is_some() {
        eprintln!("a single-server test (the cluster's are in tests/cluster.rs): skipping");
        return;
    }
    let user = std::env::var("OC_TEST_CLICKHOUSE_USER").unwrap_or_else(|_| "default".into());
    let password = std::env::var("OC_TEST_CLICKHOUSE_PASSWORD").unwrap_or_default();
    let cfg = ClickHouseConfig { url: url.clone(), user: user.clone(), password: password.clone(), database: "oc".into(), cluster: None };
    migrate_clickhouse(&cfg, false).await.expect("migrate");
    let ch = clickhouse::Client::default().with_url(&url).with_user(&user).with_password(&password);
    let org = format!("known-test-{}", std::process::id());
    let count = |sql: String| {
        let ch = ch.clone();
        async move { ch.query(&sql).fetch_one::<Count>().await.unwrap_or_else(|e| panic!("{sql}: {e}")).n }
    };
    let insert = |table: &str, known: bool, t: &str| {
        let mark = if known { ", 'oc.series_known', '1'" } else { "" };
        let (cols, values) = match table {
            "gauge" => ("Value", "1"),
            _ => ("Count, Sum, BucketCounts, ExplicitBounds, AggregationTemporality", "3, 4.5, [1, 2], [10.], 1"),
        };
        format!(
            "INSERT INTO otel.metrics_ingest_{table} (ResourceAttributes, MetricName, Attributes, TimeUnix, {cols}) VALUES \
             (map('oc.org_id', '{org}', 'host.name', 'i-0a1b2c3d4e5f60001', 'oc.metrics_retention_days', '15', 'oc.metrics_retention_5m_days', '180', 'oc.metrics_retention_1h_days', '760'{mark}), \
              'm.{table}', map('state', 'user'), '{t}', {values})"
        )
    };
    for table in ["gauge", "histogram"] {
        ch.query(&insert(table, false, "2026-10-27 10:00:10")).execute().await.expect("unmarked");
        ch.query(&insert(table, true, "2026-10-27 10:00:20")).execute().await.expect("marked");
        ch.query(&insert(table, true, "2026-10-27 10:00:30")).execute().await.expect("marked");
    }
    let o = format!("OrgId = '{org}'");
    for (table, samples) in [("gauge", "metrics_samples"), ("histogram", "metrics_dist_samples")] {
        let name = format!("MetricName = 'm.{table}'");
        assert_eq!(count(format!("SELECT count() AS n FROM otel.{samples} WHERE {o} AND {name}")).await, 3, "{table}: every point is a sample");
        assert_eq!(count(format!("SELECT count() AS n FROM otel.metrics_series WHERE {o} AND {name}")).await, 1, "{table}: one series row, from the unmarked point");
        // The mark is not a label: one series id for all three points.
        assert_eq!(count(format!("SELECT uniqExact(SeriesId) AS n FROM otel.{samples} WHERE {o} AND {name}")).await, 1);
        assert_eq!(
            count(format!("SELECT count() AS n FROM otel.{samples} WHERE {o} AND {name} AND RetentionDays = 15 AND Retention5mDays = 180 AND Retention1hDays = 760")).await,
            3,
            "{table}: samples carry the org's tiers"
        );
    }
}

/// Series rows feed the tag index: a row per label with where its value comes from (Scope bits:
/// 1 point attribute, 2 resource attribute, 4 service, 8 what a plain key reads), and the values
/// per key.
#[tokio::test]
async fn series_feed_the_tag_index() {
    let Ok(url) = std::env::var("OC_TEST_CLICKHOUSE_URL") else {
        eprintln!("OC_TEST_CLICKHOUSE_URL unset: skipping");
        return;
    };
    if std::env::var_os("OC_TEST_CLICKHOUSE_CLUSTER").is_some() {
        eprintln!("a single-server test (the cluster's are in tests/cluster.rs): skipping");
        return;
    }
    let user = std::env::var("OC_TEST_CLICKHOUSE_USER").unwrap_or_else(|_| "default".into());
    let password = std::env::var("OC_TEST_CLICKHOUSE_PASSWORD").unwrap_or_default();
    let cfg = ClickHouseConfig { url: url.clone(), user: user.clone(), password: password.clone(), database: "oc".into(), cluster: None };
    migrate_clickhouse(&cfg, false).await.expect("migrate");
    let ch = clickhouse::Client::default().with_url(&url).with_user(&user).with_password(&password);
    let org = format!("tags-test-{}", std::process::id());
    // `env` is both a resource and a point attribute with other values (the point's wins for a
    // plain key); `zone` both, with the same value; `oc.*` stamps are no labels.
    ch.query(&format!(
        "INSERT INTO otel.metrics_ingest_gauge (ResourceAttributes, ServiceName, MetricName, Attributes, TimeUnix, Value) VALUES \
         (map('oc.org_id', '{org}', 'k8s.namespace.name', 'shop', 'env', 'prod', 'zone', 'a', 'oc.series_known', ''), 'storefront-api', 'kubernetes.cpu.usage.total', \
          map('env', 'canary', 'zone', 'a', 'state', 'user'), '2026-10-27 10:00:10', 1)"
    ))
    .execute()
    .await
    .expect("insert");
    #[derive(clickhouse::Row, serde::Deserialize, Debug, PartialEq)]
    struct Tag {
        key: String,
        value: String,
        scope: u8,
    }
    let tags: Vec<Tag> = ch
        .query(&format!(
            "SELECT toString(Key) AS key, Value AS value, Scope AS scope FROM otel.metrics_tags FINAL WHERE OrgId = '{org}' AND MetricName = 'kubernetes.cpu.usage.total' ORDER BY key, value"
        ))
        .fetch_all()
        .await
        .expect("tags");
    let tag = |key: &str, value: &str, scope: u8| Tag { key: key.into(), value: value.into(), scope };
    assert_eq!(
        tags,
        vec![tag("env", "canary", 1 | 8), tag("env", "prod", 2), tag("k8s.namespace.name", "shop", 2 | 8), tag("service", "storefront-api", 4), tag("state", "user", 1 | 8), tag("zone", "a", 1 | 2 | 8)]
    );
    // Every row is the series' (one series), with its type.
    let n: u64 = ch.query(&format!("SELECT uniqExact(SeriesId) FROM otel.metrics_tags WHERE OrgId = '{org}' AND MetricType = 'gauge'")).fetch_one().await.expect("count");
    assert_eq!(n, 1);
    let values: Vec<String> = ch
        .query(&format!("SELECT concat(toString(Key), '=', Value, ':', toString(Scope)) FROM otel.metrics_tag_values FINAL WHERE OrgId = '{org}' ORDER BY 1"))
        .fetch_all()
        .await
        .expect("values");
    assert_eq!(values, ["env=canary:9", "env=prod:2", "k8s.namespace.name=shop:10", "service=storefront-api:4", "state=user:9", "zone=a:11"]);
}

/// Changing an install-wide retention reaches parts already written: lowering it drops their
/// expired data, raising it keeps data the old retention would have dropped.
#[tokio::test]
async fn retention_changes_reach_existing_parts() {
    let Ok(url) = std::env::var("OC_TEST_CLICKHOUSE_URL") else {
        eprintln!("OC_TEST_CLICKHOUSE_URL unset: skipping");
        return;
    };
    if std::env::var_os("OC_TEST_CLICKHOUSE_CLUSTER").is_some() {
        eprintln!("a single-server test (the cluster's are in tests/cluster.rs): skipping");
        return;
    }
    let user = std::env::var("OC_TEST_CLICKHOUSE_USER").unwrap_or_else(|_| "default".into());
    let password = std::env::var("OC_TEST_CLICKHOUSE_PASSWORD").unwrap_or_default();
    let cfg = ClickHouseConfig { url: url.clone(), user: user.clone(), password: password.clone(), database: "oc".into(), cluster: None };
    migrate_clickhouse(&cfg, false).await.expect("migrate");
    let ch = clickhouse::Client::default().with_url(&url).with_user(&user).with_password(&password).with_option("mutations_sync", "2");
    let org = format!("retention-test-{}", std::process::id());
    let days = |raw: u32| oc_store::RetentionDays { traces: 15, traces_max: 90, metrics: raw, metrics_5m: 90, metrics_1h: 395, ci: 30, raw_logs_hours: 72, metrics_cold_after_days: 0 };
    let count = || {
        let (ch, org) = (ch.clone(), org.clone());
        async move { ch.query(&format!("SELECT count() FROM otel.metrics_samples WHERE OrgId = '{org}'")).fetch_one::<u64>().await.expect("count") }
    };

    // Points 10 days old under a 30-day default: kept.
    oc_store::apply_retention(&cfg, days(30)).await.expect("retention 30");
    ch.query(&format!(
        "INSERT INTO otel.metrics_ingest_gauge (ResourceAttributes, MetricName, Attributes, TimeUnix, Value) \
         SELECT map('oc.org_id', '{org}'), 'system.cpu.utilization', map('cpu', toString(number)), now64(9) - INTERVAL 10 DAY, 1 FROM numbers(3)"
    ))
    .execute()
    .await
    .expect("insert");
    assert_eq!(count().await, 3);

    // Lowered to 7 days: the existing part's TTL is recalculated and the expired data dropped.
    oc_store::apply_retention(&cfg, days(7)).await.expect("retention 7");
    wait_mutations(&ch).await;
    ch.query("OPTIMIZE TABLE otel.metrics_samples FINAL").execute().await.expect("optimize");
    assert_eq!(count().await, 0, "lowering the retention drops data past it");

    // Raised again: data the 7-day retention would drop now stays.
    ch.query(&format!(
        "INSERT INTO otel.metrics_ingest_gauge (ResourceAttributes, MetricName, Attributes, TimeUnix, Value) \
         SELECT map('oc.org_id', '{org}'), 'system.cpu.utilization', map('cpu', toString(number)), now64(9) - INTERVAL 5 DAY, 1 FROM numbers(3)"
    ))
    .execute()
    .await
    .expect("insert");
    oc_store::apply_retention(&cfg, days(30)).await.expect("retention 30 again");
    wait_mutations(&ch).await;
    let expires: u32 = ch
        .query(
            "SELECT toUInt32(dateDiff('day', now(), max(delete_ttl_info_max))) FROM system.parts \
             WHERE active AND database = 'otel' AND table = 'metrics_samples' AND partition LIKE concat('%', toString(today() - 5), '%')",
        )
        .fetch_one()
        .await
        .expect("ttl info");
    assert!(expires >= 20, "the part now expires under the 30-day retention, in {expires} days");
    // Unchanged settings don't touch the tables.
    oc_store::apply_retention(&cfg, days(30)).await.expect("retention unchanged");
}

/// Waits for the background TTL recalculations `apply_retention` starts.
async fn wait_mutations(ch: &clickhouse::Client) {
    for _ in 0..60 {
        let running: u64 = ch.query("SELECT count() FROM system.mutations WHERE NOT is_done").fetch_one().await.expect("mutations");
        if running == 0 {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
    panic!("TTL recalculation didn't finish");
}

/// With cold storage on, metric parts past the move age go to the object-storage volume and stay
/// queryable. Skipped unless OC_TEST_CLICKHOUSE_COLD_URL points at a ClickHouse server whose
/// config declares the `observecraft_tiered` storage policy (the Helm chart's coldStorage).
#[tokio::test]
async fn old_metric_parts_move_to_cold_storage() {
    let Ok(url) = std::env::var("OC_TEST_CLICKHOUSE_COLD_URL") else {
        eprintln!("OC_TEST_CLICKHOUSE_COLD_URL unset: skipping");
        return;
    };
    let user = std::env::var("OC_TEST_CLICKHOUSE_USER").unwrap_or_else(|_| "default".into());
    let password = std::env::var("OC_TEST_CLICKHOUSE_PASSWORD").unwrap_or_default();
    let cfg = ClickHouseConfig { url: url.clone(), user: user.clone(), password: password.clone(), database: "oc".into(), cluster: None };
    migrate_clickhouse(&cfg, false).await.expect("migrate");
    let r = oc_store::RetentionDays { traces: 15, traces_max: 90, metrics: 30, metrics_5m: 90, metrics_1h: 395, ci: 30, raw_logs_hours: 72, metrics_cold_after_days: 2 };
    oc_store::apply_retention(&cfg, r).await.expect("retention with cold storage");
    let ch = clickhouse::Client::default().with_url(&url).with_user(&user).with_password(&password);
    let org = format!("cold-test-{}", std::process::id());
    for age in [1, 3] {
        ch.query(&format!(
            "INSERT INTO otel.metrics_ingest_gauge (ResourceAttributes, MetricName, Attributes, TimeUnix, Value) \
             SELECT map('oc.org_id', '{org}'), 'system.cpu.utilization', map('cpu', toString(number)), now64(9) - INTERVAL {age} DAY, number FROM numbers(10)"
        ))
        .execute()
        .await
        .expect("insert");
    }
    // Moves happen in the background, at merges or on schedule.
    ch.query("OPTIMIZE TABLE otel.metrics_samples FINAL").execute().await.expect("optimize");
    // (age in days, disk) of each part.
    let mut disks = Vec::new();
    for _ in 0..60 {
        disks = ch
            .query("SELECT concat(toString(dateDiff('day', toDate(min_time), today())), ':', disk_name) FROM system.parts WHERE active AND database = 'otel' AND table = 'metrics_samples' ORDER BY 1")
            .fetch_all::<String>()
            .await
            .expect("parts");
        if disks.iter().any(|d| d.ends_with(":cold")) {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
    assert!(disks.contains(&"3:cold".to_string()), "the 3-day-old part is on the cold disk: {disks:?}");
    assert!(disks.contains(&"1:default".to_string()), "the 1-day-old part stays local: {disks:?}");
    let n: u64 = ch.query(&format!("SELECT count() FROM otel.metrics_samples WHERE OrgId = '{org}'")).fetch_one().await.expect("count");
    assert_eq!(n, 20, "cold parts stay queryable");
}
