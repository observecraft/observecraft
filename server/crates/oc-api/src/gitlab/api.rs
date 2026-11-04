//! GitLab's REST API (v4) as the GitLab CI poller reads it: request paths (pure, tested below) and
//! [`Rest`], which sends them through the Developer Portal connectors' client
//! (`routes::portal_sources::send_with_status`: the synthetic tests' SSRF guard, `OC_PORTAL_ALLOW_PRIVATE`,
//! no redirects, capped answers, timeouts). The token goes only into the `PRIVATE-TOKEN` header and is
//! scrubbed from every error.

use serde_json::Value;

use crate::routes::portal_sources::{send_with_status, Auth, HttpCall};

/// Items per page of every list (GitLab's maximum).
pub const PER_PAGE: usize = 100;
const MAX_PATH_CHARS: usize = 255;

#[derive(Clone, Debug, PartialEq)]
pub enum GitlabError {
    /// HTTP 429: GitLab's rate limit.
    Throttled(String),
    /// HTTP 401 / 403: the token was refused or lacks `read_api`.
    Auth(String),
    /// HTTP 404: no such project or group (or the token can't see it).
    NotFound(String),
    Other(String),
}

impl GitlabError {
    pub fn message(&self) -> &str {
        match self {
            GitlabError::Throttled(m) | GitlabError::Auth(m) | GitlabError::NotFound(m) | GitlabError::Other(m) => m,
        }
    }
}

/// GitLab, as the poller and the connection test read it.
#[async_trait::async_trait]
pub trait GitlabApi: Send + Sync {
    /// `GET {base}/api/v4{path}` with the token.
    async fn get(&self, base: &str, token: &str, path: &str) -> Result<Value, GitlabError>;
}

/// The real API, through the SSRF guard.
pub struct Rest {
    pub allow_private: bool,
}

/// An error message without the token (in case GitLab or a proxy echoed it).
pub fn scrub(message: &str, token: &str) -> String {
    if token.is_empty() {
        message.to_string()
    } else {
        message.replace(token, "[token]")
    }
}

/// A failed request as a [`GitlabError`], by its HTTP status.
pub fn classify(status: Option<u16>, message: &str, token: &str) -> GitlabError {
    let message = scrub(message, token);
    match status {
        Some(429) => GitlabError::Throttled(message),
        Some(401) | Some(403) => GitlabError::Auth(message),
        Some(404) => GitlabError::NotFound(message),
        _ => GitlabError::Other(message),
    }
}

#[async_trait::async_trait]
impl GitlabApi for Rest {
    async fn get(&self, base: &str, token: &str, path: &str) -> Result<Value, GitlabError> {
        let call = HttpCall::get(format!("{base}/api/v4{path}"), Auth::GitlabToken(token.to_string()));
        send_with_status(&call, self.allow_private).await.map_err(|(status, message)| classify(status, &message, token))
    }
}

/// Validates a group or project path (`example-org`, `example-org/platform/storefront-api`): GitLab's
/// path characters, segments separated by `/`, no `..`. Surrounding slashes and spaces are dropped.
pub fn check_path(raw: &str) -> Result<String, String> {
    let p = raw.trim().trim_matches('/');
    if p.is_empty() {
        return Err("A path is empty".into());
    }
    let ok = p.len() <= MAX_PATH_CHARS
        && p.split('/').all(|seg| !seg.is_empty() && !seg.starts_with('.') && seg.chars().all(|c| c.is_ascii_alphanumeric() || "_.-".contains(c)));
    if !ok {
        return Err(format!("{p} is not a GitLab group or project path (like example-org/storefront-api)"));
    }
    Ok(p.to_string())
}

/// A path or id as one URL path segment (`/` encoded).
fn enc(s: &str) -> String {
    url::form_urlencoded::byte_serialize(s.as_bytes()).collect()
}

/// ISO 8601 (UTC) of epoch ms, as GitLab's `updated_after` takes it.
pub fn iso(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms).map(|d| d.format("%Y-%m-%dT%H:%M:%SZ").to_string()).unwrap_or_default()
}

pub fn user_path() -> String {
    "/user".into()
}

/// The token's own record (its scopes); personal, group and project access tokens.
pub fn token_path() -> String {
    "/personal_access_tokens/self".into()
}

pub fn group_projects_path(group: &str, include_subgroups: bool, page: usize) -> String {
    format!("/groups/{}/projects?include_subgroups={include_subgroups}&archived=false&simple=true&order_by=id&sort=asc&per_page={PER_PAGE}&page={page}", enc(group))
}

/// A project by path or numeric id.
pub fn project_path(project: &str) -> String {
    format!("/projects/{}", enc(project))
}

/// Pipelines updated after `after_ms`, oldest update first.
pub fn pipelines_path(project_id: u64, after_ms: i64, page: usize) -> String {
    format!("/projects/{project_id}/pipelines?updated_after={}&order_by=updated_at&sort=asc&per_page={PER_PAGE}&page={page}", iso(after_ms))
}

pub fn pipeline_path(project_id: u64, pipeline_id: u64) -> String {
    format!("/projects/{project_id}/pipelines/{pipeline_id}")
}

pub fn jobs_path(project_id: u64, pipeline_id: u64, page: usize) -> String {
    format!("/projects/{project_id}/pipelines/{pipeline_id}/jobs?per_page={PER_PAGE}&page={page}")
}

/// Deployments updated after `after_ms`: which environment each deploy job went to.
pub fn deployments_path(project_id: u64, after_ms: i64, page: usize) -> String {
    format!("/projects/{project_id}/deployments?updated_after={}&order_by=updated_at&sort=asc&per_page={PER_PAGE}&page={page}", iso(after_ms))
}

/// A commit by its SHA (hex only; anything else is refused).
pub fn commit_path(project_id: u64, sha: &str) -> Option<String> {
    (!sha.is_empty() && sha.len() <= 64 && sha.chars().all(|c| c.is_ascii_hexdigit())).then(|| format!("/projects/{project_id}/repository/commits/{sha}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_are_validated() {
        assert_eq!(check_path(" /example-org/platform/storefront-api/ ").unwrap(), "example-org/platform/storefront-api");
        assert_eq!(check_path("example_org.v2").unwrap(), "example_org.v2");
        for bad in ["", "/", "example-org//api", "example-org/../admin", "example org", "example-org/api?x=1", "a/.hidden", "grp/%2e%2e"] {
            assert!(check_path(bad).is_err(), "{bad}");
        }
        assert!(check_path(&"a".repeat(256)).is_err());
    }

    #[test]
    fn requests_encode_paths_and_page() {
        assert_eq!(project_path("example-org/storefront-api"), "/projects/example-org%2Fstorefront-api");
        assert_eq!(
            group_projects_path("example-org/platform", true, 2),
            "/groups/example-org%2Fplatform/projects?include_subgroups=true&archived=false&simple=true&order_by=id&sort=asc&per_page=100&page=2"
        );
        assert_eq!(pipelines_path(4211, 1_793_700_000_000, 1), "/projects/4211/pipelines?updated_after=2026-11-03T10:00:00Z&order_by=updated_at&sort=asc&per_page=100&page=1");
        assert_eq!(deployments_path(4211, 1_793_700_000_000, 3), "/projects/4211/deployments?updated_after=2026-11-03T10:00:00Z&order_by=updated_at&sort=asc&per_page=100&page=3");
        assert_eq!(jobs_path(4211, 55_001, 1), "/projects/4211/pipelines/55001/jobs?per_page=100&page=1");
        assert_eq!(commit_path(4211, "9fceb02d").as_deref(), Some("/projects/4211/repository/commits/9fceb02d"));
        assert_eq!(commit_path(4211, "../../user"), None);
    }

    #[test]
    fn errors_are_classified_and_never_carry_the_token() {
        let token = "glpat-AbCdEf123456";
        let e = classify(Some(401), &format!("The provider answered: the credentials were refused (token {token} is invalid)"), token);
        assert!(matches!(e, GitlabError::Auth(_)));
        assert!(!e.message().contains(token), "{}", e.message());
        assert!(matches!(classify(Some(429), "slow down", token), GitlabError::Throttled(_)));
        assert!(matches!(classify(Some(403), "forbidden", token), GitlabError::Auth(_)));
        assert!(matches!(classify(Some(404), "missing", token), GitlabError::NotFound(_)));
        assert!(matches!(classify(None, "connection refused", token), GitlabError::Other(_)));
    }

    #[tokio::test]
    async fn rest_sends_the_token_header_only_to_allowed_hosts() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.unwrap();
            let mut buf = vec![0u8; 16 * 1024];
            let n = sock.read(&mut buf).await.unwrap();
            let body = r#"{"message":"429 Too Many Requests"}"#;
            sock.write_all(format!("HTTP/1.1 429 X\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
            String::from_utf8_lossy(&buf[..n]).to_ascii_lowercase()
        });
        let base = format!("http://{addr}");
        let err = Rest { allow_private: true }.get(&base, "glpat-secret", "/user").await.unwrap_err();
        assert!(matches!(err, GitlabError::Throttled(_)), "{err:?}");
        let head = server.await.unwrap();
        assert!(head.starts_with("get /api/v4/user "));
        assert!(head.contains("private-token: glpat-secret"));
        // Loopback is refused unless the operator allows private targets.
        let err = Rest { allow_private: false }.get(&base.replace("http://", "https://"), "glpat-secret", "/user").await.unwrap_err();
        assert!(matches!(err, GitlabError::Other(_)) && err.message().contains("loopback"), "{err:?}");
    }
}
