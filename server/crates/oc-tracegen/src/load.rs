//! Multi-org metrics load profile (`oc-tracegen load`), for measuring storage (docs/adr/0002).
//!
//! Each org is a fictional Kubernetes cluster whose pods report container metrics the way the
//! Datadog Agent does: `/api/v2/series` JSON (zlib, like the Agent), one point per series per flush
//! interval, spread over the interval as many Agents would. A share of the pods (`otlp_share`) sends
//! OTLP/HTTP JSON instead, so both intake paths are exercised. Every `churn_minutes`, a fraction of
//! the pods is replaced by pods with new names, so series come and go as in a real cluster.
//!
//! Everything is a function of the seed, the org and the time: restarts and tests agree.
//!
//! Each org needs its own ingest key: `scripts/load-orgs.sql` creates `load-org-1..N` with the keys
//! `ocik_load_org_<n>_local_only`, the default `OC_LOAD_KEYS`.
//!
//! Env: OTLP_ENDPOINT (intake, http://localhost:4318), OC_LOAD_ORGS (4), OC_LOAD_KEYS (comma
//! separated, one per org), OC_LOAD_LARGE_SERIES (100000, the first org), OC_LOAD_SMALL_SERIES
//! (5000, every other org), OC_LOAD_LABELS (20), OC_LOAD_FLUSH_SECS (15), OC_LOAD_CHURN_FRACTION
//! (0.05), OC_LOAD_CHURN_MINUTES (10), OC_LOAD_OTLP_SHARE (0.1), OC_LOAD_SEED (1).

use std::io::Write;
use std::time::Duration;

use serde_json::{json, Value};

/// Metrics every container reports: Datadog name and type (`/api/v2/series`: 1 count, 3 gauge).
const METRICS: &[(&str, i32)] = &[
    ("kubernetes.cpu.usage.total", 3),
    ("kubernetes.cpu.requests", 3),
    ("kubernetes.cpu.limits", 3),
    ("kubernetes.memory.usage", 3),
    ("kubernetes.memory.working_set", 3),
    ("kubernetes.memory.rss", 3),
    ("kubernetes.memory.requests", 3),
    ("kubernetes.memory.limits", 3),
    ("kubernetes.network.rx_bytes", 3),
    ("kubernetes.network.tx_bytes", 3),
    ("kubernetes.filesystem.usage", 3),
    ("kubernetes.containers.restarts", 3),
    ("kubernetes.containers.running", 3),
    ("container.cpu.usage", 3),
    ("container.cpu.user", 3),
    ("container.cpu.system", 3),
    ("container.cpu.throttled", 3),
    ("container.memory.usage", 3),
    ("container.memory.rss", 3),
    ("container.memory.cache", 3),
    ("container.io.read_operations", 1),
    ("container.io.write_operations", 1),
    ("container.net.sent", 3),
    ("container.net.rcvd", 3),
    ("container.pid.thread_count", 3),
];

/// Tag keys in the order they are added (`labels` takes a prefix, then pads with `kube_label_*`).
/// The pod name comes first so that churn always creates new series.
const TAG_KEYS: &[&str] = &[
    "pod_name",
    "kube_namespace",
    "kube_cluster_name",
    "container_name",
    "kube_deployment",
    "kube_replica_set",
    "kube_container_name",
    "container_id",
    "image_name",
    "image_tag",
    "short_image",
    "node",
    "kube_service",
    "kube_ownerref_kind",
    "kube_ownerref_name",
    "pod_phase",
    "kube_qos",
    "env",
    "service",
    "version",
    "team",
    "availability-zone",
    "kube_app_name",
    "kube_app_instance",
];

/// OpenTelemetry resource attribute names for the OTLP share.
const OTEL_KEYS: &[(&str, &str)] = &[
    ("pod_name", "k8s.pod.name"),
    ("kube_namespace", "k8s.namespace.name"),
    ("kube_cluster_name", "k8s.cluster.name"),
    ("container_name", "k8s.container.name"),
    ("kube_deployment", "k8s.deployment.name"),
    ("kube_replica_set", "k8s.replicaset.name"),
    ("container_id", "container.id"),
    ("image_name", "container.image.name"),
    ("image_tag", "container.image.tag"),
    ("node", "k8s.node.name"),
    ("env", "deployment.environment"),
    ("service", "service.name"),
    ("version", "service.version"),
    ("availability-zone", "cloud.availability_zone"),
];

const SERVICES: &[&str] = &[
    "api-gateway",
    "storefront-api",
    "order-service",
    "billing",
    "catalog-service",
    "inventory-service",
    "notification-worker",
    "workflow-engine",
    "payment-service",
    "search-service",
    "cart-service",
    "pricing-service",
    "shipping-service",
    "review-service",
    "recommendation-service",
    "user-service",
];
const NAMESPACES: &[&str] = &["storefront", "checkout", "billing", "catalog", "inventory", "notifications", "search", "shipping", "platform", "analytics"];
const TEAMS: &[&str] = &["storefront", "checkout", "payments", "catalog", "platform"];
const ZONES: &[&str] = &["us-east-1a", "us-east-1b", "us-east-1c"];
/// Pods per deployment and per node.
const REPLICAS: usize = 8;
const PODS_PER_NODE: usize = 30;
/// Series per `/api/v2/series` request and pods per OTLP request.
const SERIES_PER_REQUEST: usize = 2_000;
const PODS_PER_OTLP_REQUEST: usize = 80;

#[derive(Clone, Debug)]
pub struct Profile {
    /// Ingest key and series count of each org.
    pub orgs: Vec<(String, usize)>,
    pub labels: usize,
    pub flush_secs: u64,
    pub churn_fraction: f64,
    pub churn_minutes: u64,
    pub otlp_share: f64,
    pub seed: u64,
}

impl Profile {
    pub fn from_env() -> Profile {
        let var = |k: &str| std::env::var(k).ok().filter(|v| !v.trim().is_empty());
        let num = |k: &str, default: f64| var(k).and_then(|v| v.trim().parse::<f64>().ok()).unwrap_or(default);
        let count = num("OC_LOAD_ORGS", 4.0).max(1.0) as usize;
        let keys: Vec<String> = match var("OC_LOAD_KEYS") {
            Some(list) => list.split(',').map(|k| k.trim().to_string()).filter(|k| !k.is_empty()).collect(),
            None => (1..=count).map(default_key).collect(),
        };
        let large = num("OC_LOAD_LARGE_SERIES", 100_000.0) as usize;
        let small = num("OC_LOAD_SMALL_SERIES", 5_000.0) as usize;
        Profile {
            orgs: keys.into_iter().enumerate().map(|(i, key)| (key, if i == 0 { large } else { small })).collect(),
            labels: num("OC_LOAD_LABELS", 20.0).max(1.0) as usize,
            flush_secs: num("OC_LOAD_FLUSH_SECS", 15.0).max(1.0) as u64,
            churn_fraction: num("OC_LOAD_CHURN_FRACTION", 0.05).clamp(0.0, 1.0),
            churn_minutes: num("OC_LOAD_CHURN_MINUTES", 10.0).max(1.0) as u64,
            otlp_share: num("OC_LOAD_OTLP_SHARE", 0.1).clamp(0.0, 1.0),
            seed: num("OC_LOAD_SEED", 1.0) as u64,
        }
    }

    /// Pods of an org (each container reports every metric; the last pod fewer, so the org has
    /// exactly its series count).
    pub fn pods(&self, org: usize) -> usize {
        self.orgs[org].1.div_ceil(METRICS.len())
    }

    fn metrics_of(&self, org: usize, slot: usize) -> usize {
        (self.orgs[org].1 - slot * METRICS.len()).min(METRICS.len())
    }

    /// Pods replaced per churn period.
    fn replaced(&self, org: usize) -> usize {
        (self.pods(org) as f64 * self.churn_fraction).round() as usize
    }

    /// How many times the pod in `slot` has been replaced at churn epoch `epoch`. Each epoch replaces
    /// the next `replaced` slots, wrapping around (a rolling restart through the cluster).
    fn generation(&self, org: usize, slot: usize, epoch: u64) -> u64 {
        let pods = self.pods(org) as u64;
        (epoch * self.replaced(org) as u64 + pods - 1 - slot as u64) / pods
    }

    fn epoch(&self, now_s: u64) -> u64 {
        now_s / (self.churn_minutes * 60)
    }

    /// The second of the flush interval in which a pod reports.
    fn flush_slot(&self, org: usize, slot: usize) -> u64 {
        mix(&[self.seed, org as u64, slot as u64, 0xf1]) % self.flush_secs
    }

    fn sends_otlp(&self, org: usize, slot: usize) -> bool {
        unit(mix(&[self.seed, org as u64, slot as u64, 0x07])) < self.otlp_share
    }

    /// The pod in `slot` at churn epoch `epoch`.
    pub fn pod(&self, org: usize, slot: usize, epoch: u64) -> Pod {
        let gen = self.generation(org, slot, epoch);
        let deployment_index = slot / REPLICAS;
        let service = SERVICES[deployment_index % SERVICES.len()];
        let deployment = if deployment_index < SERVICES.len() { service.to_string() } else { format!("{service}-{:02}", deployment_index / SERVICES.len()) };
        let namespace = NAMESPACES[deployment_index % NAMESPACES.len()];
        let cluster = format!("shop-{}-eks", ["prod", "eu", "apac", "staging", "dev", "qa"][org % 6]);
        let replica_set = format!("{deployment}-{}", &hex(mix(&[self.seed, org as u64, deployment_index as u64]))[..10]);
        let pod_hash = mix(&[self.seed, org as u64, slot as u64, gen]);
        let pod_name = format!("{replica_set}-{}", &hex(pod_hash)[..5]);
        let node_index = (org * 4096 + slot / PODS_PER_NODE) as u64;
        let node = format!("i-0a1b2c3d4e5f{node_index:05x}");
        let version = format!("{}.{}.{}", 1 + deployment_index % 4, deployment_index % 13, deployment_index % 7);
        let container_id = format!("{}{}{}{}", hex(pod_hash), hex(mix(&[pod_hash, 1])), hex(mix(&[pod_hash, 2])), hex(mix(&[pod_hash, 3])));
        let values: Vec<String> = TAG_KEYS
            .iter()
            .map(|key| match *key {
                "pod_name" => pod_name.clone(),
                "kube_namespace" => namespace.to_string(),
                "kube_cluster_name" => cluster.clone(),
                "container_name" | "kube_container_name" | "short_image" | "kube_service" | "service" | "kube_app_name" => service.to_string(),
                "kube_deployment" => deployment.clone(),
                "kube_replica_set" | "kube_ownerref_name" => replica_set.clone(),
                "container_id" => container_id.clone(),
                "image_name" => format!("registry.example.com/shop/{service}"),
                "image_tag" | "version" => version.clone(),
                "node" => node.clone(),
                "kube_ownerref_kind" => "replicaset".into(),
                "pod_phase" => "running".into(),
                "kube_qos" => if deployment_index.is_multiple_of(3) { "guaranteed" } else { "burstable" }.into(),
                "env" => if org % 6 == 3 { "staging" } else { "prod" }.into(),
                "team" => TEAMS[deployment_index % TEAMS.len()].into(),
                "availability-zone" => ZONES[slot % ZONES.len()].into(),
                "kube_app_instance" => deployment.clone(),
                other => unreachable!("tag {other} has no value"),
            })
            .collect();
        let mut tags: Vec<(String, String)> = TAG_KEYS.iter().zip(values).take(self.labels).map(|(k, v)| (k.to_string(), v)).collect();
        for extra in TAG_KEYS.len()..self.labels {
            tags.push((format!("kube_label_{}", extra - TAG_KEYS.len() + 1), format!("value-{}", deployment_index % 10)));
        }
        Pod { slot, host: node, tags, metrics: self.metrics_of(org, slot), otlp: self.sends_otlp(org, slot) }
    }

    /// Every pod of an org at `now_s`.
    #[cfg(test)]
    pub fn pods_at(&self, org: usize, now_s: u64) -> Vec<Pod> {
        let epoch = self.epoch(now_s);
        (0..self.pods(org)).map(|slot| self.pod(org, slot, epoch)).collect()
    }

    /// The pods of an org that report in second `now_s`.
    pub fn due(&self, org: usize, now_s: u64) -> Vec<Pod> {
        let epoch = self.epoch(now_s);
        let second = now_s % self.flush_secs;
        (0..self.pods(org)).filter(|&slot| self.flush_slot(org, slot) == second).map(|slot| self.pod(org, slot, epoch)).collect()
    }

    fn value(&self, pod: &Pod, metric: usize, now_s: u64) -> f64 {
        let base = 1.0 + (mix(&[self.seed, pod.slot as u64, metric as u64]) % 1000) as f64;
        let wave = 1.0 + 0.2 * ((now_s as f64 / 300.0) + metric as f64).sin();
        let value = base * wave;
        if METRICS[metric].1 == 1 {
            (value * self.flush_secs as f64).round()
        } else {
            value
        }
    }

    /// `/api/v2/series` bodies (uncompressed JSON) for the Datadog pods among `pods`.
    pub fn datadog_payloads(&self, pods: &[Pod], now_s: u64) -> Vec<Value> {
        let mut series = Vec::new();
        for pod in pods.iter().filter(|p| !p.otlp) {
            let tags: Vec<String> = pod.tags.iter().map(|(k, v)| format!("{k}:{v}")).collect();
            for (metric, (name, kind)) in METRICS.iter().enumerate().take(pod.metrics) {
                series.push(json!({
                    "metric": name,
                    "type": kind,
                    "interval": if *kind == 1 { self.flush_secs } else { 0 },
                    "points": [{ "timestamp": now_s, "value": self.value(pod, metric, now_s) }],
                    "resources": [{ "type": "host", "name": pod.host }],
                    "tags": tags,
                }));
            }
        }
        series.chunks(SERIES_PER_REQUEST).map(|chunk| json!({ "series": chunk })).collect()
    }

    /// OTLP/HTTP JSON bodies for the OTLP pods among `pods`: one resource per pod, its tags as
    /// resource attributes (OpenTelemetry names where there is one).
    pub fn otlp_payloads(&self, pods: &[Pod], now_s: u64) -> Vec<Value> {
        let now_ns = (now_s * 1_000_000_000).to_string();
        let start_ns = (now_s.saturating_sub(self.flush_secs) * 1_000_000_000).to_string();
        let resources: Vec<Value> = pods
            .iter()
            .filter(|p| p.otlp)
            .map(|pod| {
                let mut attributes: Vec<Value> = vec![json!({ "key": "host.name", "value": { "stringValue": pod.host } })];
                for (k, v) in &pod.tags {
                    let key = OTEL_KEYS.iter().find(|(dd, _)| dd == k).map_or(k.as_str(), |(_, otel)| otel);
                    attributes.push(json!({ "key": key, "value": { "stringValue": v } }));
                }
                let metrics: Vec<Value> = METRICS
                    .iter()
                    .enumerate()
                    .take(pod.metrics)
                    .map(|(metric, (name, kind))| {
                        let value = self.value(pod, metric, now_s);
                        if *kind == 1 {
                            json!({ "name": name, "sum": { "aggregationTemporality": 1, "isMonotonic": true, "dataPoints": [{ "startTimeUnixNano": start_ns, "timeUnixNano": now_ns, "asDouble": value }] } })
                        } else {
                            json!({ "name": name, "gauge": { "dataPoints": [{ "timeUnixNano": now_ns, "asDouble": value }] } })
                        }
                    })
                    .collect();
                json!({ "resource": { "attributes": attributes }, "scopeMetrics": [{ "scope": { "name": "oc-tracegen.load" }, "metrics": metrics }] })
            })
            .collect();
        resources.chunks(PODS_PER_OTLP_REQUEST).map(|chunk| json!({ "resourceMetrics": chunk })).collect()
    }
}

/// One pod (one container) of an org's cluster.
#[derive(Clone, Debug, PartialEq)]
pub struct Pod {
    pub slot: usize,
    pub host: String,
    pub tags: Vec<(String, String)>,
    /// How many of `METRICS` it reports.
    pub metrics: usize,
    pub otlp: bool,
}

/// The ingest key `scripts/load-orgs.sql` creates for `load-org-<n>`.
pub fn default_key(n: usize) -> String {
    format!("ocik_load_org_{n}_local_only")
}

/// SplitMix64 over the parts: a stable hash for names and values.
fn mix(parts: &[u64]) -> u64 {
    let mut h: u64 = 0x9e37_79b9_7f4a_7c15;
    for part in parts {
        h ^= part.wrapping_add(0x9e37_79b9_7f4a_7c15).wrapping_add(h << 6).wrapping_add(h >> 2);
        h = (h ^ (h >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        h = (h ^ (h >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        h ^= h >> 31;
    }
    h
}

fn unit(h: u64) -> f64 {
    (h >> 11) as f64 / (1u64 << 53) as f64
}

fn hex(h: u64) -> String {
    format!("{h:016x}")
}

fn compress(body: &[u8], gzip: bool) -> Vec<u8> {
    if gzip {
        let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        enc.write_all(body).expect("writing to memory");
        enc.finish().expect("writing to memory")
    } else {
        let mut enc = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::fast());
        enc.write_all(body).expect("writing to memory");
        enc.finish().expect("writing to memory")
    }
}

/// Sends the profile until stopped.
pub async fn run() -> anyhow::Result<()> {
    let endpoint = std::env::var("OTLP_ENDPOINT").unwrap_or_else(|_| "http://localhost:4318".into());
    let profile = Profile::from_env();
    let series: usize = profile.orgs.iter().map(|(_, n)| n).sum();
    tracing::info!(
        orgs = profile.orgs.len(),
        series,
        points_per_sec = series as u64 / profile.flush_secs,
        labels = profile.labels,
        flush_secs = profile.flush_secs,
        churn_fraction = profile.churn_fraction,
        churn_minutes = profile.churn_minutes,
        otlp_share = profile.otlp_share,
        "oc-tracegen load profile started"
    );
    let http = reqwest::Client::builder().timeout(Duration::from_secs(30)).build()?;
    let series_url = format!("{endpoint}/api/v2/series");
    let metrics_url = format!("{endpoint}/v1/metrics");
    let mut tick = tokio::time::interval(Duration::from_secs(1));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tick.tick().await;
        let now_s = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_secs();
        let mut sends = tokio::task::JoinSet::new();
        for (org, (key, _)) in profile.orgs.iter().enumerate() {
            let pods = profile.due(org, now_s);
            let bodies = profile
                .datadog_payloads(&pods, now_s)
                .into_iter()
                .map(|b| (series_url.clone(), false, b))
                .chain(profile.otlp_payloads(&pods, now_s).into_iter().map(|b| (metrics_url.clone(), true, b)));
            for (url, otlp, body) in bodies {
                let (http, key) = (http.clone(), key.clone());
                sends.spawn(async move {
                    let raw = serde_json::to_vec(&body).expect("JSON");
                    let mut req = http.post(&url).header("Content-Type", "application/json").header("Content-Encoding", if otlp { "gzip" } else { "deflate" });
                    req = if otlp { req.bearer_auth(&key) } else { req.header("DD-API-KEY", &key) };
                    if let Err(e) = req.body(compress(&raw, otlp)).send().await.and_then(|r| r.error_for_status()) {
                        tracing::warn!(error = %e, %url, "load export failed");
                    }
                });
            }
        }
        while sends.join_next().await.is_some() {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn profile(series: &[usize]) -> Profile {
        Profile { orgs: series.iter().enumerate().map(|(i, n)| (default_key(i + 1), *n)).collect(), labels: 20, flush_secs: 15, churn_fraction: 0.05, churn_minutes: 10, otlp_share: 0.1, seed: 7 }
    }

    /// Every series (metric + host + sorted tags) of an org at `now_s`, from both payload kinds.
    fn series_of(p: &Profile, org: usize, now_s: u64) -> HashSet<String> {
        let pods = p.pods_at(org, now_s);
        let mut out = HashSet::new();
        for body in p.datadog_payloads(&pods, now_s) {
            for s in body["series"].as_array().unwrap() {
                let mut tags: Vec<&str> = s["tags"].as_array().unwrap().iter().map(|t| t.as_str().unwrap()).collect();
                tags.sort();
                out.insert(format!("{}|{}|{}", s["metric"], s["resources"][0]["name"], tags.join(",")));
            }
        }
        for body in p.otlp_payloads(&pods, now_s) {
            for r in body["resourceMetrics"].as_array().unwrap() {
                let mut attrs: Vec<String> = r["resource"]["attributes"].as_array().unwrap().iter().map(|a| format!("{}={}", a["key"], a["value"]["stringValue"])).collect();
                attrs.sort();
                for m in r["scopeMetrics"][0]["metrics"].as_array().unwrap() {
                    out.insert(format!("{}|{}", m["name"], attrs.join(",")));
                }
            }
        }
        out
    }

    #[test]
    fn each_org_has_its_configured_series() {
        let p = profile(&[100_003, 5_000, 1]);
        let now = 1_800_000_000;
        assert_eq!(series_of(&p, 0, now).len(), 100_003);
        assert_eq!(series_of(&p, 1, now).len(), 5_000);
        assert_eq!(series_of(&p, 2, now).len(), 1);
    }

    #[test]
    fn every_series_reports_once_per_flush_interval() {
        let p = profile(&[20_000]);
        let start = 1_800_000_010;
        let mut sent = Vec::new();
        for now in start..start + p.flush_secs {
            let pods = p.due(0, now);
            let points: usize = p.datadog_payloads(&pods, now).iter().map(|b| b["series"].as_array().unwrap().len()).sum::<usize>()
                + p.otlp_payloads(&pods, now).iter().flat_map(|b| b["resourceMetrics"].as_array().unwrap().clone()).map(|r| r["scopeMetrics"][0]["metrics"].as_array().unwrap().len()).sum::<usize>();
            sent.push(points);
            assert!(p.datadog_payloads(&pods, now).iter().all(|b| b["series"].as_array().unwrap().len() <= SERIES_PER_REQUEST));
        }
        assert_eq!(sent.iter().sum::<usize>(), 20_000);
        // Spread over the interval, not one burst.
        assert!(sent.iter().all(|&n| n > 20_000 / 15 / 2 && n < 20_000 / 15 * 2), "{sent:?}");
    }

    #[test]
    fn labels_are_kubernetes_shaped() {
        let p = profile(&[1_000]);
        let pods = p.pods_at(0, 1_800_000_000);
        for pod in &pods {
            assert_eq!(pod.tags.len(), 20);
            let keys: HashSet<&str> = pod.tags.iter().map(|(k, _)| k.as_str()).collect();
            assert_eq!(keys.len(), 20, "keys are unique");
            for k in ["kube_cluster_name", "kube_namespace", "pod_name", "container_name", "kube_deployment", "node", "image_tag", "container_id"] {
                assert!(keys.contains(k), "{k}");
            }
            assert!(pod.host.starts_with("i-0a1b2c3d4e5f") && pod.host.len() == 19, "{}", pod.host);
            let tag = |k: &str| pod.tags.iter().find(|(key, _)| key == k).unwrap().1.clone();
            assert!(tag("pod_name").starts_with(&tag("kube_replica_set")));
            assert_eq!(tag("container_id").len(), 64);
        }
        let wide = Profile { labels: 30, ..p.clone() }.pod(0, 3, 0);
        assert_eq!(wide.tags.len(), 30);
        assert_eq!(wide.tags.last().unwrap().0, "kube_label_6");
        let narrow = Profile { labels: 3, ..p }.pod(0, 3, 0);
        assert_eq!(narrow.tags.iter().map(|(k, _)| k.as_str()).collect::<Vec<_>>(), ["pod_name", "kube_namespace", "kube_cluster_name"]);
    }

    #[test]
    fn churn_replaces_a_fraction_of_pods_each_period() {
        let p = profile(&[25_000]); // 1000 pods, 50 replaced per 10 minutes
        let period = p.churn_minutes * 60;
        let start = 1_800_000_000 / period * period;
        let before = p.pods_at(0, start);
        let after = p.pods_at(0, start + period);
        let name = |pod: &Pod| pod.tags[0].1.clone();
        let changed = before.iter().zip(&after).filter(|(a, b)| name(a) != name(b)).count();
        assert_eq!(changed, 50);
        assert_eq!(p.pods_at(0, start + period - 1), before, "stable within a period");
        // Active series stay constant; over both periods the old pods' series add up.
        let (old, new) = (series_of(&p, 0, start), series_of(&p, 0, start + period));
        assert_eq!(new.len(), 25_000);
        assert_eq!(old.union(&new).count(), 25_000 + 50 * METRICS.len());
        // After a full rotation every pod has been replaced once.
        let rotated = p.pods_at(0, start + 20 * period);
        assert!(before.iter().zip(&rotated).all(|(a, b)| name(a) != name(b)));
    }

    #[test]
    fn deterministic_for_a_seed() {
        let p = profile(&[2_000]);
        let now = 1_800_000_123;
        assert_eq!(p.datadog_payloads(&p.pods_at(0, now), now), p.datadog_payloads(&p.pods_at(0, now), now));
        let other = Profile { seed: 8, ..p.clone() };
        assert_ne!(p.pod(0, 0, 0).tags[0], other.pod(0, 0, 0).tags[0]);
    }

    #[test]
    fn a_share_of_pods_sends_otlp() {
        let p = profile(&[50_000]);
        let pods = p.pods_at(0, 1_800_000_000);
        let otlp = pods.iter().filter(|p| p.otlp).count() as f64 / pods.len() as f64;
        assert!((otlp - 0.1).abs() < 0.03, "{otlp}");
        let none = Profile { otlp_share: 0.0, ..p.clone() };
        assert!(none.otlp_payloads(&none.pods_at(0, 0), 0).is_empty());
        // OTLP resources use OpenTelemetry names and carry counts as delta sums.
        let body = &p.otlp_payloads(&pods, 1_800_000_000)[0];
        let resource = &body["resourceMetrics"][0];
        let keys: Vec<&str> = resource["resource"]["attributes"].as_array().unwrap().iter().map(|a| a["key"].as_str().unwrap()).collect();
        assert!(keys.contains(&"k8s.pod.name") && keys.contains(&"k8s.cluster.name") && keys.contains(&"host.name"));
        let metrics = resource["scopeMetrics"][0]["metrics"].as_array().unwrap();
        assert!(metrics.iter().any(|m| m["sum"]["aggregationTemporality"] == 1));
    }

    #[test]
    fn datadog_series_carry_types_and_hosts() {
        let p = Profile { otlp_share: 0.0, ..profile(&[100]) };
        let body = &p.datadog_payloads(&p.pods_at(0, 1_800_000_000), 1_800_000_000)[0];
        let series = body["series"].as_array().unwrap();
        assert_eq!(series.len(), 100);
        assert!(series.iter().any(|s| s["type"] == 1 && s["interval"] == 15));
        assert!(series.iter().all(|s| s["resources"][0]["type"] == "host" && s["tags"].as_array().unwrap().len() == 20));
    }
}
