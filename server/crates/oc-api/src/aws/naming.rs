//! How CloudWatch metrics become ObserveCraft metrics, Datadog-compatible: names, tags, the
//! account's filters, and the OTLP JSON the crawler publishes.

use std::collections::HashMap;
use std::sync::OnceLock;

use serde_json::Value;

pub use crate::cloud::naming::Point;
use crate::cloud::naming::{glob, snake, underscored};

/// A CloudWatch statistic the crawler collects, and the suffix its metric name gets (the metric's Datadog
/// statistic, Average unless `aws-metrics.json` says otherwise, is also published under the bare name).
pub const STATS: [(&str, &str); 3] = [("Average", ""), ("Sum", ".sum"), ("Maximum", ".maximum")];

/// Namespaces Datadog doesn't name `aws.<namespace>.`: RDS Proxy is `aws.rds.proxy.*`, ECS Container
/// Insights `ecs.containerinsights.*`.
const PREFIXES: [(&str, &str); 2] = [("AWS/RDS/Proxy", "aws.rds.proxy"), ("ECS/ContainerInsights", "ecs.containerinsights")];

/// How Datadog publishes a metric of the namespaces `seeds/integrations/cloud/aws-metrics.json` marks
/// `datadog` (`scripts/aws-datadog-metrics.json`): the statistic of its bare name and its name where it
/// isn't the snake-cased CloudWatch one (`401ErrorRate` → `401_error_rate`).
#[derive(Debug, Clone, PartialEq)]
pub struct DatadogMetric {
    pub statistic: String,
    pub name: Option<String>,
}

fn datadog_metrics() -> &'static HashMap<(String, String), DatadogMetric> {
    static M: OnceLock<HashMap<(String, String), DatadogMetric>> = OnceLock::new();
    M.get_or_init(|| {
        let doc: Value = serde_json::from_str(include_str!("../../../../seeds/integrations/cloud/aws-metrics.json")).expect("aws-metrics.json is valid");
        let mut out = HashMap::new();
        for (ns, spec) in doc["namespaces"].as_object().into_iter().flatten().filter(|(_, s)| s["datadog"] == true) {
            for m in spec["metrics"].as_array().into_iter().flatten() {
                let Some(name) = m["name"].as_str() else { continue };
                out.entry((ns.clone(), name.to_string()))
                    .or_insert_with(|| DatadogMetric { statistic: m["statistic"].as_str().unwrap_or("Average").to_string(), name: m["datadog_name"].as_str().map(str::to_string) });
            }
        }
        out
    })
}

/// The statistic a metric's bare name carries: Datadog's for the namespaces we follow it in, else Average.
pub fn default_statistic(namespace: &str, metric: &str) -> &'static str {
    datadog_metrics().get(&(namespace.to_string(), metric.to_string())).map_or("Average", |m| m.statistic.as_str())
}

/// The Datadog prefix of a namespace's metrics: `aws.<namespace>` (`AWS/EC2` → `aws.ec2`; namespaces
/// outside `AWS/` keep their whole name), except those of [`PREFIXES`].
pub fn metric_prefix(namespace: &str) -> String {
    if let Some((_, p)) = PREFIXES.iter().find(|(ns, _)| *ns == namespace) {
        return p.to_string();
    }
    let lower = namespace.to_ascii_lowercase();
    let ns = lower.strip_prefix("/aws/").or_else(|| lower.strip_prefix("aws/")).unwrap_or(&lower);
    format!("aws.{}", underscored(ns))
}

fn leaf(namespace: &str, metric: &str) -> String {
    datadog_metrics().get(&(namespace.to_string(), metric.to_string())).and_then(|m| m.name.clone()).unwrap_or_else(|| snake(metric))
}

/// The Datadog name of a CloudWatch metric: `aws.<namespace>.<metric>` (`AWS/EC2` + `NetworkIn` →
/// `aws.ec2.network_in`).
pub fn metric_name(namespace: &str, metric: &str) -> String {
    format!("{}.{}", metric_prefix(namespace), leaf(namespace, metric))
}

/// Whether a dimension set's series keep the bare name, and the segment of the other name Datadog gives
/// them: ECS's cluster and service metrics are also `aws.ecs.cluster.*` / `aws.ecs.service.*`, Container
/// Insights' task definition families only `ecs.containerinsights.family.*`.
fn variant(namespace: &str, dimensions: &[(String, String)]) -> (bool, Option<&'static str>) {
    let has = |d: &str| dimensions.iter().any(|(k, _)| k == d);
    match namespace {
        "AWS/ECS" if has("ServiceName") => (true, Some("service")),
        "AWS/ECS" if dimensions.len() == 1 && has("ClusterName") => (true, Some("cluster")),
        "ECS/ContainerInsights" if has("TaskDefinitionFamily") => (false, Some("family")),
        _ => (true, None),
    }
}

/// Every name a series of `statistic` is published under: the bare name (and its Datadog variant) for the
/// metric's default statistic, `.sum` / `.maximum` for those statistics.
pub fn series_names(namespace: &str, metric: &str, dimensions: &[(String, String)], statistic: &str) -> Vec<String> {
    let mut suffixes = vec![];
    if statistic == default_statistic(namespace, metric) {
        suffixes.push("");
    }
    if let Some(&(_, s)) = STATS.iter().find(|(st, s)| *st == statistic && !s.is_empty()) {
        suffixes.push(s);
    }
    let (prefix, leaf) = (metric_prefix(namespace), leaf(namespace, metric));
    let (bare, extra) = variant(namespace, dimensions);
    let bases: Vec<String> = bare.then(|| format!("{prefix}.{leaf}")).into_iter().chain(extra.map(|v| format!("{prefix}.{v}.{leaf}"))).collect();
    bases.iter().flat_map(|b| suffixes.iter().map(move |s| format!("{b}{s}"))).collect()
}

/// Whether a series is collected: RDS's account-wide aggregates (by engine, by instance class, or none)
/// are left out, as Datadog has RDS metrics per instance and per cluster (they would double the sums).
pub fn series_collected(namespace: &str, dimensions: &[(String, String)]) -> bool {
    namespace != "AWS/RDS" || dimensions.iter().any(|(k, _)| k == "DBInstanceIdentifier" || k == "DBClusterIdentifier")
}

/// Global services: CloudWatch has their metrics in us-east-1 only (CloudFront, the web ACLs of
/// CloudFront distributions, Route 53 health checks, billing). They are collected from us-east-1 even
/// when the account's regions leave it out.
pub fn global_series(namespace: &str, dimensions: &[(String, String)]) -> bool {
    match namespace {
        "AWS/CloudFront" | "AWS/Route53" | "AWS/Billing" => true,
        "AWS/WAFV2" | "WAF" => !dimensions.iter().any(|(k, _)| k == "Region"),
        _ => false,
    }
}

/// A dimension's tag key: lowercased (`FunctionName` → `functionname`), as Datadog tags them.
pub fn tag_key(dimension: &str) -> String {
    underscored(dimension)
}

fn strings(v: &Value) -> Vec<&str> {
    v.as_array().map(|a| a.iter().filter_map(Value::as_str).collect()).unwrap_or_default()
}

/// Whether the account collects a namespace: its `namespace_filters` (`include_only` / `exclude_only`),
/// and namespaces outside our catalog (`known` false) only with `collect_custom_metrics`.
pub fn namespace_selected(filters: &Value, namespace: &str, known: bool, custom: bool) -> bool {
    if !known && !custom {
        return false;
    }
    if let Some(only) = filters.get("include_only") {
        return strings(only).contains(&namespace);
    }
    !strings(&filters["exclude_only"]).contains(&namespace)
}

/// Whether a metric passes its namespace's `metric_name_filters` (Datadog metric names, `*` wildcards).
pub fn metric_selected(metric_name_filters: &Value, namespace: &str, name: &str) -> bool {
    let Some(filter) = metric_name_filters.as_array().and_then(|fs| fs.iter().find(|f| f["namespace"] == namespace)) else { return true };
    if let Some(only) = filter.get("include_only") {
        return strings(only).iter().any(|p| glob(p, name));
    }
    !strings(&filter["exclude_only"]).iter().any(|p| glob(p, name))
}

/// Whether a resource passes its namespace's `tag_filters`: one of the tags matches (`*` wildcards),
/// and none of the `!`-negated ones. A namespace without a filter collects every resource.
pub fn tags_match(filters: &Value, namespace: &str, resource_tags: &[String]) -> bool {
    let patterns: Vec<&str> = filters.as_array().map(|fs| fs.iter().filter(|f| f["namespace"] == namespace).flat_map(|f| strings(&f["tags"])).collect()).unwrap_or_default();
    if patterns.is_empty() {
        return true;
    }
    let matches = |p: &str| resource_tags.iter().any(|t| glob(p, t));
    let (negated, wanted): (Vec<&str>, Vec<&str>) = patterns.into_iter().partition(|p| p.starts_with('!'));
    if negated.iter().any(|p| matches(&p[1..])) {
        return false;
    }
    wanted.is_empty() || wanted.iter().any(|p| matches(p))
}

/// The resource a metric is about, as the Tagging API names it: (namespace, ARN format with `{p}` partition,
/// `{r}` region, `{a}` account and `{<Dimension>}` the dimensions' values). The first whose dimensions the
/// metric has is used.
const RESOURCES: [(&str, &str); 16] = [
    ("AWS/EC2", "arn:{p}:ec2:{r}:{a}:instance/{InstanceId}"),
    ("AWS/Lambda", "arn:{p}:lambda:{r}:{a}:function:{FunctionName}"),
    ("AWS/RDS", "arn:{p}:rds:{r}:{a}:db:{DBInstanceIdentifier}"),
    ("AWS/RDS", "arn:{p}:rds:{r}:{a}:cluster:{DBClusterIdentifier}"),
    ("AWS/DynamoDB", "arn:{p}:dynamodb:{r}:{a}:table/{TableName}"),
    ("AWS/SQS", "arn:{p}:sqs:{r}:{a}:{QueueName}"),
    ("AWS/SNS", "arn:{p}:sns:{r}:{a}:{TopicName}"),
    ("AWS/ApplicationELB", "arn:{p}:elasticloadbalancing:{r}:{a}:loadbalancer/{LoadBalancer}"),
    ("AWS/NetworkELB", "arn:{p}:elasticloadbalancing:{r}:{a}:loadbalancer/{LoadBalancer}"),
    ("AWS/S3", "arn:{p}:s3:::{BucketName}"),
    ("AWS/ECS", "arn:{p}:ecs:{r}:{a}:service/{ClusterName}/{ServiceName}"),
    ("AWS/ECS", "arn:{p}:ecs:{r}:{a}:cluster/{ClusterName}"),
    ("ECS/ContainerInsights", "arn:{p}:ecs:{r}:{a}:service/{ClusterName}/{ServiceName}"),
    ("ECS/ContainerInsights", "arn:{p}:ecs:{r}:{a}:cluster/{ClusterName}"),
    ("AWS/ElastiCache", "arn:{p}:elasticache:{r}:{a}:cluster:{CacheClusterId}"),
    ("AWS/CloudFront", "arn:{p}:cloudfront::{a}:distribution/{DistributionId}"),
];

/// The ARN of the resource a metric's dimensions name, for the namespaces we map.
pub fn resource_arn(partition: &str, region: &str, account: &str, namespace: &str, dimensions: &[(String, String)]) -> Option<String> {
    RESOURCES.iter().filter(|(ns, _)| *ns == namespace).find_map(|(_, format)| {
        let mut arn = format.replace("{p}", partition).replace("{r}", region).replace("{a}", account);
        while let Some(start) = arn.find('{') {
            let end = start + arn[start..].find('}')?;
            let value = dimensions.iter().find(|(k, _)| *k == arn[start + 1..end]).map(|(_, v)| v.clone())?;
            arn.replace_range(start..=end, &value);
        }
        Some(arn)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn names_follow_datadogs_aws_metrics() {
        assert_eq!(metric_name("AWS/EC2", "CPUUtilization"), "aws.ec2.cpuutilization");
        assert_eq!(metric_name("AWS/EC2", "NetworkIn"), "aws.ec2.network_in");
        assert_eq!(metric_name("AWS/EC2", "StatusCheckFailed_System"), "aws.ec2.status_check_failed_system");
        assert_eq!(metric_name("AWS/ApplicationELB", "HTTPCode_Target_5XX_Count"), "aws.applicationelb.httpcode_target_5xx_count");
        assert_eq!(metric_name("AWS/SQS", "ApproximateNumberOfMessagesVisible"), "aws.sqs.approximate_number_of_messages_visible");
        assert_eq!(metric_name("AWS/RDS/Proxy", "ClientConnections"), "aws.rds.proxy.client_connections");
        assert_eq!(metric_name("ECS/ContainerInsights", "RunningTaskCount"), "ecs.containerinsights.running_task_count");
        assert_eq!(metric_name("AWS/CloudFront", "4xxErrorRate"), "aws.cloudfront.4xx_error_rate");
        assert_eq!(metric_name("AWS/CloudFront", "401ErrorRate"), "aws.cloudfront.401_error_rate");
        assert_eq!(metric_name("AWS/WAFV2", "AllowedRequests"), "aws.wafv2.allowed_requests");
        assert_eq!(metric_name("WAF", "BlockedRequests"), "aws.waf.blocked_requests");
        assert_eq!(metric_name("AWS/RDS", "DBLoadNonCPU"), "aws.rds.dbload_non_cpu");
        assert_eq!(metric_name("AWS/RDS", "VolumeReadIOPs"), "aws.rds.volume_read_iops");
        assert_eq!(metric_name("/aws/sagemaker/Endpoints", "CPUUtilization"), "aws.sagemaker_endpoints.cpuutilization");
        assert_eq!(metric_name("Acme/Checkout", "Orders Placed"), "aws.acme_checkout.orders_placed");
    }

    #[test]
    fn dimension_tags_are_lowercased() {
        assert_eq!(tag_key("FunctionName"), "functionname");
        assert_eq!(tag_key("DBInstanceIdentifier"), "dbinstanceidentifier");
        assert_eq!(tag_key("Availability Zone"), "availability_zone");
    }

    #[test]
    fn namespace_filters_include_or_exclude_and_custom_namespaces_need_opting_in() {
        let exclude = json!({ "exclude_only": ["AWS/SQS"] });
        assert!(namespace_selected(&exclude, "AWS/EC2", true, false));
        assert!(!namespace_selected(&exclude, "AWS/SQS", true, false));
        assert!(!namespace_selected(&exclude, "Acme/Checkout", false, false));
        assert!(namespace_selected(&exclude, "Acme/Checkout", false, true));
        let include = json!({ "include_only": ["AWS/EC2"] });
        assert!(namespace_selected(&include, "AWS/EC2", true, false));
        assert!(!namespace_selected(&include, "AWS/RDS", true, false));
    }

    #[test]
    fn metric_name_filters_apply_to_their_namespace_with_wildcards() {
        let filters = json!([{ "namespace": "AWS/EC2", "include_only": ["aws.ec2.network_*"] }, { "namespace": "AWS/RDS", "exclude_only": ["aws.rds.burst_balance"] }]);
        assert!(metric_selected(&filters, "AWS/EC2", "aws.ec2.network_in"));
        assert!(!metric_selected(&filters, "AWS/EC2", "aws.ec2.cpuutilization"));
        assert!(!metric_selected(&filters, "AWS/RDS", "aws.rds.burst_balance"));
        assert!(metric_selected(&filters, "AWS/RDS", "aws.rds.free_storage_space"));
        assert!(metric_selected(&filters, "AWS/Lambda", "aws.lambda.errors"));
        assert!(metric_selected(&json!(null), "AWS/EC2", "aws.ec2.cpuutilization"));
    }

    #[test]
    fn tag_filters_keep_the_resources_with_a_matching_tag_and_negations_exclude() {
        let filters = json!([{ "namespace": "AWS/EC2", "tags": ["env:prod", "team:storefront*", "!datadog:no"] }]);
        let tags = |t: &[&str]| t.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert!(tags_match(&filters, "AWS/EC2", &tags(&["env:prod"])));
        assert!(tags_match(&filters, "AWS/EC2", &tags(&["team:storefront-api"])));
        assert!(!tags_match(&filters, "AWS/EC2", &tags(&["env:staging"])));
        assert!(!tags_match(&filters, "AWS/EC2", &tags(&["env:prod", "datadog:no"])));
        // No filter for the namespace: everything.
        assert!(tags_match(&filters, "AWS/RDS", &tags(&[])));
        assert!(tags_match(&json!([]), "AWS/EC2", &tags(&[])));
    }

    #[test]
    fn dimensions_name_the_resource_whose_tags_the_metrics_get() {
        let d = |k: &str, v: &str| vec![(k.to_string(), v.to_string())];
        let arn = |ns: &str, dims: Vec<(String, String)>| resource_arn("aws", "us-east-1", "123456789012", ns, &dims);
        assert_eq!(arn("AWS/EC2", d("InstanceId", "i-0a1b2c3d4e5f60001")).as_deref(), Some("arn:aws:ec2:us-east-1:123456789012:instance/i-0a1b2c3d4e5f60001"));
        assert_eq!(arn("AWS/Lambda", d("FunctionName", "checkout")).as_deref(), Some("arn:aws:lambda:us-east-1:123456789012:function:checkout"));
        assert_eq!(arn("AWS/RDS", d("DBInstanceIdentifier", "orders-db")).as_deref(), Some("arn:aws:rds:us-east-1:123456789012:db:orders-db"));
        assert_eq!(arn("AWS/DynamoDB", d("TableName", "carts")).as_deref(), Some("arn:aws:dynamodb:us-east-1:123456789012:table/carts"));
        assert_eq!(arn("AWS/SQS", d("QueueName", "orders")).as_deref(), Some("arn:aws:sqs:us-east-1:123456789012:orders"));
        assert_eq!(arn("AWS/ApplicationELB", d("LoadBalancer", "app/storefront/50dc6c495c0c9188")).as_deref(), Some("arn:aws:elasticloadbalancing:us-east-1:123456789012:loadbalancer/app/storefront/50dc6c495c0c9188"));
        assert_eq!(arn("AWS/S3", d("BucketName", "example-assets")).as_deref(), Some("arn:aws:s3:::example-assets"));
        assert_eq!(arn("AWS/RDS", d("DBClusterIdentifier", "orders")).as_deref(), Some("arn:aws:rds:us-east-1:123456789012:cluster:orders"));
        assert_eq!(arn("AWS/CloudFront", d("DistributionId", "E1A2B3C4D5E6F7")).as_deref(), Some("arn:aws:cloudfront::123456789012:distribution/E1A2B3C4D5E6F7"));
        let service = vec![("ClusterName".to_string(), "storefront".to_string()), ("ServiceName".to_string(), "order-service".to_string())];
        assert_eq!(arn("AWS/ECS", service.clone()).as_deref(), Some("arn:aws:ecs:us-east-1:123456789012:service/storefront/order-service"));
        assert_eq!(arn("ECS/ContainerInsights", d("ClusterName", "storefront")).as_deref(), Some("arn:aws:ecs:us-east-1:123456789012:cluster/storefront"));
        assert_eq!(resource_arn("aws-cn", "cn-north-1", "123456789012", "AWS/EC2", &d("InstanceId", "i-1")).as_deref(), Some("arn:aws-cn:ec2:cn-north-1:123456789012:instance/i-1"));
        assert_eq!(arn("AWS/EC2", d("AutoScalingGroupName", "web")), None);
        assert_eq!(arn("Acme/Checkout", d("Store", "eu")), None);
    }

    fn dims(d: &[(&str, &str)]) -> Vec<(String, String)> {
        d.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    #[test]
    fn the_bare_name_carries_datadogs_statistic_and_ecs_series_get_their_variants() {
        let none = dims(&[]);
        // Counts are sums under their bare name (aws.cloudfront.requests, aws.wafv2.blocked_requests)…
        assert_eq!(series_names("AWS/CloudFront", "Requests", &none, "Sum"), ["aws.cloudfront.requests", "aws.cloudfront.requests.sum"]);
        assert!(series_names("AWS/CloudFront", "Requests", &none, "Average").is_empty());
        assert_eq!(series_names("AWS/WAFV2", "BlockedRequests", &none, "Sum"), ["aws.wafv2.blocked_requests", "aws.wafv2.blocked_requests.sum"]);
        // …gauges averages, and namespaces we don't follow Datadog in keep the average.
        assert_eq!(series_names("AWS/RDS", "DatabaseConnections", &none, "Average"), ["aws.rds.database_connections"]);
        assert_eq!(series_names("AWS/RDS", "DatabaseConnections", &none, "Maximum"), ["aws.rds.database_connections.maximum"]);
        assert_eq!(series_names("AWS/Lambda", "Invocations", &none, "Average"), ["aws.lambda.invocations"]);
        assert_eq!(series_names("AWS/Lambda", "Invocations", &none, "Sum"), ["aws.lambda.invocations.sum"]);
        let cluster = dims(&[("ClusterName", "storefront")]);
        let service = dims(&[("ClusterName", "storefront"), ("ServiceName", "order-service")]);
        assert_eq!(series_names("AWS/ECS", "CPUUtilization", &cluster, "Average"), ["aws.ecs.cpuutilization", "aws.ecs.cluster.cpuutilization"]);
        assert_eq!(series_names("AWS/ECS", "CPUUtilization", &service, "Maximum"), ["aws.ecs.cpuutilization.maximum", "aws.ecs.service.cpuutilization.maximum"]);
        let family = dims(&[("ClusterName", "storefront"), ("TaskDefinitionFamily", "order-service")]);
        assert_eq!(series_names("ECS/ContainerInsights", "CpuUtilized", &family, "Average"), ["ecs.containerinsights.family.cpu_utilized"]);
        assert_eq!(series_names("ECS/ContainerInsights", "RunningTaskCount", &service, "Average"), ["ecs.containerinsights.running_task_count"]);
    }

    /// The service dashboards (`scripts/cloud-service-dashboards.mjs`) of the namespaces that follow Datadog chart
    /// only names this crawler publishes for the metrics' dimension sets, split and filtered by their dimensions' tags.
    #[test]
    fn the_datadog_namespaces_dashboards_chart_what_the_crawler_publishes() {
        use std::collections::{BTreeSet, HashSet};
        use std::io::Read;
        let doc: Value = serde_json::from_str(include_str!("../../../../seeds/integrations/cloud/aws-metrics.json")).unwrap();
        let catalog: Value = serde_json::from_str(include_str!("../../../../seeds/aws_integration.json")).unwrap();
        let mut text = String::new();
        flate2::read::GzDecoder::new(&include_bytes!("../../../../seeds/integrations/cloud/services.ndjson.gz")[..]).read_to_string(&mut text).unwrap();
        let query = regex::Regex::new(r#"[a-z]+:([a-z0-9_.]+)\{([^}]*)\}(?: by \{([a-z0-9_,]+)\})?"#).unwrap();
        let base: HashSet<&str> = HashSet::from(["aws_account", "region"]);
        let mut checked = BTreeSet::new();
        for (ns, spec) in doc["namespaces"].as_object().unwrap().iter().filter(|(_, s)| s["datadog"] == true) {
            // Every name and tag the crawler gives the namespace's documented series.
            let (mut names, mut tags) = (HashSet::new(), HashSet::new());
            for m in spec["metrics"].as_array().unwrap() {
                let d: Vec<(String, String)> = m["dimensions"].as_array().unwrap().iter().map(|k| (k.as_str().unwrap().to_string(), "x".to_string())).collect();
                for (stat, _) in STATS {
                    names.extend(series_names(ns, m["name"].as_str().unwrap(), &d, stat));
                }
                tags.extend(d.iter().map(|(k, _)| tag_key(k)));
            }
            let id = catalog["namespaces"].as_array().unwrap().iter().find(|n| n["namespace"] == ns.as_str()).unwrap()["id"].as_str().unwrap();
            let line: Value = text.lines().map(|l| serde_json::from_str::<Value>(l).unwrap()).find(|l| l["logo"] == format!("tile:{id}")).unwrap_or_else(|| panic!("{ns}: no dashboard"));
            assert_eq!(line["prefix"].as_str().unwrap(), format!("{}.", metric_prefix(ns)), "{ns}: installed by its metrics' prefix");
            let widgets = line["dashboards"][0]["widgets"].to_string();
            let mut charted = 0;
            for c in query.captures_iter(&widgets) {
                let name = &c[1];
                assert!(names.contains(name), "{ns}: {name} isn't published by the crawler");
                for f in c[2].split(',').filter(|f| !f.starts_with('$')) {
                    let key = f.trim_start_matches('!').split(':').next().unwrap();
                    assert!(tags.contains(key) || base.contains(key), "{ns}: {name} filters on {key}, not a tag of its series");
                }
                for t in c.get(3).map(|g| g.as_str()).unwrap_or_default().split(',').filter(|t| !t.is_empty()) {
                    assert!(tags.contains(t), "{ns}: {name} splits by {t}, not a tag of its series");
                }
                charted += 1;
            }
            assert!(charted >= 5, "{ns}: {charted} queries");
            checked.insert(ns.as_str());
        }
        assert_eq!(checked, BTreeSet::from(["AWS/CloudFront", "AWS/ECS", "AWS/RDS", "AWS/RDS/Proxy", "AWS/WAFV2", "ECS/ContainerInsights", "WAF"]));
    }

    #[test]
    fn rds_aggregates_are_left_out_and_global_services_are_known() {
        assert!(series_collected("AWS/RDS", &dims(&[("DBInstanceIdentifier", "orders-db")])));
        assert!(series_collected("AWS/RDS", &dims(&[("DBClusterIdentifier", "orders"), ("Role", "WRITER")])));
        assert!(!series_collected("AWS/RDS", &dims(&[("EngineName", "aurora-postgresql")])));
        assert!(!series_collected("AWS/RDS", &dims(&[])));
        assert!(series_collected("AWS/EC2", &dims(&[])));
        assert!(global_series("AWS/CloudFront", &dims(&[("DistributionId", "E1A2B3C4D5E6F7"), ("Region", "Global")])));
        assert!(global_series("AWS/WAFV2", &dims(&[("WebACL", "storefront-cdn"), ("Rule", "ALL")])));
        assert!(!global_series("AWS/WAFV2", &dims(&[("Region", "eu-west-1"), ("WebACL", "storefront-alb"), ("Rule", "ALL")])));
        assert!(!global_series("AWS/EC2", &dims(&[])));
    }
}
