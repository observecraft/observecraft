//! GitLab audit events (fictional `example-org` groups, projects and `example.com` people) and a GitLab answering
//! audit event requests from an event log, for the GitLab Audit Events integration's tests.

use std::sync::Mutex as StdMutex;

use serde_json::{json, Value};

use crate::gitlab::api::{GitlabApi, GitlabError};

/// An audit event as `GET /audit_events` lists it.
pub fn event(id: u64, event_name: &str, created_at: &str, details: Value) -> Value {
    let mut d = json!({
        "author_name": "Jordan Patel",
        "author_email": "jordan.patel@example.com",
        "author_class": "User",
        "target_id": 4211,
        "target_type": "Project",
        "target_details": "example-org/storefront-api",
        "ip_address": "203.0.113.24",
        "entity_path": "example-org/storefront-api",
        "event_name": event_name
    });
    if let (Some(d), Value::Object(extra)) = (d.as_object_mut(), details) {
        d.extend(extra);
    }
    json!({ "id": id, "author_id": 42, "entity_id": 4211, "entity_type": "Project", "event_name": event_name, "details": d, "created_at": created_at })
}

/// A project setting changed (`change`, `from`, `to`), as GitLab records it.
pub fn changed(id: u64, created_at: &str) -> Value {
    event(id, "project_visibility_level_updated", created_at, json!({ "change": "visibility_level", "from": "private", "to": "internal" }))
}

/// A failed login of `user` (no author: the user isn't signed in).
pub fn failed_login(id: u64, created_at: &str, user: &str) -> Value {
    json!({
        "id": id, "author_id": -1, "entity_id": -1, "entity_type": "User", "event_name": "login_failed_with_standard_authentication",
        "details": { "failed_login": "STANDARD", "author_name": user, "author_class": "User", "target_details": user, "ip_address": "198.51.100.7", "entity_path": user,
                     "custom_message": "Failed to login with STANDARD authentication" },
        "created_at": created_at
    })
}

fn param(path: &str, key: &str) -> Option<String> {
    let query = path.split_once('?')?.1;
    url::form_urlencoded::parse(query.as_bytes()).find(|(k, _)| k == key).map(|(_, v)| v.into_owned())
}

/// Microseconds: GitLab stores times finer than its answers' milliseconds.
fn us(t: &str) -> i64 {
    chrono::DateTime::parse_from_rfc3339(t).map(|t| t.timestamp_micros()).unwrap_or(0)
}

/// A stored time as GitLab answers it: to the millisecond.
fn answered(t: &str) -> String {
    chrono::DateTime::from_timestamp_millis(us(t).div_euclid(1000)).map(|d| d.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)).unwrap_or_default()
}

/// GitLab answering `…/audit_events` requests from `log` as GitLab filters them (created_after and created_before
/// included, compared with the stored microseconds) and orders them (newest first), with times to the millisecond,
/// and `/user` with `user`; records the requests.
/// `refuse` answers 403 to every request.
#[derive(Default)]
pub struct FakeAudit {
    pub log: StdMutex<Vec<Value>>,
    pub user: StdMutex<Option<Value>>,
    pub requests: StdMutex<Vec<String>>,
    pub refuse: bool,
}

impl FakeAudit {
    pub fn with(log: Vec<Value>) -> Self {
        FakeAudit { log: StdMutex::new(log), ..Default::default() }
    }

    pub fn requested(&self) -> Vec<String> {
        self.requests.lock().unwrap().clone()
    }
}

#[async_trait::async_trait]
impl GitlabApi for FakeAudit {
    async fn get(&self, base: &str, token: &str, path: &str) -> Result<Value, GitlabError> {
        assert_eq!((base, token), ("https://gitlab.example.com", "glpat-audit-test"));
        self.requests.lock().unwrap().push(path.to_string());
        if self.refuse {
            return Err(GitlabError::Auth("The provider answered: access is forbidden (403 Forbidden)".into()));
        }
        if path == super::api::USER {
            return self.user.lock().unwrap().clone().ok_or_else(|| GitlabError::NotFound("no user".into()));
        }
        if !path.contains("/audit_events?") {
            return Err(GitlabError::NotFound(format!("no answer for {path}")));
        }
        let after = param(path, "created_after").map_or(i64::MIN, |t| us(&t));
        let before = param(path, "created_before").map_or(i64::MAX, |t| us(&t));
        let per_page: usize = param(path, "per_page").and_then(|n| n.parse().ok()).unwrap_or(20);
        let mut events: Vec<Value> = self.log.lock().unwrap().iter().filter(|e| (after..=before).contains(&us(e["created_at"].as_str().unwrap()))).cloned().collect();
        events.sort_by_key(|e| std::cmp::Reverse((us(e["created_at"].as_str().unwrap()), e["id"].as_u64())));
        events.truncate(per_page);
        for e in &mut events {
            e["created_at"] = Value::String(answered(e["created_at"].as_str().unwrap()));
        }
        Ok(Value::Array(events))
    }
}
