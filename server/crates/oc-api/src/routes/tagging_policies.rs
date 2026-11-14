//! Infrastructure → Resources → Tagging: tagging policies and their coverage (docs/adr/0008-remediation-and-provenance.md,
//! decisions 2 and 3).
//!
//! - Policies live in Postgres (`tagging_policies`), per org; what they check is [`super::tagging_rules`]. The
//!   recommended set is installed once per org when it first reads its policies (`tagging_policy_settings`), so a
//!   deleted policy stays deleted. Datadog's Terraform provider has no resource for required-tag policies (its
//!   `datadog_tag_pipeline_ruleset` adds tags to costs), so the API is our own: `/api/v1/tagging-policies`.
//! - Evaluation reads the resources and their provenance ([`super::provenance::inventory`]) when asked; nothing new
//!   is ingested. A daily snapshot of the coverage (total, per policy, target, account, cluster and team;
//!   `tagging_coverage_snapshots`) gives the history; [`run_snapshots`] refreshes it hourly, orgs claimed with
//!   `FOR UPDATE SKIP LOCKED`, and writes refresh today's.
//! - Non-compliant cloud resources, Kubernetes objects and images are also Cloud Security misconfigurations (rule
//!   [`MISCONFIG_RULE`], low severity), so they reach the Security Inbox ([`misconfig_gaps`]).
//! - Reads are open to every role; writes need Standard or Admin and are audited.

use std::time::Duration;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use super::provenance::{self, summarize, ProvenanceSummary, TagTarget, TaggedResource};
use super::tagging_rules::{self as rules, CompiledRule, Coverage, CoverageCount, EvaluatedPolicy, NonCompliantResource, RecommendedPolicy, RequiredTag, TagRule, TagScope};
use crate::auth::{AuthContext, Writer};
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;

const MAX_POLICIES: i64 = 200;
const MAX_NAME_CHARS: usize = 200;
const MAX_DESCRIPTION_CHARS: usize = 2000;
/// Non-compliant resources listed on the coverage page.
const MAX_NONCOMPLIANT: usize = 500;
const HISTORY_DAYS: i32 = 90;
const SNAPSHOT_INTERVAL_MINUTES: i32 = 60;
const CLAIM_SECONDS: i32 = 600;

/// Cloud Security's misconfiguration rule for resources missing required tags.
pub const MISCONFIG_RULE: &str = "ocs-tag-001";
pub const MISCONFIG_NAME: &str = "Resource missing required tags";
pub const MISCONFIG_DESCRIPTION: &str =
    "The resource lacks tags its organization's tagging policies require, or carries values they don't allow. Without them its owner, environment or source repository is unknown, and findings on it can't be fixed where it is defined.";
pub const MISCONFIG_REMEDIATION: &str =
    "Add the missing tags where the resource is defined: Terraform default_tags (AWS) or a merged locals map (Azure), a Yor job in CI, Argo CD tracking for Kubernetes objects, OCI labels at image build. Infrastructure → Resources → Tagging lists each policy and a snippet per convention.";

/// A policy as stored, with its coverage on the current resources.
#[derive(Serialize, ToSchema, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct TaggingPolicy {
    pub id: String,
    pub name: String,
    pub description: String,
    pub target: TagTarget,
    pub scope: TagScope,
    pub required: Vec<RequiredTag>,
    pub enabled: bool,
    /// The recommended policy it comes from.
    pub recommended: Option<String>,
    pub created_by: Option<String>,
    /// Epoch ms.
    pub created_at: i64,
    pub updated_at: i64,
    pub coverage: CoverageCount,
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct TaggingPolicyInput {
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub target: TagTarget,
    #[serde(default)]
    pub scope: TagScope,
    pub required: Vec<RequiredTag>,
    #[serde(default = "enabled_default")]
    pub enabled: bool,
    /// The recommended policy it is made from.
    pub recommended: Option<String>,
}

fn enabled_default() -> bool {
    true
}

/// A recommended policy, and whether the org has a policy made from it.
#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct RecommendedView {
    #[serde(flatten)]
    pub policy: RecommendedPolicy,
    pub installed: bool,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct TaggingPolicyList {
    pub policies: Vec<TaggingPolicy>,
    pub recommended: Vec<RecommendedView>,
}

/// The day's coverage.
#[derive(Serialize, ToSchema, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CoveragePoint {
    /// YYYY-MM-DD (UTC).
    pub day: String,
    #[serde(flatten)]
    pub count: CoverageCount,
}

#[derive(Deserialize, IntoParams)]
#[serde(rename_all = "camelCase")]
pub struct CoverageQuery {
    /// Only this policy (its id).
    pub policy: Option<String>,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct TagCoverage {
    #[serde(flatten)]
    pub coverage: Coverage,
    /// The total per day, oldest first.
    pub history: Vec<CoveragePoint>,
    /// Where the evaluated resources come from.
    pub provenance: ProvenanceSummary,
    /// Argo CD Applications collected (0: Kubernetes objects resolve to their Application's name only).
    pub argocd_applications: usize,
}

// ================================================================ validation and storage

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

/// The input checked: trimmed text, a rule that compiles, a known recommended key.
fn validate(input: &TaggingPolicyInput) -> Result<(String, String, CompiledRule, Option<&'static str>), ApiError> {
    let name = check_text(&input.name, "policy name", MAX_NAME_CHARS, true)?;
    let description = check_text(&input.description, "description", MAX_DESCRIPTION_CHARS, false)?;
    let rule = CompiledRule::new(&TagRule { target: input.target, scope: input.scope.clone(), required: input.required.clone() }).map_err(ApiError::BadRequest)?;
    let recommended = input.recommended.as_deref().and_then(|k| rules::recommended_set().into_iter().find(|r| r.key == k).map(|r| r.key));
    Ok((name, description, rule, recommended))
}

#[derive(sqlx::FromRow)]
struct PolicyRow {
    id: String,
    name: String,
    description: String,
    target: String,
    scope: serde_json::Value,
    required: serde_json::Value,
    enabled: bool,
    recommended: Option<String>,
    created_by: Option<String>,
    created_at: i64,
    updated_at: i64,
}

const POLICY_SELECT: &str = "SELECT p.id::text AS id, p.name, p.description, p.target, p.scope, p.required, p.enabled, p.recommended, u.name AS created_by, \
     (extract(epoch FROM p.created_at) * 1000)::bigint AS created_at, (extract(epoch FROM p.updated_at) * 1000)::bigint AS updated_at \
     FROM tagging_policies p LEFT JOIN users u ON u.id = p.created_by AND u.org_id = p.org_id";

/// A stored policy, or None when the row no longer parses.
fn from_row(r: PolicyRow) -> Option<(TaggingPolicy, CompiledRule)> {
    let target = TagTarget::parse(&r.target)?;
    let scope: TagScope = serde_json::from_value(r.scope).ok()?;
    let required: Vec<RequiredTag> = serde_json::from_value(r.required).ok()?;
    let compiled = CompiledRule::new(&TagRule { target, scope: scope.clone(), required: required.clone() }).ok()?;
    let policy = TaggingPolicy {
        id: r.id,
        name: r.name,
        description: r.description,
        target,
        scope,
        required,
        enabled: r.enabled,
        recommended: r.recommended,
        created_by: r.created_by,
        created_at: r.created_at,
        updated_at: r.updated_at,
        coverage: CoverageCount::default(),
    };
    Some((policy, compiled))
}

/// Installs the recommended set the first time an org's policies are read.
async fn ensure_recommended(st: &AppState, org: &str) -> Result<(), ApiError> {
    let mut tx = st.postgres.begin().await?;
    let fresh = sqlx::query("INSERT INTO tagging_policy_settings (org_id) VALUES ($1) ON CONFLICT (org_id) DO NOTHING").bind(org).execute(&mut *tx).await?.rows_affected() > 0;
    if fresh {
        for p in rules::recommended_set() {
            sqlx::query(
                "INSERT INTO tagging_policies (org_id, name, description, target, scope, required, recommended) VALUES ($1, $2, $3, $4, $5, $6, $7) \
                 ON CONFLICT (org_id, name) DO NOTHING",
            )
            .bind(org)
            .bind(p.name)
            .bind(p.description)
            .bind(p.rule.target.as_str())
            .bind(serde_json::to_value(&p.rule.scope).map_err(anyhow::Error::from)?)
            .bind(serde_json::to_value(&p.rule.required).map_err(anyhow::Error::from)?)
            .bind(p.key)
            .execute(&mut *tx)
            .await?;
        }
    }
    tx.commit().await?;
    Ok(())
}

async fn load_policies(st: &AppState, org: &str) -> Result<Vec<(TaggingPolicy, CompiledRule)>, ApiError> {
    ensure_recommended(st, org).await?;
    let rows: Vec<PolicyRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!("{POLICY_SELECT} WHERE p.org_id = $1 ORDER BY p.target, p.name"))).bind(org).fetch_all(&st.postgres).await?;
    Ok(rows.into_iter().filter_map(from_row).collect())
}

async fn load_policy(st: &AppState, org: &str, id: &str) -> Result<(TaggingPolicy, CompiledRule), ApiError> {
    let not_found = || ApiError::NotFound(format!("Tagging policy {id} not found"));
    if !super::scorecards::valid_uuid(id) {
        return Err(not_found());
    }
    let row: Option<PolicyRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!("{POLICY_SELECT} WHERE p.org_id = $1 AND p.id = $2::uuid"))).bind(org).bind(id).fetch_optional(&st.postgres).await?;
    row.and_then(from_row).ok_or_else(not_found)
}

/// The targets the policies look at.
fn targets_of<'a>(policies: impl Iterator<Item = &'a TaggingPolicy>) -> Vec<TagTarget> {
    let mut t: Vec<TagTarget> = policies.map(|p| p.target).collect();
    t.sort();
    t.dedup();
    t
}

/// The enabled policies' coverage, each policy's count set on it.
fn evaluate(policies: &mut [(TaggingPolicy, CompiledRule)], resources: &[TaggedResource], limit: usize) -> Coverage {
    let enabled: Vec<EvaluatedPolicy> = policies.iter().filter(|(p, _)| p.enabled).map(|(p, c)| EvaluatedPolicy { id: &p.id, name: &p.name, rule: c }).collect();
    let coverage = rules::coverage(&enabled, resources, limit);
    for (p, c) in policies.iter_mut() {
        p.coverage = match coverage.by_policy.iter().find(|r| r.key == p.id) {
            Some(r) => r.count.clone(),
            // Disabled policies are counted alone (they don't take part in the total).
            None => rules::coverage(&[EvaluatedPolicy { id: &p.id, name: &p.name, rule: c }], resources, 0).total,
        };
    }
    coverage
}

/// Writes today's snapshot of a coverage.
async fn store_snapshot(st: &AppState, org: &str, c: &Coverage) -> Result<(), ApiError> {
    let mut rows: Vec<(&str, String, &CoverageCount)> = vec![("total", String::new(), &c.total)];
    for (dimension, list) in [("policy", &c.by_policy), ("target", &c.by_target), ("account", &c.by_account), ("cluster", &c.by_cluster), ("team", &c.by_team)] {
        rows.extend(list.iter().map(|r| (dimension, r.key.clone(), &r.count)));
    }
    let (dims, keys): (Vec<&str>, Vec<String>) = rows.iter().map(|(d, k, _)| (*d, k.clone())).unzip();
    let evaluated: Vec<i32> = rows.iter().map(|(_, _, c)| i32::try_from(c.evaluated).unwrap_or(i32::MAX)).collect();
    let compliant: Vec<i32> = rows.iter().map(|(_, _, c)| i32::try_from(c.compliant).unwrap_or(i32::MAX)).collect();
    let mut tx = st.postgres.begin().await?;
    // Today's rows are replaced whole (a deleted policy or a team gone leaves no stale row).
    sqlx::query("DELETE FROM tagging_coverage_snapshots WHERE org_id = $1 AND day = (now() AT TIME ZONE 'UTC')::date").bind(org).execute(&mut *tx).await?;
    sqlx::query(
        "INSERT INTO tagging_coverage_snapshots (org_id, day, dimension, key, evaluated, compliant) \
         SELECT $1, (now() AT TIME ZONE 'UTC')::date, * FROM UNNEST($2::text[], $3::text[], $4::int[], $5::int[])",
    )
    .bind(org)
    .bind(&dims)
    .bind(&keys)
    .bind(&evaluated)
    .bind(&compliant)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

/// Evaluates every policy of the org and stores today's snapshot.
async fn snapshot_org(st: &AppState, org: &str) -> Result<(), ApiError> {
    let mut policies = load_policies(st, org).await?;
    let resources = provenance::inventory(st, org, &targets_of(policies.iter().map(|(p, _)| p).filter(|p| p.enabled))).await?;
    let coverage = evaluate(&mut policies, &resources, 0);
    store_snapshot(st, org, &coverage).await
}

async fn claim(st: &AppState, limit: i64) -> Result<Vec<String>, ApiError> {
    let mut tx = st.postgres.begin().await?;
    let orgs: Vec<(String,)> = sqlx::query_as(
        "SELECT s.org_id FROM tagging_policy_settings s JOIN orgs o ON o.id = s.org_id WHERE s.next_snapshot_at <= now() AND (s.claimed_until IS NULL OR s.claimed_until < now()) \
         AND o.status = 'active' ORDER BY s.next_snapshot_at LIMIT $1 FOR UPDATE OF s SKIP LOCKED",
    )
    .bind(limit)
    .fetch_all(&mut *tx)
    .await?;
    for (org,) in &orgs {
        sqlx::query("UPDATE tagging_policy_settings SET claimed_until = now() + make_interval(secs => $2), next_snapshot_at = now() + make_interval(mins => $3) WHERE org_id = $1")
            .bind(org)
            .bind(CLAIM_SECONDS)
            .bind(SNAPSHOT_INTERVAL_MINUTES)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok(orgs.into_iter().map(|(o,)| o).collect())
}

/// Refreshes the daily coverage snapshot of every org with tagging policies, hourly.
pub async fn run_snapshots(st: AppState) {
    let mut tick = tokio::time::interval(Duration::from_secs(60));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tick.tick().await;
        let due = match claim(&st, 10).await {
            Ok(due) => due,
            Err(e) => {
                tracing::warn!(error = %e, "claiming tagging coverage snapshots failed");
                continue;
            }
        };
        for org in due {
            if let Err(e) = snapshot_org(&st, &org).await {
                tracing::warn!(org = %org, error = %e, "tagging coverage snapshot failed");
            }
            if let Err(e) = sqlx::query("UPDATE tagging_policy_settings SET claimed_until = NULL WHERE org_id = $1").bind(&org).execute(&st.postgres).await {
                tracing::warn!(org = %org, error = %e, "releasing the tagging coverage claim failed");
            }
        }
    }
}

/// Cloud resources, Kubernetes objects and images failing the org's enabled policies (Cloud Security's
/// [`MISCONFIG_RULE`]), and how many were evaluated.
pub async fn misconfig_gaps(st: &AppState, org: &str) -> Result<(usize, Vec<NonCompliantResource>), ApiError> {
    let mut policies: Vec<(TaggingPolicy, CompiledRule)> = load_policies(st, org).await?.into_iter().filter(|(p, _)| p.enabled && p.target != TagTarget::Service).collect();
    if policies.is_empty() {
        return Ok((0, vec![]));
    }
    let resources = provenance::inventory(st, org, &targets_of(policies.iter().map(|(p, _)| p))).await?;
    let c = evaluate(&mut policies, &resources, usize::MAX);
    Ok((c.total.evaluated, c.noncompliant))
}

/// What a misconfiguration finding says a resource lacks: `tf_repo|git_repo missing; env=production not allowed (Cloud
/// resources name their Terraform source)`.
pub fn misconfig_detail(r: &NonCompliantResource) -> String {
    r.policies.iter().map(|p| format!("{} ({})", p.gaps.iter().map(|g| g.describe()).collect::<Vec<_>>().join(", "), p.policy_name)).collect::<Vec<_>>().join("; ")
}

// ================================================================ handlers

/// The org's tagging policies with their coverage, and the recommended set.
#[utoipa::path(get, path = "/api/v1/tagging-policies", operation_id = "list_tagging_policies", responses((status = 200, body = TaggingPolicyList)), tag = "tagging")]
pub async fn list(State(st): State<AppState>, ctx: AuthContext) -> ApiResult<TaggingPolicyList> {
    let mut policies = load_policies(&st, &ctx.org_id).await?;
    let resources = provenance::inventory(&st, &ctx.org_id, &targets_of(policies.iter().map(|(p, _)| p))).await?;
    evaluate(&mut policies, &resources, 0);
    let recommended = rules::recommended_set().into_iter().map(|r| RecommendedView { installed: policies.iter().any(|(p, _)| p.recommended.as_deref() == Some(r.key)), policy: r }).collect();
    Ok(Json(TaggingPolicyList { policies: policies.into_iter().map(|(p, _)| p).collect(), recommended }))
}

/// Coverage of the enabled policies (or of one policy): per policy, target, account, cluster and team, over time, and
/// the non-compliant resources with what they lack and their source.
#[utoipa::path(get, path = "/api/v1/tagging-policies/coverage", operation_id = "get_tagging_coverage", params(CoverageQuery),
    responses((status = 200, body = TagCoverage), (status = 404, body = crate::error::ErrorBody)), tag = "tagging")]
pub async fn coverage(State(st): State<AppState>, ctx: AuthContext, Query(q): Query<CoverageQuery>) -> ApiResult<TagCoverage> {
    let org = &ctx.org_id;
    let mut policies = match q.policy.as_deref().filter(|p| !p.is_empty()) {
        Some(id) => {
            let (mut p, c) = load_policy(&st, org, id).await?;
            p.enabled = true;
            vec![(p, c)]
        }
        None => load_policies(&st, org).await?.into_iter().filter(|(p, _)| p.enabled).collect(),
    };
    let targets = targets_of(policies.iter().map(|(p, _)| p));
    let (resources, apps) = tokio::try_join!(provenance::inventory(&st, org, &targets), async {
        if targets.contains(&TagTarget::Kubernetes) {
            provenance::argo_apps(&st, org).await
        } else {
            Ok(vec![])
        }
    })?;
    let coverage = evaluate(&mut policies, &resources, MAX_NONCOMPLIANT);
    let in_scope: Vec<TaggedResource> = resources.into_iter().filter(|r| policies.iter().any(|(_, c)| c.applies(r))).collect();
    let (dimension, key) = match q.policy.as_deref().filter(|p| !p.is_empty()) {
        Some(id) => ("policy", id.to_string()),
        None => ("total", String::new()),
    };
    let history: Vec<(String, i32, i32)> = sqlx::query_as(
        "SELECT to_char(day, 'YYYY-MM-DD'), evaluated, compliant FROM tagging_coverage_snapshots WHERE org_id = $1 AND dimension = $2 AND key = $3 \
         AND day > (now() AT TIME ZONE 'UTC')::date - $4 ORDER BY day",
    )
    .bind(org)
    .bind(dimension)
    .bind(&key)
    .bind(HISTORY_DAYS)
    .fetch_all(&st.postgres)
    .await?;
    let history = history.into_iter().map(|(day, e, c)| CoveragePoint { day, count: CoverageCount::of(e.max(0) as usize, c.max(0) as usize) }).collect();
    Ok(Json(TagCoverage { coverage, history, provenance: summarize(&in_scope), argocd_applications: apps.len() }))
}

/// Creates a policy.
#[utoipa::path(post, path = "/api/v1/tagging-policies", operation_id = "create_tagging_policy", request_body = TaggingPolicyInput,
    responses((status = 200, body = TaggingPolicy), (status = 400, body = crate::error::ErrorBody), (status = 403, body = crate::error::ErrorBody)), tag = "tagging")]
pub async fn create(State(st): State<AppState>, Writer(ctx): Writer, Json(input): Json<TaggingPolicyInput>) -> ApiResult<TaggingPolicy> {
    let (name, description, rule, recommended) = validate(&input)?;
    ensure_recommended(&st, &ctx.org_id).await?;
    let mut tx = st.postgres.begin().await?;
    let (count,): (i64,) = sqlx::query_as("SELECT count(*) FROM tagging_policies WHERE org_id = $1").bind(&ctx.org_id).fetch_one(&mut *tx).await?;
    if count >= MAX_POLICIES {
        return Err(ApiError::BadRequest(format!("An organization can have at most {MAX_POLICIES} tagging policies")));
    }
    let (id,): (String,) = sqlx::query_as(
        "INSERT INTO tagging_policies (org_id, name, description, target, scope, required, enabled, recommended, created_by) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9::uuid) RETURNING id::text",
    )
    .bind(&ctx.org_id)
    .bind(&name)
    .bind(&description)
    .bind(rule.rule().target.as_str())
    .bind(serde_json::to_value(&rule.rule().scope).map_err(anyhow::Error::from)?)
    .bind(serde_json::to_value(&rule.rule().required).map_err(anyhow::Error::from)?)
    .bind(input.enabled)
    .bind(recommended)
    .bind(&ctx.user_id)
    .fetch_one(&mut *tx)
    .await
    .map_err(unique_name(&name))?;
    tx.commit().await?;
    let policy = saved(&st, &ctx.org_id, &id).await?;
    ctx.audit(&st, "tagging_policies.created", &id, &format!("{name} ({})", rule.rule().target.as_str())).await;
    Ok(Json(policy))
}

/// One policy with its coverage.
#[utoipa::path(get, path = "/api/v1/tagging-policies/{id}", operation_id = "get_tagging_policy", params(("id" = String, Path)),
    responses((status = 200, body = TaggingPolicy), (status = 404, body = crate::error::ErrorBody)), tag = "tagging")]
pub async fn get(State(st): State<AppState>, ctx: AuthContext, Path(id): Path<String>) -> ApiResult<TaggingPolicy> {
    let (mut policy, rule) = load_policy(&st, &ctx.org_id, &id).await?;
    let resources = provenance::inventory(&st, &ctx.org_id, &[policy.target]).await?;
    policy.coverage = rules::coverage(&[EvaluatedPolicy { id: &policy.id, name: &policy.name, rule: &rule }], &resources, 0).total;
    Ok(Json(policy))
}

/// Replaces a policy's definition.
#[utoipa::path(put, path = "/api/v1/tagging-policies/{id}", operation_id = "update_tagging_policy", params(("id" = String, Path)), request_body = TaggingPolicyInput,
    responses((status = 200, body = TaggingPolicy), (status = 400, body = crate::error::ErrorBody), (status = 404, body = crate::error::ErrorBody)), tag = "tagging")]
pub async fn update(State(st): State<AppState>, Writer(ctx): Writer, Path(id): Path<String>, Json(input): Json<TaggingPolicyInput>) -> ApiResult<TaggingPolicy> {
    load_policy(&st, &ctx.org_id, &id).await?;
    let (name, description, rule, _) = validate(&input)?;
    sqlx::query("UPDATE tagging_policies SET name = $3, description = $4, target = $5, scope = $6, required = $7, enabled = $8, updated_at = now() WHERE org_id = $1 AND id = $2::uuid")
        .bind(&ctx.org_id)
        .bind(&id)
        .bind(&name)
        .bind(&description)
        .bind(rule.rule().target.as_str())
        .bind(serde_json::to_value(&rule.rule().scope).map_err(anyhow::Error::from)?)
        .bind(serde_json::to_value(&rule.rule().required).map_err(anyhow::Error::from)?)
        .bind(input.enabled)
        .execute(&st.postgres)
        .await
        .map_err(unique_name(&name))?;
    let policy = saved(&st, &ctx.org_id, &id).await?;
    let keys: Vec<&str> = rule.rule().required.iter().map(|t| t.key.as_str()).collect();
    ctx.audit(&st, "tagging_policies.updated", &id, &format!("{name}: {}{}", keys.join(", "), if input.enabled { "" } else { " (disabled)" })).await;
    Ok(Json(policy))
}

/// Deletes a policy.
#[utoipa::path(delete, path = "/api/v1/tagging-policies/{id}", operation_id = "delete_tagging_policy", params(("id" = String, Path)),
    responses((status = 204), (status = 404, body = crate::error::ErrorBody)), tag = "tagging")]
pub async fn delete(State(st): State<AppState>, Writer(ctx): Writer, Path(id): Path<String>) -> Result<StatusCode, ApiError> {
    let (policy, _) = load_policy(&st, &ctx.org_id, &id).await?;
    sqlx::query("DELETE FROM tagging_policies WHERE org_id = $1 AND id = $2::uuid").bind(&ctx.org_id).bind(&id).execute(&st.postgres).await?;
    if let Err(e) = snapshot_org(&st, &ctx.org_id).await {
        tracing::warn!(org = %ctx.org_id, error = %e, "tagging coverage snapshot failed");
    }
    ctx.audit(&st, "tagging_policies.deleted", &id, &policy.name).await;
    Ok(StatusCode::NO_CONTENT)
}

/// Re-reads a policy after a write with its coverage, and refreshes today's snapshot.
async fn saved(st: &AppState, org: &str, id: &str) -> Result<TaggingPolicy, ApiError> {
    if let Err(e) = snapshot_org(st, org).await {
        tracing::warn!(org = %org, error = %e, "tagging coverage snapshot failed");
    }
    let (mut policy, rule) = load_policy(st, org, id).await?;
    let resources = provenance::inventory(st, org, &[policy.target]).await?;
    policy.coverage = rules::coverage(&[EvaluatedPolicy { id: &policy.id, name: &policy.name, rule: &rule }], &resources, 0).total;
    Ok(policy)
}

fn unique_name(name: &str) -> impl Fn(sqlx::Error) -> ApiError + '_ {
    move |e| match e {
        sqlx::Error::Database(d) if d.is_unique_violation() => ApiError::BadRequest(format!("A tagging policy named '{name}' already exists")),
        e => e.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::routes::provenance::{from_tags, tag_values};
    use crate::routes::tagging_rules::TagGap;

    fn input(name: &str, required: Vec<RequiredTag>) -> TaggingPolicyInput {
        TaggingPolicyInput { name: name.into(), description: String::new(), target: TagTarget::Cloud, scope: TagScope::default(), required, enabled: true, recommended: None }
    }

    fn req(key: &str) -> RequiredTag {
        RequiredTag { key: key.into(), allowed_values: vec![] }
    }

    fn cloud(name: &str, tags: &[&str]) -> TaggedResource {
        let tags: Vec<String> = tags.iter().map(|t| t.to_string()).collect();
        let team = tag_values(&tags, "team").first().map(|v| v.to_string());
        TaggedResource {
            id: format!("arn:aws:s3:::{name}"),
            target: TagTarget::Cloud,
            name: name.into(),
            resource_type: "AwsS3Bucket".into(),
            provider: Some("aws".into()),
            account: Some("123456789012".into()),
            region: None,
            cluster: None,
            namespace: None,
            env: None,
            team,
            provenance: from_tags(&tags),
            tags,
        }
    }

    fn policy(id: &str, enabled: bool, required: Vec<RequiredTag>) -> (TaggingPolicy, CompiledRule) {
        let rule = CompiledRule::new(&TagRule { target: TagTarget::Cloud, scope: TagScope::default(), required: required.clone() }).unwrap();
        let p = TaggingPolicy {
            id: id.into(),
            name: id.into(),
            description: String::new(),
            target: TagTarget::Cloud,
            scope: TagScope::default(),
            required,
            enabled,
            recommended: None,
            created_by: None,
            created_at: 0,
            updated_at: 0,
            coverage: CoverageCount::default(),
        };
        (p, rule)
    }

    #[test]
    fn inputs_are_trimmed_and_checked() {
        let (name, _, rule, recommended) = validate(&TaggingPolicyInput { recommended: Some("cloud_ownership".into()), ..input("  Owners ", vec![req(" team ")]) }).unwrap();
        assert_eq!((name.as_str(), rule.rule().required[0].key.as_str(), recommended), ("Owners", "team", Some("cloud_ownership")));
        assert!(validate(&input(" ", vec![req("team")])).is_err());
        assert!(validate(&input("x", vec![])).is_err());
        assert!(validate(&TaggingPolicyInput { recommended: Some("nope".into()), ..input("x", vec![req("team")]) }).unwrap().3.is_none());
    }

    #[test]
    fn disabled_policies_are_counted_alone_and_left_out_of_the_total() {
        let mut policies = vec![policy("owners", true, vec![req("team")]), policy("source", false, vec![req("tf_repo")])];
        let resources = vec![cloud("a", &["team:storefront"]), cloud("b", &[])];
        let c = evaluate(&mut policies, &resources, 10);
        assert_eq!(c.total, CoverageCount::of(2, 1));
        assert_eq!(policies[0].0.coverage, CoverageCount::of(2, 1));
        assert_eq!(policies[1].0.coverage, CoverageCount::of(2, 0));
        assert_eq!(c.noncompliant.len(), 1);
        assert_eq!(c.by_policy.len(), 1);
    }

    #[test]
    fn misconfiguration_details_name_each_policy_and_gap() {
        let r = NonCompliantResource {
            resource: cloud("a", &[]),
            policies: vec![
                super::super::tagging_rules::PolicyGaps { policy_id: "1".into(), policy_name: "Source".into(), gaps: vec![TagGap { key: "tf_repo|git_repo".into(), value: None }] },
                super::super::tagging_rules::PolicyGaps { policy_id: "2".into(), policy_name: "Owners".into(), gaps: vec![TagGap { key: "team".into(), value: None }, TagGap { key: "env".into(), value: Some("production".into()) }] },
            ],
        };
        assert_eq!(misconfig_detail(&r), "tf_repo|git_repo missing (Source); team missing, env=production not allowed (Owners)");
    }

    #[test]
    fn targets_are_listed_once() {
        let (a, _) = policy("a", true, vec![req("team")]);
        let (mut b, _) = policy("b", true, vec![req("team")]);
        b.target = TagTarget::Service;
        assert_eq!(targets_of([&a, &b, &a].into_iter()), vec![TagTarget::Cloud, TagTarget::Service]);
    }

    /// Tenancy: policies, settings and snapshots of one org are invisible to another (live Postgres).
    #[tokio::test]
    async fn policies_and_coverage_stay_within_their_org() {
        let Some((_, states)) = crate::test_support::replicas(1).await else { return };
        let st = &states[0];
        let (a, b) = (format!("tag-a-{}", super::super::workload_catalog::random_id()), format!("tag-b-{}", super::super::workload_catalog::random_id()));
        for org in [&a, &b] {
            sqlx::query("INSERT INTO orgs (id, name) VALUES ($1, $1)").bind(org).execute(&st.postgres).await.unwrap();
        }
        // Each org gets the recommended set once; deleting one keeps it deleted.
        let first = load_policies(st, &a).await.unwrap();
        assert_eq!(first.len(), rules::recommended_set().len());
        sqlx::query("DELETE FROM tagging_policies WHERE org_id = $1 AND recommended = 'cloud_ownership'").bind(&a).execute(&st.postgres).await.unwrap();
        assert_eq!(load_policies(st, &a).await.unwrap().len(), first.len() - 1);
        // Another org neither sees nor loads org A's policies.
        let id = &first.iter().find(|(p, _)| p.recommended.as_deref() == Some("images_oci_source")).unwrap().0.id;
        assert!(matches!(load_policy(st, &b, id).await, Err(ApiError::NotFound(_))));
        assert!(load_policy(st, &a, id).await.is_ok());
        assert_eq!(load_policies(st, &b).await.unwrap().len(), first.len());
        // A scanned cloud resource of org A: in A's inventory (every source's query runs) and misconfigurations, not B's.
        sqlx::query(
            "INSERT INTO posture_findings (org_id, provider, account_ref, account_id, check_id, resource_uid, resource_name, resource_type, region, evaluation, severity, resource_tags) \
             VALUES ($1, 'aws', gen_random_uuid(), '123456789012', 's3_bucket_versioning', 'arn:aws:s3:::example-shop-assets', 'example-shop-assets', 'AwsS3Bucket', 'eu-west-1', 'pass', 'low', $2)",
        )
        .bind(&a)
        .bind(serde_json::json!(["managed-by:terraform", "tf_repo:github.com/example-org/infra-terraform", "env:prod"]))
        .execute(&st.postgres)
        .await
        .unwrap();
        let inv = provenance::inventory(st, &a, &TagTarget::ALL).await.unwrap();
        assert_eq!(inv.len(), 1);
        assert_eq!(inv[0].provenance.as_ref().and_then(|p| p.repo.as_deref()), Some("github.com/example-org/infra-terraform"));
        assert!(provenance::inventory(st, &b, &TagTarget::ALL).await.unwrap().is_empty());
        let (evaluated, gaps) = misconfig_gaps(st, &a).await.unwrap();
        assert_eq!((evaluated, gaps.len()), (1, 1), "the bucket lacks tf_path");
        assert_eq!(misconfig_gaps(st, &b).await.unwrap(), (0, vec![]));
        let c = Coverage { total: CoverageCount::of(3, 1), ..Default::default() };
        store_snapshot(st, &a, &c).await.unwrap();
        let count = |org: &str| {
            let org = org.to_string();
            async move { sqlx::query_scalar::<_, i64>("SELECT count(*) FROM tagging_coverage_snapshots WHERE org_id = $1").bind(org).fetch_one(&st.postgres).await.unwrap() }
        };
        assert_eq!((count(&a).await, count(&b).await), (1, 0));
        for org in [&a, &b] {
            sqlx::query("DELETE FROM orgs WHERE id = $1").bind(org).execute(&st.postgres).await.unwrap();
        }
    }
}
