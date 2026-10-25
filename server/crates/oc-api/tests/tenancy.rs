//! Organization isolation, end to end against a running server.
//!
//! Skipped unless `OC_TEST_API_URL` is set, e.g.
//! `OC_TEST_API_URL=http://localhost:8080 OC_TEST_ADMIN_EMAIL=… OC_TEST_ADMIN_PASSWORD=… cargo test -p oc-api --test tenancy`.
//! The admin must be an instance admin. The test creates a throwaway org, checks that content and
//! membership never cross organizations, and deletes the org again.

use reqwest::header::{COOKIE, ORIGIN, SET_COOKIE};
use reqwest::{Client, Method, StatusCode};
use serde_json::{json, Value};

struct Session {
    base: String,
    http: Client,
    cookie: String,
}

impl Session {
    async fn login(base: &str, email: &str, password: &str) -> Session {
        let http = Client::new();
        let res = http.post(format!("{base}/api/v1/auth/login")).json(&json!({ "email": email, "password": password })).send().await.expect("login request");
        assert_eq!(res.status(), StatusCode::OK, "login failed: {}", res.text().await.unwrap_or_default());
        let cookie = cookie_of(&res);
        Session { base: base.into(), http, cookie }
    }

    async fn call(&mut self, method: Method, path: &str, body: Option<Value>) -> (StatusCode, Value) {
        let mut req = self.http.request(method, format!("{}{path}", self.base)).header(COOKIE, &self.cookie).header(ORIGIN, "http://localhost:5173");
        if let Some(b) = body {
            req = req.json(&b);
        }
        let res = req.send().await.expect("request");
        if res.headers().contains_key(SET_COOKIE) {
            self.cookie = cookie_of(&res);
        }
        let status = res.status();
        (status, res.json().await.unwrap_or(Value::Null))
    }
}

fn cookie_of(res: &reqwest::Response) -> String {
    let raw = res.headers().get(SET_COOKIE).and_then(|v| v.to_str().ok()).expect("session cookie");
    raw.split(';').next().unwrap_or_default().to_string()
}

/// Incident search over the last day.
fn recent() -> Value {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as i64;
    json!({ "range": { "from": now - 86_400_000, "to": now + 60_000 }, "offset": 0, "limit": 200 })
}

async fn monitor_names(s: &mut Session) -> Vec<String> {
    let (st, body) = s.call(Method::POST, "/api/v1/monitors/search", Some(json!({}))).await;
    assert_eq!(st, StatusCode::OK);
    body["monitors"].as_array().unwrap().iter().map(|m| m["name"].as_str().unwrap_or_default().to_string()).collect()
}

#[tokio::test]
async fn organizations_are_isolated() {
    let Ok(base) = std::env::var("OC_TEST_API_URL") else {
        eprintln!("OC_TEST_API_URL not set; skipping");
        return;
    };
    let email = std::env::var("OC_TEST_ADMIN_EMAIL").unwrap_or_else(|_| "alex.morgan@example.com".into());
    let password = std::env::var("OC_TEST_ADMIN_PASSWORD").expect("OC_TEST_ADMIN_PASSWORD");
    let mut admin = Session::login(&base, &email, &password).await;
    let (_, me) = admin.call(Method::GET, "/api/v1/me", None).await;
    let home = me["org"]["id"].as_str().unwrap().to_string();

    // A throwaway org whose first admin is the test account itself (joins with its current password).
    let org = format!("tenancy-test-{}", std::process::id());
    let (st, created) = admin.call(Method::POST, "/api/v1/instance/orgs", Some(json!({ "id": org, "name": "Tenancy test", "adminEmail": email }))).await;
    assert_eq!(st, StatusCode::OK, "{created}");
    let token = created["inviteUrl"].as_str().unwrap().rsplit('/').next().unwrap().to_string();
    let (st, link) = admin.call(Method::GET, &format!("/api/v1/auth/link/{token}"), None).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(link["existingAccount"], true);
    let (st, _) = admin.call(Method::POST, &format!("/api/v1/auth/link/{token}"), Some(json!({ "password": "not-the-password" }))).await;
    assert_eq!(st, StatusCode::BAD_REQUEST, "a wrong password must not join the org");
    let (st, _) = admin.call(Method::POST, &format!("/api/v1/auth/link/{token}"), Some(json!({ "password": password }))).await;
    assert_eq!(st, StatusCode::OK);

    // The accepted link signed us in to the new org: it starts empty apart from built-ins.
    let (_, me) = admin.call(Method::GET, "/api/v1/me", None).await;
    assert_eq!(me["org"]["id"], org.as_str());
    assert!(monitor_names(&mut admin).await.is_empty());
    let (st, _) = admin
        .call(Method::POST, "/api/v1/incidents", Some(json!({ "title": "tenancy probe incident", "severity": "SEV-5", "customerImpact": false, "test": true })))
        .await;
    assert!(st.is_success(), "declare incident: {st}");
    let (_, own) = admin.call(Method::POST, "/api/v1/incidents/search", Some(recent())).await;
    assert!(own.to_string().contains("tenancy probe incident"), "the org sees its own incident");

    // Back home: the probe incident is not visible there.
    let (st, _) = admin.call(Method::POST, "/api/v1/me/switch-org", Some(json!({ "org": home }))).await;
    assert_eq!(st, StatusCode::NO_CONTENT);
    let (st, incidents) = admin.call(Method::POST, "/api/v1/incidents/search", Some(recent())).await;
    assert_eq!(st, StatusCode::OK, "{incidents}");
    let leaked = incidents.to_string().contains("tenancy probe incident");
    assert!(!leaked, "an incident from another org is visible");

    // Not a member: switching is refused.
    let (st, _) = admin.call(Method::POST, "/api/v1/me/switch-org", Some(json!({ "org": "no-such-org" }))).await;
    assert_eq!(st, StatusCode::FORBIDDEN);

    // Clean up.
    let (st, _) = admin.call(Method::DELETE, &format!("/api/v1/instance/orgs/{org}"), Some(json!({ "confirm": org }))).await;
    assert_eq!(st, StatusCode::NO_CONTENT);
    let (_, orgs) = admin.call(Method::GET, "/api/v1/me/orgs", None).await;
    assert!(!orgs.to_string().contains(&org));
}
