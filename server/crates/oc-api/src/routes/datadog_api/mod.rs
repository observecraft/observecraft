//! Datadog-compatible management API (ADR-0003): Datadog's paths, documents, errors and key headers,
//! so Datadog's Terraform provider, SDKs and sync tools work against ObserveCraft with only the URL
//! and keys changed.
//!
//! Authentication, as Datadog's clients send it:
//! - `DD-APPLICATION-KEY: ocak_…` (with or without `DD-API-KEY`) or `Authorization: Bearer ocak_…` act
//!   as that application key's user, with its roles ([`DdAuth`]). A `DD-API-KEY` sent along must be a
//!   valid ingest key (`ocik_…`) of the same organization.
//! - `DD-API-KEY` alone (an ingest key) only passes `GET /api/v1/validate`.
//!
//! Datadog's clients send keys only in headers (the query-string schemes in their spec are used by no
//! operation), so `?api_key=` / `?application_key=` are not read here.
//!
//! These routes are merged next to the API outside its session middleware ([`crate::routes::app`]), so
//! the Datadog headers are read only here and never authenticate one of our own routes.
//!
//! Errors are Datadog's `{"errors": ["…"]}` with Datadog's status codes ([`DdError`]).

pub mod current_user;
pub mod slo_corrections;
pub mod slos;
pub mod dashboard_lists;
pub mod dashboard_widgets;
pub mod dashboards;
pub mod downtimes;
pub mod monitors;
pub mod keys;
pub mod permissions;
pub mod roles;
pub mod teams;
pub mod users;
pub mod validate;

#[cfg(test)]
pub mod replay;

use axum::async_trait;
use axum::extract::{ConnectInfo, FromRequestParts};
use axum::http::header::AUTHORIZATION;
use axum::http::request::Parts;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Serialize;
use utoipa::ToSchema;

use crate::auth::{crypto, AuthContext};
use crate::error::ApiError;
use crate::state::AppState;

pub const API_KEY_HEADER: &str = "dd-api-key";
pub const APP_KEY_HEADER: &str = "dd-application-key";
const APP_KEY_PREFIX: &str = "ocak_";
const INGEST_KEY_PREFIX: &str = "ocik_";

/// The Datadog-compatible routes. They authenticate themselves ([`DdAuth`]), so they are merged
/// outside the session middleware.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/v1/validate", get(validate::validate))
        .route("/api/v2/validate_keys", get(validate::validate_keys))
        .route("/api/v2/current_user", get(current_user::current_user))
        .merge(slos::router())
        .merge(slo_corrections::router())
        .merge(dashboards::router())
        .merge(dashboard_lists::router())
        .merge(monitors_and_downtimes())
        .merge(iam_router())
}

/// `datadog_monitor`, `datadog_downtime_schedule` and `datadog_downtime` ([`monitors`], [`downtimes`]).
fn monitors_and_downtimes() -> Router<AppState> {
    Router::new()
        .route("/api/v1/monitor", get(monitors::list).post(monitors::create))
        .route("/api/v1/monitor/validate", post(monitors::validate))
        .route("/api/v1/monitor/can_delete", get(monitors::can_delete))
        .route("/api/v1/monitor/search", get(monitors::search))
        .route("/api/v1/monitor/:monitor_id", get(monitors::get_one).put(monitors::update).delete(monitors::delete))
        .route("/api/v1/monitor/:monitor_id/validate", post(monitors::validate_existing))
        .route("/api/v1/downtime", get(downtimes::v1_list).post(downtimes::v1_create))
        .route("/api/v1/downtime/cancel/by_scope", post(downtimes::v1_cancel_by_scope))
        .route("/api/v1/downtime/:downtime_id", get(downtimes::v1_get).put(downtimes::v1_update).delete(downtimes::v1_cancel))
        .route("/api/v2/downtime", get(downtimes::v2_list).post(downtimes::v2_create))
        .route("/api/v2/downtime/:downtime_id", get(downtimes::v2_get).patch(downtimes::v2_update).delete(downtimes::v2_cancel))
        .route("/api/v2/monitor/:monitor_id/downtime_matches", get(downtimes::v2_monitor_matches))
}

/// Users, roles, permissions, teams and keys ([`users`], [`roles`], [`teams`], [`keys`]).
fn iam_router() -> Router<AppState> {
    use axum::routing::post;
    Router::new()
        .route("/api/v2/users", get(users::list).post(users::create))
        .route("/api/v2/users/:id", get(users::get).patch(users::update).delete(users::disable))
        .route("/api/v2/users/:id/permissions", get(users::permissions))
        .route("/api/v2/users/:id/memberships", get(teams::user_memberships))
        .route("/api/v2/user_invitations", post(users::send_invitations))
        .route("/api/v2/user_invitations/:id", get(users::get_invitation))
        .route("/api/v2/permissions", get(roles::list_permissions))
        .route("/api/v2/roles", get(roles::list).post(roles::create))
        .route("/api/v2/roles/:id", get(roles::get).patch(roles::update).delete(roles::delete))
        .route("/api/v2/roles/:id/clone", post(roles::clone))
        .route("/api/v2/roles/:id/permissions", get(roles::list_role_permissions).post(roles::add_permission).delete(roles::remove_permission))
        .route("/api/v2/roles/:id/users", get(roles::list_users).post(roles::add_user).delete(roles::remove_user))
        .route("/api/v2/team", get(teams::list).post(teams::create))
        .route("/api/v2/team/:id", get(teams::get).patch(teams::update).delete(teams::delete))
        .route("/api/v2/team/:id/links", get(teams::links).post(teams::create_link))
        .route("/api/v2/team/:id/links/:link", get(teams::link).patch(teams::update_link).delete(teams::link))
        .route("/api/v2/team/:id/memberships", get(teams::memberships).post(teams::add_member))
        .route("/api/v2/team/:id/memberships/:user", axum::routing::patch(teams::update_member).delete(teams::remove_member))
        .route("/api/v2/api_keys", get(keys::list_api_keys).post(keys::create_api_key))
        .route("/api/v2/api_keys/:id", get(keys::get_api_key).patch(keys::update_api_key).delete(keys::delete_api_key))
        .route("/api/v2/current_user/application_keys", get(keys::list_my_app_keys).post(keys::create_my_app_key))
        .route("/api/v2/current_user/application_keys/:id", get(keys::get_my_app_key).patch(keys::update_my_app_key).delete(keys::delete_my_app_key))
        .route("/api/v2/application_keys", get(keys::list_org_app_keys))
        .route("/api/v2/application_keys/:id", get(keys::get_org_app_key).patch(keys::update_org_app_key).delete(keys::delete_org_app_key))
}

// ================================================================ errors

/// Datadog's error body.
#[derive(Serialize, ToSchema)]
pub struct DatadogErrors {
    pub errors: Vec<String>,
}

/// An error answered in Datadog's shape: `{"errors": ["…"]}` with Datadog's status code.
#[derive(Debug)]
pub struct DdError {
    pub status: StatusCode,
    pub errors: Vec<String>,
}

impl DdError {
    pub fn new(status: StatusCode, message: impl Into<String>) -> Self {
        DdError { status, errors: vec![message.into()] }
    }
    pub fn forbidden() -> Self {
        Self::new(StatusCode::FORBIDDEN, "Forbidden")
    }
}

impl IntoResponse for DdError {
    fn into_response(self) -> Response {
        let mut response = (self.status, Json(DatadogErrors { errors: self.errors })).into_response();
        if self.status == StatusCode::TOO_MANY_REQUESTS {
            response.headers_mut().insert(axum::http::header::RETRY_AFTER, axum::http::HeaderValue::from_static("1"));
        }
        response
    }
}

impl From<ApiError> for DdError {
    fn from(e: ApiError) -> Self {
        match e {
            ApiError::BadRequest(m) => Self::new(StatusCode::BAD_REQUEST, m),
            ApiError::NotFound(m) => Self::new(StatusCode::NOT_FOUND, m),
            ApiError::Unauthorized(m) => Self::new(StatusCode::UNAUTHORIZED, m),
            ApiError::Forbidden(m) => Self::new(StatusCode::FORBIDDEN, m),
            ApiError::TooManyRequests(m) => Self::new(StatusCode::TOO_MANY_REQUESTS, m),
            ApiError::Conflict(m) => Self::new(StatusCode::CONFLICT, m),
            other => {
                tracing::error!(error = %other, "datadog-compatible API request failed");
                Self::new(StatusCode::INTERNAL_SERVER_ERROR, "Internal Server Error")
            }
        }
    }
}

impl From<sqlx::Error> for DdError {
    fn from(e: sqlx::Error) -> Self {
        ApiError::from(e).into()
    }
}

pub type DdResult<T> = Result<Json<T>, DdError>;

// ================================================================ credentials

/// The keys a request carries, as Datadog's clients send them. Header names are case-insensitive.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct DdKeys {
    /// `DD-API-KEY`.
    pub api_key: Option<String>,
    /// `DD-APPLICATION-KEY`, else `Authorization: Bearer`.
    pub app_key: Option<String>,
}

pub fn keys(headers: &HeaderMap) -> DdKeys {
    let header = |name: &str| headers.get(name).and_then(|v| v.to_str().ok()).map(str::trim).filter(|v| !v.is_empty()).map(str::to_string);
    DdKeys { api_key: header(API_KEY_HEADER), app_key: header(APP_KEY_HEADER).or_else(|| bearer(headers)) }
}

/// The token of `Authorization: Bearer …`.
pub fn bearer(headers: &HeaderMap) -> Option<String> {
    headers
        .get(AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split_once(' '))
        .filter(|(scheme, _)| scheme.eq_ignore_ascii_case("bearer"))
        .map(|(_, token)| token.trim().to_string())
        .filter(|t| !t.is_empty())
}

/// The org of an active ingest key (`ocik_…`, active org), or None.
pub async fn ingest_key_org(st: &AppState, key: &str) -> Result<Option<String>, DdError> {
    if !key.starts_with(INGEST_KEY_PREFIX) {
        return Ok(None);
    }
    let hash = crypto::sha256(key);
    let org: Option<(String,)> = sqlx::query_as("SELECT k.org_id FROM ingest_keys k JOIN orgs o ON o.id = k.org_id WHERE k.key_hash = $1 AND k.revoked_at IS NULL AND o.status = 'active'")
        .bind(&hash)
        .fetch_optional(&st.postgres)
        .await?;
    if org.is_some() {
        sqlx::query("UPDATE ingest_keys SET last_used_at = now() WHERE key_hash = $1 AND (last_used_at IS NULL OR last_used_at < now() - interval '1 minute')")
            .bind(&hash)
            .execute(&st.postgres)
            .await?;
    }
    Ok(org.map(|(o,)| o))
}

/// The user of an active application key (`ocak_…`), or None.
pub async fn app_key_user(st: &AppState, key: &str, ip: Option<String>) -> Result<Option<AuthContext>, DdError> {
    if !key.starts_with(APP_KEY_PREFIX) {
        return Ok(None);
    }
    Ok(crate::auth::app_key_context(st, key, ip).await?)
}

/// Resolves the application key of a request, checking a `DD-API-KEY` sent along belongs to the same org.
pub async fn resolve(st: &AppState, keys: &DdKeys, ip: Option<String>) -> Result<AuthContext, DdError> {
    let Some(app_key) = &keys.app_key else { return Err(DdError::forbidden()) };
    let Some(ctx) = app_key_user(st, app_key, ip).await? else { return Err(DdError::forbidden()) };
    if let Some(api_key) = &keys.api_key {
        if ingest_key_org(st, api_key).await?.as_deref() != Some(ctx.org_id.as_str()) {
            return Err(DdError::forbidden());
        }
    }
    Ok(ctx)
}

/// The application key's user and roles, for the Datadog-compatible routes. Rejects with 403
/// `{"errors": ["Forbidden"]}` as Datadog does for missing or invalid keys.
pub struct DdAuth(pub AuthContext);

#[async_trait]
impl FromRequestParts<AppState> for DdAuth {
    type Rejection = DdError;
    async fn from_request_parts(parts: &mut Parts, st: &AppState) -> Result<Self, DdError> {
        let peer = parts.extensions.get::<ConnectInfo<std::net::SocketAddr>>().map(|c| c.0);
        let ip = crate::auth::client_ip(st, &parts.headers, peer);
        Ok(DdAuth(resolve(st, &keys(&parts.headers), ip).await?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    fn headers(pairs: &[(&'static str, &'static str)]) -> HeaderMap {
        let mut h = HeaderMap::new();
        for (k, v) in pairs {
            h.insert(axum::http::HeaderName::from_bytes(k.as_bytes()).unwrap(), HeaderValue::from_static(v));
        }
        h
    }

    #[test]
    fn keys_from_datadog_headers_in_any_case() {
        let k = keys(&headers(&[("DD-API-KEY", "ocik_a"), ("Dd-Application-Key", "ocak_b")]));
        assert_eq!(k, DdKeys { api_key: Some("ocik_a".into()), app_key: Some("ocak_b".into()) });
        let k = keys(&headers(&[("dd-application-key", " ocak_c ")]));
        assert_eq!(k, DdKeys { api_key: None, app_key: Some("ocak_c".into()) });
    }

    #[test]
    fn bearer_token_is_the_application_key() {
        let k = keys(&headers(&[("authorization", "Bearer ocak_t")]));
        assert_eq!(k.app_key.as_deref(), Some("ocak_t"));
        let k = keys(&headers(&[("authorization", "bearer ocak_t")]));
        assert_eq!(k.app_key.as_deref(), Some("ocak_t"));
        // DD-APPLICATION-KEY wins over Authorization (the provider's bearer validation sends both slots).
        let k = keys(&headers(&[("authorization", "Bearer ocak_x"), ("dd-application-key", "ocak_y")]));
        assert_eq!(k.app_key.as_deref(), Some("ocak_y"));
        let k = keys(&headers(&[("authorization", "Basic abc")]));
        assert_eq!(k.app_key, None);
        assert_eq!(keys(&headers(&[("dd-api-key", "")])), DdKeys::default());
    }

    #[tokio::test]
    async fn errors_have_datadogs_shape() {
        let r = DdError::forbidden().into_response();
        assert_eq!(r.status(), StatusCode::FORBIDDEN);
        let body = axum::body::to_bytes(r.into_body(), 1 << 16).await.unwrap();
        assert_eq!(serde_json::from_slice::<serde_json::Value>(&body).unwrap(), serde_json::json!({ "errors": ["Forbidden"] }));

        let r = DdError::from(ApiError::BadRequest("Invalid monitor type".into())).into_response();
        assert_eq!(r.status(), StatusCode::BAD_REQUEST);
        let r = DdError::from(ApiError::TooManyRequests("slow down".into())).into_response();
        assert_eq!(r.status(), StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(r.headers()["retry-after"], "1");
        let r = DdError::from(ApiError::Internal(anyhow::anyhow!("secret detail"))).into_response();
        assert_eq!(r.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let body = axum::body::to_bytes(r.into_body(), 1 << 16).await.unwrap();
        assert!(!String::from_utf8_lossy(&body).contains("secret detail"));
    }
}
