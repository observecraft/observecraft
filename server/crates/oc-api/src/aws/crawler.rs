//! The AWS integration's CloudWatch crawler: every configured account's metrics, every
//! `OC_CLOUD_CRAWL_INTERVAL_SECS` (10 minutes), published to Kafka like everything the intake accepts.
//! Its jobs (one per account and region) are the cloud crawlers' (`crate::cloud::jobs`): claimed with
//! `FOR UPDATE SKIP LOCKED`, so adding servers adds crawl capacity.
//!
//! - **A run** ([`crawl`]) lists the region's metrics with data in the last 3 hours, keeps those the
//!   account selects (namespaces, metric names, resource tags), and reads their statistics per minute over
//!   the window since the watermark (at most 3 hours, ending 5 minutes ago: CloudWatch publishes late),
//!   500 queries (a metric's statistic) per GetMetricData call. Points published after the window has moved
//!   on aren't read again: the metric tables keep every point they get (`otel.metrics_samples` is a plain
//!   MergeTree), so re-reading an overlap would count its points twice.
//! - **Daily metrics** (S3's storage metrics and Storage Lens, `period` 86400 in `aws-metrics.json`) have one
//!   point a day, published for the previous day and not "recently active": the first run of each hour lists
//!   their namespaces whole and reads their last 3 days a day apart; each series' latest published day is kept
//!   (Postgres `aws_daily_metric_marks`), so each day is published once.
//! - **Publishing.** Points become OTLP JSON gauges named as Datadog names them: the bare name carries the
//!   metric's Datadog statistic (counts' Sum: `aws.sqs.number_of_messages_sent`, gauges' Average:
//!   `aws.ec2.cpuutilization`), with `.sum`, `.maximum`, `.minimum` and the latencies' percentiles (`.p95`),
//!   tagged with the account, the region, the account's tags, the dimensions and the resource's AWS tags
//!   (`naming::resource_arn`), written to `oc.otlp-json.metrics` keyed by org with the org's headers.
//!   The watermark advances only after Kafka acknowledged every record of the window, so a crash or a
//!   Kafka outage re-reads the window instead of losing it.
//! - **Global services** (CloudFront, the web ACLs of CloudFront distributions, Route 53 health checks,
//!   billing) have their metrics in us-east-1 only: an account whose regions leave us-east-1 out still
//!   gets a us-east-1 job, which collects only those (`naming::global_series`).
//! - **Failures** become the account's issues: a missing permission (with the action AWS names), a role
//!   that can't be assumed, throttling (exponential backoff), a disabled region (retried daily). Tag
//!   lookups that are denied don't stop the run.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;

use serde_json::Value;

use super::api::{AwsApi, AwsError, MetricId, Target, TargetAuth};
use super::naming::{self, Point};
use crate::cloud::jobs::{self, Backoff, Crawler, Issue, JobKey, MetricSink, Outcome};
use crate::routes::datadog_api::aws_integration as accounts;
use crate::state::AppState;

pub const PROVIDER: &str = "aws";
/// CloudWatch's publishing delay: the window ends this long before now.
const LAG_MS: i64 = 5 * 60_000;
/// The longest window one run reads (a longer gap starts 3 hours back).
const MAX_WINDOW_MS: i64 = 3 * 3_600_000;
/// GetMetricData: queries (a metric's statistic) per call.
const BATCH: usize = 500;
const HOUR_MS: i64 = 3_600_000;
const DAY_MS: i64 = 86_400_000;
/// The days of daily metrics each daily pass reads.
const DAILY_DAYS: i64 = 3;
/// Points per Kafka record.
pub const POINTS_PER_RECORD: usize = 10_000;
/// Regions an account with `include_all` is crawled in: the regions every account has enabled (opt-in
/// regions are crawled when the account lists them).
const DEFAULT_REGIONS: [&str; 17] = [
    "us-east-1", "us-east-2", "us-west-1", "us-west-2", "ap-south-1", "ap-northeast-1", "ap-northeast-2", "ap-northeast-3", "ap-southeast-1", "ap-southeast-2",
    "ca-central-1", "eu-central-1", "eu-west-1", "eu-west-2", "eu-west-3", "eu-north-1", "sa-east-1",
];

/// The region CloudWatch keeps the global services' metrics in.
const GLOBAL_REGION: &str = "us-east-1";

/// The regions an account's jobs run in: its regions, and us-east-1 for the global services.
pub fn job_regions(partition: &str, aws_regions: &Value) -> Vec<String> {
    let mut all = regions(partition, aws_regions);
    if partition == "aws" && !all.iter().any(|r| r == GLOBAL_REGION) {
        all.push(GLOBAL_REGION.to_string());
    }
    all
}

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
        .flat_map(|(org, id, partition, aws_regions, _)| job_regions(&partition, &aws_regions).into_iter().map(move |region| (org.clone(), id.clone(), region)))
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
    /// A region the account doesn't collect, crawled for the global services only.
    pub global_only: bool,
    /// The latest day published per daily series ([`daily_key`] → epoch ms; Postgres `aws_daily_metric_marks`).
    pub daily_marks: HashMap<String, i64>,
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
        global_only: !regions(&account.aws_partition, &account.aws_regions).contains(&key.scope),
        daily_marks: daily_marks(st, key).await?,
        org_id: key.org_id.clone(),
        collected_until: key.collected_until,
    }))
}

async fn daily_marks(st: &AppState, key: &JobKey) -> Result<HashMap<String, i64>, sqlx::Error> {
    let marks: Option<Value> = sqlx::query_scalar("SELECT marks FROM aws_daily_metric_marks WHERE org_id = $1 AND account_id = $2::uuid AND region = $3")
        .bind(&key.org_id)
        .bind(&key.account_id)
        .bind(&key.scope)
        .fetch_optional(&st.postgres)
        .await?;
    Ok(marks.and_then(|m| serde_json::from_value(m).ok()).unwrap_or_default())
}

/// Keeps a job's daily marks (once their points are published).
pub async fn save_daily_marks(st: &AppState, key: &JobKey, marks: &HashMap<String, i64>) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO aws_daily_metric_marks (org_id, account_id, region, marks) VALUES ($1, $2::uuid, $3, $4) \
         ON CONFLICT (org_id, account_id, region) DO UPDATE SET marks = EXCLUDED.marks, updated_at = now()",
    )
    .bind(&key.org_id)
    .bind(&key.account_id)
    .bind(&key.scope)
    .bind(serde_json::to_value(marks).unwrap_or_default())
    .execute(&st.postgres)
    .await?;
    Ok(())
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

/// A run's outcome and, when it read the daily metrics, the marks to keep (see [`Job::daily_marks`]).
#[derive(Debug, PartialEq)]
pub struct Crawl {
    pub outcome: Outcome,
    pub daily_marks: Option<HashMap<String, i64>>,
}

impl From<Outcome> for Crawl {
    fn from(outcome: Outcome) -> Self {
        Crawl { outcome, daily_marks: None }
    }
}

/// A daily series' key in [`Job::daily_marks`]: namespace, metric and dimensions.
pub fn daily_key(m: &MetricId) -> String {
    let dims: Vec<String> = m.dimensions.iter().map(|(k, v)| format!("{k}={v}")).collect();
    format!("{}|{}|{}", m.namespace, m.name, dims.join(","))
}

/// Each query's points as published: its names, the resource's host (EC2 instances) and tags.
fn to_points(metrics: &[(MetricId, Vec<String>)], queries: &[(usize, String)], series: Vec<super::api::Series>, base: &[(String, String)], out: &mut Vec<(usize, Point)>) {
    for s in series {
        let Some((i, stat)) = queries.get(s.metric) else { continue };
        let (m, resource) = &metrics[*i];
        let names = naming::series_names(&m.namespace, &m.name, &m.dimensions, stat);
        if names.is_empty() {
            continue;
        }
        let host = m.dimensions.iter().find(|(k, _)| m.namespace == "AWS/EC2" && k == "InstanceId").map(|(_, v)| v.clone());
        let mut tags: BTreeMap<String, String> = base.iter().cloned().collect();
        tags.extend(resource.iter().map(|t| split_tag(t)));
        tags.extend(m.dimensions.iter().map(|(k, v)| (naming::tag_key(k), v.clone())));
        let tags: Vec<(String, String)> = tags.into_iter().collect();
        for name in &names {
            out.extend(s.points.iter().map(|&(time_ms, value)| (*i, Point { name: name.clone(), host: host.clone(), tags: tags.clone(), time_ms, value })));
        }
    }
}

/// Every statistic of `metrics` over `[start, end)`, `period` seconds apart, [`BATCH`] queries per call: points
/// with the index of their metric.
async fn read(api: &dyn AwsApi, t: &Target, metrics: &[(MetricId, Vec<String>)], start: i64, end: i64, period: i32, base: &[(String, String)]) -> Result<Vec<(usize, Point)>, AwsError> {
    let queries: Vec<(usize, String)> = metrics.iter().enumerate().flat_map(|(i, (m, _))| naming::statistics(&m.namespace, &m.name).into_iter().map(move |s| (i, s))).collect();
    let mut out = vec![];
    for chunk in queries.chunks(BATCH) {
        let asked: Vec<(MetricId, String)> = chunk.iter().map(|(i, s)| (metrics[*i].0.clone(), s.clone())).collect();
        let series = api.get_metric_data(t, &asked, start, end, period).await?;
        to_points(metrics, chunk, series, base, &mut out);
    }
    Ok(out)
}

/// Reads and publishes one job's window, and on its first run of each hour the daily metrics' last 3 days.
pub async fn crawl(api: &dyn AwsApi, sink: &dyn MetricSink, headers: &[(String, String)], job: &Job, now_ms: i64, interval_ms: i64, known: &HashSet<&str>) -> Crawl {
    let end = (now_ms - LAG_MS) / 60_000 * 60_000;
    let start = job.collected_until.unwrap_or(end - interval_ms).max(end - MAX_WINDOW_MS);
    if end <= start {
        return Outcome::Idle.into();
    }
    let config = &job.metrics_config;
    let custom = config["collect_custom_metrics"].as_bool().unwrap_or(false);
    let t = &job.target;
    let selected = |m: &MetricId| {
        naming::series_collected(&m.namespace, &m.dimensions)
            && (!job.global_only || naming::global_series(&m.namespace, &m.dimensions))
            && naming::namespace_selected(&config["namespace_filters"], &m.namespace, known.contains(m.namespace.as_str()), custom)
            && naming::metric_selected(&config["metric_name_filters"], &m.namespace, &naming::metric_name(&m.namespace, &m.name))
    };
    let listed = match api.list_metrics(t).await {
        Ok(m) => m,
        Err(e) => return Outcome::Failed(issue(&e)).into(),
    };
    let regular: Vec<MetricId> = listed.into_iter().filter(|m| naming::period(&m.namespace, &m.name) == naming::MINUTE && selected(m)).collect();
    // The daily metrics, once an hour (the run whose window starts a new hour, and the first run).
    let daily_due = !job.global_only && (job.collected_until.is_none() || start / HOUR_MS != end / HOUR_MS);
    let mut daily = vec![];
    if daily_due {
        for ns in naming::daily_namespaces().into_iter().filter(|ns| naming::namespace_selected(&config["namespace_filters"], ns, known.contains(ns), custom)) {
            match api.list_namespace_metrics(t, ns).await {
                Ok(ms) => daily.extend(ms.into_iter().filter(|m| naming::period(&m.namespace, &m.name) == naming::DAY && selected(m))),
                Err(e) => return Outcome::Failed(issue(&e)).into(),
            }
        }
    }
    if regular.is_empty() && daily.is_empty() {
        return Outcome::Collected { end_ms: end, metrics: 0, points: 0, warning: None }.into();
    }
    let (tags_by_arn, warning) = match api.resource_tags(t).await {
        Ok(tags) => (tags, None),
        Err(e @ AwsError::AccessDenied { .. }) => (HashMap::new(), Some(issue(&e))),
        Err(e) => return Outcome::Failed(issue(&e)).into(),
    };
    let index = naming::TagIndex::new(&tags_by_arn);
    let mut found: HashMap<String, Vec<String>> = HashMap::new();
    let mut tagged = |ms: Vec<MetricId>| -> Vec<(MetricId, Vec<String>)> {
        ms.into_iter()
            .map(|m| {
                let tags = naming::resource_arn(&t.partition, &t.region, &t.account_id, &m.namespace, &m.dimensions)
                    .map(|arn| found.entry(arn.clone()).or_insert_with(|| index.get(&arn)).clone())
                    .unwrap_or_default();
                (m, tags)
            })
            .filter(|(m, tags)| naming::tags_match(&config["tag_filters"], &m.namespace, tags))
            .collect()
    };
    let (regular, daily) = (tagged(regular), tagged(daily));

    let base: Vec<(String, String)> = [("aws_account".to_string(), t.account_id.clone()), ("region".to_string(), t.region.clone())].into_iter().chain(job.account_tags.iter().map(|t| split_tag(t))).collect();
    let mut points: Vec<Point> = match read(api, t, &regular, start, end, naming::MINUTE, &base).await {
        Ok(p) => p.into_iter().map(|(_, p)| p).collect(),
        Err(e) => return Outcome::Failed(issue(&e)).into(),
    };
    // Daily points are re-read for 3 days (CloudWatch publishes a day's late) and published once: the marks keep
    // each series' latest published day.
    let mut daily_marks = None;
    if daily_due {
        let day_start = end / DAY_MS * DAY_MS - DAILY_DAYS * DAY_MS;
        let read = match read(api, t, &daily, day_start, end, naming::DAY, &base).await {
            Ok(p) => p,
            Err(e) => return Outcome::Failed(issue(&e)).into(),
        };
        let mut marks: HashMap<String, i64> = job.daily_marks.iter().filter(|(_, &at)| at >= day_start).map(|(k, v)| (k.clone(), *v)).collect();
        let keys: Vec<String> = daily.iter().map(|(m, _)| daily_key(m)).collect();
        let mut newest: HashMap<usize, i64> = HashMap::new();
        for (i, p) in read {
            if job.daily_marks.get(&keys[i]).is_some_and(|&at| p.time_ms <= at) {
                continue;
            }
            let n = newest.entry(i).or_insert(p.time_ms);
            *n = (*n).max(p.time_ms);
            points.push(p);
        }
        marks.extend(newest.into_iter().map(|(i, at)| (keys[i].clone(), at)));
        daily_marks = Some(marks);
    }
    for record in points.chunks(POINTS_PER_RECORD) {
        if let Err(e) = sink.publish(&job.org_id, headers, crate::cloud::naming::otlp_json(PROVIDER, record)).await {
            return Outcome::Failed(Issue { kind: "other", message: format!("publishing to Kafka failed: {e}"), missing_action: None, backoff: Backoff::Exponential }).into();
        }
    }
    Crawl { outcome: Outcome::Collected { end_ms: end, metrics: regular.len() + daily.len(), points: points.len(), warning }, daily_marks }
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
        let run = crawl(self.api.as_ref(), self.sink.as_ref(), &headers, &job, now_ms, self.interval_secs * 1000, &known_namespaces()).await;
        if let (Outcome::Collected { .. }, Some(marks)) = (&run.outcome, &run.daily_marks) {
            save_daily_marks(st, key, marks).await?;
        }
        Ok(run.outcome)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::Mutex as StdMutex;

    use super::super::api::Series;
    use crate::cloud::testing::{published, FakeSink};

    /// A GetMetricData call: its statistics, start, end and period.
    type Call = (Vec<String>, i64, i64, i32);

    /// AWS as a test sets it up: metrics (recently active, and every metric of a namespace), their values, tags, and
    /// errors to return.
    #[derive(Default)]
    struct FakeAws {
        metrics: Vec<MetricId>,
        /// Metrics ListMetrics lists only without its "recently active" filter (daily ones).
        inactive: Vec<MetricId>,
        tags: HashMap<String, Vec<String>>,
        fail_list: Option<AwsError>,
        fail_data: Option<AwsError>,
        fail_tags: Option<AwsError>,
        /// Each GetMetricData call: its statistics, start, end and period.
        calls: StdMutex<Vec<Call>>,
    }

    #[async_trait::async_trait]
    impl AwsApi for FakeAws {
        async fn list_metrics(&self, _t: &Target) -> Result<Vec<MetricId>, AwsError> {
            self.fail_list.clone().map_or(Ok(self.metrics.clone()), Err)
        }
        async fn list_namespace_metrics(&self, _t: &Target, namespace: &str) -> Result<Vec<MetricId>, AwsError> {
            Ok(self.metrics.iter().chain(&self.inactive).filter(|m| m.namespace == namespace).cloned().collect())
        }
        async fn get_metric_data(&self, _t: &Target, queries: &[(MetricId, String)], start: i64, end: i64, period: i32) -> Result<Vec<Series>, AwsError> {
            if let Some(e) = &self.fail_data {
                return Err(e.clone());
            }
            self.calls.lock().unwrap().push((queries.iter().map(|q| q.1.clone()).collect(), start, end, period));
            let step = period as i64 * 1000;
            Ok((0..queries.len()).map(|i| Series { metric: i, points: vec![(start, i as f64 + 1.0), (start + step, i as f64 + 2.0)] }).collect())
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
            global_only: false,
            daily_marks: HashMap::new(),
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
        let outcome = crawl(&aws, &sink, &headers, &job(None, config()), NOW, 600_000, &known_namespaces()).await.outcome;
        let end = (NOW - LAG_MS) / 60_000 * 60_000;
        assert_eq!(outcome, Outcome::Collected { end_ms: end, metrics: 1, points: 8, warning: None });
        // The first window is one interval; every statistic is read, in one call.
        let calls = aws.calls.lock().unwrap().clone();
        assert_eq!(calls, [(vec!["Average".to_string(), "Sum".into(), "Maximum".into(), "Minimum".into()], end - 600_000, end, 60)]);

        let points = published(&sink);
        let names: HashSet<&str> = points.iter().map(|p| p.0.as_str()).collect();
        assert_eq!(names, HashSet::from(["aws.ec2.cpuutilization", "aws.ec2.cpuutilization.sum", "aws.ec2.cpuutilization.maximum", "aws.ec2.cpuutilization.minimum"]));
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
        let aws = FakeAws { metrics: vec![metric("AWS/EC2", "CPUUtilization", &[("InstanceId", "i-1")])], ..Default::default() };
        let sink = FakeSink::default();
        let end = (NOW - LAG_MS) / 60_000 * 60_000;
        crawl(&aws, &sink, &[], &job(Some(end - 120_000), config()), NOW, 600_000, &known_namespaces()).await;
        assert_eq!(aws.calls.lock().unwrap()[0].1, end - 120_000);
        let aws = FakeAws { metrics: vec![metric("AWS/EC2", "CPUUtilization", &[("InstanceId", "i-1")])], ..Default::default() };
        crawl(&aws, &sink, &[], &job(Some(end - 86_400_000), config()), NOW, 600_000, &known_namespaces()).await;
        assert_eq!(aws.calls.lock().unwrap()[0].1, end - MAX_WINDOW_MS);
        let aws = FakeAws::default();
        assert_eq!(crawl(&aws, &sink, &[], &job(Some(end), config()), NOW, 600_000, &known_namespaces()).await.outcome, Outcome::Idle);
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
        let outcome = crawl(&aws, &sink, &[], &job(None, cfg), NOW, 600_000, &known_namespaces()).await.outcome;
        assert!(matches!(outcome, Outcome::Collected { metrics: 2, .. }), "{outcome:?}");
        let names: HashSet<String> = published(&sink).into_iter().map(|p| p.0).filter(|n| ![".sum", ".maximum", ".minimum"].iter().any(|s| n.ends_with(s))).collect();
        assert_eq!(names, HashSet::from(["aws.ec2.network_out".to_string(), "aws.acme_checkout.orders_placed".to_string()]));
    }

    #[tokio::test]
    async fn failures_publish_nothing_and_denied_tags_are_a_warning() {
        let denied = AwsError::AccessDenied { action: Some("cloudwatch:GetMetricData".into()), message: "not authorized to perform: cloudwatch:GetMetricData".into() };
        let cpu = || vec![metric("AWS/EC2", "CPUUtilization", &[("InstanceId", "i-1")])];
        let aws = FakeAws { metrics: cpu(), fail_data: Some(denied.clone()), ..Default::default() };
        let sink = FakeSink::default();
        assert_eq!(crawl(&aws, &sink, &[], &job(None, config()), NOW, 600_000, &known_namespaces()).await, Crawl { outcome: Outcome::Failed(issue(&denied)), daily_marks: None });
        assert!(sink.records.lock().unwrap().is_empty());

        let aws = FakeAws { metrics: cpu(), ..Default::default() };
        let broken = FakeSink { fail: true, ..Default::default() };
        assert!(matches!(crawl(&aws, &broken, &[], &job(None, config()), NOW, 600_000, &known_namespaces()).await.outcome, Outcome::Failed(Issue { message, .. }) if message.contains("Kafka")));

        let tags_denied = AwsError::AccessDenied { action: Some("tag:GetResources".into()), message: "denied".into() };
        let aws = FakeAws { metrics: cpu(), fail_tags: Some(tags_denied.clone()), ..Default::default() };
        let outcome = crawl(&aws, &sink, &[], &job(None, config()), NOW, 600_000, &known_namespaces()).await.outcome;
        assert!(matches!(outcome, Outcome::Collected { warning: Some(w), .. } if w == issue(&tags_denied)));
    }

    #[test]
    fn regions_and_issues() {
        assert_eq!(regions("aws", &json!({ "include_only": ["eu-west-1"] })), ["eu-west-1"]);
        assert_eq!(regions("aws", &json!({ "include_all": true })).len(), DEFAULT_REGIONS.len());
        assert_eq!(regions("aws-us-gov", &json!({ "include_all": true })), ["us-gov-west-1", "us-gov-east-1"]);
        // Global services live in us-east-1: an account without it still gets a job there.
        assert_eq!(job_regions("aws", &json!({ "include_only": ["eu-west-1"] })), ["eu-west-1", "us-east-1"]);
        assert_eq!(job_regions("aws", &json!({ "include_only": ["us-east-1", "eu-west-1"] })), ["us-east-1", "eu-west-1"]);
        assert_eq!(job_regions("aws-cn", &json!({ "include_all": true })), ["cn-north-1", "cn-northwest-1"]);
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
        assert_eq!(jobs_of(st, &org).await, ["eu-west-1", "us-east-1"], "us-east-1 stays for the global services");
        let key = JobKey { scope: "us-east-1".into(), ..key };
        assert!(load_job(st, &key).await.unwrap().unwrap().global_only);
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
        let metrics = vec![metric("AWS/EC2", "CPUUtilization", &[("InstanceId", "i-1")]), metric("AWS/S3", "BucketSizeBytes", &[("BucketName", "example-assets"), ("StorageType", "StandardStorage")])];
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
        // The daily metrics' marks are kept for the next run.
        let marks = load_job(st, &key).await.unwrap().unwrap().daily_marks;
        assert_eq!(marks.keys().collect::<Vec<_>>(), ["AWS/S3|BucketSizeBytes|BucketName=example-assets,StorageType=StandardStorage"]);

        // A missing permission: the issue names the action, the watermark stays.
        let denied = FakeAws { metrics, fail_list: Some(AwsError::AccessDenied { action: Some("cloudwatch:ListMetrics".into()), message: "denied".into() }), ..Default::default() };
        let c = crawler(denied, sink);
        jobs::record(st, &key, &c.run(st, &key, NOW + 600_000).await.unwrap(), 600).await.unwrap();
        let s = status().await;
        assert_eq!((s.collected_until, s.error.as_ref().and_then(|e| e.missing_action.clone())), (Some((NOW - LAG_MS) / 60_000 * 60_000), Some("cloudwatch:ListMetrics".to_string())));
    }

    #[tokio::test]
    async fn the_four_web_services_are_published_as_datadog_names_and_tags_them() {
        let aws = FakeAws {
            metrics: vec![
                metric("AWS/CloudFront", "Requests", &[("DistributionId", "E1A2B3C4D5E6F7"), ("Region", "Global")]),
                metric("AWS/WAFV2", "BlockedRequests", &[("Region", "us-east-1"), ("Rule", "ALL"), ("WebACL", "storefront-alb")]),
                metric("AWS/ECS", "CPUUtilization", &[("ClusterName", "storefront"), ("ServiceName", "order-service")]),
                metric("ECS/ContainerInsights", "RunningTaskCount", &[("ClusterName", "storefront"), ("ServiceName", "order-service")]),
                metric("AWS/RDS", "CPUUtilization", &[("DBInstanceIdentifier", "orders-db")]),
                metric("AWS/RDS", "CPUUtilization", &[("EngineName", "postgres")]),
                metric("AWS/RDS", "VolumeBytesUsed", &[("DBClusterIdentifier", "orders")]),
            ],
            tags: HashMap::from([
                ("arn:aws:cloudfront::123456789012:distribution/E1A2B3C4D5E6F7".to_string(), vec!["service:storefront-api".to_string()]),
                ("arn:aws:ecs:us-east-1:123456789012:service/storefront/order-service".to_string(), vec!["team:orders".to_string()]),
            ]),
            ..Default::default()
        };
        let sink = FakeSink::default();
        let outcome = crawl(&aws, &sink, &[], &job(None, config()), NOW, 600_000, &known_namespaces()).await.outcome;
        assert!(matches!(outcome, Outcome::Collected { metrics: 6, .. }), "the RDS engine aggregate is left out: {outcome:?}");
        let points = published(&sink);
        let names: std::collections::BTreeSet<&str> = points.iter().map(|p| p.0.as_str()).collect();
        for name in [
            "aws.cloudfront.requests",
            "aws.cloudfront.requests.sum",
            "aws.wafv2.blocked_requests",
            "aws.ecs.cpuutilization",
            "aws.ecs.service.cpuutilization",
            "aws.ecs.service.cpuutilization.maximum",
            "ecs.containerinsights.running_task_count",
            "aws.rds.cpuutilization",
            "aws.rds.volume_bytes_used",
        ] {
            assert!(names.contains(name), "{name} in {names:?}");
        }
        // CloudFront's request count is published once per point, from the Sum (the Average names nothing).
        let count = |n: &str| points.iter().filter(|p| p.0 == n).count();
        assert_eq!((count("aws.cloudfront.requests"), count("aws.cloudfront.requests.sum")), (2, 2));
        let attrs = |n: &str| points.iter().find(|p| p.0 == n).map(|p| p.2.iter().cloned().collect::<HashMap<_, _>>()).unwrap();
        assert_eq!(attrs("aws.cloudfront.requests")["distributionid"], "E1A2B3C4D5E6F7");
        assert_eq!(attrs("aws.cloudfront.requests")["service"], "storefront-api");
        assert_eq!((attrs("aws.wafv2.blocked_requests")["webacl"].as_str(), attrs("aws.wafv2.blocked_requests")["rule"].as_str()), ("storefront-alb", "ALL"));
        let ecs = attrs("aws.ecs.service.cpuutilization");
        assert_eq!((ecs["clustername"].as_str(), ecs["servicename"].as_str(), ecs["team"].as_str()), ("storefront", "order-service", "orders"));
        assert_eq!(attrs("aws.rds.cpuutilization")["dbinstanceidentifier"], "orders-db");
        assert_eq!(attrs("aws.rds.volume_bytes_used")["dbclusteridentifier"], "orders");
    }

    #[tokio::test]
    async fn a_region_the_account_leaves_out_collects_only_the_global_services() {
        let aws = FakeAws {
            metrics: vec![
                metric("AWS/CloudFront", "Requests", &[("DistributionId", "E1A2B3C4D5E6F7"), ("Region", "Global")]),
                metric("AWS/WAFV2", "AllowedRequests", &[("Rule", "ALL"), ("WebACL", "storefront-cdn")]),
                metric("AWS/WAFV2", "AllowedRequests", &[("Region", "us-east-1"), ("Rule", "ALL"), ("WebACL", "storefront-alb")]),
                metric("AWS/EC2", "CPUUtilization", &[("InstanceId", "i-0a1b2c3d4e5f60001")]),
            ],
            ..Default::default()
        };
        let sink = FakeSink::default();
        let global = Job { global_only: true, ..job(None, config()) };
        let outcome = crawl(&aws, &sink, &[], &global, NOW, 600_000, &known_namespaces()).await.outcome;
        assert!(matches!(outcome, Outcome::Collected { metrics: 2, .. }), "{outcome:?}");
        let webacls: HashSet<String> = published(&sink).into_iter().filter_map(|p| p.2.into_iter().find(|(k, _)| k == "webacl").map(|(_, v)| v)).collect();
        assert_eq!(webacls, HashSet::from(["storefront-cdn".to_string()]));
    }

    #[tokio::test]
    async fn ec2_aggregates_are_left_out_and_latencies_get_their_percentiles() {
        let aws = FakeAws {
            metrics: vec![
                metric("AWS/EC2", "CPUUtilization", &[("InstanceId", "i-0a1b2c3d4e5f60001")]),
                metric("AWS/EC2", "CPUUtilization", &[("AutoScalingGroupName", "storefront-web")]),
                metric("AWS/EC2", "CPUUtilization", &[]),
                metric("AWS/ApplicationELB", "TargetResponseTime", &[("LoadBalancer", "app/storefront/50dc6c495c0c9188")]),
                metric("AWS/ApplicationELB", "RequestCount", &[("LoadBalancer", "app/storefront/50dc6c495c0c9188")]),
            ],
            ..Default::default()
        };
        let sink = FakeSink::default();
        let outcome = crawl(&aws, &sink, &[], &job(None, config()), NOW, 600_000, &known_namespaces()).await.outcome;
        assert!(matches!(outcome, Outcome::Collected { metrics: 3, .. }), "{outcome:?}");
        let points = published(&sink);
        assert!(points.iter().filter(|p| p.0.starts_with("aws.ec2.")).all(|p| p.1.is_some()), "every EC2 point has its host");
        let names: HashSet<&str> = points.iter().map(|p| p.0.as_str()).collect();
        for name in [
            "aws.applicationelb.target_response_time",
            "aws.applicationelb.target_response_time.p50",
            "aws.applicationelb.target_response_time.p99",
            "aws.applicationelb.request_count",
            "aws.applicationelb.request_count.sum",
        ] {
            assert!(names.contains(name), "{name} in {names:?}");
        }
        assert!(!names.contains("aws.applicationelb.request_count.p95"));
    }

    #[tokio::test]
    async fn resource_tags_reach_every_namespace_mapped_and_negated_filters_keep_untagged_resources() {
        let aws = FakeAws {
            metrics: vec![
                metric("AWS/EBS", "VolumeReadOps", &[("VolumeId", "vol-0a1b2c3d4e5f60001")]),
                metric("AWS/EBS", "VolumeReadOps", &[("VolumeId", "vol-0a1b2c3d4e5f60002")]),
                metric("AWS/EBS", "VolumeReadOps", &[("VolumeId", "vol-0a1b2c3d4e5f60003")]),
                metric("AWS/Kafka", "BytesInPerSec", &[("Broker ID", "1"), ("Cluster Name", "orders")]),
            ],
            tags: HashMap::from([
                ("arn:aws:ec2:us-east-1:123456789012:volume/vol-0a1b2c3d4e5f60001".to_string(), vec!["team:orders".to_string()]),
                ("arn:aws:ec2:us-east-1:123456789012:volume/vol-0a1b2c3d4e5f60002".to_string(), vec!["datadog:no".to_string()]),
                ("arn:aws:kafka:us-east-1:123456789012:cluster/orders/0a1b2c3d-0000-4000-8000-000000000001-2".to_string(), vec!["service:order-service".to_string()]),
            ]),
            ..Default::default()
        };
        let sink = FakeSink::default();
        let cfg = json!({ "namespace_filters": { "exclude_only": [] }, "tag_filters": [{ "namespace": "AWS/EBS", "tags": ["!datadog:no"] }] });
        let outcome = crawl(&aws, &sink, &[], &job(None, cfg), NOW, 600_000, &known_namespaces()).await.outcome;
        assert!(matches!(outcome, Outcome::Collected { metrics: 3, .. }), "the volume tagged datadog:no is left out: {outcome:?}");
        let points = published(&sink);
        let attrs = |volume: &str| points.iter().find(|p| p.2.iter().any(|(k, v)| k == "volumeid" && v == volume)).map(|p| p.2.iter().cloned().collect::<HashMap<_, _>>());
        assert_eq!(attrs("vol-0a1b2c3d4e5f60001").unwrap()["team"], "orders");
        assert!(attrs("vol-0a1b2c3d4e5f60003").is_some(), "an untagged volume passes a negated-only filter");
        assert!(attrs("vol-0a1b2c3d4e5f60002").is_none());
        let kafka = points.iter().find(|p| p.0.starts_with("aws.kafka.")).unwrap();
        assert!(kafka.2.contains(&("service".to_string(), "order-service".to_string())));
    }

    #[tokio::test]
    async fn daily_metrics_are_read_once_an_hour_for_three_days_and_each_day_is_published_once() {
        let bucket = metric("AWS/S3", "BucketSizeBytes", &[("BucketName", "example-assets"), ("StorageType", "StandardStorage")]);
        // S3's storage metrics aren't "recently active": only a namespace's whole listing has them.
        let aws = || FakeAws { metrics: vec![metric("AWS/EC2", "CPUUtilization", &[("InstanceId", "i-1")])], inactive: vec![bucket.clone()], ..Default::default() };
        let end = (NOW - LAG_MS) / 60_000 * 60_000;
        let day_start = end / DAY_MS * DAY_MS - 3 * DAY_MS;

        // The first run reads them, a day apart over the last 3 days, and keeps each series' latest day.
        let (first, sink) = (aws(), FakeSink::default());
        let run = crawl(&first, &sink, &[], &job(None, config()), NOW, 600_000, &known_namespaces()).await;
        let calls = first.calls.lock().unwrap().clone();
        assert_eq!(calls.len(), 2);
        assert_eq!((calls[1].1, calls[1].2, calls[1].3), (day_start, end, 86_400));
        assert!(!calls[0].0.is_empty() && calls[1].0 == ["Average", "Sum", "Maximum", "Minimum"]);
        let sizes: Vec<_> = published(&sink).into_iter().filter(|p| p.0 == "aws.s3.bucket_size_bytes").collect();
        assert_eq!(sizes.len(), 2);
        assert_eq!(sizes[0].2.iter().find(|(k, _)| k == "bucketname").unwrap().1, "example-assets");
        let marks = run.daily_marks.unwrap();
        assert_eq!(marks, HashMap::from([(daily_key(&bucket), day_start + DAY_MS)]));

        // A run within the hour doesn't read them…
        let (within, sink) = (aws(), FakeSink::default());
        let mid_hour = end / HOUR_MS * HOUR_MS + 20 * 60_000;
        let later = NOW - end + mid_hour + 10 * 60_000;
        let run = crawl(&within, &sink, &[], &Job { daily_marks: marks.clone(), ..job(Some(mid_hour), config()) }, later, 600_000, &known_namespaces()).await;
        assert_eq!((within.calls.lock().unwrap().len(), run.daily_marks), (1, None));

        // …the next hour's first run does, and publishes only the days it hasn't yet.
        let (next, sink) = (aws(), FakeSink::default());
        let crossing = Job { daily_marks: HashMap::from([(daily_key(&bucket), day_start)]), ..job(Some(end / HOUR_MS * HOUR_MS + HOUR_MS - 300_000), config()) };
        let at = NOW - end + end / HOUR_MS * HOUR_MS + HOUR_MS + 300_000;
        let run = crawl(&next, &sink, &[], &crossing, at, 600_000, &known_namespaces()).await;
        let sizes: Vec<_> = published(&sink).into_iter().filter(|p| p.0 == "aws.s3.bucket_size_bytes").collect();
        assert_eq!(sizes.len(), 1, "the day already published isn't again");
        assert!(run.daily_marks.unwrap()[&daily_key(&bucket)] > day_start);

        // The daily metrics aren't read with the minute's window.
        let (regular, sink) = (FakeAws { metrics: vec![bucket.clone()], ..Default::default() }, FakeSink::default());
        crawl(&regular, &sink, &[], &job(Some(mid_hour), config()), later, 600_000, &known_namespaces()).await;
        assert!(regular.calls.lock().unwrap().is_empty());
    }
}
