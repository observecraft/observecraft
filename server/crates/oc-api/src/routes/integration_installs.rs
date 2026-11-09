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

/// Integrations set up on their own page rather than from their tile: the table holding their connection or
/// accounts, by catalog id. Having one counts as installed.
const CONNECTIONS: [(&str, &str); 6] = [
    ("authentik", "authentik_connections"),
    ("aws", "aws_accounts"),
    ("azure", "azure_accounts"),
    ("fleetdm", "fleetdm_connections"),
    ("google-cloud", "gcp_accounts"),
    ("netbird", "netbird_connections"),
];

/// The org's installed integrations (catalog ids): installed from their tile, or with a saved connection.
pub async fn installed(st: &AppState, org: &str) -> Result<Vec<String>, ApiError> {
    let connected: Vec<String> = CONNECTIONS.iter().map(|(id, table)| format!("SELECT '{id}' WHERE EXISTS (SELECT 1 FROM {table} WHERE org_id = $1)")).collect();
    let sql = format!("SELECT integration FROM integration_installs WHERE org_id = $1 UNION {} ORDER BY 1", connected.join(" UNION "));
    Ok(sqlx::query_scalar(sqlx::AssertSqlSafe(sql)).bind(org).fetch_all(&st.postgres).await?)
}

/// The installed integrations (catalog ids): from Integrations → Install, or set up on their own page.
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

    /// On the migrated schema (0001 made an older `integration_installs` that 0092 kept), installing and listing work.
    #[tokio::test]
    async fn installed_integrations_are_listed_on_the_migrated_schema() {
        let Some((_, states)) = crate::test_support::replicas(1).await else { return };
        let st = &states[0];
        let mut ctx = crate::test_support::admin_of_new_org(st, "integration-installs").await;
        ctx.user_id = sqlx::query_scalar("INSERT INTO users (id, org_id, email, name) VALUES (gen_random_uuid(), $1, 'admin@example.com', 'Admin') RETURNING id::text")
            .bind(&ctx.org_id)
            .fetch_one(&st.postgres)
            .await
            .unwrap();
        install(State(st.clone()), Writer(ctx.clone()), Path("supply-chain-firewall".into())).await.unwrap();
        let Json(ids) = list(State(st.clone()), OrgId(ctx.org_id.clone())).await.unwrap();
        assert_eq!(ids, ["supply-chain-firewall"]);
        uninstall(State(st.clone()), Writer(ctx.clone()), Path("supply-chain-firewall".into())).await.unwrap();
        assert!(list(State(st.clone()), OrgId(ctx.org_id)).await.unwrap().0.is_empty());
    }

    /// Integrations set up on their own page (a saved connection or cloud account) count as installed.
    #[tokio::test]
    async fn configured_connections_count_as_installed() {
        let Some((_, states)) = crate::test_support::replicas(1).await else { return };
        let st = &states[0];
        let org = crate::test_support::new_org(st, "integration-connections").await;
        for table in ["authentik_connections", "fleetdm_connections"] {
            sqlx::query(sqlx::AssertSqlSafe(format!("INSERT INTO {table} (org_id, base_url, token_enc) VALUES ($1, 'https://tool.example.com', 'sealed')")))
                .bind(&org)
                .execute(&st.postgres)
                .await
                .unwrap();
        }
        sqlx::query("INSERT INTO aws_accounts (org_id, aws_account_id) VALUES ($1, '123456789012')").bind(&org).execute(&st.postgres).await.unwrap();
        let Json(ids) = list(State(st.clone()), OrgId(org.clone())).await.unwrap();
        assert_eq!(ids, ["authentik", "aws", "fleetdm"]);
        let other = crate::test_support::new_org(st, "integration-connections-other").await;
        assert!(list(State(st.clone()), OrgId(other)).await.unwrap().0.is_empty(), "another org's connections don't count");
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
