//! Push connector packages (docs/connectors.md#push-only-vendors-and-webhooks): vendors that send to the intake
//! (webhooks, log drains) as a package declares, read from the server's internal listener:
//!
//! - `GET {OC_INTAKE_CONNECTORS_URL}/push`: every loaded package's push definition (bundled and the operator's),
//!   cached for a minute; the last good list is kept while the server can't be reached.
//! - `GET {OC_INTAKE_CONNECTORS_URL}/push/{id}/secrets` with `x-oc-org`: the signing secrets of the org's enabled
//!   connections of that package (unsealed by the server), cached for a minute per org and package. The org is the
//!   one of the request's ingest key, never anything the request says.
//!
//! `POST /intake/webhook/<name>` with a package's `webhook` maps the body to logs and events; `POST
//! /api/v2/logs?ddsource=<source>` of a package with a signed `push` checks the signature before the logs go on.
//! When a package declares a signature and the org saved a signing secret, unsigned or wrongly signed requests are
//! refused (401); without a saved secret the ingest key alone admits them.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::body::Bytes;
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use oc_connectors::push::{self, PushDef};
use serde::Deserialize;
use serde_json::{json, Map, Value};

use crate::datadog_events::{deliver, Event};
use crate::kafka::Sink;
use crate::{send_prepared, KeyOrg, Rejection, Shared};

/// How long definitions and secrets are trusted (a new secret applies within this).
const TTL: Duration = Duration::from_secs(60);

type Secrets = Arc<Vec<String>>;

#[derive(Default)]
pub struct PushState {
    defs: Mutex<Option<(Instant, Arc<Vec<PushDef>>)>>,
    /// When the first load last failed (retried after 10 s).
    failed: Mutex<Option<Instant>>,
    secrets: Mutex<HashMap<(String, String), (Instant, Secrets)>>,
}

#[derive(Deserialize)]
struct DefsBody {
    packages: Vec<PushDef>,
}

#[derive(Deserialize)]
struct SecretsBody {
    secrets: Vec<String>,
}

const UNAVAILABLE: Rejection = Rejection { status: StatusCode::SERVICE_UNAVAILABLE, message: "connector definitions unavailable, retry later" };

fn get(st: &Shared, url: String) -> reqwest::RequestBuilder {
    let req = st.http.get(url).timeout(Duration::from_secs(10));
    match &st.cfg.internal_token {
        Some(t) => req.bearer_auth(t),
        None => req,
    }
}

/// The packages' push definitions: fresh, else fetched, else the last good ones.
pub async fn defs(st: &Shared) -> Result<Arc<Vec<PushDef>>, Rejection> {
    let cached = st.push.defs.lock().unwrap().clone();
    if let Some((at, d)) = &cached {
        if at.elapsed() < TTL {
            return Ok(d.clone());
        }
    }
    if cached.is_none() && st.push.failed.lock().unwrap().is_some_and(|t| t.elapsed() < Duration::from_secs(10)) {
        return Err(UNAVAILABLE);
    }
    let fetched = async {
        let res = get(st, format!("{}/push", st.cfg.connectors_url)).send().await.map_err(|e| e.to_string())?;
        if !res.status().is_success() {
            return Err(format!("status {}", res.status()));
        }
        res.json::<DefsBody>().await.map_err(|e| e.to_string())
    }
    .await;
    match fetched {
        Ok(body) => {
            let d = Arc::new(body.packages);
            *st.push.defs.lock().unwrap() = Some((Instant::now(), d.clone()));
            Ok(d)
        }
        Err(e) => {
            tracing::warn!(error = %e, "connector push definitions unavailable");
            match cached {
                Some((_, d)) => {
                    // Kept, retried within 10 s.
                    *st.push.defs.lock().unwrap() = Some((Instant::now() - TTL + Duration::from_secs(10), d.clone()));
                    Ok(d)
                }
                None => {
                    *st.push.failed.lock().unwrap() = Some(Instant::now());
                    Err(UNAVAILABLE)
                }
            }
        }
    }
}

/// The org's signing secrets of a package.
async fn secrets(st: &Shared, org: &str, id: &str) -> Result<Secrets, Rejection> {
    let key = (org.to_string(), id.to_string());
    if let Some((at, s)) = st.push.secrets.lock().unwrap().get(&key) {
        if at.elapsed() < TTL {
            return Ok(s.clone());
        }
    }
    let fetched = async {
        let res = get(st, format!("{}/push/{id}/secrets", st.cfg.connectors_url)).header(crate::ORG_HEADER, org).send().await.map_err(|e| e.to_string())?;
        if !res.status().is_success() {
            return Err(format!("status {}", res.status()));
        }
        res.json::<SecretsBody>().await.map_err(|e| e.to_string())
    }
    .await;
    match fetched {
        Ok(body) => {
            let s = Arc::new(body.secrets);
            st.push.secrets.lock().unwrap().insert(key, (Instant::now(), s.clone()));
            Ok(s)
        }
        Err(e) => {
            // Fail closed: a signature can't be checked without the secrets.
            tracing::warn!(error = %e, connector = id, "connector signing secrets unavailable");
            Err(UNAVAILABLE)
        }
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64)
}

/// Checks a request against the package's signature, with the org's saved secrets.
pub async fn check(st: &Shared, org: &KeyOrg, def: &PushDef, headers: &HeaderMap, body: &[u8]) -> Result<(), Rejection> {
    let Some(sig) = def.signature() else { return Ok(()) };
    let saved = secrets(st, &org.org_id, &def.id).await?;
    if saved.is_empty() {
        return Ok(());
    }
    let value = headers.get(sig.header.as_str()).and_then(|v| v.to_str().ok());
    push::verify(sig, value, body, &saved, now_ms() / 1000).map_err(|message| Rejection { status: StatusCode::UNAUTHORIZED, message })
}

/// The package taking `/intake/webhook/<name>`, if any.
pub async fn webhook_def(st: &Shared, name: &str) -> Result<Option<PushDef>, Rejection> {
    Ok(defs(st).await?.iter().find(|d| d.takes_webhook() && d.name == name).cloned())
}

/// The package whose signed logs are sent as `ddsource=<source>`, if any. Before the definitions could ever be read
/// (the server unreachable since the intake started), none: logs with a `ddsource` don't wait for the server, the
/// ingest key admits them.
pub async fn logs_def(st: &Shared, source: &str) -> Option<PushDef> {
    defs(st).await.ok()?.iter().find(|d| d.signs_logs() && d.source == source).cloned()
}

/// A webhook of a package: signature, mapping, logs to Kafka (with their record identities) and events.
pub async fn post_webhook(st: &Shared, org: &KeyOrg, def: &PushDef, headers: &HeaderMap, raw: &[u8]) -> Response {
    if let Err(r) = check(st, org, def, headers, raw).await {
        return r.into_response();
    }
    let body: Value = if raw.iter().all(u8::is_ascii_whitespace) {
        Value::Null
    } else {
        match serde_json::from_slice(raw) {
            Ok(v) => v,
            Err(e) => return (StatusCode::BAD_REQUEST, Json(json!({ "errors": [format!("invalid JSON payload: {e}")] }))).into_response(),
        }
    };
    let request_headers: Map<String, Value> = headers
        .iter()
        .filter(|(k, _)| !matches!(k.as_str(), "authorization" | "dd-api-key" | "oc-api-key" | "cookie"))
        .filter_map(|(k, v)| Some((k.as_str().to_string(), Value::String(v.to_str().ok()?.to_string()))))
        .collect();
    let now = now_ms();
    let mut mapped = match push::webhook(def, &body, &request_headers, now) {
        Ok(m) => m,
        Err(e) => {
            tracing::warn!(connector = %def.id, error = %e, "webhook mapping failed");
            return (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "errors": ["the connector's webhook mapping is invalid"] }))).into_response();
        }
    };
    if !mapped.logs.is_empty() {
        let received = chrono::DateTime::from_timestamp_millis(now).map(|t| t.to_rfc3339()).unwrap_or_default();
        for log in &mut mapped.logs {
            if let Some(ingest) = log.get_mut("_oc_ingest").and_then(Value::as_object_mut) {
                ingest.insert("received_at".into(), Value::String(received.clone()));
            }
        }
        let body = Bytes::from(serde_json::to_vec(&mapped.logs).unwrap_or_default());
        match send_prepared(st, org, Sink::JsonLogs, Some(&HeaderValue::from_static("application/json")), None, body, &[]).await {
            Ok(up) if up.status.is_success() => {}
            Ok(up) => {
                tracing::warn!(status = %up.status, "webhook logs refused");
                return Rejection { status: StatusCode::SERVICE_UNAVAILABLE, message: "logs were not accepted" }.into_response();
            }
            Err(r) => return r.into_response(),
        }
    }
    let events: Vec<Event> = mapped
        .events
        .into_iter()
        .map(|e| Event {
            title: e.title,
            text: e.text,
            timestamp: e.time_ms / 1000,
            host: e.host,
            tags: e.tags,
            alert_type: e.alert_type,
            priority: e.priority,
            source_type_name: e.source_type_name,
            aggregation_key: e.aggregation_key,
            event_type: String::new(),
        })
        .collect();
    match deliver(st, org, &events).await {
        Ok(()) => (StatusCode::ACCEPTED, [(header::CONTENT_TYPE, "application/json")], r#"{"status":"ok"}"#).into_response(),
        Err(r) => r.into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{test_state, CachedKey, Config};
    use axum::routing::{get as route_get, post};
    use axum::Router;
    use sha2::{Digest, Sha256};

    const PACKAGE: &str = "schema_version: 1\nid: example-billing\nname: Example Billing\npublisher: Example\nvendor: Example\ndescription: Billing events.\n\
        config:\n  - { key: signing_secret, label: Signing secret, type: string, secret: true }\n\
        push: { kind: webhook, instructions: Add a webhook, signature: { header: X-Hub-Signature-256, algorithm: hmac_sha256, prefix: 'sha256=', secret: signing_secret } }\n\
        webhook:\n  record_id: \"{{ record.id }}\"\n  logs: { message: \"{{ record.type }}\", date: \"{{ record.created }}\" }\n  events: { title: \"{{ record.type }}\", text: \"{{ record.id }}\" }\n";

    fn org(id: &str) -> KeyOrg {
        KeyOrg { org_id: id.into(), span_retention_days: 15, metrics_retention_days: 30, metrics_5m_days: 90, metrics_1h_days: 395, flag_environment_id: None, ingest_mbps: None, ingest_rps: None }
    }

    fn sign(secret: &str, body: &[u8]) -> String {
        use hmac::{KeyInit, Mac};
        let mut mac = hmac::Hmac::<Sha256>::new_from_slice(secret.as_bytes()).unwrap();
        mac.update(body);
        format!("sha256={}", mac.finalize().into_bytes().iter().map(|b| format!("{b:02x}")).collect::<String>())
    }

    /// The server's internal listener (definitions, org A's secret, B has none) and the log pipeline, faked; the
    /// webhook route as the intake serves it.
    #[tokio::test]
    async fn package_webhooks_are_checked_with_the_orgs_secret_and_mapped() {
        let m: oc_connectors::manifest::Manifest = serde_yaml_ng::from_str(PACKAGE).unwrap();
        let def = PushDef::of(&m).unwrap();
        let defs = json!({ "packages": [def] });
        let server = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let server_url = format!("http://{}", server.local_addr().unwrap());
        let logs: Arc<Mutex<Vec<Value>>> = Arc::default();
        let seen = logs.clone();
        let asked: Arc<Mutex<Vec<String>>> = Arc::default();
        let asked_by = asked.clone();
        // Org A's signing secret, made for this run.
        let secret = format!("{:032x}", rand::random::<u128>());
        let saved = secret.clone();
        tokio::spawn(async move {
            let app = Router::new()
                .route("/ingest/connectors/push", route_get(move || async move { Json(defs) }))
                .route(
                    "/ingest/connectors/push/{id}/secrets",
                    route_get(move |headers: HeaderMap| async move {
                        let org = headers[crate::ORG_HEADER].to_str().unwrap().to_string();
                        asked_by.lock().unwrap().push(org.clone());
                        Json(json!({ "secrets": if org == "org-a" { vec![saved] } else { vec![] } }))
                    }),
                )
                .route(
                    "/logs",
                    post(move |body: Bytes| async move {
                        seen.lock().unwrap().extend(serde_json::from_slice::<Vec<Value>>(&body).unwrap());
                        "{}"
                    }),
                )
                .route("/v1/logs", post(|| async { "" }));
            axum::serve(server, app).await.unwrap();
        });
        let mut cfg = Config::from_env();
        cfg.connectors_url = format!("{server_url}/ingest/connectors");
        cfg.logs_url = format!("{server_url}/logs");
        cfg.collector_url = server_url.clone();
        let st = test_state(cfg);
        for (key, o) in [("ocik_push_a", "org-a"), ("ocik_push_b", "org-b")] {
            st.keys.lock().unwrap().insert(Sha256::digest(key.as_bytes()).to_vec(), CachedKey { org: Some(org(o)), at: Instant::now(), touched: Some(Instant::now()) });
        }
        let call = |key: &'static str, signature: Option<String>, body: &'static [u8]| {
            let st = st.clone();
            async move {
                let mut h = HeaderMap::new();
                h.insert("dd-api-key", HeaderValue::from_static(key));
                h.insert("x-org-claim", HeaderValue::from_static("org-b"));
                if let Some(s) = signature {
                    h.insert("x-hub-signature-256", HeaderValue::from_str(&s).unwrap());
                }
                crate::datadog_webhooks::post(axum::extract::State(st), axum::extract::Path("Example-Billing".into()), h, Bytes::from_static(body)).await.status()
            }
        };
        let body: &[u8] = br#"{"id":"evt_1","type":"invoice.paid","created":1794312000}"#;
        assert_eq!(call("ocik_push_a", Some(sign(&secret, body)), body).await, StatusCode::ACCEPTED);
        assert_eq!(call("ocik_push_a", None, body).await, StatusCode::UNAUTHORIZED, "A saved a secret: unsigned is refused");
        assert_eq!(call("ocik_push_a", Some(sign(&secret, b"other")), body).await, StatusCode::UNAUTHORIZED);
        assert_eq!(call("ocik_push_b", None, body).await, StatusCode::ACCEPTED, "B saved none: its key admits it");
        assert_eq!(call("ocik_push_b", Some("sha256=00".into()), body).await, StatusCode::ACCEPTED);
        let got = logs.lock().unwrap().clone();
        assert_eq!(got.len(), 3);
        assert_eq!((&got[0]["message"], &got[0]["ddsource"]), (&json!("invoice.paid"), &json!("example-billing")));
        assert_eq!(got[0]["_oc_ingest"]["id"], got[1]["_oc_ingest"]["id"], "a record's identity is its id and date: retries are stored once");
        assert!(got[0]["_oc_ingest"]["received_at"].is_string());
        // Secrets are asked once a minute per org, for the key's org only.
        assert_eq!(asked.lock().unwrap().clone(), ["org-a", "org-b"]);
        // Other names keep the generic mapping.
        assert_eq!(webhook_def(&st, "zabbix").await.ok().flatten().map(|d| d.id), None);
    }
}
