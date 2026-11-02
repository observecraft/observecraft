//! The Azure integration's Azure Monitor crawler, on the cloud crawlers' jobs (`crate::cloud::jobs`).
//!
//! - **Discovery.** Each app registration with metric collection on has a discovery job (scope `""`):
//!   it lists the enabled subscriptions the registration can read, hourly; each becomes a job, so a
//!   large tenant's subscriptions spread over the servers.
//! - **A subscription's run** ([`crawl`]) lists its resources, keeps those the registration collects
//!   (resource provider settings; host, App Service plan and container app filters), reads each resource
//!   type's metric names once, and their average, total and maximum per minute over the window since
//!   the watermark (at most 3 hours, ending 5 minutes ago) from the batch API, 50 resources of one type
//!   and region and 20 metrics per call.
//! - **Publishing.** Datadog names (`azure.vm.percentage_cpu`, `.total`, `.maximum`), tagged with the
//!   subscription, the resource group, the region, the resource's name and its Azure tags; VMs are the
//!   `host`. OTLP JSON to Kafka, the watermark advancing after the acknowledgement.
//! - **Failures** are the registration's issues: it can't sign in (or uses secretless authentication,
//!   which needs a federated identity ObserveCraft doesn't have), a missing role assignment (with the
//!   action Azure names), throttling.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use serde_json::Value;

use super::api::{AzureApi, AzureError, Credentials, Resource};
use super::naming::{self, AGGREGATIONS};
use crate::cloud::jobs::{self, Backoff, Crawler, Issue, JobKey, MetricSink, Outcome};
use crate::cloud::naming::{otlp_json, Point};
use crate::routes::datadog_api::azure_integration as accounts;
use crate::state::AppState;

pub const PROVIDER: &str = "azure";
/// Azure Monitor's publishing delay.
const LAG_MS: i64 = 5 * 60_000;
const MAX_WINDOW_MS: i64 = 3 * 3_600_000;
/// The batch API's limits.
const RESOURCES_PER_CALL: usize = 50;
const METRICS_PER_CALL: usize = 20;
const POINTS_PER_RECORD: usize = 10_000;

/// An Azure error as the app registration's issue.
pub fn issue(e: &AzureError) -> Issue {
    match e {
        AzureError::Auth(m) => Issue { kind: "auth", message: m.clone(), missing_action: None, backoff: Backoff::Interval },
        AzureError::Forbidden { action, message } => Issue { kind: "access_denied", message: message.clone(), missing_action: action.clone(), backoff: Backoff::Interval },
        AzureError::Throttled(m) => Issue { kind: "throttled", message: m.clone(), missing_action: None, backoff: Backoff::Exponential },
        AzureError::Other(m) => Issue { kind: "other", message: m.clone(), missing_action: None, backoff: Backoff::Exponential },
    }
}

/// Whether the app registration collects a resource: its provider's setting and its type's filter.
pub fn selected(config: &Value, r: &Resource) -> bool {
    naming::provider_enabled(config, &r.kind) && naming::filter_key(&r.kind).is_none_or(|key| naming::filters_match(config[key].as_str().unwrap_or_default(), &r.tags))
}

fn split_tag(t: &str) -> (String, String) {
    let (k, v) = t.split_once(':').unwrap_or((t, ""));
    (k.to_string(), v.to_string())
}

/// Reads and publishes one subscription's window.
#[allow(clippy::too_many_arguments)]
pub async fn crawl(api: &dyn AzureApi, sink: &dyn MetricSink, headers: &[(String, String)], org: &str, creds: &Credentials, config: &Value, subscription: &str, collected_until: Option<i64>, now_ms: i64, interval_ms: i64) -> Outcome {
    let end = (now_ms - LAG_MS) / 60_000 * 60_000;
    let start = collected_until.unwrap_or(end - interval_ms).max(end - MAX_WINDOW_MS);
    if end <= start {
        return Outcome::Idle;
    }
    let resources: Vec<Resource> = match api.resources(creds, subscription).await {
        Ok(r) => r.into_iter().filter(|r| selected(config, r)).collect(),
        Err(e) => return Outcome::Failed(issue(&e)),
    };
    let mut groups: BTreeMap<(String, String), Vec<&Resource>> = BTreeMap::new();
    for r in &resources {
        groups.entry((r.kind.clone(), r.location.clone())).or_default().push(r);
    }
    let by_id: HashMap<String, &Resource> = resources.iter().map(|r| (r.id.to_ascii_lowercase(), r)).collect();
    let mut names_by_type: HashMap<String, Vec<String>> = HashMap::new();
    let aggregations: Vec<&str> = AGGREGATIONS.iter().map(|(a, _)| *a).collect();
    let mut points = vec![];
    for ((kind, region), members) in &groups {
        if !names_by_type.contains_key(kind) {
            let names = match api.metric_names(creds, &members[0].id).await {
                Ok(n) => n,
                Err(e) => return Outcome::Failed(issue(&e)),
            };
            names_by_type.insert(kind.clone(), names);
        }
        let names = &names_by_type[kind];
        if names.is_empty() {
            continue;
        }
        for chunk in members.chunks(RESOURCES_PER_CALL) {
            let ids: Vec<String> = chunk.iter().map(|r| r.id.clone()).collect();
            for metric_chunk in names.chunks(METRICS_PER_CALL) {
                let series = match api.metrics(creds, subscription, region, kind, &ids, metric_chunk, &aggregations, start, end).await {
                    Ok(s) => s,
                    Err(e) => return Outcome::Failed(issue(&e)),
                };
                for s in series {
                    let Some(r) = by_id.get(&s.resource_id.to_ascii_lowercase()) else { continue };
                    let suffix = AGGREGATIONS.iter().find(|(a, _)| *a == s.aggregation).map(|(_, x)| *x).unwrap_or_default();
                    let name = format!("{}{suffix}", naming::metric_name(kind, &s.metric));
                    let mut tags: BTreeMap<String, String> =
                        [("subscription_id", subscription), ("resource_group", r.resource_group.as_str()), ("region", r.location.as_str()), ("name", r.name.as_str())].iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
                    tags.extend(r.tags.iter().map(|t| split_tag(t)));
                    let tags: Vec<(String, String)> = tags.into_iter().collect();
                    let host = (naming::filter_key(kind) == Some("host_filters")).then(|| r.name.clone());
                    points.extend(s.points.iter().map(|&(time_ms, value)| Point { name: name.clone(), host: host.clone(), tags: tags.clone(), time_ms, value }));
                }
            }
        }
    }
    for record in points.chunks(POINTS_PER_RECORD) {
        if let Err(e) = sink.publish(org, headers, otlp_json(PROVIDER, record)).await {
            return Outcome::Failed(Issue { kind: "other", message: format!("publishing to Kafka failed: {e}"), missing_action: None, backoff: Backoff::Exponential });
        }
    }
    Outcome::Collected { end_ms: end, metrics: resources.len(), points: points.len(), warning: None }
}

/// The Azure crawler: Azure Monitor through `api`, points to `sink`.
pub struct AzureCrawler {
    pub api: Arc<dyn AzureApi>,
    pub sink: Arc<dyn MetricSink>,
    pub interval_secs: i64,
}

#[async_trait::async_trait]
impl Crawler for AzureCrawler {
    fn provider(&self) -> &'static str {
        PROVIDER
    }

    /// One discovery job per app registration with metric collection on.
    async fn desired(&self, st: &AppState) -> Result<Vec<(String, String, String)>, sqlx::Error> {
        let rows: Vec<(String, String)> = sqlx::query_as(
            "SELECT a.org_id, a.id::text FROM azure_accounts a JOIN orgs o ON o.id = a.org_id AND o.status = 'active' WHERE COALESCE((a.config->>'metrics_enabled')::boolean, true)",
        )
        .fetch_all(&st.postgres)
        .await?;
        Ok(rows.into_iter().map(|(org, id)| (org, id, String::new())).collect())
    }

    async fn run(&self, st: &AppState, job: &JobKey, now_ms: i64) -> Result<Outcome, sqlx::Error> {
        let Some(account) = accounts::get_row(st, &job.org_id, &job.account_id).await? else { return Ok(Outcome::Idle) };
        let Some(client_secret) = accounts::client_secret(st, &job.org_id, &job.account_id).await? else {
            let message = "This app registration has no client secret: ObserveCraft authenticates with a client secret (secretless authentication needs a federated identity it doesn't have).";
            return Ok(Outcome::Failed(Issue { kind: "auth", message: message.into(), missing_action: None, backoff: Backoff::Interval }));
        };
        let creds = Credentials { tenant_id: account.tenant_name.clone(), client_id: account.client_id.clone(), client_secret };
        if job.scope.is_empty() {
            return Ok(match self.api.subscriptions(&creds).await {
                Ok(subs) => Outcome::Discovered(subs),
                Err(e) => Outcome::Failed(issue(&e)),
            });
        }
        let headers = jobs::org_headers(st, &job.org_id).await?;
        Ok(crawl(self.api.as_ref(), self.sink.as_ref(), &headers, &job.org_id, &creds, &account.config, &job.scope, job.collected_until, now_ms, self.interval_secs * 1000).await)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cloud::testing::{published, FakeSink};
    use serde_json::json;
    use std::collections::HashSet;
    use std::sync::Mutex as StdMutex;

    use super::super::api::Series;

    /// A metrics call: type, region, resources, metrics, start, end.
    type Call = (String, String, usize, usize, i64, i64);

    #[derive(Default)]
    struct FakeAzure {
        resources: Vec<Resource>,
        names: HashMap<String, Vec<String>>,
        fail_metrics: Option<AzureError>,
        calls: StdMutex<Vec<Call>>,
    }

    #[async_trait::async_trait]
    impl AzureApi for FakeAzure {
        async fn subscriptions(&self, _c: &Credentials) -> Result<Vec<String>, AzureError> {
            Ok(vec!["sub-a".into()])
        }
        async fn resources(&self, _c: &Credentials, _s: &str) -> Result<Vec<Resource>, AzureError> {
            Ok(self.resources.clone())
        }
        async fn metric_names(&self, _c: &Credentials, id: &str) -> Result<Vec<String>, AzureError> {
            let kind = self.resources.iter().find(|r| r.id == id).map(|r| r.kind.clone()).unwrap_or_default();
            Ok(self.names.get(&kind).cloned().unwrap_or_default())
        }
        async fn metrics(&self, _c: &Credentials, _s: &str, region: &str, kind: &str, ids: &[String], metrics: &[String], aggregations: &[&str], start: i64, end: i64) -> Result<Vec<Series>, AzureError> {
            if let Some(e) = &self.fail_metrics {
                return Err(e.clone());
            }
            self.calls.lock().unwrap().push((kind.into(), region.into(), ids.len(), metrics.len(), start, end));
            Ok(ids.iter().flat_map(|id| metrics.iter().flat_map(move |m| aggregations.iter().map(move |a| Series { resource_id: id.to_uppercase(), metric: m.clone(), aggregation: a.to_string(), points: vec![(start, 1.0)] }))).collect())
        }
    }

    fn resource(name: &str, kind: &str, location: &str, tags: &[&str]) -> Resource {
        Resource {
            id: format!("/subscriptions/sub-a/resourceGroups/storefront/providers/{kind}/{name}"),
            name: name.into(),
            kind: kind.into(),
            location: location.into(),
            resource_group: "storefront".into(),
            tags: tags.iter().map(|t| t.to_string()).collect(),
        }
    }

    const VM: &str = "Microsoft.Compute/virtualMachines";
    const SITE: &str = "Microsoft.Web/sites";
    const NOW: i64 = 1_700_000_000_000;

    fn creds() -> Credentials {
        Credentials { tenant_id: "t".into(), client_id: "c".into(), client_secret: "s".into() }
    }

    #[tokio::test]
    async fn a_subscription_run_publishes_the_selected_resources_metrics() {
        let azure = FakeAzure {
            resources: vec![
                resource("web-1", VM, "westeurope", &["env:prod"]),
                resource("web-2", VM, "westeurope", &["env:staging"]),
                resource("api", SITE, "northeurope", &[]),
                resource("assets", "Microsoft.Storage/storageAccounts", "westeurope", &[]),
            ],
            names: HashMap::from([(VM.to_string(), vec!["Percentage CPU".to_string()]), (SITE.to_string(), vec!["Http5xx".to_string()])]),
            ..Default::default()
        };
        let sink = FakeSink::default();
        let config = json!({ "host_filters": "env:prod", "metrics_enabled_default": true, "resource_provider_configs": [{ "namespace": "Microsoft.Storage", "metrics_enabled": false }] });
        let outcome = crawl(&azure, &sink, &[], "acme", &creds(), &config, "sub-a", None, NOW, 600_000).await;
        let end = (NOW - LAG_MS) / 60_000 * 60_000;
        assert_eq!(outcome, Outcome::Collected { end_ms: end, metrics: 2, points: 6, warning: None });
        // One batch per type and region, over one interval.
        let calls = azure.calls.lock().unwrap().clone();
        assert_eq!(calls.iter().map(|c| (c.0.as_str(), c.1.as_str(), c.2)).collect::<Vec<_>>(), [(VM, "westeurope", 1), (SITE, "northeurope", 1)]);
        assert!(calls.iter().all(|c| c.4 == end - 600_000 && c.5 == end));
        let points = published(&sink);
        let names: HashSet<&str> = points.iter().map(|p| p.0.as_str()).collect();
        assert_eq!(names, HashSet::from(["azure.vm.percentage_cpu", "azure.vm.percentage_cpu.total", "azure.vm.percentage_cpu.maximum", "azure.app_services.http5xx", "azure.app_services.http5xx.total", "azure.app_services.http5xx.maximum"]));
        let vm = points.iter().find(|p| p.0 == "azure.vm.percentage_cpu").unwrap();
        assert_eq!(vm.1.as_deref(), Some("web-1"));
        let tags: HashMap<_, _> = vm.2.iter().cloned().collect();
        assert_eq!((tags["subscription_id"].as_str(), tags["resource_group"].as_str(), tags["region"].as_str(), tags["env"].as_str()), ("sub-a", "storefront", "westeurope", "prod"));
        let site = points.iter().find(|p| p.0 == "azure.app_services.http5xx").unwrap();
        assert_eq!(site.1, None);
    }

    #[tokio::test]
    async fn large_groups_are_split_by_the_batch_limits_and_failures_publish_nothing() {
        let resources: Vec<Resource> = (0..120).map(|i| resource(&format!("vm-{i}"), VM, "westeurope", &[])).collect();
        let names: Vec<String> = (0..25).map(|i| format!("Metric {i}")).collect();
        let azure = FakeAzure { resources: resources.clone(), names: HashMap::from([(VM.to_string(), names)]), ..Default::default() };
        let sink = FakeSink::default();
        crawl(&azure, &sink, &[], "acme", &creds(), &json!({}), "sub-a", None, NOW, 600_000).await;
        let calls = azure.calls.lock().unwrap().clone();
        assert_eq!(calls.len(), 3 * 2);
        assert!(calls.iter().all(|c| c.2 <= 50 && c.3 <= 20));

        let denied = AzureError::Forbidden { action: Some("Microsoft.Insights/metrics/read".into()), message: "m".into() };
        let azure = FakeAzure { resources, names: HashMap::from([(VM.to_string(), vec!["Percentage CPU".to_string()])]), fail_metrics: Some(denied.clone()), ..Default::default() };
        let sink = FakeSink::default();
        assert_eq!(crawl(&azure, &sink, &[], "acme", &creds(), &json!({}), "sub-a", None, NOW, 600_000).await, Outcome::Failed(issue(&denied)));
        assert!(sink.records.lock().unwrap().is_empty());
        assert_eq!(issue(&denied).missing_action.as_deref(), Some("Microsoft.Insights/metrics/read"));
    }

    #[tokio::test]
    async fn discovery_lists_the_subscriptions_and_a_registration_without_a_secret_is_an_issue() {
        let Some((_, states)) = crate::test_support::replicas(1).await else { return };
        let st = &states[0];
        let org = crate::test_support::new_org(st, "azure-crawl").await;
        let body = |v: Value| v.as_object().cloned().unwrap();
        let with_secret = accounts::upsert_row(st, &org, &body(json!({ "tenant_name": "t", "client_id": "c", "client_secret": "s" }))).await.unwrap();
        let secretless = accounts::upsert_row(st, &org, &body(json!({ "tenant_name": "t", "client_id": "c2", "client_secret": "SECRETLESS_AUTH", "secretless_auth_enabled": true }))).await.unwrap();
        let crawler = AzureCrawler { api: Arc::new(FakeAzure::default()), sink: Arc::new(FakeSink::default()), interval_secs: 600 };
        let desired: Vec<_> = crawler.desired(st).await.unwrap().into_iter().filter(|d| d.0 == org).collect();
        assert_eq!(desired.len(), 2);
        let key = |id: &str| JobKey { provider: PROVIDER.into(), org_id: org.clone(), account_id: id.into(), scope: "".into(), collected_until: None };
        assert_eq!(crawler.run(st, &key(&with_secret.id), NOW).await.unwrap(), Outcome::Discovered(vec!["sub-a".into()]));
        assert!(matches!(crawler.run(st, &key(&secretless.id), NOW).await.unwrap(), Outcome::Failed(Issue { kind: "auth", .. })));
    }
}
