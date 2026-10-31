//! Custom destinations (Logs → Configuration → Archiving & Forwarding): the log indexer forwards
//! every processed log matching a destination's query to an HTTP endpoint, Splunk's HTTP Event
//! Collector or Elasticsearch, batched every 5 s.
//!
//! - Logs are shaped as Datadog forwards them: `message`, `status`, `service`, `host`, `ddsource`,
//!   `ddtags` (filtered by the destination's tag list), `timestamp` and the log's attributes.
//! - Destinations are reloaded with the other indexer settings (10 s). Each holds at most
//!   [`MAX_QUEUED`] logs between flushes; more are dropped and reported as the destination's error.
//! - Targets are resolved and checked like synthetic tests' (no private or loopback addresses unless
//!   `OC_LOG_FORWARD_ALLOW_PRIVATE=true`), and the connection is pinned to the checked address.
//! - Microsoft Sentinel needs Azure credentials ObserveCraft doesn't hold yet: those destinations
//!   are stored but not forwarded to (their error says so).

use std::collections::HashMap;
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use oc_query::{matches, parse_query, Fields, Term};
use serde_json::{json, Map, Value};

use crate::routes::synthetics_net;
use crate::state::AppState;

/// Logs held per destination between flushes.
pub const MAX_QUEUED: usize = 20_000;
/// Logs per request.
const BATCH: usize = 500;
pub const SENTINEL_UNSUPPORTED: &str = "Forwarding to Microsoft Sentinel needs Azure credentials, which ObserveCraft doesn't hold yet: logs aren't sent";

#[derive(Clone, Debug, PartialEq)]
pub enum Target {
    Http { endpoint: String, auth: HttpAuth },
    Splunk { endpoint: String, token: String, sourcetype: SourceType },
    Elastic { endpoint: String, index: String, rotation: Option<String>, user: String, password: String },
    Sentinel,
}

#[derive(Clone, Debug, PartialEq)]
pub enum HttpAuth {
    None,
    Basic { user: String, password: String },
    Header { name: String, value: String },
}

/// Splunk's source type: not configured (`_json`), explicitly none, or a value.
#[derive(Clone, Debug, PartialEq)]
pub enum SourceType {
    Default,
    Omitted,
    Value(String),
}

pub struct Destination {
    pub id: String,
    terms: Vec<Term>,
    pub target: Target,
    forward_tags: bool,
    tags_list: Vec<String>,
    allow_list: bool,
}

impl Destination {
    /// The destination's target from its stored document (Datadog's `forwarder_destination`
    /// without secrets) and secrets.
    pub fn target(destination: &Value, secrets: &Value) -> Target {
        let s = |v: &Value, k: &str| v.get(k).and_then(Value::as_str).unwrap_or("").to_string();
        let endpoint = s(destination, "endpoint");
        match s(destination, "type").as_str() {
            "splunk_hec" => Target::Splunk {
                endpoint,
                token: s(secrets, "access_token"),
                sourcetype: match destination.get("sourcetype") {
                    None => SourceType::Default,
                    Some(Value::Null) => SourceType::Omitted,
                    Some(v) => SourceType::Value(v.as_str().unwrap_or("").to_string()),
                },
            },
            "elasticsearch" => Target::Elastic {
                endpoint,
                index: s(destination, "index_name"),
                rotation: destination.get("index_rotation").and_then(Value::as_str).filter(|r| !r.is_empty()).map(str::to_string),
                user: s(secrets, "username"),
                password: s(secrets, "password"),
            },
            "microsoft_sentinel" => Target::Sentinel,
            _ => {
                let auth = destination.get("auth").cloned().unwrap_or(Value::Null);
                Target::Http {
                    endpoint,
                    auth: match s(&auth, "type").as_str() {
                        "basic" => HttpAuth::Basic { user: s(secrets, "username"), password: s(secrets, "password") },
                        "custom_header" => HttpAuth::Header { name: s(&auth, "header_name"), value: s(secrets, "header_value") },
                        _ => HttpAuth::None,
                    },
                }
            }
        }
    }

    /// The log as Datadog forwards it.
    pub fn payload(&self, ev: &Value) -> Value {
        let s = |k: &str| ev.get(k).and_then(Value::as_str).unwrap_or("").to_string();
        let mut out = match ev.get("attributes") {
            Some(Value::String(a)) => serde_json::from_str::<Map<String, Value>>(a).unwrap_or_default(),
            Some(Value::Object(a)) => a.clone(),
            _ => Map::new(),
        };
        for (k, v) in [("message", s("message")), ("status", s("status")), ("service", s("service")), ("host", s("host")), ("ddsource", s("source"))] {
            if !v.is_empty() {
                out.insert(k.into(), Value::String(v));
            }
        }
        let ts = ev.get("ts_ms").and_then(Value::as_i64).or_else(|| ev.get("timestamp").and_then(Value::as_i64));
        if let Some(ts) = ts {
            out.insert("timestamp".into(), Value::from(ts));
        }
        if self.forward_tags {
            let tags: Vec<String> = ev
                .get("tags")
                .and_then(Value::as_object)
                .map(|t| {
                    let mut kv: Vec<(&String, &Value)> = t.iter().filter(|(k, _)| self.tags_list.is_empty() || self.tags_list.iter().any(|l| l == *k) == self.allow_list).collect();
                    kv.sort_by(|a, b| a.0.cmp(b.0));
                    kv.into_iter().map(|(k, v)| format!("{k}:{}", v.as_str().map(str::to_string).unwrap_or_else(|| v.to_string()))).collect()
                })
                .unwrap_or_default();
            if !tags.is_empty() {
                out.insert("ddtags".into(), Value::String(tags.join(",")));
            }
        }
        Value::Object(out)
    }
}

#[derive(sqlx::FromRow)]
struct Row {
    id: String,
    org_id: String,
    query: String,
    forward_tags: bool,
    tags_list: Value,
    tags_list_type: String,
    destination: Value,
    secrets_enc: String,
}

#[derive(Default)]
pub struct Forwarder {
    by_org: RwLock<HashMap<String, Arc<Vec<Arc<Destination>>>>>,
    queue: Mutex<HashMap<String, Queued>>,
}

struct Queued {
    dest: Arc<Destination>,
    logs: Vec<Value>,
    dropped: u64,
}

impl Forwarder {
    pub async fn reload(&self, st: &AppState) -> Result<(), sqlx::Error> {
        let rows: Vec<Row> = sqlx::query_as(
            "SELECT d.id::text AS id, d.org_id, d.query, d.forward_tags, d.tags_list, d.tags_list_type, d.destination, d.secrets_enc \
             FROM log_custom_destinations d JOIN orgs o ON o.id = d.org_id WHERE d.enabled AND o.status = 'active' ORDER BY d.org_id, d.position, d.created_at",
        )
        .fetch_all(&st.postgres)
        .await?;
        let mut by_org: HashMap<String, Vec<Arc<Destination>>> = HashMap::new();
        for r in rows {
            let secrets = if r.secrets_enc.is_empty() { Value::Null } else { st.secrets.decrypt(&r.secrets_enc).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or(Value::Null) };
            let dest = Destination {
                target: Destination::target(&r.destination, &secrets),
                id: r.id,
                terms: parse_query(&r.query),
                forward_tags: r.forward_tags,
                tags_list: serde_json::from_value(r.tags_list).unwrap_or_default(),
                allow_list: r.tags_list_type != "BLOCK_LIST",
            };
            by_org.entry(r.org_id).or_default().push(Arc::new(dest));
        }
        *self.by_org.write().unwrap() = by_org.into_iter().map(|(k, v)| (k, Arc::new(v))).collect();
        Ok(())
    }

    /// Queues a processed log for every destination of its org whose query matches.
    pub fn record(&self, org: &str, f: &Fields<'_>, ev: &Value) {
        let Some(dests) = self.by_org.read().unwrap().get(org).cloned() else { return };
        let matching: Vec<&Arc<Destination>> = dests.iter().filter(|d| d.target != Target::Sentinel && matches(&d.terms, f)).collect();
        if matching.is_empty() {
            return;
        }
        let mut q = self.queue.lock().unwrap();
        for d in matching {
            let e = q.entry(d.id.clone()).or_insert_with(|| Queued { dest: d.clone(), logs: Vec::new(), dropped: 0 });
            if e.logs.len() < MAX_QUEUED {
                e.logs.push(d.payload(ev));
            } else {
                e.dropped += 1;
            }
        }
    }

    /// Sends what's queued; records counts and errors on the destinations.
    pub async fn flush(&self, st: &AppState) {
        let batch: Vec<Queued> = self.queue.lock().unwrap().drain().map(|(_, q)| q).collect();
        for q in batch {
            let mut sent = 0u64;
            let mut error = None;
            for chunk in q.logs.chunks(BATCH) {
                match send(st, &q.dest.target, chunk).await {
                    Ok(()) => sent += chunk.len() as u64,
                    Err(e) => {
                        error = Some(e);
                        break;
                    }
                }
            }
            if error.is_none() && q.dropped > 0 {
                error = Some(format!("{} logs dropped: the destination didn't keep up", q.dropped));
            }
            let _ = sqlx::query(
                "UPDATE log_custom_destinations SET forwarded = forwarded + $2, last_error = COALESCE($3, last_error), \
                 last_error_at = CASE WHEN $3 IS NULL THEN last_error_at ELSE now() END WHERE id = $1::uuid",
            )
            .bind(&q.dest.id)
            .bind(sent as i64)
            .bind(error)
            .execute(&st.postgres)
            .await;
        }
    }
}

/// Starts the flush loop.
pub fn spawn(st: AppState, f: Arc<Forwarder>) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(5));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tick.tick().await;
            f.flush(&st).await;
        }
    });
}

/// `https://host:port/path` → (host, port), checked and resolved; a client pinned to that address.
async fn client(st: &AppState, endpoint: &str) -> Result<reqwest::Client, String> {
    let url = url::Url::parse(endpoint).map_err(|e| format!("Invalid endpoint: {e}"))?;
    if !matches!(url.scheme(), "https" | "http") {
        return Err("The endpoint must be an https URL".into());
    }
    let host = url.host_str().ok_or("The endpoint has no host")?.to_string();
    let port = url.port_or_known_default().unwrap_or(443);
    let addr = synthetics_net::resolve(&host, port, st.config.log_forward_allow_private).await?;
    reqwest::Client::builder().resolve(&host, addr).timeout(Duration::from_secs(15)).build().map_err(|e| e.to_string())
}

/// `<endpoint>` + `path` unless the endpoint already has a path.
fn with_path(endpoint: &str, path: &str) -> String {
    match url::Url::parse(endpoint) {
        Ok(u) if u.path() == "/" || u.path().is_empty() => format!("{}{path}", endpoint.trim_end_matches('/')),
        _ => endpoint.to_string(),
    }
}

/// Elasticsearch index with its date rotation (`yyyy-MM-dd`, `yyyy-MM`, `yyyy-'W'ww`, …) appended.
pub fn index_name(index: &str, rotation: Option<&str>, now: chrono::DateTime<chrono::Utc>) -> String {
    let Some(r) = rotation else { return index.to_string() };
    let week = now.format("%V").to_string();
    let mut out = String::new();
    let mut quoted = false;
    let mut rest = r;
    while let Some(c) = rest.chars().next() {
        if c == '\'' {
            quoted = !quoted;
            rest = &rest[1..];
            continue;
        }
        if !quoted {
            if let Some((tok, val)) = [("yyyy", now.format("%Y").to_string()), ("MM", now.format("%m").to_string()), ("dd", now.format("%d").to_string()), ("HH", now.format("%H").to_string()), ("ww", week.clone())]
                .into_iter()
                .find(|(t, _)| rest.starts_with(t))
            {
                out.push_str(&val);
                rest = &rest[tok.len()..];
                continue;
            }
        }
        out.push(c);
        rest = &rest[c.len_utf8()..];
    }
    format!("{index}-{out}")
}

/// Sends logs to a target.
pub async fn send(st: &AppState, target: &Target, logs: &[Value]) -> Result<(), String> {
    let res = match target {
        Target::Sentinel => return Err(SENTINEL_UNSUPPORTED.into()),
        Target::Http { endpoint, auth } => {
            let mut req = client(st, endpoint).await?.post(endpoint).json(&logs);
            req = match auth {
                HttpAuth::None => req,
                HttpAuth::Basic { user, password } => req.basic_auth(user, Some(password)),
                HttpAuth::Header { name, value } => req.header(name.as_str(), value.as_str()),
            };
            req.send().await
        }
        Target::Splunk { endpoint, token, sourcetype } => {
            let body: String = logs
                .iter()
                .map(|l| {
                    let mut e = json!({ "event": l, "host": l.get("host"), "source": l.get("ddsource") });
                    if let Some(ts) = l.get("timestamp").and_then(Value::as_i64) {
                        e["time"] = json!(ts as f64 / 1000.0);
                    }
                    match sourcetype {
                        SourceType::Default => e["sourcetype"] = json!("_json"),
                        SourceType::Value(v) => e["sourcetype"] = json!(v),
                        SourceType::Omitted => {}
                    }
                    e.to_string()
                })
                .collect::<Vec<_>>()
                .join("\n");
            let url = with_path(endpoint, "/services/collector/event");
            client(st, &url).await?.post(&url).header("Authorization", format!("Splunk {token}")).body(body).send().await
        }
        Target::Elastic { endpoint, index, rotation, user, password } => {
            let name = index_name(index, rotation.as_deref(), chrono::Utc::now());
            let mut body = String::new();
            for l in logs {
                body.push_str(&json!({ "index": { "_index": name } }).to_string());
                body.push('\n');
                body.push_str(&l.to_string());
                body.push('\n');
            }
            let url = with_path(endpoint, "/_bulk");
            let mut req = client(st, &url).await?.post(&url).header("Content-Type", "application/x-ndjson").body(body);
            if !user.is_empty() {
                req = req.basic_auth(user, Some(password));
            }
            req.send().await
        }
    };
    let res = res.map_err(|e| format!("Sending failed: {e}"))?;
    if res.status().is_success() {
        return Ok(());
    }
    let status = res.status();
    let text: String = res.text().await.unwrap_or_default().chars().take(200).collect();
    Err(format!("The destination answered {status}: {text}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn targets_from_documents_and_secrets() {
        let t = Destination::target(&json!({"type": "http", "endpoint": "https://example.com/in", "auth": {"type": "custom_header", "header_name": "X-Key"}}), &json!({"header_value": "s"}));
        assert_eq!(t, Target::Http { endpoint: "https://example.com/in".into(), auth: HttpAuth::Header { name: "X-Key".into(), value: "s".into() } });
        let t = Destination::target(&json!({"type": "splunk_hec", "endpoint": "https://splunk.example.com", "sourcetype": null}), &json!({"access_token": "t"}));
        assert!(matches!(t, Target::Splunk { sourcetype: SourceType::Omitted, .. }));
        assert_eq!(Destination::target(&json!({"type": "microsoft_sentinel"}), &Value::Null), Target::Sentinel);
    }

    #[test]
    fn payloads_filter_tags() {
        let d = Destination { id: "d".into(), terms: vec![], target: Target::Sentinel, forward_tags: true, tags_list: vec!["env".into()], allow_list: true };
        let ev = json!({"message": "m", "service": "billing", "source": "nginx", "tags": {"env": "prod", "team": "pay"}, "attributes": "{\"http\":{\"status_code\":500}}", "ts_ms": 1000});
        let p = d.payload(&ev);
        assert_eq!(p["ddtags"], "env:prod");
        assert_eq!(p["ddsource"], "nginx");
        assert_eq!(p["http"]["status_code"], 500);
        let blocked = Destination { allow_list: false, ..d };
        assert_eq!(blocked.payload(&ev)["ddtags"], "team:pay");
    }

    #[test]
    fn elasticsearch_rotation() {
        let now = chrono::Utc.with_ymd_and_hms(2026, 10, 31, 9, 0, 0).unwrap();
        assert_eq!(index_name("nginx-logs", Some("yyyy-MM-dd"), now), "nginx-logs-2026-10-31");
        assert_eq!(index_name("x", Some("yyyy-'W'ww"), now), "x-2026-W44");
        assert_eq!(index_name("x", None, now), "x");
        assert_eq!(with_path("https://es.example.com", "/_bulk"), "https://es.example.com/_bulk");
        assert_eq!(with_path("https://es.example.com/custom", "/_bulk"), "https://es.example.com/custom");
    }
}
