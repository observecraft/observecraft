//! How Azure Monitor metrics become ObserveCraft metrics, Datadog-compatible: names
//! (`azure.vm.percentage_cpu`), the app registration's filters and resource provider settings.

use serde_json::Value;

use crate::cloud::naming::{glob, snake, underscored};

/// The aggregations the crawler reads, and the suffix their metric names get.
pub const AGGREGATIONS: [(&str, &str); 3] = [("average", ""), ("total", ".total"), ("maximum", ".maximum")];

/// Resource types Datadog names other than `azure.<provider>_<type>`.
const PREFIXES: [(&str, &str); 4] = [
    ("microsoft.compute/virtualmachines", "azure.vm"),
    ("microsoft.web/sites", "azure.app_services"),
    ("microsoft.storage/storageaccounts", "azure.storage"),
    ("microsoft.documentdb/databaseaccounts", "azure.cosmosdb"),
];

/// A resource type's metric prefix: `Microsoft.Network/loadBalancers` → `azure.network_loadbalancers`.
pub fn metric_prefix(resource_type: &str) -> String {
    let lower = resource_type.to_ascii_lowercase();
    if let Some((_, p)) = PREFIXES.iter().find(|(t, _)| *t == lower) {
        return p.to_string();
    }
    format!("azure.{}", underscored(lower.strip_prefix("microsoft.").unwrap_or(&lower)))
}

/// `Percentage CPU` of a VM → `azure.vm.percentage_cpu`.
pub fn metric_name(resource_type: &str, metric: &str) -> String {
    format!("{}.{}", metric_prefix(resource_type), snake(metric))
}

/// Datadog's Azure filters: comma-separated `key:value` tags (`*` wildcards); a resource passes with
/// one of them and none of the `!`-negated ones. Empty filters pass everything.
pub fn filters_match(filters: &str, tags: &[String]) -> bool {
    let patterns: Vec<&str> = filters.split(',').map(str::trim).filter(|p| !p.is_empty()).collect();
    let matches = |p: &str| tags.iter().any(|t| glob(p, t));
    let (negated, wanted): (Vec<&str>, Vec<&str>) = patterns.into_iter().partition(|p| p.starts_with('!'));
    if negated.iter().any(|p| matches(&p[1..])) {
        return false;
    }
    wanted.is_empty() || wanted.iter().any(|p| matches(p))
}

/// The filter field that applies to a resource type: VMs, App Service plans, container apps.
pub fn filter_key(resource_type: &str) -> Option<&'static str> {
    match resource_type.to_ascii_lowercase().as_str() {
        "microsoft.compute/virtualmachines" => Some("host_filters"),
        "microsoft.web/serverfarms" => Some("app_service_plan_filters"),
        "microsoft.app/containerapps" => Some("container_app_filters"),
        _ => None,
    }
}

/// Whether the app registration collects a resource type's metrics: its resource provider's setting,
/// or `metrics_enabled_default`.
pub fn provider_enabled(config: &Value, resource_type: &str) -> bool {
    let namespace = resource_type.split('/').next().unwrap_or_default();
    config["resource_provider_configs"]
        .as_array()
        .and_then(|cs| cs.iter().find(|c| c["namespace"].as_str().is_some_and(|n| n.eq_ignore_ascii_case(namespace))))
        .and_then(|c| c["metrics_enabled"].as_bool())
        .unwrap_or_else(|| config["metrics_enabled_default"].as_bool().unwrap_or(true))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn names_follow_datadogs_azure_metrics() {
        assert_eq!(metric_name("Microsoft.Compute/virtualMachines", "Percentage CPU"), "azure.vm.percentage_cpu");
        assert_eq!(metric_name("Microsoft.Web/sites", "Http5xx"), "azure.app_services.http5xx");
        assert_eq!(metric_name("Microsoft.Web/sites", "CpuTime"), "azure.app_services.cpu_time");
        assert_eq!(metric_name("Microsoft.Storage/storageAccounts", "Transactions"), "azure.storage.transactions");
        assert_eq!(metric_name("Microsoft.Sql/servers/databases", "cpu_percent"), "azure.sql_servers_databases.cpu_percent");
        assert_eq!(metric_name("Microsoft.Network/loadBalancers", "VipAvailability"), "azure.network_loadbalancers.vip_availability");
        assert_eq!(metric_prefix("microsoft.cache/redis"), "azure.cache_redis");
    }

    #[test]
    fn the_catalogs_prefixes_are_the_crawlers() {
        let seed: Value = serde_json::from_str(include_str!("../../../../seeds/azure_integration.json")).unwrap();
        for t in seed["resource_types"].as_array().unwrap() {
            assert_eq!(metric_prefix(t["type"].as_str().unwrap()), t["prefix"].as_str().unwrap(), "{t}");
        }
    }

    #[test]
    fn filters_are_comma_separated_tags_with_wildcards_and_negations() {
        let tags = |t: &[&str]| t.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert!(filters_match("", &tags(&[])));
        assert!(filters_match("env:prod,team:storefront*", &tags(&["team:storefront-api"])));
        assert!(!filters_match("env:prod", &tags(&["env:staging"])));
        assert!(!filters_match("env:prod, !datadog:no", &tags(&["env:prod", "datadog:no"])));
        assert!(filters_match("!datadog:no", &tags(&["env:prod"])));
    }

    #[test]
    fn filters_apply_to_their_resource_types() {
        assert_eq!(filter_key("Microsoft.Compute/virtualMachines"), Some("host_filters"));
        assert_eq!(filter_key("Microsoft.Web/serverFarms"), Some("app_service_plan_filters"));
        assert_eq!(filter_key("Microsoft.App/containerApps"), Some("container_app_filters"));
        assert_eq!(filter_key("Microsoft.Storage/storageAccounts"), None);
    }

    #[test]
    fn resource_providers_follow_their_setting_or_the_default() {
        let config = json!({ "metrics_enabled_default": true, "resource_provider_configs": [{ "namespace": "Microsoft.Web", "metrics_enabled": false }] });
        assert!(provider_enabled(&config, "Microsoft.Compute/virtualMachines"));
        assert!(!provider_enabled(&config, "microsoft.web/sites"));
        let only = json!({ "metrics_enabled_default": false, "resource_provider_configs": [{ "namespace": "Microsoft.Compute", "metrics_enabled": true }] });
        assert!(provider_enabled(&only, "Microsoft.Compute/virtualMachines"));
        assert!(!provider_enabled(&only, "Microsoft.Storage/storageAccounts"));
    }
}
