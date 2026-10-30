//! Infrastructure → Resources → Changes: what changed in the configuration of the catalog's resources.
//!
//! - The change job ([`run`]) compares, every few minutes per org, each resource of the catalog
//!   ([`super::catalog::load_all`]: hosts, Kubernetes objects, ECS) with its last snapshot
//!   (Postgres `resource_snapshots`). A snapshot is the resource's normalized configuration: its
//!   catalog fields, tags and attributes, and for Kubernetes objects their labels, annotations and
//!   spec (status, managed fields and other bookkeeping left out). New resources, deleted ones and
//!   changed fields become rows of ClickHouse `otel.resource_changes`, classified ([`classify`]) by
//!   change type, risk and noise (ephemeral pods and jobs coming and going).
//! - An org's first run only records the baseline. A run in which most of an org's resources vanish
//!   at once (a collector outage, not a mass deletion) records no deletions.
//! - Orgs are claimed with `FOR UPDATE SKIP LOCKED`, so several servers can run the job.
//!
//! Tenancy: Postgres queries filter on `org_id`; ClickHouse ones on `OrgId = {oc_org:String}`
//! through `AppState::ch(org)`.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::time::Duration;

use axum::extract::State;
use axum::Json;
use oc_query::{parse_query, Term};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use utoipa::ToSchema;

use super::catalog::{self, enum_str_static, CatalogFacet, CatalogResource, Faceted};
use super::logs::TimeRange;
use crate::error::{ApiError, ApiResult};
use crate::state::{AppState, OrgId};

const RUN_INTERVAL_MINUTES: i32 = 5;
const CLAIM_SECONDS: i32 = 300;
/// Changes a search reads at most (newest first); the facets and the histogram count these.
const MAX_ROWS: u64 = 50_000;
const MAX_PAGE: usize = 200;
/// Deletions are skipped when more than this share of an org's resources vanish in one run.
const MAX_DELETED_SHARE: f64 = 0.5;
/// Attributes of the catalog that change on their own (readiness, counts): not configuration.
const VOLATILE_ATTRIBUTES: &[&str] = &["ready", "restarts", "pods", "nodes", "tasks"];
/// Annotations Kubernetes tools keep up to date themselves.
const BOOKKEEPING_ANNOTATIONS: &[&str] = &["kubectl.kubernetes.io/last-applied-configuration", "deployment.kubernetes.io/revision", "autoscaling.alpha.kubernetes.io/conditions"];
/// Header filters of the page, in display order.
const FACETS: &[&str] = &["team", "risk", "cloud", "region", "env", "account", "service", "change_type", "resource_type"];
/// Resource types whose creation and deletion are routine (controllers replace them): noise.
const EPHEMERAL: &[&str] = &["kube_pod", "kube_job", "container", "ecs_task"];
/// Resource types whose deletion takes something users depend on away.
const CRITICAL_ON_DELETE: &[&str] = &[
    "host",
    "kube_cluster",
    "kube_node",
    "kube_namespace",
    "kube_deployment",
    "kube_statefulset",
    "kube_daemonset",
    "kube_service",
    "kube_ingress",
    "kube_persistent_volume_claim",
    "kube_persistent_volume",
    "ecs_cluster",
    "ecs_service",
];

// ================================================================ snapshots and diffs

/// A Kubernetes object's configuration: its labels, annotations and spec.
fn normalize_manifest(body: &Value) -> Value {
    let mut metadata = Map::new();
    if let Some(labels) = body.pointer("/metadata/labels").filter(|v| v.as_object().is_some_and(|m| !m.is_empty())) {
        metadata.insert("labels".into(), labels.clone());
    }
    if let Some(Value::Object(annotations)) = body.pointer("/metadata/annotations") {
        let kept: Map<String, Value> = annotations.iter().filter(|(k, _)| !BOOKKEEPING_ANNOTATIONS.contains(&k.as_str())).map(|(k, v)| (k.clone(), v.clone())).collect();
        if !kept.is_empty() {
            metadata.insert("annotations".into(), Value::Object(kept));
        }
    }
    let mut out = Map::new();
    if !metadata.is_empty() {
        out.insert("metadata".into(), Value::Object(metadata));
    }
    if let Some(spec) = body.get("spec") {
        out.insert("spec".into(), spec.clone());
    }
    Value::Object(out)
}

/// The configuration of a resource that a snapshot keeps (and that changes are reported on).
fn resource_doc(r: &CatalogResource, manifest: Option<&Value>) -> Value {
    let mut tags = r.tags.clone();
    tags.sort();
    let attributes: Map<String, Value> = r.attributes.iter().filter(|(k, _)| !VOLATILE_ATTRIBUTES.contains(&k.as_str())).map(|(k, v)| (k.clone(), Value::from(v.as_str()))).collect();
    let mut doc = json!({
        "resource_type": enum_str_static(r.resource_type),
        "name": r.name,
        "cloud": r.cloud,
        "account": r.account,
        "region": r.region,
        "cluster": r.cluster,
        "namespace": r.namespace,
        "env": r.env,
        "service": r.service,
        "team": r.team,
        "tags": tags,
        "attributes": attributes,
    });
    if let Some(m) = manifest {
        doc["manifest"] = normalize_manifest(m);
    }
    // Fields the resource doesn't have are left out, not null.
    if let Value::Object(fields) = &mut doc {
        fields.retain(|_, v| !v.is_null());
    }
    doc
}

/// Leaf values by path (`manifest.spec.replicas`); arrays of named objects by name
/// (`containers[api].image`), other arrays as one value.
fn flatten(prefix: &str, v: &Value, out: &mut BTreeMap<String, Value>) {
    let join = |k: &str| if prefix.is_empty() { k.to_string() } else { format!("{prefix}.{k}") };
    match v {
        Value::Object(m) if !m.is_empty() => {
            for (k, v) in m {
                flatten(&join(k), v, out);
            }
        }
        Value::Array(items) if !items.is_empty() && items.iter().all(|i| i.get("name").and_then(Value::as_str).is_some()) => {
            for i in items {
                let name = i["name"].as_str().unwrap_or_default();
                flatten(&format!("{prefix}[{name}]"), i, out);
            }
        }
        Value::Null => {}
        _ => {
            out.insert(prefix.to_string(), v.clone());
        }
    }
}

/// The paths whose values differ between two configurations.
pub(crate) fn changed_fields(old: &Value, new: &Value) -> Vec<String> {
    let (mut a, mut b) = (BTreeMap::new(), BTreeMap::new());
    flatten("", old, &mut a);
    flatten("", new, &mut b);
    let keys: std::collections::BTreeSet<&String> = a.keys().chain(b.keys()).collect();
    keys.into_iter().filter(|k| a.get(*k) != b.get(*k)).cloned().collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Event {
    Created,
    Deleted,
    Changed,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Classified {
    pub change_types: Vec<String>,
    pub risk: &'static str,
    pub title: String,
    pub description: String,
    pub noise: bool,
}

/// The change type of a changed field.
fn field_type(path: &str) -> &'static str {
    let p = path.to_ascii_lowercase();
    if p.starts_with("tags") || p.starts_with("manifest.metadata.") || matches!(p.as_str(), "team" | "service" | "env") {
        "tags"
    } else if p.contains("securitycontext") || p.contains("serviceaccount") || p.contains("privileged") || p.contains("capabilities") || p.contains("hostnetwork") || p.contains("hostpid") {
        "security"
    } else if p.ends_with("replicas") || p.contains(".replicas") {
        "scaling"
    } else if p.contains(".image") || p.starts_with("attributes.images") || p.starts_with("attributes.image_tags") || p.starts_with("attributes.task_definition") {
        "image"
    } else if p.contains(".ports") || p.contains("spec.rules") || p.contains("spec.tls") || p.contains("loadbalancer") || p == "attributes.service_type" || p == "manifest.spec.type" {
        "network"
    } else {
        "config"
    }
}

/// "kube_deployment" → "Kubernetes Deployment".
pub(crate) fn type_label(resource_type: &str) -> String {
    let (prefix, rest) = match resource_type.split_once('_') {
        Some(("kube", rest)) => ("Kubernetes ", rest),
        Some(("ecs", rest)) => ("ECS ", rest),
        _ => ("", resource_type),
    };
    let words: Vec<String> = rest.split('_').map(|w| { let mut c = w.chars(); c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default() }).collect();
    format!("{prefix}{}", words.join(" "))
}

fn scalar(doc: &Value, path: &str) -> Option<String> {
    let mut flat = BTreeMap::new();
    flatten("", doc, &mut flat);
    flat.get(path).map(|v| v.as_str().map(str::to_string).unwrap_or_else(|| v.to_string()))
}

/// Change types, risk, a one-line title and what the change can do.
pub(crate) fn classify(resource_type: &str, event: Event, fields: &[String], old: &Value, new: &Value) -> Classified {
    let label = type_label(resource_type);
    let ephemeral = EPHEMERAL.contains(&resource_type);
    match event {
        Event::Created => Classified {
            change_types: vec!["creation".into()],
            risk: "low",
            title: format!("{label} created"),
            description: format!("A new {} appeared in your infrastructure.", label.to_lowercase()),
            noise: ephemeral,
        },
        Event::Deleted => {
            let critical = CRITICAL_ON_DELETE.contains(&resource_type);
            Classified {
                change_types: vec!["config".into(), "deletion".into()],
                risk: if critical { "high" } else if ephemeral { "low" } else { "medium" },
                title: format!("{label} deleted"),
                description: if critical {
                    format!("Deleting a {} removes it for everything that depends on it: workloads, traffic or data it served stop being available until it's recreated.", label.to_lowercase())
                } else {
                    format!("The {} is gone. If something still references it, it can no longer start or update the workloads that use it.", label.to_lowercase())
                },
                noise: ephemeral,
            }
        }
        Event::Changed => {
            let mut types: Vec<&str> = fields.iter().map(|f| field_type(f)).collect::<HashSet<_>>().into_iter().collect();
            types.sort_by_key(|t| ["security", "scaling", "image", "network", "config", "tags"].iter().position(|x| x == t));
            let primary = types.first().copied().unwrap_or("config");
            let replicas = |doc: &Value| scalar(doc, "manifest.spec.replicas").or_else(|| scalar(doc, "attributes.replicas"));
            let (risk, title, description) = match primary {
                "security" => ("high", "Security settings changed".to_string(), "Changes to privileges, capabilities or service accounts widen or narrow what the workload can access.".to_string()),
                "scaling" => {
                    let (before, after) = (replicas(old).unwrap_or_else(|| "?".into()), replicas(new).unwrap_or_else(|| "?".into()));
                    let to_zero = after == "0";
                    (
                        if to_zero { "high" } else { "low" },
                        format!("Replicas changed from {before} to {after}"),
                        if to_zero { "Scaled to zero: the workload no longer serves anything.".to_string() } else { "The number of running copies changed: capacity and cost follow.".to_string() },
                    )
                }
                "image" => {
                    let image = scalar(new, "attributes.images").or_else(|| scalar(new, "attributes.task_definition")).unwrap_or_default();
                    (
                        "medium",
                        if image.is_empty() { "Image changed".to_string() } else { format!("Image changed to {image}") },
                        "A new image rolls out new code: if it misbehaves, roll back to the previous image.".to_string(),
                    )
                }
                "network" => ("medium", "Network settings changed".to_string(), "Ports, routing or exposure changed: clients may reach the workload differently, or not at all.".to_string()),
                "tags" => ("low", "Tags changed".to_string(), "Labels and tags changed: ownership, monitors and dashboards that select on them may pick the resource up differently.".to_string()),
                _ => (
                    "medium",
                    format!("Configuration changed ({} field{})", fields.len(), if fields.len() == 1 { "" } else { "s" }),
                    "Its configuration changed: compare the fields before and after.".to_string(),
                ),
            };
            Classified { change_types: types.into_iter().map(str::to_string).collect(), risk, title, description, noise: ephemeral && primary == "tags" }
        }
    }
}

// ================================================================ the change job

#[derive(Serialize)]
struct ChangeRow<'a> {
    #[serde(rename = "OrgId")]
    org: &'a str,
    #[serde(rename = "Detected")]
    detected: i64,
    #[serde(rename = "ResourceId")]
    resource_id: &'a str,
    #[serde(rename = "ResourceType")]
    resource_type: String,
    #[serde(rename = "ResourceName")]
    resource_name: String,
    #[serde(rename = "Cloud")]
    cloud: String,
    #[serde(rename = "Account")]
    account: String,
    #[serde(rename = "Region")]
    region: String,
    #[serde(rename = "Cluster")]
    cluster: String,
    #[serde(rename = "Namespace")]
    namespace: String,
    #[serde(rename = "Env")]
    env: String,
    #[serde(rename = "Service")]
    service: String,
    #[serde(rename = "Team")]
    team: String,
    #[serde(rename = "ChangeTypes")]
    change_types: Vec<String>,
    #[serde(rename = "ChangedFields")]
    changed_fields: Vec<String>,
    #[serde(rename = "Risk")]
    risk: &'static str,
    #[serde(rename = "Title")]
    title: String,
    #[serde(rename = "Description")]
    description: String,
    #[serde(rename = "IsNoise")]
    is_noise: bool,
    #[serde(rename = "OldFields")]
    old_fields: String,
    #[serde(rename = "NewFields")]
    new_fields: String,
}

/// A change row from the snapshot before (null: created) and after (null: deleted).
fn change_row<'a>(org: &'a str, now: i64, id: &'a str, old: &Value, new: &Value) -> Option<ChangeRow<'a>> {
    let (event, doc) = match (old.is_null(), new.is_null()) {
        (true, false) => (Event::Created, new),
        (false, true) => (Event::Deleted, old),
        (false, false) => (Event::Changed, new),
        (true, true) => return None,
    };
    let fields = if event == Event::Changed { changed_fields(old, new) } else { Vec::new() };
    if event == Event::Changed && fields.is_empty() {
        return None;
    }
    let s = |k: &str| doc.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
    let resource_type = s("resource_type");
    let c = classify(&resource_type, event, &fields, old, new);
    let fields = if event == Event::Changed {
        fields
    } else {
        let mut all = BTreeMap::new();
        flatten("", doc, &mut all);
        all.into_keys().collect()
    };
    let text = |v: &Value| if v.is_null() { String::new() } else { v.to_string() };
    Some(ChangeRow {
        org,
        detected: now,
        resource_id: id,
        resource_name: s("name"),
        cloud: s("cloud"),
        account: s("account"),
        region: s("region"),
        cluster: s("cluster"),
        namespace: s("namespace"),
        env: s("env"),
        service: s("service"),
        team: s("team"),
        resource_type,
        change_types: c.change_types,
        changed_fields: fields,
        risk: c.risk,
        title: c.title,
        description: c.description,
        is_noise: c.noise,
        old_fields: text(old),
        new_fields: text(new),
    })
}

fn hash(doc: &Value) -> String {
    crate::auth::crypto::sha256(&doc.to_string()).iter().map(|b| format!("{b:02x}")).collect()
}

/// One run for an org: compares the catalog with the snapshots, records the changes and the new snapshots.
async fn run_org(st: &AppState, org: &str) -> Result<usize, ApiError> {
    let resources = catalog::load_all(st, org).await?;
    #[derive(clickhouse::Row, Deserialize)]
    struct Body {
        uid: String,
        body: String,
    }
    let bodies: HashMap<String, Value> = st
        .ch(org)
        .query("SELECT Uid AS uid, Body AS body FROM otel.k8s_resources WHERE OrgId = {oc_org:String} AND LastSeen > now() - INTERVAL 15 MINUTE ORDER BY LastSeen DESC LIMIT 1 BY Uid")
        .fetch_all::<Body>()
        .await?
        .into_iter()
        .filter_map(|b| serde_json::from_str(&b.body).ok().map(|v| (b.uid, v)))
        .collect();
    let current: Vec<(String, Value, String)> = resources
        .iter()
        .map(|r| {
            let doc = resource_doc(r, bodies.get(&r.id));
            let h = hash(&doc);
            (r.id.clone(), doc, h)
        })
        .collect();
    let (baselined,): (bool,) = sqlx::query_as("SELECT baselined FROM resource_change_runs WHERE org_id = $1").bind(org).fetch_one(&st.postgres).await?;
    let known: Vec<(String, String, bool)> = sqlx::query_as("SELECT resource_id, hash, deleted FROM resource_snapshots WHERE org_id = $1").bind(org).fetch_all(&st.postgres).await?;
    let known: HashMap<String, (String, bool)> = known.into_iter().map(|(id, h, d)| (id, (h, d))).collect();
    let present: HashSet<&str> = current.iter().map(|(id, _, _)| id.as_str()).collect();
    let changed: Vec<&(String, Value, String)> = current.iter().filter(|(id, _, h)| known.get(id).is_none_or(|(kh, deleted)| kh != h || *deleted)).collect();
    let alive = known.values().filter(|(_, d)| !d).count();
    let mut gone: Vec<&String> = known.iter().filter(|(id, (_, d))| !d && !present.contains(id.as_str())).map(|(id, _)| id).collect();
    if alive > 0 && gone.len() as f64 > alive as f64 * MAX_DELETED_SHARE && gone.len() > 20 {
        tracing::warn!(org = %org, gone = gone.len(), alive, "most resources vanished at once; recording no deletions this run");
        gone.clear();
    }
    // The configurations before, of the resources that changed or went away.
    let need: Vec<String> = changed.iter().filter(|(id, _, _)| known.get(id).is_some_and(|(_, d)| !d)).map(|(id, _, _)| id.clone()).chain(gone.iter().map(|id| id.to_string())).collect();
    let before: HashMap<String, Value> = if need.is_empty() {
        HashMap::new()
    } else {
        sqlx::query_as::<_, (String, Value)>("SELECT resource_id, doc FROM resource_snapshots WHERE org_id = $1 AND resource_id = ANY($2)").bind(org).bind(&need).fetch_all(&st.postgres).await?.into_iter().collect()
    };
    let now = super::metrics::now_ms();
    let mut rows: Vec<ChangeRow> = Vec::new();
    if baselined {
        for (id, doc, _) in &changed {
            if let Some(row) = change_row(org, now, id, before.get(id).unwrap_or(&Value::Null), doc) {
                rows.push(row);
            }
        }
        for id in &gone {
            if let Some(row) = change_row(org, now, id, before.get(*id).unwrap_or(&Value::Null), &Value::Null) {
                rows.push(row);
            }
        }
    }
    if !rows.is_empty() {
        let mut ndjson = Vec::new();
        for r in &rows {
            serde_json::to_writer(&mut ndjson, r).map_err(anyhow::Error::from)?;
            ndjson.push(b'\n');
        }
        crate::logproc::insert_rows(st, "otel.resource_changes", ndjson).await.map_err(|e| ApiError::Internal(anyhow::anyhow!("recording resource changes failed: {e}")))?;
    }
    let mut tx = st.postgres.begin().await?;
    if !changed.is_empty() {
        let upserts = Value::Array(changed.iter().map(|(id, doc, h)| json!({"id": id, "hash": h, "doc": doc})).collect());
        sqlx::query(
            "INSERT INTO resource_snapshots (org_id, resource_id, hash, doc, deleted, seen_at) \
             SELECT $1, e->>'id', e->>'hash', e->'doc', false, now() FROM jsonb_array_elements($2::jsonb) AS e \
             ON CONFLICT (org_id, resource_id) DO UPDATE SET hash = EXCLUDED.hash, doc = EXCLUDED.doc, deleted = false, seen_at = now()",
        )
        .bind(org)
        .bind(upserts)
        .execute(&mut *tx)
        .await?;
    }
    if !gone.is_empty() {
        let ids: Vec<&str> = gone.iter().map(|s| s.as_str()).collect();
        sqlx::query("UPDATE resource_snapshots SET deleted = true, seen_at = now() WHERE org_id = $1 AND resource_id = ANY($2)").bind(org).bind(&ids).execute(&mut *tx).await?;
    }
    // Snapshots of resources deleted more than a month ago are no longer needed.
    sqlx::query("DELETE FROM resource_snapshots WHERE org_id = $1 AND deleted AND seen_at < now() - interval '30 days'").bind(org).execute(&mut *tx).await?;
    sqlx::query("UPDATE resource_change_runs SET baselined = true WHERE org_id = $1").bind(org).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(rows.len())
}

/// Claims up to `limit` active orgs due for a run (adding the orgs that have no row yet).
async fn claim(st: &AppState, limit: i64) -> Result<Vec<String>, ApiError> {
    sqlx::query("INSERT INTO resource_change_runs (org_id) SELECT id FROM orgs WHERE status = 'active' ON CONFLICT DO NOTHING").execute(&st.postgres).await?;
    let mut tx = st.postgres.begin().await?;
    let orgs: Vec<(String,)> = sqlx::query_as(
        "SELECT r.org_id FROM resource_change_runs r JOIN orgs o ON o.id = r.org_id WHERE r.next_run_at <= now() AND (r.claimed_until IS NULL OR r.claimed_until < now()) \
         AND o.status = 'active' ORDER BY r.next_run_at LIMIT $1 FOR UPDATE OF r SKIP LOCKED",
    )
    .bind(limit)
    .fetch_all(&mut *tx)
    .await?;
    for (org,) in &orgs {
        sqlx::query("UPDATE resource_change_runs SET claimed_until = now() + make_interval(secs => $2), next_run_at = now() + make_interval(mins => $3) WHERE org_id = $1")
            .bind(org)
            .bind(CLAIM_SECONDS)
            .bind(RUN_INTERVAL_MINUTES)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok(orgs.into_iter().map(|(o,)| o).collect())
}

/// Detects resource changes of every active org, every few minutes.
pub async fn run(st: AppState) {
    let mut tick = tokio::time::interval(Duration::from_secs(60));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tick.tick().await;
        let due = match claim(&st, 10).await {
            Ok(due) => due,
            Err(e) => {
                tracing::warn!(error = %e, "claiming resource change runs failed");
                continue;
            }
        };
        for org in due {
            if let Err(e) = run_org(&st, &org).await {
                tracing::warn!(org = %org, error = %e, "detecting resource changes failed");
            }
            if let Err(e) = sqlx::query("UPDATE resource_change_runs SET claimed_until = NULL WHERE org_id = $1").bind(&org).execute(&st.postgres).await {
                tracing::warn!(org = %org, error = %e, "releasing the resource change claim failed");
            }
        }
    }
}

// ================================================================ search

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ResourceChangeSearchRequest {
    pub range: TimeRange,
    /// `team:checkout risk:high change_type:deletion @is_noise:false resource_id:…`, free text on the name and title.
    #[serde(default)]
    pub query: String,
    /// What the histogram stacks by: resource_type (default), risk, change_type, team, …
    #[serde(default)]
    pub group_by: Option<String>,
    #[serde(default)]
    pub offset: usize,
    #[serde(default = "default_limit")]
    pub limit: usize,
}

fn default_limit() -> usize {
    50
}

#[derive(Serialize, ToSchema, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ResourceChange {
    /// Epoch ms.
    pub detected: i64,
    pub resource_id: String,
    pub resource_type: String,
    pub resource_name: String,
    pub cloud: String,
    pub account: String,
    pub region: String,
    pub cluster: String,
    pub namespace: String,
    pub env: String,
    pub service: String,
    pub team: String,
    pub change_types: Vec<String>,
    pub changed_fields: Vec<String>,
    /// low | medium | high
    pub risk: String,
    pub title: String,
    pub description: String,
    pub is_noise: bool,
    /// Changes of this resource among the ones matching the query.
    pub resource_changes: usize,
    /// The configuration before (null when created) and after (null when deleted); in the page only.
    pub old_fields: Option<Value>,
    pub new_fields: Option<Value>,
}

#[derive(Serialize, ToSchema)]
pub struct ResourceChangeBucket {
    /// Bucket start, epoch ms.
    pub t: i64,
    pub counts: BTreeMap<String, usize>,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ResourceChangePage {
    pub total: usize,
    pub items: Vec<ResourceChange>,
    pub histogram: Vec<ResourceChangeBucket>,
    pub bucket_ms: i64,
    pub facets: Vec<CatalogFacet>,
}

#[derive(clickhouse::Row, Deserialize)]
struct ListRow {
    detected: i64,
    resource_id: String,
    resource_type: String,
    resource_name: String,
    cloud: String,
    account: String,
    region: String,
    cluster: String,
    namespace: String,
    env: String,
    service: String,
    team: String,
    change_types: Vec<String>,
    changed_fields: Vec<String>,
    risk: String,
    title: String,
    description: String,
    is_noise: bool,
}

impl Faceted for ResourceChange {
    fn facet_values(&self, key: &str) -> Vec<&str> {
        fn one(v: &str) -> Vec<&str> {
            if v.is_empty() {
                vec![]
            } else {
                vec![v]
            }
        }
        match key {
            "team" => one(&self.team),
            "risk" | "risk_level" => vec![self.risk.as_str()],
            "cloud" => one(&self.cloud),
            "region" => one(&self.region),
            "env" => one(&self.env),
            "account" => one(&self.account),
            "service" => one(&self.service),
            "cluster" | "kube_cluster_name" => one(&self.cluster),
            "namespace" | "kube_namespace" => one(&self.namespace),
            "change_type" | "change_types" | "@change_types" => self.change_types.iter().map(String::as_str).collect(),
            "changed_field" | "@changed_fields" => self.changed_fields.iter().map(String::as_str).collect(),
            "resource_type" => vec![self.resource_type.as_str()],
            "resource_id" => vec![self.resource_id.as_str()],
            "name" => vec![self.resource_name.as_str()],
            "is_noise" | "@is_noise" => vec![if self.is_noise { "true" } else { "false" }],
            _ => vec![],
        }
    }
    fn text(&self) -> String {
        format!("{} {}", self.resource_name, self.title)
    }
}

fn risk_rank(r: &str) -> u8 {
    match r {
        "high" => 3,
        "medium" => 2,
        "low" => 1,
        _ => 0,
    }
}

/// Buckets of about 60 per range, at a round size.
fn bucket_size(range: &TimeRange) -> i64 {
    let span = (range.to - range.from).max(1);
    [300_000, 900_000, 1_800_000, 3_600_000, 10_800_000, 21_600_000, 43_200_000, 86_400_000].into_iter().find(|s| span / s <= 60).unwrap_or(86_400_000)
}

/// The matching changes (riskiest and newest first), a histogram stacked by `group_by`, and the filters' values.
pub(crate) fn search_page(rows: Vec<ResourceChange>, terms: &[Term], range: &TimeRange, group_by: &str, offset: usize, limit: usize) -> ResourceChangePage {
    let facets = catalog::facets(&rows, FACETS, terms);
    let mut matched: Vec<ResourceChange> = rows.into_iter().filter(|r| r.matches(terms)).collect();
    let mut per_resource: HashMap<String, usize> = HashMap::new();
    for r in &matched {
        *per_resource.entry(r.resource_id.clone()).or_default() += 1;
    }
    for r in &mut matched {
        r.resource_changes = per_resource[&r.resource_id];
    }
    let size = bucket_size(range);
    let mut buckets: BTreeMap<i64, BTreeMap<String, usize>> = BTreeMap::new();
    let mut t = range.from / size * size;
    while t <= range.to {
        buckets.insert(t, BTreeMap::new());
        t += size;
    }
    for r in &matched {
        let b = r.detected / size * size;
        for v in r.facet_values(group_by).into_iter().map(str::to_string).collect::<HashSet<_>>().into_iter().chain(r.facet_values(group_by).is_empty().then(|| "none".to_string())) {
            *buckets.entry(b).or_default().entry(v).or_default() += 1;
        }
    }
    matched.sort_by(|a, b| risk_rank(&b.risk).cmp(&risk_rank(&a.risk)).then(b.detected.cmp(&a.detected)).then(a.resource_id.cmp(&b.resource_id)));
    let total = matched.len();
    let items = matched.into_iter().skip(offset).take(limit.min(MAX_PAGE)).collect();
    ResourceChangePage { total, items, histogram: buckets.into_iter().map(|(t, counts)| ResourceChangeBucket { t, counts }).collect(), bucket_ms: size, facets }
}

#[utoipa::path(post, path = "/api/v1/infrastructure/resource-changes", operation_id = "search_resource_changes", request_body = ResourceChangeSearchRequest,
    responses((status = 200, body = ResourceChangePage)), tag = "infrastructure")]
pub async fn search(State(st): State<AppState>, OrgId(org): OrgId, Json(req): Json<ResourceChangeSearchRequest>) -> ApiResult<ResourceChangePage> {
    req.range.validate()?;
    let ch = st.ch(&org);
    let rows = ch
        .query(&format!(
            "SELECT toInt64(toUnixTimestamp64Milli(Detected)) AS detected, ResourceId AS resource_id, toString(ResourceType) AS resource_type, ResourceName AS resource_name, \
             toString(Cloud) AS cloud, toString(Account) AS account, toString(Region) AS region, toString(Cluster) AS cluster, toString(Namespace) AS namespace, toString(Env) AS env, \
             toString(Service) AS service, toString(Team) AS team, arrayMap(x -> toString(x), ChangeTypes) AS change_types, ChangedFields AS changed_fields, toString(Risk) AS risk, \
             Title AS title, Description AS description, IsNoise AS is_noise FROM otel.resource_changes \
             WHERE OrgId = {{oc_org:String}} AND Detected >= fromUnixTimestamp64Milli(?) AND Detected <= fromUnixTimestamp64Milli(?) ORDER BY Detected DESC LIMIT {MAX_ROWS}"
        ))
        .bind(req.range.from)
        .bind(req.range.to)
        .fetch_all::<ListRow>()
        .await?;
    let rows: Vec<ResourceChange> = rows
        .into_iter()
        .map(|r| ResourceChange {
            detected: r.detected,
            resource_id: r.resource_id,
            resource_type: r.resource_type,
            resource_name: r.resource_name,
            cloud: r.cloud,
            account: r.account,
            region: r.region,
            cluster: r.cluster,
            namespace: r.namespace,
            env: r.env,
            service: r.service,
            team: r.team,
            change_types: r.change_types,
            changed_fields: r.changed_fields,
            risk: r.risk,
            title: r.title,
            description: r.description,
            is_noise: r.is_noise,
            resource_changes: 0,
            old_fields: None,
            new_fields: None,
        })
        .collect();
    let terms = parse_query(&req.query);
    let mut page = search_page(rows, &terms, &req.range, req.group_by.as_deref().unwrap_or("resource_type"), req.offset, req.limit);
    // The configurations before and after, for the page's changes only.
    if !page.items.is_empty() {
        #[derive(clickhouse::Row, Deserialize)]
        struct Fields {
            detected: i64,
            resource_id: String,
            old_fields: String,
            new_fields: String,
        }
        let ids: Vec<String> = page.items.iter().map(|i| i.resource_id.clone()).collect();
        let from = page.items.iter().map(|i| i.detected).min().unwrap_or(req.range.from);
        let to = page.items.iter().map(|i| i.detected).max().unwrap_or(req.range.to);
        let fields = ch
            .query(
                "SELECT toInt64(toUnixTimestamp64Milli(Detected)) AS detected, ResourceId AS resource_id, OldFields AS old_fields, NewFields AS new_fields FROM otel.resource_changes \
                 WHERE OrgId = {oc_org:String} AND Detected >= fromUnixTimestamp64Milli(?) AND Detected <= fromUnixTimestamp64Milli(?) AND has(?, ResourceId)",
            )
            .bind(from)
            .bind(to)
            .bind(&ids)
            .fetch_all::<Fields>()
            .await?;
        let by_key: HashMap<(i64, String), Fields> = fields.into_iter().map(|f| ((f.detected, f.resource_id.clone()), f)).collect();
        for item in &mut page.items {
            if let Some(f) = by_key.get(&(item.detected, item.resource_id.clone())) {
                item.old_fields = serde_json::from_str(&f.old_fields).ok();
                item.new_fields = serde_json::from_str(&f.new_fields).ok();
            }
        }
    }
    Ok(Json(page))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn deployment(replicas: i64, image: &str, labels: Value) -> Value {
        json!({
            "resource_type": "kube_deployment", "name": "order-service", "team": "checkout",
            "attributes": {"replicas": replicas.to_string(), "images": image},
            "manifest": {"metadata": {"labels": labels}, "spec": {"replicas": replicas, "template": {"spec": {"containers": [{"name": "app", "image": image}]}}}},
        })
    }

    #[test]
    fn manifests_keep_configuration_not_status_or_bookkeeping() {
        let body = json!({
            "metadata": {"name": "x", "resourceVersion": "42", "managedFields": [{}], "labels": {"app": "x"},
                         "annotations": {"deployment.kubernetes.io/revision": "7", "owner": "checkout"}},
            "spec": {"replicas": 2}, "status": {"readyReplicas": 1},
        });
        assert_eq!(normalize_manifest(&body), json!({"metadata": {"labels": {"app": "x"}, "annotations": {"owner": "checkout"}}, "spec": {"replicas": 2}}));
    }

    #[test]
    fn changed_fields_name_paths_and_list_items_by_name() {
        let old = deployment(3, "example-org/order-service:2.8.0", json!({"app": "order"}));
        let new = deployment(3, "example-org/order-service:2.9.0", json!({"app": "order"}));
        assert_eq!(changed_fields(&old, &new), vec!["attributes.images", "manifest.spec.template.spec.containers[app].image"]);
        assert!(changed_fields(&old, &old).is_empty());
    }

    #[test]
    fn changes_are_typed_and_rated_by_what_they_touch() {
        let a = deployment(3, "img:1", json!({"app": "order"}));
        let scaled = deployment(5, "img:1", json!({"app": "order"}));
        let c = classify("kube_deployment", Event::Changed, &changed_fields(&a, &scaled), &a, &scaled);
        assert_eq!((c.change_types.clone(), c.risk, c.title.as_str()), (vec!["scaling".to_string()], "low", "Replicas changed from 3 to 5"));
        let zero = deployment(0, "img:1", json!({"app": "order"}));
        assert_eq!(classify("kube_deployment", Event::Changed, &changed_fields(&a, &zero), &a, &zero).risk, "high");
        let image = deployment(3, "img:2", json!({"app": "order"}));
        let c = classify("kube_deployment", Event::Changed, &changed_fields(&a, &image), &a, &image);
        assert_eq!((c.change_types.clone(), c.risk), (vec!["image".to_string()], "medium"));
        assert_eq!(c.title, "Image changed to img:2");
        let relabelled = deployment(3, "img:1", json!({"app": "orders"}));
        assert_eq!(classify("kube_deployment", Event::Changed, &changed_fields(&a, &relabelled), &a, &relabelled).risk, "low");
        let deleted = classify("kube_deployment", Event::Deleted, &[], &a, &Value::Null);
        assert_eq!((deleted.risk, deleted.title.as_str(), deleted.noise), ("high", "Kubernetes Deployment deleted", false));
        let pod = classify("kube_pod", Event::Deleted, &[], &a, &Value::Null);
        assert_eq!((pod.risk, pod.noise), ("low", true));
    }

    #[test]
    fn change_rows_skip_identical_snapshots_and_record_both_sides() {
        let a = deployment(3, "img:1", json!({}));
        assert!(change_row("demo", 1, "uid-1", &a, &a).is_none());
        let row = change_row("demo", 1, "uid-1", &Value::Null, &a).unwrap();
        assert_eq!((row.change_types.clone(), row.old_fields.as_str(), row.team.as_str()), (vec!["creation".to_string()], "", "checkout"));
        assert!(!row.changed_fields.is_empty());
    }

    fn change(id: &str, risk: &str, detected: i64, noise: bool) -> ResourceChange {
        ResourceChange {
            detected,
            resource_id: id.into(),
            resource_type: "kube_deployment".into(),
            resource_name: id.into(),
            cloud: String::new(),
            account: String::new(),
            region: "eu-central-1".into(),
            cluster: String::new(),
            namespace: String::new(),
            env: "prod".into(),
            service: String::new(),
            team: "checkout".into(),
            change_types: vec!["config".into()],
            changed_fields: vec!["attributes.images".into()],
            risk: risk.into(),
            title: String::new(),
            description: String::new(),
            is_noise: noise,
            resource_changes: 0,
            old_fields: None,
            new_fields: None,
        }
    }

    #[test]
    fn search_filters_noise_ranks_by_risk_and_counts_changes_per_resource() {
        let rows = vec![change("a", "low", 3_000, false), change("a", "high", 2_000, false), change("b", "medium", 1_000, true)];
        let page = search_page(rows, &parse_query("@is_noise:false"), &TimeRange { from: 0, to: 3_600_000 }, "risk", 0, 50);
        assert_eq!(page.total, 2);
        assert_eq!(page.items.iter().map(|i| i.risk.as_str()).collect::<Vec<_>>(), vec!["high", "low"]);
        assert_eq!(page.items[0].resource_changes, 2);
        assert_eq!(page.histogram[0].counts.get("high"), Some(&1));
        assert_eq!(type_label("kube_persistent_volume_claim"), "Kubernetes Persistent Volume Claim");
    }
}
