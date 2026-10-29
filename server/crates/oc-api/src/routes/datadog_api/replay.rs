//! Contract tests against Datadog's own recordings.
//!
//! `tests/datadog_api/` holds scenarios (`features/`) and their recorded request/response pairs
//! (`cassettes/`) from `datadog-api-client-go` (Apache-2.0, see the README there). [`Replayer`] sends
//! each recorded request to an in-process server ([`TestServer`]) with an `ocak_` test key and checks
//! the answer has the recorded status class and the recorded fields with the same JSON types. Ids,
//! timestamps and names the server generates differ from the recording, so values are not compared;
//! ids from earlier answers are mapped onto later requests instead (create → get → delete).
//!
//! Live: needs `OC_TEST_POSTGRES_URL` and `OC_TEST_CLICKHOUSE_URL` (skipped otherwise), like the other
//! live tests (`crate::test_support`).

use std::collections::{BTreeMap, HashMap};
use std::net::SocketAddr;
use std::path::PathBuf;

use serde::Deserialize;
use serde_json::Value;

use crate::auth::crypto;
use crate::state::AppState;

/// The vendored fixtures.
pub fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/datadog_api")
}

// ================================================================ features

/// A `.feature` file: its scenarios and the steps every scenario starts with.
#[derive(Debug)]
pub struct Feature {
    pub name: String,
    pub version: String,
    pub background: Vec<String>,
    pub scenarios: Vec<Scenario>,
}

#[derive(Debug, Clone)]
pub struct Scenario {
    pub name: String,
    pub steps: Vec<String>,
}

/// Which keys the scenario's client sends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sends {
    pub api_key: bool,
    pub app_key: bool,
}

impl Feature {
    /// `version/name`, e.g. `v1/authentication` for `features/v1/authentication.feature`.
    pub fn load(path: &str) -> Feature {
        let file = fixtures().join("features").join(format!("{path}.feature"));
        let text = std::fs::read_to_string(&file).unwrap_or_else(|e| panic!("{}: {e}", file.display()));
        Self::parse(path.split('/').next().unwrap_or("v1"), &text)
    }

    pub fn parse(version: &str, text: &str) -> Feature {
        let mut feature = Feature { name: String::new(), version: version.into(), background: vec![], scenarios: vec![] };
        let mut in_background = false;
        for line in text.lines().map(str::trim) {
            if let Some(name) = line.strip_prefix("Feature:") {
                feature.name = name.trim().into();
            } else if line.starts_with("Background:") {
                in_background = true;
            } else if let Some(name) = line.strip_prefix("Scenario:") {
                in_background = false;
                feature.scenarios.push(Scenario { name: name.trim().into(), steps: vec![] });
            } else if let Some(step) = ["Given ", "And ", "When ", "Then ", "But "].iter().find_map(|k| line.strip_prefix(k)) {
                match feature.scenarios.last_mut() {
                    Some(s) if !in_background => s.steps.push(step.into()),
                    _ => feature.background.push(step.into()),
                }
            }
        }
        feature
    }

    /// The cassette directory, e.g. `v1/Feature_Service_Level_Objective_Corrections`.
    pub fn cassette_dir(&self) -> PathBuf {
        fixtures().join("cassettes/TestScenarios").join(&self.version).join(format!("Feature_{}", self.name.replace(' ', "_")))
    }

    pub fn scenario(&self, name: &str) -> &Scenario {
        self.scenarios.iter().find(|s| s.name == name).unwrap_or_else(|| panic!("no scenario {name:?} in {}", self.name))
    }

    pub fn sends(&self, scenario: &Scenario) -> Sends {
        let has = |key: &str| {
            let step = format!("a valid \"{key}\" key in the system");
            self.background.iter().chain(&scenario.steps).any(|s| *s == step)
        };
        Sends { api_key: has("apiKeyAuth"), app_key: has("appKeyAuth") }
    }

    /// The recorded interactions of a scenario, or None when it has no cassette (not recorded upstream).
    pub fn cassette(&self, scenario: &Scenario) -> Option<Vec<Interaction>> {
        let file = self.cassette_dir().join(format!("{}.yaml", cassette_name(&scenario.name)));
        let text = std::fs::read_to_string(file).ok()?;
        let c: CassetteFile = serde_yaml_ng::from_str(&text).unwrap_or_else(|e| panic!("cassette of {:?}: {e}", scenario.name));
        Some(c.interactions)
    }
}

/// The cassette file name of a scenario, as datadog-api-client-go names them.
pub fn cassette_name(scenario: &str) -> String {
    let mut out = String::from("Scenario_");
    let mut gap = false;
    for c in scenario.chars().filter(|c| *c != '\'') {
        if c.is_ascii_alphanumeric() || c == '-' {
            if gap {
                out.push('_');
            }
            gap = false;
            out.push(c);
        } else {
            gap = true;
        }
    }
    if gap {
        out.push('_');
    }
    out
}

// ================================================================ cassettes

#[derive(Deserialize)]
struct CassetteFile {
    interactions: Vec<Interaction>,
}

#[derive(Deserialize, Debug, Clone)]
pub struct Interaction {
    pub request: RecordedRequest,
    pub response: RecordedResponse,
}

#[derive(Deserialize, Debug, Clone)]
pub struct RecordedRequest {
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub headers: BTreeMap<String, Vec<String>>,
    pub method: String,
    pub url: String,
}

#[derive(Deserialize, Debug, Clone)]
pub struct RecordedResponse {
    #[serde(default)]
    pub body: String,
    pub code: u16,
}

// ================================================================ replay

/// What one replayed request answered.
#[derive(Debug)]
pub struct Exchange {
    pub path: String,
    pub recorded: u16,
    pub status: u16,
    pub body: Value,
}

/// Sends recorded requests to a server, mapping recorded ids onto the ones it answered.
pub struct Replayer {
    base: String,
    api_key: String,
    app_key: String,
    http: reqwest::Client,
    /// Recorded id → our id, learned from earlier answers.
    ids: HashMap<String, Value>,
    /// Response fields not compared (paths as [`compare_shape`] names them, e.g. `widgets[].id`).
    pub ignore: Vec<String>,
}

impl Replayer {
    pub fn new(base: &str, api_key: &str, app_key: &str) -> Self {
        Replayer { base: base.trim_end_matches('/').into(), api_key: api_key.into(), app_key: app_key.into(), http: reqwest::Client::new(), ids: HashMap::new(), ignore: vec![] }
    }

    /// Replays a scenario's cassette; returns the exchanges and every mismatch found.
    pub async fn replay(&mut self, feature: &Feature, scenario: &Scenario) -> (Vec<Exchange>, Vec<String>) {
        let interactions = feature.cassette(scenario).unwrap_or_else(|| panic!("no cassette for {:?}", scenario.name));
        self.replay_interactions(feature.sends(scenario), &scenario.name, &interactions).await
    }

    /// Replays recorded interactions (a cassette, possibly adapted by the caller).
    pub async fn replay_interactions(&mut self, sends: Sends, scenario: &str, interactions: &[Interaction]) -> (Vec<Exchange>, Vec<String>) {
        let mut exchanges = vec![];
        let mut problems = vec![];
        for (i, it) in interactions.iter().enumerate() {
            let (path, query) = self.path_and_query(&it.request.url);
            let url = if query.is_empty() { format!("{}{path}", self.base) } else { format!("{}{path}?{query}", self.base) };
            let method = reqwest::Method::from_bytes(it.request.method.as_bytes()).expect("method");
            let mut req = self.http.request(method, &url);
            for (name, values) in &it.request.headers {
                for v in values {
                    req = req.header(name.as_str(), v.as_str());
                }
            }
            if sends.api_key {
                req = req.header("DD-API-KEY", &self.api_key);
            }
            if sends.app_key {
                req = req.header("DD-APPLICATION-KEY", &self.app_key);
            }
            if !it.request.body.is_empty() {
                req = req.body(self.map_body(&it.request.body));
            }
            let res = req.send().await.unwrap_or_else(|e| panic!("{} {url}: {e}", it.request.method));
            let status = res.status().as_u16();
            let text = res.text().await.unwrap_or_default();
            let body: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
            let at = format!("{scenario} #{i} {} {path}", it.request.method);
            if status / 100 != it.response.code / 100 {
                problems.push(format!("{at}: status {status}, recorded {} (body {text})", it.response.code));
            } else if let Ok(recorded) = serde_json::from_str::<Value>(&it.response.body) {
                if status < 300 {
                    compare_shape(&recorded, &body, "", &self.ignore, &mut problems, &at);
                    self.learn_ids(&recorded, &body);
                } else if recorded.get("errors").is_some() && !is_error_body(&body) {
                    problems.push(format!("{at}: error body {text} is not Datadog's {{\"errors\": [...]}}"));
                }
            }
            exchanges.push(Exchange { path, recorded: it.response.code, status, body });
        }
        (exchanges, problems)
    }

    fn path_and_query(&self, url: &str) -> (String, String) {
        let parsed = url::Url::parse(url).unwrap_or_else(|e| panic!("recorded url {url}: {e}"));
        let path = parsed.path().split('/').map(|seg| self.mapped_text(seg)).collect::<Vec<_>>().join("/");
        let query = parsed
            .query_pairs()
            .map(|(k, v)| {
                let v = v.split(',').map(|part| self.mapped_text(part)).collect::<Vec<_>>().join(",");
                format!("{}={}", urlencode(&k), urlencode(&v))
            })
            .collect::<Vec<_>>()
            .join("&");
        (path, query)
    }

    fn mapped_text(&self, s: &str) -> String {
        match self.ids.get(s) {
            Some(Value::String(v)) => v.clone(),
            Some(v) => v.to_string(),
            None => s.to_string(),
        }
    }

    /// Replaces recorded ids in a JSON body (whole values only, never substrings).
    fn map_body(&self, body: &str) -> String {
        let Ok(mut v) = serde_json::from_str::<Value>(body) else { return body.to_string() };
        self.map_value(&mut v);
        v.to_string()
    }

    fn map_value(&self, v: &mut Value) {
        match v {
            Value::Array(items) => items.iter_mut().for_each(|i| self.map_value(i)),
            Value::Object(map) => map.values_mut().for_each(|i| self.map_value(i)),
            Value::String(s) => {
                if let Some(m) = self.ids.get(s.as_str()) {
                    *v = Value::String(match m {
                        Value::String(m) => m.clone(),
                        m => m.to_string(),
                    });
                }
            }
            Value::Number(n) => {
                if let Some(m) = self.ids.get(&n.to_string()) {
                    *v = m.clone();
                }
            }
            _ => {}
        }
    }

    /// Records `recorded id → our id` for every id field the two answers have at the same place.
    fn learn_ids(&mut self, recorded: &Value, ours: &Value) {
        match (recorded, ours) {
            (Value::Object(r), Value::Object(o)) => {
                for (k, rv) in r {
                    let Some(ov) = o.get(k) else { continue };
                    let id_field = k == "id" || k.ends_with("_id");
                    match (rv, ov) {
                        (Value::String(_) | Value::Number(_), Value::String(_) | Value::Number(_)) if id_field && rv != ov => {
                            let key = match rv {
                                Value::String(s) => s.clone(),
                                other => other.to_string(),
                            };
                            self.ids.insert(key, ov.clone());
                        }
                        _ => self.learn_ids(rv, ov),
                    }
                }
            }
            (Value::Array(r), Value::Array(o)) => r.iter().zip(o).for_each(|(rv, ov)| self.learn_ids(rv, ov)),
            _ => {}
        }
    }
}

fn urlencode(s: &str) -> String {
    url::form_urlencoded::byte_serialize(s.as_bytes()).collect()
}

fn is_error_body(v: &Value) -> bool {
    v.get("errors").and_then(Value::as_array).is_some_and(|e| !e.is_empty() && e.iter().all(Value::is_string))
}

fn kind(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

/// Every field of `recorded` must be in `ours` with the same JSON type (recorded nulls match anything;
/// arrays compare their first items).
pub fn compare_shape(recorded: &Value, ours: &Value, path: &str, ignore: &[String], problems: &mut Vec<String>, at: &str) {
    if recorded.is_null() || ignore.iter().any(|i| i == path) {
        return;
    }
    if kind(recorded) != kind(ours) {
        problems.push(format!("{at}: `{}` is {} ({ours}), recorded {}", if path.is_empty() { "." } else { path }, kind(ours), kind(recorded)));
        return;
    }
    match (recorded, ours) {
        (Value::Object(r), Value::Object(o)) => {
            for (k, rv) in r {
                let child = if path.is_empty() { k.clone() } else { format!("{path}.{k}") };
                match o.get(k) {
                    Some(ov) => compare_shape(rv, ov, &child, ignore, problems, at),
                    None if rv.is_null() || ignore.contains(&child) => {}
                    None => problems.push(format!("{at}: missing `{child}`")),
                }
            }
        }
        (Value::Array(r), Value::Array(o)) => {
            if let (Some(rv), Some(ov)) = (r.first(), o.first()) {
                compare_shape(rv, ov, &format!("{path}[]"), ignore, problems, at);
            }
        }
        _ => {}
    }
}

// ================================================================ in-process server

/// The server's routes on a loopback port over the live test databases, with an org, an admin user
/// with an application key and an ingest key.
pub struct TestServer {
    pub base: String,
    pub st: AppState,
    pub org: String,
    pub user_id: String,
    pub app_key: String,
    pub api_key: String,
}

impl TestServer {
    /// None (the test is skipped) unless the live test databases are configured.
    pub async fn start(prefix: &str) -> Option<TestServer> {
        Self::start_with(prefix, |_| {}).await
    }

    /// [`TestServer::start`] with the server configuration adjusted by `configure`.
    pub async fn start_with(prefix: &str, configure: impl Fn(&mut crate::config::Config)) -> Option<TestServer> {
        let (_, mut states) = crate::test_support::replicas_with(1, configure).await?;
        let st = states.remove(0);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("addr");
        let app = crate::routes::app(st.clone()).with_state(st.clone());
        tokio::spawn(async move { axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>()).await.expect("serve") });
        let org = new_org_with_roles(&st, prefix).await;
        let (user_id, app_key) = user_with_app_key(&st, &org, "admin").await;
        let api_key = ingest_key(&st, &org).await;
        Some(TestServer { base: format!("http://{addr}"), st, org, user_id, app_key, api_key })
    }

    pub fn replayer(&self) -> Replayer {
        Replayer::new(&self.base, &self.api_key, &self.app_key)
    }
}

/// A new org with the built-in admin / standard / read-only roles.
pub async fn new_org_with_roles(st: &AppState, prefix: &str) -> String {
    let org = crate::test_support::new_org(st, prefix).await;
    for role in ["admin", "standard", "readonly"] {
        sqlx::query("INSERT INTO roles (id, org_id, name, managed) VALUES ($1 || '-' || $2, $1, $2, true)").bind(&org).bind(role).execute(&st.postgres).await.expect("role");
    }
    org
}

/// A user of `org` with the built-in `role` and an application key; returns (user id, key).
pub async fn user_with_app_key(st: &AppState, org: &str, role: &str) -> (String, String) {
    let email = format!("{}@example.com", crypto::random_token("tf-").to_lowercase().replace(['-', '_'], ""));
    let (user_id,): (String,) = sqlx::query_as("INSERT INTO users (id, org_id, email, name, status) VALUES (gen_random_uuid(), $1, $2, 'Terraform Test', 'active') RETURNING id::text")
        .bind(org)
        .bind(&email)
        .fetch_one(&st.postgres)
        .await
        .expect("user");
    sqlx::query("INSERT INTO user_roles (user_id, role_id) VALUES ($1::uuid, $2 || '-' || $3)").bind(&user_id).bind(org).bind(role).execute(&st.postgres).await.expect("user role");
    let key = crypto::random_token("ocak_");
    sqlx::query("INSERT INTO app_keys (org_id, user_id, name, key_hash, prefix) VALUES ($1, $2::uuid, 'datadog-compat test', $3, $4)")
        .bind(org)
        .bind(&user_id)
        .bind(crypto::sha256(&key))
        .bind(&key[..12])
        .execute(&st.postgres)
        .await
        .expect("app key");
    (user_id, key)
}

/// An ingest key (`ocik_…`) of `org`.
pub async fn ingest_key(st: &AppState, org: &str) -> String {
    let key = crypto::random_token("ocik_");
    sqlx::query("INSERT INTO ingest_keys (org_id, name, key_hash, prefix) VALUES ($1, 'datadog-compat test', $2, $3)")
        .bind(org)
        .bind(crypto::sha256(&key))
        .bind(&key[..12])
        .execute(&st.postgres)
        .await
        .expect("ingest key");
    key
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn cassette_names_follow_the_go_client() {
        assert_eq!(cassette_name("Validate API key returns \"OK\" response"), "Scenario_Validate_API_key_returns_OK_response");
        assert_eq!(cassette_name("Get a monitor's details returns \"OK\" response"), "Scenario_Get_a_monitors_details_returns_OK_response");
        assert_eq!(cassette_name("Create a ci-pipelines monitor returns \"OK\""), "Scenario_Create_a_ci-pipelines_monitor_returns_OK_");
    }

    #[test]
    fn every_vendored_cassette_belongs_to_a_scenario() {
        for path in [
            "v1/authentication",
            "v1/monitors",
            "v1/downtimes",
            "v1/dashboards",
            "v1/dashboard_lists",
            "v1/service_level_objectives",
            "v1/service_level_objective_corrections",
            "v2/downtimes",
            "v2/dashboard_lists",
            "v2/users",
            "v2/roles",
            "v2/teams",
            "v2/key_management",
        ] {
            let f = Feature::load(path);
            let names: std::collections::HashSet<String> = f.scenarios.iter().map(|s| cassette_name(&s.name)).collect();
            let dir = f.cassette_dir();
            let files = std::fs::read_dir(&dir).unwrap_or_else(|e| panic!("{}: {e}", dir.display()));
            for file in files {
                let name = file.unwrap().file_name().to_string_lossy().to_string();
                let Some(stem) = name.strip_suffix(".yaml") else { continue };
                assert!(names.contains(stem), "{path}: cassette {stem} has no scenario");
            }
        }
    }

    #[test]
    fn features_tell_which_keys_are_sent() {
        let f = Feature::load("v1/authentication");
        assert_eq!(f.name, "Authentication");
        let ok = f.scenario("Validate API key returns \"OK\" response");
        assert_eq!(f.sends(ok), Sends { api_key: true, app_key: false });
        let forbidden = f.scenario("Validate API key returns \"Forbidden\" response");
        assert_eq!(f.sends(forbidden), Sends { api_key: false, app_key: false });
        let m = Feature::load("v1/monitors");
        assert_eq!(m.sends(&m.scenarios[0]), Sends { api_key: true, app_key: true });
    }

    #[test]
    fn shapes_compare_fields_and_types_not_values() {
        let mut p = vec![];
        let recorded = json!({"id": 1, "name": "a", "tags": ["x"], "creator": {"email": "e"}, "deleted": null});
        compare_shape(&recorded, &json!({"id": 9, "name": "b", "tags": [], "creator": {"email": "f"}}), "", &[], &mut p, "t");
        assert!(p.is_empty(), "{p:?}");
        compare_shape(&recorded, &json!({"id": "9", "tags": [1], "creator": {}}), "", &["creator.email".into()], &mut p, "t");
        assert_eq!(p.len(), 3, "{p:?}");
        assert!(p.iter().any(|m| m.contains("`id` is string")));
        assert!(p.iter().any(|m| m.contains("missing `name`")));
        assert!(p.iter().any(|m| m.contains("`tags[]` is number")));
    }

    #[test]
    fn recorded_ids_map_onto_later_requests() {
        let mut r = Replayer::new("http://localhost:1", "k", "a");
        r.learn_ids(&json!({"id": 238669218, "slo": {"id": "abc"}, "name": "x"}), &json!({"id": 5, "slo": {"id": "def"}, "name": "y"}));
        let (path, query) = r.path_and_query("https://api.datadoghq.com/api/v1/monitor/238669218?monitor_ids=238669218,2386692");
        assert_eq!(path, "/api/v1/monitor/5");
        assert_eq!(query, "monitor_ids=5%2C2386692");
        assert_eq!(r.map_body(r#"{"monitor_ids":[238669218],"slo_id":"abc","name":"x"}"#), r#"{"monitor_ids":[5],"name":"x","slo_id":"def"}"#);
    }
}
