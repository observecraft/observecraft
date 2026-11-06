//! The AWS integration's CloudWatch crawler: every configured account's metrics, every
//! `OC_CLOUD_CRAWL_INTERVAL_SECS` (10 minutes), published to Kafka like everything the intake accepts.
//! Its jobs (one per account and region) are the cloud crawlers' (`crate::cloud::jobs`): claimed with
//! `FOR UPDATE SKIP LOCKED`, so adding servers adds crawl capacity.
//!
//! - **A run** ([`crawl`]) lists the region's metrics with data in the last 3 hours, keeps those the
//!   account selects (namespaces, metric names, resource tags), and reads Average, Sum and Maximum
//!   per minute over the window since the watermark (at most 3 hours, ending 5 minutes ago: CloudWatch
//!   publishes late), 500 metrics per GetMetricData call.
//! - **Publishing.** Points become OTLP JSON gauges named as Datadog names them (`aws.ec2.cpuutilization`,
//!   `.sum`, `.maximum`), tagged with the account, the region, the account's tags, the dimensions and
//!   the resource's AWS tags, written to `oc.otlp-json.metrics` keyed by org with the org's headers.
//!   The watermark advances only after Kafka acknowledged every record of the window, so a crash or a
//!   Kafka outage re-reads the window instead of losing it.
//! - **Failures** become the account's issues: a missing permission (with the action AWS names), a role
//!   that can't be assumed, throttling (exponential backoff), a disabled region (retried daily). Tag
//!   lookups that are denied don't stop the run.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;

use serde_json::Value;

use super::api::{AwsApi, AwsError, MetricId, Target, TargetAuth};
use super::naming::{self, Point, STATS};
use crate::cloud::jobs::{self, Backoff, Crawler, Issue, JobKey, MetricSink, Outcome};
use crate::routes::datadog_api::aws_integration as accounts;
use crate::state::AppState;

pub const PROVIDER: &str = "aws";
/// CloudWatch's publishing delay: the window ends this long before now.
const LAG_MS: i64 = 5 * 60_000;
/// The longest window one run reads (a longer gap starts 3 hours back).
const MAX_WINDOW_MS: i64 = 3 * 3_600_000;
/// GetMetricData: metrics per call.
const BATCH: usize = 500;
/// Points per Kafka record.
pub const POINTS_PER_RECORD: usize = 10_000;
/// Regions an account with `include_all` is crawled in: the regions every account has enabled (opt-in
/// regions are crawled when the account lists them).
const DEFAULT_REGIONS: [&str; 17] = [
    "us-east-1", "us-east-2", "us-west-1", "us-west-2", "ap-south-1", "ap-northeast-1", "ap-northeast-2", "ap-northeast-3", "ap-southeast-1", "ap-southeast-2",
    "ca-central-1", "eu-central-1", "eu-west-1", "eu-west-2", "eu-west-3", "eu-north-1", "sa-east-1",
];

/// The regions an account is crawled in.
pub fn regions(partition: &str, aws_regions: &Value) -> Vec<String> {
    if let Some(only) = aws_regions["include_only"].as_array() {
        return only.iter().filter_map(Value::as_str).map(str::to_string).collect();
    }
    let all: &[&str] = match partition {
        "aws-cn" => &["cn-north-1", "cn-northwest-1"],
        "aws-us-gov" => &["us-gov-west-1", "us-gov-east-1"],
        _ => &DEFAULT_REGIONS,
    };
    all.iter().map(|r| r.to_string()).collect()
}

/// The jobs: every region of every account with metric collection on (active orgs).
pub async fn desired(st: &AppState) -> Result<Vec<(String, String, String)>, sqlx::Error> {
    let rows: Vec<(String, String, String, Value, bool)> = sqlx::query_as(
        "SELECT a.org_id, a.id::text, a.aws_partition, a.aws_regions, COALESCE((a.metrics_config->>'enabled')::boolean, true) \
         FROM aws_accounts a JOIN orgs o ON o.id = a.org_id AND o.status = 'active'",
    )
    .fetch_all(&st.postgres)
    .await?;
    Ok(rows
        .into_iter()
        .filter(|r| r.4)
        .flat_map(|(org, id, partition, aws_regions, _)| regions(&partition, &aws_regions).into_iter().map(move |region| (org.clone(), id.clone(), region)))
        .collect())
}

/// A job with what its run needs.
#[derive(Clone, Debug)]
pub struct Job {
    pub org_id: String,
    pub target: Target,
    pub account_tags: Vec<String>,
    pub metrics_config: Value,
    /// The watermark, epoch ms.
    pub collected_until: Option<i64>,
}

/// How to reach an account: its role, or its access keys with the secret unsealed.
pub async fn target_auth(st: &AppState, org_id: &str, account: &accounts::AccountRow) -> Result<TargetAuth, sqlx::Error> {
    Ok(match &account.auth {
        accounts::Auth::Role { role_name, external_id } => TargetAuth::Role { role_name: role_name.clone(), external_id: external_id.clone().unwrap_or_default() },
        accounts::Auth::Keys { access_key_id, .. } => {
            let sealed: Option<String> = sqlx::query_scalar("SELECT secret_access_key_enc FROM aws_accounts WHERE org_id = $1 AND id = $2::uuid").bind(org_id).bind(&account.id).fetch_one(&st.postgres).await?;
            let secret_access_key = sealed.and_then(|s| st.secrets.decrypt(&s).ok()).unwrap_or_default();
            TargetAuth::Keys { access_key_id: access_key_id.clone(), secret_access_key }
        }
    })
}

/// A job with its account's configuration and credentials (None when the account is gone).
pub async fn load_job(st: &AppState, key: &JobKey) -> Result<Option<Job>, sqlx::Error> {
    let Some(account) = accounts::get_row(st, &key.org_id, &key.account_id).await? else { return Ok(None) };
    let auth = target_auth(st, &key.org_id, &account).await?;
    Ok(Some(Job {
        target: Target { partition: account.aws_partition.clone(), account_id: account.aws_account_id.clone(), region: key.scope.clone(), auth },
        account_tags: account.account_tags.as_array().map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect()).unwrap_or_default(),
        metrics_config: account.metrics_config.clone(),
        org_id: key.org_id.clone(),
        collected_until: key.collected_until,
    }))
}

pub fn split_tag(t: &str) -> (String, String) {
    let (k, v) = t.split_once(':').unwrap_or((t, ""));
    (k.to_string(), v.to_string())
}

/// An AWS error as the account's issue.
pub fn issue(e: &AwsError) -> Issue {
    let (kind, missing_action, backoff) = match e {
        AwsError::AccessDenied { action, .. } => ("access_denied", action.clone(), Backoff::Interval),
        AwsError::AssumeRole(_) => ("auth", None, Backoff::Interval),
        AwsError::Throttled(_) => ("throttled", None, Backoff::Exponential),
        AwsError::RegionDisabled(_) => ("disabled", None, Backoff::Day),
        AwsError::Other(_) => ("other", None, Backoff::Exponential),
    };
    Issue { kind, message: e.message(), missing_action, backoff }
}

/// Reads and publishes one job's window.
pub async fn crawl(api: &dyn AwsApi, sink: &dyn MetricSink, headers: &[(String, String)], job: &Job, now_ms: i64, interval_ms: i64, known: &HashSet<&str>) -> Outcome {
    let end = (now_ms - LAG_MS) / 60_000 * 60_000;
    let start = job.collected_until.unwrap_or(end - interval_ms).max(end - MAX_WINDOW_MS);
    if end <= start {
        return Outcome::Idle;
    }
    let config = &job.metrics_config;
    let custom = config["collect_custom_metrics"].as_bool().unwrap_or(false);
    let listed = match api.list_metrics(&job.target).await {
        Ok(m) => m,
        Err(e) => return Outcome::Failed(issue(&e)),
    };
    let candidates: Vec<MetricId> = listed
        .into_iter()
        .filter(|m| naming::namespace_selected(&config["namespace_filters"], &m.namespace, known.contains(m.namespace.as_str()), custom))
        .filter(|m| naming::metric_selected(&config["metric_name_filters"], &m.namespace, &naming::metric_name(&m.namespace, &m.name)))
        .collect();
    if candidates.is_empty() {
        return Outcome::Collected { end_ms: end, metrics: 0, points: 0, warning: None };
    }
    let t = &job.target;
    let (tags_by_arn, warning) = match api.resource_tags(t).await {
        Ok(tags) => (tags, None),
        Err(e @ AwsError::AccessDenied { .. }) => (HashMap::new(), Some(issue(&e))),
        Err(e) => return Outcome::Failed(issue(&e)),
    };
    let resource_tags = |m: &MetricId| naming::resource_arn(&t.partition, &t.region, &t.account_id, &m.namespace, &m.dimensions).and_then(|arn| tags_by_arn.get(&arn).cloned()).unwrap_or_default();
    let metrics: Vec<(MetricId, Vec<String>)> = candidates
        .into_iter()
        .map(|m| {
            let tags = resource_tags(&m);
            (m, tags)
        })
        .filter(|(m, tags)| naming::tags_match(&config["tag_filters"], &m.namespace, tags))
        .collect();

    let base: Vec<(String, String)> = [("aws_account".to_string(), t.account_id.clone()), ("region".to_string(), t.region.clone())].into_iter().chain(job.account_tags.iter().map(|t| split_tag(t))).collect();
    let mut points = vec![];
    for chunk in metrics.chunks(BATCH) {
        let ids: Vec<MetricId> = chunk.iter().map(|(m, _)| m.clone()).collect();
        for (stat, suffix) in STATS {
            let series = match api.get_metric_data(t, &ids, stat, start, end, 60).await {
                Ok(s) => s,
                Err(e) => return Outcome::Failed(issue(&e)),
            };
            for s in series {
                let Some((m, resource)) = chunk.get(s.metric) else { continue };
                let name = format!("{}{suffix}", naming::metric_name(&m.namespace, &m.name));
                let host = m.dimensions.iter().find(|(k, _)| m.namespace == "AWS/EC2" && k == "InstanceId").map(|(_, v)| v.clone());
                let mut tags: BTreeMap<String, String> = base.iter().cloned().collect();
                tags.extend(resource.iter().map(|t| split_tag(t)));
                tags.extend(m.dimensions.iter().map(|(k, v)| (naming::tag_key(k), v.clone())));
                let tags: Vec<(String, String)> = tags.into_iter().collect();
                points.extend(s.points.iter().map(|&(time_ms, value)| Point { name: name.clone(), host: host.clone(), tags: tags.clone(), time_ms, value }));
            }
        }
    }
    for record in points.chunks(POINTS_PER_RECORD) {
        if let Err(e) = sink.publish(&job.org_id, headers, crate::cloud::naming::otlp_json(PROVIDER, record)).await {
            return Outcome::Failed(Issue { kind: "other", message: format!("publishing to Kafka failed: {e}"), missing_action: None, backoff: Backoff::Exponential });
        }
    }
    Outcome::Collected { end_ms: end, metrics: metrics.len(), points: points.len(), warning }
}

/// The namespaces of our AWS catalog (outside them, metrics are custom).
pub fn known_namespaces() -> HashSet<&'static str> {
    accounts::catalog().namespaces.iter().map(|n| n.namespace.as_str()).collect()
}

/// The AWS crawler: CloudWatch through `api`, points to `sink`.
pub struct AwsCrawler {
    pub api: Arc<dyn AwsApi>,
    pub sink: Arc<dyn MetricSink>,
    pub interval_secs: i64,
}

#[async_trait::async_trait]
impl Crawler for AwsCrawler {
    fn provider(&self) -> &'static str {
        PROVIDER
    }

    async fn desired(&self, st: &AppState) -> Result<Vec<(String, String, String)>, sqlx::Error> {
        desired(st).await
    }

    async fn run(&self, st: &AppState, key: &JobKey, now_ms: i64) -> Result<Outcome, sqlx::Error> {
        let Some(job) = load_job(st, key).await? else { return Ok(Outcome::Idle) };
        let headers = jobs::org_headers(st, &job.org_id).await?;
        Ok(crawl(self.api.as_ref(), self.sink.as_ref(), &headers, &job, now_ms, self.interval_secs * 1000, &known_namespaces()).await)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::Mutex as StdMutex;

    use super::super::api::Series;
    use crate::cloud::testing::{published, FakeSink};

    /// AWS as a test sets it up: metrics, their values, tags, and errors to return.
    #[derive(Default)]
    struct FakeAws {
        metrics: Vec<MetricId>,
        tags: HashMap<String, Vec<String>>,
        fail_list: Option<AwsError>,
        fail_data: Option<AwsError>,
        fail_tags: Option<AwsError>,
        calls: StdMutex<Vec<(String, usize, i64, i64)>>,
    }

    #[async_trait::async_trait]
    impl AwsApi for FakeAws {
        async fn list_metrics(&self, _t: &Target) -> Result<Vec<MetricId>, AwsError> {
            self.fail_list.clone().map_or(Ok(self.metrics.clone()), Err)
        }
        async fn get_metric_data(&self, _t: &Target, metrics: &[MetricId], stat: &str, start: i64, end: i64, _period: i32) -> Result<Vec<Series>, AwsError> {
            if let Some(e) = &self.fail_data {
                return Err(e.clone());
            }
            self.calls.lock().unwrap().push((stat.to_string(), metrics.len(), start, end));
            Ok((0..metrics.len()).map(|i| Series { metric: i, points: vec![(start, i as f64 + 1.0), (start + 60_000, i as f64 + 2.0)] }).collect())
        }
        async fn resource_tags(&self, _t: &Target) -> Result<HashMap<String, Vec<String>>, AwsError> {
            self.fail_tags.clone().map_or(Ok(self.tags.clone()), Err)
        }
    }

    fn metric(ns: &str, name: &str, dims: &[(&str, &str)]) -> MetricId {
        MetricId { namespace: ns.into(), name: name.into(), dimensions: dims.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect() }
    }

    fn job(collected_until: Option<i64>, metrics_config: Value) -> Job {
        Job {
            org_id: "acme".into(),
            target: Target { partition: "aws".into(), account_id: "123456789012".into(), region: "us-east-1".into(), auth: TargetAuth::Role { role_name: "R".into(), external_id: "e".into() } },
            account_tags: vec!["env:prod".into()],
            metrics_config,
            collected_until,
        }
    }

    fn config() -> Value {
        json!({ "namespace_filters": { "exclude_only": ["AWS/SQS"] }, "tag_filters": [], "collect_custom_metrics": false })
    }

    const NOW: i64 = 1_700_000_000_000;

    #[tokio::test]
    async fn a_run_publishes_the_selected_metrics_named_and_tagged_as_datadog_does() {
        let aws = FakeAws {
            metrics: vec![metric("AWS/EC2", "CPUUtilization", &[("InstanceId", "i-0a1b2c3d4e5f60001")]), metric("AWS/SQS", "NumberOfMessagesSent", &[("QueueName", "orders")]), metric("Acme/Checkout", "OrdersPlaced", &[])],
            tags: HashMap::from([("arn:aws:ec2:us-east-1:123456789012:instance/i-0a1b2c3d4e5f60001".to_string(), vec!["team:storefront".to_string()])]),
            ..Default::default()
        };
        let sink = FakeSink::default();
        let headers = vec![("x-oc-org".to_string(), "acme".to_string())];
        let outcome = crawl(&aws, &sink, &headers, &job(None, config()), NOW, 600_000, &known_namespaces()).await;
        let end = (NOW - LAG_MS) / 60_000 * 60_000;
        assert_eq!(outcome, Outcome::Collected { end_ms: end, metrics: 1, points: 6, warning: None });
        // The first window is one interval; every statistic is read.
        let calls = aws.calls.lock().unwrap().clone();
        assert_eq!(calls.iter().map(|c| c.0.as_str()).collect::<Vec<_>>(), ["Average", "Sum", "Maximum"]);
        assert!(calls.iter().all(|c| c.2 == end - 600_000 && c.3 == end));

        let points = published(&sink);
        let names: HashSet<&str> = points.iter().map(|p| p.0.as_str()).collect();
        assert_eq!(names, HashSet::from(["aws.ec2.cpuutilization", "aws.ec2.cpuutilization.sum", "aws.ec2.cpuutilization.maximum"]));
        let (_, host, attrs) = &points[0];
        assert_eq!(host.as_deref(), Some("i-0a1b2c3d4e5f60001"));
        let attrs: HashMap<_, _> = attrs.iter().cloned().collect();
        assert_eq!(attrs["aws_account"], "123456789012");
        assert_eq!(attrs["region"], "us-east-1");
        assert_eq!(attrs["env"], "prod");
        assert_eq!(attrs["team"], "storefront");
        assert_eq!(attrs["instanceid"], "i-0a1b2c3d4e5f60001");
        let records = sink.records.lock().unwrap();
        assert_eq!(records[0].0, "acme");
        assert_eq!(records[0].1, headers);
    }

    #[tokio::test]
    async fn the_window_starts_at_the_watermark_and_is_at_most_three_hours() {
        let aws = FakeAws { metrics: vec![metric("AWS/EC2", "CPUUtilization", &[])], ..Default::default() };
        let sink = FakeSink::default();
        let end = (NOW - LAG_MS) / 60_000 * 60_000;
        crawl(&aws, &sink, &[], &job(Some(end - 120_000), config()), NOW, 600_000, &known_namespaces()).await;
        assert_eq!(aws.calls.lock().unwrap()[0].2, end - 120_000);
        let aws = FakeAws { metrics: vec![metric("AWS/EC2", "CPUUtilization", &[])], ..Default::default() };
        crawl(&aws, &sink, &[], &job(Some(end - 86_400_000), config()), NOW, 600_000, &known_namespaces()).await;
        assert_eq!(aws.calls.lock().unwrap()[0].2, end - MAX_WINDOW_MS);
        let aws = FakeAws::default();
        assert_eq!(crawl(&aws, &sink, &[], &job(Some(end), config()), NOW, 600_000, &known_namespaces()).await, Outcome::Idle);
    }

    #[tokio::test]
    async fn tag_filters_custom_namespaces_and_metric_name_filters_select_metrics() {
        let aws = FakeAws {
            metrics: vec![
                metric("AWS/EC2", "CPUUtilization", &[("InstanceId", "i-1")]),
                metric("AWS/EC2", "NetworkIn", &[("InstanceId", "i-2")]),
                metric("AWS/EC2", "NetworkOut", &[("InstanceId", "i-1")]),
                metric("Acme/Checkout", "OrdersPlaced", &[]),
            ],
            tags: HashMap::from([("arn:aws:ec2:us-east-1:123456789012:instance/i-1".to_string(), vec!["env:prod".to_string()])]),
            ..Default::default()
        };
        let sink = FakeSink::default();
        let cfg = json!({
            "namespace_filters": { "exclude_only": [] }, "collect_custom_metrics": true,
            "tag_filters": [{ "namespace": "AWS/EC2", "tags": ["env:prod"] }],
            "metric_name_filters": [{ "namespace": "AWS/EC2", "include_only": ["aws.ec2.network_*"] }],
        });
        let outcome = crawl(&aws, &sink, &[], &job(None, cfg), NOW, 600_000, &known_namespaces()).await;
        assert!(matches!(outcome, Outcome::Collected { metrics: 2, .. }), "{outcome:?}");
        let names: HashSet<String> = published(&sink).into_iter().map(|p| p.0).filter(|n| !n.contains(".sum") && !n.contains(".maximum")).collect();
        assert_eq!(names, HashSet::from(["aws.ec2.network_out".to_string(), "aws.acme_checkout.orders_placed".to_string()]));
    }

    #[tokio::test]
    async fn failures_publish_nothing_and_denied_tags_are_a_warning() {
        let denied = AwsError::AccessDenied { action: Some("cloudwatch:GetMetricData".into()), message: "not authorized to perform: cloudwatch:GetMetricData".into() };
        let aws = FakeAws { metrics: vec![metric("AWS/EC2", "CPUUtilization", &[])], fail_data: Some(denied.clone()), ..Default::default() };
        let sink = FakeSink::default();
        assert_eq!(crawl(&aws, &sink, &[], &job(None, config()), NOW, 600_000, &known_namespaces()).await, Outcome::Failed(issue(&denied)));
        assert!(sink.records.lock().unwrap().is_empty());

        let aws = FakeAws { metrics: vec![metric("AWS/EC2", "CPUUtilization", &[])], ..Default::default() };
        let broken = FakeSink { fail: true, ..Default::default() };
        assert!(matches!(crawl(&aws, &broken, &[], &job(None, config()), NOW, 600_000, &known_namespaces()).await, Outcome::Failed(Issue { message, .. }) if message.contains("Kafka")));

        let tags_denied = AwsError::AccessDenied { action: Some("tag:GetResources".into()), message: "denied".into() };
        let aws = FakeAws { metrics: vec![metric("AWS/EC2", "CPUUtilization", &[])], fail_tags: Some(tags_denied.clone()), ..Default::default() };
        let outcome = crawl(&aws, &sink, &[], &job(None, config()), NOW, 600_000, &known_namespaces()).await;
        assert!(matches!(outcome, Outcome::Collected { warning: Some(w), .. } if w == issue(&tags_denied)));
    }

    #[test]
    fn regions_and_issues() {
        assert_eq!(regions("aws", &json!({ "include_only": ["eu-west-1"] })), ["eu-west-1"]);
        assert_eq!(regions("aws", &json!({ "include_all": true })).len(), DEFAULT_REGIONS.len());
        assert_eq!(regions("aws-us-gov", &json!({ "include_all": true })), ["us-gov-west-1", "us-gov-east-1"]);
        let denied = issue(&AwsError::AccessDenied { action: Some("tag:GetResources".into()), message: "m".into() });
        assert_eq!((denied.kind, denied.missing_action.as_deref(), denied.backoff), ("access_denied", Some("tag:GetResources"), Backoff::Interval));
        assert_eq!(issue(&AwsError::Throttled("x".into())).backoff, Backoff::Exponential);
        assert_eq!(issue(&AwsError::RegionDisabled("x".into())).backoff, Backoff::Day);
        assert_eq!(issue(&AwsError::AssumeRole("x".into())).kind, "auth");
    }

    async fn account(st: &AppState, org: &str, regions: Value, enabled: bool) -> String {
        let attrs = json!({ "aws_account_id": "123456789012", "auth_config": { "access_key_id": "AKIAIOSFODNN7EXAMPLE", "secret_access_key": "wJalrXUtnFEMI" }, "aws_regions": regions, "metrics_config": { "enabled": enabled } });
        accounts::create_row(st, org, attrs.as_object().unwrap()).await.unwrap().id
    }

    /// Plans the AWS jobs, waiting for another test's planning to finish.
    async fn planned(st: &AppState) {
        let want = desired(st).await.unwrap();
        while jobs::plan(st, PROVIDER, &want).await.unwrap().is_none() {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    }

    async fn jobs_of(st: &AppState, org: &str) -> Vec<String> {
        sqlx::query_scalar("SELECT scope FROM cloud_crawl_jobs WHERE provider = 'aws' AND org_id = $1 ORDER BY scope").bind(org).fetch_all(&st.postgres).await.unwrap()
    }

    #[tokio::test]
    async fn the_jobs_follow_the_accounts_regions_and_carry_their_credentials() {
        let Some((_, states)) = crate::test_support::replicas(1).await else { return };
        let st = &states[0];
        let org = crate::test_support::new_org(st, "aws-crawl").await;
        let id = account(st, &org, json!({ "include_only": ["eu-west-1", "us-east-1"] }), true).await;
        planned(st).await;
        assert_eq!(jobs_of(st, &org).await, ["eu-west-1", "us-east-1"]);
        let key = JobKey { provider: PROVIDER.into(), org_id: org.clone(), account_id: id.clone(), scope: "eu-west-1".into(), collected_until: None };
        let job = load_job(st, &key).await.unwrap().unwrap();
        assert_eq!(job.target.region, "eu-west-1");
        assert!(matches!(&job.target.auth, TargetAuth::Keys { secret_access_key, .. } if secret_access_key == "wJalrXUtnFEMI"));

        // A region removed from the account, then collection turned off: their jobs go.
        let change = json!({ "aws_regions": { "include_only": ["eu-west-1"] } });
        accounts::update_row(st, &org, &id, change.as_object().unwrap()).await.unwrap();
        planned(st).await;
        assert_eq!(jobs_of(st, &org).await, ["eu-west-1"]);
        let off = json!({ "metrics_config": { "enabled": false } });
        accounts::update_row(st, &org, &id, off.as_object().unwrap()).await.unwrap();
        planned(st).await;
        assert!(jobs_of(st, &org).await.is_empty());
    }

    #[tokio::test]
    async fn the_watermark_advances_only_after_publishing_and_errors_become_issues() {
        let Some((_, states)) = crate::test_support::replicas(1).await else { return };
        let st = &states[0];
        let org = crate::test_support::new_org(st, "aws-watermark").await;
        let id = account(st, &org, json!({ "include_only": ["us-east-1"] }), true).await;
        planned(st).await;
        let key = JobKey { provider: PROVIDER.into(), org_id: org.clone(), account_id: id.clone(), scope: "us-east-1".into(), collected_until: None };
        let metrics = vec![metric("AWS/EC2", "CPUUtilization", &[("InstanceId", "i-1")])];
        let crawler = |api: FakeAws, sink: Arc<FakeSink>| AwsCrawler { api: Arc::new(api), sink, interval_secs: 600 };
        let status = || async { jobs::status(st, PROVIDER, &org, &id).await.unwrap().remove(0) };

        // Kafka down: the window stays unread.
        let broken = Arc::new(FakeSink { fail: true, ..Default::default() });
        let c = crawler(FakeAws { metrics: metrics.clone(), ..Default::default() }, broken);
        jobs::record(st, &key, &c.run(st, &key, NOW).await.unwrap(), 600).await.unwrap();
        let s = status().await;
        assert_eq!((s.collected_until, s.error.as_ref().map(|e| e.kind.clone())), (None, Some("other".to_string())));

        // Published: the watermark is the window's end, the issue is gone, the org's headers went along.
        let sink = Arc::new(FakeSink::default());
        let c = crawler(FakeAws { metrics: metrics.clone(), ..Default::default() }, sink.clone());
        jobs::record(st, &key, &c.run(st, &key, NOW).await.unwrap(), 600).await.unwrap();
        let s = status().await;
        assert_eq!((s.collected_until, s.error.is_none()), (Some((NOW - LAG_MS) / 60_000 * 60_000), true));
        assert_eq!(sink.records.lock().unwrap()[0].1[0], ("x-oc-org".to_string(), org.clone()));

        // A missing permission: the issue names the action, the watermark stays.
        let denied = FakeAws { metrics, fail_list: Some(AwsError::AccessDenied { action: Some("cloudwatch:ListMetrics".into()), message: "denied".into() }), ..Default::default() };
        let c = crawler(denied, sink);
        jobs::record(st, &key, &c.run(st, &key, NOW + 600_000).await.unwrap(), 600).await.unwrap();
        let s = status().await;
        assert_eq!((s.collected_until, s.error.as_ref().and_then(|e| e.missing_action.clone())), (Some((NOW - LAG_MS) / 60_000 * 60_000), Some("cloudwatch:ListMetrics".to_string())));
    }
}
