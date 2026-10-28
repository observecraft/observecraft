//! Infrastructure → Catalog: one searchable list of the org's infrastructure resources across types,
//! built only from what the other Infrastructure pages already read:
//! - hosts: host metrics plus Datadog Agent host metadata ([`super::hosts`]),
//! - Kubernetes and ECS resources: the collectors' object snapshots and ECS task metadata
//!   ([`super::kubernetes`]), current state only.
//!
//! Like hosts and the Kubernetes Explorer, an org's inventory is small enough to filter, facet and
//! sort in memory.

use std::collections::{BTreeMap, HashSet};

use axum::extract::State;
use axum::Json;
use oc_query::{glob, parse_query, short_tag, Term};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use super::hosts::{enum_str, load_hosts, Cloud, HostRow, HIDDEN_TAGS};
use super::kubernetes::{KubeHealth, KubeKind, KubeResource};
use super::logs::TimeRange;
use crate::error::ApiResult;
use crate::state::{AppState, OrgId};

/// The resource types of the catalog.
#[derive(Serialize, Deserialize, ToSchema, Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum CatalogType {
    #[serde(rename = "host")]
    Host,
    #[serde(rename = "kube_cluster")]
    KubeCluster,
    #[serde(rename = "kube_node")]
    KubeNode,
    #[serde(rename = "kube_namespace")]
    KubeNamespace,
    #[serde(rename = "kube_deployment")]
    KubeDeployment,
    #[serde(rename = "kube_statefulset")]
    KubeStatefulSet,
    #[serde(rename = "kube_daemonset")]
    KubeDaemonSet,
    #[serde(rename = "kube_cronjob")]
    KubeCronJob,
    #[serde(rename = "kube_job")]
    KubeJob,
    #[serde(rename = "kube_pod")]
    KubePod,
    #[serde(rename = "kube_service")]
    KubeService,
    #[serde(rename = "kube_ingress")]
    KubeIngress,
    #[serde(rename = "kube_persistent_volume_claim")]
    KubePersistentVolumeClaim,
    #[serde(rename = "kube_persistent_volume")]
    KubePersistentVolume,
    #[serde(rename = "container")]
    Container,
    #[serde(rename = "ecs_cluster")]
    EcsCluster,
    #[serde(rename = "ecs_service")]
    EcsService,
    #[serde(rename = "ecs_task")]
    EcsTask,
}

/// Broad groups of resource types (the catalog's default grouping).
#[derive(Serialize, Deserialize, ToSchema, Clone, Copy, PartialEq, Eq, Hash, Debug)]
#[serde(rename_all = "snake_case")]
pub enum CatalogCategory {
    Compute,
    Containers,
    Kubernetes,
    Ecs,
}

impl CatalogCategory {
    fn as_str(self) -> &'static str {
        match self {
            CatalogCategory::Compute => "compute",
            CatalogCategory::Containers => "containers",
            CatalogCategory::Kubernetes => "kubernetes",
            CatalogCategory::Ecs => "ecs",
        }
    }
}

impl CatalogType {
    fn category(self) -> CatalogCategory {
        match self {
            CatalogType::Host => CatalogCategory::Compute,
            CatalogType::KubePod | CatalogType::Container => CatalogCategory::Containers,
            CatalogType::EcsCluster | CatalogType::EcsService | CatalogType::EcsTask => CatalogCategory::Ecs,
            _ => CatalogCategory::Kubernetes,
        }
    }

    /// The catalog type of a Kubernetes Explorer kind; None for kinds the catalog leaves out
    /// (replica sets, RBAC, processes, …).
    fn of_kube(kind: KubeKind) -> Option<CatalogType> {
        Some(match kind {
            KubeKind::Clusters => CatalogType::KubeCluster,
            KubeKind::Nodes => CatalogType::KubeNode,
            KubeKind::Namespaces => CatalogType::KubeNamespace,
            KubeKind::Deployments => CatalogType::KubeDeployment,
            KubeKind::StatefulSets => CatalogType::KubeStatefulSet,
            KubeKind::DaemonSets => CatalogType::KubeDaemonSet,
            KubeKind::CronJobs => CatalogType::KubeCronJob,
            KubeKind::Jobs => CatalogType::KubeJob,
            KubeKind::Pods => CatalogType::KubePod,
            KubeKind::Services => CatalogType::KubeService,
            KubeKind::Ingresses => CatalogType::KubeIngress,
            KubeKind::PersistentVolumeClaims => CatalogType::KubePersistentVolumeClaim,
            KubeKind::PersistentVolumes => CatalogType::KubePersistentVolume,
            KubeKind::Containers => CatalogType::Container,
            KubeKind::EcsClusters => CatalogType::EcsCluster,
            KubeKind::EcsServices => CatalogType::EcsService,
            KubeKind::EcsTasks => CatalogType::EcsTask,
            _ => return None,
        })
    }
}

/// One row of the catalog.
#[derive(Serialize, ToSchema, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct CatalogResource {
    /// `host:<name>` for hosts, the Kubernetes Explorer uid otherwise.
    pub id: String,
    pub resource_type: CatalogType,
    pub resource_category: CatalogCategory,
    pub name: String,
    pub cloud: Option<String>,
    pub account: Option<String>,
    pub region: Option<String>,
    /// Kubernetes or ECS cluster.
    pub cluster: Option<String>,
    pub namespace: Option<String>,
    pub env: Option<String>,
    pub service: Option<String>,
    pub team: Option<String>,
    pub status: Option<String>,
    pub health: Option<KubeHealth>,
    /// Epoch ms.
    pub created_at: Option<i64>,
    pub last_seen: i64,
    /// `key:value` tags, as on the resource's own page.
    pub tags: Vec<String>,
    /// Type-specific properties for the side panel (OS, node, images, owner, …).
    pub attributes: BTreeMap<String, String>,
    /// Kubernetes Explorer kind, to open the resource there (null for hosts).
    pub kube_kind: Option<KubeKind>,
}

/// Facets of the catalog, in display order.
const FACETS: &[&str] = &["resource_category", "resource_type", "cloud", "account", "region", "cluster", "namespace", "env", "service", "team"];
const MAX_FACET_VALUES: usize = 50;

impl CatalogResource {
    /// The resource's values for a query or facet key (a tag key may have several).
    fn values(&self, key: &str) -> Vec<&str> {
        fn one(v: &Option<String>) -> Vec<&str> {
            v.as_deref().into_iter().collect()
        }
        match key {
            "resource_type" | "type" => vec![enum_str_static(self.resource_type)],
            "resource_category" | "category" => vec![self.resource_category.as_str()],
            "name" => vec![self.name.as_str()],
            "cloud" => one(&self.cloud),
            "account" => one(&self.account),
            "region" => one(&self.region),
            "cluster" | "kube_cluster_name" => one(&self.cluster),
            "namespace" | "kube_namespace" => one(&self.namespace),
            "env" => one(&self.env),
            "service" => one(&self.service),
            "team" => one(&self.team),
            "status" => one(&self.status),
            _ => {
                let prefix = format!("{key}:");
                self.tags.iter().filter_map(|t| t.strip_prefix(&prefix)).collect()
            }
        }
    }
}

/// Matching against parsed query terms: tags (with `*` wildcards) and free text on the name.
pub(crate) trait Faceted {
    fn facet_values(&self, key: &str) -> Vec<&str>;
    /// Free text is matched against this.
    fn text(&self) -> String;

    fn matches(&self, terms: &[Term]) -> bool {
        terms.iter().all(|t| match t {
            Term::Text { text, negate } => self.text().to_lowercase().contains(&text.to_lowercase()) != *negate,
            Term::Tag { key, values, negate } => self.facet_values(key).iter().any(|v| values.iter().any(|p| glob(p, v))) != *negate,
        })
    }
}

impl Faceted for CatalogResource {
    fn facet_values(&self, key: &str) -> Vec<&str> {
        self.values(key)
    }
    fn text(&self) -> String {
        self.name.clone()
    }
}

#[derive(Serialize, ToSchema, Clone, Debug, PartialEq)]
pub struct CatalogFacetValue {
    pub value: String,
    pub count: usize,
}

#[derive(Serialize, ToSchema, Clone, Debug)]
pub struct CatalogFacet {
    pub key: String,
    pub values: Vec<CatalogFacetValue>,
}

/// Counts of each facet's values; a facet's counts honour every filter but its own, so its other
/// values stay visible.
pub(crate) fn facets<T: Faceted>(items: &[T], keys: &[&str], terms: &[Term]) -> Vec<CatalogFacet> {
    keys.iter()
        .map(|key| {
            let others: Vec<Term> = terms.iter().filter(|t| !matches!(t, Term::Tag { key: k, .. } if k == key)).cloned().collect();
            let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
            for item in items.iter().filter(|i| i.matches(&others)) {
                for v in item.facet_values(key).into_iter().collect::<HashSet<_>>() {
                    *counts.entry(v).or_default() += 1;
                }
            }
            let mut values: Vec<CatalogFacetValue> = counts.into_iter().map(|(value, count)| CatalogFacetValue { value: value.to_string(), count }).collect();
            values.sort_by(|a, b| b.count.cmp(&a.count).then(a.value.cmp(&b.value)));
            values.truncate(MAX_FACET_VALUES);
            CatalogFacet { key: key.to_string(), values }
        })
        .filter(|f| !f.values.is_empty())
        .collect()
}

fn enum_str_static(t: CatalogType) -> &'static str {
    match t {
        CatalogType::Host => "host",
        CatalogType::KubeCluster => "kube_cluster",
        CatalogType::KubeNode => "kube_node",
        CatalogType::KubeNamespace => "kube_namespace",
        CatalogType::KubeDeployment => "kube_deployment",
        CatalogType::KubeStatefulSet => "kube_statefulset",
        CatalogType::KubeDaemonSet => "kube_daemonset",
        CatalogType::KubeCronJob => "kube_cronjob",
        CatalogType::KubeJob => "kube_job",
        CatalogType::KubePod => "kube_pod",
        CatalogType::KubeService => "kube_service",
        CatalogType::KubeIngress => "kube_ingress",
        CatalogType::KubePersistentVolumeClaim => "kube_persistent_volume_claim",
        CatalogType::KubePersistentVolume => "kube_persistent_volume",
        CatalogType::Container => "container",
        CatalogType::EcsCluster => "ecs_cluster",
        CatalogType::EcsService => "ecs_service",
        CatalogType::EcsTask => "ecs_task",
    }
}

fn non_empty(v: Option<&String>) -> Option<String> {
    v.filter(|v| !v.is_empty()).cloned()
}

/// A host as a catalog row.
fn from_host(h: &HostRow) -> CatalogResource {
    let s = &h.summary;
    let res = |k: &str| non_empty(h.res.get(k));
    let mut tags: Vec<String> = h.res.iter().filter(|(k, v)| !v.is_empty() && !HIDDEN_TAGS.contains(&k.as_str())).map(|(k, v)| format!("{}:{v}", short_tag(k).unwrap_or(k))).collect();
    tags.sort();
    let mut attributes = BTreeMap::from([
        ("os".to_string(), enum_str(&s.os)),
        ("collector".to_string(), s.collectors.iter().map(enum_str).collect::<Vec<_>>().join(", ")),
        ("cpu_pct".to_string(), s.cpu_pct.to_string()),
        ("mem_pct".to_string(), s.mem_pct.to_string()),
    ]);
    if let Some(d) = &s.distro {
        attributes.insert("distro".into(), d.clone());
    }
    if !s.apps.is_empty() {
        attributes.insert("apps".into(), s.apps.join(", "));
    }
    for (key, attr) in [("instance_type", "host.type"), ("availability_zone", "cloud.availability_zone"), ("arch", "host.arch")] {
        if let Some(v) = res(attr) {
            attributes.insert(key.into(), v);
        }
    }
    CatalogResource {
        id: format!("host:{}", s.name),
        resource_type: CatalogType::Host,
        resource_category: CatalogCategory::Compute,
        name: s.name.clone(),
        cloud: (s.cloud != Cloud::None).then(|| enum_str(&s.cloud)),
        account: res("cloud.account.id"),
        region: res("cloud.region"),
        cluster: res("k8s.cluster.name"),
        namespace: None,
        env: s.env.clone(),
        service: s.service.clone(),
        team: s.team.clone(),
        status: Some("Active".into()),
        health: Some(KubeHealth::Ok),
        created_at: None,
        last_seen: h.last_seen,
        tags,
        attributes,
        kube_kind: None,
    }
}

/// A Kubernetes or ECS resource as a catalog row; None for kinds the catalog leaves out.
fn from_kube(r: &KubeResource) -> Option<CatalogResource> {
    let kind = r.kind?;
    let resource_type = CatalogType::of_kube(kind)?;
    let tag = |k: &str| {
        let prefix = format!("{k}:");
        r.tags.iter().find_map(|t| t.strip_prefix(&prefix)).filter(|v| !v.is_empty()).map(str::to_string)
    };
    let label = |k: &str| non_empty(r.labels.get(k));
    let ecs = matches!(resource_type, CatalogType::EcsCluster | CatalogType::EcsService | CatalogType::EcsTask) || tag("ecs_cluster").is_some();
    let mut attributes = BTreeMap::new();
    let mut put = |k: &str, v: Option<String>| {
        if let Some(v) = v.filter(|v| !v.is_empty()) {
            attributes.insert(k.to_string(), v);
        }
    };
    put("node", r.node.clone());
    put("ready", r.ready.clone());
    put("owner", r.owner.as_ref().map(|o| format!("{}/{}", o.kind, o.name)));
    put("images", (!r.images.is_empty()).then(|| r.images.join(", ")));
    put("version", r.version.clone());
    put("instance_type", r.instance_type.clone());
    put("availability_zone", r.zone.clone().or_else(|| tag("availability_zone")));
    put("ip", r.ip.clone());
    put("service_type", r.service_type.clone());
    put("task_definition", r.task_definition.clone());
    put("launch_type", r.launch_type.clone());
    put("nodes", r.nodes.map(|n| n.to_string()));
    put("pods", r.pods.map(|n| n.to_string()));
    put("restarts", r.restarts.map(|n| n.to_string()));
    Some(CatalogResource {
        id: r.uid.clone(),
        resource_type,
        resource_category: resource_type.category(),
        name: r.name.clone(),
        cloud: if ecs { Some("aws".into()) } else { None },
        account: tag("aws_account"),
        region: tag("region").or_else(|| label("topology.kubernetes.io/region")),
        cluster: Some(r.cluster.clone()).filter(|c| !c.is_empty()),
        namespace: r.namespace.clone(),
        env: tag("env"),
        service: tag("service"),
        team: tag("team"),
        status: Some(r.status.clone()).filter(|s| !s.is_empty()),
        health: Some(r.health),
        created_at: r.created_at,
        last_seen: r.last_seen,
        tags: r.tags.clone(),
        attributes,
        kube_kind: Some(kind),
    })
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct CatalogSearchRequest {
    /// Hosts are those that reported in this range; Kubernetes and ECS resources are current.
    pub range: TimeRange,
    /// `resource_type:kube_pod env:prod cluster:shop-* team:payments` + free text on the name.
    #[serde(default)]
    pub query: String,
    /// Group the results by a key (`resource_category`, `resource_type`, `region`, `env`, a tag key, …).
    pub group_by: Option<String>,
    /// name (default), type, cluster, region or last_seen.
    pub sort: Option<String>,
    #[serde(default)]
    pub desc: bool,
    #[serde(default)]
    pub offset: usize,
    pub limit: usize,
}

#[derive(Serialize, ToSchema)]
pub struct CatalogPage {
    pub items: Vec<CatalogResource>,
    pub total: usize,
    pub facets: Vec<CatalogFacet>,
    /// When grouped: every group of the matching resources, largest first.
    pub groups: Option<Vec<CatalogGroup>>,
}

/// One group of a grouped search, with the distinct regions, envs, teams and services of its members.
#[derive(Serialize, ToSchema, Debug)]
pub struct CatalogGroup {
    pub key: String,
    pub count: usize,
    pub regions: Vec<String>,
    pub envs: Vec<String>,
    pub teams: Vec<String>,
    pub services: Vec<String>,
}

/// Distinct values shown per group column.
const MAX_GROUP_VALUES: usize = 10;

fn groups(rows: &[CatalogResource], key: &str) -> Vec<CatalogGroup> {
    use std::collections::BTreeSet;
    let mut by: BTreeMap<String, (usize, [BTreeSet<&str>; 4])> = BTreeMap::new();
    for r in rows {
        let name = r.values(key).first().map(|v| v.to_string()).unwrap_or_else(|| format!("no {key}"));
        let g = by.entry(name).or_default();
        g.0 += 1;
        for (set, field) in g.1.iter_mut().zip([&r.region, &r.env, &r.team, &r.service]) {
            set.extend(field.as_deref());
        }
    }
    let list = |s: BTreeSet<&str>| s.into_iter().take(MAX_GROUP_VALUES).map(str::to_string).collect();
    let mut out: Vec<CatalogGroup> = by
        .into_iter()
        .map(|(key, (count, [regions, envs, teams, services]))| CatalogGroup { key, count, regions: list(regions), envs: list(envs), teams: list(teams), services: list(services) })
        .collect();
    out.sort_by(|a, b| b.count.cmp(&a.count).then(a.key.cmp(&b.key)));
    out
}

fn sort_key(r: &CatalogResource, field: &str) -> (String, i64) {
    match field {
        "type" => (enum_str_static(r.resource_type).to_string(), 0),
        "cluster" => (r.cluster.clone().unwrap_or_default(), 0),
        "region" => (r.region.clone().unwrap_or_default(), 0),
        "last_seen" => (String::new(), r.last_seen),
        _ => (r.name.to_lowercase(), 0),
    }
}

/// Filters, facets, sorts and pages the catalog.
fn search_page(all: Vec<CatalogResource>, req: &CatalogSearchRequest) -> CatalogPage {
    let terms = parse_query(&req.query);
    let facets = facets(&all, FACETS, &terms);
    let mut rows: Vec<CatalogResource> = all.into_iter().filter(|r| r.matches(&terms)).collect();
    let field = req.sort.as_deref().unwrap_or("name");
    rows.sort_by(|a, b| {
        let o = sort_key(a, field).cmp(&sort_key(b, field));
        let o = if req.desc { o.reverse() } else { o };
        o.then_with(|| a.name.cmp(&b.name)).then_with(|| a.id.cmp(&b.id))
    });
    let total = rows.len();
    let groups = req.group_by.as_deref().filter(|g| !g.is_empty()).map(|g| groups(&rows, g));
    let items = rows.into_iter().skip(req.offset).take(req.limit.clamp(1, 500)).collect();
    CatalogPage { items, total, facets, groups }
}

#[utoipa::path(post, path = "/api/v1/infrastructure/catalog", operation_id = "search_catalog", request_body = CatalogSearchRequest, responses((status = 200, body = CatalogPage)), tag = "infrastructure")]
pub async fn search(State(st): State<AppState>, OrgId(org): OrgId, Json(req): Json<CatalogSearchRequest>) -> ApiResult<CatalogPage> {
    req.range.validate()?;
    let (hosts, kube) = tokio::try_join!(load_hosts(&st, &org, req.range), super::kubernetes::resources(&st, &org))?;
    let all: Vec<CatalogResource> = hosts.iter().map(from_host).chain(kube.iter().filter_map(from_kube)).collect();
    Ok(Json(search_page(all, &req)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kube(kind: KubeKind, name: &str, tags: &[&str]) -> KubeResource {
        KubeResource {
            uid: format!("uid-{name}"),
            kind: Some(kind),
            cluster: "shop-prod".into(),
            name: name.into(),
            last_seen: 1_000,
            status: "Running".into(),
            tags: tags.iter().map(|t| t.to_string()).collect(),
            ..Default::default()
        }
    }

    fn req(query: &str) -> CatalogSearchRequest {
        CatalogSearchRequest { range: TimeRange { from: 0, to: 1 }, query: query.into(), group_by: None, sort: None, desc: false, offset: 0, limit: 50 }
    }

    #[test]
    fn kubernetes_resources_take_unified_service_tags_and_skip_left_out_kinds() {
        let mut pod = kube(KubeKind::Pods, "order-service-7d9f-abcde", &["kube_cluster_name:shop-prod", "env:prod", "service:order-service", "team:orders"]);
        pod.namespace = Some("shop".into());
        pod.node = Some("i-0a1b2c3d4e5f60001".into());
        let row = from_kube(&pod).unwrap();
        assert_eq!(row.resource_type, CatalogType::KubePod);
        assert_eq!((row.env.as_deref(), row.service.as_deref(), row.team.as_deref()), (Some("prod"), Some("order-service"), Some("orders")));
        assert_eq!(row.cluster.as_deref(), Some("shop-prod"));
        assert_eq!(row.namespace.as_deref(), Some("shop"));
        assert_eq!(row.attributes.get("node").map(String::as_str), Some("i-0a1b2c3d4e5f60001"));
        assert_eq!(row.kube_kind, Some(KubeKind::Pods));
        assert!(row.cloud.is_none());
        // Replica sets and processes are not catalog resources.
        assert!(from_kube(&kube(KubeKind::ReplicaSets, "order-service-7d9f", &[])).is_none());
        assert!(from_kube(&kube(KubeKind::Processes, "nginx", &[])).is_none());
    }

    #[test]
    fn ecs_resources_are_aws_with_their_account_and_region() {
        let task = kube(KubeKind::EcsTasks, "3f2a9c", &["ecs_cluster:storefront", "region:us-east-1", "aws_account:123456789012"]);
        let row = from_kube(&task).unwrap();
        assert_eq!(row.resource_type, CatalogType::EcsTask);
        assert_eq!((row.cloud.as_deref(), row.account.as_deref(), row.region.as_deref()), (Some("aws"), Some("123456789012"), Some("us-east-1")));
    }

    #[test]
    fn search_filters_by_type_and_tags_with_facet_counts_ignoring_their_own_filter() {
        let all: Vec<CatalogResource> = [
            kube(KubeKind::Pods, "billing-1", &["env:prod", "service:billing"]),
            kube(KubeKind::Pods, "catalog-service-1", &["env:staging", "service:catalog-service"]),
            kube(KubeKind::Deployments, "billing", &["env:prod", "service:billing"]),
            kube(KubeKind::Nodes, "i-0a1b2c3d4e5f60002", &[]),
        ]
        .iter()
        .filter_map(from_kube)
        .collect();
        let page = search_page(all.clone(), &req("resource_type:kube_pod"));
        assert_eq!(page.total, 2);
        // The type facet still lists every type; the env facet only counts pods.
        let facet = |key: &str| page.facets.iter().find(|f| f.key == key).map(|f| f.values.clone()).unwrap_or_default();
        assert_eq!(facet("resource_type").len(), 3);
        assert_eq!(facet("env"), vec![CatalogFacetValue { value: "prod".into(), count: 1 }, CatalogFacetValue { value: "staging".into(), count: 1 }]);
        assert_eq!(facet("cluster"), vec![CatalogFacetValue { value: "shop-prod".into(), count: 2 }]);

        let page = search_page(all.clone(), &req("env:prod -resource_type:kube_deployment"));
        assert_eq!(page.items.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(), vec!["billing-1"]);
        // Free text matches the name; wildcards match tag values.
        assert_eq!(search_page(all.clone(), &req("catalog")).total, 1);
        assert_eq!(search_page(all, &req("service:bill*")).total, 2);
    }

    #[test]
    fn type_names_in_queries_are_the_serialized_names() {
        for kind in [KubeKind::Clusters, KubeKind::StatefulSets, KubeKind::PersistentVolumeClaims, KubeKind::Containers, KubeKind::EcsServices] {
            let t = CatalogType::of_kube(kind).unwrap();
            assert_eq!(enum_str(&t), enum_str_static(t));
        }
        assert_eq!(enum_str_static(CatalogType::Host), enum_str(&CatalogType::Host));
    }

    #[test]
    fn resource_types_fall_into_categories() {
        assert_eq!(CatalogType::Host.category(), CatalogCategory::Compute);
        assert_eq!(CatalogType::KubePod.category(), CatalogCategory::Containers);
        assert_eq!(CatalogType::Container.category(), CatalogCategory::Containers);
        assert_eq!(CatalogType::KubeDeployment.category(), CatalogCategory::Kubernetes);
        assert_eq!(CatalogType::EcsTask.category(), CatalogCategory::Ecs);
        let row = from_kube(&kube(KubeKind::Nodes, "n", &[])).unwrap();
        assert_eq!(row.resource_category, CatalogCategory::Kubernetes);
        assert_eq!(row.values("resource_category"), vec!["kubernetes"]);
    }

    #[test]
    fn grouping_counts_each_group_with_its_regions_envs_teams_and_services() {
        let all: Vec<CatalogResource> = [
            kube(KubeKind::Pods, "billing-1", &["env:prod", "service:billing", "region:eu-west-1"]),
            kube(KubeKind::Pods, "billing-2", &["env:staging", "service:billing", "team:payments"]),
            kube(KubeKind::Deployments, "billing", &["env:prod", "service:billing"]),
        ]
        .iter()
        .filter_map(from_kube)
        .collect();
        let mut r = req("");
        r.group_by = Some("resource_category".into());
        let page = search_page(all, &r);
        let groups = page.groups.unwrap();
        assert_eq!(groups.iter().map(|g| (g.key.as_str(), g.count)).collect::<Vec<_>>(), vec![("containers", 2), ("kubernetes", 1)]);
        assert_eq!(groups[0].envs, vec!["prod", "staging"]);
        assert_eq!(groups[0].regions, vec!["eu-west-1"]);
        assert_eq!(groups[0].teams, vec!["payments"]);
        assert_eq!(groups[0].services, vec!["billing"]);
        // Ungrouped searches have no groups.
        assert!(search_page(vec![], &req("")).groups.is_none());
    }

    #[test]
    fn search_sorts_and_pages() {
        let all: Vec<CatalogResource> = ["b", "c", "a"].iter().filter_map(|n| from_kube(&kube(KubeKind::Pods, n, &[]))).collect();
        let mut r = req("");
        r.desc = true;
        r.limit = 2;
        let page = search_page(all, &r);
        assert_eq!(page.total, 3);
        assert_eq!(page.items.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(), vec!["c", "b"]);
    }
}
