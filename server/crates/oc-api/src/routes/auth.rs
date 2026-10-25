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
    pub saml_enabled: bool,
    pub oidc_enabled: bool,
    pub oidc_button_label: String,
}

pub(crate) async fn login_settings(st: &AppState, org: &str) -> Result<LoginSettings, ApiError> {
    let s: Option<LoginSettings> = sqlx::query_as("SELECT password_enabled, saml_enabled, oidc_enabled, oidc_button_label FROM auth_settings WHERE org_id = $1")
        .bind(org)
        .fetch_optional(&st.postgres)
        .await?;
    Ok(s.unwrap_or(LoginSettings { password_enabled: true, saml_enabled: false, oidc_enabled: false, oidc_button_label: "Sign in with SSO".into() }))
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
struct LoginAccount {
    id: String,
    name: String,
    password_hash: Option<String>,
    mfa_secret_enc: Option<String>,
    locked_minutes: Option<i32>,
}

/// An account's active membership in an active org, with that org's password policy.
#[derive(sqlx::FromRow)]
struct LoginMembership {
    user_id: String,
    org_id: String,
    is_admin: bool,
    password_enabled: bool,
    sso_required: bool,
}

impl LoginMembership {
    /// Admins keep password access when SSO is required, so a broken IdP can't lock everyone out.
    fn allows_password(&self) -> bool {
        self.password_enabled && (!self.sso_required || self.is_admin)
    }
}

/// The account's usable memberships, best first: `prefer` (the requested org), then the last org used.
async fn memberships(st: &AppState, account_id: &str, prefer: Option<&str>) -> Result<Vec<LoginMembership>, ApiError> {
    Ok(sqlx::query_as(
        "SELECT u.id::text AS user_id, u.org_id, \
         EXISTS (SELECT 1 FROM user_roles r WHERE r.user_id = u.id AND r.role_id = u.org_id || '-admin') AS is_admin, \
         COALESCE(s.password_enabled, true) AS password_enabled, COALESCE(s.sso_required, false) AS sso_required \
         FROM users u JOIN accounts a ON a.id = u.account_id JOIN orgs o ON o.id = u.org_id AND o.status = 'active' \
         LEFT JOIN auth_settings s ON s.org_id = u.org_id \
         WHERE u.account_id = $1::uuid AND u.status = 'active' \
         ORDER BY (u.org_id = $2) DESC NULLS LAST, (u.org_id = a.last_org_id) DESC NULLS LAST, u.org_id",
    )
    .bind(account_id)
    .bind(prefer)
    .fetch_all(&st.postgres)
    .await?)
}

async fn record_failure(st: &AppState, account_id: &str) -> Result<(), ApiError> {
    sqlx::query(
        "UPDATE accounts SET failed_logins = failed_logins + 1, \
         locked_until = CASE WHEN failed_logins + 1 >= $2 THEN now() + make_interval(mins => $3) ELSE locked_until END WHERE id = $1::uuid",
    )
    .bind(account_id)
    .bind(MAX_FAILURES)
    .bind(LOCK_MINUTES)
    .execute(&st.postgres)
    .await?;
    Ok(())
}

/// Signs in to the requested org, else the last one used, else the account's first organization.
#[utoipa::path(post, path = "/api/v1/auth/login", request_body = LoginRequest, responses((status = 200, body = LoginResult), (status = 401, body = crate::error::ErrorBody), (status = 429, body = crate::error::ErrorBody)), tag = "auth")]
pub async fn login(State(st): State<AppState>, ConnectInfo(peer): ConnectInfo<SocketAddr>, headers: HeaderMap, Json(req): Json<LoginRequest>) -> Result<Response, ApiError> {
    let requested = req.org.as_deref().map(str::trim).filter(|o| !o.is_empty());
    let ip = auth::client_ip(&st, &headers, Some(peer));
    let email = req.email.trim().to_lowercase();
    // Failures are audited in the org the person asked for (when it exists), else the install's org.
    let audit_org = match requested {
        Some(o) if sqlx::query_scalar::<_, bool>("SELECT EXISTS (SELECT 1 FROM orgs WHERE id = $1)").bind(o).fetch_one(&st.postgres).await? => o.to_string(),
        _ => st.config.dev_org.clone(),
    };
    let account: Option<LoginAccount> = sqlx::query_as(
        "SELECT id::text, name, password_hash, mfa_secret_enc, \
         CASE WHEN locked_until > now() THEN ceil(extract(epoch FROM locked_until - now()) / 60)::int END AS locked_minutes \
         FROM accounts WHERE email = $1",
    )
    .bind(&email)
    .fetch_optional(&st.postgres)
    .await?;

    let Some(account) = account else {
        password::verify_dummy(&req.password);
        auth::audit(&st.postgres, &audit_org, None, &email, "login.failed", &email, "unknown account", ip.as_deref()).await;
        return Err(ApiError::Unauthorized(GENERIC_FAILURE.into()));
    };
    if let Some(m) = account.locked_minutes {
        auth::audit(&st.postgres, &audit_org, None, &account.name, "login.locked", &email, "", ip.as_deref()).await;
        return Err(ApiError::TooManyRequests(format!("Too many failed attempts. Try again in {m} minute{}.", if m == 1 { "" } else { "s" })));
    }
    let ok = match &account.password_hash {
        Some(h) => password::verify(&req.password, h),
        None => {
            password::verify_dummy(&req.password);
            false
        }
    };
    if !ok {
        record_failure(&st, &account.id).await?;
        auth::audit(&st.postgres, &audit_org, None, &account.name, "login.failed", &email, "wrong password", ip.as_deref()).await;
        return Err(ApiError::Unauthorized(GENERIC_FAILURE.into()));
    }
    let mut candidates = memberships(&st, &account.id, requested).await?;
    if let Some(o) = requested {
        candidates.retain(|m| m.org_id == o);
    }
    if candidates.is_empty() {
        auth::audit(&st.postgres, &audit_org, None, &account.name, "login.failed", &email, "no active membership", ip.as_deref()).await;
        return Err(ApiError::Unauthorized(GENERIC_FAILURE.into()));
    }
    let Some(member) = candidates.into_iter().find(LoginMembership::allows_password) else {
        return Err(ApiError::Forbidden("Password sign-in is disabled for this organization. Use single sign-on.".into()));
    };
    let mfa = account.mfa_secret_enc.is_some();
    let token = auth::create_session(&st, &member.user_id, &member.org_id, "password", mfa, ip.as_deref(), user_agent(&headers).as_deref()).await?;
    if !mfa {
        auth::audit(&st.postgres, &member.org_id, Some(&member.user_id), &account.name, "login.succeeded", &email, "password", ip.as_deref()).await;
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
    #[derive(sqlx::FromRow)]
    struct Pending {
        user_id: String,
        account_id: String,
        org: String,
        name: String,
        secret_enc: Option<String>,
        method: String,
        locked: Option<i32>,
    }
    let pending: Option<Pending> = sqlx::query_as(
        "SELECT u.id::text AS user_id, a.id::text AS account_id, u.org_id AS org, u.name, a.mfa_secret_enc AS secret_enc, s.method, \
         CASE WHEN a.locked_until > now() THEN ceil(extract(epoch FROM a.locked_until - now()) / 60)::int END AS locked \
         FROM sessions s JOIN users u ON u.id = s.user_id JOIN accounts a ON a.id = s.account_id \
         WHERE s.token_hash = $1 AND s.mfa_pending AND s.expires_at > now()",
    )
    .bind(&hash)
    .fetch_optional(&st.postgres)
    .await?;
    let Some(Pending { user_id, account_id, org, name, secret_enc: Some(secret_enc), method, locked }) = pending else {
        return Err(ApiError::Unauthorized("Your sign-in expired. Sign in again.".into()));
    };
    if let Some(m) = locked {
        return Err(ApiError::TooManyRequests(format!("Too many failed attempts. Try again in {m} minutes.")));
    }
    let secret = st.secrets.decrypt(&secret_enc)?;
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs();
    if !totp::verify(&secret, &req.code, now) {
        record_failure(&st, &account_id).await?;
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
    /// The email already has an ObserveCraft password (e.g. from another organization): an invite is
    /// accepted by entering it, not by choosing a new one.
    pub existing_account: bool,
}

#[derive(sqlx::FromRow)]
struct LinkRow {
    user_id: String,
    account_id: String,
    org_id: String,
    email: String,
    name: String,
    org_name: String,
    purpose: String,
    password_hash: Option<String>,
    mfa_enabled: bool,
    /// Active memberships in other organizations.
    other_orgs: i64,
}

async fn find_link(st: &AppState, token: &str) -> Result<LinkRow, ApiError> {
    let row: Option<LinkRow> = sqlx::query_as(
        "SELECT u.id::text AS user_id, a.id::text AS account_id, u.org_id, a.email, u.name, o.name AS org_name, t.purpose, a.password_hash, \
         (a.mfa_secret_enc IS NOT NULL) AS mfa_enabled, \
         (SELECT count(*) FROM users x WHERE x.account_id = a.id AND x.org_id <> u.org_id AND x.status = 'active') AS other_orgs \
         FROM login_tokens t JOIN users u ON u.id = t.user_id JOIN accounts a ON a.id = u.account_id JOIN orgs o ON o.id = t.org_id \
         WHERE t.token_hash = $1 AND t.used_at IS NULL AND t.expires_at > now() AND u.status <> 'deactivated' AND o.status = 'active'",
    )
    .bind(crypto::sha256(token))
    .fetch_optional(&st.postgres)
    .await?;
    row.ok_or_else(|| ApiError::NotFound("This link is invalid, expired or was already used. Ask an admin for a new one.".into()))
}

#[utoipa::path(get, path = "/api/v1/auth/link/{token}", params(("token" = String, Path)), responses((status = 200, body = LinkInfo), (status = 404, body = crate::error::ErrorBody)), tag = "auth")]
pub async fn link_info(State(st): State<AppState>, Path(token): Path<String>) -> ApiResult<LinkInfo> {
    let l = find_link(&st, &token).await?;
    let existing_account = l.purpose == "invite" && l.password_hash.is_some();
    Ok(Json(LinkInfo { email: l.email, name: l.name, org_name: l.org_name, purpose: l.purpose, existing_account }))
}

#[derive(Deserialize, ToSchema)]
pub struct AcceptLinkRequest {
    pub name: Option<String>,
    pub password: String,
}

/// Accepts an invite / reset link and signs in (MFA still applies if enabled). A new account sets its
/// password here; an existing account joins the org by confirming its current password.
#[utoipa::path(post, path = "/api/v1/auth/link/{token}", params(("token" = String, Path)), request_body = AcceptLinkRequest, responses((status = 200, body = LoginResult), (status = 400, body = crate::error::ErrorBody)), tag = "auth")]
pub async fn accept_link(State(st): State<AppState>, ConnectInfo(peer): ConnectInfo<SocketAddr>, headers: HeaderMap, Path(token): Path<String>, Json(req): Json<AcceptLinkRequest>) -> Result<Response, ApiError> {
    let ip = auth::client_ip(&st, &headers, Some(peer));
    let link = find_link(&st, &token).await?;
    let joins_existing = link.purpose == "invite" && link.password_hash.is_some();
    // Links are visible to the issuing org's admins, so a link must never set the password of an account
    // that also belongs to another org (that would hand them access there).
    if !joins_existing && link.other_orgs > 0 {
        return Err(ApiError::BadRequest(
            "This account also belongs to other organizations, so this link can't set its password. Sign in normally, or change the password from Personal Settings → Security.".into(),
        ));
    }
    let new_hash = if joins_existing {
        if !link.password_hash.as_deref().is_some_and(|h| password::verify(&req.password, h)) {
            record_failure(&st, &link.account_id).await?;
            return Err(ApiError::BadRequest("That isn't your current ObserveCraft password.".into()));
        }
        None
    } else {
        password::check_policy(&req.password).map_err(ApiError::BadRequest)?;
        Some(password::hash(&req.password)?)
    };
    let name = req.name.as_deref().map(str::trim).filter(|n| !n.is_empty() && n.chars().count() <= 120).map(str::to_string);
    let mut tx = st.postgres.begin().await?;
    let used = sqlx::query("UPDATE login_tokens SET used_at = now() WHERE token_hash = $1 AND used_at IS NULL").bind(crypto::sha256(&token)).execute(&mut *tx).await?;
    if used.rows_affected() == 0 {
        return Err(ApiError::NotFound("This link was already used.".into()));
    }
    // Other outstanding links for this membership stop working once it is accepted.
    sqlx::query("UPDATE login_tokens SET used_at = now() WHERE user_id = $1::uuid AND used_at IS NULL").bind(&link.user_id).execute(&mut *tx).await?;
    sqlx::query(
        "UPDATE users SET name = COALESCE($2, name), status = 'active', \
         login_methods = CASE WHEN 'password' = ANY(login_methods) THEN login_methods ELSE array_append(login_methods, 'password') END WHERE id = $1::uuid",
    )
    .bind(&link.user_id)
    .bind(&name)
    .execute(&mut *tx)
    .await?;
    if let Some(hash) = &new_hash {
        sqlx::query("UPDATE accounts SET password_hash = $2, name = COALESCE($3, name), failed_logins = 0, locked_until = NULL WHERE id = $1::uuid")
            .bind(&link.account_id)
            .bind(hash)
            .bind(&name)
            .execute(&mut *tx)
            .await?;
        // A password change ends every existing session.
        sqlx::query("DELETE FROM sessions WHERE account_id = $1::uuid").bind(&link.account_id).execute(&mut *tx).await?;
    }
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
/// for the first admin (the instance admin, else `OC_DEV_USER`). Runs at startup.
pub async fn bootstrap_admin(st: &AppState) -> anyhow::Result<()> {
    let org = &st.config.dev_org;
    let (can_sign_in,): (bool,) = sqlx::query_as(
        "SELECT EXISTS (SELECT 1 FROM users u JOIN accounts a ON a.id = u.account_id WHERE u.org_id = $1 AND a.password_hash IS NOT NULL AND u.status = 'active') \
         OR EXISTS (SELECT 1 FROM auth_settings WHERE org_id = $1 AND (saml_enabled OR oidc_enabled))",
    )
    .bind(org)
    .fetch_one(&st.postgres)
    .await?;
    if can_sign_in {
        return Ok(());
    }
    let admin: Option<(String, String)> = sqlx::query_as(
        "SELECT u.id::text, a.email FROM users u JOIN accounts a ON a.id = u.account_id WHERE u.org_id = $1 \
         ORDER BY a.is_instance_admin DESC, (u.id::text = $2) DESC LIMIT 1",
    )
    .bind(org)
    .bind(&st.config.dev_user)
    .fetch_optional(&st.postgres)
    .await?;
    let Some((user_id, email)) = admin else {
        tracing::warn!("no bootstrap admin found; nobody can sign in. Set OC_INSTANCE_ADMIN_EMAIL.");
        return Ok(());
    };
    let url = issue_link(st, org, &user_id, "bootstrap", None, 24).await.map_err(|e| anyhow::anyhow!("{e}"))?;
    tracing::warn!("No one can sign in yet. Set the admin password for {email} (valid 24h, single use): {url}");
    Ok(())
}

/// Grants instance administration to `OC_INSTANCE_ADMIN_EMAIL` (created as an Admin of the install's
/// org if new). Without it, the first admin (`OC_DEV_USER`) is the instance admin until one is set.
pub async fn ensure_instance_admin(st: &AppState) -> anyhow::Result<()> {
    let org = &st.config.dev_org;
    match &st.config.instance_admin_email {
        Some(email) => {
            let name = email.split('@').next().unwrap_or(email).replace(['.', '_'], " ");
            let mut tx = st.postgres.begin().await?;
            sqlx::query("INSERT INTO users (id, org_id, email, name, status) VALUES (gen_random_uuid(), $1, $2, $3, 'active') ON CONFLICT (org_id, email) DO NOTHING")
                .bind(org)
                .bind(email)
                .bind(&name)
                .execute(&mut *tx)
                .await?;
            sqlx::query("INSERT INTO user_roles (user_id, role_id) SELECT id, $1 || '-admin' FROM users WHERE org_id = $1 AND lower(email) = $2 ON CONFLICT DO NOTHING")
                .bind(org)
                .bind(email)
                .execute(&mut *tx)
                .await?;
            sqlx::query("UPDATE accounts SET is_instance_admin = (email = $1)").bind(email).execute(&mut *tx).await?;
            tx.commit().await?;
        }
        None => {
            sqlx::query("UPDATE accounts SET is_instance_admin = true WHERE id = (SELECT account_id FROM users WHERE id::text = $1) AND NOT EXISTS (SELECT 1 FROM accounts WHERE is_instance_admin)")
                .bind(&st.config.dev_user)
                .execute(&st.postgres)
                .await?;
        }
    }
    Ok(())
}

// ================================================================ self-service security

fn require_session(ctx: &AuthContext) -> Result<(), ApiError> {
    if ctx.credential != Credential::Session {
        return Err(ApiError::Forbidden("this action needs a signed-in browser session, not an application key".into()));
    }
    Ok(())
}

/// Ends every session of the current account (in every organization), including this one.
#[utoipa::path(post, path = "/api/v1/me/logout-all", responses((status = 204)), tag = "auth")]
pub async fn logout_all(State(st): State<AppState>, ctx: AuthContext) -> Result<Response, ApiError> {
    let n = sqlx::query("DELETE FROM sessions WHERE account_id = $1::uuid").bind(&ctx.account_id).execute(&st.postgres).await?.rows_affected();
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
    let (current,): (Option<String>,) = sqlx::query_as("SELECT password_hash FROM accounts WHERE id = $1::uuid").bind(&ctx.account_id).fetch_one(&st.postgres).await?;
    if let Some(h) = current {
        if !req.current_password.as_deref().is_some_and(|p| password::verify(p, &h)) {
            return Err(ApiError::BadRequest("Current password is incorrect".into()));
        }
    }
    password::check_policy(&req.new_password).map_err(ApiError::BadRequest)?;
    let hash = password::hash(&req.new_password)?;
    sqlx::query("UPDATE accounts SET password_hash = $2 WHERE id = $1::uuid").bind(&ctx.account_id).bind(&hash).execute(&st.postgres).await?;
    sqlx::query("UPDATE users SET login_methods = CASE WHEN 'password' = ANY(login_methods) THEN login_methods ELSE array_append(login_methods, 'password') END WHERE id = $1::uuid")
        .bind(&ctx.user_id)
        .execute(&st.postgres)
        .await?;
    sqlx::query("DELETE FROM sessions WHERE account_id = $1::uuid AND token_hash <> $2").bind(&ctx.account_id).bind(ctx.session_hash.clone().unwrap_or_default()).execute(&st.postgres).await?;
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
    /// Organization the session is acting in.
    pub org_id: String,
}

#[utoipa::path(get, path = "/api/v1/me/sessions", responses((status = 200, body = Vec<SessionInfo>)), tag = "auth")]
pub async fn sessions(State(st): State<AppState>, ctx: AuthContext) -> ApiResult<Vec<SessionInfo>> {
    let rows = sqlx::query_as(
        "SELECT encode(token_hash, 'hex') AS id, method, ip, user_agent, (extract(epoch FROM created_at) * 1000)::bigint AS created_at, \
         (extract(epoch FROM last_seen_at) * 1000)::bigint AS last_seen_at, token_hash = $2 AS current, org_id \
         FROM sessions WHERE account_id = $1::uuid AND NOT mfa_pending AND expires_at > now() ORDER BY last_seen_at DESC",
    )
    .bind(&ctx.account_id)
    .bind(ctx.session_hash.clone().unwrap_or_default())
    .fetch_all(&st.postgres)
    .await?;
    Ok(Json(rows))
}

#[utoipa::path(delete, path = "/api/v1/me/sessions/{id}", params(("id" = String, Path)), responses((status = 204), (status = 404, body = crate::error::ErrorBody)), tag = "auth")]
pub async fn revoke_session(State(st): State<AppState>, ctx: AuthContext, Path(id): Path<String>) -> Result<StatusCode, ApiError> {
    let n = sqlx::query("DELETE FROM sessions WHERE account_id = $1::uuid AND encode(token_hash, 'hex') = $2").bind(&ctx.account_id).bind(&id).execute(&st.postgres).await?.rows_affected();
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
    let (email, enabled): (String, bool) = sqlx::query_as("SELECT email, mfa_secret_enc IS NOT NULL FROM accounts WHERE id = $1::uuid").bind(&ctx.account_id).fetch_one(&st.postgres).await?;
    if enabled {
        return Err(ApiError::BadRequest("MFA is already enabled. Disable it first to enroll a new device.".into()));
    }
    let secret = totp::new_secret();
    sqlx::query("UPDATE accounts SET mfa_pending_enc = $2 WHERE id = $1::uuid").bind(&ctx.account_id).bind(st.secrets.encrypt(&secret)).execute(&st.postgres).await?;
    Ok(Json(MfaSetup { otpauth_url: totp::otpauth_url(&secret, &email), secret }))
}

fn now_unix() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs()
}

#[utoipa::path(post, path = "/api/v1/me/mfa/enable", request_body = CodeRequest, responses((status = 204), (status = 400, body = crate::error::ErrorBody)), tag = "auth")]
pub async fn mfa_enable(State(st): State<AppState>, ctx: AuthContext, Json(req): Json<CodeRequest>) -> Result<StatusCode, ApiError> {
    require_session(&ctx)?;
    let (pending,): (Option<String>,) = sqlx::query_as("SELECT mfa_pending_enc FROM accounts WHERE id = $1::uuid").bind(&ctx.account_id).fetch_one(&st.postgres).await?;
    let pending = pending.ok_or_else(|| ApiError::BadRequest("Start MFA setup first".into()))?;
    if !totp::verify(&st.secrets.decrypt(&pending)?, &req.code, now_unix()) {
        return Err(ApiError::BadRequest("That code doesn't match. Check the time on your device and try again.".into()));
    }
    sqlx::query("UPDATE accounts SET mfa_secret_enc = mfa_pending_enc, mfa_pending_enc = NULL WHERE id = $1::uuid").bind(&ctx.account_id).execute(&st.postgres).await?;
    sqlx::query("UPDATE users SET mfa_enabled = true WHERE account_id = $1::uuid").bind(&ctx.account_id).execute(&st.postgres).await?;
    ctx.audit(&st, "user.mfa_enabled", "", "").await;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(post, path = "/api/v1/me/mfa/disable", request_body = CodeRequest, responses((status = 204), (status = 400, body = crate::error::ErrorBody)), tag = "auth")]
pub async fn mfa_disable(State(st): State<AppState>, ctx: AuthContext, Json(req): Json<CodeRequest>) -> Result<StatusCode, ApiError> {
    require_session(&ctx)?;
    let (secret,): (Option<String>,) = sqlx::query_as("SELECT mfa_secret_enc FROM accounts WHERE id = $1::uuid").bind(&ctx.account_id).fetch_one(&st.postgres).await?;
    let secret = secret.ok_or_else(|| ApiError::BadRequest("MFA is not enabled".into()))?;
    if !totp::verify(&st.secrets.decrypt(&secret)?, &req.code, now_unix()) {
        return Err(ApiError::BadRequest("Invalid code".into()));
    }
    sqlx::query("UPDATE accounts SET mfa_secret_enc = NULL, mfa_pending_enc = NULL WHERE id = $1::uuid").bind(&ctx.account_id).execute(&st.postgres).await?;
    // Memberships signed in through an IdP keep the IdP's MFA flag.
    sqlx::query("UPDATE users SET mfa_enabled = false WHERE account_id = $1::uuid AND 'password' = ANY(login_methods)").bind(&ctx.account_id).execute(&st.postgres).await?;
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
        sqlx::query_as("SELECT password_hash IS NOT NULL, mfa_secret_enc IS NOT NULL FROM accounts WHERE id = $1::uuid").bind(&ctx.account_id).fetch_one(&st.postgres).await?;
    Ok(Json(SecurityStatus { has_password, mfa_enabled, access: ctx.access }))
}

// ---------------------------------------------------------------- organizations of the account

#[derive(Serialize, ToSchema, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct MyOrg {
    pub id: String,
    pub name: String,
    pub current: bool,
    pub is_admin: bool,
}

/// The organizations this account belongs to (active memberships in active orgs).
#[utoipa::path(get, path = "/api/v1/me/orgs", responses((status = 200, body = Vec<MyOrg>)), tag = "auth")]
pub async fn my_orgs(State(st): State<AppState>, ctx: AuthContext) -> ApiResult<Vec<MyOrg>> {
    let rows = sqlx::query_as(
        "SELECT o.id, o.name, (o.id = $2) AS current, \
         EXISTS (SELECT 1 FROM user_roles r WHERE r.user_id = u.id AND r.role_id = u.org_id || '-admin') AS is_admin \
         FROM users u JOIN orgs o ON o.id = u.org_id WHERE u.account_id = $1::uuid AND u.status = 'active' AND o.status = 'active' ORDER BY lower(o.name)",
    )
    .bind(&ctx.account_id)
    .bind(&ctx.org_id)
    .fetch_all(&st.postgres)
    .await?;
    Ok(Json(rows))
}

#[derive(Deserialize, ToSchema)]
pub struct SwitchOrgRequest {
    pub org: String,
}

/// Moves this browser session to another of the account's organizations (a new session token is issued).
/// Sessions started with single sign-on are bound to that organization; an org that requires SSO can
/// only be entered through its identity provider (admins excepted).
#[utoipa::path(post, path = "/api/v1/me/switch-org", request_body = SwitchOrgRequest, responses((status = 204), (status = 403, body = crate::error::ErrorBody)), tag = "auth")]
pub async fn switch_org(State(st): State<AppState>, ctx: AuthContext, Json(req): Json<SwitchOrgRequest>) -> Result<Response, ApiError> {
    require_session(&ctx)?;
    if req.org == ctx.org_id {
        return Ok(StatusCode::NO_CONTENT.into_response());
    }
    let old = ctx.session_hash.clone().unwrap_or_default();
    let (method,): (String,) = sqlx::query_as("SELECT method FROM sessions WHERE token_hash = $1").bind(&old).fetch_one(&st.postgres).await?;
    if method == "saml" || method == "oidc" {
        return Err(ApiError::Forbidden("You signed in with single sign-on for this organization. Sign out and sign in again to use another one.".into()));
    }
    let target = memberships(&st, &ctx.account_id, Some(&req.org)).await?.into_iter().find(|m| m.org_id == req.org);
    let Some(target) = target else { return Err(ApiError::Forbidden("You're not a member of that organization".into())) };
    if !target.allows_password() {
        return Err(ApiError::Forbidden("That organization requires single sign-on. Sign out and sign in with its identity provider.".into()));
    }
    // Rotate the token in place: the session keeps its start time (and so its absolute expiry).
    let token = crypto::random_token("ocs_");
    let moved = sqlx::query("UPDATE sessions SET token_hash = $2, user_id = $3::uuid, org_id = $4, last_seen_at = now() WHERE token_hash = $1 AND account_id = $5::uuid")
        .bind(&old)
        .bind(crypto::sha256(&token))
        .bind(&target.user_id)
        .bind(&target.org_id)
        .bind(&ctx.account_id)
        .execute(&st.postgres)
        .await?;
    if moved.rows_affected() == 0 {
        return Err(ApiError::Unauthorized("Your session expired. Sign in again.".into()));
    }
    sqlx::query("UPDATE accounts SET last_org_id = $2 WHERE id = $1::uuid").bind(&ctx.account_id).bind(&target.org_id).execute(&st.postgres).await?;
    sqlx::query("UPDATE users SET last_login_at = now() WHERE id = $1::uuid").bind(&target.user_id).execute(&st.postgres).await?;
    ctx.audit(&st, "session.switched_org", &target.org_id, "").await;
    auth::audit(&st.postgres, &target.org_id, Some(&target.user_id), &ctx.name, "login.succeeded", "", &format!("switched from {}", ctx.org_id), ctx.ip.as_deref()).await;
    Ok((StatusCode::NO_CONTENT, [(SET_COOKIE, auth::session_cookie(&st, &token))]).into_response())
}
