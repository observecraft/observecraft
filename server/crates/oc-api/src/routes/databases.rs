//! Database Monitoring: the fleet of database instances, from the integration metrics they already
//! send (no Database Monitoring payloads: query samples and explain plans aren't collected):
//! - the Datadog Agent's checks: `postgresql.*`, `mysql.*`, `redis.*`, `mongodb.*`, `sqlserver.*`
//!   (names from `integrations-core/<check>/metadata.csv`, BSD-3-Clause, Copyright (c) Datadog, Inc.),
//! - the OpenTelemetry Collector's receivers: postgresql, mysql, redis, mongodb, sqlserver.
//!
//! An instance is one engine on one host (and port). Per-database series add up to the instance.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use axum::extract::State;
use axum::Json;
use oc_query::{matches, parse_query, Fields, Term};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use super::logs::TimeRange;
use super::metric_inventory::{chart_step, series_values, SeriesValue};
use super::metrics::{points_subquery, with_window, MetricSeries};
use crate::error::ApiResult;
use crate::state::{AppState, OrgId};

const MIB: f64 = 1024.0 * 1024.0;
/// Current values are the average of the last minutes of the range.
const CURRENT_MS: i64 = 10 * 60_000;

#[derive(Serialize, Deserialize, ToSchema, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
#[serde(rename_all = "snake_case")]
pub enum DbEngine {
    Postgres,
    Mysql,
    Redis,
    Mongodb,
    Sqlserver,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum DbField {
    /// Open client connections (sum over the instance's series).
    Connections,
    /// Connection limit (max).
    MaxConnections,
    /// Transactions, queries, commands, operations or batches per second (sum).
    Throughput,
    /// Cache hit ratio reported as such, in % (average).
    HitRatio,
    /// Cache hits, misses and lookups (hits + misses) per second (sum), for the hit ratio.
    Hits,
    Misses,
    Lookups,
    /// Replication lag in seconds (max).
    Lag,
    /// Size on disk (in memory for Redis), in bytes (sum).
    Size,
}

/// (metric, engine, field, scale, data point attribute that must have one of the values).
type DbMetric = (&'static str, DbEngine, DbField, f64, Option<(&'static str, &'static [&'static str])>);

const PG_HIT: &[&str] = &["heap_hit", "idx_hit", "toast_hit", "tidx_hit"];
const PG_READ: &[&str] = &["heap_read", "idx_read", "toast_read", "tidx_read"];

const DB_METRICS: &[DbMetric] = {
    use DbEngine::*;
    use DbField::*;
    &[
        // PostgreSQL: Datadog check, then the OpenTelemetry receiver (commits / rollbacks are both's names).
        ("postgresql.connections", Postgres, Connections, 1.0, None),
        ("postgresql.max_connections", Postgres, MaxConnections, 1.0, None),
        ("postgresql.commits", Postgres, Throughput, 1.0, None),
        ("postgresql.rollbacks", Postgres, Throughput, 1.0, None),
        ("postgresql.buffer_hit", Postgres, Hits, 1.0, None),
        ("postgresql.disk_read", Postgres, Misses, 1.0, None),
        ("postgresql.replication_delay", Postgres, Lag, 1.0, None),
        ("postgresql.database_size", Postgres, Size, 1.0, None),
        ("postgresql.backends", Postgres, Connections, 1.0, None),
        ("postgresql.connection.max", Postgres, MaxConnections, 1.0, None),
        ("postgresql.blocks_read", Postgres, Hits, 1.0, Some(("source", PG_HIT))),
        ("postgresql.blocks_read", Postgres, Misses, 1.0, Some(("source", PG_READ))),
        ("postgresql.wal.lag", Postgres, Lag, 1.0, None),
        ("postgresql.db_size", Postgres, Size, 1.0, None),
        // MySQL
        ("mysql.performance.threads_connected", Mysql, Connections, 1.0, None),
        ("mysql.net.max_connections_available", Mysql, MaxConnections, 1.0, None),
        ("mysql.performance.queries", Mysql, Throughput, 1.0, None),
        ("mysql.innodb.buffer_pool_read_requests", Mysql, Lookups, 1.0, None),
        ("mysql.innodb.buffer_pool_reads", Mysql, Misses, 1.0, None),
        ("mysql.replication.seconds_behind_source", Mysql, Lag, 1.0, None),
        ("mysql.replication.seconds_behind_master", Mysql, Lag, 1.0, None),
        ("mysql.info.schema.size", Mysql, Size, MIB, None),
        ("mysql.threads", Mysql, Connections, 1.0, Some(("kind", &["connected"]))),
        ("mysql.query.count", Mysql, Throughput, 1.0, None),
        ("mysql.buffer_pool.operations", Mysql, Lookups, 1.0, Some(("operation", &["read_requests"]))),
        ("mysql.buffer_pool.operations", Mysql, Misses, 1.0, Some(("operation", &["reads"]))),
        ("mysql.replica.time_behind_source", Mysql, Lag, 1.0, None),
        // Redis
        ("redis.net.clients", Redis, Connections, 1.0, None),
        ("redis.net.instantaneous_ops_per_sec", Redis, Throughput, 1.0, None),
        ("redis.stats.keyspace_hits", Redis, Hits, 1.0, None),
        ("redis.stats.keyspace_misses", Redis, Misses, 1.0, None),
        ("redis.mem.used", Redis, Size, 1.0, None),
        ("redis.clients.connected", Redis, Connections, 1.0, None),
        ("redis.commands", Redis, Throughput, 1.0, None),
        ("redis.keyspace.hits", Redis, Hits, 1.0, None),
        ("redis.keyspace.misses", Redis, Misses, 1.0, None),
        ("redis.memory.used", Redis, Size, 1.0, None),
        // MongoDB
        ("mongodb.connections.current", Mongodb, Connections, 1.0, None),
        ("mongodb.opcounters.queryps", Mongodb, Throughput, 1.0, None),
        ("mongodb.opcounters.insertps", Mongodb, Throughput, 1.0, None),
        ("mongodb.opcounters.updateps", Mongodb, Throughput, 1.0, None),
        ("mongodb.opcounters.deleteps", Mongodb, Throughput, 1.0, None),
        ("mongodb.opcounters.getmoreps", Mongodb, Throughput, 1.0, None),
        ("mongodb.opcounters.commandps", Mongodb, Throughput, 1.0, None),
        ("mongodb.replset.replicationlag", Mongodb, Lag, 1.0, None),
        ("mongodb.stats.storagesize", Mongodb, Size, 1.0, None),
        ("mongodb.connection.count", Mongodb, Connections, 1.0, Some(("type", &["current"]))),
        ("mongodb.operation.count", Mongodb, Throughput, 1.0, None),
        ("mongodb.storage.size", Mongodb, Size, 1.0, None),
        // SQL Server
        ("sqlserver.stats.connections", Sqlserver, Connections, 1.0, None),
        ("sqlserver.stats.batch_requests", Sqlserver, Throughput, 1.0, None),
        ("sqlserver.buffer.cache_hit_ratio", Sqlserver, HitRatio, 100.0, None),
        ("sqlserver.user.connection.count", Sqlserver, Connections, 1.0, None),
        ("sqlserver.batch.request.rate", Sqlserver, Throughput, 1.0, None),
        ("sqlserver.page.buffer_cache.hit_ratio", Sqlserver, HitRatio, 1.0, None),
    ]
};

const HOST_KEYS: &[&str] = &["server", "server.address", "redis_host", "host.name"];
const PORT_KEYS: &[&str] = &["port", "server.port", "redis_port"];
const VERSION_KEYS: &[&str] = &["db.version", "postgresql.version", "mysql.version", "redis.version", "mongodb.version", "sqlserver.version"];
const DATABASE_KEYS: &[&str] = &["db", "postgresql.database.name", "database", "db.name"];

fn metric_names() -> Vec<&'static str> {
    let mut names: Vec<&'static str> = DB_METRICS.iter().map(|m| m.0).collect();
    names.dedup();
    names
}

/// The engine and field of a series, when it is one of [`DB_METRICS`].
fn classify(v: &SeriesValue) -> Option<(DbEngine, DbField, f64)> {
    DB_METRICS.iter().find(|(name, _, _, _, filter)| *name == v.metric && filter.is_none_or(|(k, vals)| v.attrs.get(k).is_some_and(|x| vals.contains(&x.as_str())))).map(|(_, e, f, s, _)| (*e, *f, *s))
}

#[derive(Serialize, ToSchema, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct DbInstance {
    /// `engine:host[:port]`.
    pub id: String,
    pub engine: DbEngine,
    pub host: String,
    pub port: Option<String>,
    pub version: Option<String>,
    pub env: Option<String>,
    pub cluster: Option<String>,
    pub service: Option<String>,
    /// Databases its series are tagged with.
    pub databases: Vec<String>,
    pub connections: Option<f64>,
    pub max_connections: Option<f64>,
    /// Transactions (PostgreSQL), queries (MySQL), commands (Redis), operations (MongoDB) or batches (SQL Server) per second.
    pub throughput: Option<f64>,
    pub cache_hit_pct: Option<f64>,
    pub replication_lag_sec: Option<f64>,
    /// On disk (in memory for Redis).
    pub size_bytes: Option<f64>,
    pub last_seen: i64,
}

#[derive(Default)]
struct Acc {
    engine: Option<DbEngine>,
    host: String,
    port: Option<String>,
    version: Option<String>,
    env: Option<String>,
    cluster: Option<String>,
    service: Option<String>,
    databases: BTreeSet<String>,
    sums: HashMap<&'static str, f64>,
    maxes: HashMap<&'static str, f64>,
    ratios: Vec<f64>,
    last: i64,
}

fn round2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

fn instance_id(engine: DbEngine, host: &str, port: Option<&str>) -> String {
    let e = serde_json::to_value(engine).ok().and_then(|v| v.as_str().map(str::to_string)).unwrap_or_default();
    match port {
        Some(p) => format!("{e}:{host}:{p}"),
        None => format!("{e}:{host}"),
    }
}

/// The instances behind `values` (one window), sorted by engine and host.
pub(crate) fn aggregate_instances(values: &[SeriesValue]) -> Vec<DbInstance> {
    let mut accs: BTreeMap<String, Acc> = BTreeMap::new();
    for v in values {
        let Some((engine, field, scale)) = classify(v) else { continue };
        let host = v.tag(HOST_KEYS).unwrap_or_else(|| "unknown".into());
        let port = v.tag(PORT_KEYS);
        let a = accs.entry(instance_id(engine, &host, port.as_deref())).or_default();
        a.engine = Some(engine);
        a.host = host;
        a.port = port;
        let fill = |slot: &mut Option<String>, keys: &[&str]| {
            if slot.is_none() {
                *slot = v.tag(keys);
            }
        };
        fill(&mut a.version, VERSION_KEYS);
        fill(&mut a.env, &["deployment.environment"]);
        fill(&mut a.cluster, &["k8s.cluster.name"]);
        fill(&mut a.service, &["service.name"]);
        a.databases.extend(v.tag(DATABASE_KEYS));
        a.last = a.last.max(v.last);
        let value = v.value * scale;
        match field {
            DbField::MaxConnections | DbField::Lag => {
                let k = if field == DbField::Lag { "lag" } else { "max" };
                let m = a.maxes.entry(k).or_insert(f64::MIN);
                *m = m.max(value);
            }
            DbField::HitRatio => a.ratios.push(value),
            f => *a.sums.entry(sum_key(f)).or_default() += value,
        }
    }
    accs.into_iter()
        .filter_map(|(id, a)| {
            let engine = a.engine?;
            let sum = |k: &str| a.sums.get(k).copied();
            let hits = sum("hits");
            let misses = sum("misses");
            let cache_hit = if !a.ratios.is_empty() {
                Some(a.ratios.iter().sum::<f64>() / a.ratios.len() as f64)
            } else if let Some(lookups) = sum("lookups").filter(|l| *l > 0.0) {
                Some((1.0 - misses.unwrap_or(0.0) / lookups) * 100.0)
            } else {
                match (hits, misses) {
                    (Some(h), m) if h + m.unwrap_or(0.0) > 0.0 => Some(h / (h + m.unwrap_or(0.0)) * 100.0),
                    _ => None,
                }
            };
            Some(DbInstance {
                id,
                engine,
                host: a.host,
                port: a.port,
                version: a.version,
                env: a.env,
                cluster: a.cluster,
                service: a.service,
                databases: a.databases.into_iter().collect(),
                connections: sum("connections").map(f64::round),
                max_connections: a.maxes.get("max").copied(),
                throughput: sum("throughput").map(round2),
                cache_hit_pct: cache_hit.map(|c| round2(c.clamp(0.0, 100.0))),
                replication_lag_sec: a.maxes.get("lag").copied().map(round2),
                size_bytes: sum("size").map(f64::round),
                last_seen: a.last,
            })
        })
        .collect()
}

fn sum_key(f: DbField) -> &'static str {
    match f {
        DbField::Connections => "connections",
        DbField::Throughput => "throughput",
        DbField::Hits => "hits",
        DbField::Misses => "misses",
        DbField::Lookups => "lookups",
        DbField::Size => "size",
        DbField::MaxConnections => "max",
        DbField::Lag => "lag",
        DbField::HitRatio => "ratio",
    }
}

fn engine_str(e: DbEngine) -> String {
    serde_json::to_value(e).ok().and_then(|v| v.as_str().map(str::to_string)).unwrap_or_default()
}

/// Facet values of an instance: `engine`, `env`, `cluster`, `host`, `service`.
fn instance_tags(i: &DbInstance) -> HashMap<String, String> {
    let mut tags = HashMap::from([("engine".to_string(), engine_str(i.engine)), ("host".to_string(), i.host.clone())]);
    for (k, v) in [("env", &i.env), ("cluster", &i.cluster), ("service", &i.service)] {
        if let Some(v) = v {
            tags.insert(k.to_string(), v.clone());
        }
    }
    tags
}

fn instance_matches(i: &DbInstance, terms: &[Term]) -> bool {
    let tags = instance_tags(i);
    let empty = HashMap::new();
    let text = format!("{} {}", i.id, i.databases.join(" "));
    matches(terms, &Fields { service: i.service.as_deref().unwrap_or(""), status: "", host: &i.host, source: "", message: &text, tags: &tags, attr: &empty, index: None })
}

const FACET_KEYS: &[&str] = &["engine", "env", "cluster", "host", "service"];

#[derive(Serialize, ToSchema)]
pub struct DbFacetValue {
    pub value: String,
    pub count: usize,
}

#[derive(Serialize, ToSchema)]
pub struct DbFacet {
    pub key: String,
    pub values: Vec<DbFacetValue>,
}

fn facets(instances: &[DbInstance]) -> Vec<DbFacet> {
    FACET_KEYS
        .iter()
        .map(|k| {
            let mut counts: BTreeMap<String, usize> = BTreeMap::new();
            for i in instances {
                if let Some(v) = instance_tags(i).remove(*k) {
                    *counts.entry(v).or_default() += 1;
                }
            }
            let mut values: Vec<DbFacetValue> = counts.into_iter().map(|(value, count)| DbFacetValue { value, count }).collect();
            values.sort_by(|a, b| b.count.cmp(&a.count).then(a.value.cmp(&b.value)));
            DbFacet { key: k.to_string(), values }
        })
        .filter(|f| !f.values.is_empty())
        .collect()
}

#[derive(Deserialize, ToSchema)]
pub struct DbSearchRequest {
    pub range: TimeRange,
    /// `engine:postgres env:prod cluster:x host:y service:z` + free text on the instance and its databases.
    #[serde(default)]
    pub query: String,
}

#[derive(Serialize, ToSchema)]
pub struct DbSearchPage {
    /// False when no database integration metrics arrived in the range (show the setup guide).
    pub enabled: bool,
    pub instances: Vec<DbInstance>,
    pub facets: Vec<DbFacet>,
    /// Database servers seen running on hosts (process metrics) whose engine sends no integration metrics.
    pub unconfigured: Vec<UnconfiguredDb>,
}

#[derive(Serialize, ToSchema, Clone, Debug, PartialEq)]
pub struct UnconfiguredDb {
    pub host: String,
    pub engine: DbEngine,
    /// The process seen (`postgres`, `mysqld`, …).
    pub process: String,
}

/// Database server executables, by engine.
const DB_PROCESSES: &[(&str, DbEngine)] = &[
    ("postgres", DbEngine::Postgres),
    ("postmaster", DbEngine::Postgres),
    ("mysqld", DbEngine::Mysql),
    ("mariadbd", DbEngine::Mysql),
    ("redis-server", DbEngine::Redis),
    ("mongod", DbEngine::Mongodb),
    ("sqlservr", DbEngine::Sqlserver),
];

/// Hosts running a database server (host, executable) that no instance of its engine reports from.
pub(crate) fn unconfigured(processes: &[(String, String)], instances: &[DbInstance]) -> Vec<UnconfiguredDb> {
    let mut out: Vec<UnconfiguredDb> = processes
        .iter()
        .filter_map(|(host, exe)| {
            let engine = DB_PROCESSES.iter().find(|(name, _)| *name == exe.as_str())?.1;
            let configured = instances.iter().any(|i| i.engine == engine && i.host == *host);
            (!configured).then(|| UnconfiguredDb { host: host.clone(), engine, process: exe.clone() })
        })
        .collect();
    out.sort_by(|a, b| a.host.cmp(&b.host).then(a.engine.cmp(&b.engine)));
    out.dedup_by(|a, b| a.host == b.host && a.engine == b.engine);
    out
}

/// (host, executable) of the database server processes hosts reported in the window.
async fn db_processes(st: &AppState, org: &str, from: i64, to: i64) -> Result<Vec<(String, String)>, crate::error::ApiError> {
    #[derive(clickhouse::Row, Deserialize)]
    struct Row {
        host: String,
        exe: String,
    }
    let src = points_subquery("gauge", &["process.cpu.utilization"]);
    let exes: Vec<&str> = DB_PROCESSES.iter().map(|(n, _)| *n).collect();
    let rows = with_window(st.ch(org), from, to)
        .query(&format!(
            "SELECT ResourceAttributes['host.name'] AS host, ResourceAttributes['process.executable.name'] AS exe FROM {src} \
             WHERE OrgId = {{oc_org:String}} AND MetricName = 'process.cpu.utilization' AND host != '' AND exe IN ? GROUP BY host, exe LIMIT 5000"
        ))
        .bind(exes)
        .fetch_all::<Row>()
        .await?;
    Ok(rows.into_iter().map(|r| (r.host, r.exe)).collect())
}

/// Database instances that reported in the last minutes of the range, with their current values.
#[utoipa::path(post, path = "/api/v1/databases/search", operation_id = "search_databases", request_body = DbSearchRequest, responses((status = 200, body = DbSearchPage)), tag = "databases")]
pub async fn search(State(st): State<AppState>, OrgId(org): OrgId, Json(req): Json<DbSearchRequest>) -> ApiResult<DbSearchPage> {
    req.range.validate()?;
    let from = req.range.from.max(req.range.to - CURRENT_MS);
    let names = metric_names();
    let (values, processes) = tokio::try_join!(series_values(&st, &org, &names, from, req.range.to, None), db_processes(&st, &org, from, req.range.to))?;
    let all = aggregate_instances(&values);
    let terms = parse_query(&req.query);
    let instances: Vec<DbInstance> = all.iter().filter(|i| instance_matches(i, &terms)).cloned().collect();
    Ok(Json(DbSearchPage { enabled: !all.is_empty(), facets: facets(&all), unconfigured: unconfigured(&processes, &all), instances }))
}

#[derive(Deserialize, ToSchema)]
pub struct DbInstanceRequest {
    pub range: TimeRange,
    /// The instance's `id` from the search.
    pub id: String,
}

#[derive(Serialize, ToSchema)]
pub struct DbInstanceDetail {
    /// Null when the instance didn't report in the range.
    pub instance: Option<DbInstance>,
    /// connections, throughput, cache_hit, replication_lag, size.
    pub series: Vec<MetricSeries>,
}

/// One instance's key series per interval.
pub(crate) fn instance_series(values: &[SeriesValue], id: &str) -> Vec<MetricSeries> {
    let mut by_t: BTreeMap<i64, Vec<SeriesValue>> = BTreeMap::new();
    for v in values {
        by_t.entry(v.t).or_default().push(v.clone());
    }
    type Pick = fn(&DbInstance) -> Option<f64>;
    let picks: [(&str, Pick, &str); 5] = [
        ("connections", |i| i.connections, "connection"),
        ("throughput", |i| i.throughput, "operation/second"),
        ("cache_hit", |i| i.cache_hit_pct, "percent"),
        ("replication_lag", |i| i.replication_lag_sec, "second"),
        ("size", |i| i.size_bytes, "byte"),
    ];
    let mut points: Vec<Vec<(i64, f64)>> = vec![Vec::new(); picks.len()];
    for (t, vs) in by_t {
        if let Some(i) = aggregate_instances(&vs).into_iter().find(|i| i.id == id) {
            for (n, (_, pick, _)) in picks.iter().enumerate() {
                if let Some(v) = pick(&i) {
                    points[n].push((t, v));
                }
            }
        }
    }
    picks.iter().zip(points).map(|((name, _, unit), points)| MetricSeries { name: name.to_string(), points, unit: Some(unit.to_string()) }).collect()
}

/// An instance's current values and its connections, throughput, cache hit ratio, replication lag and size over the range.
#[utoipa::path(post, path = "/api/v1/databases/instance", operation_id = "database_instance", request_body = DbInstanceRequest, responses((status = 200, body = DbInstanceDetail)), tag = "databases")]
pub async fn instance(State(st): State<AppState>, OrgId(org): OrgId, Json(req): Json<DbInstanceRequest>) -> ApiResult<DbInstanceDetail> {
    req.range.validate()?;
    let step = chart_step(req.range.to - req.range.from, 60);
    let names = metric_names();
    let from = req.range.from.max(req.range.to - CURRENT_MS);
    let (current, stepped) = tokio::try_join!(series_values(&st, &org, &names, from, req.range.to, None), series_values(&st, &org, &names, req.range.from, req.range.to, Some(step)))?;
    let instance = aggregate_instances(&current).into_iter().find(|i| i.id == req.id);
    Ok(Json(DbInstanceDetail { instance, series: instance_series(&stepped, &req.id) }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sv(metric: &str, value: f64, res: &[(&str, &str)], attrs: &[(&str, &str)]) -> SeriesValue {
        let map = |kv: &[(&str, &str)]| kv.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        SeriesValue { metric: metric.into(), res: map(res), attrs: map(attrs), t: 0, value, max: value, count: 1, last: 7 }
    }

    const PG_HOST: &[(&str, &str)] = &[("host.name", "i-0a1b2c3d4e5f60010"), ("deployment.environment", "prod")];

    #[test]
    fn datadog_postgres_check_adds_up_per_database() {
        let db = |d: &'static str| [("db", d), ("port", "5432")];
        let instances = aggregate_instances(&[
            sv("postgresql.connections", 12.0, PG_HOST, &db("orders")),
            sv("postgresql.connections", 8.0, PG_HOST, &db("catalog")),
            sv("postgresql.max_connections", 200.0, PG_HOST, &db("orders")),
            sv("postgresql.commits", 140.0, PG_HOST, &db("orders")),
            sv("postgresql.rollbacks", 2.5, PG_HOST, &db("orders")),
            sv("postgresql.buffer_hit", 990.0, PG_HOST, &db("orders")),
            sv("postgresql.disk_read", 10.0, PG_HOST, &db("orders")),
            sv("postgresql.replication_delay", 0.4, PG_HOST, &db("orders")),
            sv("postgresql.database_size", 1e9, PG_HOST, &db("orders")),
            sv("postgresql.database_size", 5e8, PG_HOST, &db("catalog")),
        ]);
        assert_eq!(instances.len(), 1);
        let i = &instances[0];
        assert_eq!(i.id, "postgres:i-0a1b2c3d4e5f60010:5432");
        assert_eq!(i.engine, DbEngine::Postgres);
        assert_eq!(i.env.as_deref(), Some("prod"));
        assert_eq!(i.databases, vec!["catalog", "orders"]);
        assert_eq!(i.connections, Some(20.0));
        assert_eq!(i.max_connections, Some(200.0));
        assert_eq!(i.throughput, Some(142.5), "commits + rollbacks");
        assert_eq!(i.cache_hit_pct, Some(99.0));
        assert_eq!(i.replication_lag_sec, Some(0.4));
        assert_eq!(i.size_bytes, Some(1.5e9));
        assert_eq!(i.last_seen, 7);
    }

    #[test]
    fn otel_postgres_receiver_blocks_by_source() {
        let res = [("host.name", "i-0a1b2c3d4e5f60011"), ("postgresql.database.name", "orders")];
        let instances = aggregate_instances(&[
            sv("postgresql.backends", 5.0, &res, &[]),
            sv("postgresql.blocks_read", 300.0, &res, &[("source", "heap_hit")]),
            sv("postgresql.blocks_read", 100.0, &res, &[("source", "idx_hit")]),
            sv("postgresql.blocks_read", 100.0, &res, &[("source", "heap_read")]),
            sv("postgresql.db_size", 2048.0, &res, &[]),
        ]);
        let i = &instances[0];
        assert_eq!(i.port, None);
        assert_eq!(i.id, "postgres:i-0a1b2c3d4e5f60011");
        assert_eq!(i.connections, Some(5.0));
        assert_eq!(i.cache_hit_pct, Some(80.0));
        assert_eq!(i.databases, vec!["orders"]);
    }

    #[test]
    fn mysql_hit_ratio_from_lookups_and_attribute_filters() {
        let res = [("host.name", "i-0a1b2c3d4e5f60012")];
        let instances = aggregate_instances(&[
            sv("mysql.innodb.buffer_pool_read_requests", 1000.0, &res, &[]),
            sv("mysql.innodb.buffer_pool_reads", 5.0, &res, &[]),
            sv("mysql.info.schema.size", 10.0, &res, &[("schema", "shop")]),
            sv("mysql.threads", 9.0, &res, &[("kind", "connected")]),
            sv("mysql.threads", 3.0, &res, &[("kind", "running")]),
        ]);
        let i = &instances[0];
        assert_eq!(i.cache_hit_pct, Some(99.5));
        assert_eq!(i.size_bytes, Some(10.0 * MIB));
        assert_eq!(i.connections, Some(9.0), "only connected threads");
    }

    #[test]
    fn engines_are_separate_instances_with_facets_and_query() {
        let res = [("host.name", "i-0a1b2c3d4e5f60013"), ("deployment.environment", "staging")];
        let instances = aggregate_instances(&[
            sv("redis.net.clients", 40.0, &res, &[("redis_port", "6379")]),
            sv("redis.stats.keyspace_hits", 90.0, &res, &[("redis_port", "6379")]),
            sv("redis.stats.keyspace_misses", 10.0, &res, &[("redis_port", "6379")]),
            sv("sqlserver.buffer.cache_hit_ratio", 0.97, &res, &[]),
            sv("not.a.db.metric", 1.0, &res, &[]),
        ]);
        assert_eq!(instances.len(), 2);
        let redis = instances.iter().find(|i| i.engine == DbEngine::Redis).unwrap();
        assert_eq!(redis.cache_hit_pct, Some(90.0));
        assert_eq!(redis.id, "redis:i-0a1b2c3d4e5f60013:6379");
        let sql = instances.iter().find(|i| i.engine == DbEngine::Sqlserver).unwrap();
        assert_eq!(sql.cache_hit_pct, Some(97.0));
        let f = facets(&instances);
        assert_eq!(f.iter().find(|f| f.key == "engine").unwrap().values.len(), 2);
        assert_eq!(instances.iter().filter(|i| instance_matches(i, &parse_query("engine:redis env:staging"))).count(), 1);
    }

    #[test]
    fn instance_series_follow_one_instance() {
        let res = [("host.name", "i-0a1b2c3d4e5f60010")];
        let at = |t: i64, m: &str, v: f64| SeriesValue { t, ..sv(m, v, &res, &[]) };
        let s = instance_series(&[at(0, "postgresql.connections", 3.0), at(60_000, "postgresql.connections", 4.0), at(60_000, "redis.net.clients", 9.0)], "postgres:i-0a1b2c3d4e5f60010");
        assert_eq!(s[0].name, "connections");
        assert_eq!(s[0].points, vec![(0, 3.0), (60_000, 4.0)]);
        assert!(s[1].points.is_empty());
    }

    #[test]
    fn database_processes_without_their_integration() {
        let res = [("host.name", "i-0a1b2c3d4e5f60010")];
        let instances = aggregate_instances(&[sv("postgresql.connections", 1.0, &res, &[])]);
        let procs: Vec<(String, String)> =
            [("i-0a1b2c3d4e5f60010", "postgres"), ("i-0a1b2c3d4e5f60010", "redis-server"), ("i-0a1b2c3d4e5f60011", "mysqld"), ("i-0a1b2c3d4e5f60011", "mysqld"), ("i-0a1b2c3d4e5f60011", "nginx")]
                .iter()
                .map(|(h, e)| (h.to_string(), e.to_string()))
                .collect();
        let u = unconfigured(&procs, &instances);
        assert_eq!(
            u,
            vec![
                UnconfiguredDb { host: "i-0a1b2c3d4e5f60010".into(), engine: DbEngine::Redis, process: "redis-server".into() },
                UnconfiguredDb { host: "i-0a1b2c3d4e5f60011".into(), engine: DbEngine::Mysql, process: "mysqld".into() },
            ]
        );
    }

    #[test]
    fn every_metric_name_is_listed_once() {
        let names = metric_names();
        let unique: BTreeSet<_> = names.iter().collect();
        assert_eq!(names.len(), unique.len());
    }
}
