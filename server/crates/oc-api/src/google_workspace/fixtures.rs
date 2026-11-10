//! Reports API activities (fictional `example.com` people, `example-org` documents and apps), a service account key made
//! for these tests only (it was never registered with Google), and a Reports API answering activity lists from an
//! activity log, for the Google Workspace integration's tests.

use std::collections::HashMap;
use std::sync::Mutex as StdMutex;

use serde_json::{json, Value};

use super::api::{Credentials, WorkspaceApi, WorkspaceError};

/// The test key's PKCS#8 body (2048-bit RSA, generated for these tests; no Google project knows it).
const TEST_KEY_BODY: &str = "MIIEvgIBADANBgkqhkiG9w0BAQEFAASCBKgwggSkAgEAAoIBAQC1dUka8XQlGYMPic8w3AKiKHga5gCvWt9Yi1QeghxFL7IlO6Cm6A9np8a9j7e+qbI1dYqOK4h6DOd5+cQDXVGjyURmbzAAM+r57/2atjB1bHa6zmbkv2idD8LQQqKJbMYwNvTYEP/yOA8FR3awYHj23V2SizsXfV0VNOaFDv25hBpTcapTYGOmbaQ0B7alaoeE7nxZh5JSz7WlhX23I2jMAwpTkD254YiylTHXSmZ7puLf4SQ9QECuKyxP4p6noD+MrFcEzu3vt7u+f7aDzv1Uyqpuqhv+SBnVD3k4WjlJYBzewnarM1NRH0fdvyPLwh+UJcglErQ0WAh7e+Jc7sgNAgMBAAECggEAKFv4URpQI2pLuLqFj0/38/o9UHpEv7vd/lMq+AgnYooLvPVIs6bcukjaTtI245zZysMkeJ+dIMxa9cnEO51Xaa773C6xxfHGOyYSSYVpPMzNkZoTz5/1FC3rvid8AEXlxNaMWbxaORUX3iw5bVxfu0ScKdp237Ad8ngMJ58gdtDW6QYrLEA8nMqAjslH488Cd+JIT61sqXbgf+eSi1MwaoORrtrUkSnoQuWat6qFuO17FFwZWBRxZOzT9yeLBNC31WfXJ43RKL4gr35Q6vBRvgS417iNRATseoaLmMLByE333pGwRAX+8WlA2cKIqVeYk0rPGJYLaRgZZOkTv4xujwKBgQD0cnNV8pUDavq1xO4eA1HGmrAMYy536VOd8D9z9GWge8hUUMhyQ96Gcm6SQWUn5AouVpHxI0eDmtIeAQsSVQVCxA9lgTLuwxluae+96CTfMTYjmIgs960g2/NwpRZn/GzWLAN51iqr/deYsZzomkHUm0k1EOvf3Q6OOBDJAeODEwKBgQC+CLxufHc8ffTIUXmQQRnGr6jjnLawFy1lRttWhPzIpL2MWpoAe3lhp1MAeSH1x2eqkotLgpzjF4qaBuEiXJplf3rNAyPbTUMVaL3fUEX0nYIxogbDxKDh41chFA/rajs8SrIm/vxuQe7/bJBXPi0//R0gpKmQm9MWwQRmQnrMXwKBgQCwDT3DyNJJg+LMxug39WwP/ZiNU2FEO2ddeIo6mOoYiw3cq9Gxee/qJMA91Ztk/3fVp8O0SBRZVEfAFEBFE7aWP15XXJWyIBFQ1Ds2KttjY636NQDeYgLuh3rsxfk9Ky+3kDtwpC5XcJq1/jVj0LVYyf4PmgBHWo4BH5/3JMUWwQKBgQCg+4mVFz9zMUiSAeM3Ps71L42k8/cXCStdpj7c9l+mIVipc38OdYsIHNUFi5gnOzsolmgGGamJ/0Fc5gMGNuhcnzeKQOS5PKRdmJNwf/IjHONT5GxSn1miwhHnZbVuullf4+8zdcKmLJioLHHCVlvC30szggpbBIY3ul9duWeJhQKBgHNhmGFwM4phAKDwVzHYcxJLWxsJCnLCpr2ZUJznZzLMvo7LHx2QiNNecK64C8+o6jl+BJx4+SLv8eIZcwiQJJdFpb4vyNI85LwkxXdtepxzbKojqPOgI6znF2SlWQofH9wO7tEAK0YfVRkawgNySiZyR8nY5hNDG46dqsfwto+c";

/// A service account JSON key file, as the Google Cloud console downloads it.
pub fn key_json() -> String {
    let body: Vec<String> = TEST_KEY_BODY.as_bytes().chunks(64).map(|c| String::from_utf8_lossy(c).into_owned()).collect();
    let pem = format!("-----BEGIN {k}-----\n{}\n-----END {k}-----\n", body.join("\n"), k = "PRIVATE KEY");
    json!({
        "type": "service_account",
        "project_id": "example-project",
        "private_key_id": "0123456789abcdef",
        "private_key": pem,
        "client_email": "observecraft-reports@example-project.iam.gserviceaccount.com",
        "client_id": "100000000000000000001",
        "auth_uri": "https://accounts.google.com/o/oauth2/auth",
        "token_uri": "https://oauth2.googleapis.com/token"
    })
    .to_string()
}

pub fn credentials() -> Credentials {
    Credentials { key: super::api::parse_key(&key_json()).unwrap(), subject: "admin@example.com".into() }
}

/// A parameter as the Reports API lists it: strings as `value`, integers as `intValue` (a string), booleans as
/// `boolValue`, lists as `multiValue`.
pub fn param(name: &str, v: Value) -> Value {
    match v {
        Value::Bool(b) => json!({ "name": name, "boolValue": b }),
        Value::Number(n) => json!({ "name": name, "intValue": n.to_string() }),
        Value::Array(a) => json!({ "name": name, "multiValue": a }),
        Value::String(s) => json!({ "name": name, "value": s }),
        other => json!({ "name": name, "messageValue": { "parameter": other } }),
    }
}

/// An activity of `application` by `actor` with one event (`type`, `name`, `(parameter, value)`s).
pub fn activity(application: &str, unique: &str, time: &str, actor: &str, event_type: &str, name: &str, params: &[(&str, Value)]) -> Value {
    json!({
        "kind": "admin#reports#activity",
        "id": { "time": time, "uniqueQualifier": unique, "applicationName": application, "customerId": "C01example" },
        "etag": "\"etag-1\"",
        "actor": { "email": actor, "profileId": "104857600000000000042", "callerType": "USER" },
        "ipAddress": "203.0.113.24",
        "networkInfo": { "regionCode": "US", "subdivisionCode": "US-CA", "ipAsn": [64500] },
        "events": [{ "type": event_type, "name": name, "parameters": params.iter().map(|(k, v)| param(k, v.clone())).collect::<Vec<_>>() }]
    })
}

pub fn login_success(unique: &str, time: &str) -> Value {
    activity(
        "login",
        unique,
        time,
        "jordan.patel@example.com",
        "login",
        "login_success",
        &[("login_type", json!("google_password")), ("login_challenge_method", json!(["password", "google_authenticator"])), ("is_suspicious", json!(false))],
    )
}

pub fn login_failure(unique: &str, time: &str, user: &str) -> Value {
    let mut a = activity(
        "login",
        unique,
        time,
        user,
        "login",
        "login_failure",
        &[("login_type", json!("google_password")), ("login_failure_type", json!("login_failure_invalid_password")), ("is_suspicious", json!(false))],
    );
    a["ipAddress"] = json!("198.51.100.7");
    a["networkInfo"] = json!({ "regionCode": "NL", "ipAsn": [64501] });
    a
}

pub fn suspicious_login(unique: &str, time: &str) -> Value {
    activity("login", unique, time, "sam.lee@example.com", "login", "suspicious_login", &[("login_type", json!("google_password")), ("is_suspicious", json!(true))])
}

/// An administrator granted a role (the Admin console's `ASSIGN_ROLE`).
pub fn role_assigned(unique: &str, time: &str) -> Value {
    activity("admin", unique, time, "admin@example.com", "DELEGATED_ADMIN_SETTINGS", "ASSIGN_ROLE", &[("ROLE_NAME", json!("_SEED_ADMIN_ROLE")), ("USER_EMAIL", json!("riley.chen@example.com"))])
}

/// A Drive file made visible to anyone with the link (or public on the web).
pub fn shared_publicly(unique: &str, time: &str, visibility: &str) -> Value {
    activity(
        "drive",
        unique,
        time,
        "alex.morgan@example.com",
        "acl_change",
        "change_document_visibility",
        &[
            ("doc_id", json!("1AbCdEfGhIjKlMnOpQrStUvWxYz")),
            ("doc_title", json!("Q3 storefront roadmap")),
            ("doc_type", json!("document")),
            ("owner", json!("alex.morgan@example.com")),
            ("visibility", json!(visibility)),
            ("old_value", json!(["private"])),
            ("new_value", json!([visibility])),
            ("visibility_change", json!("external")),
        ],
    )
}

/// An OAuth app authorized (`token`'s `authorize`).
pub fn token_authorized(unique: &str, time: &str) -> Value {
    activity(
        "token",
        unique,
        time,
        "casey.nguyen@example.com",
        "auth",
        "authorize",
        &[("app_name", json!("Example CRM")), ("client_id", json!("123456.apps.googleusercontent.com")), ("scope", json!(["https://www.googleapis.com/auth/drive.readonly"]))],
    )
}

fn param_of(path: &str, key: &str) -> Option<String> {
    let query = path.split_once('?')?.1;
    url::form_urlencoded::parse(query.as_bytes()).find(|(k, _)| k == key).map(|(_, v)| v.into_owned())
}

fn ms(t: &str) -> i64 {
    chrono::DateTime::parse_from_rfc3339(t).map(|t| t.timestamp_millis()).unwrap_or(0)
}

/// The Reports API answering `activity/users/all/applications/{app}` from `log` as Google does: activities from
/// `startTime` to `endTime` (both included), newest first, `maxResults` per page with an opaque `nextPageToken`;
/// records the requests. `refuse` answers every request as a key without domain-wide delegation.
#[derive(Default)]
pub struct FakeReports {
    pub log: StdMutex<Vec<Value>>,
    pub requests: StdMutex<Vec<String>>,
    pub refuse: bool,
    pub tokens: StdMutex<HashMap<String, usize>>,
}

impl FakeReports {
    pub fn with(log: Vec<Value>) -> Self {
        FakeReports { log: StdMutex::new(log), ..Default::default() }
    }

    pub fn requested(&self) -> Vec<String> {
        self.requests.lock().unwrap().clone()
    }

    /// Activities arriving late (or new): added to the log.
    pub fn add(&self, more: Vec<Value>) {
        self.log.lock().unwrap().extend(more);
    }
}

#[async_trait::async_trait]
impl WorkspaceApi for FakeReports {
    async fn get(&self, creds: &Credentials, path: &str) -> Result<Value, WorkspaceError> {
        assert_eq!(creds.subject, "admin@example.com");
        self.requests.lock().unwrap().push(path.to_string());
        if self.refuse {
            return Err(WorkspaceError::Denied("Google refused domain-wide delegation".into()));
        }
        let Some(app) = path.strip_prefix("/admin/reports/v1/activity/users/all/applications/").and_then(|p| p.split('?').next()) else {
            return Err(WorkspaceError::Other(format!("no answer for {path}")));
        };
        let start = param_of(path, "startTime").map_or(i64::MIN, |t| ms(&t));
        let end = param_of(path, "endTime").map_or(i64::MAX, |t| ms(&t));
        let max: usize = param_of(path, "maxResults").and_then(|n| n.parse().ok()).unwrap_or(1000);
        let offset = param_of(path, "pageToken").map(|t| *self.tokens.lock().unwrap().get(&t).expect("a token this fake gave")).unwrap_or(0);
        let mut items: Vec<Value> = self.log.lock().unwrap().iter().filter(|a| a["id"]["applicationName"] == app && (start..=end).contains(&ms(a["id"]["time"].as_str().unwrap()))).cloned().collect();
        items.sort_by_key(|a| std::cmp::Reverse((ms(a["id"]["time"].as_str().unwrap()), a["id"]["uniqueQualifier"].as_str().unwrap().to_string())));
        let page: Vec<Value> = items.iter().skip(offset).take(max).cloned().collect();
        let mut answer = json!({ "kind": "admin#reports#activities", "etag": "\"etag\"" });
        if !page.is_empty() {
            answer["items"] = Value::Array(page);
        }
        if offset + max < items.len() {
            let token = format!("tok-{}-{}", path.len(), offset + max);
            self.tokens.lock().unwrap().insert(token.clone(), offset + max);
            answer["nextPageToken"] = json!(token);
        }
        Ok(answer)
    }
}
