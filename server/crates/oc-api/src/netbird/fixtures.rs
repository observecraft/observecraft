//! NetBird API answers (fictional `example.com` users and peers) for the NetBird integration's tests.

use std::collections::HashMap;
use std::sync::Mutex as StdMutex;

use serde_json::{json, Value};

use super::api::{NetbirdApi, NetbirdError};

/// An audit event as `GET /api/events/audit` lists it.
pub fn event(id: u64, code: &str, activity: &str, timestamp: &str, meta: Value) -> Value {
    json!({
        "id": id,
        "timestamp": timestamp,
        "activity": activity,
        "activity_code": code,
        "initiator_id": "google-oauth2|1001",
        "initiator_name": "Jordan Patel",
        "initiator_email": "jordan.patel@example.com",
        "target_id": "cs1tnh0hhcjnqoiuebf0",
        "meta": meta
    })
}

/// A peer added by a user, with the metadata NetBird records.
pub fn peer_added(id: u64, timestamp: &str) -> Value {
    event(id, "peer.user.add", "Peer added", timestamp, json!({ "name": "laptop-jordan", "fqdn": "laptop-jordan.netbird.cloud", "ip": "100.64.0.12", "created_at": timestamp }))
}

/// Three peers: a connected laptop, a disconnected CI runner whose login expired, a server with SSH enabled.
pub fn peers() -> Vec<Value> {
    vec![
        json!({
            "id": "cs1tnh0hhcjnqoiuebf0", "name": "laptop-jordan", "hostname": "jordans-mbp", "ip": "100.64.0.12", "connected": true,
            "last_seen": "2026-11-06T11:59:58Z", "os": "Darwin 15.1", "version": "0.59.1", "login_expired": false, "approval_required": false,
            "ssh_enabled": false, "country_code": "RO", "city_name": "Bucharest", "accessible_peers_count": 2
        }),
        json!({
            "id": "cs1tnh0hhcjnqoiuebf1", "name": "build-runner-1", "hostname": "build-runner-1", "ip": "100.64.0.20", "connected": false,
            "last_seen": "2026-11-06T10:00:00Z", "os": "Ubuntu 24.04", "version": "0.58.0", "login_expired": true, "approval_required": false,
            "ssh_enabled": false, "country_code": "", "city_name": "", "accessible_peers_count": 1
        }),
        json!({
            "id": "cs1tnh0hhcjnqoiuebf2", "name": "i-0a1b2c3d4e5f60001", "hostname": "i-0a1b2c3d4e5f60001", "ip": "100.64.0.30", "connected": true,
            "last_seen": "2026-11-06T11:59:59Z", "os": "Amazon Linux 2023", "version": "0.59.1", "login_expired": false, "approval_required": false,
            "ssh_enabled": true, "country_code": "US", "city_name": "Ashburn", "accessible_peers_count": 2
        }),
    ]
}

/// NetBird answering from a table of paths; counts requests; `refuse` answers 401 to every request, `forbid` 403
/// to the audit log only (a user whose role can't read it).
#[derive(Default)]
pub struct FakeNetbird {
    pub answers: StdMutex<HashMap<String, Value>>,
    pub requests: StdMutex<Vec<String>>,
    pub refuse: bool,
    pub forbid_events: bool,
}

impl FakeNetbird {
    pub fn answer(&self, path: &str, body: Value) {
        self.answers.lock().unwrap().insert(path.to_string(), body);
    }

    pub fn requested(&self) -> Vec<String> {
        self.requests.lock().unwrap().clone()
    }
}

#[async_trait::async_trait]
impl NetbirdApi for FakeNetbird {
    async fn get(&self, base: &str, token: &str, path: &str) -> Result<Value, NetbirdError> {
        assert_eq!((base, token), ("https://netbird.example.com", "nbp_test_token"));
        self.requests.lock().unwrap().push(path.to_string());
        if self.refuse {
            return Err(NetbirdError::Auth("token invalid".into()));
        }
        if self.forbid_events && path == super::api::AUDIT_EVENTS {
            return Err(NetbirdError::Auth("permission denied".into()));
        }
        self.answers.lock().unwrap().get(path).cloned().ok_or_else(|| NetbirdError::NotFound(format!("no answer for {path}")))
    }
}
