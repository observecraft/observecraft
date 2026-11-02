//! How Cloud Monitoring metrics become ObserveCraft metrics, Datadog-compatible: names
//! (`compute.googleapis.com/instance/cpu/utilization` → `gcp.gce.instance.cpu.utilization`), the
//! service account's resource filters and metric namespace settings.

use std::sync::OnceLock;

use serde::Deserialize;
use serde_json::Value;

use crate::cloud::naming::{glob, underscored};

#[derive(Deserialize)]
struct Catalog {
    services: Vec<Service>,
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

/// The service of a metric type (`compute.googleapis.com/…` → `gce`); outside the catalog, its domain
/// without `.googleapis.com` (`custom.googleapis.com` → `custom`).
pub fn short_name(metric_type: &str) -> String {
    if let Some(s) = catalog().services.iter().find(|s| metric_type.starts_with(&s.prefix)) {
        return s.short.clone();
    }
    let domain = metric_type.split('/').next().unwrap_or_default();
    underscored(domain.strip_suffix(".googleapis.com").unwrap_or(domain))
}

/// Whether the metric type belongs to a service of the catalog (others are custom metrics).
pub fn known(metric_type: &str) -> bool {
    catalog().services.iter().any(|s| metric_type.starts_with(&s.prefix))
}

/// `compute.googleapis.com/instance/cpu/utilization` → `gcp.gce.instance.cpu.utilization`.
pub fn metric_name(metric_type: &str) -> String {
    let path = metric_type.split_once('/').map(|(_, p)| p).unwrap_or_default();
    let path: Vec<String> = path.split('/').map(underscored).filter(|p| !p.is_empty()).collect();
    format!("gcp.{}.{}", short_name(metric_type), path.join("."))
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

/// Whether a metric's service isn't turned off in `metric_namespace_configs` (by its Datadog name).
pub fn namespace_enabled(config: &Value, metric_type: &str) -> bool {
    let short = short_name(metric_type);
    !config["metric_namespace_configs"].as_array().is_some_and(|cs| cs.iter().any(|c| c["id"].as_str() == Some(short.as_str()) && c["disabled"] == true))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn names_follow_datadogs_gcp_metrics() {
        assert_eq!(metric_name("compute.googleapis.com/instance/cpu/utilization"), "gcp.gce.instance.cpu.utilization");
        assert_eq!(metric_name("storage.googleapis.com/storage/object_count"), "gcp.gcs.storage.object_count");
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
    fn disabled_metric_namespaces_are_skipped() {
        let config = json!({ "metric_namespace_configs": [{ "id": "cloudsql", "disabled": true }, { "id": "gce", "disabled": false }] });
        assert!(!namespace_enabled(&config, "cloudsql.googleapis.com/database/cpu/utilization"));
        assert!(namespace_enabled(&config, "compute.googleapis.com/instance/cpu/utilization"));
        assert!(namespace_enabled(&json!({}), "pubsub.googleapis.com/topic/send_message_operation_count"));
    }
}
