//! Integrations → Install, as Datadog's integration tiles: installing an integration records it for the org
//! (Postgres `integration_installs`) and installs what it brings, its log pipelines from the library
//! (`pipelines::library`, by their catalog `integration`) and its integrations-core dashboards
//! (`integration_dashboards`). Integrations that send only logs (Supply Chain Firewall) have no metric prefix for
//! `dashboards::enabled_integrations` to detect, so this is how their dashboards appear. Uninstalling removes the
//! record and the integration's pipelines; its dashboards stay (favorites and scheduled reports depend on them), as
//! dashboards the org deletes itself.

use axum::extract::{Path, State};
use axum::Json;
use serde::Serialize;
use utoipa::ToSchema;

use super::log_pipelines::{insert, LogPipelineInput};
use crate::auth::Writer;
use crate::error::{ApiError, ApiResult};
use crate::pipelines::library;
use crate::state::{AppState, OrgId};

/// An integration's installation state and what it brings.
#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct IntegrationInstall {
    /// Catalog id (`supply-chain-firewall`).
    pub integration: String,
    pub installed: bool,
    /// Names of its library log pipelines.
    pub pipelines: Vec<String>,
    /// How many integration dashboards it brings.
    pub dashboards: usize,
}

/// A catalog id: lowercase letters, digits, `-`, `_` and `.`.
pub fn valid_id(id: &str) -> bool {
    (1..=100).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"-_.".contains(&b)) && id.as_bytes()[0].is_ascii_alphanumeric()
}

/// The library pipelines an integration installs.
pub fn pipelines_of(id: &str) -> Vec<&'static library::IntegrationPipeline> {
    library::all().iter().filter(|p| p.integration == id).collect()
}

fn state(id: &str, installed: bool) -> IntegrationInstall {
    IntegrationInstall {
        integration: id.to_string(),
        installed,
        pipelines: pipelines_of(id).into_iter().map(|p| p.name.clone()).collect(),
        dashboards: super::integration_dashboards::dashboards_of(id).len(),
    }
}

fn checked(id: String) -> Result<String, ApiError> {
    if valid_id(&id) {
        Ok(id)
    } else {
        Err(ApiError::BadRequest("Not an integration id".into()))
    }
}

/// The org's installed integrations (catalog ids).
pub async fn installed(st: &AppState, org: &str) -> Result<Vec<String>, ApiError> {
    Ok(sqlx::query_scalar("SELECT integration FROM integration_installs WHERE org_id = $1 ORDER BY integration").bind(org).fetch_all(&st.postgres).await?)
}

/// The integrations installed from Integrations (catalog ids).
#[utoipa::path(get, path = "/api/v1/integrations/installed", operation_id = "list_installed_integrations", responses((status = 200, body = Vec<String>)), tag = "integrations")]
pub async fn list(State(st): State<AppState>, OrgId(org): OrgId) -> ApiResult<Vec<String>> {
    Ok(Json(installed(&st, &org).await?))
}

/// Installs an integration: its log pipelines (those the org has are kept) and its dashboards.
#[utoipa::path(put, path = "/api/v1/integrations/{id}/install", operation_id = "install_integration", params(("id" = String, Path)),
    responses((status = 200, body = IntegrationInstall), (status = 400, body = crate::error::ErrorBody)), tag = "integrations")]
pub async fn install(State(st): State<AppState>, Writer(ctx): Writer, Path(id): Path<String>) -> ApiResult<IntegrationInstall> {
    let id = checked(id)?;
    sqlx::query("INSERT INTO integration_installs (org_id, integration, installed_by) VALUES ($1, $2, $3::uuid) ON CONFLICT DO NOTHING")
        .bind(&ctx.org_id)
        .bind(&id)
        .bind(&ctx.user_id)
        .execute(&st.postgres)
        .await?;
    for p in pipelines_of(&id) {
        let input = LogPipelineInput { name: p.name.clone(), filter: p.filter.clone(), enabled: true, processors: p.processors.clone() };
        // None: the org installed it before (Logs → Pipelines).
        insert(&st, &ctx.org_id, &input, &format!("integration:{}", p.id)).await?;
    }
    super::integration_dashboards::install_missing(&st.postgres, &ctx.org_id, &[id.as_str()]).await.map_err(ApiError::Internal)?;
    ctx.audit(&st, "integration.installed", &id, "").await;
    Ok(Json(state(&id, true)))
}

/// Uninstalls an integration: its log pipelines are removed; its dashboards stay.
#[utoipa::path(delete, path = "/api/v1/integrations/{id}/install", operation_id = "uninstall_integration", params(("id" = String, Path)),
    responses((status = 200, body = IntegrationInstall), (status = 400, body = crate::error::ErrorBody)), tag = "integrations")]
pub async fn uninstall(State(st): State<AppState>, Writer(ctx): Writer, Path(id): Path<String>) -> ApiResult<IntegrationInstall> {
    let id = checked(id)?;
    let sources: Vec<String> = pipelines_of(&id).into_iter().map(|p| format!("integration:{}", p.id)).collect();
    let mut tx = st.postgres.begin().await?;
    sqlx::query("DELETE FROM integration_installs WHERE org_id = $1 AND integration = $2").bind(&ctx.org_id).bind(&id).execute(&mut *tx).await?;
    sqlx::query("DELETE FROM log_pipelines WHERE org_id = $1 AND source = ANY($2)").bind(&ctx.org_id).bind(&sources).execute(&mut *tx).await?;
    tx.commit().await?;
    ctx.audit(&st, "integration.uninstalled", &id, "").await;
    Ok(Json(state(&id, false)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn integration_ids_are_catalog_ids() {
        assert!(valid_id("supply-chain-firewall"));
        assert!(valid_id("sql-server"));
        assert!(!valid_id(""));
        assert!(!valid_id("-x"));
        assert!(!valid_id("Redis"));
        assert!(!valid_id("../x"));
        assert!(!valid_id(&"a".repeat(101)));
    }

    #[test]
    fn supply_chain_firewall_installs_its_pipeline_and_dashboards() {
        let s = state("supply-chain-firewall", true);
        assert_eq!(s.pipelines, ["Supply Chain Firewall"]);
        assert!(s.dashboards > 0, "its integrations-core dashboard");
        assert!(pipelines_of("redis").iter().any(|p| p.id == "redisdb"), "catalog ids, not check names");
        assert!(pipelines_of("no-such-integration").is_empty());
    }
}
