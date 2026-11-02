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
    /// Index into the metrics asked for.
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
    /// One statistic of `metrics` (at most 500) over `[start_ms, end_ms)`, `period` seconds apart.
    async fn get_metric_data(&self, target: &Target, metrics: &[MetricId], stat: &str, start_ms: i64, end_ms: i64, period: i32) -> Result<Vec<Series>, AwsError>;
    /// Every tagged resource of the region: ARN → `key:value` tags.
    async fn resource_tags(&self, target: &Target) -> Result<HashMap<String, Vec<String>>, AwsError>;
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
        let clients = Clients { cloudwatch: aws_sdk_cloudwatch::Client::new(&config), tagging: aws_sdk_resourcegroupstagging::Client::new(&config) };
        cache.insert(target.clone(), (clients.clone(), Instant::now()));
        clients
    }
}

fn dimensions(m: &MetricId) -> Vec<Dimension> {
    m.dimensions.iter().map(|(k, v)| Dimension::builder().name(k).value(v).build()).collect()
}

#[async_trait::async_trait]
impl AwsApi for Sdk {
    async fn list_metrics(&self, target: &Target) -> Result<Vec<MetricId>, AwsError> {
        let client = self.clients(target).await.cloudwatch;
        let mut pages = client.list_metrics().recently_active(RecentlyActive::Pt3H).into_paginator().send();
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

    async fn get_metric_data(&self, target: &Target, metrics: &[MetricId], stat: &str, start_ms: i64, end_ms: i64, period: i32) -> Result<Vec<Series>, AwsError> {
        let client = self.clients(target).await.cloudwatch;
        let queries: Vec<MetricDataQuery> = metrics
            .iter()
            .enumerate()
            .map(|(i, m)| {
                let metric = Metric::builder().namespace(&m.namespace).metric_name(&m.name).set_dimensions(Some(dimensions(m))).build();
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
            let series = sdk.get_metric_data(&target, &mine, "Average", now - 10 * 60_000, now + 60_000, 60).await.unwrap();
            let mut points = series[0].points.clone();
            points.sort_by_key(|p| p.0);
            assert_eq!(points, [(now - 120_000, 41.0), (now - 60_000, 43.0)]);
            sdk.resource_tags(&target).await.unwrap();
        }
    }
}
