//! Sign-in (password + MFA, one-time links), sign-out and self-service security settings
//! (password, MFA, sessions, application keys). SSO lives in `sso.rs`.

use std::net::SocketAddr;

use axum::extract::{ConnectInfo, Path, Query, State};
use axum::http::header::{SET_COOKIE, USER_AGENT};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use crate::auth::{self, crypto, password, totp, AuthContext, Credential, SESSION_COOKIE};
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;

/// Failed password / MFA attempts before an account is locked for [`LOCK_MINUTES`].
const MAX_FAILURES: i32 = 10;
const LOCK_MINUTES: i32 = 15;
const GENERIC_FAILURE: &str = "Invalid email or password";

fn user_agent(h: &HeaderMap) -> Option<String> {
    h.get(USER_AGENT).and_then(|v| v.to_str().ok()).map(str::to_string)
}

fn with_cookie<T: Serialize>(cookie: axum::http::HeaderValue, body: T) -> Response {
    ([(SET_COOKIE, cookie)], Json(body)).into_response()
}

// ================================================================ login configuration

#[derive(Deserialize, IntoParams)]
pub struct OrgParam {
    /// Organization id; defaults to the install's organization.
    pub org: Option<String>,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SsoOption {
    /// Where the browser goes to start this sign-in.
    pub login_url: String,
    pub label: String,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct LoginConfig {
    pub org_id: String,
    pub org_name: String,
    pub password_enabled: bool,
    pub saml: Option<SsoOption>,
    pub oidc: Option<SsoOption>,
}

#[derive(sqlx::FromRow)]
pub(crate) struct LoginSettings {
    pub password_enabled: bool,
    pub sso_required: bool,
    pub saml_enabled: bool,
    pub oidc_enabled: bool,
    pub oidc_button_label: String,
}

pub(crate) async fn login_settings(st: &AppState, org: &str) -> Result<LoginSettings, ApiError> {
    let s: Option<LoginSettings> = sqlx::query_as("SELECT password_enabled, sso_required, saml_enabled, oidc_enabled, oidc_button_label FROM auth_settings WHERE org_id = $1")
        .bind(org)
        .fetch_optional(&st.postgres)
        .await?;
    Ok(s.unwrap_or(LoginSettings { password_enabled: true, sso_required: false, saml_enabled: false, oidc_enabled: false, oidc_button_label: "Sign in with SSO".into() }))
}

/// What the sign-in page should offer (public).
#[utoipa::path(get, path = "/api/v1/auth/config", params(OrgParam), responses((status = 200, body = LoginConfig), (status = 404, body = crate::error::ErrorBody)), tag = "auth")]
pub async fn config(State(st): State<AppState>, Query(p): Query<OrgParam>) -> ApiResult<LoginConfig> {
    let org = p.org.unwrap_or_else(|| st.config.dev_org.clone());
    let name: Option<(String,)> = sqlx::query_as("SELECT name FROM orgs WHERE id = $1").bind(&org).fetch_optional(&st.postgres).await?;
    let Some((org_name,)) = name else { return Err(ApiError::NotFound("Unknown organization".into())) };
    let s = login_settings(&st, &org).await?;
    let enc = url::form_urlencoded::byte_serialize(org.as_bytes()).collect::<String>();
    Ok(Json(LoginConfig {
        password_enabled: s.password_enabled,
        saml: s.saml_enabled.then(|| SsoOption { login_url: format!("/api/v1/auth/saml/{enc}/login"), label: "Sign in with SAML".into() }),
        oidc: s.oidc_enabled.then(|| SsoOption { login_url: format!("/api/v1/auth/oidc/{enc}/login"), label: s.oidc_button_label }),
        org_id: org,
        org_name,
    }))
}

// ================================================================ password login + MFA

#[derive(Deserialize, ToSchema)]
pub struct LoginRequest {
    pub email: String,
    pub password: String,
    pub org: Option<String>,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct LoginResult {
    /// The session is pending until a TOTP code is verified at /api/v1/auth/mfa/verify.
    pub mfa_required: bool,
}

#[derive(sqlx::FromRow)]
struct LoginUser {
    id: String,
    name: String,
    status: String,
    password_hash: Option<String>,
    mfa_secret_enc: Option<String>,
    locked_minutes: Option<i32>,
    is_admin: bool,
}

async fn record_failure(st: &AppState, user_id: &str) -> Result<(), ApiError> {
    sqlx::query(
        "UPDATE users SET failed_logins = failed_logins + 1, \
         locked_until = CASE WHEN failed_logins + 1 >= $2 THEN now() + make_interval(mins => $3) ELSE locked_until END WHERE id = $1::uuid",
    )
    .bind(user_id)
    .bind(MAX_FAILURES)
    .bind(LOCK_MINUTES)
    .execute(&st.postgres)
    .await?;
    Ok(())
}

#[utoipa::path(post, path = "/api/v1/auth/login", request_body = LoginRequest, responses((status = 200, body = LoginResult), (status = 401, body = crate::error::ErrorBody), (status = 429, body = crate::error::ErrorBody)), tag = "auth")]
pub async fn login(State(st): State<AppState>, ConnectInfo(peer): ConnectInfo<SocketAddr>, headers: HeaderMap, Json(req): Json<LoginRequest>) -> Result<Response, ApiError> {
    let org = req.org.clone().unwrap_or_else(|| st.config.dev_org.clone());
    let ip = auth::client_ip(&st, &headers, Some(peer));
    let email = req.email.trim().to_lowercase();
    let settings = login_settings(&st, &org).await?;
    let user: Option<LoginUser> = sqlx::query_as(
        "SELECT u.id::text AS id, u.name, u.status, u.password_hash, u.mfa_secret_enc, \
         CASE WHEN u.locked_until > now() THEN ceil(extract(epoch FROM u.locked_until - now()) / 60)::int END AS locked_minutes, \
         EXISTS (SELECT 1 FROM user_roles r WHERE r.user_id = u.id AND r.role_id = u.org_id || '-admin') AS is_admin \
         FROM users u WHERE u.org_id = $1 AND lower(u.email) = $2",
    )
    .bind(&org)
    .bind(&email)
    .fetch_optional(&st.postgres)
    .await?;

    let Some(user) = user else {
        password::verify_dummy(&req.password);
        auth::audit(&st.postgres, &org, None, &email, "login.failed", &email, "unknown account", ip.as_deref()).await;
        return Err(ApiError::Unauthorized(GENERIC_FAILURE.into()));
    };
    if let Some(m) = user.locked_minutes {
        auth::audit(&st.postgres, &org, Some(&user.id), &user.name, "login.locked", &email, "", ip.as_deref()).await;
        return Err(ApiError::TooManyRequests(format!("Too many failed attempts. Try again in {m} minute{}.", if m == 1 { "" } else { "s" })));
    }
    // Admins keep password access when SSO is required, so a broken IdP can't lock everyone out.
    if !settings.password_enabled || (settings.sso_required && !user.is_admin) {
        return Err(ApiError::Forbidden("Password sign-in is disabled for this organization. Use single sign-on.".into()));
    }
    let ok = match &user.password_hash {
        Some(h) => password::verify(&req.password, h),
        None => {
            password::verify_dummy(&req.password);
            false
        }
    };
    if !ok || user.status != "active" {
        record_failure(&st, &user.id).await?;
        auth::audit(&st.postgres, &org, Some(&user.id), &user.name, "login.failed", &email, if ok { "account not active" } else { "wrong password" }, ip.as_deref()).await;
        return Err(ApiError::Unauthorized(GENERIC_FAILURE.into()));
    }
    let mfa = user.mfa_secret_enc.is_some();
    let token = auth::create_session(&st, &user.id, &org, "password", mfa, ip.as_deref(), user_agent(&headers).as_deref()).await?;
    if !mfa {
        auth::audit(&st.postgres, &org, Some(&user.id), &user.name, "login.succeeded", &email, "password", ip.as_deref()).await;
    }
    Ok(with_cookie(auth::session_cookie(&st, &token), LoginResult { mfa_required: mfa }))
}

#[derive(Deserialize, ToSchema)]
pub struct CodeRequest {
    /// 6-digit code from the authenticator app.
    pub code: String,
}

#[utoipa::path(post, path = "/api/v1/auth/mfa/verify", request_body = CodeRequest, responses((status = 200, body = LoginResult), (status = 401, body = crate::error::ErrorBody)), tag = "auth")]
pub async fn mfa_verify(State(st): State<AppState>, ConnectInfo(peer): ConnectInfo<SocketAddr>, headers: HeaderMap, Json(req): Json<CodeRequest>) -> Result<Response, ApiError> {
    let ip = auth::client_ip(&st, &headers, Some(peer));
    let token = auth::cookie(&headers, SESSION_COOKIE).ok_or_else(|| ApiError::Unauthorized("Sign in first".into()))?;
    let hash = crypto::sha256(&token);
    let pending: Option<(String, String, String, Option<String>, String, Option<i32>)> = sqlx::query_as(
        "SELECT u.id::text, u.org_id, u.name, u.mfa_secret_enc, s.method, \
         CASE WHEN u.locked_until > now() THEN ceil(extract(epoch FROM u.locked_until - now()) / 60)::int END \
         FROM sessions s JOIN users u ON u.id = s.user_id WHERE s.token_hash = $1 AND s.mfa_pending AND s.expires_at > now()",
    )
    .bind(&hash)
    .fetch_optional(&st.postgres)
    .await?;
    let Some((user_id, org, name, Some(secret_enc), method, locked)) = pending else { return Err(ApiError::Unauthorized("Your sign-in expired. Sign in again.".into())) };
    if let Some(m) = locked {
        return Err(ApiError::TooManyRequests(format!("Too many failed attempts. Try again in {m} minutes.")));
    }
    let secret = st.secrets.decrypt(&secret_enc)?;
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs();
    if !totp::verify(&secret, &req.code, now) {
        record_failure(&st, &user_id).await?;
        auth::audit(&st.postgres, &org, Some(&user_id), &name, "login.mfa_failed", "", "", ip.as_deref()).await;
        return Err(ApiError::Unauthorized("Invalid code".into()));
    }
    sqlx::query("DELETE FROM sessions WHERE token_hash = $1").bind(&hash).execute(&st.postgres).await?;
    let token = auth::create_session(&st, &user_id, &org, &method, false, ip.as_deref(), user_agent(&headers).as_deref()).await?;
    auth::audit(&st.postgres, &org, Some(&user_id), &name, "login.succeeded", "", &format!("{method} + mfa"), ip.as_deref()).await;
    Ok(with_cookie(auth::session_cookie(&st, &token), LoginResult { mfa_required: false }))
}

/// Ends this browser session.
#[utoipa::path(post, path = "/api/v1/auth/logout", responses((status = 204)), tag = "auth")]
pub async fn logout(State(st): State<AppState>, headers: HeaderMap) -> Result<Response, ApiError> {
    if let Some(token) = auth::cookie(&headers, SESSION_COOKIE) {
        sqlx::query("DELETE FROM sessions WHERE token_hash = $1").bind(crypto::sha256(&token)).execute(&st.postgres).await?;
    }
    Ok((StatusCode::NO_CONTENT, [(SET_COOKIE, auth::clear_cookie(&st))]).into_response())
}

// ================================================================ one-time links (invite / reset / bootstrap)

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct LinkInfo {
    pub email: String,
    pub name: String,
    pub org_name: String,
    /// invite | reset | bootstrap
    pub purpose: String,
}

#[derive(sqlx::FromRow)]
struct LinkRow {
    user_id: String,
    org_id: String,
    email: String,
    name: String,
    org_name: String,
    purpose: String,
    mfa_enabled: bool,
}

async fn find_link(st: &AppState, token: &str) -> Result<LinkRow, ApiError> {
    let row: Option<LinkRow> = sqlx::query_as(
        "SELECT u.id::text AS user_id, u.org_id, u.email, u.name, o.name AS org_name, t.purpose, (u.mfa_secret_enc IS NOT NULL) AS mfa_enabled \
         FROM login_tokens t JOIN users u ON u.id = t.user_id JOIN orgs o ON o.id = t.org_id \
         WHERE t.token_hash = $1 AND t.used_at IS NULL AND t.expires_at > now() AND u.status <> 'deactivated'",
    )
    .bind(crypto::sha256(token))
    .fetch_optional(&st.postgres)
    .await?;
    row.ok_or_else(|| ApiError::NotFound("This link is invalid, expired or was already used. Ask an admin for a new one.".into()))
}

#[utoipa::path(get, path = "/api/v1/auth/link/{token}", params(("token" = String, Path)), responses((status = 200, body = LinkInfo), (status = 404, body = crate::error::ErrorBody)), tag = "auth")]
pub async fn link_info(State(st): State<AppState>, Path(token): Path<String>) -> ApiResult<LinkInfo> {
    let l = find_link(&st, &token).await?;
    Ok(Json(LinkInfo { email: l.email, name: l.name, org_name: l.org_name, purpose: l.purpose }))
}

#[derive(Deserialize, ToSchema)]
pub struct AcceptLinkRequest {
    pub name: Option<String>,
    pub password: String,
}

/// Sets the password from an invite / reset link and signs in (MFA still applies if enabled).
#[utoipa::path(post, path = "/api/v1/auth/link/{token}", params(("token" = String, Path)), request_body = AcceptLinkRequest, responses((status = 200, body = LoginResult), (status = 400, body = crate::error::ErrorBody)), tag = "auth")]
pub async fn accept_link(State(st): State<AppState>, ConnectInfo(peer): ConnectInfo<SocketAddr>, headers: HeaderMap, Path(token): Path<String>, Json(req): Json<AcceptLinkRequest>) -> Result<Response, ApiError> {
    let ip = auth::client_ip(&st, &headers, Some(peer));
    let link = find_link(&st, &token).await?;
    password::check_policy(&req.password).map_err(ApiError::BadRequest)?;
    let hash = password::hash(&req.password)?;
    let name = req.name.as_deref().map(str::trim).filter(|n| !n.is_empty() && n.chars().count() <= 120).map(str::to_string);
    let mut tx = st.postgres.begin().await?;
    let used = sqlx::query("UPDATE login_tokens SET used_at = now() WHERE token_hash = $1 AND used_at IS NULL").bind(crypto::sha256(&token)).execute(&mut *tx).await?;
    if used.rows_affected() == 0 {
        return Err(ApiError::NotFound("This link was already used.".into()));
    }
    // Other outstanding links for this user stop working once a password is set.
    sqlx::query("UPDATE login_tokens SET used_at = now() WHERE user_id = $1::uuid AND used_at IS NULL").bind(&link.user_id).execute(&mut *tx).await?;
    sqlx::query(
        "UPDATE users SET password_hash = $2, name = COALESCE($3, name), status = 'active', failed_logins = 0, locked_until = NULL, \
         login_methods = CASE WHEN 'password' = ANY(login_methods) THEN login_methods ELSE array_append(login_methods, 'password') END WHERE id = $1::uuid",
    )
    .bind(&link.user_id)
    .bind(&hash)
    .bind(&name)
    .execute(&mut *tx)
    .await?;
    // A password change ends every existing session.
    sqlx::query("DELETE FROM sessions WHERE user_id = $1::uuid").bind(&link.user_id).execute(&mut *tx).await?;
    tx.commit().await?;
    let display = name.unwrap_or(link.name);
    auth::audit(&st.postgres, &link.org_id, Some(&link.user_id), &display, &format!("user.{}_accepted", link.purpose), &link.email, "", ip.as_deref()).await;
    let token = auth::create_session(&st, &link.user_id, &link.org_id, "invite", link.mfa_enabled, ip.as_deref(), user_agent(&headers).as_deref()).await?;
    Ok(with_cookie(auth::session_cookie(&st, &token), LoginResult { mfa_required: link.mfa_enabled }))
}

/// Issues a one-time link; returns the URL to hand to the user. Valid for `hours`.
pub(crate) async fn issue_link(st: &AppState, org: &str, user_id: &str, purpose: &str, created_by: Option<&str>, hours: i32) -> Result<String, ApiError> {
    let token = crypto::random_token("ocl_");
    sqlx::query("UPDATE login_tokens SET used_at = now() WHERE user_id = $1::uuid AND purpose = $2 AND used_at IS NULL").bind(user_id).bind(purpose).execute(&st.postgres).await?;
    sqlx::query("INSERT INTO login_tokens (token_hash, user_id, org_id, purpose, created_by, expires_at) VALUES ($1, $2::uuid, $3, $4, $5::uuid, now() + make_interval(hours => $6))")
        .bind(crypto::sha256(&token))
        .bind(user_id)
        .bind(org)
        .bind(purpose)
        .bind(created_by)
        .bind(hours)
        .execute(&st.postgres)
        .await?;
    Ok(format!("{}/login/link/{token}", st.config.public_url))
}

/// While nobody in the install's org can sign in (no password set, no SSO), log a one-time link
/// for the first admin. Runs at startup.
pub async fn bootstrap_admin(st: &AppState) -> anyhow::Result<()> {
    let org = &st.config.dev_org;
    let (can_sign_in,): (bool,) = sqlx::query_as(
        "SELECT EXISTS (SELECT 1 FROM users WHERE org_id = $1 AND password_hash IS NOT NULL AND status = 'active') \
         OR EXISTS (SELECT 1 FROM auth_settings WHERE org_id = $1 AND (saml_enabled OR oidc_enabled))",
    )
    .bind(org)
    .fetch_one(&st.postgres)
    .await?;
    if can_sign_in {
        return Ok(());
    }
    let exists: Option<(String,)> = sqlx::query_as("SELECT email FROM users WHERE org_id = $1 AND id::text = $2").bind(org).bind(&st.config.dev_user).fetch_optional(&st.postgres).await?;
    let Some((email,)) = exists else {
        tracing::warn!(user = %st.config.dev_user, "no bootstrap admin user found; nobody can sign in");
        return Ok(());
    };
    let url = issue_link(st, org, &st.config.dev_user, "bootstrap", None, 24).await.map_err(|e| anyhow::anyhow!("{e}"))?;
    tracing::warn!("No one can sign in yet. Set the admin password for {email} (valid 24h, single use): {url}");
    Ok(())
}

// ================================================================ self-service security

fn require_session(ctx: &AuthContext) -> Result<(), ApiError> {
    if ctx.credential != Credential::Session {
        return Err(ApiError::Forbidden("this action needs a signed-in browser session, not an application key".into()));
    }
    Ok(())
}

/// Ends every session of the current user, including this one.
#[utoipa::path(post, path = "/api/v1/me/logout-all", responses((status = 204)), tag = "auth")]
pub async fn logout_all(State(st): State<AppState>, ctx: AuthContext) -> Result<Response, ApiError> {
    let n = sqlx::query("DELETE FROM sessions WHERE user_id = $1::uuid").bind(&ctx.user_id).execute(&st.postgres).await?.rows_affected();
    ctx.audit(&st, "session.revoked_all", "", &format!("{n} sessions")).await;
    Ok((StatusCode::NO_CONTENT, [(SET_COOKIE, auth::clear_cookie(&st))]).into_response())
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct PasswordChange {
    /// Required when the account already has a password.
    pub current_password: Option<String>,
    pub new_password: String,
}

/// Changes the password and signs out every other session.
#[utoipa::path(post, path = "/api/v1/me/password", request_body = PasswordChange, responses((status = 204), (status = 400, body = crate::error::ErrorBody)), tag = "auth")]
pub async fn change_password(State(st): State<AppState>, ctx: AuthContext, Json(req): Json<PasswordChange>) -> Result<StatusCode, ApiError> {
    require_session(&ctx)?;
    let (current,): (Option<String>,) = sqlx::query_as("SELECT password_hash FROM users WHERE id = $1::uuid").bind(&ctx.user_id).fetch_one(&st.postgres).await?;
    if let Some(h) = current {
        if !req.current_password.as_deref().is_some_and(|p| password::verify(p, &h)) {
            return Err(ApiError::BadRequest("Current password is incorrect".into()));
        }
    }
    password::check_policy(&req.new_password).map_err(ApiError::BadRequest)?;
    let hash = password::hash(&req.new_password)?;
    sqlx::query("UPDATE users SET password_hash = $2, login_methods = CASE WHEN 'password' = ANY(login_methods) THEN login_methods ELSE array_append(login_methods, 'password') END WHERE id = $1::uuid")
        .bind(&ctx.user_id)
        .bind(&hash)
        .execute(&st.postgres)
        .await?;
    sqlx::query("DELETE FROM sessions WHERE user_id = $1::uuid AND token_hash <> $2").bind(&ctx.user_id).bind(ctx.session_hash.clone().unwrap_or_default()).execute(&st.postgres).await?;
    ctx.audit(&st, "user.password_changed", "", "").await;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Serialize, ToSchema, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct SessionInfo {
    pub id: String,
    pub method: String,
    pub ip: Option<String>,
    pub user_agent: Option<String>,
    pub created_at: i64,
    pub last_seen_at: i64,
    pub current: bool,
}

#[utoipa::path(get, path = "/api/v1/me/sessions", responses((status = 200, body = Vec<SessionInfo>)), tag = "auth")]
pub async fn sessions(State(st): State<AppState>, ctx: AuthContext) -> ApiResult<Vec<SessionInfo>> {
    let rows = sqlx::query_as(
        "SELECT encode(token_hash, 'hex') AS id, method, ip, user_agent, (extract(epoch FROM created_at) * 1000)::bigint AS created_at, \
         (extract(epoch FROM last_seen_at) * 1000)::bigint AS last_seen_at, token_hash = $2 AS current \
         FROM sessions WHERE user_id = $1::uuid AND NOT mfa_pending AND expires_at > now() ORDER BY last_seen_at DESC",
    )
    .bind(&ctx.user_id)
    .bind(ctx.session_hash.clone().unwrap_or_default())
    .fetch_all(&st.postgres)
    .await?;
    Ok(Json(rows))
}

#[utoipa::path(delete, path = "/api/v1/me/sessions/{id}", params(("id" = String, Path)), responses((status = 204), (status = 404, body = crate::error::ErrorBody)), tag = "auth")]
pub async fn revoke_session(State(st): State<AppState>, ctx: AuthContext, Path(id): Path<String>) -> Result<StatusCode, ApiError> {
    let n = sqlx::query("DELETE FROM sessions WHERE user_id = $1::uuid AND encode(token_hash, 'hex') = $2").bind(&ctx.user_id).bind(&id).execute(&st.postgres).await?.rows_affected();
    if n == 0 {
        return Err(ApiError::NotFound("Session not found".into()));
    }
    ctx.audit(&st, "session.revoked", &id[..12.min(id.len())], "").await;
    Ok(StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------- MFA

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct MfaSetup {
    /// Base32 secret for manual entry.
    pub secret: String,
    /// `otpauth://` URI for the QR code.
    pub otpauth_url: String,
}

/// Starts enrollment: a new secret that becomes active once confirmed with a code.
#[utoipa::path(post, path = "/api/v1/me/mfa/setup", responses((status = 200, body = MfaSetup)), tag = "auth")]
pub async fn mfa_setup(State(st): State<AppState>, ctx: AuthContext) -> ApiResult<MfaSetup> {
    require_session(&ctx)?;
    let (email, enabled): (String, bool) = sqlx::query_as("SELECT email, mfa_secret_enc IS NOT NULL FROM users WHERE id = $1::uuid").bind(&ctx.user_id).fetch_one(&st.postgres).await?;
    if enabled {
        return Err(ApiError::BadRequest("MFA is already enabled. Disable it first to enroll a new device.".into()));
    }
    let secret = totp::new_secret();
    sqlx::query("UPDATE users SET mfa_pending_enc = $2 WHERE id = $1::uuid").bind(&ctx.user_id).bind(st.secrets.encrypt(&secret)).execute(&st.postgres).await?;
    Ok(Json(MfaSetup { otpauth_url: totp::otpauth_url(&secret, &email), secret }))
}

fn now_unix() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs()
}

#[utoipa::path(post, path = "/api/v1/me/mfa/enable", request_body = CodeRequest, responses((status = 204), (status = 400, body = crate::error::ErrorBody)), tag = "auth")]
pub async fn mfa_enable(State(st): State<AppState>, ctx: AuthContext, Json(req): Json<CodeRequest>) -> Result<StatusCode, ApiError> {
    require_session(&ctx)?;
    let (pending,): (Option<String>,) = sqlx::query_as("SELECT mfa_pending_enc FROM users WHERE id = $1::uuid").bind(&ctx.user_id).fetch_one(&st.postgres).await?;
    let pending = pending.ok_or_else(|| ApiError::BadRequest("Start MFA setup first".into()))?;
    if !totp::verify(&st.secrets.decrypt(&pending)?, &req.code, now_unix()) {
        return Err(ApiError::BadRequest("That code doesn't match. Check the time on your device and try again.".into()));
    }
    sqlx::query("UPDATE users SET mfa_secret_enc = mfa_pending_enc, mfa_pending_enc = NULL, mfa_enabled = true WHERE id = $1::uuid").bind(&ctx.user_id).execute(&st.postgres).await?;
    ctx.audit(&st, "user.mfa_enabled", "", "").await;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(post, path = "/api/v1/me/mfa/disable", request_body = CodeRequest, responses((status = 204), (status = 400, body = crate::error::ErrorBody)), tag = "auth")]
pub async fn mfa_disable(State(st): State<AppState>, ctx: AuthContext, Json(req): Json<CodeRequest>) -> Result<StatusCode, ApiError> {
    require_session(&ctx)?;
    let (secret,): (Option<String>,) = sqlx::query_as("SELECT mfa_secret_enc FROM users WHERE id = $1::uuid").bind(&ctx.user_id).fetch_one(&st.postgres).await?;
    let secret = secret.ok_or_else(|| ApiError::BadRequest("MFA is not enabled".into()))?;
    if !totp::verify(&st.secrets.decrypt(&secret)?, &req.code, now_unix()) {
        return Err(ApiError::BadRequest("Invalid code".into()));
    }
    sqlx::query("UPDATE users SET mfa_secret_enc = NULL, mfa_pending_enc = NULL, mfa_enabled = false WHERE id = $1::uuid").bind(&ctx.user_id).execute(&st.postgres).await?;
    ctx.audit(&st, "user.mfa_disabled", "", "").await;
    Ok(StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------- application keys

#[derive(Serialize, ToSchema, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct AppKey {
    pub id: String,
    pub name: String,
    pub prefix: String,
    pub created_at: i64,
    pub last_used_at: Option<i64>,
}

#[utoipa::path(get, path = "/api/v1/me/app-keys", responses((status = 200, body = Vec<AppKey>)), tag = "auth")]
pub async fn app_keys(State(st): State<AppState>, ctx: AuthContext) -> ApiResult<Vec<AppKey>> {
    let rows = sqlx::query_as(
        "SELECT id::text, name, prefix, (extract(epoch FROM created_at) * 1000)::bigint AS created_at, (extract(epoch FROM last_used_at) * 1000)::bigint AS last_used_at \
         FROM app_keys WHERE user_id = $1::uuid AND revoked_at IS NULL ORDER BY created_at DESC",
    )
    .bind(&ctx.user_id)
    .fetch_all(&st.postgres)
    .await?;
    Ok(Json(rows))
}

#[derive(Deserialize, ToSchema)]
pub struct CreateAppKey {
    pub name: String,
}

#[derive(Serialize, ToSchema)]
pub struct CreatedAppKey {
    #[serde(flatten)]
    pub info: AppKey,
    /// The secret. Shown only once.
    pub key: String,
}

#[utoipa::path(post, path = "/api/v1/me/app-keys", request_body = CreateAppKey, responses((status = 200, body = CreatedAppKey)), tag = "auth")]
pub async fn create_app_key(State(st): State<AppState>, ctx: AuthContext, Json(req): Json<CreateAppKey>) -> ApiResult<CreatedAppKey> {
    require_session(&ctx)?;
    let name = req.name.trim();
    if name.is_empty() || name.chars().count() > 100 {
        return Err(ApiError::BadRequest("name must be 1–100 characters".into()));
    }
    let (count,): (i64,) = sqlx::query_as("SELECT count(*) FROM app_keys WHERE user_id = $1::uuid AND revoked_at IS NULL").bind(&ctx.user_id).fetch_one(&st.postgres).await?;
    if count >= 25 {
        return Err(ApiError::BadRequest("You already have 25 active keys. Revoke one first.".into()));
    }
    let key = crypto::random_token("ocak_");
    let prefix: String = key.chars().take(10).collect();
    let info: AppKey = sqlx::query_as(
        "INSERT INTO app_keys (org_id, user_id, name, key_hash, prefix) VALUES ($1, $2::uuid, $3, $4, $5) \
         RETURNING id::text, name, prefix, (extract(epoch FROM created_at) * 1000)::bigint AS created_at, NULL::bigint AS last_used_at",
    )
    .bind(&ctx.org_id)
    .bind(&ctx.user_id)
    .bind(name)
    .bind(crypto::sha256(&key))
    .bind(&prefix)
    .fetch_one(&st.postgres)
    .await?;
    ctx.audit(&st, "app_key.created", &prefix, name).await;
    Ok(Json(CreatedAppKey { info, key }))
}

#[utoipa::path(delete, path = "/api/v1/me/app-keys/{id}", params(("id" = String, Path)), responses((status = 204), (status = 404, body = crate::error::ErrorBody)), tag = "auth")]
pub async fn revoke_app_key(State(st): State<AppState>, ctx: AuthContext, Path(id): Path<String>) -> Result<StatusCode, ApiError> {
    let row: Option<(String,)> = sqlx::query_as("UPDATE app_keys SET revoked_at = now() WHERE user_id = $1::uuid AND id::text = $2 AND revoked_at IS NULL RETURNING prefix")
        .bind(&ctx.user_id)
        .bind(&id)
        .fetch_optional(&st.postgres)
        .await?;
    let Some((prefix,)) = row else { return Err(ApiError::NotFound("Key not found".into())) };
    ctx.audit(&st, "app_key.revoked", &prefix, "").await;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SecurityStatus {
    pub has_password: bool,
    pub mfa_enabled: bool,
    pub access: auth::Access,
}

#[utoipa::path(get, path = "/api/v1/me/security", responses((status = 200, body = SecurityStatus)), tag = "auth")]
pub async fn security(State(st): State<AppState>, ctx: AuthContext) -> ApiResult<SecurityStatus> {
    let (has_password, mfa_enabled): (bool, bool) =
        sqlx::query_as("SELECT password_hash IS NOT NULL, mfa_secret_enc IS NOT NULL FROM users WHERE id = $1::uuid").bind(&ctx.user_id).fetch_one(&st.postgres).await?;
    Ok(Json(SecurityStatus { has_password, mfa_enabled, access: ctx.access }))
}
