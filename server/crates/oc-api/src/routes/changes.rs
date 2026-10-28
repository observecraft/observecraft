//! Infrastructure → Changes (change tracking): what changed in the org's infrastructure over time,
//! derived only from data already stored:
//! - Kubernetes rollouts: the replica sets of each deployment (their revision, creation time and pod
//!   template images; old replica sets stay in the cluster for the deployment's revision history),
//! - Kubernetes spec changes: successive `metadata.generation`s of deployments, stateful sets and
//!   daemon sets in the collectors' raw object snapshots (`otel.otel_k8s_objects`, kept a day):
//!   scaling, stateful set / daemon set image rollouts and other spec edits,
//! - pod crashes and evictions: Kubernetes events (`otel.k8s_events`),
//! - deployments: the first span of a new `service.version` (APM) and CI deployment jobs,
//! - change events from the Datadog Agent and the events API (`otel.otel_events`): deploy, config,
//!   scale and feature flag events, by tag or title,
//! - configuration changes made in ObserveCraft (monitors, downtimes, log pipelines, indexes, …)
//!   from the audit trail, for organization admins only (the audit trail is admin-only).

use std::collections::{BTreeMap, HashMap};

use axum::extract::State;
use axum::Json;
use oc_query::parse_query;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;

use super::catalog::{facets, CatalogFacet, Faceted};
use super::kubernetes::{image_parts, KubeKind, KubeResource};
use super::logs::TimeRange;
use crate::auth::{Access, AuthContext};
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;

#[derive(Serialize, Deserialize, ToSchema, Clone, Copy, PartialEq, Eq, Hash, Debug)]
#[serde(rename_all = "snake_case")]
pub enum ChangeType {
    /// A new version of a service (APM, CI or a deploy event).
    Deployment,
    /// A Kubernetes workload rolled out a new pod template.
    Rollout,
    /// Replica count changes.
    Scale,
    /// Spec or configuration edits.
    Config,
    /// Pods crashing, OOM-killed or evicted.
    Crash,
    FeatureFlag,
}

impl ChangeType {
    fn as_str(self) -> &'static str {
        match self {
            ChangeType::Deployment => "deployment",
            ChangeType::Rollout => "rollout",
            ChangeType::Scale => "scale",
            ChangeType::Config => "config",
            ChangeType::Crash => "crash",
            ChangeType::FeatureFlag => "feature_flag",
        }
    }
}

/// Where a change was found.
#[derive(Serialize, Deserialize, ToSchema, Clone, Copy, PartialEq, Eq, Hash, Debug)]
#[serde(rename_all = "snake_case")]
pub enum ChangeSource {
    Kubernetes,
    Apm,
    Ci,
    Events,
    Audit,
}

impl ChangeSource {
    fn as_str(self) -> &'static str {
        match self {
            ChangeSource::Kubernetes => "kubernetes",
            ChangeSource::Apm => "apm",
            ChangeSource::Ci => "ci",
            ChangeSource::Events => "events",
            ChangeSource::Audit => "audit",
        }
    }
}

#[derive(Serialize, ToSchema, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Change {
    pub id: String,
    /// Epoch ms.
    pub timestamp: i64,
    pub change_type: ChangeType,
    pub source: ChangeSource,
    /// `kube_deployment`, `service`, `monitor`, `host`, …
    pub resource_type: String,
    pub resource: String,
    pub service: Option<String>,
    pub env: Option<String>,
    pub cluster: Option<String>,
    pub namespace: Option<String>,
    pub summary: String,
    pub author: Option<String>,
    /// Before and after (images, replicas, versions), when the change has them.
    pub previous: Option<String>,
    pub current: Option<String>,
    pub details: BTreeMap<String, String>,
    /// The Kubernetes Explorer kind and uid of the changed object, to open it there.
    pub kube_kind: Option<KubeKind>,
    pub kube_uid: Option<String>,
}

impl Change {
    fn new(id: String, timestamp: i64, change_type: ChangeType, source: ChangeSource, resource_type: &str, resource: &str, summary: String) -> Change {
        Change {
            id,
            timestamp,
            change_type,
            source,
            resource_type: resource_type.to_string(),
            resource: resource.to_string(),
            service: None,
            env: None,
            cluster: None,
            namespace: None,
            summary,
            author: None,
            previous: None,
            current: None,
            details: BTreeMap::new(),
            kube_kind: None,
            kube_uid: None,
        }
    }
}

impl Faceted for Change {
    fn facet_values(&self, key: &str) -> Vec<&str> {
        fn one(v: &Option<String>) -> Vec<&str> {
            v.as_deref().into_iter().collect()
        }
        match key {
            "change_type" | "type" => vec![self.change_type.as_str()],
            "source" => vec![self.source.as_str()],
            "resource_type" => vec![self.resource_type.as_str()],
            "resource" => vec![self.resource.as_str()],
            "service" => one(&self.service),
            "env" => one(&self.env),
            "cluster" | "kube_cluster_name" => one(&self.cluster),
            "namespace" | "kube_namespace" => one(&self.namespace),
            "author" => one(&self.author),
            k => self.details.get(k).map(String::as_str).into_iter().collect(),
        }
    }

    fn text(&self) -> String {
        format!("{} {}", self.resource, self.summary)
    }
}

fn non_empty(s: &str) -> Option<String> {
    (!s.is_empty()).then(|| s.to_string())
}

fn tag<'a>(tags: &'a [String], key: &str) -> Option<&'a str> {
    let prefix = format!("{key}:");
    tags.iter().find_map(|t| t.strip_prefix(&prefix)).filter(|v| !v.is_empty())
}

/// `registry/order-service:1.4.2` → `order-service:1.4.2`.
fn short_image(image: &str) -> String {
    let (_, short, tag) = image_parts(image);
    format!("{short}:{tag}")
}

/// "order-service:1.4.1 → 1.4.2" for each image that changed between two image lists (by position,
/// as containers keep their order in a pod template).
fn image_changes(prev: &[String], cur: &[String]) -> Vec<String> {
    cur.iter()
        .enumerate()
        .filter_map(|(i, c)| {
            let p = prev.get(i)?;
            if p == c {
                return None;
            }
            let ((pn, _, pt), (cn, short, ct)) = (image_parts(p), image_parts(c));
            Some(if pn == cn { format!("{short}: {pt} → {ct}") } else { format!("{} → {}", short_image(p), short_image(c)) })
        })
        .collect()
}

// ================================================================ Kubernetes rollouts

/// One change per deployment replica set created in `range`: a rollout of that revision, against
/// the previous revision's images.
fn replicaset_rollouts(rows: &[KubeResource], range: TimeRange) -> Vec<Change> {
    let mut by_owner: HashMap<&str, Vec<&KubeResource>> = HashMap::new();
    for r in rows.iter().filter(|r| r.kind == Some(KubeKind::ReplicaSets)) {
        if let Some(o) = r.owner.as_ref().filter(|o| o.kind == "Deployment") {
            by_owner.entry(o.uid.as_str()).or_default().push(r);
        }
    }
    let revision = |r: &KubeResource| r.annotations.get("deployment.kubernetes.io/revision").and_then(|v| v.parse::<i64>().ok());
    let mut out = Vec::new();
    for (owner_uid, mut sets) in by_owner {
        sets.sort_by_key(|r| (revision(r).unwrap_or(0), r.created_at.unwrap_or(0)));
        for (i, rs) in sets.iter().enumerate() {
            let Some(created) = rs.created_at.filter(|t| *t >= range.from && *t <= range.to) else { continue };
            let owner = rs.owner.as_ref().map(|o| o.name.as_str()).unwrap_or(rs.name.as_str());
            let prev = i.checked_sub(1).map(|p| sets[p]);
            let summary = match prev {
                None => format!("Created with {}", rs.images.iter().map(|i| short_image(i)).collect::<Vec<_>>().join(", ")),
                Some(p) => {
                    let changed = image_changes(&p.images, &rs.images);
                    if changed.is_empty() {
                        "Rolled out a new pod template".to_string()
                    } else {
                        format!("Rolled out {}", changed.join(", "))
                    }
                }
            };
            let mut c = Change::new(format!("rs:{}", rs.uid), created, ChangeType::Rollout, ChangeSource::Kubernetes, "kube_deployment", owner, summary);
            c.cluster = non_empty(&rs.cluster);
            c.namespace = rs.namespace.clone();
            c.service = tag(&rs.tags, "service").map(str::to_string);
            c.env = tag(&rs.tags, "env").map(str::to_string);
            c.previous = prev.map(|p| p.images.join(", "));
            c.current = Some(rs.images.join(", "));
            c.kube_kind = Some(KubeKind::Deployments);
            c.kube_uid = Some(owner_uid.to_string());
            c.details.insert("replica_set".into(), rs.name.clone());
            if let Some(rev) = revision(rs) {
                c.details.insert("revision".into(), rev.to_string());
            }
            if let Some(cause) = rs.annotations.get("kubernetes.io/change-cause") {
                c.details.insert("change_cause".into(), cause.clone());
            }
            out.push(c);
        }
    }
    out
}

// ================================================================ Kubernetes spec generations

/// The first snapshot of one generation of a workload.
#[derive(clickhouse::Row, Deserialize, Debug, Clone)]
struct Snapshot {
    cluster: String,
    kind: String,
    namespace: String,
    name: String,
    uid: String,
    generation: i64,
    first_seen: i64,
    spec: String,
    labels: String,
}

fn template_images(spec: &Value) -> Vec<String> {
    spec.pointer("/template/spec/containers").and_then(Value::as_array).into_iter().flatten().filter_map(|c| c.get("image").and_then(Value::as_str).map(str::to_string)).collect()
}

/// Keys of `a` and `b` (objects) whose values differ, skipping `skip`.
fn changed_keys(a: &Value, b: &Value, skip: &[&str]) -> Vec<String> {
    let empty = serde_json::Map::new();
    let (a, b) = (a.as_object().unwrap_or(&empty), b.as_object().unwrap_or(&empty));
    let mut keys: Vec<String> = a.keys().chain(b.keys()).filter(|k| !skip.contains(&k.as_str()) && a.get(*k) != b.get(*k)).cloned().collect();
    keys.sort();
    keys.dedup();
    keys
}

/// Changes between consecutive generations of each workload, for generations first seen in `range`.
/// Deployment pod template changes are left to [`replicaset_rollouts`], which also covers revisions
/// older than the raw snapshots.
fn spec_changes(mut snaps: Vec<Snapshot>, range: TimeRange) -> Vec<Change> {
    snaps.sort_by(|a, b| a.uid.cmp(&b.uid).then(a.generation.cmp(&b.generation)));
    let mut out = Vec::new();
    for pair in snaps.windows(2) {
        let (prev, cur) = (&pair[0], &pair[1]);
        if prev.uid != cur.uid || cur.first_seen < range.from || cur.first_seen > range.to {
            continue;
        }
        let (Ok(a), Ok(b)) = (serde_json::from_str::<Value>(&prev.spec), serde_json::from_str::<Value>(&cur.spec)) else { continue };
        let labels: BTreeMap<String, String> = serde_json::from_str(&cur.labels).unwrap_or_default();
        let (resource_type, kube_kind) = match cur.kind.as_str() {
            "Deployment" => ("kube_deployment", KubeKind::Deployments),
            "StatefulSet" => ("kube_statefulset", KubeKind::StatefulSets),
            "DaemonSet" => ("kube_daemonset", KubeKind::DaemonSets),
            _ => continue,
        };
        let base = |suffix: &str, change_type: ChangeType, summary: String| {
            let mut c = Change::new(format!("gen:{}:{}:{suffix}", cur.uid, cur.generation), cur.first_seen, change_type, ChangeSource::Kubernetes, resource_type, &cur.name, summary);
            c.cluster = non_empty(&cur.cluster);
            c.namespace = non_empty(&cur.namespace);
            c.service = labels.get("tags.datadoghq.com/service").or(labels.get("app.kubernetes.io/name")).or(labels.get("app")).cloned();
            c.env = labels.get("tags.datadoghq.com/env").cloned();
            c.kube_kind = Some(kube_kind);
            c.kube_uid = Some(cur.uid.clone());
            c.details.insert("generation".into(), format!("{} → {}", prev.generation, cur.generation));
            c
        };
        let replicas = |s: &Value| s.get("replicas").and_then(Value::as_i64);
        if cur.kind != "DaemonSet" && replicas(&a) != replicas(&b) {
            let (p, n) = (replicas(&a).unwrap_or(1), replicas(&b).unwrap_or(1));
            let mut c = base("scale", ChangeType::Scale, format!("Scaled from {p} to {n} replicas"));
            (c.previous, c.current) = (Some(p.to_string()), Some(n.to_string()));
            out.push(c);
        }
        if cur.kind != "Deployment" && a.get("template") != b.get("template") {
            let (pi, ci) = (template_images(&a), template_images(&b));
            let changed = image_changes(&pi, &ci);
            let mut c = if changed.is_empty() {
                let keys = changed_keys(a.pointer("/template/spec").unwrap_or(&Value::Null), b.pointer("/template/spec").unwrap_or(&Value::Null), &[]);
                base("template", ChangeType::Config, if keys.is_empty() { "Pod template changed".into() } else { format!("Pod template changed: {}", keys.join(", ")) })
            } else {
                base("template", ChangeType::Rollout, format!("Rolled out {}", changed.join(", ")))
            };
            (c.previous, c.current) = (Some(pi.join(", ")), Some(ci.join(", ")));
            out.push(c);
        }
        let other = changed_keys(&a, &b, &["replicas", "template"]);
        if !other.is_empty() {
            out.push(base("spec", ChangeType::Config, format!("Spec changed: {}", other.join(", "))));
        }
    }
    out
}

// ================================================================ Kubernetes events

#[derive(clickhouse::Row, Deserialize, Debug)]
struct KubeEventRow {
    cluster: String,
    ts: i64,
    reason: String,
    note: String,
    kind: String,
    namespace: String,
    name: String,
    uid: String,
    count: u32,
}

/// Reasons of Kubernetes events that are pod failures.
const CRASH_REASONS: &[&str] = &["BackOff", "CrashLoopBackOff", "OOMKilling", "Evicted"];

fn kube_event_change(e: KubeEventRow) -> Option<Change> {
    if !CRASH_REASONS.contains(&e.reason.as_str()) || (e.reason == "BackOff" && !e.note.contains("restarting failed container")) {
        return None;
    }
    let what = match e.reason.as_str() {
        "OOMKilling" => "OOM killed",
        "Evicted" => "Evicted",
        _ => "Crash looping",
    };
    let times = if e.count > 1 { format!(" ({}×)", e.count) } else { String::new() };
    let mut c = Change::new(
        format!("k8sev:{}:{}:{}", e.uid, e.ts, e.reason),
        e.ts,
        ChangeType::Crash,
        ChangeSource::Kubernetes,
        &format!("kube_{}", e.kind.to_lowercase()),
        &e.name,
        format!("{what}{times}: {}", e.note),
    );
    c.cluster = non_empty(&e.cluster);
    c.namespace = non_empty(&e.namespace);
    c.details.insert("reason".into(), e.reason);
    if e.kind == "Pod" {
        c.kube_kind = Some(KubeKind::Pods);
        c.kube_uid = non_empty(&e.uid);
    }
    Some(c)
}

// ================================================================ APM versions and CI deployments

#[derive(clickhouse::Row, Deserialize, Debug)]
struct VersionRow {
    service: String,
    env: String,
    version: String,
    first_seen: i64,
}

/// Spans of a version first seen this long after the range starts mean the version was deployed
/// then (a version already running at the start of the range is not a change).
const VERSION_GRACE_MS: i64 = 60_000;

/// A deployment for each version of a service (per env) that first appears in the range.
fn version_deployments(mut rows: Vec<VersionRow>, range: TimeRange) -> Vec<Change> {
    rows.sort_by(|a, b| (&a.service, &a.env, a.first_seen).cmp(&(&b.service, &b.env, b.first_seen)));
    let mut out = Vec::new();
    for (i, r) in rows.iter().enumerate() {
        if r.first_seen < range.from + VERSION_GRACE_MS {
            continue;
        }
        let prev = i.checked_sub(1).map(|p| &rows[p]).filter(|p| p.service == r.service && p.env == r.env);
        let summary = match prev {
            Some(p) => format!("Deployed version {} (was {})", r.version, p.version),
            None => format!("Deployed version {}", r.version),
        };
        let mut c = Change::new(format!("apm:{}:{}:{}", r.service, r.env, r.version), r.first_seen, ChangeType::Deployment, ChangeSource::Apm, "service", &r.service, summary);
        c.service = Some(r.service.clone());
        c.env = non_empty(&r.env);
        c.previous = prev.map(|p| p.version.clone());
        c.current = Some(r.version.clone());
        c.details.insert("version".into(), r.version.clone());
        out.push(c);
    }
    out
}

#[derive(clickhouse::Row, Deserialize, Debug)]
struct CiDeployRow {
    ts: i64,
    service: String,
    env: String,
    repository: String,
    sha: String,
    author: String,
    pipeline: String,
    job: String,
    status: String,
}

fn ci_deployment(r: CiDeployRow) -> Change {
    let target = if r.service.is_empty() { r.repository.clone() } else { r.service.clone() };
    let sha: String = r.sha.chars().take(8).collect();
    let summary = format!("{} deployed {}{} to {}", r.pipeline, target, if sha.is_empty() { String::new() } else { format!(" at {sha}") }, if r.env.is_empty() { "an environment" } else { &r.env });
    let mut c = Change::new(format!("ci:{}:{}:{}", r.pipeline, r.job, r.ts), r.ts, ChangeType::Deployment, ChangeSource::Ci, "service", &target, summary);
    c.service = non_empty(&r.service);
    c.env = non_empty(&r.env);
    c.author = non_empty(&r.author);
    c.current = non_empty(&r.sha);
    for (k, v) in [("repository", r.repository), ("pipeline", r.pipeline), ("job", r.job), ("status", r.status)] {
        if !v.is_empty() {
            c.details.insert(k.into(), v);
        }
    }
    c
}

// ================================================================ events

/// The change an event describes: by its `change_type:` / `category:change` tags, else by keywords in
/// its title. None for events that are not changes (alerts, Agent restarts, …).
fn classify_event(title: &str, tags: &[String]) -> Option<ChangeType> {
    let by_name = |v: &str| match v.to_lowercase().as_str() {
        "deployment" | "deploy" | "release" => Some(ChangeType::Deployment),
        "rollout" => Some(ChangeType::Rollout),
        "feature_flag" | "feature-flag" | "flag" => Some(ChangeType::FeatureFlag),
        "scale" | "scaling" => Some(ChangeType::Scale),
        "config" | "configuration" => Some(ChangeType::Config),
        _ => None,
    };
    if let Some(t) = tag(tags, "change_type").and_then(by_name) {
        return Some(t);
    }
    let t = title.to_lowercase();
    let has = |words: &[&str]| words.iter().any(|w| t.contains(w));
    let keyword = if has(&["feature flag", "flag "]) {
        Some(ChangeType::FeatureFlag)
    } else if has(&["deploy", "release", "rollback", "rolled back"]) {
        Some(ChangeType::Deployment)
    } else if has(&["rollout", "rolled out", "rolling update"]) {
        Some(ChangeType::Rollout)
    } else if has(&["scaled", "scaling", "autoscal"]) {
        Some(ChangeType::Scale)
    } else if has(&["config", "setting"]) {
        Some(ChangeType::Config)
    } else {
        None
    };
    keyword.or_else(|| tag(tags, "category").filter(|c| c.eq_ignore_ascii_case("change")).map(|_| ChangeType::Config))
}

#[derive(clickhouse::Row, Deserialize, Debug)]
struct EventRow {
    id: String,
    ts: i64,
    title: String,
    text: String,
    source: String,
    host: String,
    tags: String,
}

fn event_change(e: EventRow) -> Option<Change> {
    let tags: Vec<String> = e.tags.split(',').map(str::trim).filter(|t| !t.is_empty()).map(str::to_string).collect();
    let change_type = classify_event(&e.title, &tags)?;
    let service = tag(&tags, "service").map(str::to_string);
    let (resource_type, resource) = match (&service, e.host.as_str()) {
        (Some(s), _) => ("service", s.clone()),
        (None, h) if !h.is_empty() => ("host", h.to_string()),
        _ => ("event", e.source.clone()),
    };
    let mut c = Change::new(format!("event:{}", e.id), e.ts, change_type, ChangeSource::Events, resource_type, &resource, e.title);
    c.service = service;
    c.env = tag(&tags, "env").map(str::to_string);
    c.cluster = tag(&tags, "kube_cluster_name").map(str::to_string);
    c.namespace = tag(&tags, "kube_namespace").map(str::to_string);
    c.author = tag(&tags, "user").or(tag(&tags, "author")).map(str::to_string);
    if !e.source.is_empty() {
        c.details.insert("event_source".into(), e.source);
    }
    if !e.text.is_empty() {
        c.details.insert("text".into(), e.text.chars().take(2000).collect());
    }
    Some(c)
}

// ================================================================ audit trail

/// Audit trail categories that are configuration changes.
const AUDIT_CATEGORIES: &[&str] = &["monitor", "downtime", "log_pipeline", "log_index", "log_metric", "log_archive", "reference_table", "standard_attribute", "notification_channel"];

#[derive(sqlx::FromRow, Debug)]
struct AuditRow {
    id: i64,
    actor: String,
    action: String,
    target: String,
    detail: String,
    created_at: i64,
}

fn audit_change(a: AuditRow) -> Option<Change> {
    let (category, verb) = a.action.split_once('.')?;
    if !AUDIT_CATEGORIES.contains(&category) || verb == "test_notification" {
        return None;
    }
    let what = category.replace('_', " ");
    let verb = verb.replace('_', " ");
    let resource = if a.detail.is_empty() { a.target.clone() } else { a.detail.clone() };
    let summary = format!("{} {} {}", capitalize(&what), past(&verb), resource);
    let mut c = Change::new(format!("audit:{}", a.id), a.created_at, ChangeType::Config, ChangeSource::Audit, category, &resource, summary);
    c.author = non_empty(&a.actor);
    c.details.insert("action".into(), a.action);
    if !a.target.is_empty() {
        c.details.insert("target".into(), a.target);
    }
    Some(c)
}

/// `create` → `created`, `updated` stays, `reorder` → `reordered`.
fn past(verb: &str) -> String {
    if verb.ends_with('d') {
        verb.to_string()
    } else if verb.ends_with('e') {
        format!("{verb}d")
    } else {
        format!("{verb}ed")
    }
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default()
}

// ================================================================ search

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ChangeSearchRequest {
    pub range: TimeRange,
    /// `change_type:rollout service:order-service env:prod cluster:shop-*` + free text.
    #[serde(default)]
    pub query: String,
    #[serde(default)]
    pub offset: usize,
    pub limit: usize,
}

/// Changes in one time bucket, by change type.
#[derive(Serialize, ToSchema)]
pub struct ChangeBucket {
    /// Bucket start, epoch ms.
    pub t: i64,
    pub counts: BTreeMap<String, u64>,
}

#[derive(Serialize, ToSchema)]
pub struct ChangePage {
    /// Newest first.
    pub items: Vec<Change>,
    pub total: usize,
    pub facets: Vec<CatalogFacet>,
    pub histogram: Vec<ChangeBucket>,
}

const FACETS: &[&str] = &["change_type", "source", "resource_type", "service", "env", "cluster", "namespace", "author"];

/// About 60 bars over the range, on a round step.
fn bucket_step(range: TimeRange) -> i64 {
    let span = (range.to - range.from).max(60_000);
    [60_000i64, 300_000, 600_000, 1_800_000, 3_600_000, 10_800_000, 21_600_000, 43_200_000, 86_400_000].into_iter().find(|s| span / s <= 90).unwrap_or(86_400_000)
}

fn search_page(all: Vec<Change>, req: &ChangeSearchRequest) -> ChangePage {
    let terms = parse_query(&req.query);
    let in_range: Vec<Change> = all.into_iter().filter(|c| c.timestamp >= req.range.from && c.timestamp <= req.range.to).collect();
    let facets = facets(&in_range, FACETS, &terms);
    let mut rows: Vec<Change> = in_range.into_iter().filter(|c| c.matches(&terms)).collect();
    rows.sort_by(|a, b| b.timestamp.cmp(&a.timestamp).then(a.id.cmp(&b.id)));
    let step = bucket_step(req.range);
    let start = req.range.from.div_euclid(step) * step;
    let mut histogram: Vec<ChangeBucket> = (0..).map(|i| start + i * step).take_while(|t| *t <= req.range.to).map(|t| ChangeBucket { t, counts: BTreeMap::new() }).collect();
    for c in &rows {
        if let Some(b) = histogram.get_mut(((c.timestamp - start) / step) as usize) {
            *b.counts.entry(c.change_type.as_str().to_string()).or_default() += 1;
        }
    }
    let total = rows.len();
    let items = rows.into_iter().skip(req.offset).take(req.limit.clamp(1, 500)).collect();
    ChangePage { items, total, facets, histogram }
}

/// Raw object snapshots are kept a day: look this far before the range for the generation a change
/// in the range replaced.
const SNAPSHOT_LOOKBACK_MS: i64 = 86_400_000;
const MAX_ROWS: u64 = 5000;

#[utoipa::path(post, path = "/api/v1/infrastructure/changes", operation_id = "search_changes", request_body = ChangeSearchRequest, responses((status = 200, body = ChangePage)), tag = "infrastructure")]
pub async fn search(State(st): State<AppState>, auth: AuthContext, Json(req): Json<ChangeSearchRequest>) -> ApiResult<ChangePage> {
    req.range.validate()?;
    let org = auth.org_id.as_str();
    let (from, to) = (req.range.from, req.range.to);
    let kube = super::kubernetes::resources(&st, org);
    let snapshots = st
        .ch(org)
        .query(
            "SELECT ResourceAttributes['k8s.cluster.name'] AS cluster, JSONExtractString(Body, 'kind') AS kind, JSONExtractString(Body, 'metadata', 'namespace') AS namespace, \
             JSONExtractString(Body, 'metadata', 'name') AS name, JSONExtractString(Body, 'metadata', 'uid') AS uid, JSONExtractInt(Body, 'metadata', 'generation') AS generation, \
             toInt64(toUnixTimestamp64Milli(min(Timestamp))) AS first_seen, any(JSONExtractRaw(Body, 'spec')) AS spec, any(JSONExtractRaw(Body, 'metadata', 'labels')) AS labels \
             FROM otel.otel_k8s_objects WHERE OrgId = {oc_org:String} AND TimestampTime >= toDateTime(intDiv(?, 1000)) AND TimestampTime <= toDateTime(intDiv(?, 1000)) \
             AND JSONExtractString(Body, 'kind') IN ('Deployment', 'StatefulSet', 'DaemonSet') AND uid != '' \
             GROUP BY cluster, kind, namespace, name, uid, generation ORDER BY uid, generation LIMIT ?",
        )
        .bind(from - SNAPSHOT_LOOKBACK_MS)
        .bind(to)
        .bind(MAX_ROWS * 4)
        .fetch_all::<Snapshot>();
    let kube_events = st
        .ch(org)
        .query(
            "SELECT Cluster AS cluster, toInt64(toUnixTimestamp64Milli(Timestamp)) AS ts, Reason AS reason, Note AS note, RegardingKind AS kind, \
             RegardingNamespace AS namespace, RegardingName AS name, RegardingUid AS uid, Count AS count \
             FROM otel.k8s_events WHERE OrgId = {oc_org:String} AND Timestamp >= fromUnixTimestamp64Milli(?) AND Timestamp <= fromUnixTimestamp64Milli(?) \
             AND Reason IN ? ORDER BY Timestamp DESC LIMIT ?",
        )
        .bind(from)
        .bind(to)
        .bind(CRASH_REASONS)
        .bind(MAX_ROWS)
        .fetch_all::<KubeEventRow>();
    let versions = st
        .ch(org)
        .query(
            "SELECT ServiceName AS service, ResourceAttributes['deployment.environment'] AS env, ResourceAttributes['service.version'] AS version, \
             toInt64(toUnixTimestamp64Milli(min(Timestamp))) AS first_seen \
             FROM otel.otel_traces WHERE OrgId = {oc_org:String} AND Timestamp >= fromUnixTimestamp64Milli(?) AND Timestamp <= fromUnixTimestamp64Milli(?) \
             AND SpanKind IN ('Server', 'Consumer') AND version != '' GROUP BY service, env, version LIMIT ?",
        )
        .bind(from)
        .bind(to)
        .bind(MAX_ROWS)
        .fetch_all::<VersionRow>();
    let ci = st
        .ch(org)
        .query(&format!(
            "SELECT toInt64(toUnixTimestamp64Milli(Timestamp)) AS ts, SpanAttributes['deployment.service.name'] AS service, SpanAttributes['deployment.environment.name'] AS env, \
             ResourceAttributes['vcs.repository.name'] AS repository, ResourceAttributes['vcs.ref.head.revision'] AS sha, ResourceAttributes['vcs.revision.author'] AS author, \
             ResourceAttributes['cicd.pipeline.name'] AS pipeline, SpanAttributes['cicd.pipeline.task.name'] AS job, {status} AS status \
             FROM otel.otel_ci_traces WHERE OrgId = {{oc_org:String}} AND Timestamp >= fromUnixTimestamp64Milli(?) AND Timestamp <= fromUnixTimestamp64Milli(?) \
             AND SpanAttributes['cicd.pipeline.task.name'] != '' AND SpanAttributes['deployment.environment.name'] != '' ORDER BY Timestamp DESC LIMIT ?",
            status = super::ci::STATUS
        ))
        .bind(from)
        .bind(to)
        .bind(MAX_ROWS)
        .fetch_all::<CiDeployRow>();
    let events = st
        .ch(org)
        .query(
            "SELECT toString(cityHash64(Timestamp, ResourceAttributes['host.name'], LogAttributes['title'], Body)) AS id, toInt64(toUnixTimestamp64Milli(Timestamp)) AS ts, \
             LogAttributes['title'] AS title, Body AS text, LogAttributes['source_type_name'] AS source, ResourceAttributes['host.name'] AS host, LogAttributes['tags'] AS tags \
             FROM otel.otel_events WHERE OrgId = {oc_org:String} AND Timestamp >= fromUnixTimestamp64Milli(?) AND Timestamp <= fromUnixTimestamp64Milli(?) \
             ORDER BY Timestamp DESC LIMIT ?",
        )
        .bind(from)
        .bind(to)
        .bind(MAX_ROWS)
        .fetch_all::<EventRow>();
    // The audit trail is for organization admins.
    let audit = async {
        if auth.access < Access::Admin {
            return Ok(vec![]);
        }
        sqlx::query_as::<_, AuditRow>(
            "SELECT id, actor, action, target, detail, (extract(epoch FROM created_at) * 1000)::bigint AS created_at FROM audit_events \
             WHERE org_id = $1 AND created_at >= to_timestamp($2::bigint / 1000.0) AND created_at <= to_timestamp($3::bigint / 1000.0) \
             AND split_part(action, '.', 1) = ANY($4) ORDER BY created_at DESC LIMIT $5",
        )
        .bind(org)
        .bind(from)
        .bind(to)
        .bind(AUDIT_CATEGORIES)
        .bind(MAX_ROWS as i64)
        .fetch_all(&st.postgres)
        .await
    };
    let (kube, snapshots, kube_events, versions, ci, events, audit) = tokio::try_join!(
        kube,
        async { snapshots.await.map_err(ApiError::from) },
        async { kube_events.await.map_err(ApiError::from) },
        async { versions.await.map_err(ApiError::from) },
        async { ci.await.map_err(ApiError::from) },
        async { events.await.map_err(ApiError::from) },
        async { audit.await.map_err(ApiError::from) }
    )?;
    let mut all = replicaset_rollouts(&kube, req.range);
    all.extend(spec_changes(snapshots, req.range));
    all.extend(kube_events.into_iter().filter_map(kube_event_change));
    all.extend(version_deployments(versions, req.range));
    all.extend(ci.into_iter().map(ci_deployment));
    all.extend(events.into_iter().filter_map(event_change));
    all.extend(audit.into_iter().filter_map(audit_change));
    Ok(Json(search_page(all, &req)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::routes::kubernetes::KubeObjectRef;

    const RANGE: TimeRange = TimeRange { from: 1_000_000, to: 2_000_000 };

    fn rs(name: &str, revision: i64, created: i64, image: &str) -> KubeResource {
        KubeResource {
            uid: format!("uid-{name}"),
            kind: Some(KubeKind::ReplicaSets),
            cluster: "shop-prod".into(),
            namespace: Some("shop".into()),
            name: name.into(),
            created_at: Some(created),
            annotations: BTreeMap::from([("deployment.kubernetes.io/revision".to_string(), revision.to_string())]),
            owner: Some(KubeObjectRef { kind: "Deployment".into(), name: "order-service".into(), uid: "dep-1".into() }),
            images: vec![image.into()],
            tags: vec!["service:order-service".into(), "env:prod".into()],
            ..Default::default()
        }
    }

    #[test]
    fn a_new_replica_set_in_range_is_a_rollout_against_the_previous_revision() {
        let rows = vec![
            rs("order-service-a", 1, 10, "registry.example.com/order-service:1.4.0"),
            rs("order-service-b", 2, 500, "registry.example.com/order-service:1.4.1"),
            rs("order-service-c", 3, 1_500_000, "registry.example.com/order-service:1.4.2"),
        ];
        let changes = replicaset_rollouts(&rows, RANGE);
        assert_eq!(changes.len(), 1, "only revision 3 was created in the range");
        let c = &changes[0];
        assert_eq!((c.change_type, c.resource.as_str(), c.timestamp), (ChangeType::Rollout, "order-service", 1_500_000));
        assert_eq!(c.summary, "Rolled out order-service: 1.4.1 → 1.4.2");
        assert_eq!(c.previous.as_deref(), Some("registry.example.com/order-service:1.4.1"));
        assert_eq!(c.current.as_deref(), Some("registry.example.com/order-service:1.4.2"));
        assert_eq!((c.service.as_deref(), c.env.as_deref(), c.cluster.as_deref()), (Some("order-service"), Some("prod"), Some("shop-prod")));
        assert_eq!((c.kube_kind, c.kube_uid.as_deref()), (Some(KubeKind::Deployments), Some("dep-1")));
        assert_eq!(c.details.get("revision").map(String::as_str), Some("3"));
        // A deployment's first replica set is its creation.
        let created = replicaset_rollouts(&[rs("order-service-a", 1, 1_200_000, "registry.example.com/order-service:1.4.0")], RANGE);
        assert_eq!(created[0].summary, "Created with order-service:1.4.0");
    }

    fn snap(kind: &str, generation: i64, first_seen: i64, spec: Value) -> Snapshot {
        Snapshot {
            cluster: "shop-prod".into(),
            kind: kind.into(),
            namespace: "shop".into(),
            name: "cart-db".into(),
            uid: format!("uid-{kind}"),
            generation,
            first_seen,
            spec: spec.to_string(),
            labels: r#"{"tags.datadoghq.com/service":"cart-db","tags.datadoghq.com/env":"prod"}"#.into(),
        }
    }

    fn workload(replicas: i64, image: &str) -> Value {
        serde_json::json!({ "replicas": replicas, "template": { "spec": { "containers": [{ "name": "db", "image": image }] } } })
    }

    #[test]
    fn generations_become_scale_rollout_and_config_changes() {
        let snaps = vec![
            snap("StatefulSet", 1, 10, workload(1, "postgres:16.3")),
            snap("StatefulSet", 2, 1_100_000, workload(3, "postgres:16.3")),
            snap("StatefulSet", 3, 1_200_000, workload(3, "postgres:16.4")),
            snap("Deployment", 4, 1_300_000, workload(2, "registry.example.com/billing:2.0.0")),
            snap("Deployment", 5, 1_400_000, workload(2, "registry.example.com/billing:2.1.0")),
        ];
        let changes = spec_changes(snaps, RANGE);
        let summaries: Vec<(ChangeType, &str)> = changes.iter().map(|c| (c.change_type, c.summary.as_str())).collect();
        assert!(summaries.contains(&(ChangeType::Scale, "Scaled from 1 to 3 replicas")));
        assert!(summaries.contains(&(ChangeType::Rollout, "Rolled out postgres: 16.3 → 16.4")));
        // Deployment template changes come from replica sets instead.
        assert_eq!(changes.len(), 2);
        let scale = changes.iter().find(|c| c.change_type == ChangeType::Scale).unwrap();
        assert_eq!((scale.previous.as_deref(), scale.current.as_deref()), (Some("1"), Some("3")));
        assert_eq!((scale.service.as_deref(), scale.resource_type.as_str()), (Some("cart-db"), "kube_statefulset"));

        let strategy = |s: &str| serde_json::json!({ "replicas": 2, "strategy": { "type": s }, "template": {} });
        let edits = spec_changes(vec![snap("Deployment", 1, 1_100_000, strategy("RollingUpdate")), snap("Deployment", 2, 1_200_000, strategy("Recreate"))], RANGE);
        assert_eq!(edits.iter().map(|c| (c.change_type, c.summary.as_str())).collect::<Vec<_>>(), vec![(ChangeType::Config, "Spec changed: strategy")]);
        // A generation before the range is only the baseline.
        assert!(spec_changes(vec![snap("Deployment", 1, 10, strategy("A")), snap("Deployment", 2, 20, strategy("B"))], RANGE).is_empty());
    }

    #[test]
    fn crash_events_only() {
        let ev = |reason: &str, note: &str| KubeEventRow {
            cluster: "shop-prod".into(),
            ts: 1_500_000,
            reason: reason.into(),
            note: note.into(),
            kind: "Pod".into(),
            namespace: "shop".into(),
            name: "cart-7d9f-abcde".into(),
            uid: "pod-1".into(),
            count: 4,
        };
        let c = kube_event_change(ev("BackOff", "Back-off restarting failed container app in pod cart-7d9f-abcde")).unwrap();
        assert_eq!((c.change_type, c.resource_type.as_str(), c.kube_kind), (ChangeType::Crash, "kube_pod", Some(KubeKind::Pods)));
        assert!(c.summary.starts_with("Crash looping (4×)"));
        // Image pull back-offs are not crashes; other reasons are not changes.
        assert!(kube_event_change(ev("BackOff", "Back-off pulling image")).is_none());
        assert!(kube_event_change(ev("Pulled", "Successfully pulled image")).is_none());
        assert!(kube_event_change(ev("OOMKilling", "Memory cgroup out of memory")).is_some());
    }

    #[test]
    fn new_versions_in_the_range_are_deployments() {
        let v = |version: &str, first_seen: i64| VersionRow { service: "storefront-api".into(), env: "prod".into(), version: version.into(), first_seen };
        let changes = version_deployments(vec![v("1.4.1", RANGE.from), v("1.4.2", 1_500_000)], RANGE);
        assert_eq!(changes.len(), 1, "the version running when the range starts is not a change");
        assert_eq!(changes[0].summary, "Deployed version 1.4.2 (was 1.4.1)");
        assert_eq!((changes[0].previous.as_deref(), changes[0].current.as_deref()), (Some("1.4.1"), Some("1.4.2")));
    }

    #[test]
    fn ci_deployments_carry_author_and_commit() {
        let c = ci_deployment(CiDeployRow {
            ts: 1_500_000,
            service: "billing".into(),
            env: "production".into(),
            repository: "example-org/billing".into(),
            sha: "0123456789abcdef".into(),
            author: "dev@example.com".into(),
            pipeline: "deploy".into(),
            job: "deploy-production".into(),
            status: "success".into(),
        });
        assert_eq!(c.summary, "deploy deployed billing at 01234567 to production");
        assert_eq!((c.author.as_deref(), c.change_type, c.source), (Some("dev@example.com"), ChangeType::Deployment, ChangeSource::Ci));
    }

    #[test]
    fn events_are_changes_by_tag_or_title() {
        let tags = |t: &[&str]| t.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(classify_event("Something happened", &tags(&["change_type:feature_flag"])), Some(ChangeType::FeatureFlag));
        assert_eq!(classify_event("Deploy order-service v1.4.2", &[]), Some(ChangeType::Deployment));
        assert_eq!(classify_event("Updated nginx config", &[]), Some(ChangeType::Config));
        assert_eq!(classify_event("Edited", &tags(&["category:change"])), Some(ChangeType::Config));
        assert_eq!(classify_event("Datadog Agent started", &[]), None);
        let e = event_change(EventRow {
            id: "42".into(),
            ts: 1_500_000,
            title: "Deploy order-service v1.4.2".into(),
            text: "".into(),
            source: "api".into(),
            host: "".into(),
            tags: "env:prod,service:order-service,user:dev@example.com".into(),
        })
        .unwrap();
        assert_eq!((e.resource.as_str(), e.env.as_deref(), e.author.as_deref()), ("order-service", Some("prod"), Some("dev@example.com")));
    }

    #[test]
    fn audit_config_changes_only() {
        let a = |action: &str| AuditRow { id: 7, actor: "Alex Morgan".into(), action: action.into(), target: "monitor:12".into(), detail: "High error rate".into(), created_at: 1_500_000 };
        let c = audit_change(a("monitor.update")).unwrap();
        assert_eq!((c.change_type, c.resource_type.as_str(), c.author.as_deref()), (ChangeType::Config, "monitor", Some("Alex Morgan")));
        assert_eq!(c.summary, "Monitor updated High error rate");
        assert_eq!(audit_change(a("log_pipeline.created")).unwrap().summary, "Log pipeline created High error rate");
        assert!(audit_change(a("login.succeeded")).is_none());
        assert!(audit_change(a("monitor.test_notification")).is_none());
        assert_eq!(past("cancel"), "canceled");
    }

    #[test]
    fn search_filters_facets_and_buckets_by_type() {
        let mk = |id: &str, ts: i64, t: ChangeType, service: &str| {
            let mut c = Change::new(id.into(), ts, t, ChangeSource::Kubernetes, "kube_deployment", service, format!("{service} changed"));
            c.service = Some(service.into());
            c
        };
        let all = vec![
            mk("a", 1_100_000, ChangeType::Rollout, "billing"),
            mk("b", 1_900_000, ChangeType::Scale, "billing"),
            mk("c", 1_500_000, ChangeType::Rollout, "catalog-service"),
            mk("old", 10, ChangeType::Rollout, "billing"),
        ];
        let req = |q: &str| ChangeSearchRequest { range: RANGE, query: q.into(), offset: 0, limit: 50 };
        let page = search_page(all.clone(), &req("service:billing"));
        assert_eq!(page.items.iter().map(|c| c.id.as_str()).collect::<Vec<_>>(), vec!["b", "a"], "newest first, range applied");
        let types = page.facets.iter().find(|f| f.key == "change_type").unwrap();
        assert_eq!(types.values.iter().map(|v| (v.value.as_str(), v.count)).collect::<Vec<_>>(), vec![("rollout", 1), ("scale", 1)]);
        let bucketed: u64 = page.histogram.iter().flat_map(|b| b.counts.values()).sum();
        assert_eq!(bucketed, 2);
        assert!(page.histogram.windows(2).all(|w| w[1].t - w[0].t == bucket_step(RANGE)));
        assert_eq!(search_page(all, &req("catalog")).total, 1);
    }
}
