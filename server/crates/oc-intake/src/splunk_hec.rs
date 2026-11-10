//! A Splunk HTTP Event Collector-compatible endpoint, for senders that only stream to Splunk (GitHub Enterprise's
//! audit log streaming, Splunk logging drivers and forwarders), authenticated with the ingest key as the HEC token
//! (`Authorization: Splunk ocik_…`). The events become JSON logs (`oc.logs.json`) like `/api/v2/logs`:
//!
//! - `POST /services/collector[/event[/1.0]]`: HEC JSON events, concatenated or one per line (`{"time": 1794312000.5,
//!   "host": …, "source": …, "sourcetype": …, "index": …, "event": "text" | {…}, "fields": {…}}`). An object `event`
//!   is the log's attributes (its `message` the message); a text one the message. `time` is the log's date, `host`
//!   its hostname, `sourcetype` its `ddsource` (unless `?ddsource=`), `fields` its tags; `source`, `sourcetype` and
//!   `index` are kept under `splunk`.
//! - `POST /services/collector/raw[/1.0]`: one log per line; `?host=&source=&sourcetype=` apply to each.
//! - `GET /services/collector/health`.
//!
//! `?ddsource=&service=&hostname=&ddtags=` apply as on `/api/v2/logs` (`log_query`). Senders that set only a host and
//! a token (GitHub's audit log stream: no path, no query, and a sourcetype of its own choosing at most) name the source
//! in the token instead: `<ingest key>:<source>` (`ocik_…:github`). A named source wins over the `sourcetype`, an
//! event's own `ddsource` over both. Answers are HEC's
//! (`{"text":"Success","code":0}`; 400 with code 5 or 6, 401/403 with code 2 or 4, 503 with code 9).

use axum::body::Bytes;
use axum::extract::{RawQuery, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use serde_json::{json, Map, Value};

use crate::kafka::Sink;
use crate::log_query::LogDefaults;
use crate::{admit, datadog, send, Shared};

fn answer(status: StatusCode, text: &str, code: u32) -> Response {
    (status, [(header::CONTENT_TYPE, "application/json")], json!({ "text": text, "code": code }).to_string()).into_response()
}

fn text(v: &Value) -> Option<String> {
    match v {
        Value::String(s) if !s.trim().is_empty() => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

/// Unix seconds (possibly fractional, or a string) as epoch ms.
fn time_ms(v: &Value) -> Option<i64> {
    let secs = v.as_f64().or_else(|| v.as_str().and_then(|s| s.trim().parse().ok()))?;
    (secs > 0.0).then(|| (secs * 1000.0).round() as i64)
}

/// The `splunk` attribute: the HEC metadata a log keeps.
fn metadata(source: Option<String>, sourcetype: Option<String>, index: Option<String>) -> Option<Value> {
    let m: Map<String, Value> = [("source", source), ("sourcetype", sourcetype), ("index", index)].into_iter().filter_map(|(k, v)| Some((k.to_string(), Value::String(v?)))).collect();
    (!m.is_empty()).then_some(Value::Object(m))
}

/// `Authorization: Splunk <ingest key>:<source>`: the source after the key (lowercase letters, digits, `-_.`), taken
/// off so the header carries the key alone. Ingest keys are URL-safe base64 after `ocik_`, never a `:`.
pub fn token_source(headers: &mut HeaderMap) -> Option<String> {
    let auth = headers.get(header::AUTHORIZATION)?.to_str().ok()?.trim();
    let (key, source) = auth.strip_prefix("Splunk ")?.trim().split_once(':')?;
    let valid = !source.is_empty() && source.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '-' | '_' | '.'));
    if !valid || !key.starts_with("ocik_") {
        return None;
    }
    let value = HeaderValue::from_str(&format!("Splunk {key}")).ok()?;
    let source = source.to_string();
    headers.insert(header::AUTHORIZATION, value);
    Some(source)
}

/// One HEC event as a JSON log; `source` (the request's) before its `sourcetype` as the log's `ddsource`.
fn event_log(e: &Map<String, Value>, source: Option<&str>) -> Result<Value, &'static str> {
    let event = e.get("event").filter(|v| !v.is_null()).ok_or("Event field is required")?;
    let mut log = match event {
        Value::Object(o) => o.clone(),
        Value::String(s) if s.is_empty() => return Err("Event field cannot be blank"),
        other => {
            let mut m = Map::new();
            m.insert("message".into(), Value::String(text(other).unwrap_or_else(|| other.to_string())));
            m
        }
    };
    if let Some(ms) = e.get("time").and_then(time_ms) {
        log.insert("date".into(), json!(ms));
    }
    if let Some(h) = e.get("host").and_then(text) {
        log.entry("hostname").or_insert(Value::String(h));
    }
    let sourcetype = e.get("sourcetype").and_then(text);
    if let Some(st) = source.map(str::to_string).or_else(|| sourcetype.clone()) {
        log.entry("ddsource").or_insert(Value::String(st));
    }
    if let Some(Value::Object(fields)) = e.get("fields") {
        let tags: Vec<String> = fields.iter().filter_map(|(k, v)| Some(format!("{k}:{}", text(v)?))).collect();
        if !tags.is_empty() {
            log.insert("ddtags".into(), Value::String(tags.join(",")));
        }
    }
    if let Some(meta) = metadata(e.get("source").and_then(text), sourcetype, e.get("index").and_then(text)) {
        log.insert("splunk".into(), meta);
    }
    Ok(Value::Object(log))
}

/// The logs of a `/services/collector/event` body: concatenated JSON events.
pub fn events(body: &[u8], source: Option<&str>) -> Result<Vec<Value>, (u32, &'static str)> {
    let mut out = Vec::new();
    for item in serde_json::Deserializer::from_slice(body).into_iter::<Value>() {
        let Ok(Value::Object(e)) = item else { return Err((6, "Invalid data format")) };
        out.push(event_log(&e, source).map_err(|m| (if m.contains("required") { 12 } else { 13 }, m))?);
    }
    if out.is_empty() {
        return Err((5, "No data"));
    }
    Ok(out)
}

/// The logs of a `/services/collector/raw` body: a line each, with the query's `host`, `source`, `sourcetype`
/// (`ddsource` the request's `source`, else the `sourcetype`).
pub fn raw_lines(body: &[u8], query: Option<&str>, ddsource: Option<&str>) -> Result<Vec<Value>, (u32, &'static str)> {
    let mut host = None;
    let mut source = None;
    let mut sourcetype = None;
    for (k, v) in url::form_urlencoded::parse(query.unwrap_or("").as_bytes()) {
        let v = Some(v.trim().to_string()).filter(|v| !v.is_empty());
        match k.as_ref() {
            "host" => host = v,
            "source" => source = v,
            "sourcetype" => sourcetype = v,
            _ => {}
        }
    }
    let text = std::str::from_utf8(body).map_err(|_| (6, "Invalid data format"))?;
    let out: Vec<Value> = text
        .lines()
        .map(|l| l.trim_end_matches('\r'))
        .filter(|l| !l.trim().is_empty())
        .map(|l| {
            let mut log = Map::new();
            log.insert("message".into(), Value::String(l.to_string()));
            if let Some(h) = &host {
                log.insert("hostname".into(), Value::String(h.clone()));
            }
            if let Some(st) = ddsource.map(str::to_string).or_else(|| sourcetype.clone()) {
                log.insert("ddsource".into(), Value::String(st));
            }
            if let Some(meta) = metadata(source.clone(), sourcetype.clone(), None) {
                log.insert("splunk".into(), meta);
            }
            Value::Object(log)
        })
        .collect();
    if out.is_empty() {
        return Err((5, "No data"));
    }
    Ok(out)
}

type Parse = fn(&[u8], Option<&str>, Option<&str>) -> Result<Vec<Value>, (u32, &'static str)>;

async fn post(st: Shared, query: Option<String>, mut headers: HeaderMap, body: Bytes, parse: Parse) -> Response {
    let named = token_source(&mut headers);
    let mut defaults = LogDefaults::from_query(query.as_deref());
    defaults.ddsource = defaults.ddsource.or(named);
    let org = match admit(&st, &headers, body.len()).await {
        Ok(org) => org,
        Err(r) if r.status == StatusCode::UNAUTHORIZED && crate::api_key(&headers).is_none() => return answer(StatusCode::UNAUTHORIZED, "Token is required", 2),
        Err(r) if r.status == StatusCode::UNAUTHORIZED => return answer(StatusCode::FORBIDDEN, "Invalid token", 4),
        Err(r) if r.status == StatusCode::TOO_MANY_REQUESTS => return answer(StatusCode::SERVICE_UNAVAILABLE, "Server is busy", 9),
        Err(r) => return r.into_response(),
    };
    let Ok(raw) = datadog::inflate(&headers, &body) else { return answer(StatusCode::BAD_REQUEST, "Invalid data format", 6) };
    let logs = match parse(&raw, query.as_deref(), defaults.ddsource.as_deref()) {
        Ok(l) => l,
        Err((code, text)) => return answer(StatusCode::BAD_REQUEST, text, code),
    };
    let encoded = serde_json::to_vec(&logs).unwrap_or_default();
    let encoded = if defaults.is_empty() {
        encoded
    } else {
        match defaults.apply(&encoded) {
            Ok(b) => b,
            Err(_) => return answer(StatusCode::BAD_REQUEST, "Invalid data format", 6),
        }
    };
    match send(&st, &org, Sink::JsonLogs, Some(&HeaderValue::from_static("application/json")), None, Bytes::from(encoded)).await {
        Ok(up) if up.status.is_success() => answer(StatusCode::OK, "Success", 0),
        Ok(up) => {
            tracing::warn!(status = %up.status, "HEC logs refused by the pipeline");
            answer(StatusCode::SERVICE_UNAVAILABLE, "Server is busy", 9)
        }
        Err(_) => answer(StatusCode::SERVICE_UNAVAILABLE, "Server is busy", 9),
    }
}

/// `POST /services/collector[/event[/1.0]]`.
pub async fn event(State(st): State<Shared>, RawQuery(query): RawQuery, headers: HeaderMap, body: Bytes) -> Response {
    post(st, query, headers, body, |b, _, source| events(b, source)).await
}

/// `POST /services/collector/raw[/1.0]`.
pub async fn raw(State(st): State<Shared>, RawQuery(query): RawQuery, headers: HeaderMap, body: Bytes) -> Response {
    post(st, query, headers, body, raw_lines).await
}

/// `GET /services/collector/health`.
pub async fn health() -> Response {
    answer(StatusCode::OK, "HEC is healthy", 17)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{test_state, CachedKey, Config, KeyOrg};
    use axum::routing::post as route_post;
    use axum::Router;
    use sha2::{Digest, Sha256};
    use std::time::Instant;

    #[test]
    fn hec_events_are_logs() {
        let body = br#"{"time": 1794312000.25, "host": "ghes-01", "source": "github", "sourcetype": "github_audit", "index": "main", "event": {"action": "repo.create", "actor": "dev@example.com", "message": "created example-org/storefront"}, "fields": {"env": "prod"}}
{"event": "plain text line", "time": "1794312001"}"#;
        let logs = events(body, None).unwrap();
        assert_eq!(logs.len(), 2);
        assert_eq!((&logs[0]["action"], &logs[0]["message"], &logs[0]["date"], &logs[0]["hostname"]), (&json!("repo.create"), &json!("created example-org/storefront"), &json!(1_794_312_000_250i64), &json!("ghes-01")));
        assert_eq!((&logs[0]["ddsource"], &logs[0]["ddtags"], &logs[0]["splunk"]["index"]), (&json!("github_audit"), &json!("env:prod"), &json!("main")));
        assert_eq!((&logs[1]["message"], &logs[1]["date"]), (&json!("plain text line"), &json!(1_794_312_001_000i64)));
        assert_eq!(events(br#"{"time": 1}{"event": "concatenated"}"#, None).err(), Some((12, "Event field is required")));
        assert_eq!(events(b"", None).err(), Some((5, "No data")));
        assert_eq!(events(b"not json", None).err(), Some((6, "Invalid data format")));
        let named = events(br#"{"sourcetype": "httpevent", "event": {"action": "repo.create"}}{"sourcetype": "x", "event": {"ddsource": "own"}}"#, Some("github")).unwrap();
        assert_eq!((&named[0]["ddsource"], &named[0]["splunk"]["sourcetype"], &named[1]["ddsource"]), (&json!("github"), &json!("httpevent"), &json!("own")), "a named source wins over the sourcetype");
    }

    #[test]
    fn the_token_may_name_a_source_after_the_key() {
        let split = |auth: &'static str| {
            let mut h = HeaderMap::new();
            h.insert(header::AUTHORIZATION, HeaderValue::from_static(auth));
            let source = token_source(&mut h);
            (h[header::AUTHORIZATION].to_str().unwrap().to_string(), source)
        };
        assert_eq!(split("Splunk ocik_abc-_9:github"), ("Splunk ocik_abc-_9".into(), Some("github".into())));
        assert_eq!(split("Splunk ocik_abc"), ("Splunk ocik_abc".into(), None));
        assert_eq!(split("Splunk ocik_abc:Not a source"), ("Splunk ocik_abc:Not a source".into(), None));
        assert_eq!(split("Bearer ocik_abc:github"), ("Bearer ocik_abc:github".into(), None));
    }

    #[test]
    fn raw_bodies_are_a_log_per_line() {
        let logs = raw_lines(b"first line\r\n\nsecond line\n", Some("host=web-01&sourcetype=nginx&channel=x"), None).unwrap();
        assert_eq!(logs.len(), 2);
        assert_eq!((&logs[0]["message"], &logs[0]["hostname"], &logs[0]["ddsource"], &logs[1]["message"]), (&json!("first line"), &json!("web-01"), &json!("nginx"), &json!("second line")));
        assert_eq!(raw_lines(b"\n\n", None, None).err(), Some((5, "No data")));
        assert_eq!(raw_lines(b"line", Some("sourcetype=generic"), Some("github")).unwrap()[0]["ddsource"], "github");
    }

    /// The HEC token is the ingest key (`Authorization: Splunk …`); the logs reach the pipeline with the query's
    /// defaults, and the answers are HEC's.
    #[tokio::test]
    async fn the_collector_takes_the_ingest_key_as_its_token() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mut cfg = Config::from_env();
        cfg.logs_url = format!("http://{}/", listener.local_addr().unwrap());
        let got: std::sync::Arc<std::sync::Mutex<Vec<Value>>> = Default::default();
        let seen = got.clone();
        tokio::spawn(async move {
            axum::serve(listener, Router::new().route("/", route_post(move |b: Bytes| async move { seen.lock().unwrap().extend(serde_json::from_slice::<Vec<Value>>(&b).unwrap()); "{}" }))).await.unwrap();
        });
        let st = test_state(cfg);
        let org = KeyOrg { org_id: "example-org".into(), span_retention_days: 15, metrics_retention_days: 30, metrics_5m_days: 90, metrics_1h_days: 395, flag_environment_id: None, ingest_mbps: None, ingest_rps: None };
        st.keys.lock().unwrap().insert(Sha256::digest(b"ocik_hec_test").to_vec(), CachedKey { org: Some(org), at: Instant::now(), touched: Some(Instant::now()) });
        st.keys.lock().unwrap().insert(Sha256::digest(b"ocik_unknown").to_vec(), CachedKey { org: None, at: Instant::now(), touched: None });
        let call = |auth: Option<&'static str>, query: Option<&str>, body: &'static [u8]| {
            let st = st.clone();
            let query = query.map(str::to_string);
            async move {
                let mut h = HeaderMap::new();
                if let Some(a) = auth {
                    h.insert(header::AUTHORIZATION, HeaderValue::from_static(a));
                }
                let res = event(State(st), RawQuery(query), h, Bytes::from_static(body)).await;
                let status = res.status();
                (status, serde_json::from_slice::<Value>(&axum::body::to_bytes(res.into_body(), 1 << 16).await.unwrap()).unwrap())
            }
        };
        let (status, body) = call(Some("Splunk ocik_hec_test"), Some("service=github"), br#"{"event": {"action": "org.update"}, "sourcetype": "github_audit"}"#).await;
        assert_eq!((status, body), (StatusCode::OK, json!({ "text": "Success", "code": 0 })));
        let logs = got.lock().unwrap().clone();
        assert_eq!((&logs[0]["action"], &logs[0]["service"], &logs[0]["ddsource"]), (&json!("org.update"), &json!("github"), &json!("github_audit")));
        assert!(logs[0]["_oc_ingest"]["id"].is_string());
        // GitHub's stream sets no source of its own: the token names it after the key, or the query does.
        for (auth, query, want) in [("Splunk ocik_hec_test:github", None, "github"), ("Splunk ocik_hec_test", Some("ddsource=github"), "github"), ("Splunk ocik_hec_test:github", Some("ddsource=other"), "other")] {
            got.lock().unwrap().clear();
            let (status, _) = call(Some(auth), query, br#"{"event": {"action": "org.update"}, "sourcetype": "httpevent"}"#).await;
            assert_eq!((status, &got.lock().unwrap()[0]["ddsource"]), (StatusCode::OK, &json!(want)), "{auth} {query:?}");
        }
        assert_eq!(call(Some("Splunk ocik_unknown:github"), None, b"{}").await.0, StatusCode::FORBIDDEN);
        assert_eq!(call(None, None, b"{}").await, (StatusCode::UNAUTHORIZED, json!({ "text": "Token is required", "code": 2 })));
        assert_eq!(call(Some("Splunk ocik_unknown"), None, b"{}").await.0, StatusCode::FORBIDDEN);
        assert_eq!(call(Some("Splunk ocik_hec_test"), None, b"").await, (StatusCode::BAD_REQUEST, json!({ "text": "No data", "code": 5 })));
    }
}
