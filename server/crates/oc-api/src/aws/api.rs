//! The AWS APIs the crawler calls, behind [`AwsApi`] (tests use a fake), and their implementation
//! with the official AWS SDK ([`Sdk`]): the server's own identity from the default credential chain
//! (environment, EKS IRSA / Pod Identity, ECS, EC2 instance profile), AssumeRole into the customer's
//! role with its external id (or the account's access keys), CloudWatch ListMetrics / GetMetricData
//! and the Resource Groups Tagging API.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use aws_config::{BehaviorVersion, Region, SdkConfig};
use aws_sdk_cloudwatch::error::{DisplayErrorContext, ProvideErrorMetadata};
use aws_sdk_cloudwatch::primitives::DateTime;
use aws_sdk_cloudwatch::types::{Dimension, Metric, MetricDataQuery, MetricStat, RecentlyActive, ScanBy};
use tokio::sync::Mutex;

use super::naming;

/// Who to call and how: the account (its partition and id), the region and the credentials.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Target {
    pub partition: String,
    pub account_id: String,
    pub region: String,
    pub auth: TargetAuth,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum TargetAuth {
    Role { role_name: String, external_id: String },
    Keys { access_key_id: String, secret_access_key: String },
}

/// A CloudWatch metric: namespace, name and dimensions.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct MetricId {
    pub namespace: String,
    pub name: String,
    pub dimensions: Vec<(String, String)>,
}

/// The values of one metric and statistic.
#[derive(Clone, Debug, PartialEq)]
pub struct Series {
    /// Index into the queries asked for.
    pub metric: usize,
    pub points: Vec<(i64, f64)>,
}

/// Why a call failed, as the crawler acts on it.
#[derive(Clone, Debug, PartialEq)]
pub enum AwsError {
    /// The role or keys lack a permission; `action` when AWS names it (`cloudwatch:GetMetricData`).
    AccessDenied { action: Option<String>, message: String },
    /// ObserveCraft can't assume the role (trust policy, external id, role name).
    AssumeRole(String),
    /// AWS throttled the calls: retry later.
    Throttled(String),
    /// The region isn't enabled for the account (opt-in regions).
    RegionDisabled(String),
    Other(String),
}

impl AwsError {
    pub fn message(&self) -> String {
        match self {
            AwsError::AccessDenied { message, .. } | AwsError::AssumeRole(message) | AwsError::Throttled(message) | AwsError::RegionDisabled(message) | AwsError::Other(message) => message.clone(),
        }
    }
}

/// Classifies an AWS error by its code and message.
pub fn classify(code: Option<&str>, message: &str) -> AwsError {
    let message = message.to_string();
    let action = || message.split("perform: ").nth(1).and_then(|s| s.split_whitespace().next()).map(|a| a.trim_end_matches(['.', ',']).to_string());
    match code.unwrap_or_default() {
        "AccessDenied" | "AccessDeniedException" | "UnauthorizedOperation" | "UnauthorizedException" => {
            if message.contains("sts:AssumeRole") {
                AwsError::AssumeRole(message)
            } else {
                AwsError::AccessDenied { action: action(), message }
            }
        }
        "Throttling" | "ThrottlingException" | "RequestLimitExceeded" | "TooManyRequestsException" | "LimitExceededException" => AwsError::Throttled(message),
        "OptInRequired" | "InvalidClientTokenId" | "UnrecognizedClientException" | "AuthFailure" => AwsError::RegionDisabled(message),
        _ if message.contains("AssumeRole") || message.contains("failed to load credentials") || message.contains("resolve identity") => AwsError::AssumeRole(message),
        _ => AwsError::Other(message),
    }
}

/// What the crawler needs from AWS.
#[async_trait::async_trait]
pub trait AwsApi: Send + Sync {
    /// Metrics with data in the last 3 hours.
    async fn list_metrics(&self, target: &Target) -> Result<Vec<MetricId>, AwsError>;
    /// Every metric of a namespace, recently active or not (daily metrics may have no point in the last 3 hours).
    async fn list_namespace_metrics(&self, target: &Target, namespace: &str) -> Result<Vec<MetricId>, AwsError>;
    /// Each query's metric and statistic (`Average`, `Sum`, …, or an extended statistic such as `p95`; at most 500
    /// queries) over `[start_ms, end_ms)`, `period` seconds apart. [`Series::metric`] indexes the queries.
    async fn get_metric_data(&self, target: &Target, queries: &[(MetricId, String)], start_ms: i64, end_ms: i64, period: i32) -> Result<Vec<Series>, AwsError>;
    /// Every tagged resource of the region: ARN → `key:value` tags.
    async fn resource_tags(&self, target: &Target) -> Result<HashMap<String, Vec<String>>, AwsError>;
    /// One of [`super::naming::LOOKUPS`] in the region: REST APIs' ids by name, RDS proxies' and DMS replication
    /// instances' ARNs by name, WAF Classic web ACLs' ARNs by metric name (`webacls`: CloudFront's, global).
    async fn resource_ids(&self, target: &Target, lookup: &str) -> Result<HashMap<String, String>, AwsError>;
}

/// The AWS SDK. Clients are kept per account and region (their credentials are cached and renewed
/// by the SDK) for an hour.
pub struct Sdk {
    base: SdkConfig,
    clients: Mutex<HashMap<Target, (Clients, Instant)>>,
}

#[derive(Clone)]
struct Clients {
    cloudwatch: aws_sdk_cloudwatch::Client,
    tagging: aws_sdk_resourcegroupstagging::Client,
    s3: aws_sdk_s3::Client,
    apigateway: aws_sdk_apigateway::Client,
    rds: aws_sdk_rds::Client,
    dms: aws_sdk_databasemigration::Client,
    /// WAF Classic: CloudFront's web ACLs (global, us-east-1) and the region's.
    waf: aws_sdk_waf::Client,
    waf_regional: aws_sdk_wafregional::Client,
}

const CLIENT_TTL: Duration = Duration::from_secs(3600);

fn sdk_error<E: ProvideErrorMetadata + std::error::Error + 'static, R: std::fmt::Debug>(e: aws_sdk_cloudwatch::error::SdkError<E, R>) -> AwsError {
    let code = e.code().map(str::to_string);
    let message = e.message().map(str::to_string).unwrap_or_else(|| DisplayErrorContext(&e).to_string());
    classify(code.as_deref(), &message)
}

impl Sdk {
    /// The server's identity from the default credential chain.
    pub async fn from_env() -> Arc<Sdk> {
        Sdk::with_base(aws_config::defaults(BehaviorVersion::latest()).region(Region::new("us-east-1")).load().await)
    }

    /// Over a given base configuration (identity, endpoint).
    pub fn with_base(base: SdkConfig) -> Arc<Sdk> {
        Arc::new(Sdk { base, clients: Mutex::default() })
    }

    async fn clients(&self, target: &Target) -> Clients {
        let mut cache = self.clients.lock().await;
        cache.retain(|_, (_, at)| at.elapsed() < CLIENT_TTL);
        if let Some((c, _)) = cache.get(target) {
            return c.clone();
        }
        let region = Region::new(target.region.clone());
        let builder = self.base.to_builder().region(region.clone());
        let config = match &target.auth {
            TargetAuth::Role { role_name, external_id } => {
                let arn = format!("arn:{}:iam::{}:role/{}", target.partition, target.account_id, role_name);
                let provider = aws_config::sts::AssumeRoleProvider::builder(arn).external_id(external_id).session_name("observecraft").region(region).configure(&self.base).build().await;
                builder.credentials_provider(aws_sdk_cloudwatch::config::SharedCredentialsProvider::new(provider)).build()
            }
            TargetAuth::Keys { access_key_id, secret_access_key } => {
                let creds = aws_sdk_cloudwatch::config::Credentials::new(access_key_id, secret_access_key, None, None, "observecraft-aws-account");
                builder.credentials_provider(aws_sdk_cloudwatch::config::SharedCredentialsProvider::new(creds)).build()
            }
        };
        let clients = Clients {
            cloudwatch: aws_sdk_cloudwatch::Client::new(&config),
            tagging: aws_sdk_resourcegroupstagging::Client::new(&config),
            s3: aws_sdk_s3::Client::new(&config),
            apigateway: aws_sdk_apigateway::Client::new(&config),
            rds: aws_sdk_rds::Client::new(&config),
            dms: aws_sdk_databasemigration::Client::new(&config),
            waf: aws_sdk_waf::Client::from_conf(aws_sdk_waf::config::Builder::from(&config).region(Region::new("us-east-1")).build()),
            waf_regional: aws_sdk_wafregional::Client::new(&config),
        };
        cache.insert(target.clone(), (clients.clone(), Instant::now()));
        clients
    }
}

impl Sdk {
    /// The keys under `prefix` in the bucket (the region is the target's).
    pub async fn list_objects(&self, target: &Target, bucket: &str, prefix: &str) -> Result<Vec<String>, AwsError> {
        let client = self.clients(target).await.s3;
        let mut pages = client.list_objects_v2().bucket(bucket).prefix(prefix).into_paginator().send();
        let mut out = vec![];
        while let Some(page) = pages.next().await {
            out.extend(page.map_err(sdk_error)?.contents().iter().filter_map(|o| o.key().map(str::to_string)));
        }
        Ok(out)
    }

    pub async fn get_object(&self, target: &Target, bucket: &str, key: &str) -> Result<Vec<u8>, AwsError> {
        let client = self.clients(target).await.s3;
        let object = client.get_object().bucket(bucket).key(key).send().await.map_err(sdk_error)?;
        let bytes = object.body.collect().await.map_err(|e| AwsError::Other(format!("reading s3://{bucket}/{key}: {e}")))?;
        Ok(bytes.into_bytes().to_vec())
    }
}

fn dimensions(m: &MetricId) -> Vec<Dimension> {
    m.dimensions.iter().map(|(k, v)| Dimension::builder().name(k).value(v).build()).collect()
}

/// Every page of a ListMetrics call.
async fn listed(request: aws_sdk_cloudwatch::operation::list_metrics::builders::ListMetricsFluentBuilder) -> Result<Vec<MetricId>, AwsError> {
    let mut pages = request.into_paginator().send();
    let mut out = vec![];
    while let Some(page) = pages.next().await {
        for m in page.map_err(sdk_error)?.metrics() {
            let (Some(namespace), Some(name)) = (m.namespace(), m.metric_name()) else { continue };
            let mut dimensions: Vec<(String, String)> = m.dimensions().iter().filter_map(|d| Some((d.name()?.to_string(), d.value()?.to_string()))).collect();
            dimensions.sort();
            out.push(MetricId { namespace: namespace.to_string(), name: name.to_string(), dimensions });
        }
    }
    Ok(out)
}

#[async_trait::async_trait]
impl AwsApi for Sdk {
    async fn list_metrics(&self, target: &Target) -> Result<Vec<MetricId>, AwsError> {
        let client = self.clients(target).await.cloudwatch;
        listed(client.list_metrics().recently_active(RecentlyActive::Pt3H)).await
    }

    async fn list_namespace_metrics(&self, target: &Target, namespace: &str) -> Result<Vec<MetricId>, AwsError> {
        let client = self.clients(target).await.cloudwatch;
        listed(client.list_metrics().namespace(namespace)).await
    }

    async fn get_metric_data(&self, target: &Target, queries: &[(MetricId, String)], start_ms: i64, end_ms: i64, period: i32) -> Result<Vec<Series>, AwsError> {
        let client = self.clients(target).await.cloudwatch;
        let queries: Vec<MetricDataQuery> = queries
            .iter()
            .enumerate()
            .map(|(i, (m, stat))| {
                let metric = Metric::builder().namespace(naming::cloudwatch_namespace(&m.namespace)).metric_name(&m.name).set_dimensions(Some(dimensions(m))).build();
                MetricDataQuery::builder().id(format!("m{i}")).metric_stat(MetricStat::builder().metric(metric).period(period).stat(stat).build()).return_data(true).build()
            })
            .collect();
        let mut pages = client
            .get_metric_data()
            .set_metric_data_queries(Some(queries))
            .start_time(DateTime::from_millis(start_ms))
            .end_time(DateTime::from_millis(end_ms))
            .scan_by(ScanBy::TimestampAscending)
            .into_paginator()
            .send();
        let mut series: HashMap<usize, Vec<(i64, f64)>> = HashMap::new();
        while let Some(page) = pages.next().await {
            for r in page.map_err(sdk_error)?.metric_data_results() {
                let Some(i) = r.id().and_then(|id| id.strip_prefix('m')).and_then(|n| n.parse::<usize>().ok()) else { continue };
                let points = series.entry(i).or_default();
                for (t, v) in r.timestamps().iter().zip(r.values()) {
                    if let Ok(ms) = t.to_millis() {
                        points.push((ms, *v));
                    }
                }
            }
        }
        let mut out: Vec<Series> = series.into_iter().map(|(metric, points)| Series { metric, points }).collect();
        out.sort_by_key(|s| s.metric);
        Ok(out)
    }

    async fn resource_tags(&self, target: &Target) -> Result<HashMap<String, Vec<String>>, AwsError> {
        let client = self.clients(target).await.tagging;
        let mut pages = client.get_resources().into_paginator().send();
        let mut out = HashMap::new();
        while let Some(page) = pages.next().await {
            let page = page.map_err(|e| {
                let code = e.code().map(str::to_string);
                let message = e.message().map(str::to_string).unwrap_or_else(|| DisplayErrorContext(&e).to_string());
                classify(code.as_deref(), &message)
            })?;
            for r in page.resource_tag_mapping_list() {
                let Some(arn) = r.resource_arn() else { continue };
                out.insert(arn.to_string(), r.tags().iter().map(|t| format!("{}:{}", t.key(), t.value())).collect());
            }
        }
        Ok(out)
    }

    async fn resource_ids(&self, target: &Target, lookup: &str) -> Result<HashMap<String, String>, AwsError> {
        self.lookup(target, lookup).await
    }
}

/// A WAF Classic client's web ACLs (`waf` or `waf-regional`): metric name → ARN, from ListWebACLs and GetWebACL.
macro_rules! web_acls {
    ($client:expr, $out:expr) => {{
        let mut marker: Option<String> = None;
        loop {
            let page = $client.list_web_acls().limit(100).set_next_marker(marker.take()).send().await.map_err(sdk_error)?;
            for summary in page.web_acls() {
                let acl = $client.get_web_acl().web_acl_id(summary.web_acl_id()).send().await.map_err(sdk_error)?;
                if let Some((metric, arn)) = acl.web_acl().and_then(|a| Some((a.metric_name()?, a.web_acl_arn()?))) {
                    $out.insert(metric.to_string(), arn.to_string());
                }
            }
            match page.next_marker() {
                Some(next) if !page.web_acls().is_empty() => marker = Some(next.to_string()),
                _ => break,
            }
        }
    }};
}

impl Sdk {
    async fn lookup(&self, target: &Target, lookup: &str) -> Result<HashMap<String, String>, AwsError> {
        let c = self.clients(target).await;
        let mut out = HashMap::new();
        let mut keep = |name: Option<&str>, id: Option<&str>| {
            if let (Some(name), Some(id)) = (name, id) {
                out.entry(name.to_string()).or_insert_with(|| id.to_string());
            }
        };
        match lookup {
            "restapis" => {
                let mut pages = c.apigateway.get_rest_apis().limit(500).into_paginator().send();
                while let Some(page) = pages.next().await {
                    page.map_err(sdk_error)?.items().iter().for_each(|a| keep(a.name(), a.id()));
                }
            }
            "dbproxies" => {
                let mut pages = c.rds.describe_db_proxies().into_paginator().send();
                while let Some(page) = pages.next().await {
                    page.map_err(sdk_error)?.db_proxies().iter().for_each(|p| keep(p.db_proxy_name(), p.db_proxy_arn()));
                }
            }
            "replicationinstances" => {
                let mut pages = c.dms.describe_replication_instances().into_paginator().send();
                while let Some(page) = pages.next().await {
                    page.map_err(sdk_error)?.replication_instances().iter().for_each(|r| keep(r.replication_instance_identifier(), r.replication_instance_arn()));
                }
            }
            "webacls" => web_acls!(c.waf, out),
            "regionalwebacls" => web_acls!(c.waf_regional, out),
            other => return Err(AwsError::Other(format!("unknown resource lookup {other}"))),
        }
        Ok(out)
    }
}

/// Short-lived credentials handed to a Cloud Security scanner (docs/adr/0006).
#[derive(Clone, Debug, serde::Serialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct TempCredentials {
    pub access_key_id: String,
    pub secret_access_key: String,
    pub session_token: String,
    /// Epoch seconds.
    pub expiration: i64,
}

/// The AWS managed policies a posture scan's session is narrowed to, with [`SCAN_SESSION_POLICY`].
pub const SCAN_MANAGED_POLICIES: [&str; 2] = ["SecurityAudit", "job-function/ViewOnlyAccess"];

/// Prowler's read-only additions to those policies (`seeds/posture/README.md`).
pub const SCAN_SESSION_POLICY: &str = include_str!("../../../../seeds/posture/aws-scan-session-policy.json");

/// The STS region of a partition.
fn sts_region(partition: &str) -> &'static str {
    match partition {
        "aws-us-gov" => "us-gov-west-1",
        "aws-cn" => "cn-north-1",
        _ => "us-east-1",
    }
}

impl Sdk {
    /// Credentials for one posture scan of an account, valid `seconds`: AssumeRole into its role (with the external
    /// id), or GetFederationToken with its keys. Either way the session policies ([`SCAN_MANAGED_POLICIES`] and
    /// [`SCAN_SESSION_POLICY`]) cap what they can do, whatever the role or user is allowed.
    pub async fn scan_credentials(&self, partition: &str, account_id: &str, auth: &TargetAuth, seconds: i32) -> Result<TempCredentials, AwsError> {
        let policy: serde_json::Value = serde_json::from_str(SCAN_SESSION_POLICY).map_err(|e| AwsError::Other(format!("scan session policy: {e}")))?;
        let policy = policy.to_string();
        let managed: Vec<aws_sdk_sts::types::PolicyDescriptorType> =
            SCAN_MANAGED_POLICIES.iter().map(|p| aws_sdk_sts::types::PolicyDescriptorType::builder().arn(format!("arn:{partition}:iam::aws:policy/{p}")).build()).collect();
        let region = Region::new(sts_region(partition));
        let credentials = match auth {
            TargetAuth::Role { role_name, external_id } => {
                let sts = aws_sdk_sts::Client::from_conf(aws_sdk_sts::config::Builder::from(&self.base).region(region).build());
                let out = sts
                    .assume_role()
                    .role_arn(format!("arn:{partition}:iam::{account_id}:role/{role_name}"))
                    .role_session_name("observecraft-posture-scan")
                    .external_id(external_id)
                    .duration_seconds(seconds)
                    .policy(policy)
                    .set_policy_arns(Some(managed))
                    .send()
                    .await
                    .map_err(sdk_error)?;
                out.credentials().cloned()
            }
            TargetAuth::Keys { access_key_id, secret_access_key } => {
                let keys = aws_sdk_sts::config::Credentials::new(access_key_id, secret_access_key, None, None, "observecraft-aws-account");
                let sts = aws_sdk_sts::Client::from_conf(aws_sdk_sts::config::Builder::from(&self.base).region(region).credentials_provider(keys).build());
                let out = sts
                    .get_federation_token()
                    .name("observecraft-posture")
                    .duration_seconds(seconds.max(900))
                    .policy(policy)
                    .set_policy_arns(Some(managed))
                    .send()
                    .await
                    .map_err(sdk_error)?;
                out.credentials().cloned()
            }
        };
        let c = credentials.ok_or_else(|| AwsError::Other("STS returned no credentials".into()))?;
        Ok(TempCredentials {
            access_key_id: c.access_key_id().to_string(),
            secret_access_key: c.secret_access_key().to_string(),
            session_token: c.session_token().to_string(),
            expiration: c.expiration().secs(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn errors_are_classified_by_what_the_crawler_does_about_them() {
        assert_eq!(
            classify(Some("AccessDenied"), "User: arn:aws:sts::123456789012:assumed-role/ObserveCraftIntegrationRole/observecraft is not authorized to perform: cloudwatch:GetMetricData because no identity-based policy allows the cloudwatch:GetMetricData action"),
            AwsError::AccessDenied {
                action: Some("cloudwatch:GetMetricData".into()),
                message: "User: arn:aws:sts::123456789012:assumed-role/ObserveCraftIntegrationRole/observecraft is not authorized to perform: cloudwatch:GetMetricData because no identity-based policy allows the cloudwatch:GetMetricData action".into()
            }
        );
        assert!(matches!(classify(Some("AccessDenied"), "User: arn:aws:iam::210987654321:role/observecraft-server is not authorized to perform: sts:AssumeRole on resource"), AwsError::AssumeRole(_)));
        assert!(matches!(classify(Some("Throttling"), "Rate exceeded"), AwsError::Throttled(_)));
        assert!(matches!(classify(Some("OptInRequired"), "region not enabled"), AwsError::RegionDisabled(_)));
        assert!(matches!(classify(None, "failed to load credentials: AssumeRole failed"), AwsError::AssumeRole(_)));
        assert!(matches!(classify(Some("InternalFailure"), "boom"), AwsError::Other(_)));
    }

    /// Against an AWS API emulator (`OC_TEST_AWS_ENDPOINT`, e.g. LocalStack on http://localhost:4566):
    /// metrics written to CloudWatch are listed and read back with keys and through AssumeRole.
    #[tokio::test]
    async fn the_sdk_lists_and_reads_metrics_with_keys_and_an_assumed_role() {
        use aws_sdk_cloudwatch::types::MetricDatum;
        let Ok(endpoint) = std::env::var("OC_TEST_AWS_ENDPOINT") else { return };
        let creds = aws_sdk_cloudwatch::config::Credentials::new("test", "test", None, None, "test");
        let base = aws_config::defaults(BehaviorVersion::latest()).region(Region::new("us-east-1")).endpoint_url(&endpoint).credentials_provider(creds).load().await;
        let instance = format!("i-{:x}", rand::random::<u32>());
        let now = chrono::Utc::now().timestamp_millis() / 60_000 * 60_000;
        let put = aws_sdk_cloudwatch::Client::new(&base);
        for (i, v) in [(2, 41.0), (1, 43.0)] {
            let datum = MetricDatum::builder()
                .metric_name("CPUUtilization")
                .dimensions(Dimension::builder().name("InstanceId").value(&instance).build())
                .value(v)
                .timestamp(DateTime::from_millis(now - i * 60_000))
                .build();
            put.put_metric_data().namespace("AWS/EC2").metric_data(datum).send().await.unwrap();
        }
        let sdk = Sdk::with_base(base);
        let keys = Target { partition: "aws".into(), account_id: "000000000000".into(), region: "us-east-1".into(), auth: TargetAuth::Keys { access_key_id: "test".into(), secret_access_key: "test".into() } };
        let role = Target { auth: TargetAuth::Role { role_name: "ObserveCraftIntegrationRole".into(), external_id: "2803c423184c499dbd123d346e5bd16f".into() }, ..keys.clone() };
        for target in [keys, role] {
            let metrics = sdk.list_metrics(&target).await.unwrap();
            let mine: Vec<MetricId> = metrics.into_iter().filter(|m| m.dimensions == [("InstanceId".to_string(), instance.clone())]).collect();
            assert_eq!(mine.len(), 1, "{target:?}");
            assert_eq!((mine[0].namespace.as_str(), mine[0].name.as_str()), ("AWS/EC2", "CPUUtilization"));
            let queries: Vec<(MetricId, String)> = mine.iter().map(|m| (m.clone(), "Average".to_string())).collect();
            let series = sdk.get_metric_data(&target, &queries, now - 10 * 60_000, now + 60_000, 60).await.unwrap();
            assert!(sdk.list_namespace_metrics(&target, "AWS/EC2").await.unwrap().iter().any(|m| m.dimensions == [("InstanceId".to_string(), instance.clone())]));
            let mut points = series[0].points.clone();
            points.sort_by_key(|p| p.0);
            assert_eq!(points, [(now - 120_000, 41.0), (now - 60_000, 43.0)]);
            sdk.resource_tags(&target).await.unwrap();
        }
    }
}
