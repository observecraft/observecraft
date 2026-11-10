//! How Cloud Monitoring metrics become ObserveCraft metrics, Datadog-compatible: names
//! (`compute.googleapis.com/instance/cpu/utilization` → `gcp.gce.instance.cpu.utilization`) and the names
//! each kind of metric is published under ([`stats`]), the service account's resource and location filters
//! and metric namespace settings.

use std::collections::BTreeMap;
use std::sync::OnceLock;

use serde::Deserialize;
use serde_json::Value;

use crate::cloud::naming::{glob, underscored};

#[derive(Deserialize)]
struct Catalog {
    services: Vec<Service>,
    #[serde(default)]
    percentiles: Vec<String>,
    #[serde(default)]
    custom_namespaces: Vec<Service>,
}

#[derive(Deserialize)]
struct Service {
    prefix: String,
    short: String,
}

/// Our GCP catalog (`seeds/gcp_integration.json`): metric prefixes and their Datadog names.
fn catalog() -> &'static Catalog {
    static C: OnceLock<Catalog> = OnceLock::new();
    C.get_or_init(|| serde_json::from_str(include_str!("../../../../seeds/gcp_integration.json")).expect("seeds/gcp_integration.json is valid"))
}

fn service(metric_type: &str) -> Option<&'static Service> {
    let c = catalog();
    c.services.iter().chain(&c.custom_namespaces).find(|s| metric_type.starts_with(&s.prefix))
}

/// The service of a metric type (`compute.googleapis.com/…` → `gce`); outside the catalog, its domain
/// without `.googleapis.com` (`custom.googleapis.com` → `custom`).
pub fn short_name(metric_type: &str) -> String {
    if let Some(s) = service(metric_type) {
        return s.short.clone();
    }
    let domain = metric_type.split('/').next().unwrap_or_default();
    underscored(domain.strip_suffix(".googleapis.com").unwrap_or(domain))
}

/// Whether the metric type belongs to a service of the catalog (not a user-defined metric).
pub fn known(metric_type: &str) -> bool {
    catalog().services.iter().any(|s| metric_type.starts_with(&s.prefix))
}

/// `compute.googleapis.com/instance/cpu/utilization` → `gcp.gce.instance.cpu.utilization`.
pub fn metric_name(metric_type: &str) -> String {
    let path = metric_type.split_once('/').map(|(_, p)| p).unwrap_or_default();
    let path: Vec<String> = path.split('/').map(underscored).filter(|p| !p.is_empty()).collect();
    format!("gcp.{}.{}", short_name(metric_type), path.join("."))
}

/// What one published name of a metric carries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stat {
    /// A gauge's or delta's value; a cumulative counter's running total.
    Value,
    /// A cumulative counter's change since its previous point.
    Delta,
    /// A distribution's mean, count, sum of squared deviations and percentiles.
    Avg,
    SampleCount,
    SumSqDev,
    P95,
    P99,
}

impl Stat {
    pub fn suffix(self) -> &'static str {
        match self {
            Stat::Value => "",
            Stat::Delta => ".delta",
            Stat::Avg => ".avg",
            Stat::SampleCount => ".samplecount",
            Stat::SumSqDev => ".sumsqdev",
            Stat::P95 => ".p95",
            Stat::P99 => ".p99",
        }
    }
}

/// The names a metric is published under, as Datadog has them: numbers under the bare name (cumulative
/// counters also `.delta`), distributions as `.avg`, `.samplecount`, `.sumsqdev` and, for those the
/// catalog lists, `.p95` / `.p99`. Strings and money aren't collected.
pub fn stats(metric_type: &str, kind: &str, value_type: &str) -> Vec<Stat> {
    match (kind, value_type) {
        ("CUMULATIVE", "DOUBLE" | "INT64") => vec![Stat::Value, Stat::Delta],
        (_, "DOUBLE" | "INT64" | "BOOL") => vec![Stat::Value],
        (_, "DISTRIBUTION") if catalog().percentiles.iter().any(|p| p == metric_type) => vec![Stat::Avg, Stat::SampleCount, Stat::SumSqDev, Stat::P95, Stat::P99],
        (_, "DISTRIBUTION") => vec![Stat::Avg, Stat::SampleCount, Stat::SumSqDev],
        _ => vec![],
    }
}

fn strings(v: &Value) -> Vec<&str> {
    v.as_array().map(|a| a.iter().filter_map(Value::as_str).collect()).unwrap_or_default()
}

/// Whether the service account collects a monitored resource: its type's `monitored_resource_configs`
/// filters (or the deprecated `host_filters` for instances and `cloud_run_revision_filters` for Cloud Run
/// revisions): one matching tag (`*` wildcards) and none of the `!`-negated ones; no filters, everything.
pub fn resource_selected(config: &Value, resource_type: &str, tags: &[String]) -> bool {
    let mut patterns: Vec<&str> = config["monitored_resource_configs"].as_array().map(|cs| cs.iter().filter(|c| c["type"] == resource_type).flat_map(|c| strings(&c["filters"])).collect()).unwrap_or_default();
    match resource_type {
        "gce_instance" => patterns.extend(strings(&config["host_filters"])),
        "cloud_run_revision" => patterns.extend(strings(&config["cloud_run_revision_filters"])),
        _ => {}
    }
    let matches = |p: &str| tags.iter().any(|t| glob(p, t));
    let (negated, wanted): (Vec<&str>, Vec<&str>) = patterns.into_iter().partition(|p| p.starts_with('!'));
    if negated.iter().any(|p| matches(&p[1..])) {
        return false;
    }
    wanted.is_empty() || wanted.iter().any(|p| matches(p))
}

/// Whether a series' location passes `region_filter_configs` (regions, zones or multi-regions; a region
/// keeps its zones): its zone, region or location label; series without one, or `global`, follow
/// `is_global_location_enabled` (on unless turned off).
pub fn location_selected(config: &Value, labels: &BTreeMap<String, String>) -> bool {
    let location = ["zone", "region", "location"].iter().find_map(|k| labels.get(*k)).map(String::as_str).filter(|l| !l.is_empty() && *l != "global");
    let Some(location) = location else { return config["is_global_location_enabled"] != false };
    let filters = strings(&config["region_filter_configs"]);
    filters.is_empty() || filters.iter().any(|f| location == *f || location.strip_prefix(f).is_some_and(|rest| rest.starts_with('-') && rest[1..].len() == 1))
}

/// Whether a metric is collected: a service of the catalog not turned off in `metric_namespace_configs`
/// (by its Datadog name), or a user-defined domain (`custom`, `workload`, `prometheus`) turned on there.
pub fn collected(config: &Value, metric_type: &str) -> bool {
    let short = short_name(metric_type);
    let setting = config["metric_namespace_configs"].as_array().and_then(|cs| cs.iter().find(|c| c["id"].as_str() == Some(short.as_str()))).map(|c| c["disabled"] != true);
    if known(metric_type) {
        return setting.unwrap_or(true);
    }
    catalog().custom_namespaces.iter().any(|s| metric_type.starts_with(&s.prefix)) && setting == Some(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::collections::BTreeMap;

    #[test]
    fn names_follow_datadogs_gcp_metrics() {
        assert_eq!(metric_name("compute.googleapis.com/instance/cpu/utilization"), "gcp.gce.instance.cpu.utilization");
        assert_eq!(metric_name("storage.googleapis.com/storage/object_count"), "gcp.storage.storage.object_count");
        assert_eq!(metric_name("file.googleapis.com/nfs/server/free_bytes"), "gcp.file.nfs.server.free_bytes");
        assert_eq!(metric_name("appengine.googleapis.com/http/server/response_count"), "gcp.gae.http.server.response_count");
        assert_eq!(metric_name("logging.googleapis.com/user/checkout-errors"), "gcp.logging.user.checkout_errors");
        assert_eq!(metric_name("prometheus.googleapis.com/http_requests_total/counter"), "gcp.prometheus.http_requests_total.counter");
        assert_eq!(metric_name("cloudsql.googleapis.com/database/cpu/utilization"), "gcp.cloudsql.database.cpu.utilization");
        assert_eq!(metric_name("kubernetes.io/container/cpu/core_usage_time"), "gcp.gke.container.cpu.core_usage_time");
        assert_eq!(metric_name("custom.googleapis.com/checkout/orders"), "gcp.custom.checkout.orders");
        assert_eq!(short_name("pubsub.googleapis.com/subscription/num_undelivered_messages"), "pubsub");
        assert!(known("compute.googleapis.com/instance/uptime"));
        assert!(!known("custom.googleapis.com/checkout/orders"));
    }

    #[test]
    fn resource_filters_apply_to_their_resource_type() {
        let tags = |t: &[&str]| t.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let config = json!({ "monitored_resource_configs": [{ "type": "gce_instance", "filters": ["env:prod", "!datadog:no"] }], "host_filters": [], "cloud_run_revision_filters": ["service:checkout"] });
        assert!(resource_selected(&config, "gce_instance", &tags(&["env:prod"])));
        assert!(!resource_selected(&config, "gce_instance", &tags(&["env:staging"])));
        assert!(!resource_selected(&config, "gce_instance", &tags(&["env:prod", "datadog:no"])));
        assert!(resource_selected(&config, "cloud_run_revision", &tags(&["service:checkout"])));
        assert!(!resource_selected(&config, "cloud_run_revision", &tags(&["service:catalog"])));
        assert!(resource_selected(&config, "cloudsql_database", &tags(&[])));
        // The deprecated host_filters still filter instances.
        let legacy = json!({ "host_filters": ["team:storefront"] });
        assert!(!resource_selected(&legacy, "gce_instance", &tags(&["team:catalog"])));
    }

    #[test]
    fn disabled_metric_namespaces_are_skipped_and_user_defined_ones_are_opt_in() {
        let config = json!({ "metric_namespace_configs": [{ "id": "cloudsql", "disabled": true }, { "id": "gce", "disabled": false }, { "id": "prometheus", "disabled": false }] });
        assert!(!collected(&config, "cloudsql.googleapis.com/database/cpu/utilization"));
        assert!(collected(&config, "compute.googleapis.com/instance/cpu/utilization"));
        assert!(collected(&json!({}), "pubsub.googleapis.com/topic/send_message_operation_count"));
        assert!(collected(&json!({}), "networksecurity.googleapis.com/https/request_count"));
        // custom., workload. and prometheus.googleapis.com only when their namespace is turned on.
        assert!(collected(&config, "prometheus.googleapis.com/up/gauge"));
        assert!(!collected(&config, "custom.googleapis.com/checkout/orders"));
        assert!(!collected(&json!({}), "workload.googleapis.com/nginx.requests"));
        assert!(!collected(&json!({}), "agent.googleapis.com/cpu/utilization"));
    }

    #[test]
    fn each_kind_is_published_under_datadogs_names() {
        let names = |t: &str, kind: &str, value: &str| stats(t, kind, value).iter().map(|s| format!("{}{}", metric_name(t), s.suffix())).collect::<Vec<_>>();
        assert_eq!(names("compute.googleapis.com/instance/cpu/utilization", "GAUGE", "DOUBLE"), ["gcp.gce.instance.cpu.utilization"]);
        assert_eq!(names("pubsub.googleapis.com/topic/send_request_count", "DELTA", "INT64"), ["gcp.pubsub.topic.send_request_count"]);
        // Cumulative counters: the running total under the bare name, the change since the previous point as .delta.
        assert_eq!(names("kubernetes.io/container/restart_count", "CUMULATIVE", "INT64"), ["gcp.gke.container.restart_count", "gcp.gke.container.restart_count.delta"]);
        // Distributions: mean, count and sum of squared deviations; the percentiles Datadog has for some.
        assert_eq!(
            names("pubsub.googleapis.com/topic/send_request_latencies", "DELTA", "DISTRIBUTION"),
            ["gcp.pubsub.topic.send_request_latencies.avg", "gcp.pubsub.topic.send_request_latencies.samplecount", "gcp.pubsub.topic.send_request_latencies.sumsqdev"]
        );
        assert_eq!(names("loadbalancing.googleapis.com/https/total_latencies", "DELTA", "DISTRIBUTION")[3..], ["gcp.loadbalancing.https.total_latencies.p95".to_string(), "gcp.loadbalancing.https.total_latencies.p99".to_string()]);
        assert!(names("compute.googleapis.com/instance/uptime_total", "GAUGE", "STRING").is_empty());
    }

    #[test]
    fn region_filters_keep_the_resources_of_their_locations() {
        let labels = |l: &[(&str, &str)]| l.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect::<BTreeMap<_, _>>();
        let config = json!({ "region_filter_configs": ["europe-west1", "us-central1-a", "eu"] });
        assert!(location_selected(&config, &labels(&[("zone", "europe-west1-b")])));
        assert!(location_selected(&config, &labels(&[("region", "europe-west1")])));
        assert!(location_selected(&config, &labels(&[("location", "eu")])));
        assert!(location_selected(&config, &labels(&[("zone", "us-central1-a")])));
        assert!(!location_selected(&config, &labels(&[("zone", "us-central1-b")])));
        assert!(!location_selected(&config, &labels(&[("location", "europe-west10")])));
        // Global resources (no location, or `global`) follow is_global_location_enabled.
        assert!(location_selected(&config, &labels(&[("project_id", "p")])));
        assert!(location_selected(&config, &labels(&[("location", "global")])));
        let no_global = json!({ "region_filter_configs": [], "is_global_location_enabled": false });
        assert!(!location_selected(&no_global, &labels(&[("location", "global")])));
        assert!(location_selected(&no_global, &labels(&[("zone", "asia-east1-a")])));
    }

    #[test]
    fn every_gcp_dashboard_charts_what_the_crawler_publishes() {
        use std::collections::HashSet;
        use std::io::Read;
        let doc: Value = serde_json::from_str(include_str!("../../../../seeds/integrations/cloud/gcp-metrics.json")).unwrap();
        let services: Value = serde_json::from_str(include_str!("../../../../seeds/gcp_integration.json")).unwrap();
        let mut text = String::new();
        flate2::read::GzDecoder::new(&include_bytes!("../../../../seeds/integrations/cloud/services.ndjson.gz")[..]).read_to_string(&mut text).unwrap();
        let lines: Vec<Value> = text.lines().map(|l| serde_json::from_str(l).unwrap()).collect();
        let query = regex::Regex::new(r#"[a-z]+:(gcp\.[a-z0-9_.]+)\{"#).unwrap();
        // Every name the crawler gives each documented metric (of the service's main resource and its others), by its kind and value type.
        let mut names = HashSet::new();
        for spec in doc["services"].as_object().unwrap().values() {
            for m in spec["metrics"].as_array().unwrap().iter().chain(spec["others"].as_array().into_iter().flatten()) {
                let (t, kind, value) = (m["type"].as_str().unwrap(), m["kind"].as_str().unwrap(), m["value"].as_str().unwrap());
                names.extend(stats(t, kind, value).iter().map(|s| format!("{}{}", metric_name(t), s.suffix())));
            }
        }
        let mut checked = 0;
        for s in services["services"].as_array().unwrap() {
            let Some(line) = lines.iter().find(|l| l["logo"] == format!("tile:{}", s["id"].as_str().unwrap())) else { continue };
            assert_eq!(line["prefix"].as_str().unwrap(), format!("gcp.{}.", short_name(s["prefix"].as_str().unwrap())));
            for c in query.captures_iter(&line["dashboards"][0]["widgets"].to_string()) {
                assert!(names.contains(&c[1]), "{}: {} isn't published by the crawler", s["id"], &c[1]);
            }
            checked += 1;
        }
        assert!(checked >= 60, "{checked} Google Cloud service dashboards");
        let overview = include_str!("../../../../seeds/integrations/gcp/dashboards.json");
        let mut charted = 0;
        for c in query.captures_iter(overview) {
            assert!(names.contains(&c[1]), "the Google Cloud overview: {} isn't published by the crawler", &c[1]);
            charted += 1;
        }
        assert!(charted >= 20, "{charted} queries in the Google Cloud overview");
    }
}
