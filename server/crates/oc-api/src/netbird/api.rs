//! NetBird's management API (`/api`) as the NetBird integration reads it: request paths and [`Rest`], which sends
//! them through the Developer Portal connectors' client (`routes::portal_sources::send_capped`: the SSRF guard,
//! `OC_PORTAL_ALLOW_PRIVATE`, no redirects, capped answers, timeouts). The personal access token goes only into the
//! `Authorization: Token` header and is scrubbed from every error.
//!
//! NetBird answers its audit log in one list (the newest 10,000 events, no paging), so answers may be larger than
//! the portal connectors' cap: [`MAX_ANSWER_BYTES`].

use serde_json::Value;

use crate::routes::portal_sources::{send_capped, Auth, HttpCall};

/// The largest answer read: 10,000 audit events with their metadata, or a large account's peers.
pub const MAX_ANSWER_BYTES: usize = 32 << 20;
/// The token's own user.
pub const CURRENT_USER: &str = "/api/users/current";
/// The account's audit events, newest first.
pub const AUDIT_EVENTS: &str = "/api/events/audit";
/// The account's peers.
pub const PEERS: &str = "/api/peers";

#[derive(Clone, Debug, PartialEq)]
pub enum NetbirdError {
    /// HTTP 429.
    Throttled(String),
    /// HTTP 401 / 403: the token was refused or its user's role can't read this.
    Auth(String),
    /// HTTP 404: not NetBird's management API at this URL.
    NotFound(String),
    Other(String),
}

impl NetbirdError {
    pub fn message(&self) -> &str {
        match self {
            NetbirdError::Throttled(m) | NetbirdError::Auth(m) | NetbirdError::NotFound(m) | NetbirdError::Other(m) => m,
        }
    }
}

/// NetBird, as the poller and the connection test read it.
#[async_trait::async_trait]
pub trait NetbirdApi: Send + Sync {
    /// `GET {base}{path}` with the token.
    async fn get(&self, base: &str, token: &str, path: &str) -> Result<Value, NetbirdError>;
}

/// The real API, through the SSRF guard.
pub struct Rest {
    pub allow_private: bool,
}

/// A failed request as a [`NetbirdError`], by its HTTP status, without the token.
pub fn classify(status: Option<u16>, message: &str, token: &str) -> NetbirdError {
    let message = if token.is_empty() { message.to_string() } else { message.replace(token, "[token]") };
    match status {
        Some(429) => NetbirdError::Throttled(message),
        Some(401) | Some(403) => NetbirdError::Auth(message),
        Some(404) => NetbirdError::NotFound(message),
        _ => NetbirdError::Other(message),
    }
}

#[async_trait::async_trait]
impl NetbirdApi for Rest {
    async fn get(&self, base: &str, token: &str, path: &str) -> Result<Value, NetbirdError> {
        let call = HttpCall::get(format!("{base}{path}"), Auth::Raw(format!("Token {token}")));
        send_capped(&call, self.allow_private, MAX_ANSWER_BYTES).await.map_err(|(status, message)| classify(status, &message, token))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn errors_are_classified_without_the_token() {
        assert_eq!(classify(Some(401), "token nbp_secret refused", "nbp_secret"), NetbirdError::Auth("token [token] refused".into()));
        assert!(matches!(classify(Some(429), "", "t"), NetbirdError::Throttled(_)));
        assert!(matches!(classify(Some(404), "", "t"), NetbirdError::NotFound(_)));
        assert!(matches!(classify(None, "timeout", "t"), NetbirdError::Other(_)));
    }
}
