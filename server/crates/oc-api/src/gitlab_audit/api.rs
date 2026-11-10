//! GitLab's audit event API (`/api/v4`) as the GitLab Audit Events integration reads it: request paths (pure, tested
//! below). Requests go through the GitLab CI poller's client (`crate::gitlab::api::Rest`: the SSRF guard, the token
//! only in `PRIVATE-TOKEN`, scrubbed from errors).
//!
//! The poller asks only for first pages, newest first (keyset pagination orders audit events by creation time), and
//! moves `created_before` back itself, so it needs neither GitLab's `Link` header nor offset pages (deprecated for
//! these endpoints).

use crate::gitlab::api::enc;

/// Events per request (GitLab's maximum).
pub const PER_PAGE: usize = 100;

/// Whose audit events are read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Scope {
    /// Every event of the instance (`GET /audit_events`, administrators only).
    Instance,
    /// A group's events (`GET /groups/:id/audit_events`, the group's owners).
    Group(String),
    /// A project's events (`GET /projects/:id/audit_events`, its maintainers).
    Project(String),
}

impl Scope {
    /// From the stored `scope` and `scope_path`.
    pub fn parse(kind: &str, path: &str) -> Option<Scope> {
        match kind {
            "instance" => Some(Scope::Instance),
            "group" if !path.is_empty() => Some(Scope::Group(path.to_string())),
            "project" if !path.is_empty() => Some(Scope::Project(path.to_string())),
            _ => None,
        }
    }

    pub fn kind(&self) -> &'static str {
        match self {
            Scope::Instance => "instance",
            Scope::Group(_) => "group",
            Scope::Project(_) => "project",
        }
    }

    pub fn path(&self) -> &str {
        match self {
            Scope::Instance => "",
            Scope::Group(p) | Scope::Project(p) => p,
        }
    }

    fn base(&self) -> String {
        match self {
            Scope::Instance => "/audit_events".into(),
            Scope::Group(p) => format!("/groups/{}/audit_events", enc(p)),
            Scope::Project(p) => format!("/projects/{}/audit_events", enc(p)),
        }
    }
}

/// ISO 8601 (UTC, milliseconds) of epoch ms, as `created_after` / `created_before` take it.
pub fn iso_ms(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms).map(|d| d.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string()).unwrap_or_default()
}

/// The newest [`PER_PAGE`] events created from `after_ms` to `before_ms` (both included).
pub fn events_path(scope: &Scope, after_ms: i64, before_ms: i64) -> String {
    format!("{}?created_after={}&created_before={}&pagination=keyset&order_by=id&sort=desc&per_page={PER_PAGE}", scope.base(), iso_ms(after_ms), iso_ms(before_ms))
}

/// The newest event (the connection test: can the token read the scope's audit events).
pub fn latest_path(scope: &Scope) -> String {
    format!("{}?pagination=keyset&order_by=id&sort=desc&per_page=1", scope.base())
}

/// The token's own user (`is_admin` for administrators).
pub const USER: &str = "/user";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_by_scope() {
        let after = chrono::DateTime::parse_from_rfc3339("2026-11-06T11:00:00Z").unwrap().timestamp_millis();
        assert_eq!(
            events_path(&Scope::Instance, after, after + 60_250),
            "/audit_events?created_after=2026-11-06T11:00:00.000Z&created_before=2026-11-06T11:01:00.250Z&pagination=keyset&order_by=id&sort=desc&per_page=100"
        );
        assert!(events_path(&Scope::Group("example-org/platform".into()), after, after).starts_with("/groups/example-org%2Fplatform/audit_events?created_after="));
        assert_eq!(latest_path(&Scope::Project("example-org/storefront-api".into())), "/projects/example-org%2Fstorefront-api/audit_events?pagination=keyset&order_by=id&sort=desc&per_page=1");
        assert_eq!(Scope::parse("group", "example-org"), Some(Scope::Group("example-org".into())));
        assert_eq!((Scope::parse("group", ""), Scope::parse("other", "x")), (None, None));
        assert_eq!(Scope::parse("instance", "").unwrap().kind(), "instance");
    }
}
