//! Google Workspace's Admin SDK Reports API as the Google Workspace integration reads it.
//!
//! - The customer's service account key ([`ServiceAccount`]) signs a JWT assertion ([`assertion`]) for the Reports
//!   API's audit scope ([`AUDIT_SCOPE`]) on behalf of an administrator (`sub`, domain-wide delegation), exchanged at
//!   Google's token endpoint ([`TOKEN_URL`]) for an access token, cached until it nearly expires.
//! - Requests are activity lists ([`activities_path`]): `startTime` / `endTime`, newest first, `pageToken` pages.
//!
//! Both hosts are fixed (Google's): the key's own `token_uri` is never called, so a key can't point the server
//! elsewhere. Activity reads go through the Developer Portal connectors' client (the SSRF guard, no redirects,
//! timeouts, capped answers). The key and tokens are scrubbed from every error.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use base64::Engine;
use serde_json::{json, Value};
use tokio::sync::Mutex;

use crate::routes::portal_sources::{send_capped, Auth, HttpCall, TIMEOUT_SECONDS};

/// The Reports API's audit scope (read-only), the only one the service account is delegated.
pub const AUDIT_SCOPE: &str = "https://www.googleapis.com/auth/admin.reports.audit.readonly";
/// Google's OAuth token endpoint.
pub const TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
/// The Admin SDK's host.
pub const API_BASE: &str = "https://admin.googleapis.com";
/// Activities per page (the Reports API's maximum).
pub const PAGE_SIZE: usize = 1000;
/// A page's answer cap (activities of a thousand Drive events with their parameters).
const MAX_PAGE_BYTES: usize = 32 << 20;

/// The Reports API applications the integration can read: id (`applicationName`) and name.
pub const APPLICATIONS: &[(&str, &str)] = &[
    ("access_transparency", "Access Transparency"),
    ("admin", "Admin console"),
    ("calendar", "Calendar"),
    ("chat", "Google Chat"),
    ("chrome", "Chrome"),
    ("context_aware_access", "Context-Aware Access"),
    ("drive", "Drive"),
    ("gcp", "Google Cloud"),
    ("groups", "Groups"),
    ("groups_enterprise", "Enterprise Groups"),
    ("jamboard", "Jamboard"),
    ("keep", "Keep"),
    ("login", "Login"),
    ("meet", "Meet"),
    ("mobile", "Mobile devices"),
    ("rules", "Rules"),
    ("saml", "SAML"),
    ("takeout", "Takeout"),
    ("token", "OAuth tokens"),
    ("user_accounts", "User accounts"),
    ("vault", "Vault"),
];

/// Read when an admin picks none.
pub const DEFAULT_APPLICATIONS: &[&str] = &["admin", "login", "user_accounts", "token", "saml", "groups", "drive", "mobile"];

pub fn is_application(id: &str) -> bool {
    APPLICATIONS.iter().any(|(a, _)| *a == id)
}

#[derive(Clone, Debug, PartialEq)]
pub enum WorkspaceError {
    /// HTTP 429 / 503.
    Throttled(String),
    /// The key was refused (deleted, disabled, a bad signature) or the administrator doesn't exist.
    Auth(String),
    /// Domain-wide delegation isn't set up for the service account and the audit scope, or the subject isn't an
    /// administrator allowed to read reports.
    Denied(String),
    Other(String),
}

impl WorkspaceError {
    pub fn message(&self) -> &str {
        match self {
            WorkspaceError::Throttled(m) | WorkspaceError::Auth(m) | WorkspaceError::Denied(m) | WorkspaceError::Other(m) => m,
        }
    }
}

/// A service account key (Google's JSON key file), checked: its signer works.
#[derive(Clone, PartialEq)]
pub struct ServiceAccount {
    pub client_email: String,
    pub private_key_id: String,
    /// The key file as given (sealed at rest).
    pub json: String,
}

impl std::fmt::Debug for ServiceAccount {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ServiceAccount").field("client_email", &self.client_email).finish_non_exhaustive()
    }
}

/// Parses and checks a JSON key file: a service account's, with a usable RSA private key.
pub fn parse_key(json: &str) -> Result<ServiceAccount, String> {
    let v: Value = serde_json::from_str(json.trim()).map_err(|_| "The key is not JSON: paste the service account's JSON key file".to_string())?;
    if v["type"].as_str() != Some("service_account") {
        return Err("The key is not a service account key (its type must be service_account)".into());
    }
    let client_email = v["client_email"].as_str().map(str::trim).filter(|e| e.contains('@')).ok_or("The key has no client_email")?.to_string();
    if v["private_key"].as_str().is_none_or(str::is_empty) {
        return Err("The key has no private_key".into());
    }
    // gcp_auth wants a token_uri; ours is fixed, whatever the key says.
    let mut checked = v.clone();
    checked["token_uri"] = json!(TOKEN_URL);
    gcp_auth::CustomServiceAccount::from_json(&checked.to_string()).map_err(|_| "The key's private_key is not a valid RSA key".to_string())?;
    Ok(ServiceAccount { client_email, private_key_id: v["private_key_id"].as_str().unwrap_or_default().to_string(), json: json.trim().to_string() })
}

fn b64(bytes: &[u8]) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

/// The JWT claims of a token request for `subject` at `now_s` (epoch seconds), valid an hour.
pub fn claims(key: &ServiceAccount, subject: &str, now_s: i64) -> Value {
    json!({ "iss": key.client_email, "sub": subject, "scope": AUDIT_SCOPE, "aud": TOKEN_URL, "iat": now_s, "exp": now_s + 3600 })
}

/// The signed JWT assertion (RS256) of [`claims`].
pub fn assertion(key: &ServiceAccount, subject: &str, now_s: i64) -> Result<String, WorkspaceError> {
    let mut v: Value = serde_json::from_str(&key.json).map_err(|_| WorkspaceError::Auth("The service account key is not JSON".into()))?;
    v["token_uri"] = json!(TOKEN_URL);
    let account = gcp_auth::CustomServiceAccount::from_json(&v.to_string()).map_err(|_| WorkspaceError::Auth("The service account key's private_key is not a valid RSA key".into()))?;
    let mut header = json!({ "alg": "RS256", "typ": "JWT" });
    if !key.private_key_id.is_empty() {
        header["kid"] = json!(key.private_key_id);
    }
    let signed = format!("{}.{}", b64(header.to_string().as_bytes()), b64(claims(key, subject, now_s).to_string().as_bytes()));
    let signature = account.signer().sign(signed.as_bytes()).map_err(|e| WorkspaceError::Other(format!("Signing the token request failed: {e}")))?;
    Ok(format!("{signed}.{}", b64(&signature)))
}

/// RFC 3339 (UTC, milliseconds) of epoch ms, as `startTime` / `endTime` take it.
pub fn rfc3339_ms(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms).map(|d| d.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string()).unwrap_or_default()
}

/// A page of `application`'s activities from `start_ms` to `end_ms`, newest first.
pub fn activities_path(application: &str, start_ms: i64, end_ms: i64, page_token: Option<&str>, max_results: usize) -> String {
    let mut q = url::form_urlencoded::Serializer::new(String::new());
    q.append_pair("startTime", &rfc3339_ms(start_ms)).append_pair("endTime", &rfc3339_ms(end_ms)).append_pair("maxResults", &max_results.to_string());
    if let Some(t) = page_token {
        q.append_pair("pageToken", t);
    }
    format!("/admin/reports/v1/activity/users/all/applications/{}?{}", url::form_urlencoded::byte_serialize(application.as_bytes()).collect::<String>(), q.finish())
}

/// A token endpoint error (`{"error": …, "error_description": …}`) as a [`WorkspaceError`] that says what to fix.
pub fn classify_token_error(status: Option<u16>, body: &str) -> WorkspaceError {
    let v: Value = serde_json::from_str(body).unwrap_or_default();
    let (error, description) = (v["error"].as_str().unwrap_or_default(), v["error_description"].as_str().unwrap_or(body).trim());
    match (status, error) {
        (Some(429) | Some(503), _) => WorkspaceError::Throttled(format!("Google's token endpoint is busy: {description}")),
        (_, "unauthorized_client" | "access_denied") => WorkspaceError::Denied(format!(
            "Google refused domain-wide delegation: in the Admin console (Security → Access and data control → API controls → Domain-wide delegation), authorize the service account's client ID for {AUDIT_SCOPE} ({description})"
        )),
        (_, "invalid_grant") if description.to_lowercase().contains("email") || description.to_lowercase().contains("user") => {
            WorkspaceError::Auth(format!("Google doesn't know the administrator to act as: enter an administrator's email of the Workspace account ({description})"))
        }
        (_, "invalid_grant") => WorkspaceError::Auth(format!("Google refused the service account key: it may have been deleted or disabled ({description})")),
        (_, "") => WorkspaceError::Other(format!("Google's token endpoint answered {}", status.map_or("nothing".to_string(), |s| format!("HTTP {s}")))),
        _ => WorkspaceError::Auth(format!("Google refused the token request: {error} ({description})")),
    }
}

/// A Reports API error as a [`WorkspaceError`], by its HTTP status, without the token.
pub fn classify_api_error(status: Option<u16>, message: &str, token: &str) -> WorkspaceError {
    let message = if token.is_empty() { message.to_string() } else { message.replace(token, "[token]") };
    match status {
        Some(429) | Some(503) => WorkspaceError::Throttled(message),
        Some(401) => WorkspaceError::Auth(message),
        Some(403) => WorkspaceError::Denied(format!(
            "The administrator can't read the Reports API (an administrator with the Reports privilege, and the Admin SDK API enabled in the service account's project): {message}"
        )),
        _ => WorkspaceError::Other(message),
    }
}

/// A connection's credentials: the key and the administrator it acts as.
#[derive(Clone, Debug, PartialEq)]
pub struct Credentials {
    pub key: ServiceAccount,
    pub subject: String,
}

/// The Reports API, as the poller and the connection test read it.
#[async_trait::async_trait]
pub trait WorkspaceApi: Send + Sync {
    /// `GET {API_BASE}{path}` as the connection's administrator.
    async fn get(&self, creds: &Credentials, path: &str) -> Result<Value, WorkspaceError>;
}

/// A cached token's key: the service account, its key id and the administrator.
type TokenKey = (String, String, String);

/// The real API: tokens from Google's endpoint (cached per key and administrator), reads through the SSRF guard.
#[derive(Default)]
pub struct Rest {
    tokens: Mutex<HashMap<TokenKey, (String, Instant)>>,
}

impl Rest {
    async fn token(&self, creds: &Credentials) -> Result<String, WorkspaceError> {
        let cache_key = (creds.key.client_email.clone(), creds.key.private_key_id.clone(), creds.subject.clone());
        if let Some((t, until)) = self.tokens.lock().await.get(&cache_key) {
            if Instant::now() < *until {
                return Ok(t.clone());
            }
        }
        let jwt = assertion(&creds.key, &creds.subject, chrono::Utc::now().timestamp())?;
        let body = url::form_urlencoded::Serializer::new(String::new()).append_pair("grant_type", "urn:ietf:params:oauth:grant-type:jwt-bearer").append_pair("assertion", &jwt).finish();
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .no_proxy()
            .connect_timeout(Duration::from_secs(TIMEOUT_SECONDS))
            .timeout(Duration::from_secs(TIMEOUT_SECONDS))
            .build()
            .map_err(|e| WorkspaceError::Other(e.to_string()))?;
        let res = client
            .post(TOKEN_URL)
            .header(reqwest::header::CONTENT_TYPE, "application/x-www-form-urlencoded")
            .body(body)
            .send()
            .await
            .map_err(|e| WorkspaceError::Other(format!("Google's token endpoint: {e}")))?;
        let status = res.status().as_u16();
        let text = res.text().await.unwrap_or_default();
        if !(200..300).contains(&status) {
            return Err(classify_token_error(Some(status), &text));
        }
        let v: Value = serde_json::from_str(&text).unwrap_or_default();
        let token = v["access_token"].as_str().filter(|t| !t.is_empty()).ok_or_else(|| WorkspaceError::Other("Google's token endpoint answered without an access token".into()))?.to_string();
        let ttl = v["expires_in"].as_u64().unwrap_or(3600).saturating_sub(300).max(60);
        self.tokens.lock().await.insert(cache_key, (token.clone(), Instant::now() + Duration::from_secs(ttl)));
        Ok(token)
    }
}

#[async_trait::async_trait]
impl WorkspaceApi for Rest {
    async fn get(&self, creds: &Credentials, path: &str) -> Result<Value, WorkspaceError> {
        let token = self.token(creds).await?;
        let call = HttpCall::get(format!("{API_BASE}{path}"), Auth::Bearer(token.clone()));
        let answer = send_capped(&call, false, MAX_PAGE_BYTES).await.map_err(|(status, message)| classify_api_error(status, &message, &token));
        if matches!(answer, Err(WorkspaceError::Auth(_))) {
            // A token revoked early: the next request asks for a new one.
            self.tokens.lock().await.retain(|_, (t, _)| *t != token);
        }
        answer
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::google_workspace::fixtures;

    #[test]
    fn keys_are_checked() {
        let key = parse_key(&fixtures::key_json()).unwrap();
        assert_eq!((key.client_email.as_str(), key.private_key_id.as_str()), ("observecraft-reports@example-project.iam.gserviceaccount.com", "0123456789abcdef"));
        assert!(!format!("{key:?}").contains("PRIVATE KEY"), "the key isn't printed");
        let mut v: Value = serde_json::from_str(&fixtures::key_json()).unwrap();
        for (change, needle) in [
            (json!({"type": "authorized_user"}), "service_account"),
            (json!({"client_email": null}), "client_email"),
            (json!({"private_key": ""}), "private_key"),
            (json!({"private_key": "-----BEGIN PRIVATE KEY-----\nAAAA\n-----END PRIVATE KEY-----\n"}), "valid RSA"),
        ] {
            let mut bad = v.clone();
            for (k, val) in change.as_object().unwrap() {
                bad[k] = val.clone();
            }
            assert!(parse_key(&bad.to_string()).unwrap_err().contains(needle), "{change}");
        }
        assert!(parse_key("not json").unwrap_err().contains("JSON"));
        // A key without token_uri is fine: ours is fixed.
        v.as_object_mut().unwrap().remove("token_uri");
        assert!(parse_key(&v.to_string()).is_ok());
    }

    #[test]
    fn the_assertion_is_an_rs256_jwt_for_the_audit_scope_as_the_administrator() {
        let key = parse_key(&fixtures::key_json()).unwrap();
        let jwt = assertion(&key, "admin@example.com", 1_793_959_200).unwrap();
        let parts: Vec<&str> = jwt.split('.').collect();
        assert_eq!(parts.len(), 3);
        let decode = |s: &str| -> Value { serde_json::from_slice(&base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(s).unwrap()).unwrap() };
        assert_eq!(decode(parts[0]), json!({ "alg": "RS256", "typ": "JWT", "kid": "0123456789abcdef" }));
        assert_eq!(
            decode(parts[1]),
            json!({ "iss": "observecraft-reports@example-project.iam.gserviceaccount.com", "sub": "admin@example.com", "scope": AUDIT_SCOPE, "aud": TOKEN_URL, "iat": 1_793_959_200, "exp": 1_793_962_800 })
        );
        // A 2048-bit RSA signature.
        assert_eq!(base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(parts[2]).unwrap().len(), 256);
        // The key's own token_uri is never the audience.
        let mut v: Value = serde_json::from_str(&fixtures::key_json()).unwrap();
        v["token_uri"] = json!("https://169.254.169.254/token");
        let other = parse_key(&v.to_string()).unwrap();
        assert_eq!(decode(assertion(&other, "admin@example.com", 0).unwrap().split('.').nth(1).unwrap())["aud"], TOKEN_URL);
    }

    #[test]
    fn activity_paths() {
        let start = chrono::DateTime::parse_from_rfc3339("2026-11-06T11:00:00Z").unwrap().timestamp_millis();
        assert_eq!(
            activities_path("login", start, start + 60_250, None, PAGE_SIZE),
            "/admin/reports/v1/activity/users/all/applications/login?startTime=2026-11-06T11%3A00%3A00.000Z&endTime=2026-11-06T11%3A01%3A00.250Z&maxResults=1000"
        );
        assert!(activities_path("drive", start, start, Some("A:1+b/c"), 1).ends_with("&maxResults=1&pageToken=A%3A1%2Bb%2Fc"));
        assert!(is_application("user_accounts") && !is_application("../users"));
        assert!(DEFAULT_APPLICATIONS.iter().all(|a| is_application(a)));
    }

    #[test]
    fn errors_say_what_to_fix() {
        let e = classify_token_error(
            Some(401),
            r#"{"error":"unauthorized_client","error_description":"Client is unauthorized to retrieve access tokens using this method, or client not authorized for any of the scopes requested."}"#,
        );
        assert!(matches!(&e, WorkspaceError::Denied(m) if m.contains("Domain-wide delegation") && m.contains(AUDIT_SCOPE)), "{e:?}");
        let e = classify_token_error(Some(400), r#"{"error":"invalid_grant","error_description":"Invalid email or User ID"}"#);
        assert!(matches!(&e, WorkspaceError::Auth(m) if m.contains("administrator")), "{e:?}");
        let e = classify_token_error(Some(400), r#"{"error":"invalid_grant","error_description":"Invalid JWT Signature."}"#);
        assert!(matches!(&e, WorkspaceError::Auth(m) if m.contains("deleted or disabled")), "{e:?}");
        assert!(matches!(classify_token_error(Some(503), "busy"), WorkspaceError::Throttled(_)));
        let e = classify_api_error(Some(403), "The provider answered: access is forbidden (Not Authorized to access this resource/api) ya29.secret", "ya29.secret");
        assert!(matches!(&e, WorkspaceError::Denied(m) if m.contains("Reports privilege") && !m.contains("ya29.secret")), "{e:?}");
        assert!(matches!(classify_api_error(Some(429), "slow down", ""), WorkspaceError::Throttled(_)));
        assert!(matches!(classify_api_error(Some(401), "refused", ""), WorkspaceError::Auth(_)));
    }
}
