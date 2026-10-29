//! Instance administration: the operators of an install create, suspend and delete organizations.
//! Only accounts with `is_instance_admin` (from `OC_INSTANCE_ADMIN_EMAIL`) reach these endpoints.
//!
//! A new org gets the built-in roles, default login settings, the built-in dashboards and a pending
//! first admin whose one-time invite link is returned (mail delivery isn't configured yet).

use std::collections::HashMap;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use super::log_indexes::{install_metric_retention, validate_metric_limits, MetricRetention, StoredMetricRetention};
use crate::auth::{self, InstanceAdmin};
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;

/// ClickHouse tables holding per-org telemetry, with their org column; deleting an org purges them.
const TELEMETRY_TABLES: &[(&str, &str)] = &[
    ("oc.logs", "org_id"),
    ("otel.otel_traces", "OrgId"),
    ("otel.otel_ci_traces", "OrgId"),
    ("otel.otel_traces_trace_id_ts", "OrgId"),
    ("otel.otel_ci_traces_trace_id_ts", "OrgId"),
    ("otel.otel_metrics_gauge", "OrgId"),
    ("otel.otel_metrics_sum", "OrgId"),
    ("otel.otel_metrics_histogram", "OrgId"),
    ("otel.otel_metrics_exponential_histogram", "OrgId"),
    ("otel.otel_metrics_summary", "OrgId"),
    ("otel.metrics_series", "OrgId"),
    ("otel.metrics_samples", "OrgId"),
    ("otel.metrics_rollup_5m", "OrgId"),
    ("otel.metrics_rollup_1h", "OrgId"),
    ("otel.metrics_dist_samples", "OrgId"),
    ("otel.metrics_dist_rollup_5m", "OrgId"),
    ("otel.metrics_dist_rollup_1h", "OrgId"),
    ("otel.otel_k8s_objects", "OrgId"),
    ("otel.k8s_resources", "OrgId"),
    ("otel.k8s_inventory", "OrgId"),
    ("otel.k8s_events", "OrgId"),
    ("otel.otel_events", "OrgId"),
    ("otel.otel_network", "OrgId"),
    ("otel.profiles", "OrgId"),
    ("oc.logs_raw", "org_id"),
    ("oc.logs_rehydrated", "org_id"),
    ("oc.error_groups_1m", "org_id"),
    ("oc.log_facets_1m", "org_id"),
];

/// Organization ids are also URL path segments and role-id prefixes (`<org>-admin`).
fn valid_org_id(id: &str) -> bool {
    (2..=40).contains(&id.len())
        && id.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        && !id.starts_with('-')
        && !id.ends_with('-')
}

fn valid_email(email: &str) -> bool {
    email.len() <= 254 && email.split_once('@').is_some_and(|(l, d)| !l.is_empty() && d.contains('.') && !d.starts_with('.'))
}

#[derive(Serialize, ToSchema, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct InstanceOrg {
    pub id: String,
    pub name: String,
    /// active | suspended
    pub status: String,
    pub created_at: i64,
    pub members: i64,
    pub admins: Vec<String>,
    /// Logs received in the last 24 hours.
    #[sqlx(skip)]
    pub logs_24h: u64,
    /// Spans received in the last 24 hours.
    #[sqlx(skip)]
    pub spans_24h: u64,
    /// The install's own organization (home of the instance admins); can't be suspended or deleted.
    #[sqlx(skip)]
    pub primary: bool,
    /// What the org may configure (see PATCH …/limits).
    pub max_log_retention_days: i32,
    pub max_span_retention_days: i32,
    pub max_indexes: i32,
    /// Longest metric retention at full resolution and as rollups.
    #[sqlx(skip)]
    pub max_metrics: MetricRetention,
    #[sqlx(flatten)]
    #[serde(skip)]
    #[schema(ignore)]
    stored_metrics: StoredMetricRetention,
}

#[derive(clickhouse::Row, Deserialize)]
struct OrgVolume {
    org_id: String,
    n: u64,
}

async fn volumes(st: &AppState, sql: &str) -> HashMap<String, u64> {
    let rows = st.clickhouse.query(sql).fetch_all::<OrgVolume>().await;
    match rows {
        Ok(rows) => rows.into_iter().map(|r| (r.org_id, r.n)).collect(),
        Err(e) => {
            tracing::warn!(error = %e, "org ingest volume");
            HashMap::new()
        }
    }
}

#[utoipa::path(get, path = "/api/v1/instance/orgs", operation_id = "instance_list_orgs", responses((status = 200, body = Vec<InstanceOrg>)), tag = "instance")]
pub async fn list_orgs(State(st): State<AppState>, _: InstanceAdmin) -> ApiResult<Vec<InstanceOrg>> {
    let mut rows: Vec<InstanceOrg> = sqlx::query_as(
        "SELECT o.id, o.name, o.status, (extract(epoch FROM o.created_at) * 1000)::bigint AS created_at, \
         (SELECT count(*) FROM users u WHERE u.org_id = o.id AND u.status = 'active') AS members, \
         COALESCE((SELECT array_agg(a.email ORDER BY a.email) FROM users u JOIN accounts a ON a.id = u.account_id \
                   JOIN user_roles r ON r.user_id = u.id AND r.role_id = o.id || '-admin' WHERE u.org_id = o.id AND u.status <> 'deactivated'), '{}') AS admins, \
         COALESCE(d.max_log_retention_days, 30) AS max_log_retention_days, COALESCE(d.max_span_retention_days, 30) AS max_span_retention_days, COALESCE(d.max_indexes, 20) AS max_indexes, \
         d.metrics_retention_days, d.metrics_5m_days, d.metrics_1h_days, d.max_metrics_retention_days, d.max_metrics_5m_days, d.max_metrics_1h_days \
         FROM orgs o LEFT JOIN org_data_settings d ON d.org_id = o.id ORDER BY (o.id = $1) DESC, lower(o.name)",
    )
    .bind(&st.config.dev_org)
    .fetch_all(&st.postgres)
    .await?;
    let (logs, spans) = tokio::join!(
        volumes(&st, "SELECT org_id, count() AS n FROM oc.logs WHERE timestamp > now() - INTERVAL 1 DAY GROUP BY org_id"),
        volumes(&st, "SELECT OrgId AS org_id, count() AS n FROM otel.otel_traces WHERE Timestamp > now() - INTERVAL 1 DAY GROUP BY org_id"),
    );
    for r in &mut rows {
        r.logs_24h = logs.get(&r.id).copied().unwrap_or(0);
        r.spans_24h = spans.get(&r.id).copied().unwrap_or(0);
        r.primary = r.id == st.config.dev_org;
        r.max_metrics = r.stored_metrics.effective(install_metric_retention(&st.config.retention)).1;
    }
    Ok(Json(rows))
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct CreateOrgRequest {
    /// Lowercase letters, digits and dashes (2–40); used in URLs and SSO endpoints.
    pub id: String,
    pub name: String,
    /// Becomes the org's first Admin (invited; an existing account joins with its current password).
    pub admin_email: String,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct CreatedOrg {
    pub id: String,
    /// One-time link (72h) for the first admin. Share it until mail delivery is configured.
    pub invite_url: String,
}

/// Creates an organization with its built-in roles, login settings, dashboards and first admin.
pub async fn provision(st: &AppState, id: &str, name: &str, admin_email: &str, created_by: Option<&str>) -> Result<String, ApiError> {
    let mut tx = st.postgres.begin().await?;
    let created = sqlx::query("INSERT INTO orgs (id, name, created_by) VALUES ($1, $2, $3::uuid) ON CONFLICT (id) DO NOTHING")
        .bind(id)
        .bind(name)
        .bind(created_by)
        .execute(&mut *tx)
        .await?;
    if created.rows_affected() == 0 {
        return Err(ApiError::BadRequest(format!("An organization with id '{id}' already exists")));
    }
    for (suffix, role, desc) in [
        ("admin", "ObserveCraft Admin Role", "Full access, including organization settings, users and billing."),
        ("standard", "ObserveCraft Standard Role", "Create and edit dashboards, monitors and incidents; read all telemetry."),
        ("readonly", "ObserveCraft Read Only Role", "Read-only access to telemetry and assets."),
    ] {
        sqlx::query("INSERT INTO roles (id, org_id, name, description, managed) VALUES ($1, $2, $3, $4, true)")
            .bind(format!("{id}-{suffix}"))
            .bind(id)
            .bind(role)
            .bind(desc)
            .execute(&mut *tx)
            .await?;
    }
    sqlx::query("INSERT INTO auth_settings (org_id, default_role_id) VALUES ($1, $1 || '-standard')").bind(id).execute(&mut *tx).await?;
    let display = admin_email.split('@').next().unwrap_or(admin_email).replace(['.', '_'], " ");
    let (user_id,): (String,) = sqlx::query_as("INSERT INTO users (id, org_id, email, name, status, login_methods) \
         VALUES (gen_random_uuid(), $1, $2, COALESCE((SELECT name FROM accounts WHERE email = $2), $3), 'pending', '{}') RETURNING id::text")
        .bind(id)
        .bind(admin_email)
        .bind(&display)
        .fetch_one(&mut *tx)
        .await?;
    sqlx::query("INSERT INTO user_roles (user_id, role_id) VALUES ($1::uuid, $2 || '-admin')").bind(&user_id).bind(id).execute(&mut *tx).await?;
    tx.commit().await?;
    super::log_indexes::provision(st, id).await?;
    super::dashboards::install_seeds(&st.postgres, id, &user_id).await?;
    super::auth::issue_link(st, id, &user_id, "invite", None, 72).await
}

#[utoipa::path(post, path = "/api/v1/instance/orgs", operation_id = "instance_create_org", request_body = CreateOrgRequest, responses((status = 200, body = CreatedOrg), (status = 400, body = crate::error::ErrorBody)), tag = "instance")]
pub async fn create_org(State(st): State<AppState>, InstanceAdmin(ctx): InstanceAdmin, Json(req): Json<CreateOrgRequest>) -> ApiResult<CreatedOrg> {
    let id = req.id.trim().to_lowercase();
    if !valid_org_id(&id) {
        return Err(ApiError::BadRequest("Organization id must be 2–40 lowercase letters, digits or dashes".into()));
    }
    let name = req.name.trim();
    if name.is_empty() || name.chars().count() > 120 {
        return Err(ApiError::BadRequest("Name must be 1–120 characters".into()));
    }
    let email = req.admin_email.trim().to_lowercase();
    if !valid_email(&email) {
        return Err(ApiError::BadRequest(format!("'{}' is not a valid email address", req.admin_email)));
    }
    let invite_url = provision(&st, &id, name, &email, Some(&ctx.account_id)).await?;
    auth::instance_audit(&st.postgres, &ctx, "org.created", &id, &format!("first admin {email}")).await;
    auth::audit(&st.postgres, &id, None, &ctx.name, "org.created", &id, &format!("first admin {email}"), ctx.ip.as_deref()).await;
    Ok(Json(CreatedOrg { id, invite_url }))
}

#[derive(Deserialize, ToSchema)]
pub struct InstanceOrgPatch {
    pub name: Option<String>,
    /// active | suspended
    pub status: Option<String>,
}

/// Renames, suspends or reactivates an organization. Suspension ends its sessions and rejects its keys.
#[utoipa::path(patch, path = "/api/v1/instance/orgs/{id}", operation_id = "instance_update_org", params(("id" = String, Path)), request_body = InstanceOrgPatch, responses((status = 204), (status = 404, body = crate::error::ErrorBody)), tag = "instance")]
pub async fn update_org(State(st): State<AppState>, InstanceAdmin(ctx): InstanceAdmin, Path(id): Path<String>, Json(p): Json<InstanceOrgPatch>) -> Result<StatusCode, ApiError> {
    if let Some(s) = &p.status {
        if s != "active" && s != "suspended" {
            return Err(ApiError::BadRequest("status must be active or suspended".into()));
        }
        if s == "suspended" && id == st.config.dev_org {
            return Err(ApiError::BadRequest("The install's primary organization can't be suspended".into()));
        }
    }
    let name = p.name.as_deref().map(str::trim).filter(|n| !n.is_empty());
    if name.is_some_and(|n| n.chars().count() > 120) {
        return Err(ApiError::BadRequest("Name must be 1–120 characters".into()));
    }
    let n = sqlx::query("UPDATE orgs SET name = COALESCE($2, name), status = COALESCE($3, status) WHERE id = $1")
        .bind(&id)
        .bind(name)
        .bind(&p.status)
        .execute(&st.postgres)
        .await?
        .rows_affected();
    if n == 0 {
        return Err(ApiError::NotFound(format!("Organization {id} not found")));
    }
    if p.status.as_deref() == Some("suspended") {
        sqlx::query("DELETE FROM sessions WHERE org_id = $1").bind(&id).execute(&st.postgres).await?;
    }
    let what = match (&p.status, name) {
        (Some(s), _) => format!("org.{}", if s == "active" { "reactivated" } else { "suspended" }),
        _ => "org.renamed".to_string(),
    };
    auth::instance_audit(&st.postgres, &ctx, &what, &id, name.unwrap_or("")).await;
    auth::audit(&st.postgres, &id, None, &ctx.name, &what, &id, name.unwrap_or(""), ctx.ip.as_deref()).await;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct OrgLimits {
    pub max_log_retention_days: i32,
    pub max_span_retention_days: i32,
    pub max_indexes: i32,
    /// Longest metric retentions (tiers, full resolution ≤ 5-minute ≤ 1-hour); unchanged when absent.
    pub max_metrics: Option<MetricRetention>,
}

/// Sets what an org may configure. Lowering a ceiling also lowers settings above it.
#[utoipa::path(patch, path = "/api/v1/instance/orgs/{id}/limits", operation_id = "instance_update_org_limits", params(("id" = String, Path)), request_body = OrgLimits, responses((status = 204)), tag = "instance")]
pub async fn update_limits(State(st): State<AppState>, InstanceAdmin(ctx): InstanceAdmin, Path(id): Path<String>, Json(l): Json<OrgLimits>) -> Result<StatusCode, ApiError> {
    let r = &st.config.retention;
    if !(1..=r.logs_max_days as i32).contains(&l.max_log_retention_days) || !(1..=r.traces_max_days as i32).contains(&l.max_span_retention_days) || !(1..=1000).contains(&l.max_indexes) {
        return Err(ApiError::BadRequest(format!(
            "Log retention can be up to {} days and span retention up to {} days on this install (OC_RETENTION_*_MAX_DAYS)",
            r.logs_max_days, r.traces_max_days
        )));
    }
    if let Some(m) = l.max_metrics {
        validate_metric_limits(m).map_err(ApiError::BadRequest)?;
    }
    super::log_indexes::data_settings(&st, &id).await?;
    let mut tx = st.postgres.begin().await?;
    if let Some(m) = l.max_metrics {
        // A retention above its new limit comes down to it; unset ones keep following the install's
        // default unless that is above the limit.
        let d = install_metric_retention(&st.config.retention);
        sqlx::query(
            "UPDATE org_data_settings SET max_metrics_retention_days = $2, max_metrics_5m_days = $3, max_metrics_1h_days = $4, \
             metrics_retention_days = CASE WHEN COALESCE(metrics_retention_days, $5) > $2 THEN $2 ELSE metrics_retention_days END, \
             metrics_5m_days = CASE WHEN COALESCE(metrics_5m_days, $6) > $3 THEN $3 ELSE metrics_5m_days END, \
             metrics_1h_days = CASE WHEN COALESCE(metrics_1h_days, $7) > $4 THEN $4 ELSE metrics_1h_days END WHERE org_id = $1",
        )
        .bind(&id)
        .bind(m.raw)
        .bind(m.rollup_5m)
        .bind(m.rollup_1h)
        .bind(d.raw)
        .bind(d.rollup_5m)
        .bind(d.rollup_1h)
        .execute(&mut *tx)
        .await?;
    }
    sqlx::query(
        "UPDATE org_data_settings SET max_log_retention_days = $2, max_span_retention_days = $3, max_indexes = $4, \
         span_retention_days = least(span_retention_days, $3), updated_at = now() WHERE org_id = $1",
    )
    .bind(&id)
    .bind(l.max_log_retention_days)
    .bind(l.max_span_retention_days)
    .bind(l.max_indexes)
    .execute(&mut *tx)
    .await?;
    sqlx::query("UPDATE log_indexes SET retention_days = least(retention_days, $2) WHERE org_id = $1").bind(&id).bind(l.max_log_retention_days).execute(&mut *tx).await?;
    tx.commit().await?;
    let mut detail = format!("logs ≤ {} d, spans ≤ {} d, {} indexes", l.max_log_retention_days, l.max_span_retention_days, l.max_indexes);
    if let Some(m) = l.max_metrics {
        detail.push_str(&format!(", metrics ≤ {} / {} / {} d", m.raw, m.rollup_5m, m.rollup_1h));
    }
    auth::instance_audit(&st.postgres, &ctx, "org.limits_changed", &id, &detail).await;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize, ToSchema)]
pub struct DeleteOrgRequest {
    /// Must repeat the organization id.
    pub confirm: String,
}

/// Permanently deletes an organization: its users, content and settings now, its telemetry in the
/// background. Accounts that belong to other organizations keep those memberships.
#[utoipa::path(delete, path = "/api/v1/instance/orgs/{id}", operation_id = "instance_delete_org", params(("id" = String, Path)), request_body = DeleteOrgRequest, responses((status = 204), (status = 400, body = crate::error::ErrorBody)), tag = "instance")]
pub async fn delete_org(State(st): State<AppState>, InstanceAdmin(ctx): InstanceAdmin, Path(id): Path<String>, Json(req): Json<DeleteOrgRequest>) -> Result<StatusCode, ApiError> {
    if id == st.config.dev_org {
        return Err(ApiError::BadRequest("The install's primary organization can't be deleted".into()));
    }
    if req.confirm != id {
        return Err(ApiError::BadRequest("Type the organization id to confirm".into()));
    }
    let n = sqlx::query("DELETE FROM orgs WHERE id = $1").bind(&id).execute(&st.postgres).await?.rows_affected();
    if n == 0 {
        return Err(ApiError::NotFound(format!("Organization {id} not found")));
    }
    // Accounts left without any membership can't sign in anywhere; remove them.
    sqlx::query("DELETE FROM accounts a WHERE NOT a.is_instance_admin AND NOT EXISTS (SELECT 1 FROM users u WHERE u.account_id = a.id)").execute(&st.postgres).await?;
    for (table, column) in TELEMETRY_TABLES {
        // A mutation on the tables holding the rows (every shard): ClickHouse rewrites the affected
        // parts in the background. Rare enough that partitioning by org isn't worth its part count.
        let (local, on) = (st.tables.local(table), st.tables.on_cluster());
        if let Err(e) = st
            .clickhouse
            .query(&format!(
                "ALTER TABLE {local}{on} DELETE WHERE {column} = ?"
            ))
            .bind(&id)
            .execute()
            .await
        {
            tracing::error!(error = %e, table, org = %id, "purging telemetry of a deleted org");
        }
    }
    auth::instance_audit(&st.postgres, &ctx, "org.deleted", &id, "").await;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Serialize, ToSchema, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct InstanceAuditEvent {
    pub id: i64,
    pub actor: String,
    pub action: String,
    pub target: String,
    pub detail: String,
    pub ip: Option<String>,
    pub created_at: i64,
}

#[utoipa::path(get, path = "/api/v1/instance/audit", operation_id = "instance_audit", responses((status = 200, body = Vec<InstanceAuditEvent>)), tag = "instance")]
pub async fn audit(State(st): State<AppState>, _: InstanceAdmin) -> ApiResult<Vec<InstanceAuditEvent>> {
    let rows = sqlx::query_as(
        "SELECT id, actor, action, target, detail, ip, (extract(epoch FROM created_at) * 1000)::bigint AS created_at \
         FROM instance_audit_events ORDER BY created_at DESC LIMIT 200",
    )
    .fetch_all(&st.postgres)
    .await?;
    Ok(Json(rows))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn org_ids() {
        assert!(valid_org_id("acme"));
        assert!(valid_org_id("acme-eu-2"));
        assert!(!valid_org_id("a"));
        assert!(!valid_org_id("Acme"));
        assert!(!valid_org_id("-acme"));
        assert!(!valid_org_id("acme_eu"));
        assert!(!valid_org_id("acme/../x"));
    }
}
