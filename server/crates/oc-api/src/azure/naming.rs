//! How Azure Monitor metrics become ObserveCraft metrics, Datadog-compatible: names
//! (`azure.vm.percentage_cpu`), the app registration's filters and resource provider settings.

use serde_json::Value;

use crate::cloud::naming::{glob, snake, underscored};

/// The aggregations a metric is published with, and their names' suffixes: its primary aggregation (Azure
/// Monitor's `primaryAggregationType`) under the bare name, as Datadog has it (counts summed, gauges averaged),
/// and its maximum as `.max` where Azure Monitor has one (Datadog's `azure.app_services.cpu_time.max`). A
/// metric without a primary aggregation is averaged (else its first supported aggregation).
pub fn aggregations(primary: &str, supported: &[String]) -> Vec<(String, &'static str)> {
    let supported: Vec<String> = supported.iter().map(|a| a.to_ascii_lowercase()).collect();
    let primary = primary.to_ascii_lowercase();
    let primary = match primary.as_str() {
        "" | "none" => if supported.is_empty() || supported.iter().any(|a| a == "average") { "average".to_string() } else { supported[0].clone() },
        _ => primary,
    };
    let mut out = vec![(primary.clone(), "")];
    if primary != "maximum" && supported.iter().any(|a| a == "maximum") {
        out.push(("maximum".to_string(), ".max"));
    }
    out
}

/// The resource count of a type (`azure.vm.count`): one series per resource, valued 1, tagged with its status.
pub fn count_name(resource_type: &str) -> String {
    format!("{}.count", metric_prefix(resource_type))
}

/// The dimensions the crawler splits metrics by, per resource type: the most used ones (App Service instances,
/// Storage API names and response types, Cosmos DB databases and collections, Redis shards, Event Hubs and
/// Service Bus entities, backup jobs' health, model deployments, …). Others are read aggregated over their
/// dimensions, keeping the series count bounded.
const SPLITS: [(&str, &[&str]); 27] = [
    ("microsoft.web/sites", &["Instance"]),
    ("microsoft.web/sites/slots", &["Instance"]),
    ("microsoft.web/serverfarms", &["Instance"]),
    ("microsoft.storage/storageaccounts", &["ApiName", "ResponseType"]),
    ("microsoft.storage/storageaccounts/blobservices", &["ApiName", "ResponseType", "BlobType", "Tier"]),
    ("microsoft.storage/storageaccounts/fileservices", &["ApiName", "ResponseType", "FileShare"]),
    ("microsoft.storage/storageaccounts/queueservices", &["ApiName", "ResponseType"]),
    ("microsoft.storage/storageaccounts/tableservices", &["ApiName", "ResponseType"]),
    ("microsoft.documentdb/databaseaccounts", &["DatabaseName", "CollectionName", "Region", "StatusCode"]),
    ("microsoft.cache/redis", &["ShardId"]),
    ("microsoft.eventhub/namespaces", &["EntityName"]),
    ("microsoft.servicebus/namespaces", &["EntityName"]),
    ("microsoft.network/loadbalancers", &["FrontendIPAddress", "BackendIPAddress"]),
    ("microsoft.network/applicationgateways", &["Listener", "BackendSettingsPool", "HttpStatusGroup"]),
    ("microsoft.keyvault/vaults", &["ActivityType", "ActivityName", "StatusCode"]),
    ("microsoft.apimanagement/service", &["Location", "ApiId", "GatewayResponseCodeCategory"]),
    ("microsoft.containerservice/managedclusters", &["nodepool", "node", "phase"]),
    ("microsoft.network/frontdoors", &["HttpStatusGroup", "Backend"]),
    ("microsoft.cdn/profiles", &["Endpoint", "HttpStatusGroup"]),
    ("microsoft.sql/servers/elasticpools", &["DatabaseResourceId"]),
    ("microsoft.recoveryservices/vaults", &["healthStatus", "dataSourceType"]),
    ("microsoft.dataprotection/backupvaults", &["healthStatus", "dataSourceType"]),
    ("microsoft.app/containerapps", &["revisionName", "statusCodeCategory"]),
    ("microsoft.cognitiveservices/accounts", &["ModelDeploymentName", "StatusCode"]),
    ("microsoft.eventgrid/topics", &["EventSubscriptionName"]),
    ("microsoft.eventgrid/systemtopics", &["EventSubscriptionName"]),
    ("microsoft.synapse/workspaces", &["Result"]),
];

/// Which of a metric's dimensions (its definition's) the crawler splits it by.
pub fn splits(resource_type: &str, dimensions: &[String]) -> Vec<String> {
    let lower = resource_type.to_ascii_lowercase();
    let Some((_, wanted)) = SPLITS.iter().find(|(t, _)| *t == lower) else { return vec![] };
    wanted.iter().filter_map(|w| dimensions.iter().find(|d| d.eq_ignore_ascii_case(w)).cloned()).collect()
}

/// A dimension's tag key: lowercase, as Datadog tags Azure dimensions (`ApiName` → `apiname`).
pub fn dimension_tag(dimension: &str) -> String {
    underscored(dimension)
}

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
    fn the_bare_name_carries_the_primary_aggregation_and_maxima_are_dot_max() {
        let s = |a: &[&str]| a.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        // A count (Http5xx: Total) is its sum, with its maximum as `.max` (as Datadog's `cpu_time.max`).
        assert_eq!(aggregations("Total", &s(&["Total", "Average", "Maximum", "Minimum"])), [("total".to_string(), ""), ("maximum".to_string(), ".max")]);
        assert_eq!(aggregations("average", &s(&["Average", "Maximum"])), [("average".to_string(), ""), ("maximum".to_string(), ".max")]);
        assert_eq!(aggregations("Count", &s(&["Count"])), [("count".to_string(), "")]);
        assert_eq!(aggregations("Maximum", &s(&["Maximum", "Average"])), [("maximum".to_string(), "")]);
        // No primary aggregation: the average, else the first supported.
        assert_eq!(aggregations("None", &s(&["Total", "Average"])), [("average".to_string(), "")]);
        assert_eq!(aggregations("", &s(&["Total"])), [("total".to_string(), "")]);
        assert_eq!(count_name("Microsoft.Compute/virtualMachines"), "azure.vm.count");
        assert_eq!(count_name("Microsoft.Cache/redis"), "azure.cache_redis.count");
    }

    #[test]
    fn the_most_used_dimensions_are_split_and_become_lowercase_tags() {
        let s = |a: &[&str]| a.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        assert_eq!(splits("Microsoft.Web/sites", &s(&["Instance"])), ["Instance"]);
        assert_eq!(splits("microsoft.storage/storageaccounts", &s(&["GeoType", "ApiName", "Authentication", "ResponseType"])), ["ApiName", "ResponseType"]);
        assert_eq!(splits("Microsoft.Cache/Redis", &s(&["ShardId", "Port", "Primary"])), ["ShardId"]);
        assert_eq!(splits("Microsoft.EventHub/namespaces", &s(&["EntityName", "OperationResult"])), ["EntityName"]);
        assert!(splits("Microsoft.Compute/virtualMachines", &s(&["LUN"])).is_empty());
        // Backup jobs by health state and workload, container app requests by status class, model deployments.
        let backup = s(&["dataSourceURL", "backupInstanceUrl", "dataSourceType", "healthStatus", "backupInstanceName"]);
        assert_eq!(splits("Microsoft.RecoveryServices/vaults", &backup), ["healthStatus", "dataSourceType"]);
        assert_eq!(splits("Microsoft.DataProtection/backupVaults", &backup), ["healthStatus", "dataSourceType"]);
        assert_eq!(splits("Microsoft.App/containerApps", &s(&["revisionName", "podName", "statusCodeCategory", "statusCode"])), ["revisionName", "statusCodeCategory"]);
        assert_eq!(splits("Microsoft.CognitiveServices/accounts", &s(&["ApiName", "ModelDeploymentName", "StatusCode"])), ["ModelDeploymentName", "StatusCode"]);
        assert_eq!(splits("Microsoft.EventGrid/systemTopics", &s(&["EventSubscriptionName"])), ["EventSubscriptionName"]);
        assert_eq!(splits("Microsoft.Network/applicationGateways", &s(&["HttpStatusGroup"])), ["HttpStatusGroup"]);
        assert_eq!(splits("Microsoft.ApiManagement/service", &s(&["Location", "Hostname", "GatewayResponseCodeCategory", "ApiId"])), ["Location", "ApiId", "GatewayResponseCodeCategory"]);
        assert_eq!(splits("Microsoft.ContainerService/managedClusters", &s(&["phase", "namespace", "pod"])), ["phase"]);
        assert_eq!(splits("Microsoft.Synapse/workspaces", &s(&["Result", "FailureType", "Pipeline"])), ["Result"]);
        assert_eq!(dimension_tag("healthStatus"), "healthstatus");
        assert_eq!(dimension_tag("ApiName"), "apiname");
        assert_eq!(dimension_tag("Microsoft.ResourceId"), "microsoft_resourceid");
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

    /// The Azure service dashboards (`scripts/cloud-service-dashboards.mjs`) and the Azure overview
    /// (`scripts/cloud-dashboards.mjs`) chart only names the crawler publishes: a metric's bare name (its
    /// primary aggregation), its `.max`, or a type's resource count; split by the resources' tags.
    #[test]
    fn every_azure_dashboard_charts_what_the_crawler_publishes() {
        use std::collections::HashSet;
        use std::io::Read;
        let doc: Value = serde_json::from_str(include_str!("../../../../seeds/integrations/cloud/azure-metrics.json")).unwrap();
        let catalog: Value = serde_json::from_str(include_str!("../../../../seeds/azure_integration.json")).unwrap();
        let mut text = String::new();
        flate2::read::GzDecoder::new(&include_bytes!("../../../../seeds/integrations/cloud/services.ndjson.gz")[..]).read_to_string(&mut text).unwrap();
        let lines: Vec<Value> = text.lines().map(|l| serde_json::from_str(l).unwrap()).collect();
        let query = regex::Regex::new(r#"[a-z]+:(azure\.[a-z0-9_.]+)\{[^}]*\}(?: by \{([a-z0-9_,]+)\})?"#).unwrap();
        let resource_tags: HashSet<&str> = HashSet::from(["subscription_id", "resource_group", "region", "name", "status"]);
        let strings = |v: &Value| v.as_array().map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect::<Vec<_>>()).unwrap_or_default();
        // Every name the crawler gives each documented metric (the documented aggregations), and the overview's `.max` of gauges.
        // Each name's tags beside the resource's: the dimensions the crawler splits it by.
        let (mut names, mut gauges, mut split_tags) = (HashSet::new(), HashSet::new(), std::collections::HashMap::<String, Vec<String>>::new());
        for (kind, spec) in doc["resource_types"].as_object().unwrap() {
            names.insert(count_name(kind));
            for m in spec["metrics"].as_array().unwrap() {
                let (name, primary) = (m["name"].as_str().unwrap(), m["aggregation"].as_str().unwrap());
                let tags: Vec<String> = splits(kind, &strings(&m["dimensions"])).iter().map(|d| dimension_tag(d)).collect();
                for (_, suffix) in aggregations(primary, &strings(&m["aggregations"])) {
                    names.insert(format!("{}{suffix}", metric_name(kind, name)));
                    split_tags.insert(format!("{}{suffix}", metric_name(kind, name)), tags.clone());
                }
                if primary == "Average" {
                    gauges.insert(format!("{}.max", metric_name(kind, name)));
                }
            }
        }
        let mut checked = 0;
        for t in catalog["resource_types"].as_array().unwrap() {
            let Some(line) = lines.iter().find(|l| l["logo"] == format!("tile:{}", t["id"].as_str().unwrap())) else { continue };
            assert_eq!(line["prefix"].as_str().unwrap(), format!("{}.", metric_prefix(t["type"].as_str().unwrap())));
            for c in query.captures_iter(&line["dashboards"][0]["widgets"].to_string()) {
                assert!(names.contains(&c[1]), "{}: {} isn't published by the crawler", t["type"], &c[1]);
                for tag in c.get(2).map(|g| g.as_str()).unwrap_or_default().split(',').filter(|t| !t.is_empty()) {
                    let split = split_tags.get(&c[1]).is_some_and(|ts| ts.iter().any(|x| x == tag));
                    assert!(resource_tags.contains(tag) || split, "{}: {} splits by {tag}", t["type"], &c[1]);
                }
            }
            checked += 1;
        }
        assert!(checked >= 50, "{checked} Azure service dashboards");
        let overview = include_str!("../../../../seeds/integrations/azure/dashboards.json");
        let mut charted = 0;
        for c in query.captures_iter(overview) {
            assert!(names.contains(&c[1]) || gauges.contains(&c[1]), "the Azure overview: {} isn't published by the crawler", &c[1]);
            charted += 1;
        }
        assert!(charted >= 20, "{charted} queries in the Azure overview");
    }
}
