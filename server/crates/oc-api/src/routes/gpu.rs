//! GPU Monitoring: the fleet of GPUs, from the metrics their hosts already send:
//! - the Datadog Agent's GPU check (`gpu.*`, tagged `gpu_uuid`, `gpu_device`, `gpu_vendor`, …;
//!   names from `integrations-core/gpu/metadata.csv`, BSD-3-Clause, Copyright (c) Datadog, Inc.),
//! - the Datadog Agent's DCGM check (`dcgm.*`, `integrations-core/dcgm/metadata.csv`),
//! - NVIDIA's DCGM exporter scraped by an OpenTelemetry Collector (`DCGM_FI_DEV_*`, labelled
//!   `gpu`, `UUID`, `modelName`, `Hostname` and, with Kubernetes pod mapping, `namespace` / `pod` / `container`).
//!
//! A GPU is one device of a host (its UUID, else its index); several sources for the same device merge.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use axum::extract::State;
use axum::Json;
use oc_query::{matches, parse_query, Fields, Term};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use super::logs::TimeRange;
use super::metric_inventory::{chart_step, series_values, SeriesValue};
use crate::error::ApiResult;
use crate::state::{AppState, OrgId};

const MIB: f64 = 1024.0 * 1024.0;
/// Current values are the average of the last minutes of the range.
const CURRENT_MS: i64 = 10 * 60_000;
/// Below this utilization (%) a GPU is idle; below [`UNDERUSED_PCT`] it is underutilized.
const IDLE_PCT: f64 = 5.0;
const UNDERUSED_PCT: f64 = 30.0;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum GpuField {
    Util,
    MemUsed,
    MemFree,
    MemTotal,
    Temp,
    Power,
}

/// Metric → (field, scale to %, bytes, °C or W). Agent GPU memory is in bytes and power in mW;
/// DCGM frame buffer sizes are in MiB and power in W.
const GPU_METRICS: &[(&str, GpuField, f64)] = &[
    ("gpu.gr_engine_active", GpuField::Util, 1.0),
    ("gpu.memory.limit", GpuField::MemTotal, 1.0),
    ("gpu.memory.free", GpuField::MemFree, 1.0),
    ("gpu.temperature", GpuField::Temp, 1.0),
    ("gpu.power.usage", GpuField::Power, 0.001),
    ("dcgm.gpu_utilization", GpuField::Util, 1.0),
    ("dcgm.frame_buffer.used", GpuField::MemUsed, MIB),
    ("dcgm.frame_buffer.free", GpuField::MemFree, MIB),
    ("dcgm.frame_buffer.total", GpuField::MemTotal, MIB),
    ("dcgm.temperature", GpuField::Temp, 1.0),
    ("dcgm.power_usage", GpuField::Power, 1.0),
    ("DCGM_FI_DEV_GPU_UTIL", GpuField::Util, 1.0),
    ("DCGM_FI_DEV_FB_USED", GpuField::MemUsed, MIB),
    ("DCGM_FI_DEV_FB_FREE", GpuField::MemFree, MIB),
    ("DCGM_FI_DEV_GPU_TEMP", GpuField::Temp, 1.0),
    ("DCGM_FI_DEV_POWER_USAGE", GpuField::Power, 1.0),
];

const HOST_KEYS: &[&str] = &["Hostname", "host.name", "host"];
const UUID_KEYS: &[&str] = &["gpu_uuid", "UUID", "uuid"];
const INDEX_KEYS: &[&str] = &["gpu_index", "gpu", "minor_number"];
const DEVICE_KEYS: &[&str] = &["device"];
const MODEL_KEYS: &[&str] = &["gpu_device", "modelName", "model_name", "gpu_model"];
const VENDOR_KEYS: &[&str] = &["gpu_vendor"];
const DRIVER_KEYS: &[&str] = &["gpu_driver_version", "DCGM_FI_DRIVER_VERSION", "driver_version"];
const CLUSTER_KEYS: &[&str] = &["k8s.cluster.name", "kube_cluster_name", "cluster_name"];
const NAMESPACE_KEYS: &[&str] = &["k8s.namespace.name", "namespace", "kube_namespace"];
const POD_KEYS: &[&str] = &["k8s.pod.name", "pod", "pod_name"];
const CONTAINER_KEYS: &[&str] = &["k8s.container.name", "container", "container_name"];

fn metric_names() -> Vec<&'static str> {
    GPU_METRICS.iter().map(|(n, _, _)| *n).collect()
}

fn field_of(metric: &str) -> Option<(GpuField, f64)> {
    GPU_METRICS.iter().find(|(n, _, _)| *n == metric).map(|(_, f, s)| (*f, *s))
}

/// What reports a GPU.
#[derive(Serialize, ToSchema, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
#[serde(rename_all = "snake_case")]
pub enum GpuSource {
    /// The Datadog Agent's GPU check.
    DatadogGpu,
    /// The Datadog Agent's DCGM check.
    DatadogDcgm,
    /// NVIDIA's DCGM exporter through an OpenTelemetry Collector.
    DcgmExporter,
}

fn source_of(metric: &str) -> GpuSource {
    if metric.starts_with("gpu.") {
        GpuSource::DatadogGpu
    } else if metric.starts_with("dcgm.") {
        GpuSource::DatadogDcgm
    } else {
        GpuSource::DcgmExporter
    }
}

#[derive(Serialize, ToSchema, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum GpuState {
    /// Under 5 % utilization.
    Idle,
    /// Under 30 % utilization.
    Underutilized,
    Active,
    /// No utilization metric.
    NoData,
}

fn gpu_state(util: Option<f64>) -> GpuState {
    match util {
        None => GpuState::NoData,
        Some(u) if u < IDLE_PCT => GpuState::Idle,
        Some(u) if u < UNDERUSED_PCT => GpuState::Underutilized,
        Some(_) => GpuState::Active,
    }
}

#[derive(Serialize, ToSchema, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct GpuDevice {
    /// `host/uuid` (or `host/gpu<index>`).
    pub id: String,
    pub host: String,
    pub uuid: Option<String>,
    pub index: Option<String>,
    pub model: Option<String>,
    pub vendor: Option<String>,
    pub driver: Option<String>,
    pub cluster: Option<String>,
    pub util_pct: Option<f64>,
    pub mem_used_bytes: Option<f64>,
    pub mem_total_bytes: Option<f64>,
    pub temp_c: Option<f64>,
    pub power_w: Option<f64>,
    /// `namespace/pod` (or container) of the workloads its metrics are tagged with.
    pub workloads: Vec<String>,
    pub state: GpuState,
    pub sources: Vec<GpuSource>,
}

/// Values of one device being merged: per field, the values of its series.
#[derive(Default)]
struct Acc {
    host: String,
    uuid: Option<String>,
    index: Option<String>,
    model: Option<String>,
    vendor: Option<String>,
    driver: Option<String>,
    cluster: Option<String>,
    fields: HashMap<&'static str, Vec<f64>>,
    workloads: BTreeSet<String>,
    sources: BTreeSet<GpuSource>,
}

fn field_key(f: GpuField) -> &'static str {
    match f {
        GpuField::Util => "util",
        GpuField::MemUsed => "used",
        GpuField::MemFree => "free",
        GpuField::MemTotal => "total",
        GpuField::Temp => "temp",
        GpuField::Power => "power",
    }
}

fn round1(v: f64) -> f64 {
    (v * 10.0).round() / 10.0
}

/// The GPUs behind `values` (one window), sorted by host and index.
pub(crate) fn aggregate_gpus(values: &[SeriesValue]) -> Vec<GpuDevice> {
    let mut accs: BTreeMap<String, Acc> = BTreeMap::new();
    for v in values {
        let Some((field, scale)) = field_of(&v.metric) else { continue };
        let host = v.tag(HOST_KEYS).unwrap_or_else(|| "unknown".into());
        let uuid = v.tag(UUID_KEYS);
        let index = v.tag(INDEX_KEYS);
        let device = uuid.as_ref().map(|u| u.to_lowercase()).or_else(|| index.as_ref().map(|i| format!("gpu{i}"))).or_else(|| v.tag(DEVICE_KEYS)).unwrap_or_else(|| "gpu".into());
        let a = accs.entry(format!("{host}/{device}")).or_default();
        a.host = host;
        let fill = |slot: &mut Option<String>, keys: &[&str]| {
            if slot.is_none() {
                *slot = v.tag(keys);
            }
        };
        fill(&mut a.uuid, UUID_KEYS);
        fill(&mut a.index, INDEX_KEYS);
        fill(&mut a.model, MODEL_KEYS);
        fill(&mut a.vendor, VENDOR_KEYS);
        fill(&mut a.driver, DRIVER_KEYS);
        fill(&mut a.cluster, CLUSTER_KEYS);
        let workload = match (v.tag(NAMESPACE_KEYS), v.tag(POD_KEYS), v.tag(CONTAINER_KEYS)) {
            (ns, Some(pod), _) => Some(ns.map(|n| format!("{n}/{pod}")).unwrap_or(pod)),
            (_, None, Some(c)) => Some(c),
            _ => None,
        };
        a.workloads.extend(workload);
        a.sources.insert(source_of(&v.metric));
        a.fields.entry(field_key(field)).or_default().push(v.value * scale);
    }
    let mut out: Vec<GpuDevice> = accs
        .into_iter()
        .map(|(id, a)| {
            let avg = |k: &str| a.fields.get(k).filter(|v| !v.is_empty()).map(|v| v.iter().sum::<f64>() / v.len() as f64);
            let (used, free, total) = (avg("used"), avg("free"), avg("total"));
            let total = total.or(used.zip(free).map(|(u, f)| u + f));
            let used = used.or(total.zip(free).map(|(t, f)| (t - f).max(0.0)));
            let util = avg("util").map(|u| round1(u.clamp(0.0, 100.0)));
            GpuDevice {
                id,
                host: a.host,
                uuid: a.uuid,
                index: a.index,
                model: a.model,
                vendor: a.vendor,
                driver: a.driver,
                cluster: a.cluster,
                util_pct: util,
                mem_used_bytes: used.map(f64::round),
                mem_total_bytes: total.map(f64::round),
                temp_c: avg("temp").map(round1),
                power_w: avg("power").map(round1),
                workloads: a.workloads.into_iter().collect(),
                state: gpu_state(util),
                sources: a.sources.into_iter().collect(),
            }
        })
        .collect();
    out.sort_by(|a, b| a.host.cmp(&b.host).then(a.index.as_deref().unwrap_or("").cmp(b.index.as_deref().unwrap_or(""))).then(a.id.cmp(&b.id)));
    out
}

/// Facet values of a device: `cluster`, `host`, `model`, `vendor`, `state`, `kube_namespace`.
fn device_tags(d: &GpuDevice) -> HashMap<String, String> {
    let mut tags = HashMap::from([("host".to_string(), d.host.clone()), ("state".to_string(), enum_str(&d.state))]);
    for (k, v) in [("cluster", &d.cluster), ("model", &d.model), ("vendor", &d.vendor)] {
        if let Some(v) = v {
            tags.insert(k.to_string(), v.clone());
        }
    }
    if let Some(ns) = d.workloads.iter().find_map(|w| w.split_once('/').map(|(n, _)| n.to_string())) {
        tags.insert("kube_namespace".into(), ns);
    }
    tags
}

fn enum_str<T: Serialize>(v: &T) -> String {
    serde_json::to_value(v).ok().and_then(|v| v.as_str().map(str::to_string)).unwrap_or_default()
}

fn device_matches(d: &GpuDevice, terms: &[Term]) -> bool {
    let tags = device_tags(d);
    let empty = HashMap::new();
    let text = format!("{} {} {}", d.id, d.model.as_deref().unwrap_or(""), d.workloads.join(" "));
    matches(terms, &Fields { service: "", status: "", host: &d.host, source: "", message: &text, tags: &tags, attr: &empty, index: None })
}

const FACET_KEYS: &[&str] = &["cluster", "host", "model", "vendor", "state", "kube_namespace"];

#[derive(Serialize, ToSchema)]
pub struct GpuFacetValue {
    pub value: String,
    pub count: usize,
}

#[derive(Serialize, ToSchema)]
pub struct GpuFacet {
    pub key: String,
    pub values: Vec<GpuFacetValue>,
}

fn facets(devices: &[GpuDevice]) -> Vec<GpuFacet> {
    FACET_KEYS
        .iter()
        .map(|k| {
            let mut counts: BTreeMap<String, usize> = BTreeMap::new();
            for d in devices {
                if let Some(v) = device_tags(d).remove(*k) {
                    *counts.entry(v).or_default() += 1;
                }
            }
            let mut values: Vec<GpuFacetValue> = counts.into_iter().map(|(value, count)| GpuFacetValue { value, count }).collect();
            values.sort_by(|a, b| b.count.cmp(&a.count).then(a.value.cmp(&b.value)));
            GpuFacet { key: k.to_string(), values }
        })
        .filter(|f| !f.values.is_empty())
        .collect()
}

#[derive(Serialize, ToSchema, Default, PartialEq, Debug)]
#[serde(rename_all = "camelCase")]
pub struct GpuSummary {
    pub gpus: usize,
    pub hosts: usize,
    /// Average over the GPUs that report utilization.
    pub avg_util_pct: Option<f64>,
    pub mem_used_bytes: f64,
    pub mem_total_bytes: f64,
    pub idle: usize,
    pub underutilized: usize,
}

fn summarize(devices: &[GpuDevice]) -> GpuSummary {
    let utils: Vec<f64> = devices.iter().filter_map(|d| d.util_pct).collect();
    GpuSummary {
        gpus: devices.len(),
        hosts: devices.iter().map(|d| d.host.as_str()).collect::<BTreeSet<_>>().len(),
        avg_util_pct: (!utils.is_empty()).then(|| round1(utils.iter().sum::<f64>() / utils.len() as f64)),
        mem_used_bytes: devices.iter().filter_map(|d| d.mem_used_bytes).fold(0.0, |a, b| a + b),
        mem_total_bytes: devices.iter().filter_map(|d| d.mem_total_bytes).fold(0.0, |a, b| a + b),
        idle: devices.iter().filter(|d| d.state == GpuState::Idle).count(),
        underutilized: devices.iter().filter(|d| d.state == GpuState::Underutilized).count(),
    }
}

#[derive(Deserialize, ToSchema)]
pub struct GpuSearchRequest {
    pub range: TimeRange,
    /// `cluster:x host:y model:z state:idle kube_namespace:n` + free text on the device, model and workloads.
    #[serde(default)]
    pub query: String,
}

#[derive(Serialize, ToSchema)]
pub struct GpuSearchPage {
    /// False when no GPU metrics arrived at all in the range (show the setup guide).
    pub enabled: bool,
    pub summary: GpuSummary,
    pub devices: Vec<GpuDevice>,
    pub facets: Vec<GpuFacet>,
}

/// GPUs that reported in the last minutes of the range, with their current values.
#[utoipa::path(post, path = "/api/v1/gpu/search", operation_id = "search_gpus", request_body = GpuSearchRequest, responses((status = 200, body = GpuSearchPage)), tag = "infrastructure")]
pub async fn search(State(st): State<AppState>, OrgId(org): OrgId, Json(req): Json<GpuSearchRequest>) -> ApiResult<GpuSearchPage> {
    req.range.validate()?;
    let from = req.range.from.max(req.range.to - CURRENT_MS);
    let values = series_values(&st, &org, &metric_names(), from, req.range.to, None).await?;
    let all = aggregate_gpus(&values);
    let terms = parse_query(&req.query);
    let devices: Vec<GpuDevice> = all.iter().filter(|d| device_matches(d, &terms)).cloned().collect();
    Ok(Json(GpuSearchPage { enabled: !all.is_empty(), summary: summarize(&devices), facets: facets(&all), devices }))
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct GpuTimeseriesRequest {
    pub range: TimeRange,
    #[serde(default)]
    pub query: String,
    /// `host`, `cluster`, `model`, `vendor`, `state` or `kube_namespace`: one series per value; none for the whole fleet.
    pub group_by: Option<String>,
}

#[derive(Serialize, ToSchema, Debug, PartialEq)]
pub struct GpuSeries {
    /// utilization (average, %), memory_used (sum, bytes), temperature (average, °C) or power (sum, W).
    pub metric: String,
    /// The group's value; null for the whole fleet.
    pub group: Option<String>,
    pub unit: String,
    /// `[timestamp ms, value]` pairs.
    pub points: Vec<(i64, f64)>,
}

/// Most groups charted; the rest are left out (the list shows every GPU).
const MAX_GROUPS: usize = 20;

const SERIES: [(&str, &str); 4] = [("utilization", "percent"), ("memory_used", "byte"), ("temperature", "degree celsius"), ("power", "watt")];

/// Points per (series index in [`SERIES`], group).
type SeriesPoints = BTreeMap<(usize, Option<String>), Vec<(i64, f64)>>;

/// Series per interval and group: utilization and temperature average, memory and power add up.
pub(crate) fn fleet_series(values: &[SeriesValue], terms: &[Term], group_by: Option<&str>) -> Vec<GpuSeries> {
    let mut by_t: BTreeMap<i64, Vec<SeriesValue>> = BTreeMap::new();
    for v in values {
        by_t.entry(v.t).or_default().push(v.clone());
    }
    let mut out: SeriesPoints = BTreeMap::new();
    let mut groups: BTreeSet<String> = BTreeSet::new();
    for (t, vs) in by_t {
        let mut grouped: BTreeMap<Option<String>, Vec<GpuDevice>> = BTreeMap::new();
        for d in aggregate_gpus(&vs).into_iter().filter(|d| device_matches(d, terms)) {
            let key = group_by.map(|g| device_tags(&d).remove(g).unwrap_or_else(|| "N/A".into()));
            if let Some(k) = &key {
                if !groups.contains(k) && groups.len() >= MAX_GROUPS {
                    continue;
                }
                groups.insert(k.clone());
            }
            grouped.entry(key).or_default().push(d);
        }
        for (key, devices) in grouped {
            let pick: [Vec<f64>; 4] = [
                devices.iter().filter_map(|d| d.util_pct).collect(),
                devices.iter().filter_map(|d| d.mem_used_bytes).collect(),
                devices.iter().filter_map(|d| d.temp_c).collect(),
                devices.iter().filter_map(|d| d.power_w).collect(),
            ];
            for (i, xs) in pick.into_iter().enumerate() {
                if xs.is_empty() {
                    continue;
                }
                let total: f64 = xs.iter().sum();
                let v = match i {
                    0 | 2 => round1(total / xs.len() as f64),
                    3 => round1(total),
                    _ => total,
                };
                out.entry((i, key.clone())).or_default().push((t, v));
            }
        }
    }
    let mut series: Vec<GpuSeries> = out.into_iter().map(|((i, group), points)| GpuSeries { metric: SERIES[i].0.into(), group, unit: SERIES[i].1.into(), points }).collect();
    // The whole fleet always has its four series, empty when nothing matched.
    if group_by.is_none() {
        for (name, unit) in SERIES {
            if !series.iter().any(|s| s.metric == name) {
                series.push(GpuSeries { metric: name.into(), group: None, unit: unit.into(), points: vec![] });
            }
        }
        series.sort_by_key(|s| SERIES.iter().position(|(n, _)| *n == s.metric));
    }
    series
}

/// Utilization, memory, temperature and power over the range, for the GPUs matching the query, per group.
#[utoipa::path(post, path = "/api/v1/gpu/timeseries", operation_id = "gpu_timeseries", request_body = GpuTimeseriesRequest, responses((status = 200, body = Vec<GpuSeries>)), tag = "infrastructure")]
pub async fn timeseries(State(st): State<AppState>, OrgId(org): OrgId, Json(req): Json<GpuTimeseriesRequest>) -> ApiResult<Vec<GpuSeries>> {
    req.range.validate()?;
    let step = chart_step(req.range.to - req.range.from, 60);
    let values = series_values(&st, &org, &metric_names(), req.range.from, req.range.to, Some(step)).await?;
    Ok(Json(fleet_series(&values, &parse_query(&req.query), req.group_by.as_deref().filter(|g| !g.is_empty()))))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sv(metric: &str, value: f64, res: &[(&str, &str)], attrs: &[(&str, &str)]) -> SeriesValue {
        let map = |kv: &[(&str, &str)]| kv.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        SeriesValue { metric: metric.into(), res: map(res), attrs: map(attrs), t: 0, value, max: value, count: 1, last: 0 }
    }

    const HOST: (&str, &str) = ("host.name", "i-0a1b2c3d4e5f60001");

    #[test]
    fn datadog_gpu_check_metrics_become_a_device() {
        let tags = [("gpu_uuid", "GPU-3f2a"), ("gpu_device", "NVIDIA A10G"), ("gpu_vendor", "nvidia")];
        let res = [HOST, ("k8s.cluster.name", "shop-ml")];
        let devices = aggregate_gpus(&[
            sv("gpu.gr_engine_active", 72.0, &res, &tags),
            sv("gpu.memory.limit", 24.0 * 1024.0 * MIB, &res, &tags),
            sv("gpu.memory.free", 8.0 * 1024.0 * MIB, &res, &tags),
            sv("gpu.temperature", 64.0, &res, &tags),
            sv("gpu.power.usage", 150_000.0, &res, &tags),
            sv("gpu.unrelated", 1.0, &res, &tags),
        ]);
        assert_eq!(devices.len(), 1);
        let d = &devices[0];
        assert_eq!(d.id, "i-0a1b2c3d4e5f60001/gpu-3f2a");
        assert_eq!(d.model.as_deref(), Some("NVIDIA A10G"));
        assert_eq!(d.cluster.as_deref(), Some("shop-ml"));
        assert_eq!(d.util_pct, Some(72.0));
        assert_eq!(d.mem_total_bytes, Some(24.0 * 1024.0 * MIB));
        assert_eq!(d.mem_used_bytes, Some(16.0 * 1024.0 * MIB), "used = limit - free");
        assert_eq!(d.power_w, Some(150.0), "milliwatts to watts");
        assert_eq!(d.state, GpuState::Active);
        assert_eq!(d.sources, vec![GpuSource::DatadogGpu]);
    }

    #[test]
    fn dcgm_exporter_labels_and_units() {
        let gpu = |i: &str, uuid: &str| {
            vec![
                ("gpu", i.to_string()),
                ("UUID", uuid.to_string()),
                ("modelName", "NVIDIA L4".to_string()),
                ("Hostname", "i-0a1b2c3d4e5f60002".to_string()),
                ("namespace", "ml".to_string()),
                ("pod", "recs-trainer-0".to_string()),
            ]
        };
        let a = gpu("0", "GPU-aaaa");
        let b = gpu("1", "GPU-bbbb");
        let attrs = |v: &Vec<(&str, String)>| v.iter().map(|(k, x)| (k.to_string(), x.clone())).collect::<Vec<_>>();
        let mk = |m: &str, val: f64, v: &Vec<(&str, String)>| {
            let owned = attrs(v);
            let refs: Vec<(&str, &str)> = owned.iter().map(|(k, x)| (k.as_str(), x.as_str())).collect();
            sv(m, val, &[], &refs)
        };
        let devices = aggregate_gpus(&[
            mk("DCGM_FI_DEV_GPU_UTIL", 2.0, &a),
            mk("DCGM_FI_DEV_FB_USED", 1024.0, &a),
            mk("DCGM_FI_DEV_FB_FREE", 3072.0, &a),
            mk("DCGM_FI_DEV_GPU_UTIL", 18.0, &b),
            mk("DCGM_FI_DEV_POWER_USAGE", 41.26, &b),
        ]);
        assert_eq!(devices.len(), 2);
        assert_eq!(devices[0].host, "i-0a1b2c3d4e5f60002", "the exporter's Hostname label");
        assert_eq!(devices[0].index.as_deref(), Some("0"));
        assert_eq!(devices[0].mem_used_bytes, Some(1024.0 * MIB), "MiB to bytes");
        assert_eq!(devices[0].mem_total_bytes, Some(4096.0 * MIB), "total = used + free");
        assert_eq!(devices[0].state, GpuState::Idle);
        assert_eq!(devices[0].workloads, vec!["ml/recs-trainer-0"]);
        assert_eq!(devices[1].state, GpuState::Underutilized);
        assert_eq!(devices[1].power_w, Some(41.3));
        assert_eq!(devices[1].sources, vec![GpuSource::DcgmExporter]);
    }

    #[test]
    fn sources_for_one_device_merge_by_uuid() {
        let res = [HOST];
        let devices = aggregate_gpus(&[
            sv("gpu.gr_engine_active", 50.0, &res, &[("gpu_uuid", "gpu-77")]),
            sv("dcgm.gpu_utilization", 60.0, &res, &[("UUID", "GPU-77")]),
            sv("dcgm.temperature", 70.0, &res, &[("UUID", "GPU-77")]),
        ]);
        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].util_pct, Some(55.0));
        assert_eq!(devices[0].sources, vec![GpuSource::DatadogGpu, GpuSource::DatadogDcgm]);
    }

    #[test]
    fn states_from_utilization() {
        assert_eq!(gpu_state(None), GpuState::NoData);
        assert_eq!(gpu_state(Some(0.0)), GpuState::Idle);
        assert_eq!(gpu_state(Some(29.9)), GpuState::Underutilized);
        assert_eq!(gpu_state(Some(30.0)), GpuState::Active);
    }

    #[test]
    fn summary_facets_and_query() {
        let empty = summarize(&[]);
        assert!(empty.mem_used_bytes.is_sign_positive(), "an empty fleet uses 0 bytes, not -0");
        assert_eq!(empty.avg_util_pct, None);
        let res1 = [("host.name", "i-0a1b2c3d4e5f60001"), ("k8s.cluster.name", "shop-ml")];
        let res2 = [("host.name", "i-0a1b2c3d4e5f60002")];
        let devices = aggregate_gpus(&[
            sv("gpu.gr_engine_active", 0.0, &res1, &[("gpu_uuid", "GPU-1"), ("gpu_device", "NVIDIA A10G")]),
            sv("gpu.gr_engine_active", 90.0, &res1, &[("gpu_uuid", "GPU-2"), ("gpu_device", "NVIDIA A10G")]),
            sv("gpu.gr_engine_active", 20.0, &res2, &[("gpu_uuid", "GPU-3"), ("gpu_device", "NVIDIA L4")]),
        ]);
        let s = summarize(&devices);
        assert_eq!((s.gpus, s.hosts, s.idle, s.underutilized), (3, 2, 1, 1));
        assert_eq!(s.avg_util_pct, Some(36.7));
        let f = facets(&devices);
        let model = f.iter().find(|f| f.key == "model").unwrap();
        assert_eq!((model.values[0].value.as_str(), model.values[0].count), ("NVIDIA A10G", 2));
        let idle: Vec<_> = devices.iter().filter(|d| device_matches(d, &parse_query("state:idle cluster:shop-ml"))).collect();
        assert_eq!(idle.len(), 1);
        assert!(devices.iter().filter(|d| device_matches(d, &parse_query("L4"))).count() == 1, "free text on the model");
    }

    #[test]
    fn fleet_series_per_interval_and_group() {
        let at = |t: i64, m: &str, v: f64, host: &str, uuid: &str| SeriesValue { t, ..sv(m, v, &[("host.name", host)], &[("gpu_uuid", uuid)]) };
        let values = [
            at(0, "gpu.gr_engine_active", 10.0, "i-0a1b2c3d4e5f60001", "a"),
            at(0, "gpu.gr_engine_active", 30.0, "i-0a1b2c3d4e5f60002", "b"),
            at(60_000, "gpu.gr_engine_active", 50.0, "i-0a1b2c3d4e5f60001", "a"),
            at(60_000, "gpu.power.usage", 100_000.0, "i-0a1b2c3d4e5f60001", "a"),
            at(60_000, "gpu.power.usage", 50_000.0, "i-0a1b2c3d4e5f60002", "b"),
        ];
        let s = fleet_series(&values, &[], None);
        assert_eq!(s.iter().map(|s| s.metric.as_str()).collect::<Vec<_>>(), ["utilization", "memory_used", "temperature", "power"]);
        assert_eq!(s[0].points, vec![(0, 20.0), (60_000, 50.0)]);
        assert!(s[1].points.is_empty());
        assert_eq!(s[3].points, vec![(60_000, 150.0)], "power adds up");
        let by_host = fleet_series(&values, &[], Some("host"));
        let util: Vec<_> = by_host.iter().filter(|s| s.metric == "utilization").collect();
        assert_eq!(util.len(), 2);
        assert_eq!(util[0].group.as_deref(), Some("i-0a1b2c3d4e5f60001"));
        assert_eq!(util[0].points, vec![(0, 10.0), (60_000, 50.0)]);
        assert!(fleet_series(&values, &parse_query("host:nothing"), None).iter().all(|s| s.points.is_empty()));
    }
}
