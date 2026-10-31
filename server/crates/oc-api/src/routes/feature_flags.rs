//! Feature flags (Software Delivery → Feature Flags): flags, their per-environment targeting and
//! the environments SDKs evaluate in. The intake answers OpenFeature SDKs over OFREP from these
//! tables (`oc-intake/src/ofrep.rs`); the model and its evaluation are in the `oc-flags` crate.
//!
//! - Anyone in the org reads flags, environments and history; standard users and admins change
//!   them (changing which API keys default to an environment needs an admin, as keys do).
//! - Every flag change bumps its `version`; writes carry the version they were made from and are
//!   refused with 409 when someone else changed the flag meanwhile.
//! - Changes are kept in Flag History (per flag and across flags) and in the audit trail.
//! - The org's first flag creates the Production and Staging environments if it has none.
//! - The intake reports evaluation counts to the internal listener (`/ingest/feature-flag-evaluations`),
//!   written as the `feature_flag.evaluations` metric (tags `flag`, `variant`, `env`, `reason`).

use std::collections::HashSet;
use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use oc_flags::{FlagEnvConfig, FlagServe, FlagType, FlagVariant};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use crate::auth::{Access, AuthContext, Writer};
use crate::error::{ApiError, ApiResult};
use crate::log_metrics::{Point, PointValue, Retention};
use crate::state::AppState;

const MAX_FLAGS: i64 = 1000;
const MAX_ENVIRONMENTS: i64 = 50;
const MAX_NAME_CHARS: usize = 200;
const MAX_DESCRIPTION_CHARS: usize = 2000;
const MAX_TAGS: usize = 50;
const MAX_TAG_CHARS: usize = 200;
const MAX_QUERIES: usize = 50;
const MAX_QUERY_CHARS: usize = 100;
const DEFAULT_HISTORY: i64 = 200;
const MAX_HISTORY: i64 = 1000;

// ================================================================ API types

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct FeatureFlagApiKey {
    pub id: String,
    pub name: String,
    pub prefix: String,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct FeatureFlagEnvironment {
    pub id: String,
    pub name: String,
    /// The `env` values SDKs send that select this environment (`production`, `staging-*`).
    pub queries: Vec<String>,
    pub is_production: bool,
    pub requires_approval: bool,
    /// API keys whose evaluations use this environment when the context names none.
    pub api_keys: Vec<FeatureFlagApiKey>,
    /// Epoch milliseconds.
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct FeatureFlagEnvironmentInput {
    pub name: String,
    #[serde(default)]
    pub queries: Vec<String>,
    #[serde(default)]
    pub is_production: bool,
    #[serde(default)]
    pub requires_approval: bool,
    /// Ingest key ids defaulting to this environment; absent leaves them unchanged (admins only).
    pub api_key_ids: Option<Vec<String>>,
}

#[derive(Serialize, Deserialize, ToSchema, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum FlagSdkAccess {
    /// Client and server SDKs.
    All,
    /// Only client SDKs (OFREP bulk evaluation); their users can see every variant's value.
    Client,
    /// Only server SDKs (OFREP single-flag evaluation).
    Server,
}

#[derive(Serialize, Deserialize, ToSchema, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum FlagLifecycle {
    Temporary,
    Permanent,
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct FeatureFlagInput {
    /// Used in code; can't be changed once created.
    pub key: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub flag_type: FlagType,
    pub variants: Vec<FlagVariant>,
    /// What new environments serve.
    pub default_variant: String,
    pub sdk_access: FlagSdkAccess,
    pub lifecycle: FlagLifecycle,
    #[serde(default)]
    pub teams: Vec<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    /// For updates: the version the change was made from.
    pub version: Option<i32>,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct FeatureFlagEnvState {
    pub environment_id: String,
    pub environment_name: String,
    pub is_production: bool,
    pub config: FlagEnvConfig,
    pub updated_by: Option<String>,
    /// Epoch milliseconds; absent while never configured.
    pub updated_at: Option<i64>,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct FeatureFlag {
    pub id: String,
    pub key: String,
    pub name: String,
    pub description: String,
    pub flag_type: FlagType,
    pub variants: Vec<FlagVariant>,
    pub default_variant: String,
    pub sdk_access: FlagSdkAccess,
    pub lifecycle: FlagLifecycle,
    pub teams: Vec<String>,
    pub tags: Vec<String>,
    pub version: i32,
    /// One per environment of the org, in environment order.
    pub environments: Vec<FeatureFlagEnvState>,
    pub created_by: Option<String>,
    pub updated_by: Option<String>,
    /// Epoch milliseconds.
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Serialize, ToSchema)]
pub struct FeatureFlagsPage {
    pub flags: Vec<FeatureFlag>,
    pub environments: Vec<FeatureFlagEnvironment>,
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct FeatureFlagEnvUpdate {
    /// The flag version the change was made from.
    pub version: i32,
    /// A rollout with `startedAt` 0 starts now.
    pub config: FlagEnvConfig,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct FeatureFlagChange {
    pub id: i64,
    pub flag_id: Option<String>,
    pub flag_key: String,
    pub flag_name: String,
    /// Empty for changes to the flag itself.
    pub environment_name: String,
    /// created, updated, deleted, enabled, disabled, targeting, rollout
    pub action: String,
    pub summary: String,
    pub actor: String,
    /// The flag's version after the change.
    pub version: i32,
    /// Epoch milliseconds.
    pub at: i64,
}

#[derive(Deserialize, IntoParams)]
pub struct FeatureFlagHistoryParams {
    /// Default 200, at most 1000.
    pub limit: Option<i64>,
}

// ================================================================ validation

fn trimmed_list(items: &mut Vec<String>, max: usize, max_chars: usize, what: &str) -> Result<(), String> {
    let mut seen = HashSet::new();
    items.retain_mut(|t| {
        *t = t.trim().to_string();
        !t.is_empty() && seen.insert(t.clone())
    });
    if items.len() > max {
        return Err(format!("At most {max} {what}"));
    }
    if items.iter().any(|t| t.chars().count() > max_chars) {
        return Err(format!("{what} are at most {max_chars} characters"));
    }
    Ok(())
}

fn check_flag(input: &mut FeatureFlagInput) -> Result<(), String> {
    input.key = input.key.trim().to_string();
    input.name = input.name.trim().to_string();
    input.description = input.description.trim().to_string();
    if !oc_flags::valid_key(&input.key) {
        return Err("The flag key is 1–100 characters: lowercase letters, digits, '-', '_' and '.', starting with a letter or digit".into());
    }
    if input.name.is_empty() || input.name.chars().count() > MAX_NAME_CHARS {
        return Err(format!("The flag name is 1–{MAX_NAME_CHARS} characters"));
    }
    if input.description.chars().count() > MAX_DESCRIPTION_CHARS {
        return Err(format!("The description is at most {MAX_DESCRIPTION_CHARS} characters"));
    }
    for v in input.variants.iter_mut() {
        v.key = v.key.trim().to_string();
    }
    oc_flags::validate_variants(input.flag_type, &input.variants)?;
    if !input.variants.iter().any(|v| v.key == input.default_variant) {
        return Err(format!("The default variant '{}' is not one of the variants", input.default_variant));
    }
    trimmed_list(&mut input.teams, MAX_TAGS, MAX_TAG_CHARS, "teams")?;
    trimmed_list(&mut input.tags, MAX_TAGS, MAX_TAG_CHARS, "tags")?;
    Ok(())
}

fn check_environment(input: &mut FeatureFlagEnvironmentInput) -> Result<(), String> {
    input.name = input.name.trim().to_string();
    if input.name.is_empty() || input.name.chars().count() > 100 {
        return Err("The environment name is 1–100 characters".into());
    }
    let mut queries = std::mem::take(&mut input.queries).into_iter().map(|q| q.to_lowercase()).collect();
    trimmed_list(&mut queries, MAX_QUERIES, MAX_QUERY_CHARS, "queries")?;
    if queries.iter().any(|q| q.chars().any(char::is_whitespace)) {
        return Err("Queries are env values without spaces (use * as a wildcard)".into());
    }
    input.queries = queries;
    Ok(())
}

/// A new environment's configuration: off, serving the flag's default variant.
fn initial_config(default_variant: &str) -> FlagEnvConfig {
    FlagEnvConfig { enabled: false, rules: Vec::new(), default_serve: FlagServe::variant(default_variant), rollout: None }
}

// ================================================================ history

fn describe_serve(s: &FlagServe) -> String {
    match &s.variant {
        Some(v) => v.clone(),
        None => s.split.iter().map(|p| format!("{} {}%", p.variant, p.weight)).collect::<Vec<_>>().join(" / "),
    }
}

/// The history entries `(action, summary)` for an environment change.
pub fn describe_env_change(before: &FlagEnvConfig, after: &FlagEnvConfig) -> Vec<(&'static str, String)> {
    let mut out = Vec::new();
    if before.enabled != after.enabled {
        out.push(if after.enabled { ("enabled", "Turned the flag on".to_string()) } else { ("disabled", "Turned the flag off".to_string()) });
    }
    let mut parts = Vec::new();
    if before.rules != after.rules {
        let n = after.rules.len();
        parts.push(format!("{n} targeting rule{}", if n == 1 { "" } else { "s" }));
    }
    if before.default_serve != after.default_serve {
        parts.push(format!("default serves {}", describe_serve(&after.default_serve)));
    }
    if !parts.is_empty() {
        out.push(("targeting", format!("Targeting: {}", parts.join(", "))));
    }
    match (&before.rollout, &after.rollout) {
        (None, Some(r)) => out.push(("rollout", format!("Started a rollout of {} ({} steps)", r.variant, r.steps.len()))),
        (Some(_), None) => out.push(("rollout", "Stopped the rollout".to_string())),
        (Some(a), Some(b)) if a != b => out.push(("rollout", format!("Changed the rollout of {}", b.variant))),
        _ => {}
    }
    out
}

/// The history summary of a change to the flag's details (`None` when nothing changed).
fn describe_flag_change(before: &FeatureFlag, after: &FeatureFlagInput) -> Option<String> {
    let mut changed = Vec::new();
    if before.name != after.name {
        changed.push("name");
    }
    if before.description != after.description {
        changed.push("description");
    }
    if before.variants != after.variants {
        changed.push("variants");
    }
    if before.default_variant != after.default_variant {
        changed.push("default variant");
    }
    if before.sdk_access != after.sdk_access {
        changed.push("SDK access");
    }
    if before.lifecycle != after.lifecycle {
        changed.push("lifecycle");
    }
    if before.teams != after.teams {
        changed.push("teams");
    }
    if before.tags != after.tags {
        changed.push("tags");
    }
    (!changed.is_empty()).then(|| format!("Changed {}", changed.join(", ")))
}

#[allow(clippy::too_many_arguments)]
async fn record(tx: &mut sqlx::PgConnection, ctx: &AuthContext, flag_id: &str, flag_key: &str, environment: &str, action: &str, summary: &str, version: i32) -> Result<(), sqlx::Error> {
    sqlx::query("INSERT INTO feature_flag_changes (org_id, flag_id, flag_key, environment_name, action, summary, actor_id, actor, version) VALUES ($1, $2::uuid, $3, $4, $5, $6, $7::uuid, $8, $9)")
        .bind(&ctx.org_id)
        .bind(flag_id)
        .bind(flag_key)
        .bind(environment)
        .bind(action)
        .bind(summary)
        .bind(&ctx.user_id)
        .bind(&ctx.name)
        .bind(version)
        .execute(tx)
        .await?;
    Ok(())
}

// ================================================================ storage

/// A timestamp column as epoch milliseconds.
fn ms(column: &str) -> String {
    format!("(extract(epoch FROM {column}) * 1000)::bigint")
}

#[derive(sqlx::FromRow)]
struct EnvRow {
    id: String,
    name: String,
    queries: Vec<String>,
    is_production: bool,
    requires_approval: bool,
    created_at: i64,
    updated_at: i64,
}

#[derive(sqlx::FromRow)]
struct KeyRow {
    id: String,
    name: String,
    prefix: String,
    flag_environment_id: String,
}

async fn environments(st: &AppState, org: &str) -> Result<Vec<FeatureFlagEnvironment>, ApiError> {
    let rows: Vec<EnvRow> = sqlx::query_as(&format!(
        "SELECT id::text AS id, name, queries, is_production, requires_approval, {} AS created_at, {} AS updated_at FROM feature_flag_environments WHERE org_id = $1 ORDER BY is_production DESC, lower(name)",
        ms("created_at"),
        ms("updated_at")
    ))
    .bind(org)
    .fetch_all(&st.postgres)
    .await?;
    let keys: Vec<KeyRow> = sqlx::query_as(
        "SELECT id::text AS id, name, prefix, flag_environment_id::text AS flag_environment_id FROM ingest_keys WHERE org_id = $1 AND revoked_at IS NULL AND flag_environment_id IS NOT NULL ORDER BY name",
    )
    .bind(org)
    .fetch_all(&st.postgres)
    .await?;
    Ok(rows
        .into_iter()
        .map(|e| FeatureFlagEnvironment {
            api_keys: keys.iter().filter(|k| k.flag_environment_id == e.id).map(|k| FeatureFlagApiKey { id: k.id.clone(), name: k.name.clone(), prefix: k.prefix.clone() }).collect(),
            id: e.id,
            name: e.name,
            queries: e.queries,
            is_production: e.is_production,
            requires_approval: e.requires_approval,
            created_at: e.created_at,
            updated_at: e.updated_at,
        })
        .collect())
}

#[derive(sqlx::FromRow)]
struct FlagRow {
    id: String,
    key: String,
    name: String,
    description: String,
    flag_type: String,
    variants: sqlx::types::Json<Vec<FlagVariant>>,
    default_variant: String,
    sdk_access: String,
    lifecycle: String,
    teams: Vec<String>,
    tags: Vec<String>,
    version: i32,
    created_by: Option<String>,
    updated_by: Option<String>,
    created_at: i64,
    updated_at: i64,
}

#[derive(sqlx::FromRow)]
struct ConfigRow {
    flag_id: String,
    environment_id: String,
    enabled: bool,
    config: sqlx::types::Json<FlagEnvConfig>,
    updated_by: Option<String>,
    updated_at: i64,
}

fn sdk_access(s: &str) -> FlagSdkAccess {
    match s {
        "client" => FlagSdkAccess::Client,
        "server" => FlagSdkAccess::Server,
        _ => FlagSdkAccess::All,
    }
}

fn sdk_access_str(a: FlagSdkAccess) -> &'static str {
    match a {
        FlagSdkAccess::All => "all",
        FlagSdkAccess::Client => "client",
        FlagSdkAccess::Server => "server",
    }
}

fn lifecycle_str(l: FlagLifecycle) -> &'static str {
    match l {
        FlagLifecycle::Temporary => "temporary",
        FlagLifecycle::Permanent => "permanent",
    }
}

/// Flags of the org (or the one flag `id`) with their configuration in every environment.
async fn load_flags(st: &AppState, org: &str, id: Option<&str>, envs: &[FeatureFlagEnvironment]) -> Result<Vec<FeatureFlag>, ApiError> {
    let flags: Vec<FlagRow> = sqlx::query_as(&format!(
        "SELECT f.id::text AS id, f.key, f.name, f.description, f.flag_type, f.variants, f.default_variant, f.sdk_access, f.lifecycle, f.teams, f.tags, f.version, \
         c.name AS created_by, u.name AS updated_by, {} AS created_at, {} AS updated_at \
         FROM feature_flags f LEFT JOIN users c ON c.id = f.created_by LEFT JOIN users u ON u.id = f.updated_by \
         WHERE f.org_id = $1 AND ($2::uuid IS NULL OR f.id = $2::uuid) ORDER BY f.updated_at DESC",
        ms("f.created_at"),
        ms("f.updated_at")
    ))
    .bind(org)
    .bind(id)
    .fetch_all(&st.postgres)
    .await?;
    let configs: Vec<ConfigRow> = sqlx::query_as(&format!(
        "SELECT c.flag_id::text AS flag_id, c.environment_id::text AS environment_id, c.enabled, c.config, u.name AS updated_by, {} AS updated_at \
         FROM feature_flag_env_configs c LEFT JOIN users u ON u.id = c.updated_by WHERE c.org_id = $1 AND ($2::uuid IS NULL OR c.flag_id = $2::uuid)",
        ms("c.updated_at")
    ))
    .bind(org)
    .bind(id)
    .fetch_all(&st.postgres)
    .await?;
    Ok(flags
        .into_iter()
        .map(|f| {
            let environments = envs
                .iter()
                .map(|e| {
                    let stored = configs.iter().find(|c| c.flag_id == f.id && c.environment_id == e.id);
                    let config = stored.map_or_else(
                        || initial_config(&f.default_variant),
                        |c| {
                            let mut cfg = c.config.0.clone();
                            cfg.enabled = c.enabled;
                            cfg
                        },
                    );
                    FeatureFlagEnvState {
                        environment_id: e.id.clone(),
                        environment_name: e.name.clone(),
                        is_production: e.is_production,
                        config,
                        updated_by: stored.and_then(|c| c.updated_by.clone()),
                        updated_at: stored.map(|c| c.updated_at),
                    }
                })
                .collect();
            FeatureFlag {
                flag_type: FlagType::parse(&f.flag_type).unwrap_or(FlagType::Boolean),
                sdk_access: sdk_access(&f.sdk_access),
                lifecycle: if f.lifecycle == "permanent" { FlagLifecycle::Permanent } else { FlagLifecycle::Temporary },
                id: f.id,
                key: f.key,
                name: f.name,
                description: f.description,
                variants: f.variants.0,
                default_variant: f.default_variant,
                teams: f.teams,
                tags: f.tags,
                version: f.version,
                environments,
                created_by: f.created_by,
                updated_by: f.updated_by,
                created_at: f.created_at,
                updated_at: f.updated_at,
            }
        })
        .collect())
}

async fn one_flag(st: &AppState, org: &str, id: &str) -> Result<FeatureFlag, ApiError> {
    if uuid_like(id) {
        let envs = environments(st, org).await?;
        if let Some(f) = load_flags(st, org, Some(id), &envs).await?.pop() {
            return Ok(f);
        }
    }
    Err(ApiError::NotFound("feature flag not found".into()))
}

fn uuid_like(id: &str) -> bool {
    id.len() == 36 && id.chars().all(|c| c.is_ascii_hexdigit() || c == '-')
}

/// The 409 for a write made from an older version.
fn stale(current: i32) -> ApiError {
    ApiError::Conflict(format!("This flag was changed by someone else meanwhile (now version {current}). Reload it and make the change again."))
}

async fn ensure_environments(tx: &mut sqlx::PgConnection, ctx: &AuthContext) -> Result<(), sqlx::Error> {
    let (n,): (i64,) = sqlx::query_as("SELECT count(*) FROM feature_flag_environments WHERE org_id = $1").bind(&ctx.org_id).fetch_one(&mut *tx).await?;
    if n > 0 {
        return Ok(());
    }
    for (name, queries, production) in [("Production", vec!["production", "prod"], true), ("Staging", vec!["staging", "staging-*"], false)] {
        sqlx::query("INSERT INTO feature_flag_environments (org_id, name, queries, is_production, created_by) VALUES ($1, $2, $3, $4, $5::uuid)")
            .bind(&ctx.org_id)
            .bind(name)
            .bind(queries)
            .bind(production)
            .bind(&ctx.user_id)
            .execute(&mut *tx)
            .await?;
    }
    Ok(())
}

// ================================================================ handlers: flags

/// Every flag of the org with its state per environment, and the environments.
#[utoipa::path(get, path = "/api/v1/feature-flags", operation_id = "list_feature_flags", responses((status = 200, body = FeatureFlagsPage)), tag = "feature_flags")]
pub async fn list(State(st): State<AppState>, ctx: AuthContext) -> ApiResult<FeatureFlagsPage> {
    let envs = environments(&st, &ctx.org_id).await?;
    let flags = load_flags(&st, &ctx.org_id, None, &envs).await?;
    Ok(Json(FeatureFlagsPage { flags, environments: envs }))
}

#[utoipa::path(get, path = "/api/v1/feature-flags/{id}", operation_id = "get_feature_flag", params(("id" = String, Path)),
    responses((status = 200, body = FeatureFlag), (status = 404, body = crate::error::ErrorBody)), tag = "feature_flags")]
pub async fn get_one(State(st): State<AppState>, ctx: AuthContext, Path(id): Path<String>) -> ApiResult<FeatureFlag> {
    Ok(Json(one_flag(&st, &ctx.org_id, &id).await?))
}

/// Creates a flag, off in every environment and serving its default variant there.
#[utoipa::path(post, path = "/api/v1/feature-flags", operation_id = "create_feature_flag", request_body = FeatureFlagInput,
    responses((status = 200, body = FeatureFlag), (status = 400, body = crate::error::ErrorBody)), tag = "feature_flags")]
pub async fn create(State(st): State<AppState>, Writer(ctx): Writer, Json(mut input): Json<FeatureFlagInput>) -> ApiResult<FeatureFlag> {
    check_flag(&mut input).map_err(ApiError::BadRequest)?;
    let (count,): (i64,) = sqlx::query_as("SELECT count(*) FROM feature_flags WHERE org_id = $1").bind(&ctx.org_id).fetch_one(&st.postgres).await?;
    if count >= MAX_FLAGS {
        return Err(ApiError::BadRequest(format!("An organization can have at most {MAX_FLAGS} feature flags")));
    }
    let mut tx = st.postgres.begin().await?;
    ensure_environments(&mut tx, &ctx).await?;
    let (id,): (String,) = sqlx::query_as(
        "INSERT INTO feature_flags (org_id, key, name, description, flag_type, variants, default_variant, sdk_access, lifecycle, teams, tags, created_by, updated_by) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12::uuid, $12::uuid) RETURNING id::text",
    )
    .bind(&ctx.org_id)
    .bind(&input.key)
    .bind(&input.name)
    .bind(&input.description)
    .bind(input.flag_type.as_str())
    .bind(sqlx::types::Json(&input.variants))
    .bind(&input.default_variant)
    .bind(sdk_access_str(input.sdk_access))
    .bind(lifecycle_str(input.lifecycle))
    .bind(&input.teams)
    .bind(&input.tags)
    .bind(&ctx.user_id)
    .fetch_one(&mut *tx)
    .await
    .map_err(|e| match &e {
        sqlx::Error::Database(d) if d.is_unique_violation() => ApiError::BadRequest(format!("A flag with the key '{}' already exists", input.key)),
        _ => e.into(),
    })?;
    record(&mut tx, &ctx, &id, &input.key, "", "created", &format!("Created the {} flag {}", input.flag_type.as_str(), input.name), 1).await?;
    tx.commit().await?;
    ctx.audit(&st, "feature_flag.created", &input.key, &input.name).await;
    Ok(Json(one_flag(&st, &ctx.org_id, &id).await?))
}

/// Changes a flag's details and variants (its key stays). Environments that serve a removed
/// variant must be changed first.
#[utoipa::path(put, path = "/api/v1/feature-flags/{id}", operation_id = "update_feature_flag", params(("id" = String, Path)), request_body = FeatureFlagInput,
    responses((status = 200, body = FeatureFlag), (status = 400, body = crate::error::ErrorBody), (status = 404, body = crate::error::ErrorBody), (status = 409, body = crate::error::ErrorBody)), tag = "feature_flags")]
pub async fn update(State(st): State<AppState>, Writer(ctx): Writer, Path(id): Path<String>, Json(mut input): Json<FeatureFlagInput>) -> ApiResult<FeatureFlag> {
    let current = one_flag(&st, &ctx.org_id, &id).await?;
    input.key = current.key.clone();
    check_flag(&mut input).map_err(ApiError::BadRequest)?;
    let Some(version) = input.version else {
        return Err(ApiError::BadRequest("version is required".into()));
    };
    if input.flag_type != current.flag_type {
        return Err(ApiError::BadRequest("A flag's type can't be changed".into()));
    }
    for e in &current.environments {
        oc_flags::validate_config(&e.config, &input.variants).map_err(|m| ApiError::BadRequest(format!("{}: {m}", e.environment_name)))?;
    }
    let Some(summary) = describe_flag_change(&current, &input) else {
        return Ok(Json(current));
    };
    let mut tx = st.postgres.begin().await?;
    let next: Option<(i32,)> = sqlx::query_as(
        "UPDATE feature_flags SET name = $4, description = $5, variants = $6, default_variant = $7, sdk_access = $8, lifecycle = $9, teams = $10, tags = $11, \
         updated_by = $12::uuid, updated_at = now(), version = version + 1 WHERE org_id = $1 AND id = $2::uuid AND version = $3 RETURNING version",
    )
    .bind(&ctx.org_id)
    .bind(&id)
    .bind(version)
    .bind(&input.name)
    .bind(&input.description)
    .bind(sqlx::types::Json(&input.variants))
    .bind(&input.default_variant)
    .bind(sdk_access_str(input.sdk_access))
    .bind(lifecycle_str(input.lifecycle))
    .bind(&input.teams)
    .bind(&input.tags)
    .bind(&ctx.user_id)
    .fetch_optional(&mut *tx)
    .await?;
    let Some((next,)) = next else {
        return Err(stale(current.version));
    };
    record(&mut tx, &ctx, &id, &current.key, "", "updated", &summary, next).await?;
    tx.commit().await?;
    ctx.audit(&st, "feature_flag.updated", &current.key, &summary).await;
    Ok(Json(one_flag(&st, &ctx.org_id, &id).await?))
}

#[utoipa::path(delete, path = "/api/v1/feature-flags/{id}", operation_id = "delete_feature_flag", params(("id" = String, Path)),
    responses((status = 204), (status = 404, body = crate::error::ErrorBody)), tag = "feature_flags")]
pub async fn delete(State(st): State<AppState>, Writer(ctx): Writer, Path(id): Path<String>) -> Result<StatusCode, ApiError> {
    let current = one_flag(&st, &ctx.org_id, &id).await?;
    let mut tx = st.postgres.begin().await?;
    record(&mut tx, &ctx, &id, &current.key, "", "deleted", &format!("Deleted {}", current.name), current.version).await?;
    sqlx::query("DELETE FROM feature_flags WHERE org_id = $1 AND id = $2::uuid").bind(&ctx.org_id).bind(&id).execute(&mut *tx).await?;
    tx.commit().await?;
    ctx.audit(&st, "feature_flag.deleted", &current.key, &current.name).await;
    Ok(StatusCode::NO_CONTENT)
}

/// Sets a flag's configuration in one environment: on or off, targeting rules, the default and
/// a progressive rollout.
#[utoipa::path(put, path = "/api/v1/feature-flags/{id}/environments/{environment_id}", operation_id = "update_feature_flag_environment",
    params(("id" = String, Path), ("environment_id" = String, Path)), request_body = FeatureFlagEnvUpdate,
    responses((status = 200, body = FeatureFlag), (status = 400, body = crate::error::ErrorBody), (status = 404, body = crate::error::ErrorBody), (status = 409, body = crate::error::ErrorBody)), tag = "feature_flags")]
pub async fn update_environment(
    State(st): State<AppState>,
    Writer(ctx): Writer,
    Path((id, environment_id)): Path<(String, String)>,
    Json(input): Json<FeatureFlagEnvUpdate>,
) -> ApiResult<FeatureFlag> {
    let current = one_flag(&st, &ctx.org_id, &id).await?;
    let Some(env) = current.environments.iter().find(|e| e.environment_id == environment_id) else {
        return Err(ApiError::NotFound("environment not found".into()));
    };
    let mut cfg = input.config;
    oc_flags::validate_config(&cfg, &current.variants).map_err(ApiError::BadRequest)?;
    if let Some(r) = cfg.rollout.as_mut() {
        if r.started_at <= 0 {
            r.started_at = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(1, |d| d.as_millis() as i64);
        }
    }
    let changes = describe_env_change(&env.config, &cfg);
    if changes.is_empty() {
        return Ok(Json(current));
    }
    let mut tx = st.postgres.begin().await?;
    let next: Option<(i32,)> =
        sqlx::query_as("UPDATE feature_flags SET updated_by = $4::uuid, updated_at = now(), version = version + 1 WHERE org_id = $1 AND id = $2::uuid AND version = $3 RETURNING version")
            .bind(&ctx.org_id)
            .bind(&id)
            .bind(input.version)
            .bind(&ctx.user_id)
            .fetch_optional(&mut *tx)
            .await?;
    let Some((next,)) = next else {
        return Err(stale(current.version));
    };
    let enabled = cfg.enabled;
    sqlx::query(
        "INSERT INTO feature_flag_env_configs (flag_id, environment_id, org_id, enabled, config, updated_by) \
         SELECT $2::uuid, e.id, $1, $4, $5, $6::uuid FROM feature_flag_environments e WHERE e.org_id = $1 AND e.id = $3::uuid \
         ON CONFLICT (flag_id, environment_id) DO UPDATE SET enabled = EXCLUDED.enabled, config = EXCLUDED.config, updated_by = EXCLUDED.updated_by, updated_at = now()",
    )
    .bind(&ctx.org_id)
    .bind(&id)
    .bind(&environment_id)
    .bind(enabled)
    .bind(sqlx::types::Json(&cfg))
    .bind(&ctx.user_id)
    .execute(&mut *tx)
    .await?;
    for (action, summary) in &changes {
        record(&mut tx, &ctx, &id, &current.key, &env.environment_name, action, summary, next).await?;
    }
    tx.commit().await?;
    let detail = changes.iter().map(|(_, s)| s.as_str()).collect::<Vec<_>>().join("; ");
    ctx.audit(&st, "feature_flag.environment_updated", &format!("{} ({})", current.key, env.environment_name), &detail).await;
    Ok(Json(one_flag(&st, &ctx.org_id, &id).await?))
}

async fn history_of(st: &AppState, org: &str, flag: Option<&str>, limit: Option<i64>) -> Result<Vec<FeatureFlagChange>, ApiError> {
    #[derive(sqlx::FromRow)]
    struct Row {
        id: i64,
        flag_id: Option<String>,
        flag_key: String,
        flag_name: String,
        environment_name: String,
        action: String,
        summary: String,
        actor: String,
        version: i32,
        at: i64,
    }
    let rows: Vec<Row> = sqlx::query_as(&format!(
        "SELECT c.id, c.flag_id::text AS flag_id, c.flag_key, COALESCE(f.name, c.flag_key) AS flag_name, c.environment_name, c.action, c.summary, c.actor, c.version, {} AS at \
         FROM feature_flag_changes c LEFT JOIN feature_flags f ON f.id = c.flag_id AND f.org_id = c.org_id \
         WHERE c.org_id = $1 AND ($2::uuid IS NULL OR c.flag_id = $2::uuid) ORDER BY c.at DESC, c.id DESC LIMIT $3",
        ms("c.at")
    ))
    .bind(org)
    .bind(flag)
    .bind(limit.unwrap_or(DEFAULT_HISTORY).clamp(1, MAX_HISTORY))
    .fetch_all(&st.postgres)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| FeatureFlagChange {
            id: r.id,
            flag_id: r.flag_id,
            flag_key: r.flag_key,
            flag_name: r.flag_name,
            environment_name: r.environment_name,
            action: r.action,
            summary: r.summary,
            actor: r.actor,
            version: r.version,
            at: r.at,
        })
        .collect())
}

/// Global Flag History: changes across every flag, newest first.
#[utoipa::path(get, path = "/api/v1/feature-flags/history", operation_id = "list_feature_flag_history", params(FeatureFlagHistoryParams),
    responses((status = 200, body = Vec<FeatureFlagChange>)), tag = "feature_flags")]
pub async fn history(State(st): State<AppState>, ctx: AuthContext, Query(p): Query<FeatureFlagHistoryParams>) -> ApiResult<Vec<FeatureFlagChange>> {
    Ok(Json(history_of(&st, &ctx.org_id, None, p.limit).await?))
}

#[utoipa::path(get, path = "/api/v1/feature-flags/{id}/history", operation_id = "get_feature_flag_history", params(("id" = String, Path), FeatureFlagHistoryParams),
    responses((status = 200, body = Vec<FeatureFlagChange>), (status = 404, body = crate::error::ErrorBody)), tag = "feature_flags")]
pub async fn flag_history(State(st): State<AppState>, ctx: AuthContext, Path(id): Path<String>, Query(p): Query<FeatureFlagHistoryParams>) -> ApiResult<Vec<FeatureFlagChange>> {
    if !uuid_like(&id) {
        return Err(ApiError::NotFound("feature flag not found".into()));
    }
    Ok(Json(history_of(&st, &ctx.org_id, Some(&id), p.limit).await?))
}

// ================================================================ handlers: environments

#[utoipa::path(get, path = "/api/v1/feature-flags/environments", operation_id = "list_feature_flag_environments", responses((status = 200, body = Vec<FeatureFlagEnvironment>)), tag = "feature_flags")]
pub async fn list_environments(State(st): State<AppState>, ctx: AuthContext) -> ApiResult<Vec<FeatureFlagEnvironment>> {
    Ok(Json(environments(&st, &ctx.org_id).await?))
}

/// Points the given API keys (and only them) at the environment.
async fn set_default_keys(tx: &mut sqlx::PgConnection, org: &str, env: &str, keys: &[String]) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE ingest_keys SET flag_environment_id = NULL WHERE org_id = $1 AND flag_environment_id = $2::uuid AND NOT (id::text = ANY($3))")
        .bind(org)
        .bind(env)
        .bind(keys)
        .execute(&mut *tx)
        .await?;
    sqlx::query("UPDATE ingest_keys SET flag_environment_id = $2::uuid WHERE org_id = $1 AND id::text = ANY($3) AND revoked_at IS NULL").bind(org).bind(env).bind(keys).execute(&mut *tx).await?;
    Ok(())
}

fn key_changes_allowed(ctx: &AuthContext, current: &[FeatureFlagApiKey], wanted: Option<&Vec<String>>) -> Result<(), ApiError> {
    let Some(wanted) = wanted else { return Ok(()) };
    let have: HashSet<&str> = current.iter().map(|k| k.id.as_str()).collect();
    let want: HashSet<&str> = wanted.iter().map(String::as_str).collect();
    if have != want && ctx.access < Access::Admin {
        return Err(ApiError::Forbidden("organization admin role required to change API key defaults".into()));
    }
    Ok(())
}

fn env_error(e: sqlx::Error, name: &str) -> ApiError {
    match &e {
        sqlx::Error::Database(d) if d.is_unique_violation() => ApiError::BadRequest(format!("An environment named '{name}' already exists")),
        _ => e.into(),
    }
}

#[utoipa::path(post, path = "/api/v1/feature-flags/environments", operation_id = "create_feature_flag_environment", request_body = FeatureFlagEnvironmentInput,
    responses((status = 200, body = FeatureFlagEnvironment), (status = 400, body = crate::error::ErrorBody)), tag = "feature_flags")]
pub async fn create_environment(State(st): State<AppState>, Writer(ctx): Writer, Json(mut input): Json<FeatureFlagEnvironmentInput>) -> ApiResult<FeatureFlagEnvironment> {
    check_environment(&mut input).map_err(ApiError::BadRequest)?;
    key_changes_allowed(&ctx, &[], input.api_key_ids.as_ref())?;
    let (count,): (i64,) = sqlx::query_as("SELECT count(*) FROM feature_flag_environments WHERE org_id = $1").bind(&ctx.org_id).fetch_one(&st.postgres).await?;
    if count >= MAX_ENVIRONMENTS {
        return Err(ApiError::BadRequest(format!("An organization can have at most {MAX_ENVIRONMENTS} environments")));
    }
    let mut tx = st.postgres.begin().await?;
    let (id,): (String,) =
        sqlx::query_as("INSERT INTO feature_flag_environments (org_id, name, queries, is_production, requires_approval, created_by) VALUES ($1, $2, $3, $4, $5, $6::uuid) RETURNING id::text")
            .bind(&ctx.org_id)
            .bind(&input.name)
            .bind(&input.queries)
            .bind(input.is_production)
            .bind(input.requires_approval)
            .bind(&ctx.user_id)
            .fetch_one(&mut *tx)
            .await
            .map_err(|e| env_error(e, &input.name))?;
    if let Some(keys) = &input.api_key_ids {
        set_default_keys(&mut tx, &ctx.org_id, &id, keys).await?;
    }
    tx.commit().await?;
    ctx.audit(&st, "feature_flag_environment.created", &input.name, &input.queries.join(", ")).await;
    environments(&st, &ctx.org_id).await?.into_iter().find(|e| e.id == id).map(Json).ok_or_else(|| ApiError::NotFound("environment not found".into()))
}

#[utoipa::path(put, path = "/api/v1/feature-flags/environments/{id}", operation_id = "update_feature_flag_environment_settings", params(("id" = String, Path)), request_body = FeatureFlagEnvironmentInput,
    responses((status = 200, body = FeatureFlagEnvironment), (status = 400, body = crate::error::ErrorBody), (status = 404, body = crate::error::ErrorBody)), tag = "feature_flags")]
pub async fn update_environment_settings(
    State(st): State<AppState>,
    Writer(ctx): Writer,
    Path(id): Path<String>,
    Json(mut input): Json<FeatureFlagEnvironmentInput>,
) -> ApiResult<FeatureFlagEnvironment> {
    check_environment(&mut input).map_err(ApiError::BadRequest)?;
    let Some(current) = environments(&st, &ctx.org_id).await?.into_iter().find(|e| e.id == id) else {
        return Err(ApiError::NotFound("environment not found".into()));
    };
    key_changes_allowed(&ctx, &current.api_keys, input.api_key_ids.as_ref())?;
    let mut tx = st.postgres.begin().await?;
    sqlx::query("UPDATE feature_flag_environments SET name = $3, queries = $4, is_production = $5, requires_approval = $6, updated_at = now() WHERE org_id = $1 AND id = $2::uuid")
        .bind(&ctx.org_id)
        .bind(&id)
        .bind(&input.name)
        .bind(&input.queries)
        .bind(input.is_production)
        .bind(input.requires_approval)
        .execute(&mut *tx)
        .await
        .map_err(|e| env_error(e, &input.name))?;
    if let Some(keys) = &input.api_key_ids {
        set_default_keys(&mut tx, &ctx.org_id, &id, keys).await?;
    }
    tx.commit().await?;
    ctx.audit(&st, "feature_flag_environment.updated", &input.name, &input.queries.join(", ")).await;
    environments(&st, &ctx.org_id).await?.into_iter().find(|e| e.id == id).map(Json).ok_or_else(|| ApiError::NotFound("environment not found".into()))
}

/// Deletes an environment and every flag's configuration in it.
#[utoipa::path(delete, path = "/api/v1/feature-flags/environments/{id}", operation_id = "delete_feature_flag_environment", params(("id" = String, Path)),
    responses((status = 204), (status = 404, body = crate::error::ErrorBody)), tag = "feature_flags")]
pub async fn delete_environment(State(st): State<AppState>, Writer(ctx): Writer, Path(id): Path<String>) -> Result<StatusCode, ApiError> {
    let name: Option<(String,)> = if uuid_like(&id) {
        sqlx::query_as("DELETE FROM feature_flag_environments WHERE org_id = $1 AND id = $2::uuid RETURNING name").bind(&ctx.org_id).bind(&id).fetch_optional(&st.postgres).await?
    } else {
        None
    };
    let Some((name,)) = name else {
        return Err(ApiError::NotFound("environment not found".into()));
    };
    ctx.audit(&st, "feature_flag_environment.deleted", &name, "").await;
    Ok(StatusCode::NO_CONTENT)
}

// ================================================================ evaluation counts (internal)

#[derive(Deserialize)]
pub struct EvaluationCount {
    flag: String,
    #[serde(default)]
    variant: String,
    #[serde(default)]
    environment: String,
    #[serde(default)]
    reason: String,
    count: u64,
}

#[derive(Deserialize)]
pub struct EvaluationCounts {
    evaluations: Vec<EvaluationCount>,
}

/// The `feature_flag.evaluations` points of one org's counts, in the minute of `at_ms`.
fn evaluation_points(org: &str, counts: &[EvaluationCount], at_ms: i64, retention: Retention) -> Vec<Point> {
    let description: Arc<str> = Arc::from("Feature flag evaluations (OFREP)");
    let minute = at_ms - at_ms.rem_euclid(60_000);
    counts
        .iter()
        .filter(|c| c.count > 0 && !c.flag.is_empty())
        .map(|c| {
            let tags = [("flag", &c.flag), ("variant", &c.variant), ("env", &c.environment), ("reason", &c.reason)]
                .into_iter()
                .filter(|(_, v)| !v.is_empty())
                .map(|(k, v)| (k.to_string(), v.chars().take(200).collect::<String>()))
                .collect();
            Point {
                event_id: String::new(),
                org: org.to_string(),
                metric: "feature_flag.evaluations".into(),
                description: description.clone(),
                retention,
                tags,
                time_ms: minute,
                value: PointValue::Count(c.count),
            }
        })
        .collect()
}

/// `POST /ingest/feature-flag-evaluations` on the internal listener: evaluation counts from the
/// intake (which sets `x-oc-org` after checking the API key).
pub async fn ingest_evaluations(State(st): State<AppState>, headers: HeaderMap, Json(body): Json<EvaluationCounts>) -> StatusCode {
    if let Some(token) = &st.config.internal_token {
        let ok = headers.get("authorization").and_then(|v| v.to_str().ok()).and_then(|v| v.strip_prefix("Bearer ")) == Some(token.as_str());
        if !ok {
            return StatusCode::UNAUTHORIZED;
        }
    }
    let Some(org) = headers.get("x-oc-org").and_then(|v| v.to_str().ok()).filter(|o| !o.is_empty()).map(str::to_string) else {
        return StatusCode::BAD_REQUEST;
    };
    let retention: Option<(i32, i32, i32)> =
        sqlx::query_as("SELECT COALESCE(metrics_retention_days, 0), COALESCE(metrics_5m_days, 0), COALESCE(metrics_1h_days, 0) FROM org_data_settings WHERE org_id = $1")
            .bind(&org)
            .fetch_optional(&st.postgres)
            .await
            .unwrap_or(None);
    let retention = retention.map(|(raw, rollup_5m, rollup_1h)| Retention { raw, rollup_5m, rollup_1h }).unwrap_or_default();
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64);
    let points = evaluation_points(&org, &body.evaluations, now, retention);
    let (sums, _) = crate::log_metrics::rows_in_scope(&points, "observecraft.feature_flags");
    if sums.is_empty() {
        return StatusCode::OK;
    }
    match crate::logproc::insert_rows(&st, "otel.metrics_ingest_sum", sums).await {
        Ok(()) => StatusCode::OK,
        Err(e) => {
            tracing::warn!(error = %e, "writing feature flag evaluation metrics failed");
            StatusCode::SERVICE_UNAVAILABLE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oc_flags::{FlagCondition, FlagOperator, FlagRollout, FlagRolloutStep, FlagRule, FlagSplit};
    use serde_json::json;

    fn input() -> FeatureFlagInput {
        serde_json::from_value(json!({
            "key": " checkout-new-payment-flow ", "name": " New payment flow ", "flagType": "boolean",
            "variants": [{"key": "on", "value": true}, {"key": " off ", "value": false}], "defaultVariant": "off",
            "sdkAccess": "all", "lifecycle": "temporary", "teams": ["payments", " payments ", ""], "tags": ["service:storefront-api"]
        }))
        .unwrap()
    }

    #[test]
    fn flag_input_is_trimmed_and_checked() {
        let mut i = input();
        check_flag(&mut i).unwrap();
        assert_eq!((i.key.as_str(), i.name.as_str(), i.variants[1].key.as_str()), ("checkout-new-payment-flow", "New payment flow", "off"));
        assert_eq!(i.teams, ["payments"]);
        let mut bad = input();
        bad.key = "Checkout Flow".into();
        assert!(check_flag(&mut bad).is_err());
        let mut bad = input();
        bad.default_variant = "maybe".into();
        assert_eq!(check_flag(&mut bad), Err("The default variant 'maybe' is not one of the variants".into()));
        let mut bad = input();
        bad.variants[0].value = json!("yes");
        assert!(check_flag(&mut bad).is_err());
    }

    #[test]
    fn environment_input() {
        let mut e = FeatureFlagEnvironmentInput {
            name: " Staging EU ".into(),
            queries: vec![" Staging-EU ".into(), "staging-eu".into(), "".into()],
            is_production: false,
            requires_approval: false,
            api_key_ids: None,
        };
        check_environment(&mut e).unwrap();
        assert_eq!((e.name.as_str(), e.queries.clone()), ("Staging EU", vec!["staging-eu".to_string()]));
        e.queries = vec!["dev local".into()];
        assert!(check_environment(&mut e).is_err());
    }

    fn config(enabled: bool) -> FlagEnvConfig {
        FlagEnvConfig { enabled, ..initial_config("off") }
    }

    #[test]
    fn environment_changes_are_described() {
        assert!(describe_env_change(&config(false), &config(false)).is_empty());
        assert_eq!(describe_env_change(&config(false), &config(true)), vec![("enabled", "Turned the flag on".to_string())]);
        assert_eq!(describe_env_change(&config(true), &config(false)), vec![("disabled", "Turned the flag off".to_string())]);
        let mut targeted = config(true);
        targeted.rules.push(FlagRule {
            name: "staff".into(),
            conditions: vec![FlagCondition { attribute: "email".into(), operator: FlagOperator::EndsWith, values: vec!["@example.com".into()] }],
            serve: FlagServe::variant("on"),
        });
        targeted.default_serve = FlagServe { variant: None, split: vec![FlagSplit { variant: "on".into(), weight: 25.0 }, FlagSplit { variant: "off".into(), weight: 75.0 }] };
        assert_eq!(describe_env_change(&config(true), &targeted), vec![("targeting", "Targeting: 1 targeting rule, default serves on 25% / off 75%".to_string())]);
        let mut rolling = config(true);
        rolling.rollout =
            Some(FlagRollout { variant: "on".into(), steps: vec![FlagRolloutStep { percent: 10.0, duration_minutes: 60 }, FlagRolloutStep { percent: 100.0, duration_minutes: 0 }], started_at: 5 });
        assert_eq!(describe_env_change(&config(true), &rolling), vec![("rollout", "Started a rollout of on (2 steps)".to_string())]);
        assert_eq!(describe_env_change(&rolling, &config(true)), vec![("rollout", "Stopped the rollout".to_string())]);
    }

    #[test]
    fn flag_changes_are_described() {
        let mut i = input();
        check_flag(&mut i).unwrap();
        let before = FeatureFlag {
            id: "x".into(),
            key: i.key.clone(),
            name: i.name.clone(),
            description: String::new(),
            flag_type: FlagType::Boolean,
            variants: i.variants.clone(),
            default_variant: "off".into(),
            sdk_access: FlagSdkAccess::All,
            lifecycle: FlagLifecycle::Temporary,
            teams: i.teams.clone(),
            tags: i.tags.clone(),
            version: 1,
            environments: vec![],
            created_by: None,
            updated_by: None,
            created_at: 0,
            updated_at: 0,
        };
        assert_eq!(describe_flag_change(&before, &i), None);
        i.name = "Payment flow v2".into();
        i.sdk_access = FlagSdkAccess::Server;
        assert_eq!(describe_flag_change(&before, &i).as_deref(), Some("Changed name, SDK access"));
    }

    #[test]
    fn evaluation_counts_become_minute_points() {
        let counts = vec![
            EvaluationCount { flag: "catalog-search-v2".into(), variant: "on".into(), environment: "Production".into(), reason: "SPLIT".into(), count: 7 },
            EvaluationCount { flag: "catalog-search-v2".into(), variant: String::new(), environment: "Staging".into(), reason: "DISABLED".into(), count: 2 },
            EvaluationCount { flag: String::new(), variant: "x".into(), environment: String::new(), reason: String::new(), count: 1 },
        ];
        let points = evaluation_points("acme", &counts, 1_800_000_123_456, Retention::default());
        assert_eq!(points.len(), 2);
        assert_eq!(points[0].time_ms, 1_800_000_120_000);
        assert_eq!(points[0].metric, "feature_flag.evaluations");
        assert_eq!(points[0].value, PointValue::Count(7));
        assert_eq!(
            points[0].tags,
            vec![("flag".to_string(), "catalog-search-v2".to_string()), ("variant".into(), "on".into()), ("env".into(), "Production".into()), ("reason".into(), "SPLIT".into())]
        );
        // No variant tag when the flag was off (the code default was served).
        assert!(!points[1].tags.iter().any(|(k, _)| k == "variant"));
    }
}
