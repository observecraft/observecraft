//! Vanta's Build Integrations API, as the sync uses it: a client-credentials token (`POST /oauth/token`) and the
//! full set of a resource type (`PUT /v1/resources/<type>`). Only Vanta's own hosts are called ([`API_URLS`]); the
//! client secret and the token go only into the token request and the `Authorization` header, never into errors.

use std::time::Duration;

use serde_json::{json, Value};

/// Vanta's API hosts: the commercial one and Vanta Gov.
pub const API_URLS: [&str; 2] = ["https://api.vanta.com", "https://api.vanta-gov.com"];
/// The scopes a private integration's token needs to push (and read back) its resources.
pub const SCOPES: &str = "connectors.self:write-resource connectors.self:read-resource";

#[derive(Clone, Debug, PartialEq)]
pub enum VantaError {
    /// HTTP 429 (10 requests a minute per token).
    Throttled(String),
    /// HTTP 400 / 401 / 403 on the token, 401 / 403 on a push: wrong credentials, or a `resourceId` of another app.
    Auth(String),
    /// HTTP 422 / 400 on a push: the body was refused (a field Vanta doesn't accept).
    Invalid(String),
    Other(String),
}

impl VantaError {
    pub fn message(&self) -> &str {
        match self {
            VantaError::Throttled(m) | VantaError::Auth(m) | VantaError::Invalid(m) | VantaError::Other(m) => m,
        }
    }
}

/// Vanta, as the sync and the connection test call it.
#[async_trait::async_trait]
pub trait VantaApi: Send + Sync {
    /// An access token for the app (one hour; it revokes the app's previous token).
    async fn token(&self, api_url: &str, client_id: &str, client_secret: &str) -> Result<String, VantaError>;
    /// `PUT {api_url}/v1/resources/{resource_type}` with `body`; Vanta's answer.
    async fn put(&self, api_url: &str, token: &str, resource_type: &str, body: &Value) -> Result<Value, VantaError>;
}

/// The token request's body.
pub fn token_body(client_id: &str, client_secret: &str) -> Value {
    json!({ "client_id": client_id, "client_secret": client_secret, "scope": SCOPES, "grant_type": "client_credentials" })
}

/// What Vanta accepted and rejected of a push: `{"results": {"accepted", "rejected"}}`, or `{"success": true}` (user
/// accounts), which accepts all `sent`.
pub fn counts(answer: &Value, sent: usize) -> (usize, usize) {
    if let Some(r) = answer.get("results") {
        let n = |k: &str| r[k].as_u64().unwrap_or(0) as usize;
        return (n("accepted"), n("rejected"));
    }
    if answer["success"].as_bool() == Some(true) {
        (sent, 0)
    } else {
        (0, sent)
    }
}

/// A failed request as a [`VantaError`] by its HTTP status (`token`: whether it was the token request), with the
/// secrets scrubbed.
pub fn classify(status: Option<u16>, message: &str, secrets: &[&str], token: bool) -> VantaError {
    let message = secrets.iter().filter(|s| !s.is_empty()).fold(message.to_string(), |m, s| m.replace(s, "[secret]"));
    match status {
        Some(429) => VantaError::Throttled(message),
        Some(401) | Some(403) => VantaError::Auth(message),
        Some(400) if token => VantaError::Auth(message),
        Some(400) | Some(422) => VantaError::Invalid(message),
        _ => VantaError::Other(message),
    }
}

/// The real API.
pub struct Rest;

fn client() -> reqwest::Client {
    reqwest::Client::builder().timeout(Duration::from_secs(60)).user_agent("ObserveCraft-Vanta").redirect(reqwest::redirect::Policy::none()).build().unwrap_or_default()
}

async fn answer(r: Result<reqwest::Response, reqwest::Error>, secrets: &[&str], token: bool) -> Result<Value, VantaError> {
    let r = r.map_err(|e| classify(None, &format!("Vanta is unreachable: {e}"), secrets, token))?;
    let status = r.status();
    let text = r.text().await.unwrap_or_default();
    if !status.is_success() {
        let detail: String = text.chars().take(300).collect();
        return Err(classify(Some(status.as_u16()), &format!("Vanta answered {status}: {detail}"), secrets, token));
    }
    serde_json::from_str(&text).map_err(|e| VantaError::Other(format!("Vanta's answer can't be read: {e}")))
}

#[async_trait::async_trait]
impl VantaApi for Rest {
    async fn token(&self, api_url: &str, client_id: &str, client_secret: &str) -> Result<String, VantaError> {
        if !API_URLS.contains(&api_url) {
            return Err(VantaError::Other(format!("{api_url} is not Vanta's API")));
        }
        let v = answer(client().post(format!("{api_url}/oauth/token")).json(&token_body(client_id, client_secret)).send().await, &[client_secret], true).await?;
        v["access_token"].as_str().map(str::to_string).ok_or_else(|| VantaError::Other("Vanta returned no access token".into()))
    }

    async fn put(&self, api_url: &str, token: &str, resource_type: &str, body: &Value) -> Result<Value, VantaError> {
        if !API_URLS.contains(&api_url) {
            return Err(VantaError::Other(format!("{api_url} is not Vanta's API")));
        }
        answer(client().put(format!("{api_url}/v1/resources/{resource_type}")).bearer_auth(token).json(body).send().await, &[token], false).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_token_request_asks_for_the_push_scopes() {
        let b = token_body("vci_example", "vcs_secret");
        assert_eq!(b["grant_type"], "client_credentials");
        assert_eq!(b["scope"], "connectors.self:write-resource connectors.self:read-resource");
        assert_eq!((b["client_id"].as_str(), b["client_secret"].as_str()), (Some("vci_example"), Some("vcs_secret")));
    }

    #[test]
    fn answers_are_counted_in_both_shapes() {
        assert_eq!(counts(&json!({ "results": { "accepted": 7, "rejected": 2 } }), 9), (7, 2));
        assert_eq!(counts(&json!({ "success": true }), 4), (4, 0));
        assert_eq!(counts(&json!({ "success": false }), 4), (0, 4));
        assert_eq!(counts(&json!({}), 3), (0, 3));
    }

    #[test]
    fn errors_are_classified_without_the_secrets() {
        assert_eq!(classify(Some(401), "bad client vcs_secret", &["vcs_secret"], true), VantaError::Auth("bad client [secret]".into()));
        assert!(matches!(classify(Some(400), "invalid_client", &[], true), VantaError::Auth(_)), "a refused token request is the credentials");
        assert!(matches!(classify(Some(400), "missing field", &[], false), VantaError::Invalid(_)), "a refused push is its body");
        assert!(matches!(classify(Some(422), "", &[], false), VantaError::Invalid(_)));
        assert!(matches!(classify(Some(403), "", &[], false), VantaError::Auth(_)));
        assert!(matches!(classify(Some(429), "", &[], false), VantaError::Throttled(_)));
        assert!(matches!(classify(Some(503), "", &[], false), VantaError::Other(_)));
        assert!(matches!(classify(None, "timeout", &[], true), VantaError::Other(_)));
    }
}
