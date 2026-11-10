//! authentik's API (`/api/v3`) as the authentik integration reads it: request paths (pure, tested below) and
//! [`Rest`], which sends them through the Developer Portal connectors' client (`routes::portal_sources::send_with_status`:
//! the SSRF guard, `OC_PORTAL_ALLOW_PRIVATE`, no redirects, capped answers, timeouts). The token goes only into the
//! `Authorization: Bearer` header and is scrubbed from every error.

use serde_json::Value;

use crate::routes::portal_sources::{send_with_status, Auth, HttpCall};

/// Events per page (authentik's page size limit is larger; pages stay small to bound a run's memory).
pub const PAGE_SIZE: usize = 100;

#[derive(Clone, Debug, PartialEq)]
pub enum AuthentikError {
    /// HTTP 429.
    Throttled(String),
    /// HTTP 401 / 403: the token was refused or lacks `authentik_events.view_event`.
    Auth(String),
    /// HTTP 404: not authentik's API at this URL.
    NotFound(String),
    Other(String),
}

impl AuthentikError {
    pub fn message(&self) -> &str {
        match self {
            AuthentikError::Throttled(m) | AuthentikError::Auth(m) | AuthentikError::NotFound(m) | AuthentikError::Other(m) => m,
        }
    }
}

/// authentik, as the poller and the connection test read it.
#[async_trait::async_trait]
pub trait AuthentikApi: Send + Sync {
    /// `GET {base}/api/v3{path}` with the token.
    async fn get(&self, base: &str, token: &str, path: &str) -> Result<Value, AuthentikError>;
}

/// The real API, through the SSRF guard.
pub struct Rest {
    pub allow_private: bool,
}

/// A failed request as an [`AuthentikError`], by its HTTP status, without the token.
pub fn classify(status: Option<u16>, message: &str, token: &str) -> AuthentikError {
    let message = if token.is_empty() { message.to_string() } else { message.replace(token, "[token]") };
    match status {
        Some(429) => AuthentikError::Throttled(message),
        Some(401) | Some(403) => AuthentikError::Auth(message),
        Some(404) => AuthentikError::NotFound(message),
        _ => AuthentikError::Other(message),
    }
}

#[async_trait::async_trait]
impl AuthentikApi for Rest {
    async fn get(&self, base: &str, token: &str, path: &str) -> Result<Value, AuthentikError> {
        let call = HttpCall::get(format!("{base}/api/v3{path}"), Auth::Bearer(token.to_string()));
        send_with_status(&call, self.allow_private).await.map_err(|(status, message)| classify(status, &message, token))
    }
}

/// The token's own user.
pub fn me_path() -> String {
    "/core/users/me/".into()
}

/// A page of events (1-based), oldest first: new events only add to the last pages, so a page read later still
/// starts where it did.
pub fn events_path(page: usize) -> String {
    format!("/events/events/?ordering=created&page_size={PAGE_SIZE}&page={page}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_and_errors() {
        assert_eq!(events_path(2), "/events/events/?ordering=created&page_size=100&page=2");
        assert_eq!(me_path(), "/core/users/me/");
        assert_eq!(classify(Some(403), "token ak-secret refused", "ak-secret"), AuthentikError::Auth("token [token] refused".into()));
        assert!(matches!(classify(Some(429), "", "t"), AuthentikError::Throttled(_)));
        assert!(matches!(classify(None, "timeout", "t"), AuthentikError::Other(_)));
    }
}
