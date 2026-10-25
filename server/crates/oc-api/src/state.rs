use std::sync::Arc;

use axum::async_trait;
use axum::extract::FromRequestParts;
use axum::http::request::Parts;

use crate::auth::crypto::SecretBox;
use crate::auth::AuthContext;
use crate::config::Config;
use crate::error::ApiError;

#[derive(Clone)]
pub struct AppState(pub Arc<Inner>);

pub struct Inner {
    pub config: Config,
    pub clickhouse: clickhouse::Client,
    pub postgres: sqlx::PgPool,
    pub nats: async_nats::Client,
    pub secrets: SecretBox,
}

impl Inner {
    /// ClickHouse client for one organization's telemetry. Queries on the `otel.*` tables filter on
    /// `OrgId = {oc_org:String}`, a server-side query parameter this client always sends.
    pub fn ch(&self, org: &str) -> clickhouse::Client {
        self.clickhouse.clone().with_option("param_oc_org", org)
    }
}

impl std::ops::Deref for AppState {
    type Target = Inner;
    fn deref(&self) -> &Inner {
        &self.0
    }
}

/// The tenant a request acts for: the authenticated user's org. Every query is scoped by it.
pub struct OrgId(pub String);

#[async_trait]
impl FromRequestParts<AppState> for OrgId {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, ApiError> {
        Ok(OrgId(AuthContext::from_request_parts(parts, state).await?.org_id))
    }
}

/// The authenticated user's id (UUID as text).
pub struct CurrentUser(pub String);

#[async_trait]
impl FromRequestParts<AppState> for CurrentUser {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, ApiError> {
        Ok(CurrentUser(AuthContext::from_request_parts(parts, state).await?.user_id))
    }
}
