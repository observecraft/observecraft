//! Fleet's REST API (`/api/v1/fleet`) as the FleetDM integration reads it: request paths (pure, tested below) and
//! [`Rest`], which sends them through the Developer Portal connectors' client (`routes::portal_sources::send_with_status`:
//! the SSRF guard, `OC_PORTAL_ALLOW_PRIVATE`, no redirects, capped answers, timeouts). The API token goes only into the
//! `Authorization: Bearer` header and is scrubbed from every error.

use serde_json::Value;

use crate::routes::portal_sources::{send_with_status, Auth, HttpCall};

/// Activities per page.
pub const ACTIVITIES_PAGE_SIZE: usize = 100;
/// Hosts per page (a listed host is a few kB: a page stays well under the client's answer cap).
pub const HOSTS_PAGE_SIZE: usize = 200;
/// The vulnerabilities read per poll: those on the most hosts.
pub const VULNERABILITIES: usize = 50;

#[derive(Clone, Debug, PartialEq)]
pub enum FleetError {
    /// HTTP 429.
    Throttled(String),
    /// HTTP 401 / 403: the token was refused (expired, or its user was removed) or its role can't read this.
    Auth(String),
    /// HTTP 404: not Fleet's API at this URL.
    NotFound(String),
    Other(String),
}

impl FleetError {
    pub fn message(&self) -> &str {
        match self {
            FleetError::Throttled(m) | FleetError::Auth(m) | FleetError::NotFound(m) | FleetError::Other(m) => m,
        }
    }
}

/// Fleet, as the poller and the connection test read it.
#[async_trait::async_trait]
pub trait FleetApi: Send + Sync {
    /// `GET {base}/api/v1/fleet{path}` with the token.
    async fn get(&self, base: &str, token: &str, path: &str) -> Result<Value, FleetError>;
}

/// The real API, through the SSRF guard.
pub struct Rest {
    pub allow_private: bool,
}

/// A failed request as a [`FleetError`], by its HTTP status, without the token.
pub fn classify(status: Option<u16>, message: &str, token: &str) -> FleetError {
    let message = if token.is_empty() { message.to_string() } else { message.replace(token, "[token]") };
    match status {
        Some(429) => FleetError::Throttled(message),
        Some(401) | Some(403) => FleetError::Auth(message),
        Some(404) => FleetError::NotFound(message),
        _ => FleetError::Other(message),
    }
}

#[async_trait::async_trait]
impl FleetApi for Rest {
    async fn get(&self, base: &str, token: &str, path: &str) -> Result<Value, FleetError> {
        let call = HttpCall::get(format!("{base}/api/v1/fleet{path}"), Auth::Bearer(token.to_string()));
        send_with_status(&call, self.allow_private).await.map_err(|(status, message)| classify(status, &message, token))
    }
}

/// The token's own user.
pub const ME: &str = "/me";
/// The hosts' counts by status and platform.
pub const HOST_SUMMARY: &str = "/host_summary";
/// The global policies, with their passing and failing host counts.
pub const GLOBAL_POLICIES: &str = "/global/policies";

/// A page (0-based) of the activities created from `start_ms` on (Fleet's `start_created_at`, to the second), by id:
/// Fleet's order of creation, so new activities only add to the last pages.
pub fn activities_path(page: usize, start_ms: i64) -> String {
    let start = chrono::DateTime::from_timestamp_millis(start_ms.div_euclid(1000) * 1000).unwrap_or_default().format("%Y-%m-%dT%H:%M:%SZ");
    format!("/activities?page={page}&per_page={ACTIVITIES_PAGE_SIZE}&order_key=id&order_direction=asc&start_created_at={start}")
}

/// A page (0-based) of the activities, newest first: how a Fleet too old for `start_created_at` is read.
pub fn activities_newest_path(page: usize) -> String {
    format!("/activities?page={page}&per_page={ACTIVITIES_PAGE_SIZE}&order_key=id&order_direction=desc")
}

/// The newest activity (the connection test: can the token read the activities).
pub const LATEST_ACTIVITY: &str = "/activities?page=0&per_page=1&order_key=id&order_direction=desc";

/// The vulnerabilities on the most hosts, with how many Fleet knows (`count`).
pub fn vulnerabilities_path() -> String {
    format!("/vulnerabilities?page=0&per_page={VULNERABILITIES}&order_key=hosts_count&order_direction=desc")
}

/// A page of hosts (0-based), by id so pages stay stable while hosts enroll.
pub fn hosts_path(page: usize) -> String {
    format!("/hosts?page={page}&per_page={HOSTS_PAGE_SIZE}&order_key=id&order_direction=asc")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_and_errors() {
        let start = chrono::DateTime::parse_from_rfc3339("2026-11-06T11:00:00.750Z").unwrap().timestamp_millis();
        assert_eq!(activities_path(2, start), "/activities?page=2&per_page=100&order_key=id&order_direction=asc&start_created_at=2026-11-06T11:00:00Z");
        assert_eq!(activities_newest_path(1), "/activities?page=1&per_page=100&order_key=id&order_direction=desc");
        assert_eq!(vulnerabilities_path(), "/vulnerabilities?page=0&per_page=50&order_key=hosts_count&order_direction=desc");
        assert_eq!(hosts_path(0), "/hosts?page=0&per_page=200&order_key=id&order_direction=asc");
        assert_eq!(classify(Some(401), "token fleet-secret refused", "fleet-secret"), FleetError::Auth("token [token] refused".into()));
        assert!(matches!(classify(Some(429), "", "t"), FleetError::Throttled(_)));
        assert!(matches!(classify(Some(404), "", "t"), FleetError::NotFound(_)));
        assert!(matches!(classify(None, "timeout", "t"), FleetError::Other(_)));
    }
}
