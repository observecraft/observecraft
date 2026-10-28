//! Feature flag evaluation for OpenFeature SDKs over the OpenFeature Remote Evaluation Protocol
//! (OFREP, <https://github.com/open-feature/protocol>, Apache-2.0), so any OpenFeature SDK with an
//! OFREP provider evaluates the org's flags (Software Delivery → Feature Flags):
//!
//! - `POST /ofrep/v1/evaluate/flags/{key}`: one flag (server SDKs, a context per evaluation).
//!   Answers flags whose SDK access includes server SDKs.
//! - `POST /ofrep/v1/evaluate/flags`: every flag (client SDKs, which cache the result). Answers
//!   flags that client SDKs may see, with an `ETag` of the answer; `If-None-Match` with it gets
//!   `304 Not Modified`.
//!
//! Senders authenticate with the org's ingest API key like every other intake request
//! (`Authorization: Bearer ocik_…`, or OFREP's `X-API-Key`) and share its rate limit. The
//! environment is the one whose queries match the context's `env` (or `environment`), else the
//! API key's default environment, else the org's only environment.
//!
//! Flag configuration is read from Postgres per org and cached for [`FLAGS_TTL`], so a change in
//! the UI reaches SDKs within seconds. Evaluations are counted per flag, variant, environment and
//! reason and sent every minute to the server, which writes them as the `feature_flag.evaluations`
//! metric (a bulk evaluation counts once per flag it answers; `304`s are not counted).

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use oc_flags::{EvalError, FlagEnvConfig, FlagVariant, Resolution};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};

use crate::Shared;

/// How long an org's flag configuration is trusted before it is read again.
pub const FLAGS_TTL: Duration = Duration::from_secs(5);
/// How often evaluation counts are sent to the server.
pub const FLUSH_EVERY: Duration = Duration::from_secs(60);

#[derive(Clone, Debug, PartialEq)]
pub struct Environment {
    pub id: String,
    pub name: String,
    pub queries: Vec<String>,
}

/// Which SDKs a flag answers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Surface {
    /// The single-flag endpoint (server SDKs).
    Server,
    /// The bulk endpoint (client SDKs).
    Client,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Flag {
    pub key: String,
    /// `all`, `client` or `server`.
    pub sdk_access: String,
    pub variants: Vec<FlagVariant>,
    /// Per environment id; a missing environment is off.
    pub configs: HashMap<String, FlagEnvConfig>,
}

impl Flag {
    fn answers(&self, s: Surface) -> bool {
        match s {
            Surface::Server => self.sdk_access != "client",
            Surface::Client => self.sdk_access != "server",
        }
    }
}

/// One org's flags and environments.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct OrgFlags {
    pub environments: Vec<Environment>,
    /// By key, so bulk answers list flags in a stable order.
    pub flags: BTreeMap<String, Flag>,
}

/// An evaluation to count: (flag, variant, environment, reason).
pub type Counted = (String, String, String, &'static str);

/// Cached [`OrgFlags`] per org and the evaluation counts not sent yet.
#[derive(Default)]
pub struct FlagState {
    cache: Mutex<HashMap<String, (Instant, Arc<OrgFlags>)>>,
    counts: Mutex<HashMap<(String, Counted), u64>>,
}

// ================================================================ evaluation (pure)

/// A failure answer: `{key, errorCode, errorDetails}` (`key` omitted for whole-request failures).
fn failure(key: Option<&str>, code: &str, details: &str) -> Value {
    let mut out = Map::new();
    if let Some(k) = key {
        out.insert("key".into(), Value::from(k));
    }
    out.insert("errorCode".into(), Value::from(code));
    out.insert("errorDetails".into(), Value::from(details));
    Value::Object(out)
}

fn success(key: &str, r: &Resolution, env: &Environment) -> Value {
    let mut out = Map::new();
    out.insert("key".into(), Value::from(key));
    out.insert("reason".into(), Value::from(r.reason.as_str()));
    if let Some(v) = &r.variant {
        out.insert("variant".into(), Value::from(v.as_str()));
    }
    // Disabled flags carry no value: the provider falls back to the code default.
    if let Some(v) = &r.value {
        out.insert("value".into(), v.clone());
    }
    out.insert("metadata".into(), json!({ "environment": env.name }));
    Value::Object(out)
}

/// The evaluation context of a request body (`{"context": {...}}`; a missing context is empty).
pub fn parse_context(body: &[u8]) -> Result<Map<String, Value>, (&'static str, String)> {
    if body.iter().all(u8::is_ascii_whitespace) {
        return Ok(Map::new());
    }
    let v: Value = serde_json::from_slice(body).map_err(|e| ("PARSE_ERROR", format!("the request body is not JSON: {e}")))?;
    let Value::Object(mut o) = v else {
        return Err(("PARSE_ERROR", "the request body must be a JSON object".into()));
    };
    match o.remove("context") {
        None | Some(Value::Null) => Ok(Map::new()),
        Some(Value::Object(ctx)) => Ok(ctx),
        Some(_) => Err(("INVALID_CONTEXT", "context must be an object".into())),
    }
}

/// The environment of an evaluation: the one whose queries match the context's `env` (or
/// `environment`), else the API key's default, else the org's only environment.
pub fn environment<'a>(flags: &'a OrgFlags, ctx: &Map<String, Value>, key_default: Option<&str>) -> Result<&'a Environment, EvalError> {
    let named = ["env", "environment"].iter().find_map(|k| ctx.get(*k));
    match named {
        Some(Value::String(env)) => flags
            .environments
            .iter()
            .find(|e| e.queries.iter().any(|q| oc_flags::env_matches(q, env)))
            .ok_or_else(|| EvalError::InvalidContext(format!("no feature flag environment matches env '{env}'"))),
        Some(_) => Err(EvalError::InvalidContext("env must be a string".into())),
        None => {
            if let Some(e) = key_default.and_then(|id| flags.environments.iter().find(|e| e.id == id)) {
                return Ok(e);
            }
            match flags.environments.as_slice() {
                [only] => Ok(only),
                [] => Err(EvalError::InvalidContext("this organization has no feature flag environments yet".into())),
                _ => Err(EvalError::InvalidContext("send env (or environment) in the evaluation context, or give the API key a default environment".into())),
            }
        }
    }
}

fn evaluate_in(flag: &Flag, env: &Environment, ctx: &Map<String, Value>, now_ms: i64) -> Result<Resolution, EvalError> {
    let off = FlagEnvConfig::default();
    let cfg = flag.configs.get(&env.id).unwrap_or(&off);
    oc_flags::evaluate(&flag.key, &flag.variants, cfg, ctx, now_ms)
}

fn counted(flag: &str, r: &Resolution, env: &Environment) -> Counted {
    (flag.to_string(), r.variant.clone().unwrap_or_default(), env.name.clone(), r.reason.as_str())
}

/// `POST /ofrep/v1/evaluate/flags/{key}`: status, body and what to count.
pub fn evaluate_single(flags: &OrgFlags, key: &str, body: &[u8], key_default: Option<&str>, now_ms: i64) -> (StatusCode, Value, Option<Counted>) {
    let ctx = match parse_context(body) {
        Ok(c) => c,
        Err((code, details)) => return (StatusCode::BAD_REQUEST, failure(Some(key), code, &details), None),
    };
    let Some(flag) = flags.flags.get(key).filter(|f| f.answers(Surface::Server)) else {
        return (StatusCode::NOT_FOUND, failure(Some(key), "FLAG_NOT_FOUND", &format!("flag '{key}' was not found")), None);
    };
    let env = match environment(flags, &ctx, key_default) {
        Ok(e) => e,
        Err(e) => return (StatusCode::BAD_REQUEST, failure(Some(key), e.code(), &e.details()), None),
    };
    match evaluate_in(flag, env, &ctx, now_ms) {
        Ok(r) => (StatusCode::OK, success(key, &r, env), Some(counted(key, &r, env))),
        Err(e) => (StatusCode::BAD_REQUEST, failure(Some(key), e.code(), &e.details()), None),
    }
}

/// `POST /ofrep/v1/evaluate/flags`: every flag client SDKs may see, or a whole-request failure.
pub fn evaluate_bulk(flags: &OrgFlags, body: &[u8], key_default: Option<&str>, now_ms: i64) -> Result<(Value, Vec<Counted>), (StatusCode, Value)> {
    let ctx = parse_context(body).map_err(|(code, details)| (StatusCode::BAD_REQUEST, failure(None, code, &details)))?;
    let env = environment(flags, &ctx, key_default).map_err(|e| (StatusCode::BAD_REQUEST, failure(None, e.code(), &e.details())))?;
    let mut out = Vec::new();
    let mut counts = Vec::new();
    for flag in flags.flags.values().filter(|f| f.answers(Surface::Client)) {
        match evaluate_in(flag, env, &ctx, now_ms) {
            Ok(r) => {
                out.push(success(&flag.key, &r, env));
                counts.push(counted(&flag.key, &r, env));
            }
            Err(e) => out.push(failure(Some(&flag.key), e.code(), &e.details())),
        }
    }
    Ok((json!({ "flags": out, "metadata": { "environment": env.name } }), counts))
}

/// A strong ETag of an answer body.
pub fn etag(body: &[u8]) -> String {
    let digest = Sha256::digest(body);
    let hex: String = digest[..16].iter().map(|b| format!("{b:02x}")).collect();
    format!("\"{hex}\"")
}

/// Whether `If-None-Match` names `tag` (a list, weak tags or `*`).
pub fn not_modified(if_none_match: Option<&str>, tag: &str) -> bool {
    let Some(header) = if_none_match else {
        return false;
    };
    let bare = |t: &str| t.trim().trim_start_matches("W/").trim_matches('"').to_string();
    header.split(',').any(|t| t.trim() == "*" || bare(t) == bare(tag))
}

// ================================================================ storage

#[derive(sqlx::FromRow)]
struct EnvRow {
    id: String,
    name: String,
    queries: Vec<String>,
}

#[derive(sqlx::FromRow)]
struct FlagRow {
    key: String,
    sdk_access: String,
    variants: sqlx::types::Json<Vec<FlagVariant>>,
}

#[derive(sqlx::FromRow)]
struct ConfigRow {
    key: String,
    environment_id: String,
    enabled: bool,
    config: sqlx::types::Json<Value>,
}

async fn load(pg: &sqlx::PgPool, org: &str) -> Result<OrgFlags, sqlx::Error> {
    let envs: Vec<EnvRow> = sqlx::query_as("SELECT id::text AS id, name, queries FROM feature_flag_environments WHERE org_id = $1 ORDER BY lower(name)").bind(org).fetch_all(pg).await?;
    let rows: Vec<FlagRow> = sqlx::query_as("SELECT key, sdk_access, variants FROM feature_flags WHERE org_id = $1").bind(org).fetch_all(pg).await?;
    let configs: Vec<ConfigRow> = sqlx::query_as(
        "SELECT f.key, c.environment_id::text AS environment_id, c.enabled, c.config FROM feature_flag_env_configs c JOIN feature_flags f ON f.id = c.flag_id \
         WHERE c.org_id = $1 AND f.org_id = $1",
    )
    .bind(org)
    .fetch_all(pg)
    .await?;
    let mut flags: BTreeMap<String, Flag> = rows.into_iter().map(|r| (r.key.clone(), Flag { key: r.key, sdk_access: r.sdk_access, variants: r.variants.0, configs: HashMap::new() })).collect();
    for c in configs {
        let Some(flag) = flags.get_mut(&c.key) else { continue };
        match serde_json::from_value::<FlagEnvConfig>(c.config.0) {
            Ok(mut cfg) => {
                cfg.enabled = c.enabled;
                flag.configs.insert(c.environment_id, cfg);
            }
            Err(e) => tracing::warn!(flag = %c.key, error = %e, "unreadable feature flag configuration"),
        }
    }
    Ok(OrgFlags { environments: envs.into_iter().map(|e| Environment { id: e.id, name: e.name, queries: e.queries }).collect(), flags })
}

/// The org's flags, from the cache while fresh.
async fn org_flags(st: &Shared, org: &str) -> Result<Arc<OrgFlags>, sqlx::Error> {
    if let Some((at, f)) = st.flags.cache.lock().unwrap().get(org) {
        if at.elapsed() < FLAGS_TTL {
            return Ok(f.clone());
        }
    }
    let fresh = Arc::new(load(&st.pg, org).await?);
    st.flags.cache.lock().unwrap().insert(org.to_string(), (Instant::now(), fresh.clone()));
    Ok(fresh)
}

fn count(st: &Shared, org: &str, items: Vec<Counted>) {
    let mut counts = st.flags.counts.lock().unwrap();
    for c in items {
        *counts.entry((org.to_string(), c)).or_insert(0) += 1;
    }
}

/// Sends the counts gathered since the last flush to the server, one request per org.
pub async fn flush(st: &Shared) {
    let drained: Vec<((String, Counted), u64)> = std::mem::take(&mut *st.flags.counts.lock().unwrap()).into_iter().collect();
    let mut per_org: HashMap<String, Vec<Value>> = HashMap::new();
    for ((org, (flag, variant, env, reason)), n) in drained {
        per_org.entry(org).or_default().push(json!({ "flag": flag, "variant": variant, "environment": env, "reason": reason, "count": n }));
    }
    for (org, evaluations) in per_org {
        let mut req = st.http.post(&st.cfg.flag_evaluations_url).header(crate::ORG_HEADER, &org).json(&json!({ "evaluations": evaluations }));
        if let Some(t) = &st.cfg.internal_token {
            req = req.bearer_auth(t);
        }
        match req.send().await {
            Ok(res) if res.status().is_success() => {}
            Ok(res) => tracing::warn!(status = %res.status(), "server refused feature flag evaluation counts"),
            Err(e) => tracing::warn!(error = %e, "feature flag evaluation counts not sent"),
        }
    }
}

// ================================================================ handlers

/// OFREP's `X-API-Key` header, accepted like `OC-API-Key`.
fn with_ofrep_key(mut headers: HeaderMap) -> HeaderMap {
    if let Some(v) = headers.get("x-api-key").cloned() {
        if crate::api_key(&headers).is_none() {
            headers.insert("oc-api-key", v);
        }
    }
    headers
}

fn json_response(status: StatusCode, body: &Value) -> Response {
    (status, [(header::CONTENT_TYPE, "application/json")], body.to_string()).into_response()
}

fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64)
}

fn unavailable(e: sqlx::Error, key: Option<&str>) -> Response {
    tracing::error!(error = %e, "loading feature flags failed");
    json_response(StatusCode::INTERNAL_SERVER_ERROR, &failure(key, "GENERAL", "feature flags are temporarily unavailable"))
}

/// `POST /ofrep/v1/evaluate/flags/{key}`.
pub async fn evaluate_flag(State(st): State<Shared>, Path(key): Path<String>, headers: HeaderMap, body: Bytes) -> Response {
    let headers = with_ofrep_key(headers);
    let org = match crate::admit(&st, &headers, body.len()).await {
        Ok(org) => org,
        Err(r) => return r.into_response(),
    };
    let flags = match org_flags(&st, &org.org_id).await {
        Ok(f) => f,
        Err(e) => return unavailable(e, Some(&key)),
    };
    let (status, answer, c) = evaluate_single(&flags, &key, &body, org.flag_environment_id.as_deref(), now_ms());
    if let Some(c) = c {
        count(&st, &org.org_id, vec![c]);
    }
    json_response(status, &answer)
}

/// `POST /ofrep/v1/evaluate/flags`.
pub async fn evaluate_flags(State(st): State<Shared>, headers: HeaderMap, body: Bytes) -> Response {
    let headers = with_ofrep_key(headers);
    let org = match crate::admit(&st, &headers, body.len()).await {
        Ok(org) => org,
        Err(r) => return r.into_response(),
    };
    let flags = match org_flags(&st, &org.org_id).await {
        Ok(f) => f,
        Err(e) => return unavailable(e, None),
    };
    match evaluate_bulk(&flags, &body, org.flag_environment_id.as_deref(), now_ms()) {
        Ok((answer, counts)) => {
            let text = answer.to_string();
            let tag = etag(text.as_bytes());
            let if_none_match = headers.get(header::IF_NONE_MATCH).and_then(|v| v.to_str().ok());
            let mut res = if not_modified(if_none_match, &tag) {
                StatusCode::NOT_MODIFIED.into_response()
            } else {
                count(&st, &org.org_id, counts);
                (StatusCode::OK, [(header::CONTENT_TYPE, "application/json")], text).into_response()
            };
            if let Ok(v) = HeaderValue::from_str(&tag) {
                res.headers_mut().insert(header::ETAG, v);
            }
            res
        }
        Err((status, answer)) => json_response(status, &answer),
    }
}

/// Browser SDKs call OFREP from other origins: CORS for the OFREP routes.
pub fn cors() -> tower_http::cors::CorsLayer {
    use tower_http::cors::{AllowOrigin, CorsLayer};
    CorsLayer::new()
        .allow_origin(AllowOrigin::mirror_request())
        .allow_methods([axum::http::Method::POST, axum::http::Method::OPTIONS])
        .allow_headers([header::AUTHORIZATION, header::CONTENT_TYPE, header::IF_NONE_MATCH, header::HeaderName::from_static("x-api-key"), header::HeaderName::from_static("oc-api-key")])
        .expose_headers([header::ETAG])
        .max_age(Duration::from_secs(3600))
}

#[cfg(test)]
mod tests {
    use super::*;
    use oc_flags::{FlagServe, FlagSplit};

    fn env(id: &str, name: &str, queries: &[&str]) -> Environment {
        Environment { id: id.into(), name: name.into(), queries: queries.iter().map(|q| q.to_string()).collect() }
    }

    fn bool_flag(key: &str, sdk: &str, configs: &[(&str, FlagEnvConfig)]) -> Flag {
        Flag {
            key: key.into(),
            sdk_access: sdk.into(),
            variants: vec![FlagVariant { key: "on".into(), value: json!(true) }, FlagVariant { key: "off".into(), value: json!(false) }],
            configs: configs.iter().map(|(e, c)| (e.to_string(), c.clone())).collect(),
        }
    }

    fn on(variant: &str) -> FlagEnvConfig {
        FlagEnvConfig { enabled: true, rules: vec![], default_serve: FlagServe::variant(variant), rollout: None }
    }

    fn org() -> OrgFlags {
        let split = FlagEnvConfig {
            enabled: true,
            rules: vec![],
            default_serve: FlagServe { variant: None, split: vec![FlagSplit { variant: "on".into(), weight: 50.0 }, FlagSplit { variant: "off".into(), weight: 50.0 }] },
            rollout: None,
        };
        let flags = [
            bool_flag("checkout-new-payment-flow", "all", &[("prod", on("on")), ("stg", on("off"))]),
            bool_flag("catalog-search-v2", "all", &[("prod", split)]),
            bool_flag("billing-internal-ledger", "server", &[("prod", on("on"))]),
            bool_flag("storefront-dark-mode", "client", &[("prod", FlagEnvConfig { enabled: false, ..on("on") })]),
        ];
        OrgFlags { environments: vec![env("prod", "Production", &["production", "prod"]), env("stg", "Staging", &["staging-*"])], flags: flags.into_iter().map(|f| (f.key.clone(), f)).collect() }
    }

    fn body(ctx: Value) -> Vec<u8> {
        json!({ "context": ctx }).to_string().into_bytes()
    }

    #[test]
    fn single_flag_success_shape() {
        let (status, v, c) = evaluate_single(&org(), "checkout-new-payment-flow", &body(json!({"targetingKey": "user-1", "env": "production"})), None, 0);
        assert_eq!(status, StatusCode::OK);
        assert_eq!(v, json!({"key": "checkout-new-payment-flow", "reason": "STATIC", "variant": "on", "value": true, "metadata": {"environment": "Production"}}));
        assert_eq!(c, Some(("checkout-new-payment-flow".into(), "on".into(), "Production".into(), "STATIC")));
        // The environment comes from the context's env, matched by its queries.
        let (_, v, _) = evaluate_single(&org(), "checkout-new-payment-flow", &body(json!({"environment": "staging-eu"})), None, 0);
        assert_eq!(v["value"], json!(false));
    }

    #[test]
    fn single_flag_errors() {
        let o = org();
        let (status, v, c) = evaluate_single(&o, "nope", &body(json!({"env": "prod"})), None, 0);
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(v, json!({"key": "nope", "errorCode": "FLAG_NOT_FOUND", "errorDetails": "flag 'nope' was not found"}));
        assert!(c.is_none());
        // Client-only flags are invisible to server SDKs.
        assert_eq!(evaluate_single(&o, "storefront-dark-mode", &body(json!({"env": "prod"})), None, 0).0, StatusCode::NOT_FOUND);
        let (status, v, _) = evaluate_single(&o, "catalog-search-v2", &body(json!({"env": "prod"})), None, 0);
        assert_eq!((status, v["errorCode"].as_str()), (StatusCode::BAD_REQUEST, Some("TARGETING_KEY_MISSING")));
        let (status, v, _) = evaluate_single(&o, "catalog-search-v2", b"{not json", None, 0);
        assert_eq!((status, v["errorCode"].as_str()), (StatusCode::BAD_REQUEST, Some("PARSE_ERROR")));
        let (_, v, _) = evaluate_single(&o, "catalog-search-v2", br#"{"context": 5}"#, None, 0);
        assert_eq!(v["errorCode"], "INVALID_CONTEXT");
        let (_, v, _) = evaluate_single(&o, "catalog-search-v2", &body(json!({"targetingKey": 5, "env": "prod"})), None, 0);
        assert_eq!(v["errorCode"], "INVALID_CONTEXT");
        let (_, v, _) = evaluate_single(&o, "catalog-search-v2", &body(json!({"env": "qa"})), None, 0);
        assert_eq!((v["errorCode"].as_str(), v["errorDetails"].as_str()), (Some("INVALID_CONTEXT"), Some("no feature flag environment matches env 'qa'")));
        // Several environments and none named: the context must say which.
        let (_, v, _) = evaluate_single(&o, "checkout-new-payment-flow", &body(json!({})), None, 0);
        assert_eq!(v["errorCode"], "INVALID_CONTEXT");
    }

    #[test]
    fn disabled_flags_defer_to_the_code_default() {
        let mut o = org();
        o.flags.get_mut("checkout-new-payment-flow").unwrap().configs.get_mut("stg").unwrap().enabled = false;
        let (status, v, c) = evaluate_single(&o, "checkout-new-payment-flow", &body(json!({"env": "staging-us"})), None, 0);
        assert_eq!(status, StatusCode::OK);
        assert_eq!(v, json!({"key": "checkout-new-payment-flow", "reason": "DISABLED", "metadata": {"environment": "Staging"}}));
        assert_eq!(c.unwrap().3, "DISABLED");
        // No configuration for an environment means off.
        let (_, v, _) = evaluate_single(&o, "billing-internal-ledger", &body(json!({"env": "staging-us"})), None, 0);
        assert_eq!(v["reason"], "DISABLED");
    }

    #[test]
    fn environment_fallbacks() {
        let o = org();
        let ctx = Map::new();
        assert_eq!(environment(&o, &ctx, Some("stg")).unwrap().name, "Staging");
        // A default the org no longer has is ignored.
        assert!(environment(&o, &ctx, Some("gone")).is_err());
        let single = OrgFlags { environments: vec![env("prod", "Production", &["production"])], ..o.clone() };
        assert_eq!(environment(&single, &ctx, None).unwrap().name, "Production");
        // The context wins over the key's default.
        let named: Map<String, Value> = json!({"env": "prod"}).as_object().unwrap().clone();
        assert_eq!(environment(&o, &named, Some("stg")).unwrap().name, "Production");
        assert!(environment(&OrgFlags::default(), &ctx, None).is_err());
    }

    #[test]
    fn bulk_answers_client_flags_with_per_flag_failures() {
        let (v, counts) = evaluate_bulk(&org(), &body(json!({"env": "prod"})), None, 0).unwrap();
        let keys: Vec<&str> = v["flags"].as_array().unwrap().iter().map(|f| f["key"].as_str().unwrap()).collect();
        // Sorted by key; the server-only flag is left out.
        assert_eq!(keys, ["catalog-search-v2", "checkout-new-payment-flow", "storefront-dark-mode"]);
        assert_eq!(v["flags"][0], json!({"key": "catalog-search-v2", "errorCode": "TARGETING_KEY_MISSING", "errorDetails": "a percentage split needs the context's targetingKey"}));
        assert_eq!(v["flags"][2]["reason"], "DISABLED");
        assert_eq!(v["metadata"], json!({"environment": "Production"}));
        assert_eq!(counts.len(), 2);
        let (status, v) = evaluate_bulk(&org(), &body(json!({"env": "qa"})), None, 0).unwrap_err();
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(v["errorCode"], "INVALID_CONTEXT");
        assert!(v.get("key").is_none());
    }

    #[test]
    fn bulk_split_is_sticky_per_targeting_key() {
        let a = evaluate_bulk(&org(), &body(json!({"env": "prod", "targetingKey": "user-42"})), None, 0).unwrap().0;
        let b = evaluate_bulk(&org(), &body(json!({"env": "prod", "targetingKey": "user-42"})), None, 0).unwrap().0;
        assert_eq!(a, b);
        assert_eq!(a["flags"][0]["reason"], "SPLIT");
        assert_eq!(etag(a.to_string().as_bytes()), etag(b.to_string().as_bytes()));
    }

    #[test]
    fn etags() {
        let tag = etag(b"{}");
        assert!(tag.starts_with('"') && tag.ends_with('"') && tag.len() == 34);
        assert_ne!(tag, etag(b"{ }"));
        assert!(not_modified(Some(&tag), &tag));
        assert!(not_modified(Some(&format!("W/{tag}")), &tag));
        assert!(not_modified(Some(&format!("\"x\", {tag}")), &tag));
        assert!(not_modified(Some(tag.trim_matches('"')), &tag));
        assert!(not_modified(Some("*"), &tag));
        assert!(!not_modified(Some("\"other\""), &tag));
        assert!(!not_modified(None, &tag));
    }

    #[test]
    fn contexts() {
        assert_eq!(parse_context(b"").unwrap(), Map::new());
        assert_eq!(parse_context(b"{}").unwrap(), Map::new());
        assert_eq!(parse_context(br#"{"context": {"a": 1}}"#).unwrap()["a"], json!(1));
        assert_eq!(parse_context(b"[1]").unwrap_err().0, "PARSE_ERROR");
    }

    #[test]
    fn ofrep_api_key_header() {
        let mut h = HeaderMap::new();
        h.insert("x-api-key", HeaderValue::from_static("ocik_abc"));
        assert_eq!(crate::api_key(&with_ofrep_key(h)), Some("ocik_abc"));
    }
}
