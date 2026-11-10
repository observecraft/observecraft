//! How CloudWatch metrics become ObserveCraft metrics, Datadog-compatible: names, tags, the
//! account's filters, and the OTLP JSON the crawler publishes.

use std::collections::HashMap;
use std::sync::OnceLock;

use serde_json::Value;

pub use crate::cloud::naming::Point;
use crate::cloud::naming::{glob, snake, underscored};

/// The CloudWatch statistics every metric is read with, and the suffix of their names. The metric's own statistic
/// (its Datadog one: counts' Sum, gauges' Average, `aws-metrics.json`) is also, or only for the Average, its bare name.
pub const STATS: [(&str, &str); 4] = [("Average", ""), ("Sum", ".sum"), ("Maximum", ".maximum"), ("Minimum", ".minimum")];

/// The period, in seconds, of the metrics CloudWatch publishes at a finer resolution than a day.
pub const MINUTE: i32 = 60;
/// Daily metrics (S3's storage metrics, Storage Lens): one point a day, published for the previous day.
pub const DAY: i32 = 86_400;

/// Namespaces Datadog doesn't name `aws.<namespace>.`: RDS Proxy is `aws.rds.proxy.*`, ECS Container
/// Insights `ecs.containerinsights.*`.
const PREFIXES: [(&str, &str); 2] = [("AWS/RDS/Proxy", "aws.rds.proxy"), ("ECS/ContainerInsights", "ecs.containerinsights")];

/// A metric of `seeds/integrations/cloud/aws-metrics.json` as Datadog publishes it (`scripts/aws-datadog-metrics.json`
/// for the namespaces marked `datadog` and the overrides): the statistic of its bare name, its name where it isn't the
/// snake-cased CloudWatch one (`401ErrorRate` → `401_error_rate`), its extended statistics (`p95`) and its period.
#[derive(Debug, Clone, PartialEq)]
pub struct MetricSpec {
    pub statistic: Option<String>,
    pub name: Option<String>,
    pub percentiles: Vec<String>,
    pub period: i32,
}

pub(crate) fn catalog() -> &'static HashMap<(String, String), MetricSpec> {
    static M: OnceLock<HashMap<(String, String), MetricSpec>> = OnceLock::new();
    M.get_or_init(|| {
        let doc: Value = serde_json::from_str(include_str!("../../../../seeds/integrations/cloud/aws-metrics.json")).expect("aws-metrics.json is valid");
        let mut out = HashMap::new();
        for (ns, spec) in doc["namespaces"].as_object().into_iter().flatten() {
            let ns_period = spec["period"].as_i64().map_or(MINUTE, |p| p as i32);
            for m in spec["metrics"].as_array().into_iter().flatten() {
                let Some(name) = m["name"].as_str() else { continue };
                out.entry((ns.clone(), name.to_string())).or_insert_with(|| MetricSpec {
                    statistic: m["statistic"].as_str().map(str::to_string),
                    name: m["datadog_name"].as_str().map(str::to_string),
                    percentiles: strings(&m["percentiles"]).into_iter().map(str::to_string).collect(),
                    period: m["period"].as_i64().map_or(ns_period, |p| p as i32),
                });
            }
        }
        out
    })
}

/// Datadog's statistic for a metric outside our catalog: counters (counts, requests, errors, bytes, invocations,
/// throttles; not host counts, which are gauges) are sums, the rest averages.
fn conventional_statistic(metric: &str) -> &'static str {
    let snaked = snake(metric);
    let words: Vec<&str> = snaked.split('_').collect();
    let counter = words.iter().any(|w| matches!(*w, "count" | "requests" | "errors" | "bytes" | "invocations" | "throttles"));
    if counter && !snaked.ends_with("host_count") {
        "Sum"
    } else {
        "Average"
    }
}

/// The statistic a metric's bare name carries: the catalog's (Datadog's), else Datadog's convention.
pub fn default_statistic(namespace: &str, metric: &str) -> &'static str {
    match catalog().get(&(namespace.to_string(), metric.to_string())).and_then(|m| m.statistic.as_deref()) {
        Some(s) => s,
        None => conventional_statistic(metric),
    }
}

/// The statistics a metric is read with: its own, then Sum, Maximum and Minimum (the Average only when it is the
/// metric's: a count's average names nothing) and its percentiles.
pub fn statistics(namespace: &str, metric: &str) -> Vec<String> {
    let own = default_statistic(namespace, metric);
    let mut out = vec![own.to_string()];
    out.extend(STATS.iter().map(|(s, _)| s.to_string()).filter(|s| s != own && s != "Average"));
    if let Some(spec) = catalog().get(&(namespace.to_string(), metric.to_string())) {
        out.extend(spec.percentiles.iter().cloned());
    }
    out
}

/// The suffix of a statistic's name: `.sum`, `.maximum`, `.minimum`, `.samplecount`, `.p95`.
fn suffix(statistic: &str) -> Option<String> {
    match STATS.iter().find(|(s, _)| *s == statistic) {
        Some((_, s)) => (!s.is_empty()).then(|| s.to_string()),
        None if statistic.starts_with('p') => Some(format!(".{statistic}")),
        None => Some(format!(".{}", statistic.to_ascii_lowercase())),
    }
}

/// A metric's period in seconds: [`DAY`] for daily metrics, else [`MINUTE`].
pub fn period(namespace: &str, metric: &str) -> i32 {
    catalog().get(&(namespace.to_string(), metric.to_string())).map_or(MINUTE, |m| m.period)
}

/// The namespaces with daily metrics: listed without ListMetrics' "recently active" filter (a daily metric
/// may have no point in the last 3 hours).
pub fn daily_namespaces() -> Vec<&'static str> {
    let mut out: Vec<&str> = catalog().iter().filter(|(_, m)| m.period == DAY).map(|((ns, _), _)| ns.as_str()).collect();
    out.sort();
    out.dedup();
    out
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
    catalog().get(&(namespace.to_string(), metric.to_string())).and_then(|m| m.name.clone()).unwrap_or_else(|| snake(metric))
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
/// metric's own statistic, and the statistic's suffixed name (`.sum`, `.maximum`, `.minimum`, `.p95`).
pub fn series_names(namespace: &str, metric: &str, dimensions: &[(String, String)], statistic: &str) -> Vec<String> {
    let mut suffixes = vec![];
    if statistic == default_statistic(namespace, metric) {
        suffixes.push(String::new());
    }
    suffixes.extend(suffix(statistic));
    let (prefix, leaf) = (metric_prefix(namespace), leaf(namespace, metric));
    let (bare, extra) = variant(namespace, dimensions);
    let bases: Vec<String> = bare.then(|| format!("{prefix}.{leaf}")).into_iter().chain(extra.map(|v| format!("{prefix}.{v}.{leaf}"))).collect();
    bases.iter().flat_map(|b| suffixes.iter().map(move |s| format!("{b}{s}"))).collect()
}

/// Whether a series is collected: the account-wide aggregates of RDS (by engine, by instance class, or none) and
/// EC2 (by image, instance type, Auto Scaling group, or none) are left out, as Datadog has RDS metrics per instance
/// and per cluster and EC2 metrics per instance (they would double the sums).
pub fn series_collected(namespace: &str, dimensions: &[(String, String)]) -> bool {
    let has = |d: &str| dimensions.iter().any(|(k, _)| k == d);
    match namespace {
        "AWS/RDS" => has("DBInstanceIdentifier") || has("DBClusterIdentifier"),
        "AWS/EC2" => has("InstanceId"),
        _ => true,
    }
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

/// The namespace a series is named and tagged by: CloudWatch has RDS Proxy's metrics in `AWS/RDS` (by `ProxyName`),
/// which Datadog names `aws.rds.proxy.*` (our catalog's `AWS/RDS/Proxy`).
pub fn series_namespace<'a>(namespace: &'a str, dimensions: &[(String, String)]) -> &'a str {
    if namespace == "AWS/RDS" && dimensions.iter().any(|(k, _)| k == "ProxyName") { "AWS/RDS/Proxy" } else { namespace }
}

/// The CloudWatch namespace a [`series_namespace`] is read from.
pub fn cloudwatch_namespace(namespace: &str) -> &str {
    if namespace == "AWS/RDS/Proxy" { "AWS/RDS" } else { namespace }
}

/// The lookups some ARNs need, by what the metrics' dimensions carry (`{<Dimension>@<lookup>}` in [`RESOURCES`]):
/// REST APIs' ids by name (`apigateway:GET /restapis`), RDS proxies' ARNs by name (`rds:DescribeDBProxies`), DMS
/// replication instances' ARNs by identifier (`dms:DescribeReplicationInstances`), and WAF Classic web ACLs' ARNs
/// by metric name (`waf:` / `waf-regional:ListWebACLs` and `GetWebACL`; CloudFront's are global). Read once a run.
pub const LOOKUPS: [&str; 5] = ["restapis", "dbproxies", "replicationinstances", "webacls", "regionalwebacls"];

/// [`LOOKUPS`]' answers: lookup → name → id or ARN.
pub type Lookups = HashMap<String, HashMap<String, String>>;

/// The resource a metric is about, as the Tagging API names it: (namespace, ARN format with `{p}` partition,
/// `{r}` region, `{a}` account and `{<Dimension>}` the dimensions' values, `{<Dimension>:lower}` lowercased,
/// `{<Dimension>@<lookup>}` what a [`LOOKUPS`] answers for the value). The first whose dimensions the metric has
/// is used. A `*` stands for what the dimensions don't carry (the id after a web ACL's or an MSK cluster's name):
/// the resource is the tagged one the pattern matches.
const RESOURCES: &[(&str, &str)] = &[
    ("AWS/EC2", "arn:{p}:ec2:{r}:{a}:instance/{InstanceId}"),
    ("CWAgent", "arn:{p}:ec2:{r}:{a}:instance/{InstanceId}"),
    ("AWS/EBS", "arn:{p}:ec2:{r}:{a}:volume/{VolumeId}"),
    ("AWS/NATGateway", "arn:{p}:ec2:{r}:{a}:natgateway/{NatGatewayId}"),
    ("AWS/TransitGateway", "arn:{p}:ec2:{r}:{a}:transit-gateway/{TransitGateway}"),
    ("AWS/VPN", "arn:{p}:ec2:{r}:{a}:vpn-connection/{VpnId}"),
    ("AWS/PrivateLinkEndpoints", "arn:{p}:ec2:{r}:{a}:vpc-endpoint/{VPC Endpoint Id}"),
    ("AWS/EC2CapacityReservations", "arn:{p}:ec2:{r}:{a}:capacity-reservation/{CapacityReservationId}"),
    ("AWS/AutoScaling", "arn:{p}:autoscaling:{r}:{a}:autoScalingGroup:*:autoScalingGroupName/{AutoScalingGroupName}"),
    ("AWS/Lambda", "arn:{p}:lambda:{r}:{a}:function:{FunctionName}"),
    ("AWS/RDS", "arn:{p}:rds:{r}:{a}:db:{DBInstanceIdentifier}"),
    ("AWS/RDS", "arn:{p}:rds:{r}:{a}:cluster:{DBClusterIdentifier}"),
    ("AWS/DocDB", "arn:{p}:rds:{r}:{a}:db:{DBInstanceIdentifier}"),
    ("AWS/DocDB", "arn:{p}:rds:{r}:{a}:cluster:{DBClusterIdentifier}"),
    ("AWS/Neptune", "arn:{p}:rds:{r}:{a}:db:{DBInstanceIdentifier}"),
    ("AWS/Neptune", "arn:{p}:rds:{r}:{a}:cluster:{DBClusterIdentifier}"),
    ("AWS/DynamoDB", "arn:{p}:dynamodb:{r}:{a}:table/{TableName}"),
    ("AWS/DAX", "arn:{p}:dax:{r}:{a}:cache/{ClusterId}"),
    ("AWS/SQS", "arn:{p}:sqs:{r}:{a}:{QueueName}"),
    ("AWS/SNS", "arn:{p}:sns:{r}:{a}:{TopicName}"),
    ("AWS/ApiGateway", "arn:{p}:apigateway:{r}::/apis/{ApiId}"),
    ("AWS/ApiGateway", "arn:{p}:apigateway:{r}::/restapis/{ApiName@restapis}"),
    ("AWS/RDS/Proxy", "{ProxyName@dbproxies}"),
    ("AWS/DMS", "arn:{p}:dms:{r}:{a}:rep:{ReplicationInstanceExternalResourceId}"),
    ("AWS/DMS", "{ReplicationInstanceIdentifier@replicationinstances}"),
    ("AWS/AppSync", "arn:{p}:appsync:{r}:{a}:apis/{GraphQLAPIId}"),
    ("AWS/ApplicationELB", "arn:{p}:elasticloadbalancing:{r}:{a}:loadbalancer/{LoadBalancer}"),
    ("AWS/ApplicationELB", "arn:{p}:elasticloadbalancing:{r}:{a}:{TargetGroup}"),
    ("AWS/NetworkELB", "arn:{p}:elasticloadbalancing:{r}:{a}:loadbalancer/{LoadBalancer}"),
    ("AWS/NetworkELB", "arn:{p}:elasticloadbalancing:{r}:{a}:{TargetGroup}"),
    ("AWS/GatewayELB", "arn:{p}:elasticloadbalancing:{r}:{a}:loadbalancer/{LoadBalancer}"),
    ("AWS/GatewayELB", "arn:{p}:elasticloadbalancing:{r}:{a}:{TargetGroup}"),
    ("AWS/ELB", "arn:{p}:elasticloadbalancing:{r}:{a}:loadbalancer/{LoadBalancerName}"),
    ("AWS/S3", "arn:{p}:s3:::{BucketName}"),
    ("AWS/ECS", "arn:{p}:ecs:{r}:{a}:service/{ClusterName}/{ServiceName}"),
    ("AWS/ECS", "arn:{p}:ecs:{r}:{a}:cluster/{ClusterName}"),
    ("ECS/ContainerInsights", "arn:{p}:ecs:{r}:{a}:service/{ClusterName}/{ServiceName}"),
    ("ECS/ContainerInsights", "arn:{p}:ecs:{r}:{a}:cluster/{ClusterName}"),
    ("AWS/EKS", "arn:{p}:eks:{r}:{a}:cluster/{ClusterName}"),
    ("AWS/ECR", "arn:{p}:ecr:{r}:{a}:repository/{RepositoryName}"),
    ("AWS/AppRunner", "arn:{p}:apprunner:{r}:{a}:service/{ServiceName}/*"),
    ("AWS/ElasticBeanstalk", "arn:{p}:elasticbeanstalk:{r}:{a}:environment/*/{EnvironmentName}"),
    ("AWS/ElastiCache", "arn:{p}:elasticache:{r}:{a}:cluster:{CacheClusterId}"),
    ("AWS/ElastiCache", "arn:{p}:elasticache:{r}:{a}:replicationgroup:{ReplicationGroupId}"),
    ("AWS/ElastiCache", "arn:{p}:elasticache:{r}:{a}:serverlesscache:{clusterId}"),
    ("AWS/MemoryDB", "arn:{p}:memorydb:{r}:{a}:cluster/{ClusterName}"),
    ("AWS/EFS", "arn:{p}:elasticfilesystem:{r}:{a}:file-system/{FileSystemId}"),
    ("AWS/FSx", "arn:{p}:fsx:{r}:{a}:file-system/{FileSystemId}"),
    ("AWS/Kinesis", "arn:{p}:kinesis:{r}:{a}:stream/{StreamName}"),
    ("AWS/KinesisVideo", "arn:{p}:kinesisvideo:{r}:{a}:stream/{StreamName}/*"),
    ("AWS/Firehose", "arn:{p}:firehose:{r}:{a}:deliverystream/{DeliveryStreamName}"),
    ("AWS/States", "{StateMachineArn}"),
    ("AWS/States", "{ActivityArn}"),
    ("AWS/Events", "arn:{p}:events:{r}:{a}:rule/{RuleName}"),
    ("AWS/EventBridge/Pipes", "arn:{p}:pipes:{r}:{a}:pipe/{PipeName}"),
    ("AWS/Logs", "arn:{p}:logs:{r}:{a}:log-group:{LogGroupName}"),
    ("AWS/CloudFront", "arn:{p}:cloudfront::{a}:distribution/{DistributionId}"),
    ("AWS/Route53", "arn:{p}:route53:::healthcheck/{HealthCheckId}"),
    ("AWS/Route53", "arn:{p}:route53:::hostedzone/{HostedZoneId}"),
    ("AWS/Route53Resolver", "arn:{p}:route53resolver:{r}:{a}:resolver-endpoint/{EndpointId}"),
    ("AWS/WAFV2", "arn:{p}:wafv2:{r}:{a}:regional/webacl/{WebACL}/*"),
    ("WAF", "{WebACL@regionalwebacls}"),
    ("AWS/Redshift", "arn:{p}:redshift:{r}:{a}:cluster:{ClusterIdentifier}"),
    ("AWS/ES", "arn:{p}:es:{r}:{a}:domain/{DomainName}"),
    ("AWS/Kafka", "arn:{p}:kafka:{r}:{a}:cluster/{Cluster Name}/*"),
    ("AWS/AmazonMQ", "arn:{p}:mq:{r}:{a}:broker:{Broker}:*"),
    ("AWS/SageMaker", "arn:{p}:sagemaker:{r}:{a}:endpoint/{EndpointName:lower}"),
    ("/aws/sagemaker/Endpoints", "arn:{p}:sagemaker:{r}:{a}:endpoint/{EndpointName:lower}"),
    ("AWS/CodeBuild", "arn:{p}:codebuild:{r}:{a}:project/{ProjectName}"),
    ("AWS/Athena", "arn:{p}:athena:{r}:{a}:workgroup/{WorkGroup}"),
    ("AWS/Cognito", "arn:{p}:cognito-idp:{r}:{a}:userpool/{UserPool}"),
    ("AWS/Cognito", "arn:{p}:cognito-idp:{r}:{a}:userpool/{UserPoolId}"),
    ("AWS/KMS", "arn:{p}:kms:{r}:{a}:key/{KeyId}"),
    ("AWS/CertificateManager", "{CertificateArn}"),
    ("AWS/ElasticMapReduce", "arn:{p}:elasticmapreduce:{r}:{a}:cluster/{JobFlowId}"),
    ("AWS/NetworkFirewall", "arn:{p}:network-firewall:{r}:{a}:firewall/{FirewallName}"),
    ("AWS/Transfer", "arn:{p}:transfer:{r}:{a}:server/{ServerId}"),
    ("AWS/WorkSpaces", "arn:{p}:workspaces:{r}:{a}:workspace/{WorkspaceId}"),
    ("AWS/DX", "arn:{p}:directconnect:{r}:{a}:dxcon/{ConnectionId}"),
    ("AWS/StorageGateway", "arn:{p}:storagegateway:{r}:{a}:gateway/{GatewayId}"),
    ("AWS/QLDB", "arn:{p}:qldb:{r}:{a}:ledger/{LedgerName}"),
    ("AWS/CloudHSM", "arn:{p}:cloudhsm:{r}:{a}:cluster/{ClusterId}"),
    ("AWS/Backup", "arn:{p}:backup:{r}:{a}:backup-vault:{BackupVaultName}"),
    ("AWS/DataSync", "arn:{p}:datasync:{r}:{a}:task/{TaskId}"),
    ("AWS/GameLift", "arn:{p}:gamelift:{r}:{a}:fleet/{FleetId}"),
    ("AWS/KinesisAnalytics", "arn:{p}:kinesisanalytics:{r}:{a}:application/{Application}"),
    ("AWS/MediaConvert", "arn:{p}:mediaconvert:{r}:{a}:queues/{Queue}"),
    ("AWS/Scheduler", "arn:{p}:scheduler:{r}:{a}:schedule-group/{ScheduleGroup}"),
    ("CloudWatchSynthetics", "arn:{p}:synthetics:{r}:{a}:canary:{CanaryName}"),
    ("AmazonMWAA", "arn:{p}:airflow:{r}:{a}:environment/{Environment}"),
    ("Glue", "arn:{p}:glue:{r}:{a}:job/{JobName}"),
];

/// The web ACLs of CloudFront distributions are global, the others regional.
fn webacl_scope(namespace: &str, dimensions: &[(String, String)]) -> Option<&'static str> {
    if !global_series(namespace, dimensions) {
        return None;
    }
    match namespace {
        "AWS/WAFV2" => Some("arn:{p}:wafv2:{r}:{a}:global/webacl/{WebACL}/*"),
        "WAF" => Some("{WebACL@webacls}"),
        _ => None,
    }
}

fn formats<'a>(namespace: &'a str, dimensions: &[(String, String)]) -> impl Iterator<Item = &'static str> + 'a {
    webacl_scope(namespace, dimensions).into_iter().chain(RESOURCES.iter().filter(move |(ns, _)| *ns == namespace).map(|(_, f)| *f))
}

/// A format's placeholders (`{InstanceId}`, `{EndpointName:lower}`, `{ApiName@restapis}`) as (dimension, lowercased,
/// lookup), but `{p}`, `{r}` and `{a}`.
fn placeholders(format: &str) -> Vec<(&str, bool, Option<&str>)> {
    let mut out = vec![];
    let mut rest = format;
    while let Some(start) = rest.find('{') {
        let Some(len) = rest[start..].find('}') else { break };
        let inner = &rest[start + 1..start + len];
        rest = &rest[start + len + 1..];
        if matches!(inner, "p" | "r" | "a") {
            continue;
        }
        out.push(match (inner.strip_suffix(":lower"), inner.split_once('@')) {
            (Some(key), _) => (key, true, None),
            (None, Some((key, lookup))) => (key, false, Some(lookup)),
            (None, None) => (inner, false, None),
        });
    }
    out
}

/// The [`LOOKUPS`] a series' ARN needs: that of the first format whose dimensions it has.
pub fn lookup(namespace: &str, dimensions: &[(String, String)]) -> Option<&'static str> {
    let has = |d: &str| dimensions.iter().any(|(k, _)| k == d);
    let format = formats(namespace, dimensions).find(|f| placeholders(f).iter().all(|(key, _, _)| has(key)))?;
    placeholders(format).into_iter().find_map(|(_, _, l)| l).and_then(|l| LOOKUPS.iter().copied().find(|x| *x == l))
}

/// The ARN (or ARN pattern, see [`RESOURCES`]) of the resource a metric's dimensions name, for the namespaces we map.
pub fn resource_arn(partition: &str, region: &str, account: &str, namespace: &str, dimensions: &[(String, String)], ids: &Lookups) -> Option<String> {
    formats(namespace, dimensions).find_map(|format| {
        let mut arn = format.replace("{p}", partition).replace("{r}", region).replace("{a}", account);
        for (key, lower, lookup) in placeholders(format) {
            let value = dimensions.iter().find(|(k, _)| k == key).map(|(_, v)| v)?;
            let value = match lookup {
                Some(l) => ids.get(l)?.get(value)?.clone(),
                None if lower => value.to_ascii_lowercase(),
                None => value.clone(),
            };
            let placeholder = match lookup {
                Some(l) => format!("{{{key}@{l}}}"),
                None if lower => format!("{{{key}:lower}}"),
                None => format!("{{{key}}}"),
            };
            arn = arn.replacen(&placeholder, &value, 1);
        }
        Some(arn)
    })
}

/// The Tagging API's resources, found by ARN or by an ARN pattern with `*`.
pub struct TagIndex<'a> {
    tags: &'a HashMap<String, Vec<String>>,
}

impl<'a> TagIndex<'a> {
    pub fn new(tags: &'a HashMap<String, Vec<String>>) -> Self {
        TagIndex { tags }
    }

    /// The tags of the resource an ARN (or the first resource an ARN pattern) names.
    pub fn get(&self, arn: &str) -> Vec<String> {
        if !arn.contains('*') {
            return self.tags.get(arn).cloned().unwrap_or_default();
        }
        let mut found: Vec<&String> = self.tags.keys().filter(|k| glob(arn, k)).collect();
        found.sort();
        found.first().map(|k| self.tags[*k].clone()).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::collections::HashSet;

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
        // Only negations: every resource without those tags, untagged ones too.
        let negated = json!([{ "namespace": "AWS/EBS", "tags": ["!datadog:no", "!env:dev*"] }]);
        assert!(tags_match(&negated, "AWS/EBS", &tags(&[])));
        assert!(tags_match(&negated, "AWS/EBS", &tags(&["env:prod"])));
        assert!(!tags_match(&negated, "AWS/EBS", &tags(&["env:development"])));
        // No filter for the namespace: everything.
        assert!(tags_match(&filters, "AWS/RDS", &tags(&[])));
        assert!(tags_match(&json!([]), "AWS/EC2", &tags(&[])));
    }

    #[test]
    fn dimensions_name_the_resource_whose_tags_the_metrics_get() {
        let d = |k: &str, v: &str| vec![(k.to_string(), v.to_string())];
        let arn = |ns: &str, dims: Vec<(String, String)>| resource_arn("aws", "us-east-1", "123456789012", ns, &dims, &Lookups::new());
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
        assert_eq!(resource_arn("aws-cn", "cn-north-1", "123456789012", "AWS/EC2", &d("InstanceId", "i-1"), &Lookups::new()).as_deref(), Some("arn:aws-cn:ec2:cn-north-1:123456789012:instance/i-1"));
        let p = "123456789012";
        type Case = (&'static str, Vec<(String, String)>, String);
        let cases: [Case; 30] = [
            ("AWS/EBS", d("VolumeId", "vol-0a1b2c3d4e5f60001"), format!("arn:aws:ec2:us-east-1:{p}:volume/vol-0a1b2c3d4e5f60001")),
            ("AWS/ApiGateway", d("ApiId", "a1b2c3d4e5"), "arn:aws:apigateway:us-east-1::/apis/a1b2c3d4e5".to_string()),
            ("AWS/SNS", d("TopicName", "order-events"), format!("arn:aws:sns:us-east-1:{p}:order-events")),
            ("AWS/ElastiCache", d("CacheClusterId", "carts-001"), format!("arn:aws:elasticache:us-east-1:{p}:cluster:carts-001")),
            ("AWS/ElastiCache", d("ReplicationGroupId", "carts"), format!("arn:aws:elasticache:us-east-1:{p}:replicationgroup:carts")),
            ("AWS/EFS", d("FileSystemId", "fs-0a1b2c3d"), format!("arn:aws:elasticfilesystem:us-east-1:{p}:file-system/fs-0a1b2c3d")),
            ("AWS/Kinesis", d("StreamName", "orders"), format!("arn:aws:kinesis:us-east-1:{p}:stream/orders")),
            ("AWS/Firehose", d("DeliveryStreamName", "orders-archive"), format!("arn:aws:firehose:us-east-1:{p}:deliverystream/orders-archive")),
            ("AWS/States", d("StateMachineArn", "arn:aws:states:us-east-1:123456789012:stateMachine:checkout"), format!("arn:aws:states:us-east-1:{p}:stateMachine:checkout")),
            ("AWS/NATGateway", d("NatGatewayId", "nat-0a1b2c3d4e5f60001"), format!("arn:aws:ec2:us-east-1:{p}:natgateway/nat-0a1b2c3d4e5f60001")),
            ("AWS/ELB", d("LoadBalancerName", "storefront"), format!("arn:aws:elasticloadbalancing:us-east-1:{p}:loadbalancer/storefront")),
            ("AWS/ApplicationELB", d("TargetGroup", "targetgroup/storefront/0a1b2c3d4e5f6000"), format!("arn:aws:elasticloadbalancing:us-east-1:{p}:targetgroup/storefront/0a1b2c3d4e5f6000")),
            ("AWS/NetworkELB", d("TargetGroup", "targetgroup/orders/0a1b2c3d4e5f6000"), format!("arn:aws:elasticloadbalancing:us-east-1:{p}:targetgroup/orders/0a1b2c3d4e5f6000")),
            ("AWS/GatewayELB", d("LoadBalancer", "gwy/inspection/0a1b2c3d4e5f6000"), format!("arn:aws:elasticloadbalancing:us-east-1:{p}:loadbalancer/gwy/inspection/0a1b2c3d4e5f6000")),
            ("AWS/Route53", d("HealthCheckId", "0a1b2c3d-0000-4000-8000-000000000001"), "arn:aws:route53:::healthcheck/0a1b2c3d-0000-4000-8000-000000000001".to_string()),
            ("AWS/WAFV2", dims(&[("Region", "us-east-1"), ("Rule", "ALL"), ("WebACL", "storefront-alb")]), format!("arn:aws:wafv2:us-east-1:{p}:regional/webacl/storefront-alb/*")),
            ("AWS/WAFV2", dims(&[("Rule", "ALL"), ("WebACL", "storefront-cdn")]), format!("arn:aws:wafv2:us-east-1:{p}:global/webacl/storefront-cdn/*")),
            ("AWS/Redshift", d("ClusterIdentifier", "analytics"), format!("arn:aws:redshift:us-east-1:{p}:cluster:analytics")),
            ("AWS/ES", d("DomainName", "catalog-search"), format!("arn:aws:es:us-east-1:{p}:domain/catalog-search")),
            ("AWS/Kafka", d("Cluster Name", "orders"), format!("arn:aws:kafka:us-east-1:{p}:cluster/orders/*")),
            ("AWS/AmazonMQ", d("Broker", "billing"), format!("arn:aws:mq:us-east-1:{p}:broker:billing:*")),
            ("AWS/SageMaker", dims(&[("EndpointName", "Recommendations"), ("VariantName", "AllTraffic")]), format!("arn:aws:sagemaker:us-east-1:{p}:endpoint/recommendations")),
            ("/aws/sagemaker/Endpoints", d("EndpointName", "recommendations"), format!("arn:aws:sagemaker:us-east-1:{p}:endpoint/recommendations")),
            ("AWS/DocDB", d("DBInstanceIdentifier", "catalog-1"), format!("arn:aws:rds:us-east-1:{p}:db:catalog-1")),
            ("AWS/Neptune", d("DBClusterIdentifier", "graph"), format!("arn:aws:rds:us-east-1:{p}:cluster:graph")),
            ("AWS/EKS", d("ClusterName", "storefront"), format!("arn:aws:eks:us-east-1:{p}:cluster/storefront")),
            ("AWS/AutoScaling", d("AutoScalingGroupName", "storefront-web"), format!("arn:aws:autoscaling:us-east-1:{p}:autoScalingGroup:*:autoScalingGroupName/storefront-web")),
            ("AWS/Events", d("RuleName", "nightly-billing"), format!("arn:aws:events:us-east-1:{p}:rule/nightly-billing")),
            ("AWS/Logs", d("LogGroupName", "/aws/lambda/checkout"), format!("arn:aws:logs:us-east-1:{p}:log-group:/aws/lambda/checkout")),
            ("CWAgent", d("InstanceId", "i-0a1b2c3d4e5f60001"), format!("arn:aws:ec2:us-east-1:{p}:instance/i-0a1b2c3d4e5f60001")),
        ];
        for (ns, dims, want) in cases {
            assert_eq!(arn(ns, dims.clone()).as_deref(), Some(want.as_str()), "{ns} {dims:?}");
        }
        assert_eq!(arn("AWS/EC2", d("AutoScalingGroupName", "web")), None);
        assert_eq!(arn("Acme/Checkout", d("Store", "eu")), None);
    }

    #[test]
    fn resources_their_metrics_name_by_name_are_found_through_a_lookup() {
        let ids: Lookups = HashMap::from([
            ("restapis".to_string(), HashMap::from([("storefront-api".to_string(), "a1b2c3d4e5".to_string())])),
            ("dbproxies".to_string(), HashMap::from([("orders-proxy".to_string(), "arn:aws:rds:us-east-1:123456789012:db-proxy:prx-0a1b2c3d4e5f60001".to_string())])),
            ("replicationinstances".to_string(), HashMap::from([("orders-replication".to_string(), "arn:aws:dms:us-east-1:123456789012:rep:EXAMPLEORDERSREPLICATION0001".to_string())])),
            ("webacls".to_string(), HashMap::from([("StorefrontCdn".to_string(), "arn:aws:waf::123456789012:webacl/0a1b2c3d-0000-4000-8000-000000000001".to_string())])),
            ("regionalwebacls".to_string(), HashMap::from([("StorefrontAlb".to_string(), "arn:aws:waf-regional:us-east-1:123456789012:webacl/0a1b2c3d-0000-4000-8000-000000000002".to_string())])),
        ]);
        let arn = |ns: &str, d: &[(&str, &str)]| resource_arn("aws", "us-east-1", "123456789012", ns, &dims(d), &ids);
        // REST APIs (v1) carry their name, their ARN their id; HTTP and WebSocket APIs carry their id.
        assert_eq!(arn("AWS/ApiGateway", &[("ApiName", "storefront-api"), ("Stage", "prod")]).as_deref(), Some("arn:aws:apigateway:us-east-1::/restapis/a1b2c3d4e5"));
        assert_eq!(arn("AWS/ApiGateway", &[("ApiId", "f1e2d3c4b5")]).as_deref(), Some("arn:aws:apigateway:us-east-1::/apis/f1e2d3c4b5"));
        assert_eq!(arn("AWS/RDS/Proxy", &[("ProxyName", "orders-proxy"), ("TargetGroup", "default"), ("TargetRole", "READ_WRITE")]).as_deref(), Some("arn:aws:rds:us-east-1:123456789012:db-proxy:prx-0a1b2c3d4e5f60001"));
        assert_eq!(arn("AWS/DMS", &[("ReplicationInstanceIdentifier", "orders-replication"), ("ReplicationTaskIdentifier", "ABCDEFGHIJ")]).as_deref(), Some("arn:aws:dms:us-east-1:123456789012:rep:EXAMPLEORDERSREPLICATION0001"));
        assert_eq!(arn("AWS/DMS", &[("ReplicationInstanceExternalResourceId", "EXAMPLEORDERSREPLICATION0001")]).as_deref(), Some("arn:aws:dms:us-east-1:123456789012:rep:EXAMPLEORDERSREPLICATION0001"));
        // WAF Classic's web ACLs by their metric name: CloudFront's are global, the others regional.
        assert_eq!(arn("WAF", &[("Rule", "ALL"), ("WebACL", "StorefrontCdn")]).as_deref(), Some("arn:aws:waf::123456789012:webacl/0a1b2c3d-0000-4000-8000-000000000001"));
        assert_eq!(arn("WAF", &[("Region", "us-east-1"), ("Rule", "ALL"), ("WebACL", "StorefrontAlb")]).as_deref(), Some("arn:aws:waf-regional:us-east-1:123456789012:webacl/0a1b2c3d-0000-4000-8000-000000000002"));
        // A name the lookup doesn't have names no resource.
        assert_eq!(arn("AWS/ApiGateway", &[("ApiName", "checkout-api")]), None);
        assert_eq!(resource_arn("aws", "us-east-1", "123456789012", "AWS/RDS/Proxy", &dims(&[("ProxyName", "orders-proxy")]), &Lookups::new()), None);
        // The lookup a series needs, if any.
        assert_eq!(lookup("AWS/ApiGateway", &dims(&[("ApiName", "storefront-api"), ("Stage", "prod")])), Some("restapis"));
        assert_eq!(lookup("AWS/ApiGateway", &dims(&[("ApiId", "f1e2d3c4b5")])), None);
        assert_eq!(lookup("AWS/RDS/Proxy", &dims(&[("ProxyName", "orders-proxy")])), Some("dbproxies"));
        assert_eq!(lookup("AWS/DMS", &dims(&[("ReplicationInstanceIdentifier", "orders-replication"), ("ReplicationTaskIdentifier", "T")])), Some("replicationinstances"));
        assert_eq!(lookup("AWS/DMS", &dims(&[("ReplicationInstanceExternalResourceId", "E")])), None);
        assert_eq!(lookup("WAF", &dims(&[("Rule", "ALL"), ("WebACL", "StorefrontCdn")])), Some("webacls"));
        assert_eq!(lookup("WAF", &dims(&[("Region", "us-east-1"), ("Rule", "ALL"), ("WebACL", "StorefrontAlb")])), Some("regionalwebacls"));
        assert_eq!(lookup("AWS/EC2", &dims(&[("InstanceId", "i-0a1b2c3d4e5f60001")])), None);
        assert_eq!(LOOKUPS.len(), 5);
    }

    #[test]
    fn rds_proxy_series_are_their_own_namespace() {
        // CloudWatch has RDS Proxy's metrics in AWS/RDS, by ProxyName; Datadog names them aws.rds.proxy.*.
        assert_eq!(series_namespace("AWS/RDS", &dims(&[("ProxyName", "orders-proxy")])), "AWS/RDS/Proxy");
        assert_eq!(series_namespace("AWS/RDS", &dims(&[("ProxyName", "orders-proxy"), ("TargetGroup", "default"), ("Target", "db:orders-db")])), "AWS/RDS/Proxy");
        assert_eq!(series_namespace("AWS/RDS", &dims(&[("DBInstanceIdentifier", "orders-db")])), "AWS/RDS");
        assert_eq!(series_namespace("AWS/EC2", &dims(&[("InstanceId", "i-0a1b2c3d4e5f60001")])), "AWS/EC2");
        assert_eq!(cloudwatch_namespace("AWS/RDS/Proxy"), "AWS/RDS");
        assert_eq!(cloudwatch_namespace("ECS/ContainerInsights"), "ECS/ContainerInsights");
        assert!(series_collected("AWS/RDS/Proxy", &dims(&[("ProxyName", "orders-proxy")])));
    }

    #[test]
    fn resources_are_found_by_their_arn_or_its_pattern() {
        let tags: HashMap<String, Vec<String>> = HashMap::from([
            ("arn:aws:ec2:us-east-1:123456789012:volume/vol-1".to_string(), vec!["team:orders".to_string()]),
            ("arn:aws:kafka:us-east-1:123456789012:cluster/orders/0a1b2c3d-0000-4000-8000-000000000001-2".to_string(), vec!["env:prod".to_string()]),
        ]);
        let index = TagIndex::new(&tags);
        assert_eq!(index.get("arn:aws:ec2:us-east-1:123456789012:volume/vol-1"), ["team:orders"]);
        assert_eq!(index.get("arn:aws:kafka:us-east-1:123456789012:cluster/orders/*"), ["env:prod"]);
        assert!(index.get("arn:aws:kafka:us-east-1:123456789012:cluster/payments/*").is_empty());
        assert!(index.get("arn:aws:ec2:us-east-1:123456789012:volume/vol-2").is_empty());
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
        // …in every namespace, from the seed's statistic (aws.lambda.invocations, aws.sqs.number_of_messages_sent)…
        assert_eq!(series_names("AWS/Lambda", "Invocations", &none, "Sum"), ["aws.lambda.invocations", "aws.lambda.invocations.sum"]);
        assert!(series_names("AWS/Lambda", "Invocations", &none, "Average").is_empty());
        assert_eq!(series_names("AWS/SQS", "NumberOfMessagesSent", &none, "Sum"), ["aws.sqs.number_of_messages_sent", "aws.sqs.number_of_messages_sent.sum"]);
        assert_eq!(series_names("AWS/ApiGateway", "Count", &none, "Sum")[0], "aws.apigateway.count");
        assert_eq!(series_names("AWS/ApplicationELB", "RequestCount", &none, "Sum")[0], "aws.applicationelb.request_count");
        assert_eq!(series_names("AWS/S3", "AllRequests", &none, "Sum")[0], "aws.s3.all_requests");
        // …gauges averages, with .maximum, .minimum and the latencies' percentiles.
        assert_eq!(series_names("AWS/RDS", "DatabaseConnections", &none, "Average"), ["aws.rds.database_connections"]);
        assert_eq!(series_names("AWS/RDS", "DatabaseConnections", &none, "Maximum"), ["aws.rds.database_connections.maximum"]);
        assert_eq!(series_names("AWS/RDS", "DatabaseConnections", &none, "Minimum"), ["aws.rds.database_connections.minimum"]);
        assert_eq!(series_names("AWS/ApplicationELB", "TargetResponseTime", &none, "Average"), ["aws.applicationelb.target_response_time"]);
        assert_eq!(series_names("AWS/ApplicationELB", "TargetResponseTime", &none, "p95"), ["aws.applicationelb.target_response_time.p95"]);
        assert_eq!(series_names("AWS/Lambda", "Duration", &none, "p99.9"), ["aws.lambda.duration.p99.9"]);
        let cluster = dims(&[("ClusterName", "storefront")]);
        let service = dims(&[("ClusterName", "storefront"), ("ServiceName", "order-service")]);
        assert_eq!(series_names("AWS/ECS", "CPUUtilization", &cluster, "Average"), ["aws.ecs.cpuutilization", "aws.ecs.cluster.cpuutilization"]);
        assert_eq!(series_names("AWS/ECS", "CPUUtilization", &service, "Maximum"), ["aws.ecs.cpuutilization.maximum", "aws.ecs.service.cpuutilization.maximum"]);
        let family = dims(&[("ClusterName", "storefront"), ("TaskDefinitionFamily", "order-service")]);
        assert_eq!(series_names("ECS/ContainerInsights", "CpuUtilized", &family, "Average"), ["ecs.containerinsights.family.cpu_utilized"]);
        assert_eq!(series_names("ECS/ContainerInsights", "RunningTaskCount", &service, "Average"), ["ecs.containerinsights.running_task_count"]);
    }

    #[test]
    fn each_metric_reads_its_statistics_and_unknown_metrics_follow_datadogs_convention() {
        // A count: its Sum (the bare name), Maximum and Minimum; the Average names nothing, so it isn't read.
        assert_eq!(statistics("AWS/Lambda", "Invocations"), ["Sum", "Maximum", "Minimum"]);
        // A gauge: Average (bare), Sum, Maximum, Minimum; a latency adds its percentiles.
        assert_eq!(statistics("AWS/EC2", "CPUUtilization"), ["Average", "Sum", "Maximum", "Minimum"]);
        assert_eq!(statistics("AWS/ApiGateway", "Latency"), ["Average", "Sum", "Maximum", "Minimum", "p50", "p75", "p90", "p95", "p99"]);
        // A sample count's bare name, with its .samplecount.
        let (ns, m) = catalog().iter().find(|(_, s)| s.statistic.as_deref() == Some("SampleCount")).map(|((ns, m), _)| (ns.clone(), m.clone())).unwrap();
        assert_eq!(statistics(&ns, &m), ["SampleCount", "Sum", "Maximum", "Minimum"]);
        assert_eq!(series_names(&ns, &m, &[], "SampleCount"), [metric_name(&ns, &m), format!("{}.samplecount", metric_name(&ns, &m))]);
        // Metrics outside the seed: counters (counts, requests, errors, bytes) are sums, the rest averages.
        assert_eq!(default_statistic("Acme/Checkout", "OrdersPlacedCount"), "Sum");
        assert_eq!(default_statistic("AWS/S3", "SelectRequests"), "Sum");
        assert_eq!(default_statistic("Acme/Checkout", "PaymentErrors"), "Sum");
        assert_eq!(default_statistic("Acme/Checkout", "BytesShipped"), "Sum");
        assert_eq!(default_statistic("Acme/Checkout", "QueueDepth"), "Average");
        assert_eq!(default_statistic("Acme/Checkout", "HealthyHostCount"), "Average");
    }

    #[test]
    fn daily_metrics_are_read_once_a_day_for_the_previous_day() {
        assert_eq!(period("AWS/S3", "BucketSizeBytes"), 86_400);
        assert_eq!(period("AWS/S3", "NumberOfObjects"), 86_400);
        assert_eq!(period("AWS/S3/Storage-Lens", "StorageBytes"), 86_400);
        assert_eq!(period("AWS/S3", "AllRequests"), 60);
        assert_eq!(period("AWS/EC2", "CPUUtilization"), 60);
        assert_eq!(period("Acme/Checkout", "Orders"), 60);
        assert_eq!(daily_namespaces(), ["AWS/S3", "AWS/S3/Storage-Lens"]);
    }

    /// The service dashboards (`scripts/cloud-service-dashboards.mjs`) of the namespaces that follow Datadog chart
    /// only names this crawler publishes for the metrics' dimension sets, split and filtered by their dimensions' tags.
    #[test]
    fn the_datadog_namespaces_dashboards_chart_what_the_crawler_publishes() {
        use std::collections::BTreeSet;
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
                for stat in statistics(ns, m["name"].as_str().unwrap()) {
                    names.extend(series_names(ns, m["name"].as_str().unwrap(), &d, &stat));
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

    /// Every AWS service dashboard charts only names the crawler publishes for its namespace's metrics.
    #[test]
    fn every_service_dashboard_charts_what_the_crawler_publishes() {
        use std::io::Read;
        let doc: Value = serde_json::from_str(include_str!("../../../../seeds/integrations/cloud/aws-metrics.json")).unwrap();
        let catalog: Value = serde_json::from_str(include_str!("../../../../seeds/aws_integration.json")).unwrap();
        let mut text = String::new();
        flate2::read::GzDecoder::new(&include_bytes!("../../../../seeds/integrations/cloud/services.ndjson.gz")[..]).read_to_string(&mut text).unwrap();
        let lines: Vec<Value> = text.lines().map(|l| serde_json::from_str(l).unwrap()).collect();
        let query = regex::Regex::new(r#"[a-z]+:([a-z0-9_.]+)\{"#).unwrap();
        let mut checked = 0;
        for (ns, spec) in doc["namespaces"].as_object().unwrap() {
            let Some(id) = catalog["namespaces"].as_array().unwrap().iter().find(|n| n["namespace"] == ns.as_str()).and_then(|n| n["id"].as_str()) else { continue };
            let Some(line) = lines.iter().find(|l| l["logo"] == format!("tile:{id}")) else { continue };
            let mut names = HashSet::new();
            for m in spec["metrics"].as_array().unwrap() {
                let d: Vec<(String, String)> = m["dimensions"].as_array().unwrap().iter().map(|k| (k.as_str().unwrap().to_string(), "x".to_string())).collect();
                let name = m["name"].as_str().unwrap();
                for stat in statistics(ns, name) {
                    names.extend(series_names(ns, name, &d, &stat));
                }
            }
            for c in query.captures_iter(&line["dashboards"][0]["widgets"].to_string()) {
                assert!(names.contains(&c[1]), "{ns}: {} isn't published by the crawler", &c[1]);
            }
            checked += 1;
        }
        assert!(checked >= 110, "{checked} AWS service dashboards");
    }

    #[test]
    fn rds_and_ec2_aggregates_are_left_out_and_global_services_are_known() {
        assert!(series_collected("AWS/RDS", &dims(&[("DBInstanceIdentifier", "orders-db")])));
        assert!(series_collected("AWS/RDS", &dims(&[("DBClusterIdentifier", "orders"), ("Role", "WRITER")])));
        assert!(!series_collected("AWS/RDS", &dims(&[("EngineName", "aurora-postgresql")])));
        assert!(!series_collected("AWS/RDS", &dims(&[])));
        // EC2 per instance only, as Datadog: the account-wide aggregates (by image, instance type, Auto Scaling group,
        // or none) would add to the instances' sums without a host.
        assert!(series_collected("AWS/EC2", &dims(&[("InstanceId", "i-0a1b2c3d4e5f60001")])));
        assert!(!series_collected("AWS/EC2", &dims(&[])));
        assert!(!series_collected("AWS/EC2", &dims(&[("AutoScalingGroupName", "storefront-web")])));
        assert!(!series_collected("AWS/EC2", &dims(&[("InstanceType", "m6i.large")])));
        assert!(!series_collected("AWS/EC2", &dims(&[("ImageId", "ami-0a1b2c3d4e5f60001")])));
        assert!(series_collected("AWS/EBS", &dims(&[("VolumeId", "vol-0a1b2c3d4e5f60001")])));
        assert!(global_series("AWS/CloudFront", &dims(&[("DistributionId", "E1A2B3C4D5E6F7"), ("Region", "Global")])));
        assert!(global_series("AWS/WAFV2", &dims(&[("WebACL", "storefront-cdn"), ("Rule", "ALL")])));
        assert!(!global_series("AWS/WAFV2", &dims(&[("Region", "eu-west-1"), ("WebACL", "storefront-alb"), ("Rule", "ALL")])));
        assert!(!global_series("AWS/EC2", &dims(&[])));
    }
}
