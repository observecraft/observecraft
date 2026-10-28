//! Infrastructure → Resources → Policies: rules on the resource catalog, their compliance and history.
//!
//! - Policies live in Postgres (`resource_policies`); what they check and how they are evaluated is
//!   in [`super::resource_policy_rules`]. Evaluation runs on the current catalog
//!   ([`super::catalog::load_all`]), so every read is up to date; nothing new is ingested.
//! - A daily snapshot per policy (`resource_policy_snapshots`, the day's latest evaluation) gives the
//!   score over time. [`run_snapshots`] refreshes it hourly; orgs are claimed with
//!   `FOR UPDATE SKIP LOCKED`, so several servers can run it. Writes also refresh today's snapshot.
//! - Reads are open to every role; creating, changing and deleting policies needs Standard or Admin,
//!   and each write is in the audit trail.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::time::Duration;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use oc_query::{parse_query, Term};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use super::catalog::{self, CatalogFacet, CatalogResource, CatalogType};
use super::resource_policy_rules::{self as rules, CompiledPolicy, PolicyCategory, PolicyCondition, PolicyEvaluation, PolicyTarget, PolicyTemplate};
use crate::auth::{AuthContext, Writer};
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;

const MAX_POLICIES: i64 = 500;
const MAX_NAME_CHARS: usize = 200;
const MAX_DESCRIPTION_CHARS: usize = 2000;
/// Noncompliant resources listed on a policy's page, and in a preview.
const MAX_VIOLATIONS: usize = 500;
const PREVIEW_VIOLATIONS: usize = 20;
const HISTORY_DAYS: i32 = 90;
const SNAPSHOT_INTERVAL_MINUTES: i32 = 60;
const CLAIM_SECONDS: i32 = 600;
/// Header filters of the policies page, in display order.
const FILTER_FACETS: &[&str] = &["team", "cloud", "region", "env", "account", "service"];
/// Attribute keys listed per resource type in the editor, and example values per key.
const MAX_ATTRIBUTES: usize = 150;
const MAX_EXAMPLES: usize = 5;

/// A policy as stored, with its compliance on the current catalog.
#[derive(Serialize, ToSchema, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ResourcePolicy {
    pub id: String,
    pub name: String,
    pub description: String,
    /// 0 (P0, most urgent) … 4.
    pub priority: u8,
    pub category: PolicyCategory,
    pub resource_type: CatalogType,
    pub scope: String,
    pub conditions: Vec<PolicyCondition>,
    pub template: Option<String>,
    /// Name of the user who created it.
    pub created_by: Option<String>,
    /// Epoch ms.
    pub created_at: i64,
    pub updated_at: i64,
    pub evaluation: PolicyEvaluation,
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ResourcePolicyInput {
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub priority: u8,
    pub category: PolicyCategory,
    pub resource_type: CatalogType,
    #[serde(default)]
    pub scope: String,
    pub conditions: Vec<PolicyCondition>,
    /// The template it was made from.
    pub template: Option<String>,
}

#[derive(Deserialize, ToSchema, Default)]
#[serde(rename_all = "camelCase")]
pub struct ResourcePolicySearchRequest {
    /// The page's filters as a catalog query (`team:checkout region:eu-west-1 env:prod`): scores count
    /// only the resources matching it.
    #[serde(default)]
    pub query: String,
}

/// A template with how the catalog would score today.
#[derive(Serialize, ToSchema, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ResourcePolicyTemplateView {
    #[serde(flatten)]
    pub template: PolicyTemplate,
    /// Resources of the template's type exist in the catalog.
    pub available: bool,
    pub preview: PolicyEvaluation,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ResourcePolicyList {
    pub policies: Vec<ResourcePolicy>,
    pub templates: Vec<ResourcePolicyTemplateView>,
    /// Values of the header filters with resource counts.
    pub facets: Vec<CatalogFacet>,
}

/// A noncompliant resource and why.
#[derive(Serialize, ToSchema, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ResourcePolicyViolation {
    pub resource: CatalogResource,
    pub reason: String,
}

#[derive(Serialize, ToSchema, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ResourcePolicySnapshot {
    /// `YYYY-MM-DD` (UTC).
    pub day: String,
    pub evaluated: i32,
    pub noncompliant: i32,
    pub score: Option<f64>,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ResourcePolicyDetail {
    pub policy: ResourcePolicy,
    /// Daily snapshots, oldest first (last 90 days).
    pub history: Vec<ResourcePolicySnapshot>,
    /// Noncompliant resources (at most 500), by name.
    pub violations: Vec<ResourcePolicyViolation>,
}

#[derive(Deserialize, IntoParams)]
pub struct ResourcePolicyDetailQuery {
    /// The page's filters, as in the list.
    #[serde(default)]
    pub query: String,
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ResourcePolicyPreviewRequest {
    pub resource_type: CatalogType,
    #[serde(default)]
    pub scope: String,
    pub conditions: Vec<PolicyCondition>,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ResourcePolicyPreview {
    pub evaluation: PolicyEvaluation,
    /// The first noncompliant resources.
    pub violations: Vec<ResourcePolicyViolation>,
}

/// An attribute a policy can check on a resource type, with how many resources have it.
#[derive(Serialize, ToSchema, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ResourcePolicyAttribute {
    pub key: String,
    pub count: usize,
    pub examples: Vec<String>,
}

#[derive(Serialize, ToSchema, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ResourcePolicyTypeAttributes {
    pub resource_type: CatalogType,
    pub resources: usize,
    pub attributes: Vec<ResourcePolicyAttribute>,
}

// ================================================================ pure helpers

/// Fields every catalog row has, offered before attributes and tags.
const FIELDS: &[&str] = &["name", "cloud", "account", "region", "cluster", "namespace", "env", "service", "team", "status"];

/// Per resource type, the keys a policy can check (fields, attributes, tag keys) with counts and examples.
fn attributes_by_type(resources: &[CatalogResource]) -> Vec<ResourcePolicyTypeAttributes> {
    // Per type: the type, its resource count, and per key the resources having it and example values.
    type KeyStats = BTreeMap<String, (usize, BTreeSet<String>)>;
    let mut by: BTreeMap<&'static str, (CatalogType, usize, KeyStats)> = BTreeMap::new();
    for r in resources {
        let entry = by.entry(rules::type_str(r.resource_type)).or_insert_with(|| (r.resource_type, 0, BTreeMap::new()));
        entry.1 += 1;
        let mut keys: BTreeSet<String> = FIELDS.iter().map(|k| k.to_string()).collect();
        keys.extend(r.attributes.keys().cloned());
        keys.extend(r.tags.iter().filter_map(|t| t.split_once(':')).map(|(k, _)| k.to_string()).filter(|k| !k.is_empty()));
        for key in keys {
            let values = r.values(&key);
            if values.is_empty() {
                continue;
            }
            let a = entry.2.entry(key).or_default();
            a.0 += 1;
            for v in values {
                if a.1.len() < MAX_EXAMPLES {
                    a.1.insert(v.to_string());
                }
            }
        }
    }
    by.into_values()
        .map(|(resource_type, resources, attrs)| {
            let mut attributes: Vec<ResourcePolicyAttribute> = attrs.into_iter().map(|(key, (count, ex))| ResourcePolicyAttribute { key, count, examples: ex.into_iter().collect() }).collect();
            // Fields first, then the most common keys.
            attributes.sort_by(|a, b| {
                let field = |k: &str| FIELDS.iter().position(|f| *f == k).unwrap_or(usize::MAX);
                field(&a.key).cmp(&field(&b.key)).then(b.count.cmp(&a.count)).then(a.key.cmp(&b.key))
            });
            attributes.truncate(MAX_ATTRIBUTES);
            ResourcePolicyTypeAttributes { resource_type, resources, attributes }
        })
        .collect()
}

fn check_text(value: &str, what: &str, max: usize, required: bool) -> Result<String, ApiError> {
    let v = value.trim();
    if required && v.is_empty() {
        return Err(ApiError::BadRequest(format!("The {what} is required")));
    }
    if v.chars().count() > max {
        return Err(ApiError::BadRequest(format!("The {what} has at most {max} characters")));
    }
    Ok(v.to_string())
}

/// The input checked: trimmed text, a priority in range, conditions that compile.
fn validate(input: &ResourcePolicyInput) -> Result<(String, String, PolicyTarget), ApiError> {
    let name = check_text(&input.name, "policy name", MAX_NAME_CHARS, true)?;
    let description = check_text(&input.description, "description", MAX_DESCRIPTION_CHARS, false)?;
    if input.priority > 4 {
        return Err(ApiError::BadRequest("The priority is P0 to P4".into()));
    }
    let target = PolicyTarget { resource_type: input.resource_type, scope: input.scope.trim().to_string(), conditions: input.conditions.clone() };
    CompiledPolicy::new(&target).map_err(ApiError::BadRequest)?;
    Ok((name, description, target))
}

fn violations(policy: &CompiledPolicy, resources: &[CatalogResource], filter: &[Term], limit: usize) -> (PolicyEvaluation, Vec<ResourcePolicyViolation>) {
    let (evaluation, found) = rules::evaluate(policy, resources, filter);
    let by_id: HashMap<&str, &CatalogResource> = resources.iter().map(|r| (r.id.as_str(), r)).collect();
    let mut out: Vec<ResourcePolicyViolation> =
        found.into_iter().filter_map(|v| by_id.get(v.resource_id.as_str()).map(|r| ResourcePolicyViolation { resource: (*r).clone(), reason: v.reason })).collect();
    out.sort_by(|a, b| a.resource.name.cmp(&b.resource.name).then(a.resource.id.cmp(&b.resource.id)));
    out.truncate(limit);
    (evaluation, out)
}

// ================================================================ storage

#[derive(sqlx::FromRow)]
struct PolicyRow {
    id: String,
    name: String,
    description: String,
    priority: i16,
    category: String,
    resource_type: String,
    scope: String,
    conditions: serde_json::Value,
    template: Option<String>,
    created_by: Option<String>,
    created_at: i64,
    updated_at: i64,
}

const POLICY_SELECT: &str = "SELECT p.id::text AS id, p.name, p.description, p.priority, p.category, p.resource_type, p.scope, p.conditions, p.template, u.name AS created_by, \
     (extract(epoch FROM p.created_at) * 1000)::bigint AS created_at, (extract(epoch FROM p.updated_at) * 1000)::bigint AS updated_at \
     FROM resource_policies p LEFT JOIN users u ON u.id = p.created_by AND u.org_id = p.org_id";

/// A stored policy, or None when the row no longer parses (a resource type that was renamed).
fn from_row(r: PolicyRow) -> Option<(ResourcePolicy, CompiledPolicy)> {
    let resource_type: CatalogType = serde_json::from_value(serde_json::Value::String(r.resource_type)).ok()?;
    let conditions: Vec<PolicyCondition> = serde_json::from_value(r.conditions).ok()?;
    let compiled = CompiledPolicy::new(&PolicyTarget { resource_type, scope: r.scope.clone(), conditions: conditions.clone() }).ok()?;
    let policy = ResourcePolicy {
        id: r.id,
        name: r.name,
        description: r.description,
        priority: r.priority.clamp(0, 4) as u8,
        category: PolicyCategory::parse(&r.category)?,
        resource_type,
        scope: r.scope,
        conditions,
        template: r.template,
        created_by: r.created_by,
        created_at: r.created_at,
        updated_at: r.updated_at,
        evaluation: PolicyEvaluation::default(),
    };
    Some((policy, compiled))
}

async fn load_policies(st: &AppState, org: &str) -> Result<Vec<(ResourcePolicy, CompiledPolicy)>, ApiError> {
    let rows: Vec<PolicyRow> = sqlx::query_as(&format!("{POLICY_SELECT} WHERE p.org_id = $1 ORDER BY p.priority, p.name")).bind(org).fetch_all(&st.postgres).await?;
    Ok(rows.into_iter().filter_map(from_row).collect())
}

async fn load_policy(st: &AppState, org: &str, id: &str) -> Result<(ResourcePolicy, CompiledPolicy), ApiError> {
    let not_found = || ApiError::NotFound(format!("Policy {id} not found"));
    if !super::scorecards::valid_uuid(id) {
        return Err(not_found());
    }
    let row: Option<PolicyRow> = sqlx::query_as(&format!("{POLICY_SELECT} WHERE p.org_id = $1 AND p.id = $2::uuid")).bind(org).bind(id).fetch_optional(&st.postgres).await?;
    row.and_then(from_row).ok_or_else(not_found)
}

/// Writes today's snapshot of each evaluated policy (the whole catalog, without page filters).
async fn store_snapshots(st: &AppState, org: &str, evaluated: &[(String, PolicyEvaluation)]) -> Result<(), ApiError> {
    let mut tx = st.postgres.begin().await?;
    for (id, e) in evaluated {
        sqlx::query(
            "INSERT INTO resource_policy_snapshots (org_id, policy_id, day, evaluated, noncompliant) VALUES ($1, $2::uuid, (now() AT TIME ZONE 'UTC')::date, $3, $4) \
             ON CONFLICT (policy_id, day) DO UPDATE SET evaluated = EXCLUDED.evaluated, noncompliant = EXCLUDED.noncompliant, taken_at = now()",
        )
        .bind(org)
        .bind(id)
        .bind(i32::try_from(e.evaluated).unwrap_or(i32::MAX))
        .bind(i32::try_from(e.noncompliant).unwrap_or(i32::MAX))
        .execute(&mut *tx)
        .await?;
    }
    sqlx::query("UPDATE resource_policy_settings SET snapshot_at = now() WHERE org_id = $1").bind(org).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(())
}

/// Evaluates every policy of the org on the current catalog and stores today's snapshots.
async fn snapshot_org(st: &AppState, org: &str) -> Result<usize, ApiError> {
    let policies = load_policies(st, org).await?;
    if policies.is_empty() {
        return Ok(0);
    }
    let catalog = catalog::load_all(st, org).await?;
    let evaluated: Vec<(String, PolicyEvaluation)> = policies.iter().map(|(p, c)| (p.id.clone(), rules::evaluate(c, &catalog, &[]).0)).collect();
    store_snapshots(st, org, &evaluated).await?;
    Ok(evaluated.len())
}

/// Claims up to `limit` orgs due for a snapshot and holds them while they run.
async fn claim(st: &AppState, limit: i64) -> Result<Vec<String>, ApiError> {
    let mut tx = st.postgres.begin().await?;
    let orgs: Vec<(String,)> = sqlx::query_as(
        "SELECT s.org_id FROM resource_policy_settings s JOIN orgs o ON o.id = s.org_id WHERE s.next_snapshot_at <= now() AND (s.claimed_until IS NULL OR s.claimed_until < now()) \
         AND o.status = 'active' ORDER BY s.next_snapshot_at LIMIT $1 FOR UPDATE OF s SKIP LOCKED",
    )
    .bind(limit)
    .fetch_all(&mut *tx)
    .await?;
    for (org,) in &orgs {
        sqlx::query("UPDATE resource_policy_settings SET claimed_until = now() + make_interval(secs => $2), next_snapshot_at = now() + make_interval(mins => $3) WHERE org_id = $1")
            .bind(org)
            .bind(CLAIM_SECONDS)
            .bind(SNAPSHOT_INTERVAL_MINUTES)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok(orgs.into_iter().map(|(o,)| o).collect())
}

/// Refreshes the daily compliance snapshots of every org with policies, hourly.
pub async fn run_snapshots(st: AppState) {
    let mut tick = tokio::time::interval(Duration::from_secs(60));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tick.tick().await;
        let due = match claim(&st, 10).await {
            Ok(due) => due,
            Err(e) => {
                tracing::warn!(error = %e, "claiming resource policy snapshots failed");
                continue;
            }
        };
        for org in due {
            if let Err(e) = snapshot_org(&st, &org).await {
                tracing::warn!(org = %org, error = %e, "resource policy snapshot failed");
            }
            if let Err(e) = sqlx::query("UPDATE resource_policy_settings SET claimed_until = NULL WHERE org_id = $1").bind(&org).execute(&st.postgres).await {
                tracing::warn!(org = %org, error = %e, "releasing the resource policy snapshot claim failed");
            }
        }
    }
}

// ================================================================ handlers

/// Policies with their compliance (counting only resources matching the page's filters), the
/// templates with a preview score, and the filters' values.
#[utoipa::path(post, path = "/api/v1/infrastructure/policies/search", operation_id = "search_resource_policies", request_body = ResourcePolicySearchRequest,
    responses((status = 200, body = ResourcePolicyList)), tag = "infrastructure")]
pub async fn search(State(st): State<AppState>, ctx: AuthContext, Json(req): Json<ResourcePolicySearchRequest>) -> ApiResult<ResourcePolicyList> {
    let (policies, catalog) = tokio::try_join!(load_policies(&st, &ctx.org_id), catalog::load_all(&st, &ctx.org_id))?;
    let filter = parse_query(&req.query);
    let policies = policies
        .into_iter()
        .map(|(mut p, c)| {
            p.evaluation = rules::evaluate(&c, &catalog, &filter).0;
            p
        })
        .collect();
    let templates = rules::templates()
        .into_iter()
        .filter_map(|t| {
            let c = CompiledPolicy::new(&t.target).ok()?;
            let available = catalog.iter().any(|r| r.resource_type == t.target.resource_type);
            Some(ResourcePolicyTemplateView { preview: rules::evaluate(&c, &catalog, &filter).0, available, template: t })
        })
        .collect();
    let facets = catalog::facets(&catalog, FILTER_FACETS, &filter);
    Ok(Json(ResourcePolicyList { policies, templates, facets }))
}

/// The attributes policies can check, per resource type in the catalog.
#[utoipa::path(get, path = "/api/v1/infrastructure/policies/attributes", operation_id = "resource_policy_attributes", responses((status = 200, body = Vec<ResourcePolicyTypeAttributes>)), tag = "infrastructure")]
pub async fn attributes(State(st): State<AppState>, ctx: AuthContext) -> ApiResult<Vec<ResourcePolicyTypeAttributes>> {
    Ok(Json(attributes_by_type(&catalog::load_all(&st, &ctx.org_id).await?)))
}

/// What a policy being edited would score on the current catalog.
#[utoipa::path(post, path = "/api/v1/infrastructure/policies/preview", operation_id = "preview_resource_policy", request_body = ResourcePolicyPreviewRequest,
    responses((status = 200, body = ResourcePolicyPreview), (status = 400, body = crate::error::ErrorBody)), tag = "infrastructure")]
pub async fn preview(State(st): State<AppState>, ctx: AuthContext, Json(req): Json<ResourcePolicyPreviewRequest>) -> ApiResult<ResourcePolicyPreview> {
    let compiled = CompiledPolicy::new(&PolicyTarget { resource_type: req.resource_type, scope: req.scope, conditions: req.conditions }).map_err(ApiError::BadRequest)?;
    let catalog = catalog::load_all(&st, &ctx.org_id).await?;
    let (evaluation, violations) = violations(&compiled, &catalog, &[], PREVIEW_VIOLATIONS);
    Ok(Json(ResourcePolicyPreview { evaluation, violations }))
}

/// A policy with its score over time and its noncompliant resources.
#[utoipa::path(get, path = "/api/v1/infrastructure/policies/{id}", operation_id = "get_resource_policy", params(("id" = String, Path), ResourcePolicyDetailQuery),
    responses((status = 200, body = ResourcePolicyDetail), (status = 404, body = crate::error::ErrorBody)), tag = "infrastructure")]
pub async fn get(State(st): State<AppState>, ctx: AuthContext, Path(id): Path<String>, Query(q): Query<ResourcePolicyDetailQuery>) -> ApiResult<ResourcePolicyDetail> {
    let (mut policy, compiled) = load_policy(&st, &ctx.org_id, &id).await?;
    let catalog = catalog::load_all(&st, &ctx.org_id).await?;
    let (evaluation, violations) = violations(&compiled, &catalog, &parse_query(&q.query), MAX_VIOLATIONS);
    policy.evaluation = evaluation;
    let history: Vec<(String, i32, i32)> = sqlx::query_as(
        "SELECT to_char(day, 'YYYY-MM-DD'), evaluated, noncompliant FROM resource_policy_snapshots WHERE org_id = $1 AND policy_id = $2::uuid \
         AND day > (now() AT TIME ZONE 'UTC')::date - $3 ORDER BY day",
    )
    .bind(&ctx.org_id)
    .bind(&id)
    .bind(HISTORY_DAYS)
    .fetch_all(&st.postgres)
    .await?;
    let history = history
        .into_iter()
        .map(|(day, evaluated, noncompliant)| ResourcePolicySnapshot { day, evaluated, noncompliant, score: rules::score(evaluated.max(0) as usize, noncompliant.max(0) as usize) })
        .collect();
    Ok(Json(ResourcePolicyDetail { policy, history, violations }))
}

/// Re-reads a policy after a write, evaluates it on the whole catalog and stores today's snapshot.
async fn saved(st: &AppState, org: &str, id: &str) -> Result<ResourcePolicy, ApiError> {
    let (mut policy, compiled) = load_policy(st, org, id).await?;
    let catalog = catalog::load_all(st, org).await?;
    policy.evaluation = rules::evaluate(&compiled, &catalog, &[]).0;
    store_snapshots(st, org, &[(policy.id.clone(), policy.evaluation.clone())]).await?;
    Ok(policy)
}

fn unique_name(name: &str) -> impl Fn(sqlx::Error) -> ApiError + '_ {
    move |e| match e {
        sqlx::Error::Database(d) if d.is_unique_violation() => ApiError::BadRequest(format!("A policy named '{name}' already exists")),
        e => e.into(),
    }
}

/// Creates a policy.
#[utoipa::path(post, path = "/api/v1/infrastructure/policies", operation_id = "create_resource_policy", request_body = ResourcePolicyInput,
    responses((status = 200, body = ResourcePolicy), (status = 400, body = crate::error::ErrorBody), (status = 403, body = crate::error::ErrorBody)), tag = "infrastructure")]
pub async fn create(State(st): State<AppState>, Writer(ctx): Writer, Json(input): Json<ResourcePolicyInput>) -> ApiResult<ResourcePolicy> {
    let (name, description, target) = validate(&input)?;
    let mut tx = st.postgres.begin().await?;
    let (count,): (i64,) = sqlx::query_as("SELECT count(*) FROM resource_policies WHERE org_id = $1").bind(&ctx.org_id).fetch_one(&mut *tx).await?;
    if count >= MAX_POLICIES {
        return Err(ApiError::BadRequest(format!("An organization can have at most {MAX_POLICIES} policies")));
    }
    let conditions = serde_json::to_value(&target.conditions).map_err(anyhow::Error::from)?;
    let (id,): (String,) = sqlx::query_as(
        "INSERT INTO resource_policies (org_id, name, description, priority, category, resource_type, scope, conditions, template, created_by) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10::uuid) RETURNING id::text",
    )
    .bind(&ctx.org_id)
    .bind(&name)
    .bind(&description)
    .bind(i16::from(input.priority))
    .bind(input.category.as_str())
    .bind(rules::type_str(target.resource_type))
    .bind(&target.scope)
    .bind(conditions)
    .bind(input.template.as_deref().filter(|t| rules::templates().iter().any(|x| x.key == *t)))
    .bind(&ctx.user_id)
    .fetch_one(&mut *tx)
    .await
    .map_err(unique_name(&name))?;
    sqlx::query("INSERT INTO resource_policy_settings (org_id) VALUES ($1) ON CONFLICT (org_id) DO NOTHING").bind(&ctx.org_id).execute(&mut *tx).await?;
    tx.commit().await?;
    let policy = saved(&st, &ctx.org_id, &id).await?;
    ctx.audit(&st, "resource_policies.created", &id, &format!("{name} (P{}, {})", input.priority, rules::type_str(target.resource_type))).await;
    Ok(Json(policy))
}

/// Replaces a policy's definition.
#[utoipa::path(put, path = "/api/v1/infrastructure/policies/{id}", operation_id = "update_resource_policy", params(("id" = String, Path)), request_body = ResourcePolicyInput,
    responses((status = 200, body = ResourcePolicy), (status = 400, body = crate::error::ErrorBody), (status = 404, body = crate::error::ErrorBody)), tag = "infrastructure")]
pub async fn update(State(st): State<AppState>, Writer(ctx): Writer, Path(id): Path<String>, Json(input): Json<ResourcePolicyInput>) -> ApiResult<ResourcePolicy> {
    load_policy(&st, &ctx.org_id, &id).await?;
    let (name, description, target) = validate(&input)?;
    let conditions = serde_json::to_value(&target.conditions).map_err(anyhow::Error::from)?;
    sqlx::query(
        "UPDATE resource_policies SET name = $3, description = $4, priority = $5, category = $6, resource_type = $7, scope = $8, conditions = $9, updated_at = now() \
         WHERE org_id = $1 AND id = $2::uuid",
    )
    .bind(&ctx.org_id)
    .bind(&id)
    .bind(&name)
    .bind(&description)
    .bind(i16::from(input.priority))
    .bind(input.category.as_str())
    .bind(rules::type_str(target.resource_type))
    .bind(&target.scope)
    .bind(conditions)
    .execute(&st.postgres)
    .await
    .map_err(unique_name(&name))?;
    let policy = saved(&st, &ctx.org_id, &id).await?;
    let rule: Vec<String> = target.conditions.iter().map(PolicyCondition::describe).collect();
    ctx.audit(&st, "resource_policies.updated", &id, &format!("{name}: {}", rule.join(" and "))).await;
    Ok(Json(policy))
}

/// Deletes a policy and its history.
#[utoipa::path(delete, path = "/api/v1/infrastructure/policies/{id}", operation_id = "delete_resource_policy", params(("id" = String, Path)),
    responses((status = 204), (status = 404, body = crate::error::ErrorBody)), tag = "infrastructure")]
pub async fn delete(State(st): State<AppState>, Writer(ctx): Writer, Path(id): Path<String>) -> Result<StatusCode, ApiError> {
    let (policy, _) = load_policy(&st, &ctx.org_id, &id).await?;
    sqlx::query("DELETE FROM resource_policies WHERE org_id = $1 AND id = $2::uuid").bind(&ctx.org_id).bind(&id).execute(&st.postgres).await?;
    ctx.audit(&st, "resource_policies.deleted", &id, &policy.name).await;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::routes::resource_policy_rules::PolicyOperator;

    fn res(t: CatalogType, id: &str, tags: &[&str], attrs: &[(&str, &str)]) -> CatalogResource {
        CatalogResource {
            id: id.into(),
            resource_type: t,
            resource_category: catalog::CatalogCategory::Kubernetes,
            name: id.into(),
            cloud: None,
            account: None,
            region: None,
            cluster: Some("shop-prod-eu".into()),
            namespace: None,
            env: None,
            service: None,
            team: None,
            status: None,
            health: None,
            created_at: None,
            last_seen: 0,
            tags: tags.iter().map(|t| t.to_string()).collect(),
            attributes: attrs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
            kube_kind: None,
        }
    }

    fn input(priority: u8, conditions: Vec<PolicyCondition>) -> ResourcePolicyInput {
        ResourcePolicyInput {
            name: "  Prod replicas  ".into(),
            description: String::new(),
            priority,
            category: PolicyCategory::Reliability,
            resource_type: CatalogType::KubeDeployment,
            scope: " env:prod ".into(),
            conditions,
            template: None,
        }
    }

    #[test]
    fn inputs_are_trimmed_and_checked() {
        let ok = vec![PolicyCondition { attribute: "replicas".into(), operator: PolicyOperator::Gte, values: vec!["2".into()] }];
        let (name, _, target) = validate(&input(1, ok.clone())).unwrap();
        assert_eq!((name.as_str(), target.scope.as_str()), ("Prod replicas", "env:prod"));
        assert!(matches!(validate(&input(5, ok)), Err(ApiError::BadRequest(m)) if m.contains("P0 to P4")));
        assert!(matches!(validate(&input(1, vec![])), Err(ApiError::BadRequest(_))));
        let mut blank = input(1, vec![PolicyCondition { attribute: "team".into(), operator: PolicyOperator::Exists, values: vec![] }]);
        blank.name = "  ".into();
        assert!(matches!(validate(&blank), Err(ApiError::BadRequest(m)) if m.contains("required")));
    }

    #[test]
    fn attributes_are_listed_per_type_with_fields_first() {
        let all = vec![
            res(CatalogType::KubeDeployment, "a", &["team:checkout", "app:shop"], &[("replicas", "1")]),
            res(CatalogType::KubeDeployment, "b", &["app:shop"], &[("replicas", "3"), ("image_tags", "2.4, latest")]),
            res(CatalogType::Host, "h", &[], &[("os_release", "22.04")]),
        ];
        let types = attributes_by_type(&all);
        assert_eq!(types.iter().map(|t| (rules::type_str(t.resource_type), t.resources)).collect::<Vec<_>>(), vec![("host", 1), ("kube_deployment", 2)]);
        let dep = &types[1].attributes;
        let keys: Vec<&str> = dep.iter().map(|a| a.key.as_str()).collect();
        // name and cluster are fields (every row has them); the rest by count.
        // The team field is unset in these rows (a field shadows its tag), so the tag is not offered.
        assert_eq!(keys, vec!["name", "cluster", "app", "replicas", "image_tags"]);
        let replicas = dep.iter().find(|a| a.key == "replicas").unwrap();
        assert_eq!((replicas.count, replicas.examples.clone()), (2, vec!["1".to_string(), "3".to_string()]));
        assert_eq!(dep.iter().find(|a| a.key == "image_tags").unwrap().examples, vec!["2.4", "latest"]);
    }

    #[test]
    fn violations_list_resources_by_name_with_the_reason() {
        let all =
            vec![res(CatalogType::KubeDeployment, "b", &[], &[("replicas", "1")]), res(CatalogType::KubeDeployment, "a", &[], &[]), res(CatalogType::KubeDeployment, "c", &[], &[("replicas", "4")])];
        let p = CompiledPolicy::new(&PolicyTarget {
            resource_type: CatalogType::KubeDeployment,
            scope: String::new(),
            conditions: vec![PolicyCondition { attribute: "replicas".into(), operator: PolicyOperator::Gte, values: vec!["2".into()] }],
        })
        .unwrap();
        let (e, v) = violations(&p, &all, &[], 10);
        assert_eq!((e.evaluated, e.noncompliant, e.score), (3, 2, Some(33.3)));
        assert_eq!(v.iter().map(|x| (x.resource.name.as_str(), x.reason.as_str())).collect::<Vec<_>>(), vec![("a", "replicas is missing"), ("b", "replicas is 1")]);
        assert_eq!(violations(&p, &all, &[], 1).1.len(), 1);
    }
}
