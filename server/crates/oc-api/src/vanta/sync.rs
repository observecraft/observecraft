//! The Vanta sync, on the cloud crawlers' jobs (`crate::cloud::jobs`, provider [`PROVIDER`], one job per connection,
//! scope [`SCOPE`], every [`INTERVAL_SECS`]), so several servers share the connections (`FOR UPDATE SKIP LOCKED`).
//!
//! Vanta keeps one active token per app (a new one revokes the previous), so a run gets one token and sends every
//! resource type the connection has a `resourceId` for, in [`TYPES`]' order. Each `PUT` carries the type's full
//! state, which Vanta replaces: at most [`MAX_RESOURCES`] (the collectors order them, most important first). A type
//! Vanta refuses doesn't stop the others; each type's result is kept (`vanta_sync_results`) for the page.

use std::collections::HashMap;
use std::sync::Arc;

use serde_json::{json, Value};

use super::api::{counts, VantaApi, VantaError};
use crate::cloud::jobs::{Backoff, Crawler, Issue, JobKey, Outcome};
use crate::state::AppState;

pub const PROVIDER: &str = "vanta";
/// A connection's only job scope.
pub const SCOPE: &str = "sync";
/// How often a connection is synced (Vanta's guidance: hourly).
pub const INTERVAL_SECS: i64 = 3600;
/// Resources Vanta takes per `PUT`.
pub const MAX_RESOURCES: usize = 10_000;
/// The resource types sent, in this order (components before the vulnerabilities that name them). The last three are
/// custom resources of their own in Vanta (one Developer Console resource each).
pub const TYPES: [&str; 7] = ["user_account", "vulnerable_component", "package_vulnerability", "security_task", "monitor", "detection_rule", "log_index"];

/// The Build Integrations endpoint (`/v1/resources/<endpoint>`) a type is sent to.
pub fn endpoint(kind: &str) -> &str {
    match kind {
        "package_vulnerability" => "package_vulnerability_connectors",
        "monitor" | "detection_rule" | "log_index" => "custom_resource",
        k => k,
    }
}

/// The Developer Console base type a type's resource is created with.
pub fn base_type(kind: &str) -> &'static str {
    match kind {
        "user_account" => "UserAccount",
        "vulnerable_component" => "VulnerableComponent",
        "package_vulnerability" => "PackageVulnerability",
        "security_task" => "SecurityTask",
        _ => "CustomResource",
    }
}

/// An org's connection, with its secret unsealed.
#[derive(Clone, Debug)]
pub struct Connection {
    pub id: String,
    pub api_url: String,
    pub client_id: String,
    pub client_secret: String,
    /// The Developer Console's `resourceId` per resource type.
    pub resource_ids: HashMap<String, String>,
}

impl Connection {
    /// The types of [`TYPES`] this connection sends (those with a `resourceId`).
    pub fn types(&self) -> Vec<&'static str> {
        TYPES.into_iter().filter(|t| self.resource_ids.get(*t).is_some_and(|id| !id.trim().is_empty())).collect()
    }
}

/// One resource type's sync.
#[derive(Clone, Debug, PartialEq)]
pub struct TypeResult {
    pub resource_type: String,
    pub sent: usize,
    pub accepted: usize,
    pub rejected: usize,
    pub error: Option<String>,
}

/// A Vanta error as the connection's issue.
pub fn issue(e: &VantaError) -> Issue {
    let (kind, backoff) = match e {
        VantaError::Throttled(_) => ("throttled", Backoff::Exponential),
        VantaError::Auth(_) => ("auth", Backoff::Interval),
        VantaError::Invalid(_) => ("other", Backoff::Interval),
        VantaError::Other(_) => ("other", Backoff::Exponential),
    };
    Issue { kind, message: e.message().to_string(), missing_action: None, backoff }
}

/// Sends `resources` (per type, in order) with one token; the run's outcome and each type's result.
pub async fn sync(api: &dyn VantaApi, conn: &Connection, resources: Vec<(&str, Vec<Value>)>, now_ms: i64) -> (Outcome, Vec<TypeResult>) {
    let resource_id = |t: &str| conn.resource_ids.get(t).map(|id| id.trim().to_string()).filter(|id| !id.is_empty());
    let resources: Vec<(&str, Vec<Value>)> = resources.into_iter().filter(|(t, _)| resource_id(t).is_some()).collect();
    if resources.is_empty() {
        return (Outcome::Idle, Vec::new());
    }
    let failed = |t: &str, why: String| TypeResult { resource_type: t.to_string(), sent: 0, accepted: 0, rejected: 0, error: Some(why) };
    let token = match api.token(&conn.api_url, &conn.client_id, &conn.client_secret).await {
        Ok(t) => t,
        Err(e) => return (Outcome::Failed(issue(&e)), resources.iter().map(|(t, _)| failed(t, e.message().to_string())).collect()),
    };
    let (mut results, mut problems) = (Vec::new(), Vec::new());
    let (mut taken, mut accepted, mut sent_total) = (0, 0, 0);
    let (mut first_error, mut throttled): (Option<VantaError>, Option<VantaError>) = (None, None);
    for (t, mut items) in resources {
        if let Some(e) = &throttled {
            results.push(failed(t, format!("not sent: {}", e.message())));
            continue;
        }
        let total = items.len();
        let capped = (total > MAX_RESOURCES).then(|| format!("{MAX_RESOURCES} of {total} sent: Vanta takes at most {MAX_RESOURCES}"));
        items.truncate(MAX_RESOURCES);
        let sent = items.len();
        let body = json!({ "resourceId": resource_id(t), "resources": items });
        match api.put(&conn.api_url, &token, endpoint(t), &body).await {
            Ok(answer) => {
                let (a, r) = counts(&answer, sent);
                (taken, accepted, sent_total) = (taken + 1, accepted + a, sent_total + sent);
                let notes: Vec<String> = (r > 0).then(|| format!("{r} of {sent} rejected")).into_iter().chain(capped).collect();
                let error = (!notes.is_empty()).then(|| notes.join("; "));
                if let Some(e) = &error {
                    problems.push(format!("{t}: {e}"));
                }
                results.push(TypeResult { resource_type: t.to_string(), sent, accepted: a, rejected: r, error });
            }
            Err(e) => {
                problems.push(format!("{t}: {}", e.message()));
                results.push(failed(t, e.message().to_string()));
                if matches!(e, VantaError::Throttled(_)) {
                    throttled = Some(e.clone());
                }
                first_error.get_or_insert(e);
            }
        }
    }
    // Throttled: the next run sends again; nothing taken: the run failed as its first error.
    if let Some(e) = throttled.or(if taken == 0 { first_error } else { None }) {
        return (Outcome::Failed(issue(&e)), results);
    }
    let warning = (!problems.is_empty()).then(|| Issue { kind: "other", message: problems.join("; "), missing_action: None, backoff: Backoff::Interval });
    (Outcome::Collected { end_ms: now_ms, metrics: accepted, points: sent_total, warning }, results)
}

/// `vanta_connections`' id, api_url, client_id, client_secret_enc and resource_ids.
type ConnectionRow = (String, String, String, String, sqlx::types::Json<HashMap<String, String>>);

/// The org's connection with its secret unsealed.
pub async fn connection(st: &AppState, org: &str) -> Result<Option<Connection>, sqlx::Error> {
    let row: Option<ConnectionRow> =
        sqlx::query_as("SELECT id::text, api_url, client_id, client_secret_enc, resource_ids FROM vanta_connections WHERE org_id = $1").bind(org).fetch_optional(&st.postgres).await?;
    Ok(row.and_then(|(id, api_url, client_id, sealed, ids)| {
        let client_secret = st.secrets.decrypt(&sealed).ok()?;
        Some(Connection { id, api_url, client_id, client_secret, resource_ids: ids.0 })
    }))
}

/// The resources of each of `types`, read from ObserveCraft (the vulnerabilities once for both their types).
async fn collect(st: &AppState, org: &str, types: &[&'static str], now_ms: i64) -> Result<Vec<(&'static str, Vec<Value>)>, crate::error::ApiError> {
    let needs_vulnerabilities = types.iter().any(|t| matches!(*t, "vulnerable_component" | "package_vulnerability"));
    let mut vulnerabilities = if needs_vulnerabilities { super::vulnerabilities::collect(st, org).await? } else { Default::default() };
    let mut out = Vec::new();
    for t in types {
        let items = match *t {
            "user_account" => super::users::user_accounts(st, org).await?,
            "vulnerable_component" => std::mem::take(&mut vulnerabilities.components),
            "package_vulnerability" => std::mem::take(&mut vulnerabilities.vulnerabilities),
            "security_task" => super::findings::security_tasks(st, org).await?,
            "monitor" => super::custom::monitors(st, org, now_ms).await?,
            "detection_rule" => super::custom::detection_rules(st, org).await?,
            "log_index" => super::custom::log_indexes(st, org).await?,
            _ => Vec::new(),
        };
        out.push((*t, items));
    }
    Ok(out)
}

/// Keeps each type's result for the page.
pub async fn save_results(st: &AppState, connection_id: &str, results: &[TypeResult]) -> Result<(), sqlx::Error> {
    for r in results {
        sqlx::query(
            "INSERT INTO vanta_sync_results (connection_id, resource_type, synced_at, sent, accepted, rejected, error) VALUES ($1::uuid, $2, now(), $3, $4, $5, $6) \
             ON CONFLICT (connection_id, resource_type) DO UPDATE SET synced_at = now(), sent = EXCLUDED.sent, accepted = EXCLUDED.accepted, rejected = EXCLUDED.rejected, error = EXCLUDED.error",
        )
        .bind(connection_id)
        .bind(&r.resource_type)
        .bind(r.sent as i32)
        .bind(r.accepted as i32)
        .bind(r.rejected as i32)
        .bind(&r.error)
        .execute(&st.postgres)
        .await?;
    }
    Ok(())
}

/// The sync: Vanta through `api`.
pub struct VantaCrawler {
    pub api: Arc<dyn VantaApi>,
}

#[async_trait::async_trait]
impl Crawler for VantaCrawler {
    fn provider(&self) -> &'static str {
        PROVIDER
    }

    /// One job per connection.
    async fn desired(&self, st: &AppState) -> Result<Vec<(String, String, String)>, sqlx::Error> {
        let rows: Vec<(String, String)> = sqlx::query_as("SELECT c.org_id, c.id::text FROM vanta_connections c JOIN orgs o ON o.id = c.org_id AND o.status = 'active'").fetch_all(&st.postgres).await?;
        Ok(rows.into_iter().map(|(org, id)| (org, id, SCOPE.to_string())).collect())
    }

    async fn run(&self, st: &AppState, job: &JobKey, now_ms: i64) -> Result<Outcome, sqlx::Error> {
        let Some(conn) = connection(st, &job.org_id).await? else { return Ok(Outcome::Idle) };
        if conn.id != job.account_id || job.scope != SCOPE {
            return Ok(Outcome::Idle);
        }
        let resources = match collect(st, &job.org_id, &conn.types(), now_ms).await {
            Ok(r) => r,
            Err(e) => return Ok(Outcome::Failed(Issue { kind: "other", message: format!("Reading ObserveCraft's records failed: {e}"), missing_action: None, backoff: Backoff::Exponential })),
        };
        let (outcome, results) = sync(self.api.as_ref(), &conn, resources, now_ms).await;
        save_results(st, &conn.id, &results).await?;
        Ok(outcome)
    }
}

#[cfg(test)]
pub mod fake {
    use std::sync::Mutex;

    use super::*;

    /// Vanta's API, recorded: token requests and pushes (type, body); `refuse_token`, and an error per type.
    #[derive(Default)]
    pub struct FakeVanta {
        pub refuse_token: bool,
        pub errors: HashMap<String, VantaError>,
        /// Items of each push Vanta rejects (counted from the end).
        pub reject: usize,
        pub tokens: Mutex<usize>,
        pub pushes: Mutex<Vec<(String, Value)>>,
    }

    #[async_trait::async_trait]
    impl VantaApi for FakeVanta {
        async fn token(&self, _api_url: &str, _client_id: &str, _client_secret: &str) -> Result<String, VantaError> {
            *self.tokens.lock().unwrap() += 1;
            if self.refuse_token {
                return Err(VantaError::Auth("Vanta answered 401 Unauthorized: invalid_client".into()));
            }
            Ok("vat_example".into())
        }

        async fn put(&self, _api_url: &str, token: &str, resource_type: &str, body: &Value) -> Result<Value, VantaError> {
            assert_eq!(token, "vat_example");
            self.pushes.lock().unwrap().push((resource_type.to_string(), body.clone()));
            if let Some(e) = self.errors.get(resource_type) {
                return Err(e.clone());
            }
            let n = body["resources"].as_array().map_or(0, Vec::len);
            let rejected = self.reject.min(n);
            Ok(json!({ "results": { "accepted": n - rejected, "rejected": rejected } }))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fake::FakeVanta;
    use super::*;

    fn conn(ids: &[(&str, &str)]) -> Connection {
        Connection {
            id: "00000000-0000-4000-8000-0000000000c1".into(),
            api_url: "https://api.vanta.com".into(),
            client_id: "vci_example".into(),
            client_secret: "vcs_secret".into(),
            resource_ids: ids.iter().map(|(t, id)| (t.to_string(), id.to_string())).collect(),
        }
    }

    fn items(prefix: &str, n: usize) -> Vec<Value> {
        (0..n).map(|i| json!({ "uniqueId": format!("{prefix}{i}"), "displayName": format!("{prefix} {i}") })).collect()
    }

    const NOW: i64 = 1_795_000_000_000;

    #[test]
    fn kinds_go_to_vantas_endpoints_in_a_fixed_order() {
        assert_eq!(endpoint("monitor"), "custom_resource");
        assert_eq!(endpoint("detection_rule"), "custom_resource");
        assert_eq!(endpoint("log_index"), "custom_resource");
        assert_eq!(endpoint("package_vulnerability"), "package_vulnerability_connectors");
        assert_eq!(endpoint("vulnerable_component"), "vulnerable_component");
        assert_eq!(endpoint("user_account"), "user_account");
        assert_eq!(endpoint("security_task"), "security_task");
        // Components before the vulnerabilities that name them, whatever order they were saved in.
        let all = conn(&[("log_index", "r7"), ("package_vulnerability", "r3"), ("user_account", "r1"), ("vulnerable_component", "r2"), ("monitor", "r5"), ("security_task", "r4"), ("detection_rule", "r6")]);
        assert_eq!(all.types(), TYPES);
        assert_eq!(TYPES, ["user_account", "vulnerable_component", "package_vulnerability", "security_task", "monitor", "detection_rule", "log_index"]);
    }

    #[test]
    fn only_types_with_a_resource_id_are_sent() {
        assert_eq!(conn(&[("user_account", "res_users")]).types(), ["user_account"]);
        assert!(conn(&[("user_account", " ")]).types().is_empty());
        assert!(conn(&[("not_a_type", "res_x")]).types().is_empty());
    }

    #[tokio::test]
    async fn one_token_then_each_types_full_state_in_order() {
        let api = FakeVanta::default();
        let c = conn(&[("user_account", "res_users"), ("monitor", "res_monitors")]);
        let (outcome, results) = sync(&api, &c, vec![("user_account", items("u", 3)), ("monitor", items("m", 2))], NOW).await;
        assert_eq!(*api.tokens.lock().unwrap(), 1, "one token for the run: a new one revokes the last");
        let pushes = api.pushes.lock().unwrap().clone();
        assert_eq!(pushes.iter().map(|(t, _)| t.as_str()).collect::<Vec<_>>(), ["user_account", "custom_resource"], "a monitor is one of Vanta's custom resources");
        assert_eq!(pushes[0].1["resourceId"], "res_users");
        assert_eq!(pushes[0].1["resources"].as_array().unwrap().len(), 3);
        assert_eq!(pushes[1].1["resourceId"], "res_monitors");
        assert_eq!(outcome, Outcome::Collected { end_ms: NOW, metrics: 5, points: 5, warning: None });
        assert_eq!(results[0], TypeResult { resource_type: "user_account".into(), sent: 3, accepted: 3, rejected: 0, error: None });
    }

    #[tokio::test]
    async fn an_empty_type_is_still_sent_so_vanta_deletes_what_is_gone() {
        let api = FakeVanta::default();
        let (outcome, _) = sync(&api, &conn(&[("user_account", "res_users")]), vec![("user_account", vec![])], NOW).await;
        assert_eq!(api.pushes.lock().unwrap()[0].1["resources"], json!([]));
        assert_eq!(outcome, Outcome::Collected { end_ms: NOW, metrics: 0, points: 0, warning: None });
    }

    #[tokio::test]
    async fn nothing_configured_asks_for_no_token() {
        let api = FakeVanta::default();
        let (outcome, results) = sync(&api, &conn(&[]), vec![], NOW).await;
        assert_eq!((outcome, results.len(), *api.tokens.lock().unwrap()), (Outcome::Idle, 0, 0));
    }

    #[tokio::test]
    async fn a_refused_token_sends_nothing_and_says_so_per_type() {
        let api = FakeVanta { refuse_token: true, ..Default::default() };
        let (outcome, results) = sync(&api, &conn(&[("user_account", "res_users")]), vec![("user_account", items("u", 2))], NOW).await;
        match outcome {
            Outcome::Failed(i) => assert_eq!((i.kind, i.backoff), ("auth", Backoff::Interval)),
            other => panic!("{other:?}"),
        }
        assert!(api.pushes.lock().unwrap().is_empty());
        assert_eq!(results[0].sent, 0);
        assert!(results[0].error.as_deref().unwrap().contains("invalid_client"), "{results:?}");
    }

    #[tokio::test]
    async fn rejected_items_and_a_refused_type_are_warnings_while_others_go_through() {
        let api = FakeVanta { reject: 1, errors: HashMap::from([("custom_resource".to_string(), VantaError::Invalid("Vanta answered 422: missing field".into()))]), ..Default::default() };
        let c = conn(&[("user_account", "res_users"), ("monitor", "res_monitors")]);
        let (outcome, results) = sync(&api, &c, vec![("user_account", items("u", 3)), ("monitor", items("m", 2))], NOW).await;
        match outcome {
            Outcome::Collected { metrics, points, warning: Some(w), .. } => {
                assert_eq!((metrics, points), (2, 3), "accepted and sent of the types Vanta took");
                assert!(w.message.contains("user_account: 1 of 3 rejected") && w.message.contains("monitor: Vanta answered 422"), "{}", w.message);
            }
            other => panic!("{other:?}"),
        }
        assert_eq!((results[0].accepted, results[0].rejected), (2, 1));
        assert_eq!((results[1].sent, results[1].error.is_some()), (0, true));
    }

    #[tokio::test]
    async fn when_every_type_fails_the_run_fails() {
        let api = FakeVanta { errors: HashMap::from([("user_account".to_string(), VantaError::Auth("Vanta answered 403: resource of another app".into()))]), ..Default::default() };
        let (outcome, _) = sync(&api, &conn(&[("user_account", "res_users")]), vec![("user_account", items("u", 1))], NOW).await;
        assert!(matches!(outcome, Outcome::Failed(Issue { kind: "auth", .. })), "{outcome:?}");
    }

    #[tokio::test]
    async fn throttling_stops_the_run_for_the_next_one() {
        let api = FakeVanta { errors: HashMap::from([("user_account".to_string(), VantaError::Throttled("Vanta answered 429".into()))]), ..Default::default() };
        let c = conn(&[("user_account", "res_users"), ("monitor", "res_monitors")]);
        let (outcome, results) = sync(&api, &c, vec![("user_account", items("u", 1)), ("monitor", items("m", 1))], NOW).await;
        assert!(matches!(outcome, Outcome::Failed(Issue { kind: "throttled", .. })), "{outcome:?}");
        assert_eq!(api.pushes.lock().unwrap().len(), 1, "no more requests once Vanta throttles");
        assert_eq!(results.len(), 2);
    }

    #[tokio::test]
    async fn more_than_vanta_takes_sends_the_first_and_warns() {
        let api = FakeVanta::default();
        let (outcome, results) = sync(&api, &conn(&[("user_account", "res_users")]), vec![("user_account", items("u", MAX_RESOURCES + 5))], NOW).await;
        assert_eq!(api.pushes.lock().unwrap()[0].1["resources"].as_array().unwrap().len(), MAX_RESOURCES);
        assert_eq!(api.pushes.lock().unwrap()[0].1["resources"][0]["uniqueId"], "u0");
        match outcome {
            Outcome::Collected { warning: Some(w), .. } => assert!(w.message.contains("10000 of 10005"), "{}", w.message),
            other => panic!("{other:?}"),
        }
        assert!(results[0].error.as_deref().unwrap().contains("10000 of 10005"));
    }
}
