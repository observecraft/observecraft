//! The GCP integration's Cloud Monitoring crawler, on the cloud crawlers' jobs (`crate::cloud::jobs`).
//!
//! - **Discovery.** Each service account has a discovery job (scope `""`): this install impersonates it
//!   and lists the active projects it can see, hourly; each becomes a job, so an organization's projects
//!   spread over the servers.
//! - **A project's run** ([`crawl`]) lists its metric descriptors, keeps those of our catalog's services
//!   that aren't turned off (`metric_namespace_configs`) and can be aligned, and reads each one's time
//!   series per minute over the window since the watermark (at most 3 hours, ending 5 minutes ago),
//!   several metric types at a time; series whose monitored resource the filters exclude are dropped.
//! - **Publishing.** Datadog names (`gcp.gce.instance.cpu.utilization`), tagged with the project, the
//!   resource's and the metric's labels and the account's tags; instances are the `host`. OTLP JSON to
//!   Kafka, the watermark advancing after the acknowledgement.
//! - **Failures** are the account's issues: it can't be impersonated (Service Account Token Creator), a
//!   missing permission (named), the Cloud Monitoring API disabled in a project (retried daily), throttling.

use std::collections::BTreeMap;
use std::sync::Arc;

use futures::stream::{self, StreamExt};
use serde_json::Value;

use super::api::{aligner, Descriptor, GcpApi, GcpError};
use super::naming;
use crate::cloud::jobs::{self, Backoff, Crawler, Issue, JobKey, MetricSink, Outcome};
use crate::cloud::naming::{otlp_json, Point};
use crate::routes::datadog_api::gcp_integration as accounts;
use crate::state::AppState;

pub const PROVIDER: &str = "gcp";
const LAG_MS: i64 = 5 * 60_000;
const MAX_WINDOW_MS: i64 = 3 * 3_600_000;
/// Metric types read at a time within a project.
const PARALLEL: usize = 8;
const POINTS_PER_RECORD: usize = 10_000;

/// A Google error as the service account's issue.
pub fn issue(e: &GcpError) -> Issue {
    match e {
        GcpError::Auth(m) => Issue { kind: "auth", message: m.clone(), missing_action: None, backoff: Backoff::Interval },
        GcpError::Forbidden { permission, message } => Issue { kind: "access_denied", message: message.clone(), missing_action: permission.clone(), backoff: Backoff::Interval },
        GcpError::Disabled(m) => Issue { kind: "disabled", message: m.clone(), missing_action: None, backoff: Backoff::Day },
        GcpError::Throttled(m) => Issue { kind: "throttled", message: m.clone(), missing_action: None, backoff: Backoff::Exponential },
        GcpError::Other(m) => Issue { kind: "other", message: m.clone(), missing_action: None, backoff: Backoff::Exponential },
    }
}

fn split_tag(t: &str) -> (String, String) {
    let (k, v) = t.split_once(':').unwrap_or((t, ""));
    (k.to_string(), v.to_string())
}

/// Reads and publishes one project's window.
#[allow(clippy::too_many_arguments)]
pub async fn crawl(api: &dyn GcpApi, sink: &dyn MetricSink, headers: &[(String, String)], org: &str, service_account: &str, config: &Value, project: &str, collected_until: Option<i64>, now_ms: i64, interval_ms: i64) -> Outcome {
    let end = (now_ms - LAG_MS) / 60_000 * 60_000;
    let start = collected_until.unwrap_or(end - interval_ms).max(end - MAX_WINDOW_MS);
    if end <= start {
        return Outcome::Idle;
    }
    let descriptors: Vec<Descriptor> = match api.descriptors(service_account, project).await {
        Ok(d) => d.into_iter().filter(|d| naming::known(&d.metric_type) && naming::namespace_enabled(config, &d.metric_type) && aligner(d).is_some()).collect(),
        Err(e) => return Outcome::Failed(issue(&e)),
    };
    let account_tags: Vec<(String, String)> = config["account_tags"].as_array().map(|a| a.iter().filter_map(Value::as_str).map(split_tag).collect()).unwrap_or_default();
    // The calls are built first: a stream combinator's closure over borrowed descriptors isn't Send-general.
    let calls: Vec<_> = descriptors.iter().map(|d| async move { (d.metric_type.clone(), api.time_series(service_account, project, d, start, end).await) }).collect();
    let results: Vec<(String, Result<Vec<super::api::Series>, GcpError>)> = stream::iter(calls).buffer_unordered(PARALLEL).collect().await;
    let mut points = vec![];
    let mut metrics = 0;
    for (metric_type, result) in results {
        let series = match result {
            Ok(s) => s,
            Err(e) => return Outcome::Failed(issue(&e)),
        };
        let name = naming::metric_name(&metric_type);
        for s in series {
            let mut tags: BTreeMap<String, String> = account_tags.iter().cloned().collect();
            tags.insert("project_id".into(), project.into());
            tags.extend(s.resource_labels.iter().cloned());
            tags.extend(s.metric_labels.iter().cloned());
            let flat: Vec<String> = tags.iter().map(|(k, v)| format!("{k}:{v}")).collect();
            if !naming::resource_selected(config, &s.resource_type, &flat) {
                continue;
            }
            metrics += 1;
            let host = (s.resource_type == "gce_instance").then(|| tags.get("instance_id").cloned()).flatten();
            let tags: Vec<(String, String)> = tags.into_iter().collect();
            points.extend(s.points.iter().map(|&(time_ms, value)| Point { name: name.clone(), host: host.clone(), tags: tags.clone(), time_ms, value }));
        }
    }
    for record in points.chunks(POINTS_PER_RECORD) {
        if let Err(e) = sink.publish(org, headers, otlp_json(PROVIDER, record)).await {
            return Outcome::Failed(Issue { kind: "other", message: format!("publishing to Kafka failed: {e}"), missing_action: None, backoff: Backoff::Exponential });
        }
    }
    Outcome::Collected { end_ms: end, metrics, points: points.len(), warning: None }
}

/// The GCP crawler: Cloud Monitoring through `api`, points to `sink`.
pub struct GcpCrawler {
    pub api: Arc<dyn GcpApi>,
    pub sink: Arc<dyn MetricSink>,
    pub interval_secs: i64,
}

#[async_trait::async_trait]
impl Crawler for GcpCrawler {
    fn provider(&self) -> &'static str {
        PROVIDER
    }

    /// One discovery job per service account.
    async fn desired(&self, st: &AppState) -> Result<Vec<(String, String, String)>, sqlx::Error> {
        let rows: Vec<(String, String)> = sqlx::query_as("SELECT a.org_id, a.id::text FROM gcp_accounts a JOIN orgs o ON o.id = a.org_id AND o.status = 'active'").fetch_all(&st.postgres).await?;
        Ok(rows.into_iter().map(|(org, id)| (org, id, String::new())).collect())
    }

    async fn run(&self, st: &AppState, job: &JobKey, now_ms: i64) -> Result<Outcome, sqlx::Error> {
        let Some(account) = accounts::get_row(st, &job.org_id, &job.account_id).await? else { return Ok(Outcome::Idle) };
        if job.scope.is_empty() {
            return Ok(match self.api.projects(&account.client_email).await {
                Ok(projects) => Outcome::Discovered(projects),
                Err(e) => Outcome::Failed(issue(&e)),
            });
        }
        let headers = jobs::org_headers(st, &job.org_id).await?;
        Ok(crawl(self.api.as_ref(), self.sink.as_ref(), &headers, &job.org_id, &account.client_email, &account.config, &job.scope, job.collected_until, now_ms, self.interval_secs * 1000).await)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cloud::testing::{published, FakeSink};
    use serde_json::json;
    use std::collections::{HashMap, HashSet};

    use super::super::api::Series;

    #[derive(Default)]
    struct FakeGcp {
        descriptors: Vec<Descriptor>,
        series: HashMap<String, Vec<Series>>,
        fail: Option<GcpError>,
    }

    #[async_trait::async_trait]
    impl GcpApi for FakeGcp {
        async fn projects(&self, _sa: &str) -> Result<Vec<String>, GcpError> {
            Ok(vec!["example-storefront".into()])
        }
        async fn descriptors(&self, _sa: &str, _p: &str) -> Result<Vec<Descriptor>, GcpError> {
            Ok(self.descriptors.clone())
        }
        async fn time_series(&self, _sa: &str, _p: &str, d: &Descriptor, start: i64, _end: i64) -> Result<Vec<Series>, GcpError> {
            if let Some(e) = &self.fail {
                return Err(e.clone());
            }
            Ok(self.series.get(&d.metric_type).cloned().unwrap_or_default().into_iter().map(|mut s| {
                s.points = vec![(start, 0.5)];
                s
            }).collect())
        }
    }

    fn d(t: &str, kind: &str, value: &str) -> Descriptor {
        Descriptor { metric_type: t.into(), kind: kind.into(), value_type: value.into() }
    }

    fn instance(id: &str, env: &str) -> Series {
        Series {
            resource_type: "gce_instance".into(),
            resource_labels: vec![("instance_id".into(), id.into()), ("zone".into(), "europe-west1-b".into())],
            metric_labels: vec![("env".into(), env.into())],
            points: vec![],
        }
    }

    const CPU: &str = "compute.googleapis.com/instance/cpu/utilization";
    const SQL: &str = "cloudsql.googleapis.com/database/cpu/utilization";
    const NOW: i64 = 1_700_000_000_000;

    #[tokio::test]
    async fn a_project_run_publishes_the_selected_series_named_and_tagged_as_datadog_does() {
        let gcp = FakeGcp {
            descriptors: vec![d(CPU, "GAUGE", "DOUBLE"), d(SQL, "GAUGE", "DOUBLE"), d("custom.googleapis.com/checkout/orders", "GAUGE", "DOUBLE"), d("compute.googleapis.com/instance/uptime_total", "GAUGE", "STRING")],
            series: HashMap::from([(CPU.to_string(), vec![instance("111", "prod"), instance("222", "staging")]), (SQL.to_string(), vec![Series { resource_type: "cloudsql_database".into(), resource_labels: vec![("database_id".into(), "p:orders".into())], metric_labels: vec![], points: vec![] }])]),
            ..Default::default()
        };
        let sink = FakeSink::default();
        let config = json!({ "monitored_resource_configs": [{ "type": "gce_instance", "filters": ["env:prod"] }], "metric_namespace_configs": [], "account_tags": ["team:storefront"] });
        let outcome = crawl(&gcp, &sink, &[], "acme", "sa@example-storefront.iam.gserviceaccount.com", &config, "example-storefront", None, NOW, 600_000).await;
        assert_eq!(outcome, Outcome::Collected { end_ms: (NOW - LAG_MS) / 60_000 * 60_000, metrics: 2, points: 2, warning: None });
        let points = published(&sink);
        let names: HashSet<&str> = points.iter().map(|p| p.0.as_str()).collect();
        assert_eq!(names, HashSet::from(["gcp.gce.instance.cpu.utilization", "gcp.cloudsql.database.cpu.utilization"]));
        let cpu = points.iter().find(|p| p.0 == "gcp.gce.instance.cpu.utilization").unwrap();
        assert_eq!(cpu.1.as_deref(), Some("111"));
        let tags: HashMap<_, _> = cpu.2.iter().cloned().collect();
        assert_eq!((tags["project_id"].as_str(), tags["zone"].as_str(), tags["team"].as_str()), ("example-storefront", "europe-west1-b", "storefront"));

        // A disabled namespace is skipped.
        let sink = FakeSink::default();
        let off = json!({ "metric_namespace_configs": [{ "id": "cloudsql", "disabled": true }] });
        crawl(&gcp, &sink, &[], "acme", "sa", &off, "example-storefront", None, NOW, 600_000).await;
        assert!(published(&sink).iter().all(|p| !p.0.starts_with("gcp.cloudsql.")));
    }

    #[tokio::test]
    async fn failures_publish_nothing_and_become_issues() {
        let denied = GcpError::Forbidden { permission: Some("monitoring.timeSeries.list".into()), message: "denied".into() };
        let gcp = FakeGcp { descriptors: vec![d(CPU, "GAUGE", "DOUBLE")], fail: Some(denied.clone()), ..Default::default() };
        let sink = FakeSink::default();
        assert_eq!(crawl(&gcp, &sink, &[], "acme", "sa", &json!({}), "example-storefront", None, NOW, 600_000).await, Outcome::Failed(issue(&denied)));
        assert!(sink.records.lock().unwrap().is_empty());
        assert_eq!(issue(&denied).missing_action.as_deref(), Some("monitoring.timeSeries.list"));
        assert_eq!(issue(&GcpError::Disabled("x".into())).backoff, Backoff::Day);
    }

    #[tokio::test]
    async fn discovery_lists_the_projects_of_each_service_account() {
        let Some((_, states)) = crate::test_support::replicas(1).await else { return };
        let st = &states[0];
        let org = crate::test_support::new_org(st, "gcp-crawl").await;
        let created = accounts::create_row(st, &org, json!({ "client_email": "sa@example-storefront.iam.gserviceaccount.com" }).as_object().unwrap()).await.unwrap();
        let crawler = GcpCrawler { api: Arc::new(FakeGcp::default()), sink: Arc::new(FakeSink::default()), interval_secs: 600 };
        assert!(crawler.desired(st).await.unwrap().iter().any(|d| d.0 == org && d.1 == created.id && d.2.is_empty()));
        let key = JobKey { provider: PROVIDER.into(), org_id: org.clone(), account_id: created.id.clone(), scope: "".into(), collected_until: None };
        assert_eq!(crawler.run(st, &key, NOW).await.unwrap(), Outcome::Discovered(vec!["example-storefront".into()]));
    }
}
