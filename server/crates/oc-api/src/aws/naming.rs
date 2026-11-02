//! How CloudWatch metrics become ObserveCraft metrics, Datadog-compatible: names, tags, the
//! account's filters, and the OTLP JSON the crawler publishes.

use serde_json::Value;

pub use crate::cloud::naming::Point;
use crate::cloud::naming::{glob, snake, underscored};

/// A CloudWatch statistic the crawler collects, and the suffix its metric name gets.
pub const STATS: [(&str, &str); 3] = [("Average", ""), ("Sum", ".sum"), ("Maximum", ".maximum")];

/// The Datadog name of a CloudWatch metric: `aws.<namespace>.<metric>` (`AWS/EC2` + `NetworkIn` →
/// `aws.ec2.network_in`; namespaces outside `AWS/` keep their whole name).
pub fn metric_name(namespace: &str, metric: &str) -> String {
    let lower = namespace.to_ascii_lowercase();
    let ns = lower.strip_prefix("/aws/").or_else(|| lower.strip_prefix("aws/")).unwrap_or(&lower);
    format!("aws.{}.{}", underscored(ns), snake(metric))
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

/// The resource a metric is about, as the Tagging API names it: (namespace, dimension, ARN format
/// with `{p}` partition, `{r}` region, `{a}` account and `{v}` the dimension's value).
const RESOURCES: [(&str, &str, &str); 11] = [
    ("AWS/EC2", "InstanceId", "arn:{p}:ec2:{r}:{a}:instance/{v}"),
    ("AWS/Lambda", "FunctionName", "arn:{p}:lambda:{r}:{a}:function:{v}"),
    ("AWS/RDS", "DBInstanceIdentifier", "arn:{p}:rds:{r}:{a}:db:{v}"),
    ("AWS/DynamoDB", "TableName", "arn:{p}:dynamodb:{r}:{a}:table/{v}"),
    ("AWS/SQS", "QueueName", "arn:{p}:sqs:{r}:{a}:{v}"),
    ("AWS/SNS", "TopicName", "arn:{p}:sns:{r}:{a}:{v}"),
    ("AWS/ApplicationELB", "LoadBalancer", "arn:{p}:elasticloadbalancing:{r}:{a}:loadbalancer/{v}"),
    ("AWS/NetworkELB", "LoadBalancer", "arn:{p}:elasticloadbalancing:{r}:{a}:loadbalancer/{v}"),
    ("AWS/S3", "BucketName", "arn:{p}:s3:::{v}"),
    ("AWS/ECS", "ClusterName", "arn:{p}:ecs:{r}:{a}:cluster/{v}"),
    ("AWS/ElastiCache", "CacheClusterId", "arn:{p}:elasticache:{r}:{a}:cluster:{v}"),
];

/// The ARN of the resource a metric's dimensions name, for the namespaces we map.
pub fn resource_arn(partition: &str, region: &str, account: &str, namespace: &str, dimensions: &[(String, String)]) -> Option<String> {
    RESOURCES.iter().find_map(|(ns, dim, format)| {
        let value = dimensions.iter().find(|(k, _)| k == dim).map(|(_, v)| v).filter(|_| *ns == namespace)?;
        Some(format.replace("{p}", partition).replace("{r}", region).replace("{a}", account).replace("{v}", value))
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
        assert_eq!(metric_name("AWS/RDS/Proxy", "ClientConnections"), "aws.rds_proxy.client_connections");
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
        assert_eq!(resource_arn("aws-cn", "cn-north-1", "123456789012", "AWS/EC2", &d("InstanceId", "i-1")).as_deref(), Some("arn:aws-cn:ec2:cn-north-1:123456789012:instance/i-1"));
        assert_eq!(arn("AWS/EC2", d("AutoScalingGroupName", "web")), None);
        assert_eq!(arn("Acme/Checkout", d("Store", "eu")), None);
    }

}
