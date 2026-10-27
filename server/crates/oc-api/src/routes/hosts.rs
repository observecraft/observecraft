//! Infrastructure host list and host panel, built from what hosts actually report:
//! - inventory + identity: resource attributes on `system.cpu.utilization` (any `hostmetrics` source),
//! - CPU / memory / load / uptime: host metrics,
//! - service + error spans: service entry spans carrying `host.name`,
//! - processes: `process.*` metrics; logs: `oc.logs.host`,
//! - pods and containers: the Kubernetes inventory of the node with the host's name.
//!
//! Host lists are small (a fleet, not events), so filtering, grouping and sorting happen in memory
//! after two aggregate queries.

use std::collections::{BTreeMap, HashMap};

use axum::extract::{Path, State};
use axum::Json;
use oc_query::{matches, parse_query, short_tag, Fields, MetricQuery, SpaceAgg, TagFilter, Term};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use super::logs::TimeRange;
use super::metrics::{evaluate_query, name_list, now_ms, points_subquery, with_window, MetricSeries, OPEN_END_MS};
use crate::error::{ApiError, ApiResult};
use crate::state::{AppState, OrgId};

const GIB: f64 = 1024.0 * 1024.0 * 1024.0;
/// Hosts considered in one search; beyond this the list needs server-side paging in SQL.
const MAX_HOSTS: u64 = 5000;
/// CPU of OpenTelemetry hosts (by state, 0–1) and Datadog Agents (percent).
const HOST_CPU: &[&str] = &["system.cpu.utilization", "system.cpu.idle", "system.cpu.user"];
/// Memory, load and uptime for the host list.
const HOST_VALUES: &[&str] = &["system.memory.utilization", "system.mem.pct_usable", "system.cpu.load_average.1m", "system.load.1", "system.uptime"];

#[derive(Serialize, Deserialize, ToSchema, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum Cloud {
    Aws,
    Azure,
    Gcp,
    Oracle,
    Alibaba,
    None,
}

#[derive(Serialize, Deserialize, ToSchema, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum HostOs {
    Linux,
    Windows,
    Darwin,
}

#[derive(Serialize, Deserialize, ToSchema, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum CollectorKind {
    Observecraft,
    Otel,
    Datadog,
}

#[derive(Serialize, Deserialize, ToSchema, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum Software {
    Kubernetes,
    Docker,
}

#[derive(Serialize, ToSchema, Clone, Default)]
pub struct HostMonitorCounts {
    pub alert: u32,
    pub warn: u32,
    pub ok: u32,
}

#[derive(Serialize, ToSchema, Clone)]
#[serde(rename_all = "camelCase")]
pub struct HostSummary {
    pub name: String,
    pub cloud: Cloud,
    pub os: HostOs,
    pub distro: Option<String>,
    pub collectors: Vec<CollectorKind>,
    pub software: Vec<Software>,
    pub env: Option<String>,
    pub service: Option<String>,
    pub team: Option<String>,
    pub gpu: bool,
    pub monitors: HostMonitorCounts,
    pub cpu_pct: f64,
    pub mem_pct: f64,
    /// APM error spans from this host in range; null when no services on it send traces.
    pub error_spans: Option<u64>,
}

/// Everything known about a host, gathered once per request.
struct HostRow {
    summary: HostSummary,
    res: HashMap<String, String>,
    cpu_user: f64,
    load1: f64,
    uptime: Option<f64>,
}

impl HostRow {
    fn field(&self, key: &str) -> Option<String> {
        let s = &self.summary;
        match key {
            "host" => Some(s.name.clone()),
            "cloud" => Some(enum_str(&s.cloud)),
            "os" => Some(enum_str(&s.os)),
            "env" => s.env.clone(),
            "team" => s.team.clone(),
            "service" => s.service.clone(),
            "gpu" => Some(s.gpu.to_string()),
            "distro" => s.distro.clone(),
            "cluster" | "kube_cluster_name" => self.res.get("k8s.cluster.name").cloned(),
            "region" => self.res.get("cloud.region").cloned(),
            "collector" => Some(s.collectors.iter().map(enum_str).collect::<Vec<_>>().join("+")),
            k => self.res.get(k).cloned(),
        }
    }

    fn metric(&self, metric: &str) -> f64 {
        match metric {
            "system.cpu.user" => self.cpu_user,
            "system.mem.pct_usable" => 100.0 - self.summary.mem_pct,
            "system.uptime" => self.uptime.unwrap_or(0.0),
            "system.load.1" => self.load1,
            _ => 0.0,
        }
    }
}

fn enum_str<T: Serialize>(v: &T) -> String {
    serde_json::to_value(v).ok().and_then(|v| v.as_str().map(str::to_string)).unwrap_or_default()
}

fn finite(v: f64) -> f64 {
    if v.is_finite() {
        v
    } else {
        0.0
    }
}

fn round1(v: f64) -> f64 {
    (v * 10.0).round() / 10.0
}

#[derive(clickhouse::Row, Deserialize)]
struct Inventory {
    name: String,
    res: String,
    idle: f64,
    user: f64,
}

#[derive(clickhouse::Row, Deserialize)]
struct SpanHost {
    name: String,
    service: String,
    errors: u64,
}

/// All hosts that reported host metrics in `range`, with current (last 5 minutes of range) values.
async fn load_hosts(st: &AppState, org: &str, range: TimeRange) -> Result<Vec<HostRow>, ApiError> {
    range.validate()?;
    let recent_from = range.from.max(range.to - 5 * 60_000);
    #[derive(clickhouse::Row, Deserialize)]
    struct Vals {
        name: String,
        mem: f64,
        load1: f64,
        uptime: f64,
    }
    let cpu_src = points_subquery("gauge", HOST_CPU);
    let values_src = points_subquery("gauge", HOST_VALUES);
    let inventory = with_window(st.ch(org), range.from, range.to)
        .query(&format!(
            // OpenTelemetry hosts report system.cpu.utilization by state (0–1); Datadog Agents
            // system.cpu.idle / system.cpu.user in percent.
            "SELECT ResourceAttributes['host.name'] AS name, toJSONString(argMax(ResourceAttributes, TimeUnix)) AS res, \
             avgIf(if(MetricName = 'system.cpu.idle', Value / 100, Value), (MetricName = 'system.cpu.idle' OR Attributes['state'] = 'idle') AND TimeUnix >= fromUnixTimestamp64Milli(?)) AS idle, \
             avgIf(if(MetricName = 'system.cpu.user', Value / 100, Value), (MetricName = 'system.cpu.user' OR Attributes['state'] = 'user') AND TimeUnix >= fromUnixTimestamp64Milli(?)) AS user \
             FROM {cpu_src} \
             WHERE OrgId = {{oc_org:String}} AND MetricName IN ({names}) \
             AND TimeUnix >= fromUnixTimestamp64Milli(?) AND TimeUnix <= fromUnixTimestamp64Milli(?) AND name != '' \
             GROUP BY name ORDER BY name LIMIT ?",
            names = name_list(HOST_CPU),
        ))
        .bind(recent_from)
        .bind(recent_from)
        .bind(range.from)
        .bind(range.to)
        .bind(MAX_HOSTS)
        .fetch_all::<Inventory>();
    let values = with_window(st.ch(org), recent_from, range.to)
        .query(&format!(
            "SELECT ResourceAttributes['host.name'] AS name, \
             avgIf(if(MetricName = 'system.mem.pct_usable', 1 - Value, Value), MetricName = 'system.mem.pct_usable' OR (MetricName = 'system.memory.utilization' AND Attributes['state'] = 'used')) AS mem, \
             avgIf(Value, MetricName IN ('system.cpu.load_average.1m', 'system.load.1')) AS load1, \
             maxIf(Value, MetricName = 'system.uptime') AS uptime \
             FROM {values_src} \
             WHERE OrgId = {{oc_org:String}} AND MetricName IN ({names}) \
             AND TimeUnix >= fromUnixTimestamp64Milli(?) AND TimeUnix <= fromUnixTimestamp64Milli(?) GROUP BY name",
            names = name_list(HOST_VALUES),
        ))
        .bind(recent_from)
        .bind(range.to)
        .fetch_all::<Vals>();
    let spans = st
        .ch(org)
        .query(
            "SELECT ResourceAttributes['host.name'] AS name, topK(1)(ServiceName)[1] AS service, countIf(StatusCode = 'Error') AS errors \
             FROM otel.otel_traces WHERE OrgId = {oc_org:String} AND SpanKind IN ('Server', 'Consumer') \
             AND Timestamp >= fromUnixTimestamp64Milli(?) AND Timestamp <= fromUnixTimestamp64Milli(?) AND name != '' GROUP BY name",
        )
        .bind(range.from)
        .bind(range.to)
        .fetch_all::<SpanHost>();
    // Datadog Agent host metadata (OS, hardware, agent version, cloud, host tags).
    #[derive(sqlx::FromRow)]
    struct DdHost {
        host: String,
        agent_version: String,
        os: String,
        platform: String,
        kernel: String,
        arch: String,
        cpu_cores: Option<i32>,
        memory_bytes: Option<i64>,
        cloud: String,
        instance_id: String,
        host_tags: Vec<String>,
    }
    let dd_hosts = sqlx::query_as::<_, DdHost>(
        "SELECT host, agent_version, os, platform, kernel, arch, cpu_cores, memory_bytes, cloud, instance_id, host_tags FROM datadog_hosts WHERE org_id = $1",
    )
    .bind(org)
    .fetch_all(&st.postgres);
    let (inventory, values, spans, dd_hosts) = tokio::try_join!(
        async { inventory.await.map_err(ApiError::from) },
        async { values.await.map_err(ApiError::from) },
        async { spans.await.map_err(ApiError::from) },
        async { dd_hosts.await.map_err(ApiError::from) }
    )?;
    let dd_hosts: HashMap<String, DdHost> =
        dd_hosts.into_iter().map(|h| (h.host.clone(), h)).collect();
    let values: HashMap<String, Vals> = values.into_iter().map(|v| (v.name.clone(), v)).collect();
    let spans: HashMap<String, SpanHost> = spans.into_iter().map(|s| (s.name.clone(), s)).collect();

    Ok(inventory
        .into_iter()
        .map(|i| {
            let mut res: HashMap<String, String> = serde_json::from_str(&i.res).unwrap_or_default();
            if let Some(d) = dd_hosts.get(&i.name) {
                // What the Datadog Agent reports about the host, under the OTel resource names
                // the rest of the page reads (resource attributes from metrics win).
                let mut fill = |k: &str, v: String| {
                    if !v.is_empty() {
                        res.entry(k.to_string()).or_insert(v);
                    }
                };
                fill("os.type", d.os.clone());
                fill("os.description", d.platform.clone());
                fill("os.version", d.kernel.clone());
                fill("host.arch", d.arch.clone());
                fill("cloud.provider", d.cloud.clone());
                fill("host.id", d.instance_id.clone());
                fill(
                    "host.cpu.count",
                    d.cpu_cores.map(|c| c.to_string()).unwrap_or_default(),
                );
                fill(
                    "host.memory.total",
                    d.memory_bytes.map(|m| m.to_string()).unwrap_or_default(),
                );
                fill(
                    "observecraft.collector.version",
                    if d.agent_version.is_empty() {
                        String::new()
                    } else {
                        format!("datadog-agent {}", d.agent_version)
                    },
                );
                for tag in &d.host_tags {
                    if let Some((k, v)) = tag.split_once(':') {
                        let key = oc_query::resource_alias(k).unwrap_or(k);
                        fill(key, v.to_string());
                    }
                }
            }
            let v = values.get(&i.name);
            let span = spans.get(&i.name);
            let get = |k: &str| res.get(k).filter(|v| !v.is_empty()).cloned();
            let cloud = match get("cloud.provider").as_deref() {
                Some("aws") => Cloud::Aws,
                Some("azure") => Cloud::Azure,
                Some("gcp") => Cloud::Gcp,
                Some("oracle_cloud" | "oracle") => Cloud::Oracle,
                Some("alibaba_cloud" | "alibaba") => Cloud::Alibaba,
                _ => Cloud::None,
            };
            let os = match get("os.type").as_deref() {
                Some("windows") => HostOs::Windows,
                Some("darwin") => HostOs::Darwin,
                _ => HostOs::Linux,
            };
            let distro = get("os.description").and_then(|d| {
                let d = d.to_lowercase();
                ["ubuntu", "debian", "centos", "redhat", "amazon", "alpine"].into_iter().find(|x| d.contains(x)).map(str::to_string)
            });
            let collector = match get("observecraft.collector").as_deref() {
                Some("observecraft" | "observecraft-agent" | "observecraft-cluster-agent") => {
                    CollectorKind::Observecraft
                }
                Some("datadog-agent") => CollectorKind::Datadog,
                _ => CollectorKind::Otel,
            };
            let software = if get("k8s.cluster.name").is_some() {
                vec![Software::Kubernetes]
            } else {
                vec![]
            };
            let summary = HostSummary {
                name: i.name.clone(),
                cloud,
                os,
                distro,
                collectors: vec![collector],
                software,
                env: get("deployment.environment"),
                service: span.map(|s| s.service.clone()).filter(|s| !s.is_empty()),
                team: get("team"),
                gpu: false,
                monitors: HostMonitorCounts::default(),
                cpu_pct: round1(finite((1.0 - i.idle) * 100.0).clamp(0.0, 100.0)),
                mem_pct: round1(finite(v.map_or(f64::NAN, |v| v.mem) * 100.0)),
                error_spans: span.map(|s| s.errors),
            };
            HostRow {
                summary,
                cpu_user: round1(finite(i.user * 100.0)),
                load1: v.map_or(0.0, |v| finite(v.load1)),
                uptime: v.map(|v| v.uptime).filter(|u| u.is_finite() && *u > 0.0),
                res,
            }
        })
        .collect())
}

// ================================================================ search

#[derive(Deserialize, ToSchema)]
pub struct MetricFilter {
    pub metric: String,
    pub min: Option<f64>,
    pub max: Option<f64>,
}

#[derive(Deserialize, ToSchema, Clone, Copy)]
#[serde(rename_all = "snake_case")]
pub enum HostSortField {
    Name,
    Cpu,
    Mem,
    Env,
    Monitors,
}

#[derive(Deserialize, ToSchema)]
pub struct HostSort {
    pub field: HostSortField,
    pub dir: super::dashboards::SortDir,
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct HostSearchRequest {
    pub range: TimeRange,
    /// `cloud:aws os:linux env:prod team:x collector:otel software:kubernetes` + free text on the name.
    #[serde(default)]
    pub query: String,
    #[serde(default)]
    pub metric_filters: Vec<MetricFilter>,
    pub group_by: Option<String>,
    pub sort: HostSort,
    pub offset: usize,
    pub limit: usize,
}

#[derive(Serialize, ToSchema)]
pub struct HostListItem {
    #[serde(flatten)]
    pub host: HostSummary,
    pub group: Option<String>,
}

#[derive(Serialize, ToSchema)]
pub struct HostGroup {
    pub key: String,
    pub count: usize,
}

#[derive(Serialize, ToSchema)]
pub struct HostSearchPage {
    pub hosts: Vec<HostListItem>,
    pub total: usize,
    /// When grouped: every group in the full result, in display order.
    pub groups: Option<Vec<HostGroup>>,
}

fn host_matches(h: &HostRow, terms: &[Term]) -> bool {
    let empty = HashMap::new();
    let mut tags = HashMap::new();
    for key in ["cloud", "os", "env", "team", "service", "gpu", "distro", "cluster", "kube_cluster_name", "region"] {
        if let Some(v) = h.field(key) {
            tags.insert(key.to_string(), v);
        }
    }
    // Multi-valued facets: a host matches when any of its values does.
    let (multi, rest): (Vec<&Term>, Vec<&Term>) = terms.iter().partition(|t| matches!(t, Term::Tag { key, .. } if key == "collector" || key == "software"));
    for t in multi {
        if let Term::Tag { key, values, negate } = t {
            let have: Vec<String> = if key == "collector" { h.summary.collectors.iter().map(enum_str).collect() } else { h.summary.software.iter().map(enum_str).collect() };
            if values.iter().any(|v| have.contains(v)) == *negate {
                return false;
            }
        }
    }
    let rest: Vec<Term> = rest.into_iter().cloned().collect();
    matches(&rest, &Fields { service: h.summary.service.as_deref().unwrap_or(""), status: "", host: &h.summary.name, source: "", message: &h.summary.name, tags: &tags, attr: &empty, index: None })
}

#[utoipa::path(post, path = "/api/v1/hosts/search", operation_id = "search_hosts", request_body = HostSearchRequest, responses((status = 200, body = HostSearchPage)), tag = "infrastructure")]
pub async fn search(State(st): State<AppState>, OrgId(org): OrgId, Json(req): Json<HostSearchRequest>) -> ApiResult<HostSearchPage> {
    let terms = parse_query(&req.query);
    let mut hosts: Vec<HostRow> = load_hosts(&st, &org, req.range)
        .await?
        .into_iter()
        .filter(|h| host_matches(h, &terms))
        .filter(|h| {
            req.metric_filters.iter().all(|f| {
                let v = h.metric(&f.metric);
                f.min.map_or(true, |m| v >= m) && f.max.map_or(true, |m| v <= m)
            })
        })
        .collect();
    let dir = match req.sort.dir {
        super::dashboards::SortDir::Asc => std::cmp::Ordering::Less,
        super::dashboards::SortDir::Desc => std::cmp::Ordering::Greater,
    };
    let by_field = |a: &HostRow, b: &HostRow| {
        let o = match req.sort.field {
            HostSortField::Cpu => a.summary.cpu_pct.total_cmp(&b.summary.cpu_pct),
            HostSortField::Mem => a.summary.mem_pct.total_cmp(&b.summary.mem_pct),
            HostSortField::Env => a.summary.env.cmp(&b.summary.env),
            HostSortField::Monitors => (a.summary.monitors.alert, a.summary.monitors.warn).cmp(&(b.summary.monitors.alert, b.summary.monitors.warn)),
            HostSortField::Name => a.summary.name.cmp(&b.summary.name),
        };
        if dir == std::cmp::Ordering::Less {
            o
        } else {
            o.reverse()
        }
    };
    let total = hosts.len();
    let (items, groups) = match req.group_by.as_deref().filter(|g| !g.is_empty()) {
        Some(g) => {
            let group_of = |h: &HostRow| h.field(g).filter(|v| !v.is_empty()).unwrap_or_else(|| format!("no {g}"));
            let mut counts: BTreeMap<String, usize> = BTreeMap::new();
            for h in &hosts {
                *counts.entry(group_of(h)).or_default() += 1;
            }
            let mut groups: Vec<HostGroup> = counts.into_iter().map(|(key, count)| HostGroup { key, count }).collect();
            groups.sort_by(|a, b| b.count.cmp(&a.count).then(a.key.cmp(&b.key)));
            let rank: HashMap<&str, usize> = groups.iter().enumerate().map(|(i, g)| (g.key.as_str(), i)).collect();
            hosts.sort_by(|a, b| rank[group_of(a).as_str()].cmp(&rank[group_of(b).as_str()]).then_with(|| by_field(a, b)));
            let items = hosts.iter().skip(req.offset).take(req.limit.min(500)).map(|h| HostListItem { host: h.summary.clone(), group: Some(group_of(h)) }).collect();
            (items, Some(groups))
        }
        None => {
            hosts.sort_by(by_field);
            (hosts.iter().skip(req.offset).take(req.limit.min(500)).map(|h| HostListItem { host: h.summary.clone(), group: None }).collect(), None)
        }
    };
    Ok(Json(HostSearchPage { hosts: items, total, groups }))
}

// ================================================================ host panel

#[derive(Serialize, ToSchema)]
pub struct TriggeredMonitor {
    pub id: String,
    pub name: String,
    pub status: String,
    pub query: String,
    pub threshold: f64,
    pub series: Vec<(i64, f64)>,
}

#[derive(Serialize, ToSchema)]
pub struct HostIntegration {
    pub name: String,
    pub status: String,
    pub metrics: Vec<String>,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct KubernetesInfo {
    pub cluster: String,
    pub cri_name: String,
    pub cri_version: String,
    pub kubelet_version: String,
}

#[derive(Serialize, ToSchema)]
pub struct Pipeline {
    pub name: String,
    pub receivers: Vec<String>,
    pub processors: Vec<String>,
    pub exporters: Vec<String>,
}

#[derive(Serialize, ToSchema, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Product {
    Apm,
    Infra,
    Logs,
    Processes,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct HostDetail {
    #[serde(flatten)]
    pub host: HostSummary,
    pub account: Option<String>,
    pub region: Option<String>,
    /// "EC2 Instance", "Other Host", …
    pub host_type: String,
    pub collector_version: String,
    pub uptime_sec: f64,
    pub ntp_offset_ms: f64,
    pub tags: BTreeMap<String, String>,
    pub products: Vec<Product>,
    pub triggered_monitors: Vec<TriggeredMonitor>,
    pub integrations: Vec<HostIntegration>,
    pub kubernetes: Option<KubernetesInfo>,
    /// Collector config applied to this host, when the collector reports it.
    pub collector_config: String,
    pub pipelines: Vec<Pipeline>,
}

#[derive(Deserialize, ToSchema)]
pub struct HostRequest {
    pub range: TimeRange,
}

/// Resource attributes not worth showing as tags.
const HIDDEN_TAGS: &[&str] = &[
    "oc.org_id",
    "oc.retention_days",
    "host.id",
    "observecraft.collector.version",
    "host.cpu.count",
    "process.pid",
    "process.command",
    "process.owner",
    "process.executable.name",
];

#[utoipa::path(post, path = "/api/v1/hosts/{name}", params(("name" = String, Path)), request_body = HostRequest, responses((status = 200, body = HostDetail), (status = 404, body = crate::error::ErrorBody)), tag = "infrastructure")]
pub async fn detail(State(st): State<AppState>, OrgId(org): OrgId, Path(name): Path<String>, Json(req): Json<HostRequest>) -> ApiResult<HostDetail> {
    let host = load_hosts(&st, &org, req.range).await?.into_iter().find(|h| h.summary.name == name).ok_or_else(|| ApiError::NotFound(format!("Host {name} has not reported in this time range")))?;
    let has_logs = st
        .ch(&org)
        .query("SELECT count() FROM oc.logs WHERE org_id = ? AND host = ? AND timestamp >= fromUnixTimestamp64Milli(?) AND timestamp <= fromUnixTimestamp64Milli(?)")
        .bind(&org)
        .bind(&name)
        .bind(req.range.from)
        .bind(req.range.to)
        .fetch_one::<u64>();
    let now = now_ms();
    let procs_src = points_subquery("gauge", &["process.cpu.utilization"]);
    let has_procs = with_window(st.ch(&org), now - 15 * 60_000, now + OPEN_END_MS)
        .query(&format!("SELECT count() FROM {procs_src} WHERE OrgId = {{oc_org:String}} AND MetricName = 'process.cpu.utilization' AND ResourceAttributes['host.name'] = ? AND TimeUnix > now() - INTERVAL 15 MINUTE"))
        .bind(&name)
        .fetch_one::<u64>();
    let (has_logs, has_procs) = tokio::try_join!(has_logs, has_procs)?;
    let get = |k: &str| host.res.get(k).filter(|v| !v.is_empty()).cloned();
    let mut products = vec![Product::Infra];
    if host.summary.error_spans.is_some() {
        products.push(Product::Apm);
    }
    if has_logs > 0 {
        products.push(Product::Logs);
    }
    if has_procs > 0 {
        products.push(Product::Processes);
    }
    let host_type = match get("cloud.platform").as_deref() {
        Some("aws_ec2") => "EC2 Instance",
        Some("gcp_compute_engine") => "GCE Instance",
        Some("azure_vm") => "Azure VM",
        _ => "Other Host",
    }
    .to_string();
    let tags = host
        .res
        .iter()
        .filter(|(k, v)| !v.is_empty() && !HIDDEN_TAGS.contains(&k.as_str()))
        .map(|(k, v)| (short_tag(k).map(str::to_string).unwrap_or_else(|| k.clone()), v.clone()))
        .collect();
    let node = match get("k8s.cluster.name") {
        Some(_) => super::kubernetes::host_inventory(&st, &org, &name).await?.0,
        None => None,
    };
    let kubernetes = get("k8s.cluster.name").or(node.as_ref().map(|n| n.cluster.clone())).map(|cluster| {
        // containerd://1.7.18 → (containerd, 1.7.18)
        let runtime = node.as_ref().and_then(|n| n.runtime.clone()).unwrap_or_default();
        let (cri_name, cri_version) = runtime.split_once("://").map(|(a, b)| (a.to_string(), b.to_string())).unwrap_or_else(|| ("—".into(), "—".into()));
        KubernetesInfo { cluster, cri_name, cri_version, kubelet_version: node.as_ref().and_then(|n| n.version.clone()).unwrap_or_else(|| "—".into()) }
    });
    let collector_config = "# This host's collector does not report its configuration.\n# Configurations pushed from ObserveCraft (Fleet Automation) will appear here.\n".to_string();
    Ok(Json(HostDetail {
        account: get("cloud.account.id"),
        region: get("cloud.region"),
        host_type,
        collector_version: get("observecraft.collector.version").unwrap_or_else(|| "unknown".into()),
        uptime_sec: host.uptime.unwrap_or(0.0),
        ntp_offset_ms: 0.0,
        tags,
        products,
        triggered_monitors: vec![],
        integrations: vec![],
        kubernetes,
        collector_config,
        pipelines: vec![],
        host: host.summary,
    }))
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProcessShare {
    pub name: String,
    pub value: f64,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct HostMetrics {
    pub cpu: Vec<MetricSeries>,
    pub load: Vec<MetricSeries>,
    /// `[usable GiB, total GiB]`.
    pub memory: Vec<MetricSeries>,
    /// Share of total memory per process (%), largest first.
    pub process_memory: Vec<ProcessShare>,
}

fn host_query(agg: SpaceAgg, metric: &str, host: &str, extra: &[(&str, &str, bool)]) -> MetricQuery {
    let mut filters = vec![TagFilter { key: "host".into(), value: host.into(), negate: false }];
    filters.extend(extra.iter().map(|(k, v, n)| TagFilter { key: k.to_string(), value: v.to_string(), negate: *n }));
    MetricQuery { agg, metric: metric.into(), filters, by: vec![] }
}

#[utoipa::path(post, path = "/api/v1/hosts/{name}/metrics", params(("name" = String, Path)), request_body = HostRequest, responses((status = 200, body = HostMetrics)), tag = "infrastructure")]
pub async fn metrics(State(st): State<AppState>, OrgId(org): OrgId, Path(name): Path<String>, Json(req): Json<HostRequest>) -> ApiResult<HostMetrics> {
    let series = |label: String, groups: Vec<super::metrics::Group>, scale: f64| MetricSeries {
        name: label,
        points: groups.into_iter().next().map(|g| g.points.into_iter().map(|(t, v)| (t, (v * scale * 100.0).round() / 100.0)).collect()).unwrap_or_default(),
    };
    let cpu_states = ["user", "system", "iowait", "idle"];
    let mut cpu = Vec::new();
    for s in cpu_states {
        let g = evaluate_query(&st, &org, &host_query(SpaceAgg::Avg, &format!("system.cpu.{s}"), &name, &[]), req.range, 120).await?;
        cpu.push(series(format!("system.cpu.{s}{{host:{name}}}"), g, 1.0));
    }
    let mut load = Vec::new();
    for w in ["1", "5", "15"] {
        let g = evaluate_query(&st, &org, &host_query(SpaceAgg::Avg, &format!("system.load.{w}"), &name, &[]), req.range, 120).await?;
        load.push(series(format!("system.load.{w}{{host:{name}}}"), g, 1.0));
    }
    let usable = evaluate_query(&st, &org, &host_query(SpaceAgg::Sum, "system.memory.usage", &name, &[("state", "used", true)]), req.range, 120).await?;
    let total = evaluate_query(&st, &org, &host_query(SpaceAgg::Sum, "system.memory.usage", &name, &[]), req.range, 120).await?;
    let total_bytes = total.first().and_then(|g| g.points.last()).map(|p| p.1).unwrap_or(0.0);
    let memory = vec![series(format!("sum:system.mem.usable{{host:{name}}}"), usable, 1.0 / GIB), series(format!("sum:system.mem.total{{host:{name}}}"), total, 1.0 / GIB)];

    #[derive(clickhouse::Row, Deserialize)]
    struct Proc {
        name: String,
        rss: f64,
    }
    let now = now_ms();
    let mem_src = points_subquery("sum", &["process.memory.usage"]);
    let procs = with_window(st.ch(&org), now - 10 * 60_000, now + OPEN_END_MS)
        .query(&format!(
            "SELECT name, sum(rss) AS rss FROM (\
               SELECT ResourceAttributes['process.pid'] AS pid, any(ResourceAttributes['process.executable.name']) AS name, argMax(Value, TimeUnix) AS rss \
               FROM {mem_src} \
               WHERE OrgId = {{oc_org:String}} AND MetricName = 'process.memory.usage' AND ResourceAttributes['host.name'] = ? AND TimeUnix > now() - INTERVAL 10 MINUTE \
               GROUP BY pid) GROUP BY name ORDER BY rss DESC"
        ))
        .bind(&name)
        .fetch_all::<Proc>()
        .await?;
    let mut process_memory: Vec<ProcessShare> = Vec::new();
    if total_bytes > 0.0 {
        let mut other = 0.0;
        for (i, p) in procs.iter().enumerate() {
            let pct = p.rss / total_bytes * 100.0;
            if i < 6 {
                process_memory.push(ProcessShare { name: p.name.clone(), value: (pct * 100.0).round() / 100.0 });
            } else {
                other += pct;
            }
        }
        if other > 0.0 {
            process_memory.push(ProcessShare { name: "other".into(), value: (other * 100.0).round() / 100.0 });
        }
    }
    Ok(Json(HostMetrics { cpu, load, memory, process_memory }))
}

#[derive(Deserialize, ToSchema)]
pub struct HostPageRequest {
    #[serde(default)]
    pub query: String,
    pub offset: usize,
    pub limit: usize,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct Process {
    pub pid: i64,
    pub command: String,
    pub user: String,
    pub cpu_pct: f64,
    pub rss_bytes: f64,
}

#[derive(Serialize, ToSchema)]
pub struct ProcessPage {
    pub items: Vec<Process>,
    pub total: usize,
    /// False when the host's collector doesn't report process metrics.
    pub enabled: bool,
}

/// Processes seen in the last 10 minutes, by CPU.
#[utoipa::path(post, path = "/api/v1/hosts/{name}/processes", params(("name" = String, Path)), request_body = HostPageRequest, responses((status = 200, body = ProcessPage)), tag = "infrastructure")]
pub async fn processes(State(st): State<AppState>, OrgId(org): OrgId, Path(name): Path<String>, Json(req): Json<HostPageRequest>) -> ApiResult<ProcessPage> {
    #[derive(clickhouse::Row, Deserialize)]
    struct Row {
        pid: String,
        command: String,
        user: String,
        cpu: f64,
        rss: f64,
    }
    let now = now_ms();
    let cpu_src = points_subquery("gauge", &["process.cpu.utilization"]);
    let mem_src = points_subquery("sum", &["process.memory.usage"]);
    let rows = with_window(st.ch(&org), now - 10 * 60_000, now + OPEN_END_MS)
        .query(&format!(
            "SELECT ResourceAttributes['process.pid'] AS pid, any(ResourceAttributes['process.command']) AS command, any(ResourceAttributes['process.owner']) AS user, \
             avgIf(Value, MetricName = 'process.cpu.utilization') AS cpu, argMaxIf(Value, TimeUnix, MetricName = 'process.memory.usage') AS rss FROM (\
               SELECT ResourceAttributes, MetricName, Value, TimeUnix FROM {cpu_src} WHERE OrgId = {{oc_org:String}} AND MetricName = 'process.cpu.utilization' AND ResourceAttributes['host.name'] = ? AND TimeUnix > now() - INTERVAL 10 MINUTE \
               UNION ALL SELECT ResourceAttributes, MetricName, Value, TimeUnix FROM {mem_src} WHERE OrgId = {{oc_org:String}} AND MetricName = 'process.memory.usage' AND ResourceAttributes['host.name'] = ? AND TimeUnix > now() - INTERVAL 10 MINUTE) \
             GROUP BY pid ORDER BY cpu DESC"
        ))
        .bind(&name)
        .bind(&name)
        .fetch_all::<Row>()
        .await?;
    let enabled = !rows.is_empty();
    let q = req.query.trim().to_lowercase();
    let matched: Vec<Process> = rows
        .into_iter()
        .filter(|r| q.is_empty() || r.command.to_lowercase().contains(&q) || r.user.to_lowercase().contains(&q))
        .map(|r| Process { pid: r.pid.parse().unwrap_or(0), command: r.command, user: r.user, cpu_pct: round1(finite(r.cpu * 100.0)), rss_bytes: finite(r.rss).round() })
        .collect();
    let total = matched.len();
    Ok(Json(ProcessPage { items: matched.into_iter().skip(req.offset).take(req.limit.min(500)).collect(), total, enabled }))
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct HostPod {
    pub uid: String,
    pub name: String,
    /// As `kubectl get pods` prints it: Running, CrashLoopBackOff, Completed, …
    pub status: String,
    pub health: super::kubernetes::KubeHealth,
    pub cluster: String,
    pub namespace: String,
    pub age_sec: Option<f64>,
    pub ready: String,
    pub restarts: u32,
    pub cpu_pct_of_limit: Option<f64>,
    pub mem_pct_of_limit: Option<f64>,
}

#[derive(Serialize, ToSchema)]
pub struct HostPodPage {
    pub items: Vec<HostPod>,
    pub total: usize,
}

#[derive(Serialize, ToSchema, Clone, Copy)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum HostContainerStatus {
    Up,
    Waiting,
    Exited,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct HostContainer {
    pub name: String,
    pub pod: String,
    pub status: HostContainerStatus,
    pub image: String,
    /// CPU use as a share of the node's cores.
    pub cpu_pct: Option<f64>,
    /// Working set.
    pub rss_bytes: Option<f64>,
    /// Container memory limit, else the node's allocatable memory.
    pub mem_limit_bytes: Option<f64>,
    pub started_at: Option<i64>,
}

#[derive(Serialize, ToSchema)]
pub struct HostContainerPage {
    pub items: Vec<HostContainer>,
    pub total: usize,
}

fn pct_of(u: &super::kubernetes::KubeUtilization) -> Option<f64> {
    Some(round1(u.usage? / u.limits.filter(|l| *l > 0.0)? * 100.0))
}

/// Pods the Kubernetes collector reports on this host (its node).
#[utoipa::path(post, path = "/api/v1/hosts/{name}/pods", params(("name" = String, Path)), request_body = HostPageRequest, responses((status = 200, body = HostPodPage)), tag = "infrastructure")]
pub async fn pods(State(st): State<AppState>, OrgId(org): OrgId, Path(name): Path<String>, Json(req): Json<HostPageRequest>) -> ApiResult<HostPodPage> {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis() as i64;
    let q = req.query.trim().to_lowercase();
    let (_, pods) = super::kubernetes::host_inventory(&st, &org, &name).await?;
    let mut items: Vec<HostPod> = pods
        .into_iter()
        .filter(|p| q.is_empty() || p.name.to_lowercase().contains(&q))
        .map(|p| HostPod {
            age_sec: p.created_at.map(|c| ((now - c) / 1000) as f64),
            ready: p.ready.clone().unwrap_or_default(),
            restarts: p.restarts.unwrap_or(0),
            cpu_pct_of_limit: p.cpu.as_ref().and_then(pct_of),
            mem_pct_of_limit: p.memory.as_ref().and_then(pct_of),
            namespace: p.namespace.unwrap_or_default(),
            uid: p.uid,
            name: p.name,
            status: p.status,
            health: p.health,
            cluster: p.cluster,
        })
        .collect();
    items.sort_by(|a, b| a.name.cmp(&b.name));
    let total = items.len();
    Ok(Json(HostPodPage { items: items.into_iter().skip(req.offset).take(req.limit.min(500)).collect(), total }))
}

/// Containers of the pods on this host, by memory.
#[utoipa::path(post, path = "/api/v1/hosts/{name}/containers", params(("name" = String, Path)), request_body = HostPageRequest, responses((status = 200, body = HostContainerPage)), tag = "infrastructure")]
pub async fn containers(State(st): State<AppState>, OrgId(org): OrgId, Path(name): Path<String>, Json(req): Json<HostPageRequest>) -> ApiResult<HostContainerPage> {
    let q = req.query.trim().to_lowercase();
    let (node, pods) = super::kubernetes::host_inventory(&st, &org, &name).await?;
    let cores = node.as_ref().and_then(|n| n.cpu.as_ref()).and_then(|c| c.capacity).filter(|c| *c > 0.0);
    let node_mem = node.as_ref().and_then(|n| n.memory.as_ref()).and_then(|m| m.capacity);
    let mut items: Vec<HostContainer> = pods
        .iter()
        .flat_map(|p| p.containers.iter().filter(|c| !c.init).map(move |c| (p, c)))
        .filter(|(p, c)| q.is_empty() || c.name.to_lowercase().contains(&q) || c.image.to_lowercase().contains(&q) || p.name.to_lowercase().contains(&q))
        .map(|(p, c)| HostContainer {
            name: c.name.clone(),
            pod: p.name.clone(),
            status: match c.state.as_str() {
                "running" => HostContainerStatus::Up,
                "terminated" => HostContainerStatus::Exited,
                _ => HostContainerStatus::Waiting,
            },
            image: c.image.clone(),
            cpu_pct: c.cpu.usage.zip(cores).map(|(u, n)| round1(u / n * 100.0)),
            rss_bytes: c.memory.usage,
            mem_limit_bytes: c.memory.limits.or(node_mem),
            started_at: c.started_at,
        })
        .collect();
    items.sort_by(|a, b| b.rss_bytes.unwrap_or(0.0).total_cmp(&a.rss_bytes.unwrap_or(0.0)).then(a.name.cmp(&b.name)));
    let total = items.len();
    Ok(Json(HostContainerPage { items: items.into_iter().skip(req.offset).take(req.limit.min(500)).collect(), total }))
}
