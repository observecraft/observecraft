//! The native connectors: integrations with their own code, table and setup page, registered as [`Connector`]s for
//! what is generic about them (the catalog entry, connected or not, their connections and jobs).

use std::sync::Arc;

use super::{ConnectionRef, Connector, ConnectorEntry, ConnectorKind};
use crate::state::AppState;

/// A native connector: its entry, its jobs' provider and the table of its connections.
pub struct Native {
    entry: ConnectorEntry,
    provider: &'static str,
    /// `SELECT id, label FROM … WHERE org_id = $1`.
    list: &'static str,
}

#[async_trait::async_trait]
impl Connector for Native {
    fn entry(&self) -> &ConnectorEntry {
        &self.entry
    }

    fn provider(&self) -> &str {
        self.provider
    }

    async fn connections(&self, st: &AppState, org: &str) -> Result<Vec<ConnectionRef>, sqlx::Error> {
        let rows: Vec<(String, String)> = sqlx::query_as(sqlx::AssertSqlSafe(self.list)).bind(org).fetch_all(&st.postgres).await?;
        Ok(rows.into_iter().map(|(id, name)| ConnectionRef { id, name }).collect())
    }
}

#[allow(clippy::too_many_arguments)]
fn native(id: &str, name: &str, vendor: &str, description: &str, categories: &[&str], source: &str, page: &str, provider: &'static str, list: &'static str) -> Arc<dyn Connector> {
    Arc::new(Native {
        entry: ConnectorEntry {
            id: id.into(),
            name: name.into(),
            publisher: "ObserveCraft".into(),
            vendor: vendor.into(),
            version: env!("CARGO_PKG_VERSION").into(),
            description: description.into(),
            tiles: vec![id.into()],
            logo: id.into(),
            categories: categories.iter().map(|c| c.to_string()).collect(),
            source: source.into(),
            kind: ConnectorKind::Native,
            third_party: false,
            page: Some(page.into()),
            docs_url: None,
            setup: None,
            links: vec![],
            config: vec![],
            streams: vec![],
            push: None,
            dashboards: vec![],
            pipeline: None,
            monitors: vec![],
            testable: true,
        },
        provider,
        list,
    })
}

/// Every native connector.
pub fn all() -> Vec<Arc<dyn Connector>> {
    vec![
        native(
            "authentik",
            "authentik",
            "Authentik Security",
            "The instance's events (logins, failed logins, policy and model changes) as logs.",
            &["Security", "Log Collection"],
            "authentik",
            "/integrations/authentik",
            crate::authentik::crawler::PROVIDER,
            "SELECT id::text, base_url FROM authentik_connections WHERE org_id = $1",
        ),
        native(
            "fleetdm",
            "FleetDM",
            "Fleet Device Management",
            "A Fleet server's activities as logs; its hosts and policies as metrics.",
            &["Security", "Log Collection", "Metrics"],
            "fleetdm",
            "/integrations/fleetdm",
            crate::fleetdm::crawler::PROVIDER,
            "SELECT id::text, base_url FROM fleetdm_connections WHERE org_id = $1",
        ),
        native(
            "gitlab-audit-events",
            "GitLab Audit Events",
            "GitLab",
            "An instance's, group's or project's audit events as logs.",
            &["Security", "Log Collection", "Source Control"],
            "gitlab",
            "/integrations/gitlab-audit-events",
            crate::gitlab_audit::crawler::PROVIDER,
            "SELECT id::text, base_url FROM gitlab_audit_connections WHERE org_id = $1",
        ),
        native(
            "google-workspace",
            "Google Workspace",
            "Google",
            "The Reports API's activities (logins, Drive, admin, tokens, …) as logs.",
            &["Security", "Log Collection", "Collaboration"],
            "google-workspace",
            "/integrations/google-workspace",
            crate::google_workspace::crawler::PROVIDER,
            "SELECT id::text, client_email FROM google_workspace_connections WHERE org_id = $1",
        ),
        native(
            "azure-active-directory",
            "Microsoft Entra ID",
            "Microsoft",
            "A tenant's sign-in, audit and risk detection logs from Microsoft Graph, with an Azure app registration.",
            &["Security", "Log Collection", "Azure"],
            "azure.activedirectory",
            "/integrations/azure?detail=entra",
            crate::entra::crawler::PROVIDER,
            "SELECT account_id::text, account_id::text FROM entra_log_settings WHERE org_id = $1",
        ),
    ]
}

/// Whether the org has a connection of the native connector `id`.
pub async fn connected(st: &AppState, org: &str, id: &str) -> Result<bool, sqlx::Error> {
    let table = match id {
        "authentik" => "authentik_connections",
        "fleetdm" => "fleetdm_connections",
        "gitlab-audit-events" => "gitlab_audit_connections",
        "google-workspace" => "google_workspace_connections",
        "azure-active-directory" => "entra_log_settings",
        _ => return Ok(false),
    };
    sqlx::query_scalar(sqlx::AssertSqlSafe(format!("SELECT EXISTS (SELECT 1 FROM {table} WHERE org_id = $1)"))).bind(org).fetch_one(&st.postgres).await
}
