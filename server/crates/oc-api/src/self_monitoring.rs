//! ObserveCraft monitoring itself: every minute one server (a Postgres advisory lock) measures what keeps the
//! install healthy and writes it as `observecraft.*` gauges into the operator's organization, straight into the
//! metric ingest tables (not through Kafka, so a Kafka outage is still measured):
//!   - ClickHouse: reachable, disks, table sizes, seconds since each signal table's last write, parts per
//!     partition (inserts are throttled, then refused, past `parts_to_delay_insert` / `_throw_insert`), mutations,
//!     replication;
//!   - the log ingester's Kafka lag (`log_ingest_partitions` against the topics' latest offsets);
//!   - Postgres: connections against `max_connections`, database size;
//!   - background jobs: overdue and failing cloud crawl jobs, the logs-based metrics outbox;
//!   - cost: the install's containers (the Datadog Agent's `container.*` of `OC_SELF_MONITORING_NAMESPACE`) at
//!     the org's Kubernetes prices (Kubernetes → Autoscaling settings) plus ClickHouse and Postgres storage at
//!     `OC_SELF_MONITORING_STORAGE_GB_MONTH`, and the next 30 days with the disks' growth;
//!   - hourly, each organization's usage: the events it stored in the previous hour per signal, and an estimate
//!     of their bytes (the table's bytes per row).
//!
//! The operator's organization is `OC_SELF_MONITORING_ORG`, else the first instance admin's. Only it sees these
//! metrics (other orgs' usage is aggregated there and nowhere else), the "ObserveCraft Overview" dashboard and the
//! recommended monitors created there (`observecraft:self-monitoring`, notifying `OC_SELF_MONITORING_NOTIFY`).
//! A crashed install can't alert about itself: `OC_HEARTBEAT_URL` (e.g. a healthchecks.io check) is called every
//! minute while Postgres and ClickHouse answer, so the service outside alerts when the calls stop.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::time::{Duration, Instant};

use serde::Deserialize;
use serde_json::{json, Value};

use crate::routes::logs::TimeRange;
use crate::state::AppState;

/// Advisory lock held by the server that collects.
const LOCK_KEY: i64 = 0x0b5e_5e1f_0000_0001;
/// The tables of each signal: (signal, table read, its table in `system.parts`, time column, org column).
const SIGNALS: [(&str, &str, &str, &str, &str); 6] = [
    ("logs", "oc.logs_read", "logs", "timestamp", "org_id"),
    ("spans", "otel.otel_traces", "otel_traces", "Timestamp", "OrgId"),
    ("metric_points", "otel.metrics_samples", "metrics_samples", "TimeUnix", "OrgId"),
    ("rum", "otel.otel_rum", "otel_rum", "Timestamp", "OrgId"),
    ("events", "otel.otel_events", "otel_events", "Timestamp", "OrgId"),
    ("profiles", "otel.profiles", "profiles", "Start", "OrgId"),
];
const HOURS_PER_MONTH: f64 = 730.0;
const GIB: f64 = 1024.0 * 1024.0 * 1024.0;

/// Settings (environment).
#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    pub enabled: bool,
    /// The operator's organization; else the first instance admin's.
    pub org: Option<String>,
    /// Handles the recommended monitors notify (`@slack-ops @ops@example.com`).
    pub notify: String,
    pub heartbeat_url: Option<String>,
    /// Kubernetes namespace ObserveCraft runs in (its containers' cost).
    pub namespace: String,
    /// USD per GB-month of disk.
    pub storage_gb_month: f64,
}

impl Settings {
    pub fn from_env() -> Settings {
        let var = |k: &str| std::env::var(k).ok().map(|v| v.trim().to_string()).filter(|v| !v.is_empty());
        Settings {
            enabled: var("OC_SELF_MONITORING").is_none_or(|v| v != "false" && v != "0"),
            org: var("OC_SELF_MONITORING_ORG"),
            notify: var("OC_SELF_MONITORING_NOTIFY").unwrap_or_default(),
            heartbeat_url: var("OC_HEARTBEAT_URL").filter(|u| u.starts_with("http://") || u.starts_with("https://")),
            namespace: var("OC_SELF_MONITORING_NAMESPACE").unwrap_or_else(|| "observecraft".into()),
            storage_gb_month: var("OC_SELF_MONITORING_STORAGE_GB_MONTH").and_then(|v| v.parse().ok()).filter(|p: &f64| p.is_finite() && *p >= 0.0).unwrap_or(0.08),
        }
    }
}

/// The operator's organization: the setting when it exists, else the first instance admin's.
pub async fn operator_org(st: &AppState) -> Option<String> {
    static RESOLVED: tokio::sync::OnceCell<Option<String>> = tokio::sync::OnceCell::const_new();
    RESOLVED
        .get_or_init(|| async {
            let settings = Settings::from_env();
            let found: Result<Option<String>, sqlx::Error> = match &settings.org {
                Some(org) => sqlx::query_scalar("SELECT id FROM orgs WHERE id = $1").bind(org).fetch_optional(&st.postgres).await,
                None => {
                    sqlx::query_scalar(
                        "SELECT u.org_id FROM users u JOIN accounts a ON a.id = u.account_id WHERE a.is_instance_admin AND u.status = 'active' \
                         ORDER BY a.created_at, u.org_id LIMIT 1",
                    )
                    .fetch_optional(&st.postgres)
                    .await
                }
            };
            match found {
                Ok(org) => org,
                Err(e) => {
                    tracing::warn!(error = %e, "self-monitoring: finding the operator's organization failed");
                    None
                }
            }
        })
        .await
        .clone()
}

/// One gauge value.
#[derive(Clone, Debug, PartialEq)]
pub struct Gauge {
    pub name: &'static str,
    pub tags: Vec<(&'static str, String)>,
    pub value: f64,
}

fn gauge(name: &'static str, value: f64) -> Gauge {
    Gauge { name, tags: Vec::new(), value }
}

fn tagged(name: &'static str, tags: &[(&'static str, &str)], value: f64) -> Gauge {
    Gauge { name, tags: tags.iter().map(|(k, v)| (*k, v.to_string())).collect(), value }
}

/// The `otel.metrics_ingest_gauge` rows (JSONEachRow) of `gauges` at `time_ms` in `org`.
pub fn rows(org: &str, time_ms: i64, gauges: &[Gauge]) -> Vec<u8> {
    let time = crate::pipelines::dates::to_rfc3339(time_ms);
    let mut out = Vec::new();
    for g in gauges.iter().filter(|g| g.value.is_finite()) {
        let attrs: BTreeMap<&str, &str> = g.tags.iter().map(|(k, v)| (*k, v.as_str())).collect();
        let row = json!({
            "ResourceAttributes": { "oc.org_id": org, "service.name": "observecraft" },
            "ServiceName": "observecraft",
            "ScopeName": "observecraft.self_monitoring",
            "MetricName": g.name,
            "MetricDescription": "",
            "MetricUnit": "",
            "Attributes": attrs,
            "StartTimeUnix": time,
            "TimeUnix": time,
            "Value": g.value,
        });
        if serde_json::to_writer(&mut out, &row).is_ok() {
            out.push(b'\n');
        }
    }
    out
}

/// Starts the loop (every server runs it; the one holding the lock collects).
pub async fn run(st: AppState) {
    let settings = Settings::from_env();
    if !settings.enabled {
        tracing::info!("self-monitoring is off (OC_SELF_MONITORING=false)");
        return;
    }
    let mut collector = Collector::default();
    let mut lock: Option<sqlx::pool::PoolConnection<sqlx::Postgres>> = None;
    let mut tick = tokio::time::interval(Duration::from_secs(60));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tick.tick().await;
        if !holds_lock(&st, &mut lock).await {
            continue;
        }
        let Some(org) = operator_org(&st).await else {
            if !collector.warned_no_org {
                tracing::warn!("self-monitoring: no operator organization (set OC_SELF_MONITORING_ORG or make an account instance admin)");
                collector.warned_no_org = true;
            }
            continue;
        };
        if !collector.monitors_installed {
            match install_monitors(&st, &org, &settings.notify).await {
                Ok(n) => {
                    collector.monitors_installed = true;
                    if n > 0 {
                        tracing::info!(org, created = n, "self-monitoring: recommended monitors created");
                    }
                }
                Err(e) => tracing::warn!(error = %e, "self-monitoring: creating the recommended monitors failed"),
            }
        }
        let now = crate::routes::metrics::now_ms();
        let (gauges, healthy) = collector.collect(&st, &org, &settings, now).await;
        let body = rows(&org, now, &gauges);
        if let Err(e) = crate::logproc::insert_rows(&st, "otel.metrics_ingest_gauge", body).await {
            tracing::warn!(error = %e, "self-monitoring: writing the metrics failed");
        }
        if let (true, Some(url)) = (healthy, settings.heartbeat_url.as_deref()) {
            if let Err(e) = st.ingest_http.get(url).timeout(Duration::from_secs(10)).send().await.and_then(|r| r.error_for_status()) {
                tracing::warn!(error = %e, "self-monitoring: the heartbeat call failed");
            }
        }
    }
}

/// Whether this server holds the collector lock (taken on a connection kept for it; lost with the connection).
async fn holds_lock(st: &AppState, lock: &mut Option<sqlx::pool::PoolConnection<sqlx::Postgres>>) -> bool {
    if let Some(conn) = lock.as_mut() {
        if sqlx::query("SELECT 1").execute(&mut **conn).await.is_ok() {
            return true;
        }
        *lock = None;
    }
    let Ok(mut conn) = st.postgres.acquire().await else { return false };
    match sqlx::query_scalar::<_, bool>("SELECT pg_try_advisory_lock($1)").bind(LOCK_KEY).fetch_one(&mut *conn).await {
        Ok(true) => {
            *lock = Some(conn);
            true
        }
        _ => false,
    }
}

#[derive(Default)]
struct Collector {
    warned_no_org: bool,
    monitors_installed: bool,
    /// The last hour whose usage was written (epoch ms of its start).
    usage_hour: Option<i64>,
    /// Tables seen written in the last day: their age is reported while it grows.
    written: HashSet<String>,
}

#[derive(clickhouse::Row, Deserialize)]
struct DiskRow {
    replica: String,
    disk: String,
    total: u64,
    free: u64,
}

#[derive(clickhouse::Row, Deserialize)]
struct TableRow {
    table: String,
    bytes: u64,
    rows: u64,
    age: i64,
    parts_max: u64,
}

#[derive(clickhouse::Row, Deserialize)]
struct UsageRow {
    org: String,
    n: u64,
}

impl Collector {
    async fn collect(&mut self, st: &AppState, org: &str, settings: &Settings, now: i64) -> (Vec<Gauge>, bool) {
        let mut out = vec![gauge("observecraft.up", 1.0)];
        let pg_ok = self.postgres(st, &mut out).await;
        let (ch_ok, disks, tables) = self.clickhouse(st, &mut out).await;
        self.kafka(st, &mut out).await;
        self.jobs(st, &mut out).await;
        self.cost(st, org, settings, now, &disks, &mut out).await;
        if ch_ok {
            self.usage(st, now, &tables, &mut out).await;
        }
        (out, pg_ok && ch_ok)
    }

    async fn postgres(&mut self, st: &AppState, out: &mut Vec<Gauge>) -> bool {
        let row: Result<(i64, String, i64), sqlx::Error> = sqlx::query_as(
            "SELECT (SELECT count(*) FROM pg_stat_activity), current_setting('max_connections'), pg_database_size(current_database())",
        )
        .fetch_one(&st.postgres)
        .await;
        match row {
            Ok((used, max, size)) => {
                let max: f64 = max.parse().unwrap_or(0.0);
                out.push(gauge("observecraft.postgres.connections.used", used as f64));
                out.push(gauge("observecraft.postgres.connections.max", max));
                if max > 0.0 {
                    out.push(gauge("observecraft.postgres.connections.used_pct", used as f64 * 100.0 / max));
                }
                out.push(gauge("observecraft.postgres.database.bytes", size as f64));
                true
            }
            Err(e) => {
                tracing::warn!(error = %e, "self-monitoring: Postgres");
                false
            }
        }
    }

    /// ClickHouse's gauges; whether it answered, its disks and its tables.
    async fn clickhouse(&mut self, st: &AppState, out: &mut Vec<Gauge>) -> (bool, Vec<DiskRow>, Vec<TableRow>) {
        let ch = &st.clickhouse;
        let cluster = st.tables.cluster.clone();
        // Every replica's system tables in a cluster, the server's own otherwise.
        let from = |table: &str| match &cluster {
            Some(_) => format!("clusterAllReplicas({{cluster:String}}, system.{table})"),
            None => format!("system.{table}"),
        };
        let q = |sql: String| {
            let q = ch.query(&sql);
            match &cluster {
                Some(c) => q.with_setting("param_cluster", c.as_str()),
                None => q,
            }
        };
        let started = Instant::now();
        let disks = match q(format!("SELECT hostName() AS replica, name AS disk, total_space AS total, free_space AS free FROM {} WHERE type = 'Local' OR type = 'local'", from("disks")))
            .fetch_all::<DiskRow>()
            .await
        {
            Ok(d) => d,
            Err(e) => {
                tracing::warn!(error = %e, "self-monitoring: ClickHouse");
                out.push(gauge("observecraft.clickhouse.reachable", 0.0));
                return (false, Vec::new(), Vec::new());
            }
        };
        out.push(gauge("observecraft.clickhouse.reachable", 1.0));
        out.push(gauge("observecraft.clickhouse.ping_ms", started.elapsed().as_secs_f64() * 1000.0));
        for d in &disks {
            let tags = [("replica", d.replica.as_str()), ("disk", d.disk.as_str())];
            let used = d.total.saturating_sub(d.free) as f64;
            out.push(tagged("observecraft.clickhouse.disk.total_bytes", &tags, d.total as f64));
            out.push(tagged("observecraft.clickhouse.disk.free_bytes", &tags, d.free as f64));
            out.push(tagged("observecraft.clickhouse.disk.used_bytes", &tags, used));
            if d.total > 0 {
                out.push(tagged("observecraft.clickhouse.disk.used_pct", &tags, used * 100.0 / d.total as f64));
            }
        }
        // Tables (the local ones in a cluster, named without `_local`), summed over replicas.
        let tables = q(format!(
            "SELECT replaceRegexpOne(table, '_local$', '') AS table, sum(bytes) AS bytes, sum(rows) AS rows, \
                    toInt64(dateDiff('second', max(modified), now())) AS age, max(parts) AS parts_max \
             FROM (SELECT table, partition, sum(bytes_on_disk) AS bytes, sum(rows) AS rows, max(modification_time) AS modified, count() AS parts \
                   FROM {} WHERE active AND database IN ('otel', 'oc') GROUP BY hostName(), table, partition) \
             GROUP BY table",
            from("parts")
        ))
        .fetch_all::<TableRow>()
        .await
        .unwrap_or_else(|e| {
            tracing::warn!(error = %e, "self-monitoring: ClickHouse parts");
            Vec::new()
        });
        let signal_tables: HashSet<&str> = SIGNALS.iter().map(|(_, _, parts, _, _)| *parts).collect();
        for t in &tables {
            let tags = [("table", t.table.as_str())];
            out.push(tagged("observecraft.clickhouse.table.bytes", &tags, t.bytes as f64));
            out.push(tagged("observecraft.clickhouse.table.rows", &tags, t.rows as f64));
            out.push(tagged("observecraft.clickhouse.table.parts_max_per_partition", &tags, t.parts_max as f64));
            if signal_tables.contains(t.table.as_str()) && report_age(&mut self.written, &t.table, t.age) {
                out.push(tagged("observecraft.clickhouse.table.last_write_age_seconds", &tags, t.age as f64));
            }
        }
        let mutations: Result<(u64, u64), _> = q(format!(
            "SELECT countIf(NOT is_done), countIf(NOT is_done AND latest_fail_reason != '') FROM {} WHERE database IN ('otel', 'oc')",
            from("mutations")
        ))
        .fetch_one()
        .await;
        if let Ok((pending, failed)) = mutations {
            out.push(gauge("observecraft.clickhouse.mutations.pending", pending as f64));
            out.push(gauge("observecraft.clickhouse.mutations.failed", failed as f64));
        }
        if cluster.is_some() {
            let replication: Result<(u64, u64), _> =
                q(format!("SELECT toUInt64(max(absolute_delay)), toUInt64(sum(queue_size)) FROM {} WHERE database IN ('otel', 'oc')", from("replicas"))).fetch_one().await;
            if let Ok((delay, queue)) = replication {
                out.push(gauge("observecraft.clickhouse.replication.max_delay_seconds", delay as f64));
                out.push(gauge("observecraft.clickhouse.replication.queue_size", queue as f64));
            }
        }
        (true, disks, tables)
    }

    /// The log ingester's lag: the topics' latest offsets less the offsets the servers stored up to.
    async fn kafka(&mut self, st: &AppState, out: &mut Vec<Gauge>) {
        if !st.config.log_ingest_kafka {
            return;
        }
        let Some(brokers) = st.config.kafka_brokers.as_deref() else { return };
        let stored: Vec<(String, i32, i64)> = match sqlx::query_as("SELECT topic, partition, next_offset FROM log_ingest_partitions").fetch_all(&st.postgres).await {
            Ok(rows) => rows,
            Err(e) => {
                tracing::warn!(error = %e, "self-monitoring: log ingest offsets");
                return;
            }
        };
        let latest = match latest_offsets(brokers, &stored).await {
            Ok(l) => l,
            Err(e) => {
                tracing::warn!(error = %e, "self-monitoring: Kafka");
                out.push(gauge("observecraft.kafka.reachable", 0.0));
                return;
            }
        };
        out.push(gauge("observecraft.kafka.reachable", 1.0));
        for (topic, lag) in consumer_lag(&stored, &latest) {
            out.push(tagged("observecraft.kafka.consumer_lag", &[("topic", topic.as_str()), ("consumer", "log-ingest")], lag as f64));
        }
    }

    async fn jobs(&mut self, st: &AppState, out: &mut Vec<Gauge>) {
        let jobs: Result<Vec<(String, i64, i64)>, sqlx::Error> = sqlx::query_as(
            "SELECT provider, count(*) FILTER (WHERE next_run_at < now() - interval '15 minutes' AND (claimed_until IS NULL OR claimed_until < now())), \
                    count(*) FILTER (WHERE failures >= 3) FROM cloud_crawl_jobs GROUP BY provider",
        )
        .fetch_all(&st.postgres)
        .await;
        if let Ok(jobs) = jobs {
            for (provider, overdue, failing) in jobs {
                out.push(tagged("observecraft.jobs.overdue", &[("provider", provider.as_str())], overdue as f64));
                out.push(tagged("observecraft.jobs.failing", &[("provider", provider.as_str())], failing as f64));
            }
        }
        if let Ok(n) = sqlx::query_scalar::<_, i64>("SELECT count(*) FROM log_metric_outbox").fetch_one(&st.postgres).await {
            out.push(gauge("observecraft.log_metrics.outbox_backlog", n as f64));
        }
    }

    /// The install's monthly cost at the current rate, by container and storage, and the next 30 days'.
    async fn cost(&mut self, st: &AppState, org: &str, settings: &Settings, now: i64, disks: &[DiskRow], out: &mut Vec<Gauge>) {
        let prices = crate::routes::kubernetes_autoscaling::load_settings(st, org).await.unwrap_or_default();
        let range = TimeRange { from: now - 10 * 60_000, to: now };
        let ns = settings.namespace.replace([',', '}', '{', ' '], "");
        let mut by_container: BTreeMap<String, (f64, f64)> = BTreeMap::new();
        for (i, metric) in ["container.cpu.usage", "container.memory.usage"].iter().enumerate() {
            let query = format!("avg:{metric}{{kube_namespace:{ns}}} by {{kube_container_name}}");
            if let Ok((_, groups)) = crate::routes::metrics::evaluate_formula(st, org, &query, range, 10).await {
                for g in groups {
                    let Some(name) = g.tags.get("kube_container_name").cloned() else { continue };
                    let values: Vec<f64> = g.points.iter().map(|(_, v)| *v).filter(|v| v.is_finite()).collect();
                    if values.is_empty() {
                        continue;
                    }
                    let avg = values.iter().sum::<f64>() / values.len() as f64;
                    let e = by_container.entry(name).or_default();
                    if i == 0 {
                        e.0 = avg / 1e9; // nanocores → cores
                    } else {
                        e.1 = avg / GIB;
                    }
                }
            }
        }
        let mut total = 0.0;
        for (container, (cores, gib)) in &by_container {
            let (cpu, memory) = (cores * prices.cpu_hour_price * HOURS_PER_MONTH, gib * prices.memory_gib_hour_price * HOURS_PER_MONTH);
            out.push(tagged("observecraft.cost.estimated_monthly", &[("component", container.as_str()), ("resource", "cpu")], cpu));
            out.push(tagged("observecraft.cost.estimated_monthly", &[("component", container.as_str()), ("resource", "memory")], memory));
            total += cpu + memory;
        }
        let ch_used: f64 = disks.iter().map(|d| d.total.saturating_sub(d.free) as f64).sum();
        let pg_bytes = out.iter().find(|g| g.name == "observecraft.postgres.database.bytes").map_or(0.0, |g| g.value);
        for (component, bytes) in [("clickhouse", ch_used), ("postgres", pg_bytes)] {
            let usd = bytes / 1e9 * settings.storage_gb_month;
            out.push(tagged("observecraft.cost.estimated_monthly", &[("component", component), ("resource", "storage")], usd));
            total += usd;
        }
        // The disks' growth over the last week (from these gauges), and when they fill up.
        let week = TimeRange { from: now - 7 * 86_400_000, to: now };
        let mut growth_total = 0.0;
        if let Ok((_, groups)) = crate::routes::metrics::evaluate_formula(st, org, "avg:observecraft.clickhouse.disk.used_bytes{*} by {replica}", week, 168).await {
            for g in groups {
                let Some(replica) = g.tags.get("replica").cloned() else { continue };
                let Some(per_day) = growth_per_day(&g.points) else { continue };
                growth_total += per_day.max(0.0);
                out.push(tagged("observecraft.clickhouse.disk.growth_bytes_per_day", &[("replica", replica.as_str())], per_day));
                let free: u64 = disks.iter().filter(|d| d.replica == replica).map(|d| d.free).sum();
                if let Some(days) = days_until_full(free as f64, per_day) {
                    out.push(tagged("observecraft.clickhouse.disk.days_until_full", &[("replica", replica.as_str())], days));
                }
            }
        }
        out.push(gauge("observecraft.cost.projected_monthly", projected_monthly(total, growth_total, settings.storage_gb_month)));
    }

    /// Hourly: the events each organization stored in the previous hour, per signal, and their estimated bytes.
    async fn usage(&mut self, st: &AppState, now: i64, tables: &[TableRow], out: &mut Vec<Gauge>) {
        let hour = now - now.rem_euclid(3_600_000) - 3_600_000;
        if self.usage_hour == Some(hour) {
            return;
        }
        self.usage_hour = Some(hour);
        for (signal, table, parts, time, org_col) in SIGNALS {
            let Some(t) = tables.iter().find(|t| t.table == parts) else { continue };
            let bytes_per_row = if t.rows > 0 { t.bytes as f64 / t.rows as f64 } else { 0.0 };
            let sql = usage_sql(table, time, org_col);
            let rows = st.clickhouse.query(&sql).bind(hour / 1000).bind(hour / 1000 + 3600).fetch_all::<UsageRow>().await;
            match rows {
                Ok(rows) => {
                    for r in rows.into_iter().filter(|r| !r.org.is_empty()) {
                        let tags = [("org", r.org.as_str()), ("signal", signal)];
                        out.push(tagged("observecraft.usage.events", &tags, r.n as f64));
                        out.push(tagged("observecraft.usage.estimated_bytes", &tags, r.n as f64 * bytes_per_row));
                    }
                }
                Err(e) => tracing::warn!(error = %e, signal, "self-monitoring: usage"),
            }
        }
    }
}

/// Each organization's events of `table` in an hour (`?` from, `?` to, epoch seconds): two columns read.
fn usage_sql(table: &str, time: &str, org_col: &str) -> String {
    format!("SELECT {org_col} AS org, count() AS n FROM {table} WHERE {time} >= toDateTime(?) AND {time} < toDateTime(?) AND {org_col} != '' GROUP BY {org_col}")
}

/// Whether a signal table's age is reported: while it was written in the last day (an install that never
/// receives RUM shouldn't alert that RUM stopped), and on while that day lasts.
fn report_age(written: &mut HashSet<String>, table: &str, age_secs: i64) -> bool {
    if age_secs < 86_400 {
        written.insert(table.to_string());
        return true;
    }
    written.remove(table);
    false
}

/// The partitions' latest offsets (rskafka), by (topic, partition).
async fn latest_offsets(brokers: &str, partitions: &[(String, i32, i64)]) -> Result<HashMap<(String, i32), i64>, String> {
    use rskafka::client::partition::{OffsetAt, UnknownTopicHandling};
    static CLIENT: tokio::sync::Mutex<Option<std::sync::Arc<rskafka::client::Client>>> = tokio::sync::Mutex::const_new(None);
    let client = {
        let mut c = CLIENT.lock().await;
        match c.as_ref() {
            Some(c) => c.clone(),
            None => {
                let brokers: Vec<String> = brokers.split(',').map(|b| b.trim().to_string()).filter(|b| !b.is_empty()).collect();
                let built = std::sync::Arc::new(rskafka::client::ClientBuilder::new(brokers).client_id("observecraft-self-monitoring").build().await.map_err(|e| e.to_string())?);
                *c = Some(built.clone());
                built
            }
        }
    };
    let mut out = HashMap::new();
    for (topic, partition, _) in partitions {
        let pc = client.partition_client(topic.clone(), *partition, UnknownTopicHandling::Error).await.map_err(|e| e.to_string())?;
        let latest = tokio::time::timeout(Duration::from_secs(10), pc.get_offset(OffsetAt::Latest)).await.map_err(|_| "timed out".to_string())?.map_err(|e| e.to_string())?;
        out.insert((topic.clone(), *partition), latest);
    }
    Ok(out)
}

/// Records not yet stored, per topic.
fn consumer_lag(stored: &[(String, i32, i64)], latest: &HashMap<(String, i32), i64>) -> BTreeMap<String, i64> {
    let mut lag = BTreeMap::new();
    for (topic, partition, next) in stored {
        if let Some(l) = latest.get(&(topic.clone(), *partition)) {
            *lag.entry(topic.clone()).or_insert(0) += (l - next).max(0);
        }
    }
    lag
}

/// Bytes per day between the first and last points at least 6 hours apart (none before that).
fn growth_per_day(points: &[(i64, f64)]) -> Option<f64> {
    let valid: Vec<&(i64, f64)> = points.iter().filter(|(_, v)| v.is_finite()).collect();
    let (first, last) = (valid.first()?, valid.last()?);
    let span_ms = last.0 - first.0;
    (span_ms >= 6 * 3_600_000).then(|| (last.1 - first.1) / (span_ms as f64 / 86_400_000.0))
}

/// Days until `free` bytes are used at `per_day`; none while the disk doesn't grow.
fn days_until_full(free: f64, per_day: f64) -> Option<f64> {
    (per_day > 0.0).then(|| (free / per_day).min(3650.0))
}

/// Next 30 days: today's monthly rate plus the storage the disks add over them (half of it on average).
fn projected_monthly(monthly_now: f64, growth_bytes_per_day: f64, gb_month: f64) -> f64 {
    monthly_now + growth_bytes_per_day * 30.0 / 2.0 / 1e9 * gb_month
}

/// The recommended monitors: (name, query, options, message, priority).
pub fn monitor_seeds(notify: &str) -> Vec<(&'static str, &'static str, Value, String, i16)> {
    let msg = |text: &str| if notify.is_empty() { text.to_string() } else { format!("{text}\n\n{notify}") };
    let threshold = |comparator: &str, critical: f64, warning: Option<f64>, window: i64, no_data: bool| {
        let mut thresholds = json!({ "critical": critical });
        if let Some(w) = warning {
            thresholds["warning"] = json!(w);
        }
        json!({ "detection": "threshold", "timeAggregation": "avg", "windowMinutes": window, "comparator": comparator, "thresholds": thresholds, "notifyNoData": no_data, "noDataMinutes": 10 })
    };
    vec![
        (
            "[ObserveCraft] ClickHouse disk almost full on {{replica.name}}",
            "max:observecraft.clickhouse.disk.used_pct{*} by {replica}",
            threshold("above", 85.0, Some(75.0), 10, false),
            msg("ClickHouse's disk on {{replica.name}} is {{value}}% used. Inserts fail when it is full: add disk, lower retention (Settings → Data Retention) or move old data to S3."),
            1,
        ),
        (
            "[ObserveCraft] ClickHouse disk fills up within days on {{replica.name}}",
            "min:observecraft.clickhouse.disk.days_until_full{*} by {replica}",
            threshold("below", 7.0, Some(14.0), 60, false),
            msg("At the last week's growth, ClickHouse's disk on {{replica.name}} is full in {{value}} days."),
            2,
        ),
        (
            "[ObserveCraft] No new data stored in {{table.name}}",
            "max:observecraft.clickhouse.table.last_write_age_seconds{*} by {table}",
            threshold("above", 900.0, Some(300.0), 5, false),
            msg("Nothing was written to {{table.name}} for {{value}} seconds: check the intake, Kafka, the Collector and the server's log ingester."),
            1,
        ),
        (
            "[ObserveCraft] Log ingestion is falling behind on {{topic.name}}",
            "max:observecraft.kafka.consumer_lag{consumer:log-ingest} by {topic}",
            threshold("above", 1_000_000.0, Some(200_000.0), 10, false),
            msg("{{value}} log records on {{topic.name}} wait to be stored. Add server replicas or raise OC_LOG_INGEST_CONCURRENCY; check ClickHouse's inserts."),
            2,
        ),
        (
            "[ObserveCraft] Kafka is unreachable",
            "min:observecraft.kafka.reachable{*}",
            threshold("below", 0.5, None, 5, false),
            msg("The server can't reach Kafka: the intake can't accept telemetry."),
            1,
        ),
        (
            "[ObserveCraft] ClickHouse is unreachable",
            "min:observecraft.clickhouse.reachable{*}",
            threshold("below", 0.5, None, 3, false),
            msg("The server can't query ClickHouse: pages, monitors and ingestion are failing."),
            1,
        ),
        (
            "[ObserveCraft] Too many ClickHouse parts in {{table.name}}",
            "max:observecraft.clickhouse.table.parts_max_per_partition{*} by {table}",
            threshold("above", 600.0, Some(300.0), 15, false),
            msg("A partition of {{table.name}} has {{value}} parts: ClickHouse slows inserts past 1,000 and refuses them past 3,000. Merges are behind (CPU, disk) or inserts are too small."),
            2,
        ),
        (
            "[ObserveCraft] ClickHouse mutations are failing",
            "max:observecraft.clickhouse.mutations.failed{*}",
            threshold("above", 0.5, None, 30, false),
            msg("{{value}} ClickHouse mutations (deletes, retention changes) keep failing: see system.mutations' latest_fail_reason."),
            3,
        ),
        (
            "[ObserveCraft] Postgres connections near the limit",
            "max:observecraft.postgres.connections.used_pct{*}",
            threshold("above", 90.0, Some(75.0), 10, false),
            msg("Postgres has {{value}}% of max_connections in use: raise max_connections or lower the servers' pool size."),
            2,
        ),
        (
            "[ObserveCraft] Cloud crawl jobs are overdue for {{provider.name}}",
            "max:observecraft.jobs.overdue{*} by {provider}",
            threshold("above", 0.5, None, 30, false),
            msg("{{value}} {{provider.name}} crawl jobs are more than 15 minutes late: the servers' crawlers are stuck or too few."),
            3,
        ),
        (
            "[ObserveCraft] Self-monitoring stopped reporting",
            "max:observecraft.up{*}",
            threshold("below", 0.5, None, 5, true),
            msg("ObserveCraft stopped measuring itself: no server holds the self-monitoring lock or writing metrics fails."),
            2,
        ),
    ]
}

/// Creates the recommended monitors the operator's org lacks (by name); returns how many.
async fn install_monitors(st: &AppState, org: &str, notify: &str) -> Result<usize, sqlx::Error> {
    let existing: Vec<String> = sqlx::query_scalar("SELECT name FROM monitors WHERE org_id = $1 AND 'observecraft:self-monitoring' = ANY(tags)").bind(org).fetch_all(&st.postgres).await?;
    let mut created = 0;
    for (name, query, options, message, priority) in monitor_seeds(notify) {
        if existing.iter().any(|e| e == name) {
            continue;
        }
        sqlx::query(
            "INSERT INTO monitors (org_id, name, type, query, definition, status, message, priority, tags, teams, draft) \
             VALUES ($1, $2, 'metric', $3, $4, 'nodata', $5, $6, $7, '{}', false)",
        )
        .bind(org)
        .bind(name)
        .bind(query)
        .bind(&options)
        .bind(message)
        .bind(priority)
        .bind(vec!["observecraft:self-monitoring".to_string(), "service:observecraft".to_string()])
        .execute(&st.postgres)
        .await?;
        created += 1;
    }
    Ok(created)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gauges_become_ingest_rows_of_the_operator_org() {
        let rows = rows("ops", 1_792_933_200_000, &[tagged("observecraft.clickhouse.disk.used_pct", &[("replica", "ch-0"), ("disk", "default")], 42.5), gauge("observecraft.up", f64::NAN)]);
        let lines: Vec<Value> = rows.split(|b| *b == b'\n').filter(|l| !l.is_empty()).map(|l| serde_json::from_slice(l).unwrap()).collect();
        assert_eq!(lines.len(), 1, "non-finite values are skipped");
        let r = &lines[0];
        assert_eq!(r["ResourceAttributes"]["oc.org_id"], "ops");
        assert_eq!(r["MetricName"], "observecraft.clickhouse.disk.used_pct");
        assert_eq!(r["Attributes"], json!({ "disk": "default", "replica": "ch-0" }));
        assert_eq!(r["Value"], 42.5);
        assert!(r["TimeUnix"].as_str().unwrap().starts_with("2026-"));
    }

    #[test]
    fn lag_sums_partitions_and_never_goes_negative() {
        let stored = vec![("oc.logs.json".to_string(), 0, 90), ("oc.logs.json".to_string(), 1, 50), ("oc.otlp.logs".to_string(), 0, 10)];
        let latest = HashMap::from([(("oc.logs.json".to_string(), 0), 100), (("oc.logs.json".to_string(), 1), 40), (("oc.otlp.logs".to_string(), 0), 10)]);
        assert_eq!(consumer_lag(&stored, &latest), BTreeMap::from([("oc.logs.json".to_string(), 10), ("oc.otlp.logs".to_string(), 0)]));
    }

    #[test]
    fn growth_needs_six_hours_and_projects_when_the_disk_fills() {
        let h = 3_600_000;
        assert_eq!(growth_per_day(&[(0, 100.0), (5 * h, 200.0)]), None);
        let per_day = growth_per_day(&[(0, 0.0), (6 * h, f64::NAN), (12 * h, 50e9)]).unwrap();
        assert!((per_day - 100e9).abs() < 1.0);
        assert_eq!(days_until_full(700e9, per_day), Some(7.0));
        assert_eq!(days_until_full(700e9, -5.0), None);
        // 100 GB/day for 30 days at $0.08/GB-month: half of 3 TB on average, $120.
        assert!((projected_monthly(500.0, per_day, 0.08) - 620.0).abs() < 1e-6);
    }

    #[test]
    fn ages_are_reported_only_for_tables_written_in_the_last_day() {
        let mut written = HashSet::new();
        assert!(report_age(&mut written, "otel_traces", 30));
        assert!(report_age(&mut written, "otel_traces", 3_600));
        assert!(!report_age(&mut written, "otel_traces", 90_000));
        assert!(!report_age(&mut written, "otel_rum", 400_000));
    }

    #[test]
    fn usage_reads_one_hour_of_every_org_without_formatting_values() {
        let sql = usage_sql("otel.otel_traces", "Timestamp", "OrgId");
        assert!(sql.contains("FROM otel.otel_traces WHERE Timestamp >= toDateTime(?) AND Timestamp < toDateTime(?) AND OrgId != ''"));
        assert!(sql.ends_with("GROUP BY OrgId"));
    }

    #[test]
    fn recommended_monitors_parse_and_notify_the_configured_handles() {
        let seeds = monitor_seeds("@slack-ops");
        assert!(seeds.len() >= 10);
        let names: HashSet<&str> = seeds.iter().map(|s| s.0).collect();
        assert_eq!(names.len(), seeds.len(), "names are unique (they key the seeding)");
        for (name, query, options, message, _) in &seeds {
            assert!(name.starts_with("[ObserveCraft] "));
            assert!(oc_query::parse_metric_query(query).is_ok(), "{query}");
            assert!(options["thresholds"]["critical"].is_number());
            assert!(message.ends_with("@slack-ops"), "{name}");
        }
        assert!(!monitor_seeds("")[0].3.contains('@'));
    }

    #[test]
    fn settings_default_to_on_with_the_observecraft_namespace() {
        let s = Settings::from_env();
        assert!(!s.namespace.is_empty());
        assert!(s.storage_gb_month >= 0.0);
    }
}
