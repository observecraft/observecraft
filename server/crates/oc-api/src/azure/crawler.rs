//! The Azure integration's Azure Monitor crawler, on the cloud crawlers' jobs (`crate::cloud::jobs`).
//!
//! - **Discovery.** Each app registration with metric collection on has a discovery job (scope `""`):
//!   it lists the enabled subscriptions the registration can read, hourly; each becomes a job, so a
//!   large tenant's subscriptions spread over the servers.
//! - **A subscription's run** ([`crawl`]) lists its resources, keeps those the registration collects
//!   (resource provider settings; host, App Service plan and container app filters), reads each resource
//!   type's metric definitions once, and their values over the window since the watermark (at most 3 hours,
//!   ending 5 minutes ago): 50 resources of one type and region and 20 metrics per call, from the regional
//!   batch API, or Resource Manager's per-resource API for global resources (DNS zones, Front Door, …).
//!   Storage accounts' blob, file, queue and table services, which the list leaves out, are read from their
//!   accounts' ids (`…/blobServices/default`); function apps (sites of kind `functionapp`) apart from web apps.
//! - **Time grains.** Each metric is read at its finest grain. One-minute metrics follow the watermark;
//!   coarser ones (storage capacity: hourly) are re-read over their last intervals (at least an hour, 3 of
//!   them, at most 3 days) when one completed or once an hour, and each series' latest published point is
//!   kept (Postgres `azure_metric_state.marks`), so a point is published once.
//! - **Publishing.** Datadog's names: the bare name carries the metric's primary aggregation (counts summed,
//!   gauges averaged), `.max` its maximum; the most used dimensions are split (App Service instances, Storage
//!   API names and response types, Redis shards, Event Hubs entities, …, `naming::splits`) and tagged in
//!   lowercase (`apiname`). Tagged with the subscription, the resource group, the region, the resource's
//!   name and its Azure tags; VMs are the `host`. `azure.<type>.count` counts every selected resource each
//!   minute, with its Resource Health `status` (Running, Degraded, Unavailable, Unknown; `query_failed`
//!   when Resource Health can't be read). OTLP JSON to Kafka, the watermark advancing after the
//!   acknowledgement.
//! - **Refusals.** A type or batch Azure Monitor refuses (HTTP 400: no platform metrics) is skipped and kept
//!   as the run's warning; the rest is still published.
//! - **Failed reads** (throttling, server errors) don't drop the run: what was read is published, the failed
//!   one-minute reads are kept (`azure_metric_state.pending`, a day at most) and retried on the next runs,
//!   and the run's warning says so. A run whose every read failed is the registration's issue (backoff).
//! - **Failures** are the registration's issues: it can't sign in (or uses secretless authentication,
//!   which needs a federated identity ObserveCraft doesn't have), a missing role assignment (with the
//!   action Azure names).

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::api::{AzureApi, AzureError, Credentials, MetricDef, MetricsQuery, Resource, Series};
use super::naming;
use crate::cloud::jobs::{self, Backoff, Crawler, Issue, JobKey, MetricSink, Outcome};
use crate::cloud::naming::{otlp_json, snake, Point};
use crate::routes::datadog_api::azure_integration as accounts;
use crate::state::AppState;

pub const PROVIDER: &str = "azure";
/// Azure Monitor's publishing delay.
const LAG_MS: i64 = 5 * 60_000;
const MAX_WINDOW_MS: i64 = 3 * 3_600_000;
const MINUTE_MS: i64 = 60_000;
const HOUR_MS: i64 = 3_600_000;
const DAY_MS: i64 = 86_400_000;
/// The batch API's limits.
const RESOURCES_PER_CALL: usize = 50;
const METRICS_PER_CALL: usize = 20;
const POINTS_PER_RECORD: usize = 10_000;
/// Failed reads kept for the next runs: at most this many, for at most a day.
const MAX_PENDING: usize = 200;
const PENDING_MAX_AGE_MS: i64 = DAY_MS;
/// Marks older than the longest re-read window (3 days) and a day are dropped.
const MARKS_KEEP_MS: i64 = 4 * DAY_MS;

/// An Azure error as the app registration's issue.
pub fn issue(e: &AzureError) -> Issue {
    match e {
        AzureError::Auth(m) => Issue { kind: "auth", message: m.clone(), missing_action: None, backoff: Backoff::Interval },
        AzureError::Forbidden { action, message } => Issue { kind: "access_denied", message: message.clone(), missing_action: action.clone(), backoff: Backoff::Interval },
        AzureError::Throttled(m) => Issue { kind: "throttled", message: m.clone(), missing_action: None, backoff: Backoff::Exponential },
        AzureError::Rejected(m) | AzureError::Other(m) => Issue { kind: "other", message: m.clone(), missing_action: None, backoff: Backoff::Exponential },
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

/// A one-minute read that failed, retried on the next runs.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Pending {
    /// The resources' [`naming::named_type`].
    pub kind: String,
    pub region: String,
    pub resource_ids: Vec<String>,
    pub metrics: Vec<String>,
    pub dimensions: Vec<String>,
    pub start_ms: i64,
    pub end_ms: i64,
}

/// A subscription job's state beside its watermark (Postgres `azure_metric_state`).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct State {
    /// The latest published point of each coarse-grained series ([`mark_key`] → epoch ms).
    #[serde(default)]
    pub marks: HashMap<String, i64>,
    #[serde(default)]
    pub pending: Vec<Pending>,
}

/// A run's outcome and, when it collected, the state to keep.
#[derive(Debug, PartialEq)]
pub struct Crawl {
    pub outcome: Outcome,
    pub state: Option<State>,
}

impl From<Outcome> for Crawl {
    fn from(outcome: Outcome) -> Self {
        Crawl { outcome, state: None }
    }
}

/// A series' key in [`State::marks`]: resource, metric and split dimensions' values.
pub fn mark_key(s: &Series) -> String {
    let dims: Vec<String> = s.dimensions.iter().map(|(k, v)| format!("{}={v}", k.to_ascii_lowercase())).collect();
    format!("{}|{}|{}", s.resource_id.to_ascii_lowercase(), s.metric, dims.join(","))
}

/// How far back a coarse grain is re-read: 3 of its intervals, at least an hour, at most 3 days.
fn lookback_ms(grain_ms: i64) -> i64 {
    (3 * grain_ms).clamp(HOUR_MS, 3 * DAY_MS)
}

/// Metrics read together: one grain, the same split dimensions.
type ReadKey = (i64, Vec<String>);

fn reads<'a>(kind: &str, defs: &'a [MetricDef]) -> BTreeMap<ReadKey, Vec<&'a MetricDef>> {
    let mut out: BTreeMap<ReadKey, Vec<&MetricDef>> = BTreeMap::new();
    // `azure.<type>.count` is the resource count.
    for d in defs.iter().filter(|d| snake(&d.name) != "count") {
        out.entry((d.grain_ms, naming::splits(kind, &d.dimensions))).or_default().push(d);
    }
    out
}

/// The aggregations a read asks for: every one its metrics are published with.
fn asked(defs: &[&MetricDef]) -> Vec<&'static str> {
    let mut out: Vec<&'static str> = vec![];
    for d in defs {
        for (a, _) in naming::aggregations(&d.primary, &d.aggregations) {
            let a: &'static str = match a.as_str() {
                "total" => "total",
                "maximum" => "maximum",
                "minimum" => "minimum",
                "count" => "count",
                _ => "average",
            };
            if !out.contains(&a) {
                out.push(a);
            }
        }
    }
    out
}

/// A resource's tags: subscription, resource group, region, name and its Azure tags.
fn resource_tags(subscription: &str, r: &Resource) -> BTreeMap<String, String> {
    let mut tags: BTreeMap<String, String> =
        [("subscription_id", subscription), ("resource_group", r.resource_group.as_str()), ("region", r.location.as_str()), ("name", r.name.as_str())].iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
    tags.extend(r.tags.iter().map(|t| split_tag(t)));
    tags
}

fn host(r: &Resource) -> Option<String> {
    (naming::filter_key(&r.kind) == Some("host_filters")).then(|| r.name.clone())
}

/// A read's series as points, named and tagged; with `marks`, only points after each series' mark (the marks advance).
fn to_points(kind: &str, subscription: &str, defs: &[&MetricDef], series: Vec<Series>, by_id: &HashMap<String, &Resource>, marks: Option<&mut HashMap<String, i64>>, out: &mut Vec<Point>) {
    let mut newest: HashMap<String, i64> = HashMap::new();
    for s in series {
        let Some(r) = by_id.get(&s.resource_id.to_ascii_lowercase()) else { continue };
        let Some(def) = defs.iter().find(|d| d.name.eq_ignore_ascii_case(&s.metric)) else { continue };
        let Some((_, suffix)) = naming::aggregations(&def.primary, &def.aggregations).into_iter().find(|(a, _)| a.eq_ignore_ascii_case(&s.aggregation)) else { continue };
        let name = format!("{}{suffix}", naming::metric_name(kind, &def.name));
        let mut tags = resource_tags(subscription, r);
        for (k, v) in &s.dimensions {
            tags.entry(naming::dimension_tag(k)).or_insert_with(|| v.clone());
        }
        let tags: Vec<(String, String)> = tags.into_iter().collect();
        let after = match marks.as_deref() {
            Some(m) => {
                let key = mark_key(&s);
                let after = m.get(&key).copied().unwrap_or(i64::MIN);
                if let Some(latest) = s.points.iter().map(|p| p.0).filter(|t| *t > after).max() {
                    let e = newest.entry(key).or_insert(latest);
                    *e = (*e).max(latest);
                }
                after
            }
            None => i64::MIN,
        };
        out.extend(s.points.iter().filter(|p| p.0 > after).map(|&(time_ms, value)| Point { name: name.clone(), host: host(r), tags: tags.clone(), time_ms, value }));
    }
    if let Some(m) = marks {
        m.extend(newest);
    }
}

/// A storage account's blob, file, queue and table services (`…/blobServices/default`), which the resources list
/// leaves out: their metrics are their own types' (`azure.storage_storageaccounts_blobservices.*`), read from the
/// regional batch API like the account's, and tagged as the account (its name, group, region and tags).
fn storage_services(r: &Resource) -> Vec<Resource> {
    if !r.kind.eq_ignore_ascii_case("microsoft.storage/storageaccounts") {
        return vec![];
    }
    // Premium block blob and file share accounts have only their own service; general-purpose ones have the four.
    let services: &[&str] = match r.azure_kind.to_ascii_lowercase().as_str() {
        "blobstorage" | "blockblobstorage" => &["blobServices"],
        "filestorage" => &["fileServices"],
        _ => &["blobServices", "fileServices", "queueServices", "tableServices"],
    };
    services.iter().map(|s| Resource { id: format!("{}/{s}/default", r.id), kind: format!("Microsoft.Storage/storageAccounts/{s}"), azure_kind: String::new(), ..r.clone() }).collect()
}

/// The id Resource Health reports a resource under (lowercase): a storage service's account.
fn health_id(r: &Resource) -> String {
    let id = r.id.to_ascii_lowercase();
    if r.kind.to_ascii_lowercase().starts_with("microsoft.storage/storageaccounts/") {
        return id.rsplitn(3, '/').nth(2).unwrap_or(&id).to_string();
    }
    id
}

/// Resource Health's state as Datadog's `status` tag.
fn status(state: Option<&String>) -> String {
    match state.map(String::as_str) {
        Some("Available") => "Running".into(),
        Some(s) => s.to_string(),
        None => "Unknown".into(),
    }
}

/// What a run's reads did.
#[derive(Default)]
struct Reads {
    points: Vec<Point>,
    ok: usize,
    failed: usize,
    failure: Option<Issue>,
    refusal: Option<Issue>,
    pending: Vec<Pending>,
}

/// Reads and publishes one subscription's window.
#[allow(clippy::too_many_arguments)]
pub async fn crawl(api: &dyn AzureApi, sink: &dyn MetricSink, headers: &[(String, String)], org: &str, creds: &Credentials, config: &Value, subscription: &str, collected_until: Option<i64>, state: &State, now_ms: i64, interval_ms: i64) -> Crawl {
    let end = (now_ms - LAG_MS) / MINUTE_MS * MINUTE_MS;
    let start = collected_until.unwrap_or(end - interval_ms).max(end - MAX_WINDOW_MS);
    if end <= start {
        return Outcome::Idle.into();
    }
    let resources: Vec<Resource> = match api.resources(creds, subscription).await {
        Ok(r) => r.into_iter().filter(|r| selected(config, r)).flat_map(|r| { let services = storage_services(&r); std::iter::once(r).chain(services) }).collect(),
        Err(e) => return Outcome::Failed(issue(&e)).into(),
    };
    // By the type their metrics are named by (function apps apart from web apps) and region.
    let mut groups: BTreeMap<(String, String), Vec<&Resource>> = BTreeMap::new();
    for r in &resources {
        groups.entry((naming::named_type(&r.kind, &r.azure_kind), r.location.to_ascii_lowercase())).or_default().push(r);
    }
    let by_id: HashMap<String, &Resource> = resources.iter().map(|r| (r.id.to_ascii_lowercase(), r)).collect();
    let mut defs_by_type: HashMap<String, Vec<MetricDef>> = HashMap::new();
    let mut marks: HashMap<String, i64> = state.marks.iter().filter(|(_, &at)| at >= end - MARKS_KEEP_MS).map(|(k, v)| (k.clone(), *v)).collect();
    // Coarse grains: on the first run, once an hour, and when one of their intervals completed.
    let hourly = collected_until.is_none() || start / HOUR_MS != end / HOUR_MS;
    let mut run = Reads::default();

    // The failed reads of earlier runs first (still within a day).
    for p in state.pending.iter().filter(|p| p.end_ms > end - PENDING_MAX_AGE_MS) {
        let Some(first) = p.resource_ids.first() else { continue };
        if !defs_by_type.contains_key(&p.kind) {
            match api.metric_definitions(creds, first).await {
                Ok(d) => {
                    defs_by_type.insert(p.kind.clone(), d);
                }
                Err(e @ (AzureError::Auth(_) | AzureError::Forbidden { .. })) => return Outcome::Failed(issue(&e)).into(),
                Err(_) => {
                    run.pending.push(p.clone());
                    continue;
                }
            }
        }
        let defs: Vec<&MetricDef> = defs_by_type[&p.kind].iter().filter(|d| p.metrics.contains(&d.name)).collect();
        let aggregations = asked(&defs);
        let q = MetricsQuery { subscription, region: &p.region, resource_type: naming::namespace(&p.kind), resource_ids: &p.resource_ids, metrics: &p.metrics, aggregations: &aggregations, grain_ms: MINUTE_MS, dimensions: &p.dimensions, start_ms: p.start_ms, end_ms: p.end_ms };
        match api.metrics(creds, &q).await {
            Ok(series) => {
                run.ok += 1;
                to_points(&p.kind, subscription, &defs, series, &by_id, None, &mut run.points);
            }
            Err(e @ (AzureError::Auth(_) | AzureError::Forbidden { .. })) => return Outcome::Failed(issue(&e)).into(),
            Err(AzureError::Rejected(_)) => {}
            Err(e) => {
                run.failed += 1;
                run.failure.get_or_insert_with(|| issue(&e));
                run.pending.push(p.clone());
            }
        }
    }

    for ((kind, region), members) in &groups {
        if !defs_by_type.contains_key(kind) {
            let defs = match api.metric_definitions(creds, &members[0].id).await {
                Ok(d) => d,
                Err(e @ AzureError::Rejected(_)) => {
                    run.refusal.get_or_insert_with(|| issue(&e));
                    vec![]
                }
                Err(e @ (AzureError::Auth(_) | AzureError::Forbidden { .. })) => return Outcome::Failed(issue(&e)).into(),
                Err(e) => {
                    // Read again on the next run.
                    run.failed += 1;
                    run.failure.get_or_insert_with(|| issue(&e));
                    continue;
                }
            };
            defs_by_type.insert(kind.clone(), defs);
        }
        for ((grain, dimensions), defs) in reads(kind, &defs_by_type[kind]) {
            let (from, to) = if grain <= MINUTE_MS {
                (start, end)
            } else if hourly || start / grain != end / grain {
                let to = end / grain * grain;
                (to - lookback_ms(grain), to)
            } else {
                continue;
            };
            let aggregations = asked(&defs);
            for chunk in members.chunks(RESOURCES_PER_CALL) {
                let ids: Vec<String> = chunk.iter().map(|r| r.id.clone()).collect();
                for metric_chunk in defs.chunks(METRICS_PER_CALL) {
                    let names: Vec<String> = metric_chunk.iter().map(|d| d.name.clone()).collect();
                    let q = MetricsQuery { subscription, region, resource_type: naming::namespace(kind), resource_ids: &ids, metrics: &names, aggregations: &aggregations, grain_ms: grain.max(MINUTE_MS), dimensions: &dimensions, start_ms: from, end_ms: to };
                    match api.metrics(creds, &q).await {
                        Ok(series) => {
                            run.ok += 1;
                            to_points(kind, subscription, metric_chunk, series, &by_id, (grain > MINUTE_MS).then_some(&mut marks), &mut run.points);
                        }
                        Err(e @ AzureError::Rejected(_)) => {
                            run.refusal.get_or_insert_with(|| issue(&e));
                        }
                        Err(e @ (AzureError::Auth(_) | AzureError::Forbidden { .. })) => return Outcome::Failed(issue(&e)).into(),
                        Err(e) => {
                            run.failed += 1;
                            run.failure.get_or_insert_with(|| issue(&e));
                            // Coarse grains are re-read over their last intervals anyway.
                            if grain <= MINUTE_MS {
                                run.pending.push(Pending { kind: kind.clone(), region: region.clone(), resource_ids: ids.clone(), metrics: names, dimensions: dimensions.clone(), start_ms: from, end_ms: to });
                            }
                        }
                    }
                }
            }
        }
    }
    if run.ok == 0 && run.failed > 0 {
        // Nothing was read: the watermark stays and the next run reads the window again.
        return Outcome::Failed(run.failure.unwrap_or_else(|| issue(&AzureError::Other("every read failed".into())))).into();
    }

    // Every selected resource, each minute of the window, with its Resource Health status.
    let health = api.availability(creds, subscription).await.ok();
    for r in &resources {
        let mut tags = resource_tags(subscription, r);
        tags.insert("status".into(), health.as_ref().map_or_else(|| "query_failed".to_string(), |h| status(h.get(&health_id(r)))));
        let tags: Vec<(String, String)> = tags.into_iter().collect();
        let name = naming::count_name(&naming::named_type(&r.kind, &r.azure_kind));
        run.points.extend((start..end).step_by(MINUTE_MS as usize).map(|time_ms| Point { name: name.clone(), host: host(r), tags: tags.clone(), time_ms, value: 1.0 }));
    }

    for record in run.points.chunks(POINTS_PER_RECORD) {
        if let Err(e) = sink.publish(org, headers, otlp_json(PROVIDER, record)).await {
            return Outcome::Failed(Issue { kind: "other", message: format!("publishing to Kafka failed: {e}"), missing_action: None, backoff: Backoff::Exponential }).into();
        }
    }
    let warning = match run.failure {
        Some(f) => Some(Issue {
            message: format!("{} of {} Azure Monitor reads failed and are retried on the next runs: {}", run.failed, run.failed + run.ok, f.message),
            ..f
        }),
        None => run.refusal,
    };
    let mut pending = run.pending;
    if pending.len() > MAX_PENDING {
        pending.drain(..pending.len() - MAX_PENDING);
    }
    Crawl { outcome: Outcome::Collected { end_ms: end, metrics: resources.len(), points: run.points.len(), warning }, state: Some(State { marks, pending }) }
}

async fn load_state(st: &AppState, key: &JobKey) -> Result<State, sqlx::Error> {
    let row: Option<(Value, Value)> = sqlx::query_as("SELECT marks, pending FROM azure_metric_state WHERE org_id = $1 AND account_id = $2::uuid AND subscription = $3")
        .bind(&key.org_id)
        .bind(&key.account_id)
        .bind(&key.scope)
        .fetch_optional(&st.postgres)
        .await?;
    Ok(row.map(|(marks, pending)| State { marks: serde_json::from_value(marks).unwrap_or_default(), pending: serde_json::from_value(pending).unwrap_or_default() }).unwrap_or_default())
}

/// Keeps a subscription job's state (once its points are published).
async fn save_state(st: &AppState, key: &JobKey, state: &State) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO azure_metric_state (org_id, account_id, subscription, marks, pending) VALUES ($1, $2::uuid, $3, $4, $5) \
         ON CONFLICT (org_id, account_id, subscription) DO UPDATE SET marks = EXCLUDED.marks, pending = EXCLUDED.pending, updated_at = now()",
    )
    .bind(&key.org_id)
    .bind(&key.account_id)
    .bind(&key.scope)
    .bind(serde_json::to_value(&state.marks).unwrap_or_default())
    .bind(serde_json::to_value(&state.pending).unwrap_or_default())
    .execute(&st.postgres)
    .await?;
    Ok(())
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
        let state = load_state(st, job).await?;
        let run = crawl(self.api.as_ref(), self.sink.as_ref(), &headers, &job.org_id, &creds, &account.config, &job.scope, job.collected_until, &state, now_ms, self.interval_secs * 1000).await;
        if let (Outcome::Collected { .. }, Some(state)) = (&run.outcome, &run.state) {
            save_state(st, job, state).await?;
        }
        Ok(run.outcome)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cloud::testing::{published, FakeSink};
    use serde_json::json;
    use std::collections::HashSet;
    use std::sync::Mutex as StdMutex;

    /// A metrics call: type, region, resources, metrics, start, end, grain, split dimensions.
    type Call = (String, String, usize, usize, i64, i64, i64, Vec<String>);

    #[derive(Default)]
    struct FakeAzure {
        resources: Vec<Resource>,
        defs: HashMap<String, Vec<MetricDef>>,
        fail_metrics: Option<AzureError>,
        /// Types whose reads Azure Monitor throttles.
        throttled: StdMutex<Vec<String>>,
        /// Types whose batches Azure Monitor refuses.
        rejected: Vec<String>,
        /// Resource Health (None: it can't be read).
        health: Option<HashMap<String, String>>,
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
        async fn metric_definitions(&self, _c: &Credentials, id: &str) -> Result<Vec<MetricDef>, AzureError> {
            // A listed resource's type, else the one its id names (a storage account's `…/blobServices/default`).
            let named = || {
                let parts: Vec<&str> = id.split("/providers/").nth(1).unwrap_or_default().split('/').collect();
                std::iter::once(parts[0]).chain(parts.iter().skip(1).step_by(2).copied()).collect::<Vec<_>>().join("/")
            };
            let kind = self.resources.iter().find(|r| r.id == id).map(|r| r.kind.clone()).unwrap_or_else(named);
            Ok(self.defs.get(&kind).cloned().unwrap_or_default())
        }
        async fn metrics(&self, _c: &Credentials, q: &MetricsQuery<'_>) -> Result<Vec<Series>, AzureError> {
            if let Some(e) = &self.fail_metrics {
                return Err(e.clone());
            }
            if self.rejected.iter().any(|k| k == q.resource_type) {
                return Err(AzureError::Rejected(format!("{} is not a supported platform metric namespace", q.resource_type)));
            }
            if self.throttled.lock().unwrap().iter().any(|k| k == q.resource_type) {
                return Err(AzureError::Throttled("Too many requests".into()));
            }
            self.calls.lock().unwrap().push((q.resource_type.into(), q.region.into(), q.resource_ids.len(), q.metrics.len(), q.start_ms, q.end_ms, q.grain_ms, q.dimensions.to_vec()));
            // A point per grain over the window; split dimensions get one value each.
            let dims: Vec<(String, String)> = q.dimensions.iter().map(|d| (d.to_ascii_lowercase(), format!("{d}-0"))).collect();
            let points: Vec<(i64, f64)> = (q.start_ms..q.end_ms).step_by(q.grain_ms as usize).map(|t| (t, 1.0)).collect();
            Ok(q.resource_ids
                .iter()
                .flat_map(|id| {
                    let (dims, points) = (dims.clone(), points.clone());
                    q.metrics.iter().flat_map(move |m| {
                        let (dims, points, id) = (dims.clone(), points.clone(), id.clone());
                        q.aggregations.iter().map(move |a| Series { resource_id: id.to_uppercase(), metric: m.clone(), aggregation: a.to_string(), dimensions: dims.clone(), points: points.clone() })
                    })
                })
                .collect())
        }
        async fn availability(&self, _c: &Credentials, _s: &str) -> Result<HashMap<String, String>, AzureError> {
            self.health.clone().ok_or_else(|| AzureError::Forbidden { action: Some("Microsoft.ResourceHealth/availabilityStatuses/read".into()), message: "denied".into() })
        }
    }

    fn resource(name: &str, kind: &str, location: &str, tags: &[&str]) -> Resource {
        Resource {
            id: format!("/subscriptions/sub-a/resourceGroups/storefront/providers/{kind}/{name}"),
            name: name.into(),
            kind: kind.into(),
            azure_kind: String::new(),
            location: location.into(),
            resource_group: "storefront".into(),
            tags: tags.iter().map(|t| t.to_string()).collect(),
        }
    }

    fn def(name: &str, primary: &str, aggregations: &[&str], grain_ms: i64, dimensions: &[&str]) -> MetricDef {
        MetricDef { name: name.into(), primary: primary.into(), aggregations: aggregations.iter().map(|a| a.to_string()).collect(), grain_ms, dimensions: dimensions.iter().map(|d| d.to_string()).collect() }
    }

    const VM: &str = "Microsoft.Compute/virtualMachines";
    const SITE: &str = "Microsoft.Web/sites";
    const STORAGE: &str = "Microsoft.Storage/storageAccounts";
    const NOW: i64 = 1_700_000_000_000;
    const END: i64 = (NOW - LAG_MS) / 60_000 * 60_000;

    fn creds() -> Credentials {
        Credentials { tenant_id: "t".into(), client_id: "c".into(), client_secret: "s".into() }
    }

    fn cpu() -> MetricDef {
        def("Percentage CPU", "Average", &["Average", "Minimum", "Maximum", "Total", "Count"], 60_000, &[])
    }

    async fn run(azure: &FakeAzure, config: &Value, collected_until: Option<i64>, state: &State, now: i64) -> (Crawl, FakeSink) {
        let sink = FakeSink::default();
        let crawl = crawl(azure, &sink, &[], "acme", &creds(), config, "sub-a", collected_until, state, now, 600_000).await;
        (crawl, sink)
    }

    #[tokio::test]
    async fn a_subscription_run_publishes_the_selected_resources_metrics_as_datadog_names_them() {
        let azure = FakeAzure {
            resources: vec![
                resource("web-1", VM, "westeurope", &["env:prod"]),
                resource("web-2", VM, "westeurope", &["env:staging"]),
                resource("api", SITE, "northeurope", &[]),
                resource("assets", STORAGE, "westeurope", &[]),
            ],
            defs: HashMap::from([(VM.to_string(), vec![cpu()]), (SITE.to_string(), vec![def("Http5xx", "Total", &["Total"], 60_000, &["Instance"])])]),
            health: Some(HashMap::from([(resource("web-1", VM, "", &[]).id.to_ascii_lowercase(), "Available".to_string()), (resource("api", SITE, "", &[]).id.to_ascii_lowercase(), "Degraded".to_string())])),
            ..Default::default()
        };
        let config = json!({ "host_filters": "env:prod", "metrics_enabled_default": true, "resource_provider_configs": [{ "namespace": "Microsoft.Storage", "metrics_enabled": false }] });
        let (crawl, sink) = run(&azure, &config, None, &State::default(), NOW).await;
        // 3 series and the 2 selected resources' counts, each minute of the 10-minute window.
        assert_eq!(crawl.outcome, Outcome::Collected { end_ms: END, metrics: 2, points: (3 + 2) * 10, warning: None });
        // One batch per type and region, over one interval; App Service split by instance.
        let calls = azure.calls.lock().unwrap().clone();
        assert_eq!(calls.iter().map(|c| (c.0.as_str(), c.1.as_str(), c.2, c.7.clone())).collect::<Vec<_>>(), [(VM, "westeurope", 1, vec![]), (SITE, "northeurope", 1, vec!["Instance".to_string()])]);
        assert!(calls.iter().all(|c| c.4 == END - 600_000 && c.5 == END && c.6 == 60_000));
        let points = published(&sink);
        let names: HashSet<&str> = points.iter().map(|p| p.0.as_str()).collect();
        // The bare name is the primary aggregation (Http5xx: its total); a maximum is `.max`.
        assert_eq!(names, HashSet::from(["azure.vm.percentage_cpu", "azure.vm.percentage_cpu.max", "azure.app_services.http5xx", "azure.vm.count", "azure.app_services.count"]));
        let vm = points.iter().find(|p| p.0 == "azure.vm.percentage_cpu").unwrap();
        assert_eq!(vm.1.as_deref(), Some("web-1"));
        let tags: HashMap<_, _> = vm.2.iter().cloned().collect();
        assert_eq!((tags["subscription_id"].as_str(), tags["resource_group"].as_str(), tags["region"].as_str(), tags["env"].as_str()), ("sub-a", "storefront", "westeurope", "prod"));
        let site = points.iter().find(|p| p.0 == "azure.app_services.http5xx").unwrap();
        assert_eq!(site.1, None);
        assert!(site.2.contains(&("instance".to_string(), "Instance-0".to_string())), "{:?}", site.2);
        // Resource counts carry Resource Health's status.
        let status = |name: &str| points.iter().find(|p| p.0 == name).unwrap().2.iter().find(|t| t.0 == "status").unwrap().1.clone();
        assert_eq!((status("azure.vm.count"), status("azure.app_services.count")), ("Running".to_string(), "Degraded".to_string()));
        // Without Resource Health, the status says so.
        let azure = FakeAzure { health: None, ..azure };
        let (_, sink) = run(&azure, &config, None, &State::default(), NOW).await;
        assert!(published(&sink).iter().filter(|p| p.0.ends_with(".count")).all(|p| p.2.contains(&("status".to_string(), "query_failed".to_string()))));
    }

    #[tokio::test]
    async fn function_apps_are_named_as_datadogs_azure_functions() {
        let functions = Resource { azure_kind: "functionapp,linux".into(), ..resource("order-functions", SITE, "westeurope", &[]) };
        let web = Resource { azure_kind: "app,linux".into(), ..resource("storefront-web", SITE, "westeurope", &[]) };
        let azure = FakeAzure {
            resources: vec![functions.clone(), web.clone()],
            defs: HashMap::from([(SITE.to_string(), vec![def("Http5xx", "Total", &["Total"], 60_000, &["Instance"]), def("FunctionExecutionCount", "Total", &["Total"], 60_000, &[])])]),
            health: Some(HashMap::from([(functions.id.to_ascii_lowercase(), "Available".to_string())])),
            ..Default::default()
        };
        let (crawl, sink) = run(&azure, &json!({}), None, &State::default(), NOW).await;
        assert!(matches!(crawl.outcome, Outcome::Collected { warning: None, .. }), "{:?}", crawl.outcome);
        // Both are read from Azure Monitor's `Microsoft.Web/sites` namespace, in their own batches.
        let calls = azure.calls.lock().unwrap().clone();
        assert!(calls.iter().all(|c| c.0 == SITE), "{calls:?}");
        assert_eq!(calls.len(), 2 * 2);
        let points = published(&sink);
        let named = |prefix: &str| points.iter().filter(|p| p.0.starts_with(prefix)).map(|p| p.2.iter().find(|t| t.0 == "name").unwrap().1.clone()).collect::<HashSet<_>>();
        assert_eq!(named("azure.functions."), HashSet::from(["order-functions".to_string()]));
        assert_eq!(named("azure.app_services."), HashSet::from(["storefront-web".to_string()]));
        let names: HashSet<&str> = points.iter().map(|p| p.0.as_str()).collect();
        for n in ["azure.functions.function_execution_count", "azure.functions.http5xx", "azure.functions.count", "azure.app_services.http5xx", "azure.app_services.count"] {
            assert!(names.contains(n), "{n}: {names:?}");
        }
        let count = points.iter().find(|p| p.0 == "azure.functions.count").unwrap();
        assert!(count.2.contains(&("status".to_string(), "Running".to_string())));
    }

    #[tokio::test]
    async fn storage_accounts_blob_file_queue_and_table_services_are_read_as_their_own_types() {
        const BLOBS: &str = "Microsoft.Storage/storageAccounts/blobServices";
        const FILES: &str = "Microsoft.Storage/storageAccounts/fileServices";
        const QUEUES: &str = "Microsoft.Storage/storageAccounts/queueServices";
        const TABLES: &str = "Microsoft.Storage/storageAccounts/tableServices";
        let general = Resource { azure_kind: "StorageV2".into(), ..resource("storefrontassets", STORAGE, "westeurope", &["env:prod"]) };
        let shares = Resource { azure_kind: "FileStorage".into(), ..resource("ordershares", STORAGE, "westeurope", &[]) };
        let transactions = || def("Transactions", "Total", &["Total"], 60_000, &["ResponseType", "GeoType", "ApiName", "Authentication", "TransactionType"]);
        let azure = FakeAzure {
            resources: vec![general.clone(), shares.clone()],
            defs: HashMap::from([
                (STORAGE.to_string(), vec![transactions()]),
                (BLOBS.to_string(), vec![transactions(), def("BlobCapacity", "Average", &["Average"], HOUR_MS, &["BlobType", "Tier"])]),
                (FILES.to_string(), vec![transactions()]),
                (QUEUES.to_string(), vec![transactions()]),
                (TABLES.to_string(), vec![transactions()]),
            ]),
            health: Some(HashMap::from([(general.id.to_ascii_lowercase(), "Available".to_string()), (shares.id.to_ascii_lowercase(), "Degraded".to_string())])),
            ..Default::default()
        };
        let (crawl, sink) = run(&azure, &json!({}), None, &State::default(), NOW).await;
        assert!(matches!(crawl.outcome, Outcome::Collected { warning: None, .. }), "{:?}", crawl.outcome);
        // A general-purpose account has the four services, a file storage account only files; read per service type.
        let calls = azure.calls.lock().unwrap().clone();
        let read = |kind: &str| calls.iter().filter(|c| c.0 == kind && c.6 == MINUTE_MS).map(|c| c.2).sum::<usize>();
        assert_eq!((read(STORAGE), read(BLOBS), read(FILES), read(QUEUES), read(TABLES)), (2, 1, 2, 1, 1));
        assert_eq!(calls.iter().find(|c| c.0 == BLOBS && c.6 == HOUR_MS).unwrap().7, ["BlobType", "Tier"]);
        let points = published(&sink);
        let blob = points.iter().find(|p| p.0 == "azure.storage_storageaccounts_blobservices.transactions").unwrap();
        let tags: HashMap<_, _> = blob.2.iter().cloned().collect();
        // Tagged as their account (its name, resource group and Azure tags), split by API and response type.
        assert_eq!((tags["name"].as_str(), tags["resource_group"].as_str(), tags["env"].as_str(), tags["apiname"].as_str()), ("storefrontassets", "storefront", "prod", "ApiName-0"));
        assert!(points.iter().any(|p| p.0 == "azure.storage_storageaccounts_blobservices.blob_capacity"));
        // Their resource counts carry the account's Resource Health status.
        let status = |name: &str, account: &str| {
            let p = points.iter().find(|p| p.0 == name && p.2.contains(&("name".to_string(), account.to_string()))).expect("the count of a storage service");
            p.2.iter().find(|t| t.0 == "status").unwrap().1.clone()
        };
        assert_eq!(status("azure.storage_storageaccounts_blobservices.count", "storefrontassets"), "Running");
        assert_eq!(status("azure.storage_storageaccounts_fileservices.count", "ordershares"), "Degraded");
        assert_eq!(status("azure.storage_storageaccounts_tableservices.count", "storefrontassets"), "Running");
        assert!(!points.iter().any(|p| p.0 == "azure.storage_storageaccounts_queueservices.count" && p.2.contains(&("name".to_string(), "ordershares".to_string()))));
    }

    #[tokio::test]
    async fn large_groups_are_split_by_the_batch_limits_and_a_missing_role_fails_the_run() {
        let resources: Vec<Resource> = (0..120).map(|i| resource(&format!("vm-{i}"), VM, "westeurope", &[])).collect();
        let defs: Vec<MetricDef> = (0..25).map(|i| def(&format!("Metric {i}"), "Average", &[], 60_000, &[])).collect();
        let azure = FakeAzure { resources: resources.clone(), defs: HashMap::from([(VM.to_string(), defs)]), ..Default::default() };
        run(&azure, &json!({}), None, &State::default(), NOW).await;
        let calls = azure.calls.lock().unwrap().clone();
        assert_eq!(calls.len(), 3 * 2);
        assert!(calls.iter().all(|c| c.2 <= 50 && c.3 <= 20));

        let denied = AzureError::Forbidden { action: Some("Microsoft.Insights/metrics/read".into()), message: "m".into() };
        let azure = FakeAzure { resources, defs: HashMap::from([(VM.to_string(), vec![cpu()])]), fail_metrics: Some(denied.clone()), ..Default::default() };
        let (crawl, sink) = run(&azure, &json!({}), None, &State::default(), NOW).await;
        assert_eq!(crawl, Crawl { outcome: Outcome::Failed(issue(&denied)), state: None });
        assert!(sink.records.lock().unwrap().is_empty());
        assert_eq!(issue(&denied).missing_action.as_deref(), Some("Microsoft.Insights/metrics/read"));
    }

    #[tokio::test]
    async fn throttled_reads_publish_the_rest_and_are_retried_on_the_next_run() {
        let azure = FakeAzure {
            resources: vec![resource("web-1", VM, "westeurope", &[]), resource("api", SITE, "northeurope", &[])],
            defs: HashMap::from([(VM.to_string(), vec![cpu()]), (SITE.to_string(), vec![def("Requests", "Total", &["Total"], 60_000, &[])])]),
            throttled: StdMutex::new(vec![SITE.to_string()]),
            health: Some(HashMap::new()),
            ..Default::default()
        };
        let (crawl, sink) = run(&azure, &json!({}), None, &State::default(), NOW).await;
        let Outcome::Collected { end_ms, warning: Some(w), .. } = &crawl.outcome else { panic!("{:?}", crawl.outcome) };
        assert_eq!((*end_ms, w.kind), (END, "throttled"));
        assert!(w.message.contains("1 of 2 Azure Monitor reads failed"), "{}", w.message);
        assert!(published(&sink).iter().any(|p| p.0 == "azure.vm.percentage_cpu"));
        assert!(!published(&sink).iter().any(|p| p.0 == "azure.app_services.requests"));
        let state = crawl.state.unwrap();
        assert_eq!(state.pending, [Pending { kind: SITE.into(), region: "northeurope".into(), resource_ids: vec![resource("api", SITE, "", &[]).id], metrics: vec!["Requests".into()], dimensions: vec![], start_ms: END - 600_000, end_ms: END }]);

        // The next run reads the failed window as well as its own.
        azure.throttled.lock().unwrap().clear();
        azure.calls.lock().unwrap().clear();
        let (crawl, sink) = run(&azure, &json!({}), Some(END), &state, NOW + 600_000).await;
        assert!(matches!(crawl.outcome, Outcome::Collected { warning: None, .. }), "{:?}", crawl.outcome);
        assert!(crawl.state.unwrap().pending.is_empty());
        let site_windows: Vec<(i64, i64)> = azure.calls.lock().unwrap().iter().filter(|c| c.0 == SITE).map(|c| (c.4, c.5)).collect();
        assert_eq!(site_windows, [(END - 600_000, END), (END, END + 600_000)]);
        assert_eq!(published(&sink).iter().filter(|p| p.0 == "azure.app_services.requests").count(), 20);

        // Every read throttled: the run fails and the watermark stays.
        azure.throttled.lock().unwrap().extend([VM.to_string(), SITE.to_string()]);
        let (crawl, sink) = run(&azure, &json!({}), Some(END), &State::default(), NOW + 600_000).await;
        assert!(matches!(crawl.outcome, Outcome::Failed(Issue { kind: "throttled", .. })), "{:?}", crawl.outcome);
        assert!(sink.records.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn coarse_grains_are_read_over_their_last_intervals_and_each_point_is_published_once() {
        let azure = FakeAzure {
            resources: vec![resource("assets", STORAGE, "westeurope", &[])],
            defs: HashMap::from([(STORAGE.to_string(), vec![def("UsedCapacity", "Average", &["Average"], 3_600_000, &[]), def("Transactions", "Total", &["Total"], 60_000, &["ApiName", "ResponseType", "GeoType"])])]),
            health: Some(HashMap::new()),
            ..Default::default()
        };
        let (crawl, sink) = run(&azure, &json!({}), None, &State::default(), NOW).await;
        let calls = azure.calls.lock().unwrap().clone();
        let hourly = calls.iter().find(|c| c.6 == HOUR_MS).unwrap();
        let hour_end = END / HOUR_MS * HOUR_MS;
        assert_eq!((hourly.4, hourly.5), (hour_end - 3 * HOUR_MS, hour_end));
        let minute = calls.iter().find(|c| c.6 == MINUTE_MS).unwrap();
        assert_eq!(minute.7, ["ApiName", "ResponseType"]);
        let used = |sink: &FakeSink| published(sink).iter().filter(|p| p.0 == "azure.storage.used_capacity").count();
        assert_eq!(used(&sink), 3);
        let state = crawl.state.unwrap();
        assert_eq!(state.marks.values().copied().collect::<Vec<_>>(), [hour_end - HOUR_MS]);

        // Within the same hour, the hourly metrics aren't read again.
        azure.calls.lock().unwrap().clear();
        let within = END + 2 * MINUTE_MS;
        assert_eq!(within / HOUR_MS, END / HOUR_MS);
        let (_, sink) = run(&azure, &json!({}), Some(END), &state, within + LAG_MS).await;
        assert!(azure.calls.lock().unwrap().iter().all(|c| c.6 == MINUTE_MS));
        assert_eq!(used(&sink), 0);

        // The next hour re-reads 3 hours and publishes only the new one.
        let next = hour_end + HOUR_MS + 5 * MINUTE_MS;
        let (crawl, sink) = run(&azure, &json!({}), Some(next - 600_000), &state, next + LAG_MS).await;
        assert_eq!(used(&sink), 1);
        assert_eq!(crawl.state.unwrap().marks.values().copied().collect::<Vec<_>>(), [hour_end]);
    }

    #[tokio::test]
    async fn global_resources_are_read_as_their_own_group() {
        const ZONES: &str = "Microsoft.Network/dnszones";
        let azure = FakeAzure {
            resources: vec![resource("example.com", ZONES, "global", &[]), resource("example.org", ZONES, "Global", &[])],
            defs: HashMap::from([(ZONES.to_string(), vec![def("QueryVolume", "Total", &["Total"], 300_000, &[])])]),
            health: Some(HashMap::new()),
            ..Default::default()
        };
        let (crawl, sink) = run(&azure, &json!({}), None, &State::default(), NOW).await;
        assert!(matches!(crawl.outcome, Outcome::Collected { warning: None, .. }));
        let calls = azure.calls.lock().unwrap().clone();
        assert_eq!(calls.iter().map(|c| (c.1.as_str(), c.2, c.6)).collect::<Vec<_>>(), [("global", 2, 300_000)]);
        assert!(published(&sink).iter().any(|p| p.0 == "azure.network_dnszones.query_volume"));
        assert_eq!(published(&sink).iter().filter(|p| p.0 == "azure.network_dnszones.count").count(), 20);
    }

    #[tokio::test]
    async fn a_batch_azure_refuses_is_skipped_with_a_warning() {
        const RESTORE_POINTS: &str = "Microsoft.Compute/restorePointCollections";
        let azure = FakeAzure {
            resources: vec![resource("web-1", VM, "westeurope", &[]), resource("rp", RESTORE_POINTS, "westeurope", &[])],
            defs: HashMap::from([(VM.to_string(), vec![cpu()]), (RESTORE_POINTS.to_string(), vec![def("Anything", "Average", &[], 60_000, &[])])]),
            rejected: vec![RESTORE_POINTS.to_string()],
            health: Some(HashMap::new()),
            ..Default::default()
        };
        let (crawl, sink) = run(&azure, &json!({}), None, &State::default(), NOW).await;
        let Outcome::Collected { points, warning: Some(w), .. } = crawl.outcome else { panic!("{:?}", crawl.outcome) };
        assert_eq!((points, w.kind), ((2 + 2) * 10, "other"));
        assert!(w.message.contains(RESTORE_POINTS));
        assert_eq!(published(&sink).len(), 40);
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

        // A subscription's state is kept between runs and goes with the app registration.
        let sub = JobKey { scope: "sub-a".into(), ..key(&with_secret.id) };
        let state = State { marks: HashMap::from([("k".to_string(), 1)]), pending: vec![Pending { kind: VM.into(), region: "westeurope".into(), resource_ids: vec!["r".into()], metrics: vec!["m".into()], dimensions: vec![], start_ms: 1, end_ms: 2 }] };
        save_state(st, &sub, &state).await.unwrap();
        assert_eq!(load_state(st, &sub).await.unwrap(), state);
        sqlx::query("DELETE FROM azure_accounts WHERE org_id = $1 AND id = $2::uuid").bind(&org).bind(&with_secret.id).execute(&st.postgres).await.unwrap();
        assert_eq!(load_state(st, &sub).await.unwrap(), State::default());
    }
}
