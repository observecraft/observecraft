//! Real User Monitoring: the events of Datadog's browser and mobile SDKs (`POST /api/v2/rum`).
//!
//! The SDKs post newline-delimited JSON events (views, actions, errors, resources, long tasks,
//! vitals), deflate-compressed or not, with the application's client token (`dd-api-key=pub…`) and
//! `ddtags` in the query string. Behind the SDK's `proxy` option the original path and query arrive
//! in `?ddforward=`. A client token is public (pages embed it), so it is accepted here only; ingest
//! keys (`ocik_…`) work too, for server-side senders.
//!
//! Each event becomes an OTLP log record with resource `oc.signal = dd.rum`, which the collector
//! routes to `otel.otel_rum`: Body = the event's JSON, LogAttributes = its fields flattened to dotted
//! paths, plus the browser, OS and device read from the User-Agent and `network.client.ip`, as
//! Datadog adds them; resource = service.name, deployment.environment and service.version.
//! Applications whose event processing is off (`NONE`) are acknowledged and dropped; the
//! error-focused mode (`ERROR_FOCUSED_MODE`) keeps every session for now (a batch doesn't tell
//! whether its session will have an error).

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use axum::body::Bytes;
use axum::extract::{RawQuery, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::datadog::inflate;
use crate::otlp;
use crate::{admit, send, within_limits, KeyOrg, Rejection, Shared};

/// Fields kept per event, and the length of a kept value.
const MAX_FIELDS: usize = 300;
const MAX_VALUE: usize = 4096;
const MAX_DEPTH: usize = 6;
/// Client tokens are looked up again after this long.
const TOKEN_TTL: Duration = Duration::from_secs(60);

/// The org of a client token, and whether its application keeps events.
#[derive(Clone, Debug)]
pub struct TokenApp {
    pub org: KeyOrg,
    pub keep: bool,
}

/// Client tokens seen lately (by hash): the application, or None for an unknown token.
pub type TokenCache = std::sync::Mutex<std::collections::HashMap<Vec<u8>, (Option<TokenApp>, Instant)>>;

/// The query parameters of a request, with those forwarded through the SDK's proxy option (`ddforward`).
pub fn params(query: &str) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for (k, v) in url::form_urlencoded::parse(query.as_bytes()) {
        if k == "ddforward" {
            if let Some((_, inner)) = v.split_once('?') {
                for (ik, iv) in url::form_urlencoded::parse(inner.as_bytes()) {
                    out.entry(ik.into_owned()).or_insert_with(|| iv.into_owned());
                }
            }
        } else {
            out.insert(k.into_owned(), v.into_owned());
        }
    }
    out
}

/// `ddtags` (`sdk_version:5.x,env:prod,service:storefront-web,version:1.4.2`) as pairs.
fn ddtags(raw: &str) -> Vec<(String, String)> {
    raw.split(',').filter_map(|t| t.trim().split_once(':')).map(|(k, v)| (k.to_string(), v.to_string())).collect()
}

/// An event's fields as dotted paths (`view.name`), scalars only; arrays of scalars are comma-joined.
pub fn flatten(v: &Value) -> Vec<(String, String)> {
    fn walk(prefix: &str, v: &Value, depth: usize, out: &mut Vec<(String, String)>) {
        if out.len() >= MAX_FIELDS {
            return;
        }
        let scalar = |v: &Value| match v {
            Value::String(s) => Some(s.clone()),
            Value::Number(n) => Some(n.to_string()),
            Value::Bool(b) => Some(b.to_string()),
            _ => None,
        };
        match v {
            Value::Object(m) if depth < MAX_DEPTH => {
                for (k, child) in m {
                    let path = if prefix.is_empty() { k.clone() } else { format!("{prefix}.{k}") };
                    walk(&path, child, depth + 1, out);
                }
            }
            Value::Array(a) if !prefix.is_empty() => {
                let items: Vec<String> = a.iter().filter_map(scalar).collect();
                if !items.is_empty() && items.len() == a.len() {
                    out.push((prefix.to_string(), items.join(",").chars().take(MAX_VALUE).collect()));
                }
            }
            Value::Null => {}
            other if !prefix.is_empty() => {
                if let Some(s) = scalar(other) {
                    out.push((prefix.to_string(), s.chars().take(MAX_VALUE).collect()));
                }
            }
            _ => {}
        }
    }
    let mut out = Vec::new();
    walk("", v, 0, &mut out);
    out
}

/// What the User-Agent says about the browser, OS and device, under Datadog's RUM attribute names.
pub fn user_agent(ua: &str) -> Vec<(&'static str, String)> {
    let version = |marker: &str| -> Option<String> {
        let rest = &ua[ua.find(marker)? + marker.len()..];
        let v: String = rest.chars().take_while(|c| c.is_ascii_digit() || *c == '.' || *c == '_').collect();
        (!v.is_empty()).then(|| v.replace('_', "."))
    };
    let lower = ua.to_ascii_lowercase();
    let mobile = ua.contains("Mobile") || ua.contains("iPhone");
    let browser = if ua.contains("Edg/") {
        Some(("Edge", version("Edg/")))
    } else if ua.contains("OPR/") {
        Some(("Opera", version("OPR/")))
    } else if ua.contains("CriOS/") {
        Some(("Chrome Mobile iOS", version("CriOS/")))
    } else if ua.contains("Firefox/") {
        Some((if mobile { "Firefox Mobile" } else { "Firefox" }, version("Firefox/")))
    } else if ua.contains("Chrome/") {
        Some((if mobile { "Chrome Mobile" } else { "Chrome" }, version("Chrome/")))
    } else if ua.contains("Safari/") && ua.contains("Version/") {
        Some((if mobile { "Mobile Safari" } else { "Safari" }, version("Version/")))
    } else {
        None
    };
    let os = if let Some(v) = version("iPhone OS ").or_else(|| version("CPU OS ")) {
        Some(("iOS", Some(v)))
    } else if ua.contains("Android") {
        Some(("Android", version("Android ")))
    } else if ua.contains("Windows NT") {
        Some(("Windows", version("Windows NT ")))
    } else if ua.contains("Mac OS X") {
        Some(("Mac OS X", version("Mac OS X ")))
    } else if ua.contains("CrOS") {
        Some(("Chrome OS", None))
    } else if ua.contains("Linux") {
        Some(("Linux", None))
    } else {
        None
    };
    let device = if ["bot", "crawler", "spider", "headless"].iter().any(|b| lower.contains(b)) {
        "Bot"
    } else if ua.contains("iPad") || ua.contains("Tablet") || (ua.contains("Android") && !ua.contains("Mobile")) {
        "Tablet"
    } else if mobile || ua.contains("Android") {
        "Mobile"
    } else {
        "Desktop"
    };
    let mut out = vec![("device.type", device.to_string())];
    if let Some((name, v)) = browser {
        out.push(("browser.name", name.to_string()));
        if let Some(v) = v {
            out.push(("browser.version_major", v.split('.').next().unwrap_or_default().to_string()));
            out.push(("browser.version", v));
        }
    }
    if let Some((name, v)) = os {
        out.push(("os.name", name.to_string()));
        if let Some(v) = v {
            out.push(("os.version_major", v.split('.').next().unwrap_or_default().to_string()));
            out.push(("os.version", v));
        }
    }
    out
}

/// What the request adds to every event: its tags, the client's address and User-Agent.
#[derive(Default)]
pub struct Context {
    pub tags: Vec<(String, String)>,
    pub client_ip: Option<String>,
    pub user_agent: Option<String>,
}

/// OTLP logs for the events, one resource per (service, env, version).
pub fn to_otlp(events: &[Value], ctx: &Context, now_ms: i64) -> otlp::ExportLogsServiceRequest {
    let tag = |k: &str| ctx.tags.iter().find(|(tk, _)| tk == k).map(|(_, v)| v.clone()).unwrap_or_default();
    let ua = ctx.user_agent.as_deref().map(user_agent).unwrap_or_default();
    let mut groups: BTreeMap<(String, String, String), Vec<otlp::LogRecord>> = BTreeMap::new();
    for e in events {
        let s = |k: &str| e.get(k).and_then(Value::as_str).unwrap_or("").to_string();
        let service = Some(s("service")).filter(|v| !v.is_empty()).unwrap_or_else(|| tag("service"));
        let env = Some(s("env")).filter(|v| !v.is_empty()).unwrap_or_else(|| tag("env"));
        let version = Some(s("version")).filter(|v| !v.is_empty()).unwrap_or_else(|| tag("version"));
        let mut fields = flatten(e);
        let has = |fields: &[(String, String)], k: &str| fields.iter().any(|(fk, _)| fk == k);
        for (k, v) in &ua {
            if !has(&fields, k) {
                fields.push((k.to_string(), v.clone()));
            }
        }
        if let Some(ip) = &ctx.client_ip {
            fields.push(("network.client.ip".into(), ip.clone()));
        }
        for (k, v) in [("service", &service), ("env", &env), ("version", &version)] {
            if !v.is_empty() && !has(&fields, k) {
                fields.push((k.to_string(), v.clone()));
            }
        }
        let at = e.get("date").and_then(Value::as_i64).filter(|d| *d > 0).unwrap_or(now_ms);
        let t = (at.max(0) as u64) * 1_000_000;
        let error = s("type") == "error";
        groups.entry((service, env, version)).or_default().push(otlp::LogRecord {
            time_unix_nano: t,
            observed_time_unix_nano: (now_ms.max(0) as u64) * 1_000_000,
            severity_number: if error { 17 } else { 9 },
            severity_text: if error { "error" } else { "info" }.into(),
            body: Some(otlp::AnyValue { value: Some(otlp::any_value::Value::StringValue(e.to_string())) }),
            attributes: fields.into_iter().map(|(k, v)| otlp::kv(k, v)).collect(),
        });
    }
    otlp::ExportLogsServiceRequest {
        resource_logs: groups
            .into_iter()
            .map(|((service, env, version), log_records)| {
                let mut attributes = vec![otlp::kv("oc.signal", "dd.rum")];
                for (k, v) in [("service.name", service), ("deployment.environment", env), ("service.version", version)] {
                    if !v.is_empty() {
                        attributes.push(otlp::kv(k, v));
                    }
                }
                otlp::ResourceLogs { resource: Some(otlp::Resource { attributes }), scope_logs: vec![otlp::ScopeLogs { scope: otlp::scope("datadog.rum"), log_records }] }
            })
            .collect(),
    }
}

/// The events of a body: one JSON object per line (a JSON array is accepted too).
pub fn events(raw: &[u8]) -> Vec<Value> {
    let text = String::from_utf8_lossy(raw);
    if let Ok(Value::Array(a)) = serde_json::from_str::<Value>(text.trim()) {
        return a.into_iter().filter(Value::is_object).collect();
    }
    text.lines().filter_map(|l| serde_json::from_str::<Value>(l.trim()).ok()).filter(Value::is_object).collect()
}

/// The application of a client token (`pub…`), cached for [`TOKEN_TTL`].
async fn token_app(st: &Shared, token: &str) -> anyhow::Result<Option<TokenApp>> {
    let hash = Sha256::digest(token.as_bytes()).to_vec();
    if let Some((app, at)) = st.rum_tokens.lock().unwrap().get(&hash) {
        if at.elapsed() < TOKEN_TTL {
            return Ok(app.clone());
        }
    }
    #[derive(sqlx::FromRow)]
    struct Row {
        #[sqlx(flatten)]
        org: KeyOrg,
        state: String,
    }
    let row: Option<Row> = sqlx::query_as(
        "SELECT a.org_id, COALESCE(s.span_retention_days, 0) AS span_retention_days, COALESCE(s.metrics_retention_days, 0) AS metrics_retention_days, \
         COALESCE(s.metrics_5m_days, 0) AS metrics_5m_days, COALESCE(s.metrics_1h_days, 0) AS metrics_1h_days, NULL::text AS flag_environment_id, \
         a.rum_event_processing_state AS state FROM rum_applications a JOIN orgs o ON o.id = a.org_id LEFT JOIN org_data_settings s ON s.org_id = a.org_id \
         WHERE a.client_token = $1 AND o.status = 'active'",
    )
    .bind(token)
    .fetch_optional(&st.pg)
    .await?;
    let app = row.map(|r| TokenApp { org: r.org, keep: r.state != "NONE" });
    st.rum_tokens.lock().unwrap().insert(hash, (app.clone(), Instant::now()));
    Ok(app)
}

/// The sender: an application's client token (query string or `DD-API-KEY`), else an ingest key.
pub async fn authenticate(st: &Shared, headers: &HeaderMap, query: &BTreeMap<String, String>, bytes: usize) -> Result<TokenApp, Rejection> {
    let token = query.get("dd-api-key").cloned().or_else(|| headers.get("dd-api-key").and_then(|v| v.to_str().ok()).map(str::to_string)).unwrap_or_default();
    if token.starts_with("pub") {
        let app = match token_app(st, token.trim()).await {
            Ok(Some(app)) => app,
            Ok(None) => return Err(Rejection { status: StatusCode::FORBIDDEN, message: "invalid client token" }),
            Err(e) => {
                tracing::error!(error = %e, "client token lookup failed");
                return Err(Rejection { status: StatusCode::SERVICE_UNAVAILABLE, message: "temporarily unavailable" });
            }
        };
        if !within_limits(st, &app.org.org_id, bytes) {
            return Err(Rejection { status: StatusCode::TOO_MANY_REQUESTS, message: "rate limit exceeded for this organization" });
        }
        return Ok(app);
    }
    let mut h = headers.clone();
    if !token.is_empty() && !h.contains_key("dd-api-key") {
        if let Ok(v) = HeaderValue::from_str(&token) {
            h.insert("dd-api-key", v);
        }
    }
    Ok(TokenApp { org: admit(st, &h, bytes).await?, keep: true })
}

/// The client's address: the first `X-Forwarded-For` hop, else `X-Real-IP`.
fn client_ip(headers: &HeaderMap) -> Option<String> {
    let first = headers.get("x-forwarded-for").and_then(|v| v.to_str().ok()).and_then(|v| v.split(',').next()).map(str::trim);
    first.or_else(|| headers.get("x-real-ip").and_then(|v| v.to_str().ok())).filter(|ip| ip.parse::<std::net::IpAddr>().is_ok()).map(str::to_string)
}

fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

/// `POST /api/v2/rum`: a batch of RUM events.
pub async fn post(State(st): State<Shared>, RawQuery(query): RawQuery, headers: HeaderMap, body: Bytes) -> Response {
    let q = params(query.as_deref().unwrap_or(""));
    let app = match authenticate(&st, &headers, &q, body.len()).await {
        Ok(app) => app,
        Err(r) => return r.into_response(),
    };
    let raw = match inflate(&headers, &body) {
        Ok(raw) => raw,
        Err(e) => return (StatusCode::BAD_REQUEST, e).into_response(),
    };
    let list = events(&raw);
    if !app.keep || list.is_empty() {
        return accepted();
    }
    let ctx = Context { tags: ddtags(q.get("ddtags").map(String::as_str).unwrap_or("")), client_ip: client_ip(&headers), user_agent: headers.get(header::USER_AGENT).and_then(|v| v.to_str().ok()).map(str::to_string) };
    let req = to_otlp(&list, &ctx, now_ms());
    match send(&st, &app.org, crate::kafka::Sink::Logs, Some(&HeaderValue::from_static("application/x-protobuf")), None, otlp::encode(&req)).await {
        Ok(up) if up.status.is_success() => accepted(),
        Ok(up) => {
            tracing::warn!(status = %up.status, "collector refused converted RUM events");
            Rejection { status: StatusCode::SERVICE_UNAVAILABLE, message: "events were not accepted" }.into_response()
        }
        Err(r) => r.into_response(),
    }
}

fn accepted() -> Response {
    (StatusCode::ACCEPTED, [(header::CONTENT_TYPE, "application/json")], "{}").into_response()
}

/// Browsers post from any page, without credentials: any origin may send.
pub fn cors() -> tower_http::cors::CorsLayer {
    use tower_http::cors::{AllowOrigin, CorsLayer};
    CorsLayer::new()
        .allow_origin(AllowOrigin::any())
        .allow_methods([axum::http::Method::POST, axum::http::Method::OPTIONS])
        .allow_headers([header::CONTENT_TYPE, header::CONTENT_ENCODING, header::HeaderName::from_static("dd-api-key")])
        .max_age(Duration::from_secs(3600))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn query_parameters_come_through_the_proxy_too() {
        let q = params("ddsource=browser&dd-api-key=pubabc&ddtags=env%3Aprod%2Cservice%3Astorefront-web");
        assert_eq!(q["dd-api-key"], "pubabc");
        assert_eq!(ddtags(&q["ddtags"]), vec![("env".into(), "prod".into()), ("service".into(), "storefront-web".into())]);
        let fwd = params("ddforward=%2Fapi%2Fv2%2Frum%3Fddsource%3Dbrowser%26dd-api-key%3Dpubxyz");
        assert_eq!(fwd["dd-api-key"], "pubxyz");
        assert_eq!(fwd["ddsource"], "browser");
    }

    #[test]
    fn events_are_flattened_to_dotted_paths() {
        let e = json!({"type": "view", "date": 1_700_000_000_000i64, "view": {"id": "v1", "name": "/checkout", "loading_time": 1_250_000_000, "frustration": {"count": 0}},
            "session": {"id": "s1", "type": "user"}, "usr": {"id": "u-42"}, "context": {"cart": {"items": [1, 2]}}, "_dd": {"document_version": 3}, "nothing": null});
        let f: BTreeMap<String, String> = flatten(&e).into_iter().collect();
        assert_eq!(f["view.name"], "/checkout");
        assert_eq!(f["view.loading_time"], "1250000000");
        assert_eq!(f["session.type"], "user");
        assert_eq!(f["context.cart.items"], "1,2");
        assert_eq!(f["_dd.document_version"], "3");
        assert!(!f.contains_key("nothing"));
    }

    #[test]
    fn user_agents_name_browser_os_and_device() {
        let get = |ua: &str| user_agent(ua).into_iter().collect::<BTreeMap<_, _>>();
        let mac = get("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36");
        assert_eq!((mac["browser.name"].as_str(), mac["browser.version_major"].as_str(), mac["os.name"].as_str(), mac["os.version"].as_str()), ("Chrome", "130", "Mac OS X", "10.15.7"));
        assert_eq!(mac["device.type"], "Desktop");
        let iphone = get("Mozilla/5.0 (iPhone; CPU iPhone OS 17_4 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.4 Mobile/15E148 Safari/604.1");
        assert_eq!((iphone["browser.name"].as_str(), iphone["os.name"].as_str(), iphone["os.version"].as_str(), iphone["device.type"].as_str()), ("Mobile Safari", "iOS", "17.4", "Mobile"));
        let edge = get("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36 Edg/130.0.2849.68");
        assert_eq!((edge["browser.name"].as_str(), edge["os.name"].as_str()), ("Edge", "Windows"));
        let tablet = get("Mozilla/5.0 (Linux; Android 14; SM-X710) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36");
        assert_eq!(tablet["device.type"], "Tablet");
        assert_eq!(get("Googlebot/2.1 (+http://www.google.com/bot.html)")["device.type"], "Bot");
    }

    #[test]
    fn bodies_are_ndjson_or_an_array() {
        assert_eq!(events(b"{\"type\":\"view\"}\n{\"type\":\"action\"}\nnot json\n").len(), 2);
        assert_eq!(events(b"[{\"type\":\"error\"}, 3]").len(), 1);
    }

    #[test]
    fn events_become_otlp_logs_routed_to_rum() {
        let ctx = Context {
            tags: ddtags("env:prod,service:storefront-web,version:1.4.2,sdk_version:5.28.0"),
            client_ip: Some("198.51.100.7".into()),
            user_agent: Some("Mozilla/5.0 (X11; Linux x86_64; rv:131.0) Gecko/20100101 Firefox/131.0".into()),
        };
        let evs = vec![
            json!({"type": "error", "date": 1_700_000_000_123i64, "application": {"id": "app-1"}, "error": {"message": "Cannot read properties of undefined", "source": "source"}}),
            json!({"type": "view", "service": "admin-web", "view": {"name": "/"}}),
        ];
        let req = to_otlp(&evs, &ctx, 5);
        assert_eq!(req.resource_logs.len(), 2);
        let storefront = req.resource_logs.iter().find(|r| r.resource.as_ref().unwrap().attributes.contains(&otlp::kv("service.name", "storefront-web"))).unwrap();
        let res = &storefront.resource.as_ref().unwrap().attributes;
        assert!(res.contains(&otlp::kv("oc.signal", "dd.rum")) && res.contains(&otlp::kv("deployment.environment", "prod")) && res.contains(&otlp::kv("service.version", "1.4.2")));
        let r = &storefront.scope_logs[0].log_records[0];
        assert_eq!((r.time_unix_nano, r.severity_text.as_str()), (1_700_000_000_123_000_000, "error"));
        for kv in [otlp::kv("error.message", "Cannot read properties of undefined"), otlp::kv("browser.name", "Firefox"), otlp::kv("os.name", "Linux"), otlp::kv("network.client.ip", "198.51.100.7")] {
            assert!(r.attributes.contains(&kv), "{kv:?}");
        }
        // An event's own service wins over the request's tags, and dates default to now.
        let admin = req.resource_logs.iter().find(|r| r.resource.as_ref().unwrap().attributes.contains(&otlp::kv("service.name", "admin-web"))).unwrap();
        assert_eq!(admin.scope_logs[0].log_records[0].time_unix_nano, 5_000_000);
    }
}
