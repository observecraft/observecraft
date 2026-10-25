//! Authentication and authorization.
//!
//! Every `/api/v1/*` request (except the public `/api/v1/auth/*` endpoints) must carry either
//! - the `oc_session` cookie set at login (browser), or
//! - `Authorization: Bearer ocak_…` — an application key (scripts, Terraform).
//!
//! The [`require`] middleware resolves the credential to an [`AuthContext`]; handlers take
//! [`OrgId`]/[`CurrentUser`] (any authenticated user), [`Writer`] (Standard or Admin), [`Admin`] or
//! [`InstanceAdmin`]. Access level comes from the user's built-in role (`<org>-admin` / `-standard` / `-readonly`).
//!
//! An *account* (email, password, MFA) signs in; a session acts as one of the account's memberships
//! (`users` rows, one per org), which switching orgs changes. Suspended orgs reject every credential.
//!
//! Cookie-authenticated writes must come from an allowed Origin (CSRF defence on top of SameSite=Lax).

pub mod crypto;
pub mod password;
pub mod totp;

use std::net::SocketAddr;

use axum::async_trait;
use axum::extract::{ConnectInfo, FromRequestParts, Request, State};
use axum::http::header::{AUTHORIZATION, COOKIE, ORIGIN, REFERER};
use axum::http::request::Parts;
use axum::http::{HeaderMap, HeaderValue, Method};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

use crate::error::ApiError;
use crate::state::AppState;

pub const SESSION_COOKIE: &str = "oc_session";
/// Sessions slide with activity but never outlive this.
const ABSOLUTE_SESSION_DAYS: i64 = 7;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum Access {
    ReadOnly,
    Standard,
    Admin,
}

/// How the request authenticated.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Credential {
    Session,
    AppKey,
}

#[derive(Clone, Debug)]
pub struct AuthContext {
    /// The membership (`users.id`) acting in `org_id`.
    pub user_id: String,
    pub org_id: String,
    /// The signed-in identity behind the membership.
    pub account_id: String,
    /// May create and manage organizations.
    pub instance_admin: bool,
    pub name: String,
    pub access: Access,
    pub credential: Credential,
    /// SHA-256 of the session token (to identify "this session" in session lists / logout).
    pub session_hash: Option<Vec<u8>>,
    pub ip: Option<String>,
}

pub fn access_from_roles(org: &str, role_ids: &[String]) -> Access {
    if role_ids.iter().any(|r| *r == format!("{org}-admin")) {
        Access::Admin
    } else if role_ids.iter().any(|r| *r == format!("{org}-standard")) {
        Access::Standard
    } else {
        Access::ReadOnly
    }
}

// ================================================================ request helpers

pub fn cookie(headers: &HeaderMap, name: &str) -> Option<String> {
    headers.get_all(COOKIE).iter().filter_map(|v| v.to_str().ok()).flat_map(|v| v.split(';')).find_map(|kv| {
        let (k, v) = kv.trim().split_once('=')?;
        (k == name).then(|| v.to_string())
    })
}

pub fn session_cookie(st: &AppState, token: &str) -> HeaderValue {
    let secure = if st.config.cookie_secure { "; Secure" } else { "" };
    let max_age = st.config.session_hours * 3600;
    HeaderValue::from_str(&format!("{SESSION_COOKIE}={token}; Path=/; HttpOnly; SameSite=Lax; Max-Age={max_age}{secure}")).expect("cookie header")
}

pub fn clear_cookie(st: &AppState) -> HeaderValue {
    let secure = if st.config.cookie_secure { "; Secure" } else { "" };
    HeaderValue::from_str(&format!("{SESSION_COOKIE}=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0{secure}")).expect("cookie header")
}

/// Client address: the first `X-Forwarded-For` hop when running behind a trusted proxy, else the peer.
pub fn client_ip(st: &AppState, headers: &HeaderMap, peer: Option<SocketAddr>) -> Option<String> {
    if st.config.trust_proxy {
        if let Some(ip) = headers.get("x-forwarded-for").and_then(|v| v.to_str().ok()).and_then(|v| v.split(',').next()) {
            return Some(ip.trim().to_string());
        }
    }
    peer.map(|p| p.ip().to_string())
}

fn origin_allowed(st: &AppState, headers: &HeaderMap) -> bool {
    let origin = headers
        .get(ORIGIN)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string)
        .or_else(|| headers.get(REFERER).and_then(|v| v.to_str().ok()).and_then(|r| url::Url::parse(r).ok()).map(|u| u.origin().ascii_serialization()));
    let Some(origin) = origin else { return false };
    let public = url::Url::parse(&st.config.public_url).map(|u| u.origin().ascii_serialization()).unwrap_or_default();
    origin == public || st.config.cors_origins.iter().any(|o| *o == origin)
}

fn is_public(path: &str) -> bool {
    !path.starts_with("/api/") || path == "/api/openapi.json" || path.starts_with("/api/v1/auth/")
}

// ================================================================ middleware

#[derive(sqlx::FromRow)]
struct Principal {
    user_id: String,
    org_id: String,
    account_id: String,
    instance_admin: bool,
    org_active: bool,
    name: String,
    status: String,
    role_ids: Vec<String>,
    mfa_pending: bool,
    stale: bool,
}

const PRINCIPAL_SELECT: &str = "SELECT u.id::text AS user_id, u.org_id, u.account_id::text AS account_id, a.is_instance_admin AS instance_admin, \
    (o.status = 'active') AS org_active, u.name, u.status, \
    COALESCE((SELECT array_agg(role_id) FROM user_roles WHERE user_id = u.id), '{}') AS role_ids";
const PRINCIPAL_JOIN: &str = "JOIN accounts a ON a.id = u.account_id JOIN orgs o ON o.id = u.org_id";

impl Principal {
    fn context(self, credential: Credential, session_hash: Option<Vec<u8>>, ip: Option<String>) -> AuthContext {
        AuthContext {
            access: access_from_roles(&self.org_id, &self.role_ids),
            user_id: self.user_id,
            org_id: self.org_id,
            account_id: self.account_id,
            instance_admin: self.instance_admin,
            name: self.name,
            credential,
            session_hash,
            ip,
        }
    }
}

/// Resolves the request's credential (if any) to an [`AuthContext`].
pub async fn authenticate(st: &AppState, headers: &HeaderMap, peer: Option<SocketAddr>) -> Result<Option<AuthContext>, ApiError> {
    let ip = client_ip(st, headers, peer);
    if let Some(key) = headers.get(AUTHORIZATION).and_then(|v| v.to_str().ok()).and_then(|v| v.strip_prefix("Bearer ")) {
        let hash = crypto::sha256(key.trim());
        let p: Option<Principal> = sqlx::query_as(&format!(
            "{PRINCIPAL_SELECT}, false AS mfa_pending, (k.last_used_at IS NULL OR k.last_used_at < now() - interval '1 minute') AS stale \
             FROM app_keys k JOIN users u ON u.id = k.user_id {PRINCIPAL_JOIN} WHERE k.key_hash = $1 AND k.revoked_at IS NULL"
        ))
        .bind(&hash)
        .fetch_optional(&st.postgres)
        .await?;
        let Some(p) = p.filter(|p| p.status == "active" && p.org_active) else { return Err(ApiError::Unauthorized("invalid or revoked application key".into())) };
        if p.stale {
            sqlx::query("UPDATE app_keys SET last_used_at = now() WHERE key_hash = $1").bind(&hash).execute(&st.postgres).await?;
        }
        return Ok(Some(p.context(Credential::AppKey, None, ip)));
    }
    let Some(token) = cookie(headers, SESSION_COOKIE) else { return Ok(None) };
    let hash = crypto::sha256(&token);
    let p: Option<Principal> = sqlx::query_as(&format!(
        "{PRINCIPAL_SELECT}, s.mfa_pending, (s.last_seen_at < now() - interval '1 minute') AS stale \
         FROM sessions s JOIN users u ON u.id = s.user_id {PRINCIPAL_JOIN} WHERE s.token_hash = $1 AND s.expires_at > now()"
    ))
    .bind(&hash)
    .fetch_optional(&st.postgres)
    .await?;
    let Some(p) = p.filter(|p| p.status == "active" && p.org_active && !p.mfa_pending) else { return Ok(None) };
    if p.stale {
        // Sliding expiry, capped at the absolute lifetime.
        sqlx::query(&format!(
            "UPDATE sessions SET last_seen_at = now(), ip = COALESCE($2, ip), \
             expires_at = least(now() + make_interval(hours => $3), created_at + interval '{ABSOLUTE_SESSION_DAYS} days') WHERE token_hash = $1"
        ))
        .bind(&hash)
        .bind(&ip)
        .bind(st.config.session_hours as i32)
        .execute(&st.postgres)
        .await?;
    }
    Ok(Some(p.context(Credential::Session, Some(hash), ip)))
}

/// Rejects unauthenticated API calls and attaches the [`AuthContext`] for handlers.
pub async fn require(State(st): State<AppState>, mut req: Request, next: Next) -> Response {
    let peer = req.extensions().get::<ConnectInfo<SocketAddr>>().map(|c| c.0);
    let path = req.uri().path().to_string();
    let ctx = match authenticate(&st, req.headers(), peer).await {
        Ok(c) => c,
        Err(e) => return e.into_response(),
    };
    if let Some(ctx) = &ctx {
        let writes = !matches!(*req.method(), Method::GET | Method::HEAD | Method::OPTIONS);
        // The SAML ACS is a cross-site POST by design (the IdP's page submits it); it is validated by signature.
        let cross_site_by_design = path.starts_with("/api/v1/auth/saml/") && path.ends_with("/acs");
        if ctx.credential == Credential::Session && writes && !cross_site_by_design && !origin_allowed(&st, req.headers()) {
            return ApiError::Forbidden("cross-origin request rejected".into()).into_response();
        }
        req.extensions_mut().insert(ctx.clone());
    } else if !is_public(&path) {
        return ApiError::Unauthorized("sign in required".into()).into_response();
    }
    next.run(req).await
}

// ================================================================ extractors

#[async_trait]
impl FromRequestParts<AppState> for AuthContext {
    type Rejection = ApiError;
    async fn from_request_parts(parts: &mut Parts, _st: &AppState) -> Result<Self, ApiError> {
        parts.extensions.get::<AuthContext>().cloned().ok_or_else(|| ApiError::Unauthorized("sign in required".into()))
    }
}

/// Any authenticated user with at least Standard access (creates/edits content).
pub struct Writer(#[allow(dead_code)] pub AuthContext);

#[async_trait]
impl FromRequestParts<AppState> for Writer {
    type Rejection = ApiError;
    async fn from_request_parts(parts: &mut Parts, st: &AppState) -> Result<Self, ApiError> {
        let ctx = AuthContext::from_request_parts(parts, st).await?;
        if ctx.access < Access::Standard {
            return Err(ApiError::Forbidden("your role is read-only".into()));
        }
        Ok(Writer(ctx))
    }
}

/// Organization administrators (users, roles, SSO, audit trail).
pub struct Admin(pub AuthContext);

#[async_trait]
impl FromRequestParts<AppState> for Admin {
    type Rejection = ApiError;
    async fn from_request_parts(parts: &mut Parts, st: &AppState) -> Result<Self, ApiError> {
        let ctx = AuthContext::from_request_parts(parts, st).await?;
        if ctx.access < Access::Admin {
            return Err(ApiError::Forbidden("organization admin role required".into()));
        }
        Ok(Admin(ctx))
    }
}

/// Operators of the install: create, suspend and delete organizations.
pub struct InstanceAdmin(pub AuthContext);

#[async_trait]
impl FromRequestParts<AppState> for InstanceAdmin {
    type Rejection = ApiError;
    async fn from_request_parts(parts: &mut Parts, st: &AppState) -> Result<Self, ApiError> {
        let ctx = AuthContext::from_request_parts(parts, st).await?;
        if !ctx.instance_admin {
            return Err(ApiError::Forbidden("instance administrator required".into()));
        }
        Ok(InstanceAdmin(ctx))
    }
}

// ================================================================ sessions + audit

/// Starts a session acting as membership `user_id`; returns the cookie token. `mfa_pending` sessions only unlock MFA verification.
pub async fn create_session(st: &AppState, user_id: &str, org_id: &str, method: &str, mfa_pending: bool, ip: Option<&str>, user_agent: Option<&str>) -> Result<String, ApiError> {
    let token = crypto::random_token("ocs_");
    // An MFA challenge must be answered within 10 minutes.
    let hours = if mfa_pending { 0 } else { st.config.session_hours as i32 };
    let mins = if mfa_pending { 10 } else { 0 };
    let inserted = sqlx::query(
        "INSERT INTO sessions (token_hash, user_id, account_id, org_id, method, mfa_pending, ip, user_agent, expires_at) \
         SELECT $1, id, account_id, $3, $4, $5, $6, $7, now() + make_interval(hours => $8, mins => $9) FROM users WHERE id = $2::uuid AND org_id = $3",
    )
    .bind(crypto::sha256(&token))
    .bind(user_id)
    .bind(org_id)
    .bind(method)
    .bind(mfa_pending)
    .bind(ip)
    .bind(user_agent.map(|u| u.chars().take(300).collect::<String>()))
    .bind(hours)
    .bind(mins)
    .execute(&st.postgres)
    .await?;
    if inserted.rows_affected() == 0 {
        return Err(ApiError::Unauthorized("not a member of this organization".into()));
    }
    if !mfa_pending {
        sqlx::query("UPDATE users SET last_login_at = now() WHERE id = $1::uuid").bind(user_id).execute(&st.postgres).await?;
        sqlx::query("UPDATE accounts SET last_login_at = now(), failed_logins = 0, locked_until = NULL, last_org_id = $2 WHERE id = (SELECT account_id FROM users WHERE id = $1::uuid)")
            .bind(user_id)
            .bind(org_id)
            .execute(&st.postgres)
            .await?;
    }
    Ok(token)
}

#[allow(clippy::too_many_arguments)]
pub async fn audit(pg: &sqlx::PgPool, org: &str, actor_id: Option<&str>, actor: &str, action: &str, target: &str, detail: &str, ip: Option<&str>) {
    let r = sqlx::query("INSERT INTO audit_events (org_id, actor_id, actor, action, target, detail, ip) VALUES ($1, $2::uuid, $3, $4, $5, $6, $7)")
        .bind(org)
        .bind(actor_id)
        .bind(actor)
        .bind(action)
        .bind(target)
        .bind(detail)
        .bind(ip)
        .execute(pg)
        .await;
    if let Err(e) = r {
        tracing::error!(error = %e, action, "failed to write audit event");
    }
}

/// Instance-level events (organizations created, suspended, deleted); kept even when the org is gone.
pub async fn instance_audit(pg: &sqlx::PgPool, ctx: &AuthContext, action: &str, target: &str, detail: &str) {
    let r = sqlx::query("INSERT INTO instance_audit_events (actor_id, actor, action, target, detail, ip) VALUES ($1::uuid, $2, $3, $4, $5, $6)")
        .bind(&ctx.account_id)
        .bind(&ctx.name)
        .bind(action)
        .bind(target)
        .bind(detail)
        .bind(&ctx.ip)
        .execute(pg)
        .await;
    if let Err(e) = r {
        tracing::error!(error = %e, action, "failed to write instance audit event");
    }
}

impl AuthContext {
    pub async fn audit(&self, st: &AppState, action: &str, target: &str, detail: &str) {
        audit(&st.postgres, &self.org_id, Some(&self.user_id), &self.name, action, target, detail, self.ip.as_deref()).await;
    }
}

/// Removes expired sessions, used one-time links, stale SSO requests and replay records.
pub async fn sweep(pg: &sqlx::PgPool) {
    for q in [
        "DELETE FROM sessions WHERE expires_at < now()",
        "DELETE FROM login_tokens WHERE expires_at < now() - interval '7 days'",
        "DELETE FROM sso_requests WHERE created_at < now() - interval '15 minutes'",
        "DELETE FROM saml_seen_assertions WHERE expires_at < now()",
    ] {
        if let Err(e) = sqlx::query(q).execute(pg).await {
            tracing::warn!(error = %e, "auth sweep failed");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn access_levels() {
        assert_eq!(access_from_roles("default", &["default-admin".into(), "x".into()]), Access::Admin);
        assert_eq!(access_from_roles("default", &["default-standard".into()]), Access::Standard);
        assert_eq!(access_from_roles("default", &["custom".into()]), Access::ReadOnly);
        // Another org's admin role grants nothing here.
        assert_eq!(access_from_roles("default", &["other-admin".into()]), Access::ReadOnly);
    }

    #[test]
    fn cookie_parsing() {
        let mut h = HeaderMap::new();
        h.insert(COOKIE, HeaderValue::from_static("a=1; oc_session=ocs_abc; b=2"));
        assert_eq!(cookie(&h, SESSION_COOKIE).as_deref(), Some("ocs_abc"));
        assert_eq!(cookie(&h, "missing"), None);
    }

    #[test]
    fn public_paths() {
        assert!(is_public("/"));
        assert!(is_public("/dashboards/abc"));
        assert!(is_public("/api/v1/auth/login"));
        assert!(!is_public("/api/v1/logs/search"));
        assert!(!is_public("/api/v1/me"));
    }
}
