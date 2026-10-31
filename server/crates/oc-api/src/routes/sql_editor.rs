//! Dashboards → SQL Editor: read-only SQL over the org's data.
//!
//! Users query virtual tables (`hosts`, `spans`, `logs`, `monitors`, …), never stored tables.
//! [`oc_query::sql_editor::check_sql`] accepts one SELECT built from allowed syntax and functions and
//! turns every table it names into a placeholder; this module replaces each placeholder with a
//! subquery that is already limited to the caller's org:
//! - telemetry tables read ClickHouse with `OrgId = {oc_org:String}` / `org_id = ?` and the selected
//!   time range (logs also apply the reader's log restrictions);
//! - inventory tables (hosts, containers, Kubernetes, resources) and metadata tables (monitors,
//!   dashboards, incidents, Synthetic tests) are read here for the org (`org_id = $1` in Postgres)
//!   and passed to ClickHouse as one bound JSON value per table.
//!
//! The whole query then runs in ClickHouse through `AppState::ch(org)` with `readonly = 2`, a time
//! and memory limit, and at most [`MAX_ROWS`] rows returned. Saved queries live in Postgres
//! (`sql_queries`).

use std::collections::HashMap;
use std::sync::LazyLock;
use std::time::Instant;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use oc_query::sql_editor::{check_sql, CheckedSql, SqlPiece, ALLOWED_FUNCTIONS, MAX_SQL_CHARS};
use oc_query::Param;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use utoipa::ToSchema;

use super::catalog::CatalogResource;
use super::hosts::{enum_str, HostRow};
use super::kubernetes::{KubeKind, KubeResource};
use super::logs::{bind, TimeRange};
use crate::auth::{AuthContext, Writer};
use crate::error::{ApiError, ApiResult};
use crate::log_access::LogAccess;
use crate::state::AppState;

/// Rows a query returns at most.
pub const MAX_ROWS: u64 = 10_000;
/// Rows of one inventory or metadata table passed to a query.
const MAX_TABLE_ROWS: usize = 10_000;
/// Bytes of results read from ClickHouse at most.
const MAX_RESULT_BYTES: usize = 64 << 20;
const MAX_EXECUTION_SECS: u32 = 30;
const MAX_MEMORY_BYTES: u64 = 2 << 30;
/// Longest time range a query covers.
const MAX_RANGE_DAYS: u32 = 7;
const DEFAULT_RANGE_MS: i64 = 3_600_000;
const MAX_SAVED_QUERIES: i64 = 1_000;
const MAX_NAME_CHARS: usize = 200;

// ================================================================ virtual tables

/// One column of a virtual table: its ClickHouse type and, for tables read from ClickHouse or
/// Postgres, the expression that produces it.
struct Col {
    name: &'static str,
    ty: &'static str,
    doc: &'static str,
    expr: &'static str,
}

const fn col(name: &'static str, ty: &'static str, doc: &'static str, expr: &'static str) -> Col {
    Col { name, ty, doc, expr }
}

/// Values bound to the `?` of a ClickHouse source, in order.
#[derive(Clone, Copy)]
enum Bind {
    From,
    To,
}

enum Source {
    /// Columns' expressions over `rest` (FROM … WHERE <org filter> …).
    ClickHouse { rest: &'static str, binds: &'static [Bind] },
    /// Indexed logs, read through the Logs pages' scope (org, time range and the reader's log restrictions).
    Logs,
    /// Postgres rows as JSON arrays: columns' expressions over `rest` (FROM … WHERE … org_id = $1 … LIMIT).
    Postgres { rest: &'static str, order: &'static str },
    /// Built from what the infrastructure pages load.
    Inventory(Inventory),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Inventory {
    Hosts,
    Containers,
    Kube(KubeKind),
    Resources,
}

struct VirtualTable {
    name: &'static str,
    category: &'static str,
    doc: &'static str,
    cols: &'static [Col],
    source: Source,
}

const SPAN_RANGE: &[Bind] = &[Bind::From, Bind::To];
const METRIC_RANGE: &[Bind] = &[Bind::From, Bind::From, Bind::To];

const SPAN_COLS: &[Col] = &[
    col("timestamp", "DateTime64(9)", "Start of the span", "Timestamp"),
    col("trace_id", "String", "Trace id", "TraceId"),
    col("span_id", "String", "Span id", "SpanId"),
    col("parent_id", "String", "Parent span id ('' for a root span)", "ParentSpanId"),
    col("service", "String", "Service name", "ServiceName"),
    col("name", "String", "Operation or endpoint", "SpanName"),
    col("kind", "String", "Server, Client, Consumer, Producer or Internal", "toString(SpanKind)"),
    col("status", "String", "ok, error or unset", "lower(toString(StatusCode))"),
    col("duration_ms", "Float64", "Duration in milliseconds", "Duration / 1e6"),
    col("env", "String", "deployment.environment", "ResourceAttributes['deployment.environment']"),
    col("version", "String", "service.version", "ResourceAttributes['service.version']"),
    col("host", "String", "host.name", "ResourceAttributes['host.name']"),
    col(
        "http_status_code",
        "String",
        "http.status_code / http.response.status_code",
        "if(SpanAttributes['http.status_code'] != '', SpanAttributes['http.status_code'], SpanAttributes['http.response.status_code'])",
    ),
    col("attributes", "Map(String, String)", "Span attributes", "CAST(SpanAttributes, 'Map(String, String)')"),
    col("resource", "Map(String, String)", "Resource attributes", "CAST(mapFilter((k, v) -> NOT startsWith(k, 'oc.'), ResourceAttributes), 'Map(String, String)')"),
];

const SERVICE_COLS: &[Col] = &[
    col("service", "String", "Service name", "ServiceName"),
    col("env", "String", "deployment.environment", "ResourceAttributes['deployment.environment']"),
    col("requests", "UInt64", "Entry spans (server and consumer) in the range", "count()"),
    col("errors", "UInt64", "Entry spans with an error status", "countIf(StatusCode = 'Error')"),
    col("error_rate", "Float64", "errors / requests (0 to 1)", "countIf(StatusCode = 'Error') / count()"),
    col("avg_ms", "Float64", "Average latency, ms", "avg(Duration) / 1e6"),
    col("p50_ms", "Float64", "Median latency, ms", "quantile(0.5)(Duration) / 1e6"),
    col("p95_ms", "Float64", "p95 latency, ms", "quantile(0.95)(Duration) / 1e6"),
    col("p99_ms", "Float64", "p99 latency, ms", "quantile(0.99)(Duration) / 1e6"),
    col("hosts", "UInt64", "Distinct hosts serving it", "uniq(ResourceAttributes['host.name'])"),
    col("version", "String", "Latest service.version seen", "argMax(ResourceAttributes['service.version'], Timestamp)"),
];

const LOG_COLS: &[Col] = &[
    col("timestamp", "DateTime64(3)", "Time of the log", "timestamp"),
    col("status", "String", "Level: debug, info, warn, error, critical, …", "status"),
    col("service", "String", "Service", "service"),
    col("host", "String", "Host", "host"),
    col("source", "String", "Source (integration or language)", "source"),
    col("message", "String", "Message", "message"),
    col("index", "String", "Log index", "index_name"),
    col("trace_id", "String", "Trace id of a correlated span", "trace_id"),
    col("tags", "Map(String, String)", "Infrastructure tags (env, kube_namespace, …)", "tags"),
    col("attributes", "Map(String, String)", "Event attributes, flattened (http.status_code, …)", "attr"),
];

const METRIC_COLS: &[Col] = &[
    col("name", "String", "Metric name", "s.MetricName"),
    col("type", "String", "gauge, sum, histogram, …", "s.MetricType"),
    col("unit", "String", "Unit", "s.MetricUnit"),
    col("service", "String", "Service of the series", "s.ServiceName"),
    col(
        "tags",
        "Map(String, String)",
        "Series tags (resource and point attributes)",
        "CAST(mapUpdate(mapFilter((k, v) -> NOT startsWith(k, 'oc.'), s.ResourceAttributes), s.Attributes), 'Map(String, String)')",
    ),
    col("value", "Float64", "Latest value in the range", "l.value"),
    col("timestamp", "DateTime64(3)", "Time of the latest value", "l.ts"),
];

const CI_COLS: &[Col] = &[
    col("timestamp", "DateTime64(9)", "Start of the run", "Timestamp"),
    col("pipeline", "String", "Pipeline name", "SpanName"),
    col("repository", "String", "Repository", "ResourceAttributes['vcs.repository.name']"),
    col("branch", "String", "Branch", "ResourceAttributes['vcs.ref.head.name']"),
    col("commit_sha", "String", "Commit", "ResourceAttributes['vcs.ref.head.revision']"),
    col("provider", "String", "CI provider", "ResourceAttributes['cicd.provider.name']"),
    col("status", "String", "success, error, canceled or skipped", "__CI_STATUS__"),
    col("duration_ms", "Float64", "Duration in milliseconds", "Duration / 1e6"),
    col("run_id", "String", "Run id", "ResourceAttributes['cicd.pipeline.run.id']"),
    col("run_url", "String", "Link to the run", "ResourceAttributes['cicd.pipeline.run.url.full']"),
    col("team", "String", "Owning team", "ResourceAttributes['team']"),
    col("trace_id", "String", "Trace of the run (its jobs and steps)", "TraceId"),
];

const HOST_COLS: &[Col] = &[
    col("name", "String", "Host name", ""),
    col("cloud", "String", "aws, azure, gcp, … or none", ""),
    col("os", "String", "linux, windows or darwin", ""),
    col("distro", "Nullable(String)", "OS distribution", ""),
    col("env", "Nullable(String)", "env tag", ""),
    col("service", "Nullable(String)", "Main service on the host", ""),
    col("team", "Nullable(String)", "team tag", ""),
    col("region", "Nullable(String)", "cloud.region", ""),
    col("zone", "Nullable(String)", "cloud.availability_zone", ""),
    col("account", "Nullable(String)", "cloud.account.id", ""),
    col("instance_type", "Nullable(String)", "host.type", ""),
    col("cluster", "Nullable(String)", "Kubernetes cluster", ""),
    col("cpu_pct", "Float64", "CPU used, %", ""),
    col("mem_pct", "Float64", "Memory used, %", ""),
    col("error_spans", "Nullable(UInt64)", "APM error spans from the host in the range", ""),
    col("collectors", "Array(String)", "observecraft, otel, datadog", ""),
    col("apps", "Array(String)", "Integrations the host runs", ""),
    col("gpu", "Bool", "Whether it reports GPUs", ""),
    col("last_seen", "DateTime64(3)", "Latest report", ""),
];

const CONTAINER_COLS: &[Col] = &[
    col("uid", "String", "Id (pod uid / container name)", ""),
    col("name", "String", "Container name", ""),
    col("pod", "Nullable(String)", "Pod (or ECS task)", ""),
    col("owner_kind", "Nullable(String)", "Pod or Task", ""),
    col("cluster", "String", "Cluster", ""),
    col("namespace", "Nullable(String)", "Namespace", ""),
    col("node", "Nullable(String)", "Node or host", ""),
    col("image", "Nullable(String)", "Image", ""),
    col("status", "String", "Running, Waiting, Terminated, CrashLoopBackOff, …", ""),
    col("restarts", "UInt32", "Restarts", ""),
    col("cpu_cores", "Nullable(Float64)", "CPU used, cores", ""),
    col("memory_bytes", "Nullable(Float64)", "Memory used, bytes", ""),
    col("memory_limit_bytes", "Nullable(Float64)", "Memory limit, bytes", ""),
    col("started_at", "Nullable(DateTime64(3))", "Start time", ""),
    col("tags", "Array(String)", "Tags (kube_namespace:…, image_tag:…)", ""),
];

const KUBE_COLS: &[Col] = &[
    col("uid", "String", "Object uid", ""),
    col("cluster", "String", "Cluster", ""),
    col("namespace", "Nullable(String)", "Namespace", ""),
    col("name", "String", "Name", ""),
    col("status", "String", "Status (Running, Pending, Ready, …)", ""),
    col("health", "String", "ok, warning, error or info", ""),
    col("owner_kind", "Nullable(String)", "Owner kind (ReplicaSet, Deployment, …)", ""),
    col("owner_name", "Nullable(String)", "Owner name", ""),
    col("node", "Nullable(String)", "Node (pods)", ""),
    col("ready", "Nullable(String)", "Ready containers or replicas, e.g. 2/3", ""),
    col("restarts", "Nullable(UInt32)", "Container restarts (pods)", ""),
    col("desired", "Nullable(UInt32)", "Desired replicas", ""),
    col("available", "Nullable(UInt32)", "Available replicas", ""),
    col("cpu_usage", "Nullable(Float64)", "CPU used, cores", ""),
    col("cpu_requests", "Nullable(Float64)", "CPU requested, cores", ""),
    col("cpu_limits", "Nullable(Float64)", "CPU limit, cores", ""),
    col("memory_usage", "Nullable(Float64)", "Memory used, bytes", ""),
    col("memory_requests", "Nullable(Float64)", "Memory requested, bytes", ""),
    col("memory_limits", "Nullable(Float64)", "Memory limit, bytes", ""),
    col("version", "Nullable(String)", "Kubernetes version (nodes, clusters)", ""),
    col("images", "Array(String)", "Container images", ""),
    col("labels", "Map(String, String)", "Labels", ""),
    col("tags", "Array(String)", "Tags", ""),
    col("created_at", "Nullable(DateTime64(3))", "Creation time", ""),
    col("last_seen", "DateTime64(3)", "Latest snapshot", ""),
];

const RESOURCE_COLS: &[Col] = &[
    col("id", "String", "Resource id", ""),
    col("type", "String", "host, kube_deployment, aws_ecs_task, …", ""),
    col("category", "String", "Resource category", ""),
    col("name", "String", "Name", ""),
    col("cloud", "Nullable(String)", "Cloud provider", ""),
    col("account", "Nullable(String)", "Cloud account", ""),
    col("region", "Nullable(String)", "Region", ""),
    col("cluster", "Nullable(String)", "Kubernetes or ECS cluster", ""),
    col("namespace", "Nullable(String)", "Namespace", ""),
    col("env", "Nullable(String)", "env tag", ""),
    col("service", "Nullable(String)", "service tag", ""),
    col("team", "Nullable(String)", "team tag", ""),
    col("status", "Nullable(String)", "Status", ""),
    col("created_at", "Nullable(DateTime64(3))", "Creation time", ""),
    col("last_seen", "DateTime64(3)", "Latest report", ""),
    col("tags", "Array(String)", "Tags", ""),
    col("attributes", "Map(String, String)", "Type-specific properties", ""),
];

const MONITOR_COLS: &[Col] = &[
    col("id", "Int64", "Monitor id", "m.id"),
    col("name", "String", "Name", "m.name"),
    col("type", "String", "metric, apm, log or synthetics", "m.type"),
    col("status", "String", "ok, warn, alert or nodata", "m.status"),
    col("query", "String", "Monitor query", "m.query"),
    col("priority", "Nullable(Int64)", "P1 to P5", "m.priority"),
    col("tags", "Array(String)", "Tags", "m.tags"),
    col("teams", "Array(String)", "Owning teams", "m.teams"),
    col("draft", "Bool", "Saved as draft", "m.draft"),
    col("alerting_groups", "Int64", "Groups in alert", "(SELECT count(*) FROM monitor_groups g WHERE g.monitor_id = m.id AND g.status = 'alert')"),
    col("state_changed_at", "DateTime64(3)", "Last status change", "(extract(epoch from m.state_changed_at) * 1000)::bigint"),
    col("created_at", "DateTime64(3)", "Creation time", "(extract(epoch from m.created_at) * 1000)::bigint"),
    col("modified_at", "DateTime64(3)", "Last edit", "(extract(epoch from m.modified_at) * 1000)::bigint"),
];

const DASHBOARD_COLS: &[Col] = &[
    col("id", "String", "Dashboard id", "d.id"),
    col("title", "String", "Title", "d.title"),
    col("description", "Nullable(String)", "Description", "d.description"),
    col("kind", "String", "custom or integration", "d.kind"),
    col("teams", "Array(String)", "Owning teams", "d.teams"),
    col("author", "Nullable(String)", "Author", "(SELECT u.name FROM users u WHERE u.id = d.author_id AND u.org_id = d.org_id)"),
    col("widgets", "Int64", "Widgets on the dashboard", "COALESCE(jsonb_array_length(CASE WHEN jsonb_typeof(d.definition->'widgets') = 'array' THEN d.definition->'widgets' END), 0)"),
    col("views", "Int64", "Times opened", "d.view_count"),
    col("created_at", "DateTime64(3)", "Creation time", "(extract(epoch from d.created_at) * 1000)::bigint"),
    col("modified_at", "DateTime64(3)", "Last edit", "(extract(epoch from d.modified_at) * 1000)::bigint"),
];

const INCIDENT_COLS: &[Col] = &[
    col("id", "Int64", "Incident number (IR-<id>)", "i.public_id"),
    col("title", "String", "Title", "i.title"),
    col("status", "String", "active, stable or resolved", "i.status"),
    col("severity", "String", "SEV-1 to SEV-5 or UNKNOWN", "i.severity"),
    col("type", "String", "Incident type", "i.incident_type"),
    col("detection_method", "String", "monitor, customer, employee, automation or unknown", "i.detection_method"),
    col("customer_impact", "Bool", "Customers affected", "i.customer_impact"),
    col("services", "Array(String)", "Impacted services", "i.services"),
    col("teams", "Array(String)", "Teams", "i.teams"),
    col("commander", "Nullable(String)", "Incident commander", "(SELECT u.name FROM users u WHERE u.id = i.commander_id AND u.org_id = i.org_id)"),
    col("test", "Bool", "Test incident", "i.test"),
    col("declared_at", "DateTime64(3)", "Declared", "(extract(epoch from i.declared_at) * 1000)::bigint"),
    col("detected_at", "DateTime64(3)", "Detected", "(extract(epoch from i.detected_at) * 1000)::bigint"),
    col("resolved_at", "Nullable(DateTime64(3))", "Resolved", "(extract(epoch from i.resolved_at) * 1000)::bigint"),
];

const SYNTHETIC_COLS: &[Col] = &[
    col("id", "String", "Test id", "t.id::text"),
    col("name", "String", "Name", "t.name"),
    col("type", "String", "http, ssl, dns or tcp", "t.type"),
    col("target", "String", "URL or host tested", "COALESCE(NULLIF(t.request->>'url', ''), t.request->>'host', '')"),
    col("locations", "Array(String)", "Locations it runs from", "t.locations"),
    col("frequency_s", "Int64", "Seconds between runs", "t.frequency_s"),
    col("env", "String", "env", "t.env"),
    col("team", "String", "Owning team", "t.team"),
    col("tags", "Array(String)", "Tags", "t.tags"),
    col("paused", "Bool", "Paused", "t.paused"),
    col("status", "String", "alert while failing, else ok (paused: paused)", "CASE WHEN t.paused THEN 'paused' WHEN t.failing_since IS NOT NULL THEN 'alert' ELSE 'ok' END"),
    col("failing_since", "Nullable(DateTime64(3))", "Start of the current failure", "(extract(epoch from t.failing_since) * 1000)::bigint"),
    col("last_run_at", "Nullable(DateTime64(3))", "Latest run", "(extract(epoch from t.last_run_at) * 1000)::bigint"),
    col("created_at", "DateTime64(3)", "Creation time", "(extract(epoch from t.created_at) * 1000)::bigint"),
];

const KUBE_TABLES: &[(&str, KubeKind, &str)] = &[
    ("k8s_clusters", KubeKind::Clusters, "Kubernetes clusters"),
    ("k8s_nodes", KubeKind::Nodes, "Kubernetes nodes"),
    ("k8s_namespaces", KubeKind::Namespaces, "Kubernetes namespaces"),
    ("k8s_pods", KubeKind::Pods, "Kubernetes pods"),
    ("k8s_deployments", KubeKind::Deployments, "Kubernetes deployments"),
    ("k8s_daemonsets", KubeKind::DaemonSets, "Kubernetes daemon sets"),
    ("k8s_statefulsets", KubeKind::StatefulSets, "Kubernetes stateful sets"),
    ("k8s_jobs", KubeKind::Jobs, "Kubernetes jobs"),
    ("k8s_cronjobs", KubeKind::CronJobs, "Kubernetes cron jobs"),
    ("k8s_services", KubeKind::Services, "Kubernetes services"),
    ("k8s_ingresses", KubeKind::Ingresses, "Kubernetes ingresses"),
];

static TABLES: LazyLock<Vec<VirtualTable>> = LazyLock::new(|| {
    let mut t = vec![
        VirtualTable {
            name: "spans",
            category: "apm",
            doc: "APM spans in the selected time range",
            cols: SPAN_COLS,
            source: Source::ClickHouse {
                rest: "FROM otel.otel_traces WHERE OrgId = {oc_org:String} AND Timestamp >= fromUnixTimestamp64Milli(?) AND Timestamp <= fromUnixTimestamp64Milli(?)",
                binds: SPAN_RANGE,
            },
        },
        VirtualTable {
            name: "services",
            category: "apm",
            doc: "APM services and environments with request, error and latency stats over the selected range",
            cols: SERVICE_COLS,
            source: Source::ClickHouse {
                rest: "FROM otel.otel_traces WHERE OrgId = {oc_org:String} AND SpanKind IN ('Server', 'Consumer') AND Timestamp >= fromUnixTimestamp64Milli(?) AND Timestamp <= fromUnixTimestamp64Milli(?) GROUP BY service, env",
                binds: SPAN_RANGE,
            },
        },
        VirtualTable {
            name: "logs",
            category: "logs",
            doc: "Indexed logs in the selected time range (your log restrictions apply)",
            cols: LOG_COLS,
            source: Source::Logs,
        },
        VirtualTable {
            name: "metrics",
            category: "metrics",
            doc: "Metric series with their latest value in the selected range",
            cols: METRIC_COLS,
            source: Source::ClickHouse {
                rest: "FROM (SELECT SeriesId, MetricName, toString(MetricType) AS MetricType, MetricUnit, ServiceName, ResourceAttributes, Attributes FROM otel.metrics_series FINAL \
                       WHERE OrgId = {oc_org:String} AND LastSeen >= toStartOfHour(fromUnixTimestamp64Milli(?))) AS s \
                       INNER JOIN (SELECT SeriesId, argMax(Value, TimeUnix) AS value, max(TimeUnix) AS ts FROM otel.metrics_samples_read \
                       WHERE OrgId = {oc_org:String} AND TimeUnix >= fromUnixTimestamp64Milli(?) AND TimeUnix <= fromUnixTimestamp64Milli(?) GROUP BY SeriesId) AS l ON l.SeriesId = s.SeriesId",
                binds: METRIC_RANGE,
            },
        },
        VirtualTable {
            name: "ci_pipelines",
            category: "ci",
            doc: "CI pipeline runs started in the selected range",
            cols: CI_COLS,
            source: Source::ClickHouse {
                rest: "FROM otel.otel_ci_traces WHERE OrgId = {oc_org:String} AND ParentSpanId = '' AND Timestamp >= fromUnixTimestamp64Milli(?) AND Timestamp <= fromUnixTimestamp64Milli(?)",
                binds: SPAN_RANGE,
            },
        },
        VirtualTable { name: "hosts", category: "infrastructure", doc: "Hosts that reported in the selected range", cols: HOST_COLS, source: Source::Inventory(Inventory::Hosts) },
        VirtualTable { name: "containers", category: "containers", doc: "Containers of Kubernetes pods and ECS tasks", cols: CONTAINER_COLS, source: Source::Inventory(Inventory::Containers) },
        VirtualTable { name: "resources", category: "infrastructure", doc: "The resource catalog: hosts, Kubernetes and ECS resources", cols: RESOURCE_COLS, source: Source::Inventory(Inventory::Resources) },
        VirtualTable {
            name: "monitors",
            category: "monitors",
            doc: "Monitors and their current status",
            cols: MONITOR_COLS,
            source: Source::Postgres { rest: "FROM monitors m WHERE m.org_id = $1 ORDER BY m.id LIMIT 10000", order: "m.id" },
        },
        VirtualTable {
            name: "dashboards",
            category: "dashboards",
            doc: "Dashboards",
            cols: DASHBOARD_COLS,
            source: Source::Postgres { rest: "FROM dashboards d WHERE d.org_id = $1 AND d.deleted_at IS NULL ORDER BY d.modified_at DESC LIMIT 10000", order: "d.modified_at DESC" },
        },
        VirtualTable {
            name: "incidents",
            category: "incidents",
            doc: "Incidents",
            cols: INCIDENT_COLS,
            source: Source::Postgres { rest: "FROM incidents i WHERE i.org_id = $1 ORDER BY i.declared_at DESC LIMIT 10000", order: "i.declared_at DESC" },
        },
        VirtualTable {
            name: "synthetics_tests",
            category: "synthetics",
            doc: "Synthetic tests and whether they are failing",
            cols: SYNTHETIC_COLS,
            source: Source::Postgres { rest: "FROM synthetic_tests t WHERE t.org_id = $1 ORDER BY t.name LIMIT 10000", order: "t.name" },
        },
    ];
    for (name, kind, doc) in KUBE_TABLES {
        t.push(VirtualTable { name, category: "kubernetes", doc, cols: KUBE_COLS, source: Source::Inventory(Inventory::Kube(*kind)) });
    }
    t
});

fn table(name: &str) -> Option<&'static VirtualTable> {
    TABLES.iter().find(|t| t.name == name)
}

fn table_names() -> Vec<&'static str> {
    TABLES.iter().map(|t| t.name).collect()
}

/// The ClickHouse expression of a column (CI status comes from the CI module).
fn ch_expr(c: &Col) -> &str {
    if c.expr == "__CI_STATUS__" {
        super::ci::STATUS
    } else {
        c.expr
    }
}

/// The subquery of a ClickHouse-backed table: `SELECT <expr> AS <col>, … <rest>`.
fn clickhouse_subquery(cols: &[Col], rest: &str) -> String {
    let select: Vec<String> = cols.iter().map(|c| format!("{} AS `{}`", ch_expr(c), c.name)).collect();
    format!("SELECT {} {rest}", select.join(", "))
}

/// A ClickHouse-backed table's subquery and its bound values.
fn clickhouse_source(cols: &[Col], rest: &str, binds: &[Bind], range: TimeRange) -> (String, Vec<Param>) {
    let params = binds
        .iter()
        .map(|b| match b {
            Bind::From => Param::Int(range.from),
            Bind::To => Param::Int(range.to),
        })
        .collect();
    (clickhouse_subquery(cols, rest), params)
}

/// The logs table: the same scope as the Logs pages (org, range and the reader's log restrictions).
fn logs_source(cols: &[Col], org: &str, range: TimeRange, access: &LogAccess) -> (String, Vec<Param>) {
    let scope = super::logs::scope(org, access, range, &[], None);
    (clickhouse_subquery(cols, &format!("FROM {} WHERE {}", scope.table, scope.clause)), scope.params)
}

/// The type a column is read from JSON as (times travel as epoch milliseconds).
fn json_type(ty: &str) -> String {
    ty.replace("DateTime64(3)", "Int64")
}

/// The subquery of a materialized table: its rows arrive as one bound JSON array of arrays.
fn json_subquery(cols: &[Col]) -> String {
    let tuple: Vec<String> = cols.iter().map(|c| json_type(c.ty)).collect();
    let select: Vec<String> = cols
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let v = format!("tupleElement(t, {})", i + 1);
            let v = if c.ty.contains("DateTime64(3)") { format!("fromUnixTimestamp64Milli({v}, 'UTC')") } else { v };
            format!("{v} AS `{}`", c.name)
        })
        .collect();
    format!("SELECT {} FROM (SELECT arrayJoin(JSONExtract(?, 'Array(Tuple({}))')) AS t)", select.join(", "), tuple.join(", "))
}

/// The Postgres query returning a table's rows as a JSON array of arrays.
fn postgres_query(cols: &[Col], rest: &str, order: &str) -> String {
    let values: Vec<&str> = cols.iter().map(|c| c.expr).collect();
    format!("SELECT COALESCE(json_agg(json_build_array({}) ORDER BY {order}), '[]')::text FROM (SELECT * {rest}) AS {}", values.join(", "), alias_of(rest))
}

/// The alias a Postgres source gives its table (`FROM monitors m …` → `m`).
fn alias_of(rest: &str) -> &str {
    rest.split_whitespace().nth(2).unwrap_or("x")
}

/// Joins the checked pieces with the org-scoped subqueries into one query and its parameters.
fn assemble(checked: &CheckedSql, subqueries: &HashMap<String, (String, Vec<Param>)>) -> Result<(String, Vec<Param>), ApiError> {
    let mut sql = String::new();
    let mut params = Vec::new();
    for piece in &checked.pieces {
        match piece {
            SqlPiece::Text(t) => sql.push_str(t),
            SqlPiece::Literal(l) => {
                sql.push('?');
                params.push(Param::Str(l.clone()));
            }
            SqlPiece::Table(name) => {
                let (sub, sub_params) = subqueries.get(name).ok_or_else(|| ApiError::Internal(anyhow::anyhow!("no subquery for {name}")))?;
                sql.push('(');
                sql.push_str(sub);
                sql.push(')');
                params.extend(sub_params.iter().cloned());
            }
        }
    }
    Ok((sql, params))
}

// ---------------------------------------------------------------- inventory rows

fn ms(v: Option<i64>) -> Value {
    v.map_or(Value::Null, Value::from)
}

fn host_row(h: &HostRow) -> Vec<Value> {
    let s = &h.summary;
    let res = |k: &str| h.res.get(k).cloned();
    vec![
        json!(s.name),
        json!(enum_str(&s.cloud)),
        json!(enum_str(&s.os)),
        json!(s.distro),
        json!(s.env),
        json!(s.service),
        json!(s.team),
        json!(res("cloud.region")),
        json!(res("cloud.availability_zone")),
        json!(res("cloud.account.id")),
        json!(res("host.type")),
        json!(res("k8s.cluster.name")),
        json!(s.cpu_pct),
        json!(s.mem_pct),
        json!(s.error_spans),
        json!(s.collectors.iter().map(enum_str).collect::<Vec<_>>()),
        json!(s.apps),
        json!(s.gpu),
        json!(h.last_seen),
    ]
}

fn container_row(r: &KubeResource) -> Vec<Value> {
    vec![
        json!(r.uid),
        json!(r.name),
        json!(r.owner.as_ref().map(|o| &o.name)),
        json!(r.owner.as_ref().map(|o| &o.kind)),
        json!(r.cluster),
        json!(r.namespace),
        json!(r.node),
        json!(r.images.first()),
        json!(r.status),
        json!(r.restarts.unwrap_or(0)),
        json!(r.cpu.as_ref().and_then(|c| c.usage)),
        json!(r.memory.as_ref().and_then(|m| m.usage)),
        json!(r.memory.as_ref().and_then(|m| m.limits)),
        ms(r.created_at),
        json!(r.tags),
    ]
}

fn kube_row(r: &KubeResource) -> Vec<Value> {
    let cpu = r.cpu.clone().unwrap_or_default();
    let mem = r.memory.clone().unwrap_or_default();
    vec![
        json!(r.uid),
        json!(r.cluster),
        json!(r.namespace),
        json!(r.name),
        json!(r.status),
        json!(enum_str(&r.health)),
        json!(r.owner.as_ref().map(|o| &o.kind)),
        json!(r.owner.as_ref().map(|o| &o.name)),
        json!(r.node),
        json!(r.ready),
        json!(r.restarts),
        json!(r.desired),
        json!(r.available),
        json!(cpu.usage),
        json!(cpu.requests),
        json!(cpu.limits),
        json!(mem.usage),
        json!(mem.requests),
        json!(mem.limits),
        json!(r.version),
        json!(r.images),
        json!(r.labels),
        json!(r.tags),
        ms(r.created_at),
        json!(r.last_seen),
    ]
}

fn resource_row(r: &CatalogResource) -> Vec<Value> {
    vec![
        json!(r.id),
        json!(enum_str(&r.resource_type)),
        json!(enum_str(&r.resource_category)),
        json!(r.name),
        json!(r.cloud),
        json!(r.account),
        json!(r.region),
        json!(r.cluster),
        json!(r.namespace),
        json!(r.env),
        json!(r.service),
        json!(r.team),
        json!(r.status),
        ms(r.created_at),
        json!(r.last_seen),
        json!(r.tags),
        json!(r.attributes),
    ]
}

/// The infrastructure inventories a query needs, each loaded once.
#[derive(Default)]
struct Loaded {
    hosts: Option<Vec<HostRow>>,
    kube: Option<std::sync::Arc<Vec<KubeResource>>>,
    catalog: Option<Vec<CatalogResource>>,
}

impl Loaded {
    async fn rows(&mut self, st: &AppState, org: &str, range: TimeRange, inv: Inventory) -> Result<Vec<Vec<Value>>, ApiError> {
        Ok(match inv {
            Inventory::Hosts => {
                if self.hosts.is_none() {
                    self.hosts = Some(super::hosts::load_hosts(st, org, range).await?);
                }
                self.hosts.iter().flatten().map(host_row).collect()
            }
            Inventory::Containers | Inventory::Kube(_) => {
                if self.kube.is_none() {
                    self.kube = Some(super::kubernetes::resources(st, org).await?);
                }
                let kind = match inv {
                    Inventory::Kube(k) => k,
                    _ => KubeKind::Containers,
                };
                let rows = self.kube.iter().flat_map(|k| k.iter()).filter(|r| r.kind == Some(kind));
                if inv == Inventory::Containers {
                    rows.map(container_row).collect()
                } else {
                    rows.map(kube_row).collect()
                }
            }
            Inventory::Resources => {
                if self.catalog.is_none() {
                    self.catalog = Some(super::catalog::load_all(st, org).await?);
                }
                self.catalog.iter().flatten().map(resource_row).collect()
            }
        })
    }
}

// ================================================================ API types

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SqlTableColumn {
    pub name: String,
    #[serde(rename = "type")]
    pub ty: String,
    pub description: String,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SqlTableInfo {
    pub name: String,
    /// infrastructure, containers, kubernetes, apm, logs, metrics, ci, monitors, dashboards, incidents, synthetics.
    pub category: String,
    pub description: String,
    pub columns: Vec<SqlTableColumn>,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SqlSchema {
    pub tables: Vec<SqlTableInfo>,
    /// Functions queries may call.
    pub functions: Vec<String>,
    pub max_rows: u64,
}

#[derive(Serialize, Deserialize, ToSchema, Clone)]
#[serde(rename_all = "camelCase")]
pub struct SqlTemplate {
    pub id: String,
    pub title: String,
    pub description: String,
    /// Category of the data it reads (for the icon).
    pub source: String,
    pub sql: String,
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SqlRunRequest {
    pub sql: String,
    /// Time range of the telemetry tables (spans, logs, metrics, services, ci_pipelines) and of
    /// hosts; the last hour when omitted. At most 7 days.
    pub range: Option<TimeRange>,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SqlResultColumn {
    pub name: String,
    #[serde(rename = "type")]
    pub ty: String,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SqlResult {
    pub columns: Vec<SqlResultColumn>,
    /// Rows as arrays, in column order.
    #[schema(value_type = Vec<Vec<serde_json::Value>>)]
    pub rows: Vec<Vec<Value>>,
    /// More rows matched than were returned.
    pub truncated: bool,
    pub elapsed_ms: u64,
    /// Virtual tables the query read.
    pub tables: Vec<String>,
    /// Limits that applied (inventory tables cut, …).
    pub notices: Vec<String>,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SavedSqlQuery {
    pub id: String,
    pub name: String,
    pub sql: String,
    pub team: String,
    pub author: String,
    pub updated_by: String,
    /// Epoch ms.
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SavedSqlQueryInput {
    pub name: String,
    pub sql: String,
    #[serde(default)]
    pub team: String,
}

// ================================================================ handlers

static TEMPLATES: LazyLock<Vec<SqlTemplate>> = LazyLock::new(|| serde_json::from_str(include_str!("../../../../seeds/sql_templates.json")).expect("seeds/sql_templates.json is valid"));

#[utoipa::path(get, path = "/api/v1/sql/schema", operation_id = "sql_schema", responses((status = 200, body = SqlSchema)), tag = "sql")]
pub async fn schema(_ctx: AuthContext) -> ApiResult<SqlSchema> {
    let tables = TABLES
        .iter()
        .map(|t| SqlTableInfo {
            name: t.name.into(),
            category: t.category.into(),
            description: t.doc.into(),
            columns: t.cols.iter().map(|c| SqlTableColumn { name: c.name.into(), ty: c.ty.into(), description: c.doc.into() }).collect(),
        })
        .collect();
    Ok(Json(SqlSchema { tables, functions: ALLOWED_FUNCTIONS.iter().map(|f| f.to_string()).collect(), max_rows: MAX_ROWS }))
}

#[utoipa::path(get, path = "/api/v1/sql/templates", operation_id = "sql_templates", responses((status = 200, body = Vec<SqlTemplate>)), tag = "sql")]
pub async fn templates(_ctx: AuthContext) -> ApiResult<Vec<SqlTemplate>> {
    Ok(Json(TEMPLATES.clone()))
}

/// Runs one read-only SELECT over the virtual tables (see the module docs).
#[utoipa::path(post, path = "/api/v1/sql/run", operation_id = "sql_run", request_body = SqlRunRequest, responses((status = 200, body = SqlResult), (status = 400, description = "Query refused or failed")), tag = "sql")]
pub async fn run(State(st): State<AppState>, ctx: AuthContext, access: LogAccess, Json(req): Json<SqlRunRequest>) -> ApiResult<SqlResult> {
    let started = Instant::now();
    let now = super::metrics::now_ms();
    let range = req.range.unwrap_or(TimeRange { from: now - DEFAULT_RANGE_MS, to: now });
    range.validate_within(MAX_RANGE_DAYS)?;
    let names = table_names();
    let checked = check_sql(&req.sql, &names, MAX_ROWS).map_err(|e| ApiError::BadRequest(e.0))?;
    let org = ctx.org_id.as_str();

    let mut subqueries = HashMap::new();
    let mut notices = Vec::new();
    let mut loaded = Loaded::default();
    for name in &checked.tables {
        let t = table(name).ok_or_else(|| ApiError::BadRequest(format!("Unknown table {name}")))?;
        let (sql, params) = match &t.source {
            Source::ClickHouse { rest, binds } => clickhouse_source(t.cols, rest, binds, range),
            Source::Logs => logs_source(t.cols, org, range, &access),
            Source::Postgres { rest, order } => {
                let (json,): (String,) = sqlx::query_as(&postgres_query(t.cols, rest, order)).bind(org).fetch_one(&st.postgres).await?;
                (json_subquery(t.cols), vec![Param::Str(json)])
            }
            Source::Inventory(inv) => {
                let mut rows = loaded.rows(&st, org, range, *inv).await?;
                if rows.len() > MAX_TABLE_ROWS {
                    notices.push(format!("{name}: only the first {MAX_TABLE_ROWS} of {} rows are queried", rows.len()));
                    rows.truncate(MAX_TABLE_ROWS);
                }
                (json_subquery(t.cols), vec![Param::Str(serde_json::to_string(&rows).map_err(anyhow::Error::from)?)])
            }
        };
        subqueries.insert(name.clone(), (sql, params));
    }
    let (sql, params) = assemble(&checked, &subqueries)?;
    let ch = st
        .ch(org)
        .with_option("readonly", "2")
        .with_option("max_execution_time", st.config.org_query.timeout_secs.min(MAX_EXECUTION_SECS).to_string())
        .with_option("max_memory_usage", st.config.org_query.max_memory_bytes.min(MAX_MEMORY_BYTES).to_string())
        .with_option("max_query_size", (64u64 << 20).to_string())
        .with_option("output_format_json_quote_64bit_integers", "0")
        .with_option("output_format_json_quote_64bit_floats", "0");
    let mut cursor = bind(ch.query(&sql), &params).fetch_bytes("JSONCompactEachRowWithNamesAndTypes")?;
    let mut body = Vec::new();
    let mut too_big = false;
    while let Some(chunk) = cursor.next().await.map_err(|e| query_error(&e))? {
        if body.len() + chunk.len() > MAX_RESULT_BYTES {
            too_big = true;
            break;
        }
        body.extend_from_slice(&chunk);
    }
    if too_big {
        return Err(ApiError::BadRequest(format!("The result is larger than {} MB; select fewer columns or rows", MAX_RESULT_BYTES >> 20)));
    }
    let (columns, mut rows) = parse_result(&body)?;
    let truncated = rows.len() as u64 > MAX_ROWS;
    rows.truncate(MAX_ROWS as usize);
    Ok(Json(SqlResult { columns, rows, truncated, elapsed_ms: started.elapsed().as_millis() as u64, tables: checked.tables, notices }))
}

/// Shows ClickHouse's reason without the rewritten query, internal names or server details.
fn query_error(e: &clickhouse::error::Error) -> ApiError {
    tracing::warn!(error = %e, "sql editor query failed");
    match e {
        clickhouse::error::Error::BadResponse(msg) => ApiError::BadRequest(clean_error(msg)),
        clickhouse::error::Error::TimedOut => ApiError::BadRequest("The query took too long".into()),
        _ => ApiError::BadRequest("The query failed".into()),
    }
}

fn clean_error(msg: &str) -> String {
    let msg = msg.split("DB::Exception: ").nth(1).unwrap_or(msg);
    let cut = [" in scope ", ": In scope", ": While processing", ". (", " (version", ": while executing", "\nStack trace"].iter().filter_map(|m| msg.find(m)).min().unwrap_or(msg.len());
    let reason = msg[..cut].trim();
    let internal = ["otel.", "oc.", "OrgId", "org_id", "__oc_", "JSONExtract", "tupleElement", "arrayJoin(JSONExtract"];
    if reason.is_empty() || internal.iter().any(|i| reason.contains(i)) {
        "The query failed; check column names and types".into()
    } else if reason.contains("TIMEOUT_EXCEEDED") || reason.contains("Timeout exceeded") {
        "The query took too long; narrow the time range or add filters".into()
    } else if reason.contains("MEMORY_LIMIT_EXCEEDED") || reason.contains("Memory limit") {
        "The query needs too much memory; aggregate more or add filters".into()
    } else {
        reason.chars().take(500).collect()
    }
}

/// Reads JSONCompactEachRowWithNamesAndTypes: names, types, then one array per row.
fn parse_result(body: &[u8]) -> Result<(Vec<SqlResultColumn>, Vec<Vec<Value>>), ApiError> {
    let text = std::str::from_utf8(body).map_err(|_| ApiError::BadRequest("The result isn't valid UTF-8".into()))?;
    let mut lines = text.lines().filter(|l| !l.trim().is_empty());
    let parse = |l: &str| serde_json::from_str::<Vec<Value>>(l).map_err(|e| ApiError::Internal(anyhow::anyhow!("unreadable result row: {e}")));
    let names = lines.next().map(parse).transpose()?.unwrap_or_default();
    let types = lines.next().map(parse).transpose()?.unwrap_or_default();
    let columns = names
        .iter()
        .zip(types.iter().chain(std::iter::repeat(&Value::Null)))
        .map(|(n, t)| SqlResultColumn { name: n.as_str().unwrap_or_default().to_string(), ty: t.as_str().unwrap_or_default().to_string() })
        .collect();
    let rows = lines.map(parse).collect::<Result<Vec<_>, _>>()?;
    Ok((columns, rows))
}

// ---------------------------------------------------------------- saved queries

#[derive(sqlx::FromRow)]
struct QueryRow {
    id: String,
    name: String,
    sql: String,
    team: String,
    author: String,
    updated_by: String,
    created_at: i64,
    updated_at: i64,
}

impl From<QueryRow> for SavedSqlQuery {
    fn from(r: QueryRow) -> Self {
        SavedSqlQuery { id: r.id, name: r.name, sql: r.sql, team: r.team, author: r.author, updated_by: r.updated_by, created_at: r.created_at, updated_at: r.updated_at }
    }
}

const QUERY_SELECT: &str = "SELECT id::text AS id, name, sql, team, author, updated_by, (extract(epoch from created_at) * 1000)::bigint AS created_at, \
                            (extract(epoch from updated_at) * 1000)::bigint AS updated_at FROM sql_queries";

fn validate(input: &mut SavedSqlQueryInput) -> Result<(), String> {
    input.name = input.name.trim().to_string();
    input.team = input.team.trim().to_string();
    if input.name.is_empty() {
        return Err("Name the query".into());
    }
    if input.name.chars().count() > MAX_NAME_CHARS {
        return Err(format!("Names are at most {MAX_NAME_CHARS} characters"));
    }
    if input.team.chars().count() > MAX_NAME_CHARS {
        return Err(format!("Team handles are at most {MAX_NAME_CHARS} characters"));
    }
    if input.sql.trim().is_empty() {
        return Err("The query is empty".into());
    }
    if input.sql.chars().count() > MAX_SQL_CHARS {
        return Err(format!("Queries are at most {MAX_SQL_CHARS} characters"));
    }
    Ok(())
}

async fn get_query(st: &AppState, org: &str, id: &str) -> Result<SavedSqlQuery, ApiError> {
    let row: Option<QueryRow> = sqlx::query_as(&format!("{QUERY_SELECT} WHERE org_id = $1 AND id::text = $2")).bind(org).bind(id).fetch_optional(&st.postgres).await?;
    row.map(SavedSqlQuery::from).ok_or_else(|| ApiError::NotFound("Query not found".into()))
}

#[utoipa::path(get, path = "/api/v1/sql/queries", operation_id = "list_sql_queries", responses((status = 200, body = Vec<SavedSqlQuery>)), tag = "sql")]
pub async fn list_queries(State(st): State<AppState>, ctx: AuthContext) -> ApiResult<Vec<SavedSqlQuery>> {
    let rows: Vec<QueryRow> = sqlx::query_as(&format!("{QUERY_SELECT} WHERE org_id = $1 ORDER BY updated_at DESC LIMIT {MAX_SAVED_QUERIES}")).bind(&ctx.org_id).fetch_all(&st.postgres).await?;
    Ok(Json(rows.into_iter().map(SavedSqlQuery::from).collect()))
}

#[utoipa::path(post, path = "/api/v1/sql/queries", operation_id = "create_sql_query", request_body = SavedSqlQueryInput, responses((status = 200, body = SavedSqlQuery)), tag = "sql")]
pub async fn create_query(State(st): State<AppState>, Writer(ctx): Writer, Json(mut input): Json<SavedSqlQueryInput>) -> ApiResult<SavedSqlQuery> {
    validate(&mut input).map_err(ApiError::BadRequest)?;
    let (count,): (i64,) = sqlx::query_as("SELECT count(*) FROM sql_queries WHERE org_id = $1").bind(&ctx.org_id).fetch_one(&st.postgres).await?;
    if count >= MAX_SAVED_QUERIES {
        return Err(ApiError::BadRequest(format!("An organization can save at most {MAX_SAVED_QUERIES} queries")));
    }
    let (id,): (String,) = sqlx::query_as("INSERT INTO sql_queries (org_id, name, sql, team, author, updated_by) VALUES ($1, $2, $3, $4, $5, $5) RETURNING id::text")
        .bind(&ctx.org_id)
        .bind(&input.name)
        .bind(&input.sql)
        .bind(&input.team)
        .bind(&ctx.name)
        .fetch_one(&st.postgres)
        .await?;
    ctx.audit(&st, "sql_query.created", &input.name, "").await;
    Ok(Json(get_query(&st, &ctx.org_id, &id).await?))
}

#[utoipa::path(put, path = "/api/v1/sql/queries/{id}", operation_id = "update_sql_query", params(("id" = String, Path)), request_body = SavedSqlQueryInput, responses((status = 200, body = SavedSqlQuery)), tag = "sql")]
pub async fn update_query(State(st): State<AppState>, Writer(ctx): Writer, Path(id): Path<String>, Json(mut input): Json<SavedSqlQueryInput>) -> ApiResult<SavedSqlQuery> {
    validate(&mut input).map_err(ApiError::BadRequest)?;
    let done = sqlx::query("UPDATE sql_queries SET name = $3, sql = $4, team = $5, updated_by = $6, updated_at = now() WHERE org_id = $1 AND id::text = $2")
        .bind(&ctx.org_id)
        .bind(&id)
        .bind(&input.name)
        .bind(&input.sql)
        .bind(&input.team)
        .bind(&ctx.name)
        .execute(&st.postgres)
        .await?;
    if done.rows_affected() == 0 {
        return Err(ApiError::NotFound("Query not found".into()));
    }
    ctx.audit(&st, "sql_query.updated", &input.name, "").await;
    Ok(Json(get_query(&st, &ctx.org_id, &id).await?))
}

#[utoipa::path(delete, path = "/api/v1/sql/queries/{id}", operation_id = "delete_sql_query", params(("id" = String, Path)), responses((status = 204)), tag = "sql")]
pub async fn delete_query(State(st): State<AppState>, Writer(ctx): Writer, Path(id): Path<String>) -> Result<StatusCode, ApiError> {
    let row: Option<(String,)> = sqlx::query_as("DELETE FROM sql_queries WHERE org_id = $1 AND id::text = $2 RETURNING name").bind(&ctx.org_id).bind(&id).fetch_optional(&st.postgres).await?;
    let Some((name,)) = row else { return Err(ApiError::NotFound("Query not found".into())) };
    ctx.audit(&st, "sql_query.deleted", &name, "").await;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_template_passes_the_checker() {
        let names = table_names();
        assert!(TEMPLATES.len() >= 8);
        for t in TEMPLATES.iter() {
            let checked = check_sql(&t.sql, &names, MAX_ROWS).unwrap_or_else(|e| panic!("template {}: {e}", t.id));
            for name in &checked.tables {
                let table = table(name).expect("known table");
                for piece in &checked.pieces {
                    if let SqlPiece::Text(text) = piece {
                        assert!(!text.contains("otel.") && !text.contains("oc."), "{}", t.id);
                    }
                }
                assert!(!table.cols.is_empty());
            }
        }
        let ids: std::collections::HashSet<&str> = TEMPLATES.iter().map(|t| t.id.as_str()).collect();
        assert_eq!(ids.len(), TEMPLATES.len(), "template ids are unique");
    }

    #[test]
    fn every_virtual_table_is_scoped_to_the_org() {
        for t in TABLES.iter() {
            match &t.source {
                Source::ClickHouse { rest, binds } => {
                    let sql = clickhouse_subquery(t.cols, rest);
                    // Every stored table it reads filters on the org: one filter per FROM/JOIN of a stored table.
                    let stored = format!("{}.", "otel");
                    let reads = sql.matches(&format!("FROM {stored}")).count() + sql.matches(&format!("JOIN {stored}")).count();
                    let filters = sql.matches("OrgId = {oc_org:String}").count();
                    assert!(reads >= 1 && filters >= reads, "{}: {sql}", t.name);
                    assert_eq!(sql.matches('?').count(), binds.len(), "{}: one value per ?", t.name);
                }
                Source::Logs => {
                    let (sql, params) = logs_source(t.cols, "org-1", TimeRange { from: 1, to: 2 }, &LogAccess::resolve(&[], false, &[], &[]));
                    assert!(sql.contains("org_id = ?") && matches!(&params[0], Param::Str(o) if o == "org-1"), "{sql}");
                    assert_eq!(sql.matches('?').count(), params.len());
                }
                Source::Postgres { rest, order } => {
                    let sql = postgres_query(t.cols, rest, order);
                    let alias = alias_of(rest);
                    assert!(rest.contains(&format!("{alias}.org_id = $1")), "{}: {sql}", t.name);
                    assert!(rest.contains("LIMIT 10000"), "{}", t.name);
                    // Joined Postgres tables are reached through the org's own row.
                    for sub in sql.split("(SELECT ").skip(2) {
                        assert!(sub.contains(&format!("{alias}.")), "{}: {sub}", t.name);
                    }
                }
                Source::Inventory(_) => assert!(t.cols.iter().all(|c| c.expr.is_empty())),
            }
        }
    }

    #[test]
    fn table_names_are_unique_and_plain() {
        let names = table_names();
        let set: std::collections::HashSet<_> = names.iter().collect();
        assert_eq!(set.len(), names.len());
        for t in TABLES.iter() {
            assert!(t.name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_'), "{}", t.name);
            for c in t.cols {
                assert!(c.name.chars().all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '_'), "{}.{}", t.name, c.name);
            }
        }
    }

    #[test]
    fn inventory_rows_match_their_columns() {
        let r = KubeResource { uid: "u".into(), name: "checkout".into(), ..Default::default() };
        assert_eq!(kube_row(&r).len(), KUBE_COLS.len());
        assert_eq!(container_row(&r).len(), CONTAINER_COLS.len());
        assert_eq!(HOST_COLS.len(), 19);
        assert_eq!(RESOURCE_COLS.len(), 17);
    }

    #[test]
    fn log_restrictions_apply_to_the_logs_table() {
        let logs = table("logs").unwrap();
        assert!(matches!(logs.source, Source::Logs));
        let range = TimeRange { from: 1, to: 2 };
        let rule = || crate::log_access::Restriction { terms: oc_query::parse_query("service:billing"), roles: ["r-billing".to_string()].into(), teams: Default::default() };
        let restricted = LogAccess::resolve(&[rule()], false, &["r-billing".to_string()], &[]);
        let (sql, params) = logs_source(logs.cols, "org-1", range, &restricted);
        assert_eq!(sql.matches('?').count(), params.len());
        assert!(params.iter().any(|p| matches!(p, Param::Str(s) if s == "billing")), "the restriction's value is bound");
        assert!(matches!(&params[0], Param::Str(s) if s == "org-1"));
        let (open, open_params) = logs_source(logs.cols, "org-1", range, &LogAccess::resolve(&[], false, &[], &[]));
        assert!(open.len() < sql.len() && open_params.len() < params.len());
        // A reader no restriction lets through sees no logs at all.
        let other = LogAccess::resolve(&[rule()], false, &[], &[]);
        let (none, _) = logs_source(logs.cols, "org-1", range, &other);
        assert!(other.restricted() && none.len() > open.len());
    }

    #[test]
    fn materialized_tables_read_their_rows_from_one_bound_value() {
        let sql = json_subquery(&[col("name", "String", "", ""), col("seen", "Nullable(DateTime64(3))", "", ""), col("tags", "Array(String)", "", "")]);
        assert_eq!(
            sql,
            "SELECT tupleElement(t, 1) AS `name`, fromUnixTimestamp64Milli(tupleElement(t, 2), 'UTC') AS `seen`, tupleElement(t, 3) AS `tags` \
             FROM (SELECT arrayJoin(JSONExtract(?, 'Array(Tuple(String, Nullable(Int64), Array(String)))')) AS t)"
        );
    }

    #[test]
    fn pieces_and_subqueries_assemble_with_parameters_in_order() {
        let checked = check_sql("SELECT name FROM hosts WHERE os = 'linux' AND name IN (SELECT host FROM logs WHERE message LIKE '%x%')", &table_names(), MAX_ROWS).unwrap();
        let subqueries = HashMap::from([
            ("hosts".to_string(), ("SELECT ? AS name".to_string(), vec![Param::Str("[]".into())])),
            ("logs".to_string(), ("SELECT host FROM oc.logs WHERE org_id = ? AND timestamp >= ?".to_string(), vec![Param::Str("org-1".into()), Param::Int(5)])),
        ]);
        let (sql, params) = assemble(&checked, &subqueries).unwrap();
        assert_eq!(
            sql,
            "SELECT `name` FROM (SELECT ? AS name) AS `hosts` WHERE `os` = ? AND `name` IN (SELECT `host` FROM (SELECT host FROM oc.logs WHERE org_id = ? AND timestamp >= ?) AS `logs` WHERE `message` LIKE ?) LIMIT 10001"
        );
        let shown: Vec<String> = params
            .iter()
            .map(|p| match p {
                Param::Str(s) => s.clone(),
                Param::Int(i) => i.to_string(),
            })
            .collect();
        assert_eq!(shown, vec!["[]", "linux", "org-1", "5", "%x%"]);
        assert_eq!(sql.matches('?').count(), params.len());
    }

    #[test]
    fn clickhouse_errors_are_shown_without_internals() {
        assert_eq!(
            clean_error("Code: 47. DB::Exception: Unknown expression identifier `cpu` in scope SELECT cpu FROM hosts: In scope SELECT ... (UNKNOWN_IDENTIFIER) (version 26.8.36.3 (official build))"),
            "Unknown expression identifier `cpu`"
        );
        assert_eq!(clean_error("Code: 60. DB::Exception: Table otel.otel_traces does not exist. (UNKNOWN_TABLE)"), "The query failed; check column names and types");
        assert_eq!(clean_error("Code: 159. DB::Exception: Timeout exceeded: elapsed 30 seconds. (TIMEOUT_EXCEEDED)"), "The query took too long; narrow the time range or add filters");
    }

    #[test]
    fn results_are_read_with_names_and_types() {
        let body = b"[\"os\", \"hosts\"]\n[\"String\", \"UInt64\"]\n[\"linux\", 3]\n[\"windows\", 1]\n";
        let (cols, rows) = parse_result(body).unwrap();
        assert_eq!(cols.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), vec!["os", "hosts"]);
        assert_eq!(cols[1].ty, "UInt64");
        assert_eq!(rows, vec![vec![json!("linux"), json!(3)], vec![json!("windows"), json!(1)]]);
        assert!(parse_result(b"").unwrap().1.is_empty());
    }

    #[test]
    fn saved_queries_are_validated() {
        let mut ok = SavedSqlQueryInput { name: "  Hosts by OS ".into(), sql: "SELECT 1".into(), team: " sre ".into() };
        validate(&mut ok).unwrap();
        assert_eq!((ok.name.as_str(), ok.team.as_str()), ("Hosts by OS", "sre"));
        assert!(validate(&mut SavedSqlQueryInput { name: " ".into(), sql: "SELECT 1".into(), team: String::new() }).is_err());
        assert!(validate(&mut SavedSqlQueryInput { name: "x".into(), sql: " ".into(), team: String::new() }).is_err());
        assert!(validate(&mut SavedSqlQueryInput { name: "x".repeat(201), sql: "SELECT 1".into(), team: String::new() }).is_err());
    }
}
