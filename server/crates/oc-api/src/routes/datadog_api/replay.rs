//! Contract tests against Datadog's own recordings.
//!
//! The scenarios (`features/`) and their recorded request/response pairs (`cassettes/`) are
//! `datadog-api-client-go`'s (Apache-2.0) at a pinned commit. They are not part of this repository:
//! `scripts/datadog-fixtures/fetch.sh` (`make datadog-fixtures`) downloads them into [`fixtures_dir`],
//! and the tests read them there as they are upstream. [`Replayer`] sends each recorded request to an
//! in-process server ([`TestServer`]) with an `ocak_` test key and checks the answer has the recorded
//! status class and the recorded fields with the same JSON types. Ids, timestamps and names the server
//! generates differ from the recording, so values are not compared; ids from earlier answers are
//! mapped onto later requests instead (create → get → delete).
//!
//! Without the fixtures, [`Feature::load`] returns None and the replay tests are skipped, unless
//! `OC_REQUIRE_DATADOG_FIXTURES=1` (CI), which makes them fail. Live: they also need
//! `OC_TEST_POSTGRES_URL` and `OC_TEST_CLICKHOUSE_URL` (skipped otherwise), like the other live tests
//! (`crate::test_support`).

use std::collections::{BTreeMap, HashMap};
use std::net::SocketAddr;
use std::path::PathBuf;

use serde::Deserialize;
use serde_json::Value;

use crate::auth::crypto;
use crate::state::AppState;

/// The `datadog-api-client-go` commit the fixtures come from (pinned in `scripts/datadog-fixtures/fetch.sh`).
pub const FIXTURES_COMMIT: &str = "1a5c98969c6a269710149bfa7ca7e39320a1e644";

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.is_empty())
}

/// Where `fetch.sh` puts the fixtures: `$OC_DATADOG_FIXTURES`, or
/// `${XDG_CACHE_HOME:-$HOME/.cache}/observecraft/datadog-api-client-go/<commit>`.
pub fn fixtures_dir() -> PathBuf {
    if let Some(dir) = env("OC_DATADOG_FIXTURES") {
        return dir.into();
    }
    let cache = env("XDG_CACHE_HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(env("HOME").unwrap_or_default()).join(".cache"));
    cache.join("observecraft/datadog-api-client-go").join(FIXTURES_COMMIT)
}

/// The fetched scenario tree (upstream's `tests/scenarios`), or None (the test is skipped) when the
/// fixtures haven't been fetched. With `OC_REQUIRE_DATADOG_FIXTURES=1` it panics instead, so CI can't
/// skip them silently.
pub fn fixtures() -> Option<PathBuf> {
    let dir = fixtures_dir();
    if dir.join(".complete").is_file() {
        return Some(dir.join("tests/scenarios"));
    }
    let hint = format!("Datadog API fixtures not found in {} (fetch them with `make datadog-fixtures`)", dir.display());
    assert!(env("OC_REQUIRE_DATADOG_FIXTURES").as_deref() != Some("1"), "{hint}; OC_REQUIRE_DATADOG_FIXTURES=1 requires them");
    eprintln!("{hint}: skipping");
    None
}

/// Upstream scenarios whose cassettes we don't replay, by feature.
enum NotReplayed {
    /// Scenarios whose name contains one of these.
    Containing(&'static [&'static str]),
    /// Every scenario but these.
    AllBut(&'static [&'static str]),
}

fn not_replayed(version: &str, feature: &str) -> Option<NotReplayed> {
    match (version, feature) {
        // Shared (public) dashboards are a separate API, out of scope (and their recordings carry share tokens).
        ("v1", "Dashboards") => Some(NotReplayed::Containing(&["shared dashboard"])),
        // Personal access tokens and scoped application keys are not supported.
        ("v2", "Key Management") => Some(NotReplayed::Containing(&["personal access token", "with scopes"])),
        // Teams and memberships only: not links, hierarchy, connections, notification rules, sync or
        // permission settings, nor the scenarios that set avatars, banners or modules.
        // Budgets and custom costs files; not the cloud accounts, tag pipelines, allocation rules or
        // commitments, nor the recordings of fixed upstream ids (`Get Custom Costs File returns "OK"`).
        ("v2", "Cloud Cost Management") => Some(NotReplayed::AllBut(&[
            "Delete Custom Costs file returns \"Not Found\" response",
            "Delete a budget returns \"Bad Request\" response",
            "Get Custom Costs file returns \"Not Found\" response",
            "Get a budget returns \"Not Found\" response",
            "List budgets returns \"OK\" response",
            "Upload Custom Costs File returns \"Accepted\" response",
            "Upload Custom Costs file returns \"Bad Request\" response",
        ])),
        // Workload Protection's agent rules and policies; not the delete recordings answered 404
        // upstream. (The legacy cloud workload security endpoints' US1-FED recordings have no
        // cassette under their scenarios' names, so they aren't replayed either.)
        ("v2", "CSM Threats") => Some(NotReplayed::Containing(&[
            "Delete a Workload Protection agent rule returns \"OK\" response",
            "Delete a Workload Protection policy returns \"OK\" response",
        ])),
        // RUM applications; not events search and aggregation, source maps or repository information.
        // (Creating is replayed by the scenarios' given steps: its own cassettes were recorded under
        // other names than its scenarios.)
        ("v2", "RUM") => Some(NotReplayed::AllBut(&[
            "Delete a RUM application returns \"No Content\" response",
            "Delete a RUM application returns \"Not Found\" response",
            "Get a RUM application returns \"Not Found\" response",
            "Get a RUM application returns \"OK\" response",
            "List all the RUM applications returns \"OK\" response",
            "Update a RUM application returns \"OK\" response",
            "Update a RUM application with Product Scales returns \"OK\" response",
        ])),
        // Detection rules (log detection), suppressions and signals that aren't recordings of
        // Datadog's own signals; not the other rule types, notification, mute, due date or inbox
        // rules, findings, jobs, version histories or Terraform conversion.
        ("v2", "Security Monitoring") => Some(NotReplayed::AllBut(&[
            "Create a detection rule returns \"Bad Request\" response",
            "Create a detection rule returns \"OK\" response",
            "Delete an existing rule returns \"OK\" response",
            "Get a rule's details returns \"Not Found\" response",
            "Get a rule's details returns \"OK\" response",
            "List rules returns \"OK\" response",
            "Update an existing rule returns \"Bad Request\" response",
            "Update an existing rule returns \"Not Found\" response",
            "Update an existing rule returns \"OK\" response",
            "Validate a detection rule returns \"OK\" response",
            "Validate a detection rule returns \"Bad Request\" response",
            "Get a signal's details returns \"Not Found\" response",
            "Create a suppression rule returns \"OK\" response",
            "Create a suppression rule with an exclusion query returns \"OK\" response",
            "Delete a suppression rule returns \"OK\" response",
            "Get a suppression rule returns \"OK\" response",
            "Get a suppression rule returns \"Not Found\" response",
            "Get all suppression rules returns \"OK\" response with pagination",
            "Get all suppression rules returns \"OK\" response with sort ascending",
            "Get all suppression rules returns \"OK\" response with sort descending",
            "Update a suppression rule returns \"OK\" response",
            "Validate a suppression rule returns \"OK\" response",
            "Validate a suppression rule returns \"Bad Request\" response",
        ])),
        ("v2", "Teams") => Some(NotReplayed::AllBut(&[
            "Add a user to a team returns \"Represents a user's association to a team\" response",
            "Create a team returns \"CREATED\" response",
            "Get a team returns \"OK\" response",
            "Get all teams returns \"OK\" response",
            "Get all teams returns \"OK\" response with pagination",
            "Get all teams with fields_team parameter returns \"OK\" response",
            "Get links for a team returns \"OK\" response",
            "Get team memberships returns \"Represents a user's association to a team\" response",
            "Get user memberships returns \"Represents a user's association to a team\" response",
            "Remove a team returns \"No Content\" response",
            "Remove a user from a team returns \"No Content\" response",
            "Update a team with partial update returns \"OK\" response",
            "Update a user's membership attributes on a team returns \"OK\" response",
        ])),
        _ => None,
    }
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
    /// `version/name`, e.g. `v1/authentication` for `features/v1/authentication.feature`; None (the
    /// test is skipped) when the fixtures haven't been fetched ([`fixtures`]).
    pub fn load(path: &str) -> Option<Feature> {
        let file = fixtures()?.join("features").join(format!("{path}.feature"));
        let text = std::fs::read_to_string(&file).unwrap_or_else(|e| panic!("{}: {e}", file.display()));
        Some(Self::parse(path.split('/').next().unwrap_or("v1"), &text))
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
        fixtures_dir().join("tests/scenarios/cassettes/TestScenarios").join(&self.version).join(format!("Feature_{}", self.name.replace(' ', "_")))
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

    /// Whether we replay a scenario's cassette (see [`not_replayed`]).
    pub fn replayed(&self, scenario: &Scenario) -> bool {
        match not_replayed(&self.version, &self.name) {
            None => true,
            Some(NotReplayed::Containing(parts)) => !parts.iter().any(|p| scenario.name.contains(p)),
            Some(NotReplayed::AllBut(names)) => names.contains(&scenario.name.as_str()),
        }
    }

    /// A cassette of the feature recorded under a name no scenario has (upstream renamed the scenario
    /// and kept the recording), or None.
    pub fn cassette_named(&self, name: &str) -> Option<Vec<Interaction>> {
        let text = std::fs::read_to_string(self.cassette_dir().join(format!("{}.yaml", cassette_name(name)))).ok()?;
        let c: CassetteFile = serde_yaml_ng::from_str(&text).unwrap_or_else(|e| panic!("cassette {name:?}: {e}"));
        Some(c.interactions)
    }

    /// The recorded interactions of a scenario, or None when it has no cassette (not recorded upstream)
    /// or one we don't replay.
    pub fn cassette(&self, scenario: &Scenario) -> Option<Vec<Interaction>> {
        if !self.replayed(scenario) {
            return None;
        }
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

/// Datadog's error bodies: `{"errors": ["…"]}`, or `{"errors": [{"title": "…", …}]}` (the
/// Remote Configuration endpoints).
fn is_error_body(v: &Value) -> bool {
    v.get("errors").and_then(Value::as_array).is_some_and(|e| !e.is_empty() && e.iter().all(|x| x.is_string() || x.get("title").is_some_and(Value::is_string)))
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
        // The recordings carry their test org's addresses: never mail anyone, whatever OC_SMTP_URL says.
        let (_, mut states) = crate::test_support::replicas_with(1, |c| {
            configure(c);
            c.smtp_url = None;
        })
        .await?;
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

    /// The features the tests replay (what `scripts/datadog-fixtures/fetch.sh` downloads).
    const FEATURES: [&str; 20] = [
        "v1/authentication",
        "v2/csm_threats",
        "v2/csm_agents",
        "v2/csm_coverage_analysis",
        "v2/application_security",
        "v2/rum",
        "v2/security_monitoring",
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
        "v2/cloud_cost_management",
    ];

    #[test]
    fn the_fetch_script_downloads_the_replayed_features_at_the_pinned_commit() {
        let script = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../../scripts/datadog-fixtures/fetch.sh")).unwrap();
        assert!(script.contains(&format!("\nCOMMIT={FIXTURES_COMMIT}\n")), "fetch.sh pins another commit than FIXTURES_COMMIT");
        let listed = script.split("FEATURES=\"").nth(1).and_then(|s| s.split('"').next()).unwrap();
        let mut listed: Vec<&str> = listed.split_whitespace().collect();
        let mut ours = FEATURES.to_vec();
        listed.sort();
        ours.sort();
        assert_eq!(listed, ours);
    }

    #[test]
    fn scenarios_not_replayed_exist_upstream() {
        for path in FEATURES {
            let Some(f) = Feature::load(path) else { return };
            let recorded = |s: &&Scenario| f.cassette_dir().join(format!("{}.yaml", cassette_name(&s.name))).is_file();
            assert!(f.scenarios.iter().filter(recorded).any(|s| f.replayed(s)), "{path}: no cassette found");
            match not_replayed(&f.version, &f.name) {
                None => {}
                Some(NotReplayed::Containing(parts)) => {
                    for part in parts {
                        assert!(f.scenarios.iter().filter(recorded).any(|s| s.name.contains(part)), "{path}: no recorded scenario contains {part:?}");
                    }
                }
                Some(NotReplayed::AllBut(names)) => {
                    for name in names {
                        assert!(recorded(&f.scenario(name)), "{path}: {name:?} has no cassette");
                    }
                }
            }
        }
    }

    #[test]
    fn without_fixtures_replays_skip_unless_required() {
        // Not through the environment of this process: other tests read it concurrently.
        let missing = std::env::temp_dir().join("oc-no-datadog-fixtures");
        let run = |required: &str| {
            std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "routes::datadog_api::replay::tests::features_tell_which_keys_are_sent", "--nocapture"])
                .env("OC_DATADOG_FIXTURES", &missing)
                .env("OC_REQUIRE_DATADOG_FIXTURES", required)
                .output()
                .unwrap()
        };
        let skipped = run("");
        let stderr = String::from_utf8_lossy(&skipped.stderr);
        assert!(skipped.status.success() && stderr.contains("make datadog-fixtures") && stderr.contains("skipping"), "{stderr}");
        let required = run("1");
        let out = String::from_utf8_lossy(&required.stdout).to_string() + &String::from_utf8_lossy(&required.stderr);
        assert!(!required.status.success() && out.contains("OC_REQUIRE_DATADOG_FIXTURES=1 requires them"), "{out}");
    }

    #[test]
    fn features_tell_which_keys_are_sent() {
        let Some(f) = Feature::load("v1/authentication") else { return };
        assert_eq!(f.name, "Authentication");
        let ok = f.scenario("Validate API key returns \"OK\" response");
        assert_eq!(f.sends(ok), Sends { api_key: true, app_key: false });
        let forbidden = f.scenario("Validate API key returns \"Forbidden\" response");
        assert_eq!(f.sends(forbidden), Sends { api_key: false, app_key: false });
        let m = Feature::load("v1/monitors").unwrap();
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
