//! Infrastructure → Resources → Cloudcraft: an architecture diagram of the resource catalog.
//!
//! The diagram is a tree of groups (account → region → zone for hosts, cluster → namespace for
//! Kubernetes, region → cluster for ECS; any catalog key can be chosen) whose leaves are resources:
//! hosts, Kubernetes nodes, workloads, services and ingresses, ECS services and tasks. Built only
//! from [`super::catalog::load_all`]; overlays add:
//! - health: the resource's own health, monitors alerting on it (`monitor_groups`) and the APM error
//!   rate of its service over the last hour;
//! - dependencies: service-to-service calls from APM spans over the last hour.
//!
//! Large inventories stay drawable: a group shows at most [`MAX_NODES_PER_GROUP`] resources (the
//! rest become one "+N" block) and at most [`MAX_GROUPS_PER_LEVEL`] subgroups (the smallest merge
//! into "Other").

use std::collections::{BTreeMap, HashMap};

use axum::extract::State;
use axum::Json;
use oc_query::{parse_query, Param};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use super::apm::ENTRY;
use super::catalog::{self, CatalogResource, CatalogType, Faceted};
use super::kubernetes::KubeHealth;
use super::logs::bind;
use super::monitor_engine::parse_group_key;
use crate::auth::AuthContext;
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;

pub const MAX_NODES_PER_GROUP: usize = 48;
pub const MAX_GROUPS_PER_LEVEL: usize = 24;
const MAX_LEVELS: usize = 4;
const MAX_EDGES: u64 = 2000;
const OVERLAY_WINDOW_MS: i64 = 3_600_000;
/// APM error rates that make a service's resources warn or fail in the health overlay.
const ERROR_RATE_WARNING: f64 = 0.01;
const ERROR_RATE_ERROR: f64 = 0.05;
/// Group keys the diagram can use.
const GROUP_KEYS: &[&str] = &["cloud", "account", "region", "availability_zone", "cluster", "namespace", "env", "service", "team"];
/// Value of the merged group of small groups.
pub const OTHER_GROUP: &str = "__other__";

#[derive(Serialize, Deserialize, ToSchema, Clone, Copy, PartialEq, Eq, Debug, Hash)]
#[serde(rename_all = "snake_case")]
pub enum CloudMapProvider {
    Kubernetes,
    Hosts,
    Ecs,
}

impl CloudMapProvider {
    fn includes(self, t: CatalogType) -> bool {
        use CatalogType::*;
        match self {
            CloudMapProvider::Kubernetes => matches!(t, KubeNode | KubeDeployment | KubeStatefulSet | KubeDaemonSet | KubeCronJob | KubeService | KubeIngress),
            CloudMapProvider::Hosts => t == Host,
            CloudMapProvider::Ecs => matches!(t, EcsService | EcsTask),
        }
    }

    pub fn default_group_by(self) -> &'static [&'static str] {
        match self {
            CloudMapProvider::Kubernetes => &["cluster", "namespace"],
            CloudMapProvider::Hosts => &["cloud", "region", "availability_zone"],
            CloudMapProvider::Ecs => &["region", "cluster"],
        }
    }
}

#[derive(Serialize, Deserialize, ToSchema, Clone, Copy, PartialEq, Eq, Debug, Default)]
#[serde(rename_all = "snake_case")]
pub enum CloudMapOverlay {
    #[default]
    Default,
    Health,
    Dependencies,
}

#[derive(Serialize, ToSchema, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Default)]
#[serde(rename_all = "snake_case")]
pub enum CloudMapHealth {
    #[default]
    Unknown,
    Ok,
    Warning,
    Error,
}

impl From<KubeHealth> for CloudMapHealth {
    fn from(h: KubeHealth) -> Self {
        match h {
            KubeHealth::Ok => CloudMapHealth::Ok,
            KubeHealth::Warning => CloudMapHealth::Warning,
            KubeHealth::Error => CloudMapHealth::Error,
            KubeHealth::Info => CloudMapHealth::Unknown,
        }
    }
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct CloudMapRequest {
    pub provider: CloudMapProvider,
    /// Catalog search narrowing the resources (`env:prod team:checkout`, free text on names).
    #[serde(default)]
    pub query: String,
    /// Group keys, outermost first; empty for the provider's default.
    #[serde(default)]
    pub group_by: Vec<String>,
    #[serde(default)]
    pub overlay: CloudMapOverlay,
}

/// One block of the diagram: a resource, or `aggregate` resources folded into one.
#[derive(Serialize, ToSchema, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct CloudMapNode {
    pub id: String,
    pub name: String,
    pub resource_type: CatalogType,
    pub health: CloudMapHealth,
    pub service: Option<String>,
    /// Monitors alerting (or warning) on the resource; health overlay only.
    pub alerts: u32,
    /// APM error rate of the resource's service over the last hour (0–1); health overlay only.
    pub error_rate: Option<f64>,
    /// How many resources this block stands for when folded ("+N"); null for one resource.
    pub aggregate: Option<usize>,
    /// The catalog row, for the side panel (null for folded blocks).
    pub resource: Option<CatalogResource>,
}

#[derive(Serialize, ToSchema, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct CloudMapGroup {
    /// The group key (`region`), or `root`.
    pub key: String,
    /// The value (`eu-west-1`); "" when the resources have none; `__other__` for merged small groups.
    pub value: String,
    /// Resources under the group, folded ones included.
    pub count: usize,
    /// Worst health of the group's resources.
    pub health: CloudMapHealth,
    #[schema(no_recursion)]
    pub groups: Vec<CloudMapGroup>,
    pub nodes: Vec<CloudMapNode>,
}

/// A service calling another (dependencies overlay).
#[derive(Serialize, ToSchema, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CloudMapEdge {
    pub from: String,
    pub to: String,
    pub requests: u64,
    pub error_rate: f64,
}

/// A provider tab with how many resources it has and, for hosts, their clouds.
#[derive(Serialize, ToSchema, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CloudMapProviderSummary {
    pub provider: CloudMapProvider,
    pub resources: usize,
    pub clouds: Vec<String>,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct CloudMap {
    pub providers: Vec<CloudMapProviderSummary>,
    /// The group keys used.
    pub group_by: Vec<String>,
    pub root: CloudMapGroup,
    pub edges: Vec<CloudMapEdge>,
    /// Matching resources, folded ones included.
    pub total: usize,
}

// ================================================================ pure helpers

/// The value of a group key for a resource ("" when it has none).
fn group_value(r: &CatalogResource, key: &str) -> String {
    r.values(key).first().map(|v| v.to_string()).unwrap_or_default()
}

/// ECS tasks that belong to a service are drawn as their service.
fn drawn(r: &CatalogResource) -> bool {
    r.resource_type != CatalogType::EcsTask || r.values("ecs_service").is_empty()
}

pub fn providers(all: &[CatalogResource]) -> Vec<CloudMapProviderSummary> {
    [CloudMapProvider::Kubernetes, CloudMapProvider::Hosts, CloudMapProvider::Ecs]
        .into_iter()
        .map(|p| {
            let rows: Vec<&CatalogResource> = all.iter().filter(|r| p.includes(r.resource_type) && drawn(r)).collect();
            let mut clouds: Vec<String> = rows.iter().filter_map(|r| r.cloud.clone()).collect();
            clouds.sort();
            clouds.dedup();
            CloudMapProviderSummary { provider: p, resources: rows.len(), clouds }
        })
        .collect()
}

/// The group keys of a request: known keys, at most four, the provider's default when empty.
pub fn group_keys(provider: CloudMapProvider, requested: &[String]) -> Result<Vec<String>, ApiError> {
    if requested.is_empty() {
        return Ok(provider.default_group_by().iter().map(|k| k.to_string()).collect());
    }
    if requested.len() > MAX_LEVELS {
        return Err(ApiError::BadRequest(format!("Group by at most {MAX_LEVELS} keys")));
    }
    if let Some(bad) = requested.iter().find(|k| !GROUP_KEYS.contains(&k.as_str())) {
        return Err(ApiError::BadRequest(format!("Can't group by '{bad}'")));
    }
    Ok(requested.to_vec())
}

fn node(r: &CatalogResource) -> CloudMapNode {
    CloudMapNode {
        id: r.id.clone(),
        name: r.name.clone(),
        resource_type: r.resource_type,
        health: r.health.map(CloudMapHealth::from).unwrap_or_default(),
        service: r.service.clone(),
        alerts: 0,
        error_rate: None,
        aggregate: None,
        resource: Some(r.clone()),
    }
}

/// Worst first, then by name: what stays visible when a group folds.
fn order(nodes: &mut [CloudMapNode]) {
    nodes.sort_by(|a, b| b.health.cmp(&a.health).then(b.alerts.cmp(&a.alerts)).then(a.name.cmp(&b.name)).then(a.id.cmp(&b.id)));
}

/// Keeps a group's first resources and folds the rest into one block.
fn fold(mut nodes: Vec<CloudMapNode>) -> Vec<CloudMapNode> {
    order(&mut nodes);
    if nodes.len() <= MAX_NODES_PER_GROUP {
        return nodes;
    }
    let rest = nodes.split_off(MAX_NODES_PER_GROUP - 1);
    let first = &rest[0];
    nodes.push(CloudMapNode {
        id: format!("more:{}", first.id),
        name: format!("+{} more", rest.len()),
        resource_type: first.resource_type,
        health: rest.iter().map(|n| n.health).max().unwrap_or_default(),
        service: None,
        alerts: rest.iter().map(|n| n.alerts).sum(),
        error_rate: None,
        aggregate: Some(rest.len()),
        resource: None,
    });
    nodes
}

/// Builds the group tree of `rows` for `keys` (outermost first).
pub fn build_tree(rows: Vec<CloudMapNode>, keys: &[String], values: &dyn Fn(&CloudMapNode, &str) -> String) -> CloudMapGroup {
    fn level(key: &str, value: String, rows: Vec<CloudMapNode>, keys: &[String], values: &dyn Fn(&CloudMapNode, &str) -> String) -> CloudMapGroup {
        let count = rows.len();
        let health = rows.iter().map(|n| n.health).max().unwrap_or_default();
        let Some((next, deeper)) = keys.split_first() else {
            return CloudMapGroup { key: key.into(), value, count, health, groups: vec![], nodes: fold(rows) };
        };
        let mut by: BTreeMap<String, Vec<CloudMapNode>> = BTreeMap::new();
        for n in rows {
            by.entry(values(&n, next)).or_default().push(n);
        }
        let mut parts: Vec<(String, Vec<CloudMapNode>)> = by.into_iter().collect();
        // Largest first; named groups before the one without a value.
        parts.sort_by(|a, b| a.0.is_empty().cmp(&b.0.is_empty()).then(b.1.len().cmp(&a.1.len())).then(a.0.cmp(&b.0)));
        let mut groups: Vec<CloudMapGroup> = Vec::new();
        if parts.len() > MAX_GROUPS_PER_LEVEL {
            let small: Vec<CloudMapNode> = parts.split_off(MAX_GROUPS_PER_LEVEL - 1).into_iter().flat_map(|(_, n)| n).collect();
            groups.extend(parts.into_iter().map(|(v, n)| level(next, v, n, deeper, values)));
            groups.push(level(next, OTHER_GROUP.into(), small, &[], values));
        } else {
            groups.extend(parts.into_iter().map(|(v, n)| level(next, v, n, deeper, values)));
        }
        CloudMapGroup { key: key.into(), value, count, health, groups, nodes: vec![] }
    }
    level("root", String::new(), rows, keys, values)
}

/// Monitor groups (as `parse_group_key` maps) that name a specific resource, matched against it.
fn alerts_on(r: &CatalogResource, alerting: &[BTreeMap<String, String>]) -> u32 {
    const IDENTIFYING: &[&str] = &["host", "service", "kube_deployment", "kube_stateful_set", "kube_daemon_set", "kube_service", "kube_node", "ecs_service", "kube_cronjob", "kube_ingress"];
    let host = r.resource_type == CatalogType::Host || r.resource_type == CatalogType::KubeNode;
    alerting
        .iter()
        .filter(|tags| tags.keys().any(|k| IDENTIFYING.contains(&k.as_str())))
        .filter(|tags| {
            tags.iter().all(|(k, v)| {
                let vals = if k == "host" && host { vec![r.name.as_str()] } else { r.values(k) };
                vals.contains(&v.as_str())
            })
        })
        .count() as u32
}

/// Applies the health overlay: monitors in alert and the service's error rate raise a node's health.
fn apply_health(n: &mut CloudMapNode, r: &CatalogResource, alerting: &[BTreeMap<String, String>], error_rates: &HashMap<String, f64>) {
    n.alerts = alerts_on(r, alerting);
    if n.alerts > 0 {
        n.health = n.health.max(CloudMapHealth::Error);
    }
    if let Some(rate) = r.service.as_ref().and_then(|s| error_rates.get(s)) {
        n.error_rate = Some(*rate);
        let h = if *rate >= ERROR_RATE_ERROR {
            CloudMapHealth::Error
        } else if *rate >= ERROR_RATE_WARNING {
            CloudMapHealth::Warning
        } else {
            CloudMapHealth::Ok
        };
        n.health = n.health.max(h);
    }
}

// ================================================================ telemetry

async fn alerting_groups(st: &AppState, org: &str) -> Result<Vec<BTreeMap<String, String>>, ApiError> {
    let rows: Vec<(String,)> = sqlx::query_as("SELECT g.group_key FROM monitor_groups g JOIN monitors m ON m.id = g.monitor_id WHERE m.org_id = $1 AND g.status IN ('alert', 'warn') LIMIT 5000")
        .bind(org)
        .fetch_all(&st.postgres)
        .await?;
    Ok(rows.into_iter().map(|(k,)| parse_group_key(&k)).filter(|t| !t.is_empty()).collect())
}

fn window() -> Vec<Param> {
    let to = super::metrics::now_ms();
    vec![Param::Int(to - OVERLAY_WINDOW_MS), Param::Int(to)]
}

async fn service_error_rates(st: &AppState, org: &str) -> Result<HashMap<String, f64>, ApiError> {
    let rows = bind(
        st.ch(org).query(&format!(
            "SELECT toString(ServiceName) AS service, count() AS hits, countIf(StatusCode = 'Error') AS errors FROM otel.otel_traces \
             WHERE OrgId = {{oc_org:String}} AND Timestamp >= fromUnixTimestamp64Milli(?) AND Timestamp <= fromUnixTimestamp64Milli(?) AND {ENTRY} GROUP BY service"
        )),
        &window(),
    )
    .fetch_all::<(String, u64, u64)>()
    .await?;
    Ok(rows.into_iter().filter(|(_, hits, _)| *hits > 0).map(|(s, hits, errors)| (s, errors as f64 / hits as f64)).collect())
}

async fn service_edges(st: &AppState, org: &str) -> Result<Vec<CloudMapEdge>, ApiError> {
    let mut params = window();
    params.push(Param::Int(MAX_EDGES as i64));
    let rows = bind(
        st.ch(org).query(
            "SELECT toString(ServiceName) AS src, SpanAttributes['peer.service'] AS dst, count() AS hits, countIf(StatusCode = 'Error') AS errors FROM otel.otel_traces \
             WHERE OrgId = {oc_org:String} AND Timestamp >= fromUnixTimestamp64Milli(?) AND Timestamp <= fromUnixTimestamp64Milli(?) AND SpanKind IN ('Client', 'Producer') \
             AND dst != '' AND dst != src GROUP BY src, dst ORDER BY hits DESC LIMIT ?",
        ),
        &params,
    )
    .fetch_all::<(String, String, u64, u64)>()
    .await?;
    Ok(rows.into_iter().map(|(from, to, hits, errors)| CloudMapEdge { from, to, requests: hits, error_rate: if hits == 0 { 0.0 } else { errors as f64 / hits as f64 } }).collect())
}

// ================================================================ handler

/// The diagram of one provider's resources, grouped, with the requested overlay.
#[utoipa::path(post, path = "/api/v1/infrastructure/cloud-map", operation_id = "cloud_map", request_body = CloudMapRequest,
    responses((status = 200, body = CloudMap), (status = 400, body = crate::error::ErrorBody)), tag = "infrastructure")]
pub async fn cloud_map(State(st): State<AppState>, ctx: AuthContext, Json(req): Json<CloudMapRequest>) -> ApiResult<CloudMap> {
    let keys = group_keys(req.provider, &req.group_by)?;
    let org = &ctx.org_id;
    let all = catalog::load_all(&st, org).await?;
    let terms = parse_query(&req.query);
    let rows: Vec<&CatalogResource> = all.iter().filter(|r| req.provider.includes(r.resource_type) && drawn(r) && r.matches(&terms)).collect();
    let (alerting, error_rates, edges) = match req.overlay {
        CloudMapOverlay::Default => (vec![], HashMap::new(), vec![]),
        CloudMapOverlay::Health => {
            let (a, e) = tokio::try_join!(alerting_groups(&st, org), service_error_rates(&st, org))?;
            (a, e, vec![])
        }
        CloudMapOverlay::Dependencies => (vec![], HashMap::new(), service_edges(&st, org).await?),
    };
    let nodes: Vec<CloudMapNode> = rows
        .iter()
        .map(|r| {
            let mut n = node(r);
            if req.overlay == CloudMapOverlay::Health {
                apply_health(&mut n, r, &alerting, &error_rates);
            }
            n
        })
        .collect();
    let total = nodes.len();
    let root = build_tree(nodes, &keys, &|n, k| n.resource.as_ref().map(|r| group_value(r, k)).unwrap_or_default());
    // Only calls between services on the diagram.
    let shown: std::collections::HashSet<&str> = rows.iter().filter_map(|r| r.service.as_deref()).collect();
    let edges = edges.into_iter().filter(|e| shown.contains(e.from.as_str()) && shown.contains(e.to.as_str())).collect();
    Ok(Json(CloudMap { providers: providers(&all), group_by: keys, root, edges, total }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::routes::catalog::CatalogCategory;

    fn res(t: CatalogType, name: &str, region: &str, health: KubeHealth, tags: &[&str]) -> CatalogResource {
        CatalogResource {
            id: format!("id-{name}"),
            resource_type: t,
            resource_category: CatalogCategory::Kubernetes,
            name: name.into(),
            cloud: Some("aws".into()),
            account: Some("123456789012".into()),
            region: Some(region.into()).filter(|r: &String| !r.is_empty()),
            cluster: Some("shop-prod-eu".into()),
            namespace: tags.iter().find_map(|t| t.strip_prefix("kube_namespace:")).map(str::to_string),
            env: tags.iter().find_map(|t| t.strip_prefix("env:")).map(str::to_string),
            service: tags.iter().find_map(|t| t.strip_prefix("service:")).map(str::to_string),
            team: None,
            status: None,
            health: Some(health),
            created_at: None,
            last_seen: 0,
            tags: tags.iter().map(|t| t.to_string()).collect(),
            attributes: BTreeMap::new(),
            kube_kind: None,
        }
    }

    fn tree(rows: &[CatalogResource], keys: &[&str]) -> CloudMapGroup {
        let keys: Vec<String> = keys.iter().map(|k| k.to_string()).collect();
        build_tree(rows.iter().map(node).collect(), &keys, &|n, k| n.resource.as_ref().map(|r| group_value(r, k)).unwrap_or_default())
    }

    #[test]
    fn resources_group_by_each_key_with_counts_and_worst_health() {
        let rows = vec![
            res(CatalogType::Host, "a", "eu-west-1", KubeHealth::Ok, &[]),
            res(CatalogType::Host, "b", "eu-west-1", KubeHealth::Error, &[]),
            res(CatalogType::Host, "c", "us-east-1", KubeHealth::Ok, &[]),
            res(CatalogType::Host, "d", "", KubeHealth::Warning, &[]),
        ];
        let root = tree(&rows, &["account", "region"]);
        assert_eq!((root.count, root.health), (4, CloudMapHealth::Error));
        let account = &root.groups[0];
        assert_eq!((account.key.as_str(), account.value.as_str(), account.count), ("account", "123456789012", 4));
        // Largest first, the group without a value last.
        let regions: Vec<(&str, usize, CloudMapHealth)> = account.groups.iter().map(|g| (g.value.as_str(), g.count, g.health)).collect();
        assert_eq!(regions, vec![("eu-west-1", 2, CloudMapHealth::Error), ("us-east-1", 1, CloudMapHealth::Ok), ("", 1, CloudMapHealth::Warning)]);
        // Leaves are worst first.
        assert_eq!(account.groups[0].nodes.iter().map(|n| n.name.as_str()).collect::<Vec<_>>(), vec!["b", "a"]);
        assert!(account.groups[0].groups.is_empty());
    }

    #[test]
    fn big_groups_fold_their_resources_and_many_groups_merge_into_other() {
        let many: Vec<CatalogResource> = (0..MAX_NODES_PER_GROUP + 10).map(|i| res(CatalogType::Host, &format!("h{i:03}"), "eu-west-1", KubeHealth::Ok, &[])).collect();
        let root = tree(&many, &["region"]);
        let nodes = &root.groups[0].nodes;
        assert_eq!(nodes.len(), MAX_NODES_PER_GROUP);
        let more = nodes.last().unwrap();
        assert_eq!((more.aggregate, more.name.as_str()), (Some(11), "+11 more"));
        assert!(more.resource.is_none());
        assert_eq!(root.groups[0].count, MAX_NODES_PER_GROUP + 10);

        let spread: Vec<CatalogResource> = (0..MAX_GROUPS_PER_LEVEL + 5).map(|i| res(CatalogType::Host, &format!("h{i}"), &format!("region-{i:02}"), KubeHealth::Ok, &[])).collect();
        let root = tree(&spread, &["region", "availability_zone"]);
        assert_eq!(root.groups.len(), MAX_GROUPS_PER_LEVEL);
        let other = root.groups.last().unwrap();
        assert_eq!((other.value.as_str(), other.count), (OTHER_GROUP, 6));
        // The merged group is flat: its resources are listed directly.
        assert!(other.groups.is_empty());
        assert_eq!(other.nodes.len(), 6);
    }

    #[test]
    fn providers_pick_their_resource_types_and_ecs_tasks_draw_as_their_service() {
        let rows = vec![
            res(CatalogType::KubeDeployment, "order-service", "", KubeHealth::Ok, &[]),
            res(CatalogType::KubePod, "order-service-abc", "", KubeHealth::Ok, &[]),
            res(CatalogType::Host, "i-0a1b2c3d4e5f60001", "eu-west-1", KubeHealth::Ok, &[]),
            res(CatalogType::EcsService, "storefront", "eu-west-1", KubeHealth::Ok, &[]),
            res(CatalogType::EcsTask, "t1", "eu-west-1", KubeHealth::Ok, &["ecs_service:storefront"]),
            res(CatalogType::EcsTask, "t2", "eu-west-1", KubeHealth::Ok, &[]),
        ];
        let p = providers(&rows);
        assert_eq!(p.iter().map(|x| (x.provider, x.resources)).collect::<Vec<_>>(), vec![(CloudMapProvider::Kubernetes, 1), (CloudMapProvider::Hosts, 1), (CloudMapProvider::Ecs, 2)]);
        assert_eq!(p[1].clouds, vec!["aws"]);
    }

    #[test]
    fn group_keys_default_per_provider_and_reject_unknown_keys() {
        assert_eq!(group_keys(CloudMapProvider::Kubernetes, &[]).unwrap(), vec!["cluster", "namespace"]);
        assert_eq!(group_keys(CloudMapProvider::Hosts, &["team".into()]).unwrap(), vec!["team"]);
        assert!(group_keys(CloudMapProvider::Hosts, &["password".into()]).is_err());
        assert!(group_keys(CloudMapProvider::Hosts, &vec!["region".to_string(); 5]).is_err());
    }

    #[test]
    fn health_overlay_counts_monitors_naming_the_resource_and_service_error_rates() {
        let r = res(CatalogType::KubeDeployment, "order-service", "", KubeHealth::Ok, &["kube_deployment:order-service", "service:order-service", "env:prod"]);
        let host = res(CatalogType::Host, "i-0a1b2c3d4e5f60001", "", KubeHealth::Ok, &[]);
        let alerting =
            vec![parse_group_key("kube_deployment:order-service,env:prod"), parse_group_key("env:prod"), parse_group_key("kube_deployment:billing"), parse_group_key("host:i-0a1b2c3d4e5f60001")];
        assert_eq!(alerts_on(&r, &alerting), 1, "only groups that name the resource count");
        assert_eq!(alerts_on(&host, &alerting), 1);
        let mut n = node(&r);
        apply_health(&mut n, &r, &[], &HashMap::from([("order-service".to_string(), 0.02)]));
        assert_eq!((n.health, n.error_rate), (CloudMapHealth::Warning, Some(0.02)));
        let mut n = node(&r);
        apply_health(&mut n, &r, &alerting, &HashMap::new());
        assert_eq!((n.health, n.alerts), (CloudMapHealth::Error, 1));
    }
}
