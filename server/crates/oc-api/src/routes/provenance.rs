//! Where a resource comes from (docs/adr/0008-remediation-and-provenance.md, decision 1): the repository, file and
//! revision that define a cloud resource, a Kubernetes object, a container image or a service, read only from the
//! conventions the customer's tooling follows. No provenance is a valid answer.
//!
//! - **Cloud resources** (the cloud account scans' resources, `posture_findings`, with their tags): our `tf_repo` /
//!   `tf_path` tags (AWS provider `default_tags`, Azure `locals` merged into `tags`), or Yor's `git_org` / `git_repo` /
//!   `git_file` / `git_commit` / `yor_name` / `yor_trace`; then a Terraform state index ([`parse_state`]) when the org
//!   connected a read-only state source (`terraform_state_sources`; reading the state is not built yet). A
//!   `managed-by` tag alone names no repository.
//! - **Kubernetes objects** (the inventory from the orchestrator manifests): Argo CD's tracking annotation
//!   `argocd.argoproj.io/tracking-id` (`<app>:<group>/<kind>:<namespace>/<name>`), else its tracking label
//!   (`app.kubernetes.io/instance`, or `argocd.argoproj.io/instance`); the Application's `spec.source` gives the
//!   repository, path and revision when its object is collected (the Cluster Agent's custom resource collection of
//!   `argoproj.io/v1alpha1/applications`), else only the Application's name is known.
//! - **Images**: the OCI labels `org.opencontainers.image.source` / `.revision`, or the `git.repository_url` /
//!   `git.commit.sha` tags the Datadog Agent derives from them, when the image's tags carry them.
//! - **Services**: the tracers' git metadata (`DD_GIT_REPOSITORY_URL` / `DD_GIT_COMMIT_SHA`, sent as
//!   `_dd.git.repository_url` / `_dd.git.commit.sha` span tags) or OpenTelemetry's `git.*` / `vcs.*` attributes.
//!
//! Resolvers are pure functions; [`inventory`] loads the org's resources with their tags and provenance (Postgres rows
//! on `org_id`, ClickHouse on `OrgId = {oc_org:String}` through `AppState::ch(org)`).

use std::collections::{BTreeMap, HashMap};

use axum::extract::{Query, State};
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::{IntoParams, ToSchema};

use super::code_security_model::normalize_repo;
use super::kubernetes::{KubeKind, KubeResource};
use crate::auth::AuthContext;
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;

/// What defines the resource.
#[derive(Serialize, Deserialize, ToSchema, Clone, Copy, PartialEq, Eq, Debug, Hash)]
#[serde(rename_all = "snake_case")]
pub enum ProvenanceKind {
    Terraform,
    Helm,
    Kustomize,
    Manifest,
    Dockerfile,
    Code,
}

/// Exact: the resource names its source itself. Inferred: a likely match that needs a confirmation.
#[derive(Serialize, Deserialize, ToSchema, Clone, Copy, PartialEq, Eq, Debug, Hash)]
#[serde(rename_all = "snake_case")]
pub enum Confidence {
    Exact,
    Inferred,
}

/// How the source was found.
#[derive(Serialize, Deserialize, ToSchema, Clone, Copy, PartialEq, Eq, Debug, Hash)]
#[serde(rename_all = "snake_case")]
pub enum ProvenanceVia {
    Tag,
    State,
    Argocd,
    Oci,
    GitMetadata,
}

impl ProvenanceVia {
    pub fn as_str(self) -> &'static str {
        match self {
            ProvenanceVia::Tag => "tag",
            ProvenanceVia::State => "state",
            ProvenanceVia::Argocd => "argocd",
            ProvenanceVia::Oci => "oci",
            ProvenanceVia::GitMetadata => "git_metadata",
        }
    }
}

/// Where a resource comes from.
#[derive(Serialize, Deserialize, ToSchema, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Provenance {
    /// Repository without scheme or `.git` (`github.com/example-org/infra-terraform`); null when only the Argo CD
    /// Application is known (its object isn't collected).
    pub repo: Option<String>,
    /// File or directory in the repository (`envs/prod/storage`, `charts/order-service`).
    pub path: Option<String>,
    /// Branch, tag or commit.
    #[serde(rename = "ref")]
    pub git_ref: Option<String>,
    /// The resource in that file: a Terraform address (`module.storage.aws_s3_bucket.assets`) or a Kubernetes object
    /// (`Deployment/checkout/order-service`).
    pub address: Option<String>,
    /// The Argo CD Application that deploys a Kubernetes object.
    pub application: Option<String>,
    pub kind: ProvenanceKind,
    pub confidence: Confidence,
    pub via: ProvenanceVia,
}

impl Provenance {
    fn new(kind: ProvenanceKind, confidence: Confidence, via: ProvenanceVia) -> Provenance {
        Provenance { repo: None, path: None, git_ref: None, address: None, application: None, kind, confidence, via }
    }
}

// ================================================================ tags

/// Tag keys compare without case, `-` and `_` alike (`Managed-By` = `managed_by`).
pub fn norm_key(k: &str) -> String {
    k.trim().to_ascii_lowercase().replace('-', "_")
}

/// Every value of `key` among `key:value` tags.
pub fn tag_values<'a>(tags: &'a [String], key: &str) -> Vec<&'a str> {
    let want = norm_key(key);
    tags.iter().filter_map(|t| t.split_once(':')).filter(|(k, _)| norm_key(k) == want).map(|(_, v)| v.trim()).filter(|v| !v.is_empty()).collect()
}

fn first_tag(tags: &[String], keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|k| tag_values(tags, k).first().map(|v| v.to_string()))
}

fn non_empty(v: Option<&str>) -> Option<String> {
    v.map(str::trim).filter(|v| !v.is_empty()).map(str::to_string)
}

/// The kind of file a path names.
fn kind_of_path(path: &str) -> ProvenanceKind {
    let p = path.to_ascii_lowercase();
    if p.ends_with(".tf") || p.ends_with(".tf.json") || !p.contains('.') {
        ProvenanceKind::Terraform
    } else if p.ends_with("dockerfile") || p.contains("dockerfile.") {
        ProvenanceKind::Dockerfile
    } else {
        ProvenanceKind::Manifest
    }
}

/// A cloud resource's source from its tags: `tf_repo` / `tf_path`, or Yor's `git_*` tags.
pub fn from_tags(tags: &[String]) -> Option<Provenance> {
    if let Some(repo) = first_tag(tags, &["tf_repo"]) {
        let mut p = Provenance::new(ProvenanceKind::Terraform, Confidence::Exact, ProvenanceVia::Tag);
        p.repo = Some(normalize_repo(&repo));
        p.path = first_tag(tags, &["tf_path"]);
        return Some(p);
    }
    let repo = first_tag(tags, &["git_repo"])?;
    // Yor tags the repository's name and its owner apart, without the host.
    let repo = match first_tag(tags, &["git_org"]) {
        Some(org) if !repo.contains('/') => format!("{org}/{repo}"),
        _ => repo,
    };
    let path = first_tag(tags, &["git_file"]);
    let mut p = Provenance::new(path.as_deref().map(kind_of_path).unwrap_or(ProvenanceKind::Terraform), Confidence::Exact, ProvenanceVia::Tag);
    p.repo = Some(normalize_repo(&repo));
    p.path = path;
    p.git_ref = first_tag(tags, &["git_commit"]);
    p.address = first_tag(tags, &["yor_name"]);
    Some(p)
}

// ================================================================ Terraform state

/// Cloud ids → their Terraform resource, from a state file (format version 4) kept in `repo` at `path`. Ids are the
/// ARNs, Azure resource ids and Google self links of the managed resources, lowercased. Reading the org's state
/// sources (`terraform_state_sources`) to feed it is not built yet, so only tests call it today.
#[allow(dead_code)]
pub fn parse_state(state: &Value, repo: &str, path: &str) -> HashMap<String, Provenance> {
    let mut out = HashMap::new();
    for r in state.get("resources").and_then(Value::as_array).into_iter().flatten() {
        if r.get("mode").and_then(Value::as_str) != Some("managed") {
            continue;
        }
        let (Some(t), Some(name)) = (r.get("type").and_then(Value::as_str), r.get("name").and_then(Value::as_str)) else { continue };
        let base = match r.get("module").and_then(Value::as_str).filter(|m| !m.is_empty()) {
            Some(m) => format!("{m}.{t}.{name}"),
            None => format!("{t}.{name}"),
        };
        for i in r.get("instances").and_then(Value::as_array).into_iter().flatten() {
            let address = match i.get("index_key") {
                Some(Value::String(k)) => format!("{base}[\"{k}\"]"),
                Some(Value::Number(n)) => format!("{base}[{n}]"),
                _ => base.clone(),
            };
            let attrs = i.get("attributes");
            let id = attrs.and_then(|a| a.get("id")).and_then(Value::as_str).filter(|id| id.starts_with("/subscriptions/") || id.starts_with("projects/"));
            let ids = ["arn", "self_link"].iter().filter_map(|k| attrs.and_then(|a| a.get(*k)).and_then(Value::as_str)).chain(id);
            for id in ids.filter(|id| !id.is_empty()) {
                let mut p = Provenance::new(ProvenanceKind::Terraform, Confidence::Exact, ProvenanceVia::State);
                p.repo = Some(normalize_repo(repo));
                p.path = non_empty(Some(path));
                p.address = Some(address.clone());
                out.insert(id.to_ascii_lowercase(), p);
            }
        }
    }
    out
}

/// A cloud resource's source: its tags first, then the state index.
pub fn resolve_cloud(id: &str, tags: &[String], state: &HashMap<String, Provenance>) -> Option<Provenance> {
    from_tags(tags).or_else(|| state.get(&id.to_ascii_lowercase()).cloned())
}

// ================================================================ Argo CD

pub const TRACKING_ANNOTATION: &str = "argocd.argoproj.io/tracking-id";
/// Argo CD's label tracking: its default label, and the one older setups configure.
const TRACKING_LABELS: [&str; 2] = ["app.kubernetes.io/instance", "argocd.argoproj.io/instance"];

/// Argo CD's tracking id, `<app>:<group>/<kind>:<namespace>/<name>` (`<app>` is `<namespace>_<name>` for Applications
/// outside Argo CD's own namespace; the group is empty for core kinds and the namespace for cluster-scoped objects).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrackingId {
    pub app: String,
    pub app_namespace: Option<String>,
    pub group: String,
    pub kind: String,
    pub namespace: String,
    pub name: String,
}

pub fn parse_tracking_id(s: &str) -> Option<TrackingId> {
    let (app, rest) = s.trim().split_once(':')?;
    let (group_kind, ns_name) = rest.split_once(':')?;
    let (group, kind) = group_kind.rsplit_once('/')?;
    let (namespace, name) = ns_name.split_once('/')?;
    if app.is_empty() || kind.is_empty() || name.is_empty() {
        return None;
    }
    let (app_namespace, app) = match app.split_once('_') {
        Some((ns, a)) if !ns.is_empty() && !a.is_empty() => (Some(ns.to_string()), a.to_string()),
        _ => (None, app.to_string()),
    };
    Some(TrackingId { app, app_namespace, group: group.into(), kind: kind.into(), namespace: namespace.into(), name: name.into() })
}

/// An Argo CD Application object (its `spec` and `status`).
#[derive(Clone, Debug, Default)]
pub struct ArgoApp {
    pub cluster: String,
    pub namespace: String,
    pub name: String,
    pub doc: Value,
}

impl ArgoApp {
    /// An Application of Argo CD, not another `Application` kind: it has a source.
    pub fn is_argo(doc: &Value) -> bool {
        doc.pointer("/spec/source").is_some() || doc.pointer("/spec/sources").is_some_and(Value::is_array)
    }

    /// The source the objects come from: `spec.source`, or of several sources the first with a path or a chart.
    fn source(&self) -> Option<&Value> {
        self.doc.pointer("/spec/source").filter(|s| s.is_object()).or_else(|| {
            let sources = self.doc.pointer("/spec/sources")?.as_array()?;
            sources.iter().find(|s| s.get("path").is_some() || s.get("chart").is_some()).or(sources.first())
        })
    }

    /// The source's repository, path, revision and tool.
    fn provenance(&self) -> Option<Provenance> {
        let s = self.source()?;
        let text = |k: &str| non_empty(s.get(k).and_then(Value::as_str));
        let repo = text("repoURL")?;
        let source_type = self.doc.pointer("/status/sourceType").and_then(Value::as_str).unwrap_or("");
        let kind = match source_type {
            "Helm" => ProvenanceKind::Helm,
            "Kustomize" => ProvenanceKind::Kustomize,
            "Directory" => ProvenanceKind::Manifest,
            _ if s.get("chart").is_some() || s.get("helm").is_some() => ProvenanceKind::Helm,
            _ if s.get("kustomize").is_some() => ProvenanceKind::Kustomize,
            _ => ProvenanceKind::Manifest,
        };
        let mut p = Provenance::new(kind, Confidence::Exact, ProvenanceVia::Argocd);
        p.repo = Some(normalize_repo(&repo));
        p.path = text("path").or_else(|| text("chart"));
        p.git_ref = text("targetRevision");
        p.application = Some(self.name.clone());
        Some(p)
    }
}

/// A Kubernetes object's source through Argo CD: its tracking annotation (exact), else its tracking label (inferred
/// unless an Application of that name exists, since Helm sets `app.kubernetes.io/instance` too).
pub fn from_kube(r: &KubeResource, apps: &[ArgoApp]) -> Option<Provenance> {
    let api_kind = r.kind.and_then(KubeKind::api_kind).unwrap_or("");
    let namespace = r.namespace.as_deref().unwrap_or("");
    let tracked = r.annotations.get(TRACKING_ANNOTATION).and_then(|v| parse_tracking_id(v)).filter(|t| t.name == r.name);
    let (app, app_namespace, labelled) = match &tracked {
        Some(t) => (t.app.clone(), t.app_namespace.clone(), false),
        None => (TRACKING_LABELS.iter().find_map(|k| non_empty(r.labels.get(*k).map(String::as_str)))?, None, true),
    };
    let candidates = apps.iter().filter(|a| a.name == app && app_namespace.as_ref().is_none_or(|ns| *ns == a.namespace));
    // The Application usually lives with Argo CD, which may run in another cluster.
    let found = candidates.clone().find(|a| a.cluster == r.cluster).or_else(|| candidates.clone().next());
    let mut p = match found.and_then(ArgoApp::provenance) {
        Some(p) => p,
        None if labelled => Provenance::new(ProvenanceKind::Manifest, Confidence::Inferred, ProvenanceVia::Argocd),
        None => Provenance::new(ProvenanceKind::Manifest, Confidence::Exact, ProvenanceVia::Argocd),
    };
    if labelled && found.is_none() {
        p.confidence = Confidence::Inferred;
    }
    p.application = Some(app);
    let kind = tracked.as_ref().map(|t| t.kind.as_str()).unwrap_or(api_kind);
    p.address = Some(if namespace.is_empty() { format!("{kind}/{}", r.name) } else { format!("{kind}/{namespace}/{}", r.name) });
    Some(p)
}

// ================================================================ images and services

pub const OCI_SOURCE: &str = "org.opencontainers.image.source";
pub const OCI_REVISION: &str = "org.opencontainers.image.revision";

/// An image's source from its OCI labels (or the `git.*` tags the Datadog Agent makes of them).
pub fn from_oci(labels: &[String]) -> Option<Provenance> {
    let repo = first_tag(labels, &[OCI_SOURCE, "git.repository_url"])?;
    let mut p = Provenance::new(ProvenanceKind::Dockerfile, Confidence::Exact, ProvenanceVia::Oci);
    p.repo = Some(normalize_repo(&repo));
    p.git_ref = first_tag(labels, &[OCI_REVISION, "git.commit.sha"]);
    Some(p)
}

/// A service's source from the git metadata on its spans.
pub fn from_git_metadata(repository_url: &str, commit_sha: &str) -> Option<Provenance> {
    let repo = non_empty(Some(repository_url))?;
    let mut p = Provenance::new(ProvenanceKind::Code, Confidence::Exact, ProvenanceVia::GitMetadata);
    p.repo = Some(normalize_repo(&repo));
    p.git_ref = non_empty(Some(commit_sha));
    Some(p)
}

// ================================================================ the org's resources

/// What a tagging policy or a provenance lookup is about.
#[derive(Serialize, Deserialize, ToSchema, Clone, Copy, PartialEq, Eq, Debug, Hash, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum TagTarget {
    /// Cloud resources (AWS, Azure, Google Cloud), as the account scans list them.
    Cloud,
    /// Kubernetes workloads, services and ingresses.
    Kubernetes,
    /// Container images (their SBOMs).
    Image,
    /// APM services.
    Service,
}

impl TagTarget {
    pub const ALL: [TagTarget; 4] = [TagTarget::Cloud, TagTarget::Kubernetes, TagTarget::Image, TagTarget::Service];

    pub fn as_str(self) -> &'static str {
        match self {
            TagTarget::Cloud => "cloud",
            TagTarget::Kubernetes => "kubernetes",
            TagTarget::Image => "image",
            TagTarget::Service => "service",
        }
    }

    pub fn parse(s: &str) -> Option<TagTarget> {
        TagTarget::ALL.into_iter().find(|t| t.as_str() == s)
    }
}

/// A resource with its tags and source.
#[derive(Serialize, Deserialize, ToSchema, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TaggedResource {
    /// The cloud resource's id (ARN, Azure resource id, …), the Kubernetes uid, the image id or `service:<name>`.
    pub id: String,
    pub target: TagTarget,
    pub name: String,
    /// `AwsS3Bucket`, `kube_deployment`, `container_image`, `service`.
    pub resource_type: String,
    /// aws | azure | gcp
    pub provider: Option<String>,
    pub account: Option<String>,
    pub region: Option<String>,
    pub cluster: Option<String>,
    pub namespace: Option<String>,
    pub env: Option<String>,
    pub team: Option<String>,
    /// `key:value` tags (Kubernetes labels and the Argo CD tracking annotation included).
    pub tags: Vec<String>,
    pub provenance: Option<Provenance>,
}

/// The Kubernetes kinds tagging and provenance are about: what people deploy (owned objects follow their owner).
pub fn kube_type(kind: KubeKind) -> Option<&'static str> {
    Some(match kind {
        KubeKind::Deployments => "kube_deployment",
        KubeKind::StatefulSets => "kube_statefulset",
        KubeKind::DaemonSets => "kube_daemonset",
        KubeKind::CronJobs => "kube_cronjob",
        KubeKind::Services => "kube_service",
        KubeKind::Ingresses => "kube_ingress",
        _ => return None,
    })
}

/// A Kubernetes object as a tagged resource; None for kinds left out and owned objects.
pub fn kube_resource(r: &KubeResource) -> Option<TaggedResource> {
    let resource_type = kube_type(r.kind?)?;
    if r.owner.is_some() {
        return None;
    }
    let mut tags = r.tags.clone();
    if let Some(t) = r.annotations.get(TRACKING_ANNOTATION) {
        tags.push(format!("{TRACKING_ANNOTATION}:{t}"));
    }
    let tag = |k: &str| tag_values(&r.tags, k).first().map(|v| v.to_string());
    Some(TaggedResource {
        id: r.uid.clone(),
        target: TagTarget::Kubernetes,
        name: r.name.clone(),
        resource_type: resource_type.into(),
        provider: None,
        account: tag("aws_account"),
        region: tag("region"),
        cluster: Some(r.cluster.clone()).filter(|c| !c.is_empty()),
        namespace: r.namespace.clone(),
        env: tag("env"),
        team: tag("team"),
        tags,
        provenance: r.provenance.clone(),
    })
}

/// A scanned cloud resource as a tagged resource.
#[allow(clippy::too_many_arguments)]
pub fn cloud_resource(provider: &str, account: &str, uid: &str, name: &str, resource_type: &str, region: &str, tags: Vec<String>, state: &HashMap<String, Provenance>) -> TaggedResource {
    let first = |k: &str| tag_values(&tags, k).first().map(|v| v.to_string());
    TaggedResource {
        id: uid.into(),
        target: TagTarget::Cloud,
        name: if name.is_empty() { uid.rsplit(['/', ':']).next().unwrap_or(uid).to_string() } else { name.to_string() },
        resource_type: resource_type.into(),
        provider: Some(provider.into()),
        account: Some(account.into()),
        region: non_empty(Some(region)),
        cluster: None,
        namespace: None,
        env: first("env"),
        team: first("team"),
        provenance: resolve_cloud(uid, &tags, state),
        tags,
    }
}

/// An image (its SBOM's entity) as a tagged resource.
pub fn image_resource(id: &str, name: &str, env: &str, tags: Vec<String>) -> TaggedResource {
    let first = |k: &str| tag_values(&tags, k).first().map(|v| v.to_string());
    TaggedResource {
        id: id.into(),
        target: TagTarget::Image,
        name: name.into(),
        resource_type: "container_image".into(),
        provider: None,
        account: None,
        region: None,
        cluster: first("kube_cluster_name"),
        namespace: None,
        env: non_empty(Some(env)).or_else(|| first("env")),
        team: first("team"),
        provenance: from_oci(&tags),
        tags,
    }
}

/// A service's latest metadata on its spans.
#[derive(clickhouse::Row, Deserialize, Clone, Debug, Default)]
pub struct ServiceGitMeta {
    pub service: String,
    pub env: String,
    pub version: String,
    pub team: String,
    pub repository_url: String,
    pub commit_sha: String,
}

/// A service as a tagged resource: its unified service tags and git metadata as tags.
pub fn service_resource(s: &ServiceGitMeta) -> TaggedResource {
    let tags: Vec<String> = [("service", &s.service), ("env", &s.env), ("version", &s.version), ("team", &s.team), ("git.repository_url", &s.repository_url), ("git.commit.sha", &s.commit_sha)]
        .iter()
        .filter(|(_, v)| !v.is_empty())
        .map(|(k, v)| format!("{k}:{v}"))
        .collect();
    TaggedResource {
        id: format!("service:{}", s.service),
        target: TagTarget::Service,
        name: s.service.clone(),
        resource_type: "service".into(),
        provider: None,
        account: None,
        region: None,
        cluster: None,
        namespace: None,
        env: non_empty(Some(&s.env)),
        team: non_empty(Some(&s.team)),
        provenance: from_git_metadata(&s.repository_url, &s.commit_sha),
        tags,
    }
}

/// Spans looked at for services' metadata.
const SERVICE_LOOKBACK_HOURS: u32 = 6;
const MAX_SERVICES: u32 = 5000;
/// SBOMs older than this are of images gone (as Cloud Security's).
const IMAGE_STALE_DAYS: i32 = 7;

/// The Argo CD Applications in the Kubernetes inventory (the Cluster Agent's custom resource collection).
pub async fn argo_apps(st: &AppState, org: &str) -> Result<Vec<ArgoApp>, ApiError> {
    Ok(super::kubernetes_autoscaling::custom_objects(st, org, &["Application"])
        .await?
        .into_iter()
        .filter(|o| ArgoApp::is_argo(&o.doc))
        .map(|o| ArgoApp { cluster: o.cluster, namespace: o.namespace, name: o.name, doc: o.doc })
        .collect())
}

async fn services(st: &AppState, org: &str) -> Result<Vec<ServiceGitMeta>, ApiError> {
    // The latest non-empty value of the first attribute set (tracers put git metadata on a trace chunk's first span).
    let first = |exprs: &[&str]| {
        let e = format!("coalesce({}, '')", exprs.iter().map(|e| format!("nullIf({e}, '')")).collect::<Vec<_>>().join(", "));
        format!("anyLastIf({e}, {e} != '')")
    };
    let sql = format!(
        "SELECT toString(ServiceName) AS service, {env} AS env, {version} AS version, {team} AS team, {repo} AS repository_url, {sha} AS commit_sha \
         FROM otel.otel_traces WHERE OrgId = {{oc_org:String}} AND Timestamp > now() - INTERVAL {SERVICE_LOOKBACK_HOURS} HOUR AND ServiceName != '' \
         GROUP BY ServiceName ORDER BY ServiceName LIMIT {MAX_SERVICES}",
        env = first(&["ResourceAttributes['deployment.environment']", "ResourceAttributes['deployment.environment.name']"]),
        version = first(&["ResourceAttributes['service.version']"]),
        team = first(&["ResourceAttributes['team']", "SpanAttributes['team']"]),
        repo = first(&["ResourceAttributes['git.repository_url']", "SpanAttributes['_dd.git.repository_url']", "SpanAttributes['git.repository_url']", "ResourceAttributes['vcs.repository.url.full']"]),
        sha = first(&["ResourceAttributes['git.commit.sha']", "SpanAttributes['_dd.git.commit.sha']", "SpanAttributes['git.commit.sha']", "ResourceAttributes['vcs.ref.head.revision']"]),
    );
    Ok(st.ch(org).query(&sql).fetch_all::<ServiceGitMeta>().await?)
}

async fn cloud_resources(st: &AppState, org: &str) -> Result<Vec<TaggedResource>, ApiError> {
    let rows: Vec<(String, String, String, String, String, String, Value)> = sqlx::query_as(
        "SELECT DISTINCT ON (resource_uid) provider, account_id, resource_uid, resource_name, resource_type, region, resource_tags FROM posture_findings \
         WHERE org_id = $1 ORDER BY resource_uid, last_seen_at DESC",
    )
    .bind(org)
    .fetch_all(&st.postgres)
    .await?;
    // No state source is read yet (see the module docs): tags only.
    let state = HashMap::new();
    Ok(rows
        .into_iter()
        .map(|(provider, account, uid, name, resource_type, region, tags)| {
            let tags: Vec<String> = tags.as_array().into_iter().flatten().filter_map(Value::as_str).map(str::to_string).collect();
            cloud_resource(&provider, &account, &uid, &name, &resource_type, &region, tags, &state)
        })
        .collect())
}

async fn images(st: &AppState, org: &str) -> Result<Vec<TaggedResource>, ApiError> {
    let rows: Vec<(String, String, String, Vec<String>)> =
        sqlx::query_as("SELECT id, name, env, tags FROM csm_sbom_entities WHERE org_id = $1 AND kind = 'container_image' AND last_seen > now() - make_interval(days => $2) ORDER BY name, id")
            .bind(org)
            .bind(IMAGE_STALE_DAYS)
            .fetch_all(&st.postgres)
            .await?;
    Ok(rows.into_iter().map(|(id, name, env, tags)| image_resource(&id, &name, &env, tags)).collect())
}

/// The org's resources of `targets` with their tags and provenance.
pub async fn inventory(st: &AppState, org: &str, targets: &[TagTarget]) -> Result<Vec<TaggedResource>, ApiError> {
    let want = |t: TagTarget| targets.contains(&t);
    let (cloud, kube, imgs, svcs) = tokio::try_join!(
        async { if want(TagTarget::Cloud) { cloud_resources(st, org).await } else { Ok(vec![]) } },
        async { if want(TagTarget::Kubernetes) { super::kubernetes::resources(st, org).await.map(|rows| rows.iter().filter_map(kube_resource).collect()) } else { Ok(vec![]) } },
        async { if want(TagTarget::Image) { images(st, org).await } else { Ok(vec![]) } },
        async { if want(TagTarget::Service) { services(st, org).await.map(|s| s.iter().map(service_resource).collect()) } else { Ok(vec![]) } },
    )?;
    Ok([cloud, kube, imgs, svcs].into_iter().flatten().collect())
}

// ================================================================ API

#[derive(Deserialize, IntoParams)]
#[serde(rename_all = "camelCase")]
pub struct ProvenanceQuery {
    /// cloud | kubernetes | image | service; all when empty.
    pub target: Option<String>,
    /// Only resources without a source.
    #[serde(default)]
    pub missing: bool,
}

/// Counts of resources with and without a source, per target and per way it was found.
#[derive(Serialize, ToSchema, Debug, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct ProvenanceSummary {
    pub total: usize,
    pub resolved: usize,
    /// Per target (`cloud`, …): [resources, resolved].
    pub by_target: BTreeMap<String, [usize; 2]>,
    /// Per way (`tag`, `argocd`, …): resolved resources.
    pub by_via: BTreeMap<String, usize>,
}

pub fn summarize(rows: &[TaggedResource]) -> ProvenanceSummary {
    let mut s = ProvenanceSummary::default();
    for r in rows {
        s.total += 1;
        let t = s.by_target.entry(r.target.as_str().into()).or_default();
        t[0] += 1;
        if let Some(p) = &r.provenance {
            s.resolved += 1;
            t[1] += 1;
            *s.by_via.entry(p.via.as_str().into()).or_default() += 1;
        }
    }
    s
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProvenanceList {
    pub summary: ProvenanceSummary,
    pub resources: Vec<TaggedResource>,
}

/// The org's resources with their source (Cloud Security → Inventory).
#[utoipa::path(get, path = "/api/v1/provenance/resources", operation_id = "list_resource_provenance", params(ProvenanceQuery),
    responses((status = 200, body = ProvenanceList), (status = 400, body = crate::error::ErrorBody)), tag = "tagging")]
pub async fn resources(State(st): State<AppState>, ctx: AuthContext, Query(q): Query<ProvenanceQuery>) -> ApiResult<ProvenanceList> {
    let targets: Vec<TagTarget> = match q.target.as_deref().filter(|t| !t.is_empty()) {
        Some(t) => vec![TagTarget::parse(t).ok_or_else(|| ApiError::BadRequest(format!("Unknown target '{t}'")))?],
        None => TagTarget::ALL.to_vec(),
    };
    let rows = inventory(&st, &ctx.org_id, &targets).await?;
    let summary = summarize(&rows);
    let resources = rows.into_iter().filter(|r| !q.missing || r.provenance.is_none()).collect();
    Ok(Json(ProvenanceList { summary, resources }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn tags(t: &[&str]) -> Vec<String> {
        t.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn terraform_tags_name_the_repository_and_module() {
        let p = from_tags(&tags(&["env:prod", "tf_repo:https://github.com/example-org/infra-terraform.git", "tf_path:envs/prod/storage", "managed-by:terraform"])).unwrap();
        assert_eq!(p.repo.as_deref(), Some("github.com/example-org/infra-terraform"));
        assert_eq!(p.path.as_deref(), Some("envs/prod/storage"));
        assert_eq!((p.kind, p.confidence, p.via), (ProvenanceKind::Terraform, Confidence::Exact, ProvenanceVia::Tag));
        // Keys compare without case, dashes and underscores alike (Azure tags are often written TF-Repo).
        let p = from_tags(&tags(&["TF-Repo:gitlab.example.com/example-org/infra-terraform"])).unwrap();
        assert_eq!(p.repo.as_deref(), Some("gitlab.example.com/example-org/infra-terraform"));
        assert_eq!(p.path, None);
    }

    #[test]
    fn yor_tags_give_the_file_commit_and_resource() {
        let p = from_tags(&tags(&[
            "git_org:example-org",
            "git_repo:infra-terraform",
            "git_file:modules/storage/main.tf",
            "git_commit:4f1c2a9e0b7d",
            "yor_name:assets",
            "yor_trace:7e3c1a52-9b0d-4c6e-8f21-2d5a4b3c1e00",
        ]))
        .unwrap();
        assert_eq!(p.repo.as_deref(), Some("example-org/infra-terraform"));
        assert_eq!(p.path.as_deref(), Some("modules/storage/main.tf"));
        assert_eq!(p.git_ref.as_deref(), Some("4f1c2a9e0b7d"));
        assert_eq!(p.address.as_deref(), Some("assets"));
        assert_eq!(p.kind, ProvenanceKind::Terraform);
        // CloudFormation templates tagged by Yor are manifests.
        assert_eq!(from_tags(&tags(&["git_repo:example-org/stacks", "git_file:stacks/queue.yaml"])).unwrap().kind, ProvenanceKind::Manifest);
    }

    #[test]
    fn managed_by_alone_or_no_tags_have_no_provenance() {
        assert_eq!(from_tags(&tags(&["managed-by:terraform", "team:storefront"])), None);
        assert_eq!(from_tags(&[]), None);
    }

    #[test]
    fn terraform_state_maps_cloud_ids_to_addresses() {
        let state = json!({"version": 4, "resources": [
            {"mode": "managed", "type": "aws_s3_bucket", "name": "assets", "module": "module.storage",
             "instances": [{"attributes": {"arn": "arn:aws:s3:::example-shop-assets", "id": "example-shop-assets"}}]},
            {"mode": "managed", "type": "azurerm_storage_account", "name": "logs",
             "instances": [{"index_key": "prod", "attributes": {"id": "/subscriptions/0000/resourceGroups/shop/providers/Microsoft.Storage/storageAccounts/shoplogs"}}]},
            {"mode": "data", "type": "aws_iam_policy_document", "name": "x", "instances": [{"attributes": {"arn": "arn:aws:iam::123456789012:policy/x"}}]}
        ]});
        let idx = parse_state(&state, "https://github.com/example-org/infra-terraform", "envs/prod");
        assert_eq!(idx.len(), 2);
        let s3 = &idx["arn:aws:s3:::example-shop-assets"];
        assert_eq!(s3.address.as_deref(), Some("module.storage.aws_s3_bucket.assets"));
        assert_eq!((s3.via, s3.kind), (ProvenanceVia::State, ProvenanceKind::Terraform));
        let az = &idx["/subscriptions/0000/resourcegroups/shop/providers/microsoft.storage/storageaccounts/shoplogs"];
        assert_eq!(az.address.as_deref(), Some("azurerm_storage_account.logs[\"prod\"]"));
        // Tags win over the state; the state answers for untagged resources, ids without case.
        assert_eq!(resolve_cloud("ARN:AWS:S3:::EXAMPLE-SHOP-ASSETS", &[], &idx).unwrap().via, ProvenanceVia::State);
        assert_eq!(resolve_cloud("arn:aws:s3:::example-shop-assets", &tags(&["tf_repo:example-org/x"]), &idx).unwrap().via, ProvenanceVia::Tag);
    }

    #[test]
    fn argo_tracking_ids_parse() {
        let t = parse_tracking_id("order-service:apps/Deployment:checkout/order-service").unwrap();
        assert_eq!(t, TrackingId { app: "order-service".into(), app_namespace: None, group: "apps".into(), kind: "Deployment".into(), namespace: "checkout".into(), name: "order-service".into() });
        // Core kinds have no group; cluster-scoped objects no namespace; apps outside argocd's namespace are prefixed.
        let t = parse_tracking_id("storefront:/Service:checkout/storefront-api").unwrap();
        assert_eq!((t.group.as_str(), t.kind.as_str()), ("", "Service"));
        let t = parse_tracking_id("platform:rbac.authorization.k8s.io/ClusterRole:/viewer").unwrap();
        assert_eq!((t.namespace.as_str(), t.name.as_str()), ("", "viewer"));
        let t = parse_tracking_id("team-apps_billing:apps/Deployment:payments/billing").unwrap();
        assert_eq!((t.app.as_str(), t.app_namespace.as_deref()), ("billing", Some("team-apps")));
        assert_eq!(parse_tracking_id("order-service"), None);
        assert_eq!(parse_tracking_id("app:apps/Deployment"), None);
        assert_eq!(parse_tracking_id(":apps/Deployment:ns/x"), None);
    }

    fn deployment(name: &str, annotations: &[(&str, &str)], labels: &[(&str, &str)]) -> KubeResource {
        KubeResource {
            uid: format!("uid-{name}"),
            kind: Some(KubeKind::Deployments),
            cluster: "shop-prod-eu".into(),
            namespace: Some("checkout".into()),
            name: name.into(),
            annotations: annotations.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
            labels: labels.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
            ..Default::default()
        }
    }

    fn app(name: &str, cluster: &str, doc: Value) -> ArgoApp {
        ArgoApp { cluster: cluster.into(), namespace: "argocd".into(), name: name.into(), doc }
    }

    #[test]
    fn kubernetes_objects_resolve_through_their_argo_application() {
        let apps = vec![
            app("order-service", "ops", json!({"spec": {"source": {"repoURL": "https://github.com/example-org/k8s-apps.git", "path": "apps/order-service/overlays/prod", "targetRevision": "main", "kustomize": {}}}, "status": {"sourceType": "Kustomize"}})),
            app("billing", "ops", json!({"spec": {"sources": [{"repoURL": "https://github.com/example-org/k8s-apps.git", "ref": "values"}, {"repoURL": "https://charts.example.com", "chart": "billing", "targetRevision": "3.2.1", "helm": {}}]}})),
        ];
        let d = deployment("order-service", &[(TRACKING_ANNOTATION, "order-service:apps/Deployment:checkout/order-service")], &[]);
        let p = from_kube(&d, &apps).unwrap();
        assert_eq!(p.repo.as_deref(), Some("github.com/example-org/k8s-apps"));
        assert_eq!(p.path.as_deref(), Some("apps/order-service/overlays/prod"));
        assert_eq!(p.git_ref.as_deref(), Some("main"));
        assert_eq!(p.application.as_deref(), Some("order-service"));
        assert_eq!(p.address.as_deref(), Some("Deployment/checkout/order-service"));
        assert_eq!((p.kind, p.confidence, p.via), (ProvenanceKind::Kustomize, Confidence::Exact, ProvenanceVia::Argocd));
        // Several sources: the one with a chart; Helm from the source.
        let b = deployment("billing", &[(TRACKING_ANNOTATION, "billing:apps/Deployment:checkout/billing")], &[]);
        let p = from_kube(&b, &apps).unwrap();
        assert_eq!((p.repo.as_deref(), p.path.as_deref(), p.git_ref.as_deref(), p.kind), (Some("charts.example.com"), Some("billing"), Some("3.2.1"), ProvenanceKind::Helm));
    }

    #[test]
    fn without_the_application_object_only_its_name_is_known() {
        let d = deployment("catalog-service", &[(TRACKING_ANNOTATION, "catalog:apps/Deployment:checkout/catalog-service")], &[]);
        let p = from_kube(&d, &[]).unwrap();
        assert_eq!((p.repo, p.application.as_deref(), p.confidence), (None, Some("catalog"), Confidence::Exact));
        // A tracking id copied from another object doesn't track this one.
        assert_eq!(from_kube(&deployment("other", &[(TRACKING_ANNOTATION, "catalog:apps/Deployment:checkout/catalog-service")], &[]), &[]), None);
    }

    #[test]
    fn the_instance_label_is_inferred_unless_its_application_exists() {
        let d = deployment("inventory-service", &[], &[("app.kubernetes.io/instance", "inventory")]);
        assert_eq!(from_kube(&d, &[]).unwrap().confidence, Confidence::Inferred);
        let apps = vec![app("inventory", "ops", json!({"spec": {"source": {"repoURL": "git@github.com:example-org/k8s-apps.git", "path": "apps/inventory"}}}))];
        let p = from_kube(&d, &apps).unwrap();
        assert_eq!((p.confidence, p.repo.as_deref(), p.kind), (Confidence::Exact, Some("github.com/example-org/k8s-apps"), ProvenanceKind::Manifest));
        assert_eq!(from_kube(&deployment("plain", &[], &[("app", "plain")]), &apps), None);
    }

    #[test]
    fn the_application_in_the_same_cluster_wins() {
        let apps = vec![
            app("shop", "ops", json!({"spec": {"source": {"repoURL": "https://github.com/example-org/ops.git", "path": "a"}}})),
            app("shop", "shop-prod-eu", json!({"spec": {"source": {"repoURL": "https://github.com/example-org/shop.git", "path": "b"}}})),
        ];
        let d = deployment("x", &[(TRACKING_ANNOTATION, "shop:apps/Deployment:checkout/x")], &[]);
        assert_eq!(from_kube(&d, &apps).unwrap().path.as_deref(), Some("b"));
        assert!(!ArgoApp::is_argo(&json!({"spec": {"descriptor": {}}})));
    }

    #[test]
    fn oci_labels_and_agent_git_tags_name_the_image_source() {
        let p = from_oci(&tags(&["image_name:registry.example.com/storefront-api", "org.opencontainers.image.source:https://github.com/example-org/storefront-api", "org.opencontainers.image.revision:9c1e7b2"])).unwrap();
        assert_eq!((p.repo.as_deref(), p.git_ref.as_deref(), p.kind, p.via), (Some("github.com/example-org/storefront-api"), Some("9c1e7b2"), ProvenanceKind::Dockerfile, ProvenanceVia::Oci));
        let p = from_oci(&tags(&["git.repository_url:github.com/example-org/billing", "git.commit.sha:ab12"])).unwrap();
        assert_eq!(p.repo.as_deref(), Some("github.com/example-org/billing"));
        assert_eq!(from_oci(&tags(&["org.opencontainers.image.revision:9c1e7b2"])), None);
    }

    #[test]
    fn services_resolve_from_git_metadata() {
        let s = ServiceGitMeta { service: "order-service".into(), env: "prod".into(), version: "2.4.1".into(), repository_url: "https://github.com/example-org/order-service.git".into(), commit_sha: "4f1c2a9".into(), ..Default::default() };
        let r = service_resource(&s);
        assert_eq!(r.id, "service:order-service");
        assert_eq!(r.tags, tags(&["service:order-service", "env:prod", "version:2.4.1", "git.repository_url:https://github.com/example-org/order-service.git", "git.commit.sha:4f1c2a9"]));
        let p = r.provenance.unwrap();
        assert_eq!((p.repo.as_deref(), p.git_ref.as_deref(), p.kind), (Some("github.com/example-org/order-service"), Some("4f1c2a9"), ProvenanceKind::Code));
        assert!(service_resource(&ServiceGitMeta { service: "billing".into(), ..Default::default() }).provenance.is_none());
    }

    #[test]
    fn kube_resources_keep_deployed_kinds_and_the_tracking_annotation() {
        let mut d = deployment("order-service", &[(TRACKING_ANNOTATION, "order-service:apps/Deployment:checkout/order-service")], &[]);
        d.tags = tags(&["env:prod", "team:checkout"]);
        let r = kube_resource(&d).unwrap();
        assert_eq!((r.resource_type.as_str(), r.env.as_deref(), r.team.as_deref()), ("kube_deployment", Some("prod"), Some("checkout")));
        assert!(r.tags.contains(&format!("{TRACKING_ANNOTATION}:order-service:apps/Deployment:checkout/order-service")));
        let mut pod = deployment("order-service-1", &[], &[]);
        pod.kind = Some(KubeKind::Pods);
        assert!(kube_resource(&pod).is_none());
        let mut owned = deployment("x", &[], &[]);
        owned.owner = Some(super::super::kubernetes::KubeObjectRef { kind: "Rollout".into(), name: "x".into(), uid: "u".into() });
        assert!(kube_resource(&owned).is_none());
    }

    #[test]
    fn cloud_resources_take_names_and_tags() {
        let r = cloud_resource("aws", "123456789012", "arn:aws:s3:::example-shop-assets", "", "AwsS3Bucket", "eu-west-1", tags(&["team:storefront", "tf_repo:example-org/infra-terraform"]), &HashMap::new());
        assert_eq!((r.name.as_str(), r.team.as_deref(), r.region.as_deref()), ("example-shop-assets", Some("storefront"), Some("eu-west-1")));
        assert_eq!(r.provenance.unwrap().via, ProvenanceVia::Tag);
        let i = image_resource("sha256:abc", "registry.example.com/billing:3.2.1", "", tags(&["env:staging"]));
        assert_eq!((i.env.as_deref(), i.provenance), (Some("staging"), None));
    }

    #[test]
    fn summary_counts_resolved_resources_per_target_and_way() {
        let a = service_resource(&ServiceGitMeta { service: "a".into(), repository_url: "github.com/example-org/a".into(), ..Default::default() });
        let b = service_resource(&ServiceGitMeta { service: "b".into(), ..Default::default() });
        let s = summarize(&[a, b]);
        assert_eq!((s.total, s.resolved), (2, 1));
        assert_eq!(s.by_target["service"], [2, 1]);
        assert_eq!(s.by_via["git_metadata"], 1);
    }
}
