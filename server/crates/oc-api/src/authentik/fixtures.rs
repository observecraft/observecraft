//! authentik API answers (fictional `example.com` users and apps) for the authentik integration's tests.

use std::collections::HashMap;
use std::sync::Mutex as StdMutex;

use serde_json::{json, Value};

use super::api::{AuthentikApi, AuthentikError};

/// An event as `GET /api/v3/events/events/` lists it.
pub fn event(pk: &str, action: &str, created: &str, context: Value) -> Value {
    json!({
        "pk": pk,
        "user": { "pk": 42, "username": "jordan.patel", "email": "jordan.patel@example.com" },
        "action": action,
        "app": "authentik.events.signals",
        "context": context,
        "client_ip": "203.0.113.24",
        "created": created,
        "expires": "2027-11-05T10:00:00.000000Z",
        "brand": { "pk": "fcba828076b94dedb2d5a6b4c5556fa1", "app": "authentik_brands", "name": "Example portal", "model_name": "brand" }
    })
}

/// A login with the request, geo and ASN context authentik records.
pub fn login(pk: &str, created: &str) -> Value {
    event(
        pk,
        "login",
        created,
        json!({
            "auth_method": "password",
            "http_request": { "args": {}, "path": "/api/v3/flows/executor/default-authentication-flow/", "method": "POST", "user_agent": "Mozilla/5.0" },
            "geo": { "city": "Bucharest", "country": "RO", "continent": "EU", "lat": 44.43, "long": 26.1 },
            "asn": { "asn": 64500, "as_org": "Example ISP", "network": "203.0.113.0/24" }
        }),
    )
}

/// A page of events (`pagination.next` is 0 on the last page, as authentik answers).
pub fn page(results: Vec<Value>, next: u64) -> Value {
    json!({ "pagination": { "next": next, "previous": 0, "count": results.len(), "current": 1, "total_pages": 1, "start_index": 1, "end_index": results.len() }, "results": results, "autocomplete": {} })
}

/// authentik answering from a table of paths; counts requests; `refuse` answers 403 to every request.
#[derive(Default)]
pub struct FakeAuthentik {
    pub answers: StdMutex<HashMap<String, Value>>,
    pub requests: StdMutex<Vec<String>>,
    pub refuse: bool,
}

impl FakeAuthentik {
    pub fn answer(&self, path: String, body: Value) {
        self.answers.lock().unwrap().insert(path, body);
    }

    pub fn requested(&self) -> Vec<String> {
        self.requests.lock().unwrap().clone()
    }
}

#[async_trait::async_trait]
impl AuthentikApi for FakeAuthentik {
    async fn get(&self, base: &str, token: &str, path: &str) -> Result<Value, AuthentikError> {
        assert_eq!((base, token), ("https://portal.example.com", "ak-test-token"));
        self.requests.lock().unwrap().push(path.to_string());
        if self.refuse {
            return Err(AuthentikError::Auth("You do not have permission to perform this action.".into()));
        }
        self.answers.lock().unwrap().get(path).cloned().ok_or_else(|| AuthentikError::NotFound(format!("no answer for {path}")))
    }
}
