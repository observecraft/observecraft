//! Log aggregates (ADR-0002): logs written to `oc.logs` land, per minute, in the error groups and
//! standard facet tables. Skipped unless OC_TEST_CLICKHOUSE_URL points at a single ClickHouse server
//! (user `default`, or OC_TEST_CLICKHOUSE_USER / OC_TEST_CLICKHOUSE_PASSWORD).

use oc_store::{migrate_clickhouse, ClickHouseConfig};

#[derive(clickhouse::Row, serde::Deserialize, Debug, PartialEq)]
struct Group {
    service: String,
    kind: String,
    key: String,
    env: String,
    n: u64,
    first: i64,
    last: i64,
    sample: String,
}

#[derive(clickhouse::Row, serde::Deserialize, Debug, PartialEq)]
struct FacetRow {
    minute: i64,
    status: String,
    host: String,
    kube_namespace: String,
    retention_days: u16,
    n: u64,
}

#[tokio::test]
async fn logs_land_in_the_error_groups_and_facet_aggregates() {
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
    let org = format!("log-aggregates-{}", std::process::id());

    // Two inserts in the same minute (two blocks: the aggregates merge them), plus a later minute.
    let insert = |values: String| {
        let ch = ch.clone();
        async move {
            ch.query(&format!("INSERT INTO oc.logs (org_id, timestamp, status, service, host, source, message, tags, attr, retention_days) VALUES {values}")).execute().await.expect("insert logs")
        }
    };
    insert(format!(
        "('{org}', '2026-10-27 10:00:05.000', 'error', 'billing', 'i-0a1b2c3d4e5f60001', 'python', 'ValueError: order 41 not found', map('env', 'prod', 'kube_namespace', 'payments'), map('error.kind', 'ValueError'), 3650), \
         ('{org}', '2026-10-27 10:00:20.500', 'info', 'billing', 'i-0a1b2c3d4e5f60001', 'python', 'charged order 41', map('env', 'prod', 'kube_namespace', 'payments'), map(), 3650)"
    ))
    .await;
    insert(format!(
        "('{org}', '2026-10-27 10:00:40.250', 'error', 'billing', 'i-0a1b2c3d4e5f60001', 'python', 'ValueError: order 982 not found', map('env', 'prod', 'kube_namespace', 'payments'), map('error.kind', 'ValueError'), 3650), \
         ('{org}', '2026-10-27 10:01:10.000', 'error', 'billing', 'i-0a1b2c3d4e5f60002', 'python', 'ValueError: order 7 not found', map('env', 'prod'), map('error.kind', 'ValueError'), 365)"
    ))
    .await;

    // One error group, in two minutes and two retentions: counts, first/last seen and the newest message.
    let groups = ch
        .query(
            "SELECT toString(service) AS service, toString(error_kind) AS kind, error_key AS key, toString(env) AS env, sum(n) AS n, \
             toUnixTimestamp64Milli(min(first_seen)) AS first, toUnixTimestamp64Milli(max(last_seen)) AS last, argMaxMerge(sample) AS sample \
             FROM oc.error_groups_1m WHERE org_id = ? GROUP BY service, kind, key, env",
        )
        .bind(&org)
        .fetch_all::<Group>()
        .await
        .expect("error groups");
    assert_eq!(
        groups,
        vec![Group {
            service: "billing".into(),
            kind: "ValueError".into(),
            key: "ValueError: order 0 not found".into(),
            env: "prod".into(),
            n: 3,
            first: 1_793_095_205_000,
            last: 1_793_095_270_000,
            sample: "ValueError: order 7 not found".into(),
        }]
    );

    // Facet counts per minute and combination of the standard facets, with each log's retention.
    let facets = ch
        .query(
            "SELECT toInt64(toUnixTimestamp(minute)) AS minute, toString(status) AS status, toString(host) AS host, toString(kube_namespace) AS kube_namespace, \
             retention_days, sum(n) AS n FROM oc.log_facets_1m WHERE org_id = ? GROUP BY minute, status, host, kube_namespace, retention_days ORDER BY minute, status",
        )
        .bind(&org)
        .fetch_all::<FacetRow>()
        .await
        .expect("facets");
    assert_eq!(
        facets,
        vec![
            FacetRow { minute: 1_793_095_200, status: "error".into(), host: "i-0a1b2c3d4e5f60001".into(), kube_namespace: "payments".into(), retention_days: 3650, n: 2 },
            FacetRow { minute: 1_793_095_200, status: "info".into(), host: "i-0a1b2c3d4e5f60001".into(), kube_namespace: "payments".into(), retention_days: 3650, n: 1 },
            FacetRow { minute: 1_793_095_260, status: "error".into(), host: "i-0a1b2c3d4e5f60002".into(), kube_namespace: String::new(), retention_days: 365, n: 1 },
        ]
    );
}
