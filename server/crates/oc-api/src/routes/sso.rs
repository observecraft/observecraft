//! Single sign-on: SAML 2.0 (SP-initiated, HTTP-Redirect → HTTP-POST) and OpenID Connect
//! (authorization code + PKCE), configured per org on the Login Methods page.
//!
//! SAML hardening: the IdP metadata must contain a signing certificate (an unsigned response is
//! never accepted), only SHA-2 signature algorithms are allowed, the response must answer a request
//! we issued (InResponseTo, single use), and assertion ids are remembered until they expire (replay).
//!
//! Both protocols end in [`complete_login`]: find the user by IdP subject or email, create them on first
//! login when JIT provisioning is on, apply group mappings (team memberships; role mappings replace roles),
//! then start a session. The IdP is responsible for MFA on SSO logins.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::str::FromStr;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use axum::extract::{ConnectInfo, Form, Path, Query, State};
use axum::http::header::{CONTENT_TYPE, LOCATION, SET_COOKIE, USER_AGENT};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use base64::Engine;
use openidconnect::core::{CoreAuthenticationFlow, CoreClient, CoreProviderMetadata};
use openidconnect::{AuthorizationCode, ClientId, ClientSecret, CsrfToken, IssuerUrl, Nonce, PkceCodeChallenge, PkceCodeVerifier, RedirectUrl, Scope, TokenResponse};
use samael::crypto::AllowedSignatureAlgorithm;
use samael::metadata::{EntityDescriptor, HTTP_REDIRECT_BINDING};
use samael::service_provider::{ServiceProvider, ServiceProviderBuilder};
use samael::traits::ToXml;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use crate::auth::{self, Admin};
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;

/// Entra's group claim URI in SAML; OIDC group claims are matched against it too.
const MS_GROUPS_URI: &str = "http://schemas.microsoft.com/ws/2008/06/identity/claims/groups";

// ================================================================ settings

#[derive(sqlx::FromRow, Clone)]
struct Settings {
    password_enabled: bool,
    sso_required: bool,
    jit_provisioning: bool,
    default_role_id: Option<String>,
    saml_enabled: bool,
    saml_idp_metadata: Option<String>,
    saml_email_attr: String,
    saml_name_attr: String,
    saml_groups_attr: String,
    oidc_enabled: bool,
    oidc_issuer: Option<String>,
    oidc_client_id: Option<String>,
    oidc_client_secret_enc: Option<String>,
    oidc_groups_claim: String,
    oidc_button_label: String,
}

async fn settings(st: &AppState, org: &str) -> Result<Settings, ApiError> {
    sqlx::query_as("SELECT password_enabled, sso_required, jit_provisioning, default_role_id, saml_enabled, saml_idp_metadata, saml_email_attr, saml_name_attr, saml_groups_attr, oidc_enabled, oidc_issuer, oidc_client_id, oidc_client_secret_enc, oidc_groups_claim, oidc_button_label FROM auth_settings WHERE org_id = $1")
        .bind(org)
        .fetch_optional(&st.postgres)
        .await?
        .ok_or_else(|| ApiError::NotFound("Unknown organization".into()))
}

fn enc(s: &str) -> String {
    url::form_urlencoded::byte_serialize(s.as_bytes()).collect()
}

fn sp_urls(st: &AppState, org: &str) -> (String, String) {
    let base = format!("{}/api/v1/auth/saml/{}", st.config.public_url, enc(org));
    (format!("{base}/metadata"), format!("{base}/acs"))
}

fn oidc_redirect(st: &AppState, org: &str) -> String {
    format!("{}/api/v1/auth/oidc/{}/callback", st.config.public_url, enc(org))
}

/// Parses IdP metadata and builds our service provider. Refuses metadata without a signing certificate.
fn service_provider(st: &AppState, org: &str, metadata_xml: &str) -> Result<ServiceProvider, String> {
    let idp = EntityDescriptor::from_str(metadata_xml).map_err(|e| format!("IdP metadata is not valid SAML metadata: {e}"))?;
    let (entity_id, acs) = sp_urls(st, org);
    let sp = ServiceProviderBuilder::default()
        .entity_id(entity_id.clone())
        .metadata_url(entity_id)
        .acs_url(acs)
        .idp_metadata(idp)
        .allow_idp_initiated(false)
        .allowed_signature_algorithms(Some(vec![
            AllowedSignatureAlgorithm::RsaSha256,
            AllowedSignatureAlgorithm::RsaSha384,
            AllowedSignatureAlgorithm::RsaSha512,
            AllowedSignatureAlgorithm::EcdsaSha256,
            AllowedSignatureAlgorithm::EcdsaSha384,
            AllowedSignatureAlgorithm::EcdsaSha512,
        ]))
        .build()
        .map_err(|e| format!("SAML configuration error: {e}"))?;
    match sp.idp_signing_certs() {
        Ok(Some(certs)) if !certs.is_empty() => {}
        _ => return Err("IdP metadata has no signing certificate; ObserveCraft only accepts signed SAML responses.".into()),
    }
    if sp.sso_binding_location(HTTP_REDIRECT_BINDING).is_none() {
        return Err("IdP metadata has no HTTP-Redirect SingleSignOnService endpoint.".into());
    }
    Ok(sp)
}

/// SSO failures go back to the sign-in page with a message (these are browser navigations, not API calls).
fn fail(st: &AppState, msg: &str) -> Response {
    tracing::warn!(reason = msg, "sso login failed");
    redirect(&format!("{}/login?error={}", st.config.public_url, enc(msg)))
}

fn redirect(to: &str) -> Response {
    (StatusCode::SEE_OTHER, [(LOCATION, to.to_string())]).into_response()
}

/// Only same-app paths may be used as the post-login destination (no open redirects).
fn safe_redirect(r: Option<&str>) -> String {
    match r {
        Some(p) if p.starts_with('/') && !p.starts_with("//") && !p.starts_with("/\\") && !p.starts_with("/api/") && p.len() < 500 => p.to_string(),
        _ => "/".into(),
    }
}

#[derive(Deserialize, IntoParams)]
pub struct LoginParams {
    /// App path to return to after sign-in.
    pub redirect: Option<String>,
}

// ================================================================ SSO discovery

#[derive(Deserialize, ToSchema)]
pub struct DiscoverRequest {
    pub email: String,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct DiscoverResult {
    /// Where to send the browser; absent when no organization offers SSO.
    pub login_url: Option<String>,
}

/// "Sign in with your organization": picks the SSO login for an email address. The answer doesn't
/// depend on whether the account exists (the install's org is used when the email is unknown), so it
/// can't be used to probe for users.
#[utoipa::path(post, path = "/api/v1/auth/sso/discover", request_body = DiscoverRequest, responses((status = 200, body = DiscoverResult)), tag = "auth")]
pub async fn discover_sso(State(st): State<AppState>, Json(req): Json<DiscoverRequest>) -> ApiResult<DiscoverResult> {
    let email = req.email.trim().to_lowercase();
    let org: Option<(String,)> = sqlx::query_as("SELECT org_id FROM users WHERE lower(email) = $1 AND status <> 'deactivated' ORDER BY created_at LIMIT 1").bind(&email).fetch_optional(&st.postgres).await?;
    let org = org.map(|o| o.0).unwrap_or_else(|| st.config.dev_org.clone());
    let s = settings(&st, &org).await?;
    let base = format!("/api/v1/auth/{{}}/{}/login", enc(&org));
    let login_url = if s.saml_enabled { Some(base.replace("{}", "saml")) } else if s.oidc_enabled { Some(base.replace("{}", "oidc")) } else { None };
    Ok(Json(DiscoverResult { login_url }))
}

// ================================================================ SAML

#[utoipa::path(get, path = "/api/v1/auth/saml/{org}/metadata", params(("org" = String, Path)), responses((status = 200, description = "SP metadata XML", content_type = "application/samlmetadata+xml")), tag = "auth")]
pub async fn saml_metadata(State(st): State<AppState>, Path(org): Path<String>) -> Result<Response, ApiError> {
    let s = settings(&st, &org).await?;
    let (entity_id, acs) = sp_urls(&st, &org);
    // Before the IdP is configured we can still publish our side (entity id + ACS) for the IdP admin.
    let sp = match s.saml_idp_metadata.as_deref().map(|m| service_provider(&st, &org, m)) {
        Some(Ok(sp)) => sp,
        _ => ServiceProviderBuilder::default().entity_id(entity_id.clone()).metadata_url(entity_id).acs_url(acs).build().map_err(|e| anyhow::anyhow!("{e}"))?,
    };
    let xml = sp.metadata().map_err(|e| anyhow::anyhow!("{e}"))?.to_string().map_err(|e| anyhow::anyhow!("{e}"))?;
    Ok(([(CONTENT_TYPE, "application/samlmetadata+xml")], xml).into_response())
}

#[utoipa::path(get, path = "/api/v1/auth/saml/{org}/login", params(("org" = String, Path), LoginParams), responses((status = 303, description = "Redirect to the IdP")), tag = "auth")]
pub async fn saml_login(State(st): State<AppState>, Path(org): Path<String>, Query(p): Query<LoginParams>) -> Response {
    let s = match settings(&st, &org).await {
        Ok(s) => s,
        Err(_) => return fail(&st, "Unknown organization"),
    };
    let Some(meta) = s.saml_idp_metadata.filter(|_| s.saml_enabled) else { return fail(&st, "SAML sign-in is not enabled") };
    let sp = match service_provider(&st, &org, &meta) {
        Ok(sp) => sp,
        Err(e) => return fail(&st, &e),
    };
    let idp_url = sp.sso_binding_location(HTTP_REDIRECT_BINDING).unwrap_or_default();
    let req = match sp.make_authentication_request(&idp_url) {
        Ok(r) => r,
        Err(e) => return fail(&st, &format!("Could not build SAML request: {e}")),
    };
    // RelayState is our own random handle for this attempt; it maps back to the request id + destination.
    let relay = auth::crypto::random_token("");
    if let Err(e) = sqlx::query("INSERT INTO sso_requests (id, org_id, protocol, nonce, redirect_to) VALUES ($1, $2, 'saml', $3, $4)")
        .bind(&relay)
        .bind(&org)
        .bind(&req.id)
        .bind(safe_redirect(p.redirect.as_deref()))
        .execute(&st.postgres)
        .await
    {
        tracing::error!(error = %e, "store saml request");
        return fail(&st, "Sign-in is temporarily unavailable");
    }
    match req.redirect(&relay) {
        Ok(Some(url)) => redirect(url.as_str()),
        _ => fail(&st, "Could not build the IdP redirect"),
    }
}

/// HTTP-POST binding form fields.
#[derive(Deserialize, ToSchema)]
pub struct AcsForm {
    #[serde(rename = "SAMLResponse")]
    saml_response: String,
    #[serde(rename = "RelayState")]
    relay_state: Option<String>,
}

/// Assertion Consumer Service: the IdP POSTs the signed response here.
#[utoipa::path(post, path = "/api/v1/auth/saml/{org}/acs", params(("org" = String, Path)), responses((status = 303, description = "Signed in; redirect to the app")), tag = "auth")]
pub async fn saml_acs(State(st): State<AppState>, ConnectInfo(peer): ConnectInfo<SocketAddr>, headers: HeaderMap, Path(org): Path<String>, Form(form): Form<AcsForm>) -> Response {
    let ip = auth::client_ip(&st, &headers, Some(peer));
    let Some(relay) = form.relay_state else { return fail(&st, "IdP-initiated sign-in is not supported. Start from the ObserveCraft sign-in page.") };
    let req: Option<(String, String)> = sqlx::query_as("DELETE FROM sso_requests WHERE id = $1 AND org_id = $2 AND protocol = 'saml' AND created_at > now() - interval '10 minutes' RETURNING nonce, redirect_to")
        .bind(&relay)
        .bind(&org)
        .fetch_optional(&st.postgres)
        .await
        .unwrap_or(None);
    let Some((request_id, redirect_to)) = req else { return fail(&st, "This sign-in attempt expired. Please try again.") };
    let s = match settings(&st, &org).await {
        Ok(s) => s,
        Err(_) => return fail(&st, "Unknown organization"),
    };
    let Some(meta) = s.saml_idp_metadata.clone().filter(|_| s.saml_enabled) else { return fail(&st, "SAML sign-in is not enabled") };
    let sp = match service_provider(&st, &org, &meta) {
        Ok(sp) => sp,
        Err(e) => return fail(&st, &e),
    };
    // samael reports every signature failure the same way; name the common misconfiguration explicitly.
    let sha1 = base64::engine::general_purpose::STANDARD
        .decode(form.saml_response.trim())
        .ok()
        .and_then(|b| String::from_utf8(b).ok())
        .is_some_and(|x| x.contains("xmldsig#rsa-sha1\"") || x.contains("xmldsig#sha1\""));
    if sha1 {
        auth::audit(&st.postgres, &org, None, "saml", "login.failed", "", "IdP signed the response with SHA-1, which is not accepted", ip.as_deref()).await;
        return fail(&st, "Your identity provider signs with SHA-1. Configure it to use SHA-256 (Entra: Signing Algorithm = SHA-256).");
    }
    // Errors are turned into text before any await: samael's boxed errors aren't Send.
    let parsed = sp.parse_base64_response(form.saml_response.trim(), Some(&[request_id.as_str()])).map_err(|e| e.to_string());
    drop(sp);
    let assertion = match parsed {
        Ok(a) => a,
        Err(e) => {
            auth::audit(&st.postgres, &org, None, "saml", "login.failed", "", &format!("SAML response rejected: {e}"), ip.as_deref()).await;
            return fail(&st, "The identity provider's response could not be verified.");
        }
    };
    // Replay protection: each assertion id is accepted once.
    let expires = assertion.conditions.as_ref().and_then(|c| c.not_on_or_after).map(|t| t.timestamp()).unwrap_or_else(|| chrono_now() + 3600);
    let fresh = sqlx::query("INSERT INTO saml_seen_assertions (assertion_id, expires_at) VALUES ($1, to_timestamp($2)) ON CONFLICT DO NOTHING")
        .bind(&assertion.id)
        .bind(expires as f64)
        .execute(&st.postgres)
        .await
        .map(|r| r.rows_affected() == 1)
        .unwrap_or(false);
    if !fresh {
        return fail(&st, "This sign-in response was already used.");
    }
    let subject = assertion.subject.as_ref().and_then(|s| s.name_id.as_ref()).map(|n| n.value.trim().to_string()).unwrap_or_default();
    let mut attrs: HashMap<String, Vec<String>> = HashMap::new();
    for stmt in assertion.attribute_statements.iter().flatten() {
        for a in &stmt.attributes {
            let values: Vec<String> = a.values.iter().filter_map(|v| v.value.clone()).collect();
            for key in [a.name.clone(), a.friendly_name.clone()].into_iter().flatten() {
                attrs.entry(key).or_default().extend(values.iter().cloned());
            }
        }
    }
    let first = |k: &str| attrs.get(k).and_then(|v| v.first()).cloned();
    let email = first(&s.saml_email_attr).or_else(|| first("email")).or_else(|| subject.contains('@').then(|| subject.clone()));
    let name = first(&s.saml_name_attr).or_else(|| first("displayName")).or_else(|| first("name"));
    let groups = attrs.get(&s.saml_groups_attr).cloned().unwrap_or_default();
    // Mapping keys are matched against attribute names (the configured groups attribute included).
    let mut claims = attrs.clone();
    claims.insert(s.saml_groups_attr.clone(), groups);
    complete_login(&st, &s, &org, "saml", &subject, email, name, &claims, &redirect_to, ip, headers.get(USER_AGENT).and_then(|v| v.to_str().ok())).await
}

fn chrono_now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs() as i64
}

// ================================================================ OIDC

/// Sent on every request to the identity provider: reqwest sends no User-Agent by default, and providers behind a
/// WAF (or CDN bot rules) answer 403 to requests without one.
const OIDC_USER_AGENT: &str = concat!("ObserveCraft-SSO/", env!("CARGO_PKG_VERSION"), " (+https://github.com/observecraft/observecraft)");

fn http_client() -> Result<openidconnect::reqwest::Client, ApiError> {
    // No redirects: following them during discovery/token exchange enables SSRF.
    openidconnect::reqwest::ClientBuilder::new()
        .user_agent(OIDC_USER_AGENT)
        .redirect(openidconnect::reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("http client: {e}")))
}

/// Discovery documents are cached for 10 minutes per issuer.
async fn discover(issuer: &str) -> Result<CoreProviderMetadata, String> {
    static CACHE: OnceLock<Mutex<HashMap<String, (CoreProviderMetadata, Instant)>>> = OnceLock::new();
    let cache = CACHE.get_or_init(Default::default);
    if let Some((m, at)) = cache.lock().unwrap().get(issuer) {
        if at.elapsed() < Duration::from_secs(600) {
            return Ok(m.clone());
        }
    }
    let url = IssuerUrl::new(issuer.to_string()).map_err(|e| format!("invalid issuer URL: {e}"))?;
    let client = http_client().map_err(|e| e.to_string())?;
    let m = CoreProviderMetadata::discover_async(url, &client).await.map_err(|e| format!("OIDC discovery failed for {issuer}: {e}"))?;
    cache.lock().unwrap().insert(issuer.to_string(), (m.clone(), Instant::now()));
    Ok(m)
}

#[utoipa::path(get, path = "/api/v1/auth/oidc/{org}/login", params(("org" = String, Path), LoginParams), responses((status = 303, description = "Redirect to the IdP")), tag = "auth")]
pub async fn oidc_login(State(st): State<AppState>, Path(org): Path<String>, Query(p): Query<LoginParams>) -> Response {
    let s = match settings(&st, &org).await {
        Ok(s) => s,
        Err(_) => return fail(&st, "Unknown organization"),
    };
    let (Some(issuer), Some(client_id), true) = (s.oidc_issuer.clone(), s.oidc_client_id.clone(), s.oidc_enabled) else { return fail(&st, "OIDC sign-in is not enabled") };
    let meta = match discover(&issuer).await {
        Ok(m) => m,
        Err(e) => return fail(&st, &e),
    };
    let secret = s.oidc_client_secret_enc.as_deref().and_then(|e| st.secrets.decrypt(e).ok()).map(ClientSecret::new);
    let Ok(redirect_url) = RedirectUrl::new(oidc_redirect(&st, &org)) else { return fail(&st, "Invalid redirect URL (check OC_PUBLIC_URL)") };
    let client = CoreClient::from_provider_metadata(meta, ClientId::new(client_id), secret).set_redirect_uri(redirect_url);
    let (challenge, verifier) = PkceCodeChallenge::new_random_sha256();
    let (url, state, nonce) = client
        .authorize_url(CoreAuthenticationFlow::AuthorizationCode, CsrfToken::new_random, Nonce::new_random)
        .add_scope(Scope::new("email".into()))
        .add_scope(Scope::new("profile".into()))
        .set_pkce_challenge(challenge)
        .url();
    if let Err(e) = sqlx::query("INSERT INTO sso_requests (id, org_id, protocol, nonce, pkce_verifier, redirect_to) VALUES ($1, $2, 'oidc', $3, $4, $5)")
        .bind(state.secret())
        .bind(&org)
        .bind(nonce.secret())
        .bind(verifier.secret())
        .bind(safe_redirect(p.redirect.as_deref()))
        .execute(&st.postgres)
        .await
    {
        tracing::error!(error = %e, "store oidc request");
        return fail(&st, "Sign-in is temporarily unavailable");
    }
    redirect(url.as_str())
}

#[derive(Deserialize)]
pub struct CallbackParams {
    code: Option<String>,
    state: Option<String>,
    error: Option<String>,
    error_description: Option<String>,
}

#[utoipa::path(get, path = "/api/v1/auth/oidc/{org}/callback", params(("org" = String, Path)), responses((status = 303, description = "Signed in; redirect to the app")), tag = "auth")]
pub async fn oidc_callback(State(st): State<AppState>, ConnectInfo(peer): ConnectInfo<SocketAddr>, headers: HeaderMap, Path(org): Path<String>, Query(p): Query<CallbackParams>) -> Response {
    let ip = auth::client_ip(&st, &headers, Some(peer));
    if let Some(e) = p.error {
        return fail(&st, &format!("The identity provider refused the sign-in: {}", p.error_description.unwrap_or(e)));
    }
    let (Some(code), Some(state)) = (p.code, p.state) else { return fail(&st, "Missing authorization code") };
    let req: Option<(String, String, String)> = sqlx::query_as("DELETE FROM sso_requests WHERE id = $1 AND org_id = $2 AND protocol = 'oidc' AND created_at > now() - interval '10 minutes' RETURNING nonce, pkce_verifier, redirect_to")
        .bind(&state)
        .bind(&org)
        .fetch_optional(&st.postgres)
        .await
        .unwrap_or(None);
    let Some((nonce, verifier, redirect_to)) = req else { return fail(&st, "This sign-in attempt expired. Please try again.") };
    let s = match settings(&st, &org).await {
        Ok(s) => s,
        Err(_) => return fail(&st, "Unknown organization"),
    };
    let (Some(issuer), Some(client_id), true) = (s.oidc_issuer.clone(), s.oidc_client_id.clone(), s.oidc_enabled) else { return fail(&st, "OIDC sign-in is not enabled") };
    let meta = match discover(&issuer).await {
        Ok(m) => m,
        Err(e) => return fail(&st, &e),
    };
    let secret = s.oidc_client_secret_enc.as_deref().and_then(|e| st.secrets.decrypt(e).ok()).map(ClientSecret::new);
    let Ok(redirect_url) = RedirectUrl::new(oidc_redirect(&st, &org)) else { return fail(&st, "Invalid redirect URL") };
    let client = CoreClient::from_provider_metadata(meta, ClientId::new(client_id), secret).set_redirect_uri(redirect_url);
    let Ok(http) = http_client() else { return fail(&st, "Sign-in is temporarily unavailable") };
    let exchange = match client.exchange_code(AuthorizationCode::new(code)) {
        Ok(x) => x,
        Err(e) => return fail(&st, &format!("Provider has no token endpoint: {e}")),
    };
    let token = match exchange.set_pkce_verifier(PkceCodeVerifier::new(verifier)).request_async(&http).await {
        Ok(t) => t,
        Err(e) => {
            auth::audit(&st.postgres, &org, None, "oidc", "login.failed", "", &format!("token exchange failed: {e}"), ip.as_deref()).await;
            return fail(&st, "Could not complete sign-in with the identity provider.");
        }
    };
    let Some(id_token) = token.id_token() else { return fail(&st, "The identity provider did not return an ID token") };
    let verifier = client.id_token_verifier();
    let claims = match id_token.claims(&verifier, &Nonce::new(nonce)) {
        Ok(c) => c,
        Err(e) => {
            auth::audit(&st.postgres, &org, None, "oidc", "login.failed", "", &format!("ID token rejected: {e}"), ip.as_deref()).await;
            return fail(&st, "The identity provider's token could not be verified.");
        }
    };
    let subject = claims.subject().as_str().to_string();
    let email = claims.email().map(|e| e.as_str().to_string());
    let name = claims.name().and_then(|n| n.get(None)).map(|n| n.as_str().to_string());
    // Group claims aren't part of the standard claim set: read them from the (already verified) token payload.
    let raw: HashMap<String, serde_json::Value> = id_token
        .to_string()
        .split('.')
        .nth(1)
        .and_then(|p| base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(p).ok())
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default();
    let mut attrs: HashMap<String, Vec<String>> = HashMap::new();
    for (k, v) in raw {
        let vals = match v {
            serde_json::Value::Array(a) => a.into_iter().filter_map(|x| x.as_str().map(str::to_string)).collect(),
            serde_json::Value::String(x) => vec![x],
            _ => continue,
        };
        attrs.insert(k, vals);
    }
    if attrs.contains_key("_claim_names") {
        tracing::warn!("OIDC token has a groups overage claim; configure the app to emit groups assigned to the application");
    }
    let groups = attrs.get(&s.oidc_groups_claim).cloned().unwrap_or_default();
    // Mappings written for Entra's SAML group URI also apply to the OIDC groups claim.
    attrs.insert(MS_GROUPS_URI.into(), groups);
    complete_login(&st, &s, &org, "oidc", &subject, email, name, &attrs, &redirect_to, ip, headers.get(USER_AGENT).and_then(|v| v.to_str().ok())).await
}

// ================================================================ provisioning

#[allow(clippy::too_many_arguments)]
async fn complete_login(
    st: &AppState,
    s: &Settings,
    org: &str,
    protocol: &str,
    subject: &str,
    email: Option<String>,
    name: Option<String>,
    claims: &HashMap<String, Vec<String>>,
    redirect_to: &str,
    ip: Option<String>,
    user_agent: Option<&str>,
) -> Response {
    match provision(st, s, org, protocol, subject, email, name, claims).await {
        Ok((user_id, display, email)) => match auth::create_session(st, &user_id, org, protocol, false, ip.as_deref(), user_agent).await {
            Ok(token) => {
                auth::audit(&st.postgres, org, Some(&user_id), &display, "login.succeeded", &email, protocol, ip.as_deref()).await;
                (StatusCode::SEE_OTHER, [(SET_COOKIE, auth::session_cookie(st, &token)), (LOCATION, format!("{}{}", st.config.public_url, redirect_to).parse().expect("location"))]).into_response()
            }
            Err(e) => fail(st, &e.to_string()),
        },
        Err(msg) => {
            auth::audit(&st.postgres, org, None, protocol, "login.failed", subject, &msg, ip.as_deref()).await;
            fail(st, &msg)
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn provision(st: &AppState, s: &Settings, org: &str, protocol: &str, subject: &str, email: Option<String>, name: Option<String>, claims: &HashMap<String, Vec<String>>) -> Result<(String, String, String), String> {
    let db = |e: sqlx::Error| {
        tracing::error!(error = %e, "sso provisioning");
        "Sign-in is temporarily unavailable".to_string()
    };
    if subject.is_empty() {
        return Err("The identity provider did not send a user identifier".into());
    }
    let external = format!("{protocol}:{subject}");
    let email = email.map(|e| e.trim().to_lowercase()).filter(|e| e.contains('@'));
    let mut tx = st.postgres.begin().await.map_err(db)?;
    let existing: Option<(String, String, String, String)> = sqlx::query_as(
        "SELECT id::text, status, name, email FROM users WHERE org_id = $1 AND (external_id = $2 OR ($3::text IS NOT NULL AND lower(email) = $3)) \
         ORDER BY (external_id = $2) DESC LIMIT 1",
    )
    .bind(org)
    .bind(&external)
    .bind(&email)
    .fetch_optional(&mut *tx)
    .await
    .map_err(db)?;
    let (user_id, display, email) = match existing {
        Some((_, status, ..)) if status == "deactivated" => return Err("Your ObserveCraft account is deactivated. Contact an administrator.".into()),
        Some((id, _, old_name, old_email)) => {
            sqlx::query(
                "UPDATE users SET external_id = $2, status = 'active', name = COALESCE($3, name), \
                 login_methods = CASE WHEN $4 = ANY(login_methods) THEN login_methods ELSE array_append(login_methods, $4) END WHERE id = $1::uuid",
            )
            .bind(&id)
            .bind(&external)
            .bind(&name)
            .bind(protocol)
            .execute(&mut *tx)
            .await
            .map_err(db)?;
            (id, name.unwrap_or(old_name), email.unwrap_or(old_email))
        }
        None => {
            if !s.jit_provisioning {
                return Err("You don't have an ObserveCraft account yet. Ask an administrator to invite you.".into());
            }
            let Some(email) = email else { return Err("The identity provider did not send an email address".into()) };
            let display = name.unwrap_or_else(|| email.split('@').next().unwrap_or(&email).replace(['.', '_'], " "));
            let (id,): (String,) = sqlx::query_as("INSERT INTO users (id, org_id, email, name, status, login_methods, external_id) VALUES (gen_random_uuid(), $1, $2, $3, 'active', ARRAY[$4], $5) RETURNING id::text")
                .bind(org)
                .bind(&email)
                .bind(&display)
                .bind(protocol)
                .bind(&external)
                .fetch_one(&mut *tx)
                .await
                .map_err(db)?;
            if let Some(role) = &s.default_role_id {
                sqlx::query("INSERT INTO user_roles (user_id, role_id) VALUES ($1::uuid, $2) ON CONFLICT DO NOTHING").bind(&id).bind(role).execute(&mut *tx).await.map_err(db)?;
            }
            (id, display, email)
        }
    };

    // Group mappings: a mapping matches when the user's claim/attribute named `attribute_key` contains `attribute_value`.
    let mappings: Vec<(String, String, String, String)> = sqlx::query_as("SELECT kind, target, attribute_key, attribute_value FROM saml_mappings WHERE org_id = $1").bind(org).fetch_all(&mut *tx).await.map_err(db)?;
    let matches = |key: &str, value: &str| claims.get(key).is_some_and(|vals| vals.iter().any(|v| v == value));
    let mut roles = Vec::new();
    for (kind, target, key, value) in &mappings {
        if !matches(key, value) {
            continue;
        }
        if kind == "team" {
            sqlx::query("INSERT INTO team_members (org_id, handle, user_id) VALUES ($1, $2, $3::uuid) ON CONFLICT DO NOTHING").bind(org).bind(target).bind(&user_id).execute(&mut *tx).await.map_err(db)?;
        } else {
            roles.push(target.clone());
        }
    }
    // Role mappings are authoritative when any matches: the IdP decides the user's roles at each login.
    if !roles.is_empty() {
        sqlx::query("DELETE FROM user_roles WHERE user_id = $1::uuid").bind(&user_id).execute(&mut *tx).await.map_err(db)?;
        sqlx::query("INSERT INTO user_roles (user_id, role_id) SELECT $1::uuid, id FROM roles WHERE org_id = $2 AND id = ANY($3) ON CONFLICT DO NOTHING")
            .bind(&user_id)
            .bind(org)
            .bind(&roles)
            .execute(&mut *tx)
            .await
            .map_err(db)?;
    }
    tx.commit().await.map_err(db)?;
    Ok((user_id, display, email))
}

// ================================================================ Login Methods (admin)

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ServiceProviderInfo {
    /// SAML: Identifier (Entity ID) to enter in the IdP.
    pub entity_id: String,
    /// SAML: Reply URL (Assertion Consumer Service URL).
    pub acs_url: String,
    pub metadata_url: String,
    /// OIDC: redirect URI to register with the IdP.
    pub oidc_redirect_url: String,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SamlStatus {
    pub enabled: bool,
    pub idp_metadata: Option<String>,
    pub idp_entity_id: Option<String>,
    pub idp_sso_url: Option<String>,
    /// Why the stored metadata can't be used, if it can't.
    pub problem: Option<String>,
    pub email_attribute: String,
    pub name_attribute: String,
    pub groups_attribute: String,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct OidcStatus {
    pub enabled: bool,
    pub issuer: Option<String>,
    pub client_id: Option<String>,
    pub has_client_secret: bool,
    pub groups_claim: String,
    pub button_label: String,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AuthSettingsView {
    pub password_enabled: bool,
    pub sso_required: bool,
    pub jit_provisioning: bool,
    pub default_role_id: Option<String>,
    pub saml: SamlStatus,
    pub oidc: OidcStatus,
    pub service_provider: ServiceProviderInfo,
}

fn view(st: &AppState, org: &str, s: Settings) -> AuthSettingsView {
    let (entity_id, acs_url) = sp_urls(st, org);
    let (idp_entity_id, idp_sso_url, problem) = match s.saml_idp_metadata.as_deref() {
        None => (None, None, None),
        Some(m) => match service_provider(st, org, m) {
            Ok(sp) => (Some(sp.idp_metadata.entity_id.clone()).flatten(), sp.sso_binding_location(HTTP_REDIRECT_BINDING), None),
            Err(e) => (None, None, Some(e)),
        },
    };
    AuthSettingsView {
        password_enabled: s.password_enabled,
        sso_required: s.sso_required,
        jit_provisioning: s.jit_provisioning,
        default_role_id: s.default_role_id,
        saml: SamlStatus { enabled: s.saml_enabled, idp_metadata: s.saml_idp_metadata, idp_entity_id, idp_sso_url, problem, email_attribute: s.saml_email_attr, name_attribute: s.saml_name_attr, groups_attribute: s.saml_groups_attr },
        oidc: OidcStatus { enabled: s.oidc_enabled, issuer: s.oidc_issuer, client_id: s.oidc_client_id, has_client_secret: s.oidc_client_secret_enc.is_some(), groups_claim: s.oidc_groups_claim, button_label: s.oidc_button_label },
        service_provider: ServiceProviderInfo { metadata_url: entity_id.clone(), entity_id, acs_url, oidc_redirect_url: oidc_redirect(st, org) },
    }
}

#[utoipa::path(get, path = "/api/v1/org/login-methods", responses((status = 200, body = AuthSettingsView)), tag = "auth")]
pub async fn get_settings(State(st): State<AppState>, Admin(ctx): Admin) -> ApiResult<AuthSettingsView> {
    let s = settings(&st, &ctx.org_id).await?;
    Ok(Json(view(&st, &ctx.org_id, s)))
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AuthSettingsUpdate {
    pub password_enabled: bool,
    pub sso_required: bool,
    pub jit_provisioning: bool,
    pub default_role_id: Option<String>,
    pub saml_enabled: bool,
    /// IdP federation metadata XML (Entra: "Federation Metadata XML").
    pub saml_idp_metadata: Option<String>,
    /// Alternatively, an https URL the server fetches the metadata from (Entra: "App Federation Metadata Url").
    pub saml_idp_metadata_url: Option<String>,
    pub saml_email_attribute: Option<String>,
    pub saml_name_attribute: Option<String>,
    pub saml_groups_attribute: Option<String>,
    pub oidc_enabled: bool,
    pub oidc_issuer: Option<String>,
    pub oidc_client_id: Option<String>,
    /// Replaces the stored secret when present; omit to keep it.
    pub oidc_client_secret: Option<String>,
    pub oidc_groups_claim: Option<String>,
    pub oidc_button_label: Option<String>,
}

async fn fetch_metadata(url: &str) -> Result<String, ApiError> {
    let u = url::Url::parse(url).map_err(|_| ApiError::BadRequest("metadata URL is not a valid URL".into()))?;
    if u.scheme() != "https" && !(u.scheme() == "http" && matches!(u.host_str(), Some("localhost" | "127.0.0.1"))) {
        return Err(ApiError::BadRequest("metadata URL must use https".into()));
    }
    let client = http_client()?;
    let resp = client.get(u).send().await.map_err(|e| ApiError::BadRequest(format!("could not fetch metadata: {e}")))?;
    if !resp.status().is_success() {
        return Err(ApiError::BadRequest(format!("metadata URL returned {}", resp.status())));
    }
    let body = resp.text().await.map_err(|e| ApiError::BadRequest(format!("could not read metadata: {e}")))?;
    if body.len() > 1_000_000 {
        return Err(ApiError::BadRequest("metadata document is too large".into()));
    }
    Ok(body)
}

fn opt(s: Option<String>) -> Option<String> {
    s.map(|v| v.trim().to_string()).filter(|v| !v.is_empty())
}

#[utoipa::path(put, path = "/api/v1/org/login-methods", request_body = AuthSettingsUpdate, responses((status = 200, body = AuthSettingsView), (status = 400, body = crate::error::ErrorBody)), tag = "auth")]
pub async fn update_settings(State(st): State<AppState>, Admin(ctx): Admin, Json(u): Json<AuthSettingsUpdate>) -> ApiResult<AuthSettingsView> {
    let org = ctx.org_id.clone();
    let current = settings(&st, &org).await?;
    let metadata = match opt(u.saml_idp_metadata_url.clone()) {
        Some(url) => Some(fetch_metadata(&url).await?),
        None => opt(u.saml_idp_metadata.clone()).or(current.saml_idp_metadata.clone()),
    };
    if let Some(m) = &metadata {
        if let Err(e) = service_provider(&st, &org, m) {
            // Storing broken metadata is allowed only while SAML stays disabled.
            if u.saml_enabled {
                return Err(ApiError::BadRequest(e));
            }
        }
    } else if u.saml_enabled {
        return Err(ApiError::BadRequest("Provide the IdP metadata before enabling SAML".into()));
    }
    let issuer = opt(u.oidc_issuer.clone()).map(|i| i.trim_end_matches('/').to_string());
    let client_id = opt(u.oidc_client_id.clone());
    if u.oidc_enabled {
        let (Some(i), Some(_)) = (&issuer, &client_id) else { return Err(ApiError::BadRequest("OIDC needs an issuer URL and a client ID".into())) };
        discover(i).await.map_err(ApiError::BadRequest)?;
    }
    if !u.password_enabled && !u.saml_enabled && !u.oidc_enabled {
        return Err(ApiError::BadRequest("Enable at least one login method".into()));
    }
    if u.sso_required && !u.saml_enabled && !u.oidc_enabled {
        return Err(ApiError::BadRequest("Enable SAML or OIDC before requiring SSO".into()));
    }
    let secret_enc = opt(u.oidc_client_secret.clone()).map(|s| st.secrets.encrypt(&s)).or(current.oidc_client_secret_enc.clone());
    sqlx::query(
        "UPDATE auth_settings SET password_enabled = $2, sso_required = $3, jit_provisioning = $4, \
         default_role_id = (SELECT id FROM roles WHERE org_id = $1 AND id = $5), \
         saml_enabled = $6, saml_idp_metadata = $7, saml_email_attr = COALESCE($8, saml_email_attr), saml_name_attr = COALESCE($9, saml_name_attr), \
         saml_groups_attr = COALESCE($10, saml_groups_attr), oidc_enabled = $11, oidc_issuer = $12, oidc_client_id = $13, oidc_client_secret_enc = $14, \
         oidc_groups_claim = COALESCE($15, oidc_groups_claim), oidc_button_label = COALESCE($16, oidc_button_label), updated_at = now() WHERE org_id = $1",
    )
    .bind(&org)
    .bind(u.password_enabled)
    .bind(u.sso_required)
    .bind(u.jit_provisioning)
    .bind(&u.default_role_id)
    .bind(u.saml_enabled)
    .bind(&metadata)
    .bind(opt(u.saml_email_attribute.clone()))
    .bind(opt(u.saml_name_attribute.clone()))
    .bind(opt(u.saml_groups_attribute.clone()))
    .bind(u.oidc_enabled)
    .bind(&issuer)
    .bind(&client_id)
    .bind(&secret_enc)
    .bind(opt(u.oidc_groups_claim.clone()))
    .bind(opt(u.oidc_button_label.clone()))
    .execute(&st.postgres)
    .await?;
    let changes = [
        ("password", current.password_enabled, u.password_enabled),
        ("sso_required", current.sso_required, u.sso_required),
        ("saml", current.saml_enabled, u.saml_enabled),
        ("oidc", current.oidc_enabled, u.oidc_enabled),
        ("jit", current.jit_provisioning, u.jit_provisioning),
    ]
    .iter()
    .filter(|(_, a, b)| a != b)
    .map(|(k, _, b)| format!("{k}={b}"))
    .collect::<Vec<_>>()
    .join(", ");
    ctx.audit(&st, "org.login_methods_updated", "", &changes).await;
    let s = settings(&st, &org).await?;
    Ok(Json(view(&st, &org, s)))
}

#[cfg(test)]
mod tests {
    use super::safe_redirect;

    #[test]
    fn redirects_stay_in_app() {
        assert_eq!(safe_redirect(Some("/dashboards/abc?x=1")), "/dashboards/abc?x=1");
        assert_eq!(safe_redirect(Some("//evil.example")), "/");
        assert_eq!(safe_redirect(Some("/\\evil.example")), "/");
        assert_eq!(safe_redirect(Some("https://evil.example")), "/");
        assert_eq!(safe_redirect(Some("/api/v1/me")), "/");
        assert_eq!(safe_redirect(None), "/");
    }

    /// The identity provider sees a User-Agent on discovery, JWKS, token and userinfo requests.
    #[tokio::test]
    async fn oidc_requests_carry_a_user_agent() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.unwrap();
            let mut buf = vec![0u8; 4096];
            let n = sock.read(&mut buf).await.unwrap();
            sock.write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 0\r\nconnection: close\r\n\r\n").await.unwrap();
            String::from_utf8_lossy(&buf[..n]).to_lowercase()
        });
        super::http_client().unwrap().get(format!("http://{addr}/.well-known/openid-configuration")).send().await.unwrap();
        let request = server.await.unwrap();
        assert!(request.contains(&format!("user-agent: {}", super::OIDC_USER_AGENT.to_lowercase())), "{request}");
    }
}
