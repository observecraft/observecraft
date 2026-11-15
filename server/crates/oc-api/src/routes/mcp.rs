//! `POST /api/v1/mcp`: the MCP server for AI agents (ADR-0009, `docs/mcp.md`). Streamable HTTP with JSON
//! answers only; the protocol is [`crate::mcp`], the tools [`crate::mcp::tools`].
//!
//! Authenticated with an application key only (`Authorization: Bearer ocak_…`, or `DD-APPLICATION-KEY`
//! with an optional `DD-API-KEY` of the same org), never a session cookie, so it is merged outside the
//! session middleware ([`super::app`]). The org and user come from the key.

use axum::extract::{ConnectInfo, State};
use axum::http::header::{ORIGIN, WWW_AUTHENTICATE};
use axum::http::{HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::Router;

use super::datadog_api;
use crate::mcp::{self, Message, Reply};
use crate::state::AppState;

/// The largest request body (a JSON-RPC message with tool arguments).
const MAX_BODY: usize = 1 << 20;

pub fn router() -> Router<AppState> {
    Router::new().route("/api/v1/mcp", post(handle).get(not_allowed).delete(not_allowed))
}

fn unauthorized(message: &str) -> Response {
    let mut r = Reply::error(StatusCode::UNAUTHORIZED, None, mcp::INVALID_REQUEST, message, None).into_response();
    r.headers_mut().insert(WWW_AUTHENTICATE, HeaderValue::from_static("Bearer realm=\"ObserveCraft\""));
    r
}

/// One JSON-RPC message from an MCP client (`initialize`, `server/discover`, `ping`, `tools/list`,
/// `tools/call`); notifications are accepted with 202.
#[utoipa::path(post, path = "/api/v1/mcp", operation_id = "mcp_message", request_body(content = Object, description = "A JSON-RPC 2.0 request or notification"),
    params(("Authorization" = Option<String>, Header, description = "Bearer ocak_… (an application key)"), ("DD-APPLICATION-KEY" = Option<String>, Header, description = "Application key, instead of Authorization"),
        ("MCP-Protocol-Version" = Option<String>, Header, description = "2026-07-28, 2025-11-25, 2025-06-18 or 2025-03-26")),
    responses((status = 200, description = "JSON-RPC response", body = Object), (status = 202, description = "Notification accepted"), (status = 400, description = "Malformed message, header mismatch or unsupported protocol version", body = Object),
        (status = 401, description = "No valid application key"), (status = 403, description = "Origin not allowed"), (status = 404, description = "Unknown method (2026-07-28)", body = Object)),
    tag = "mcp")]
pub async fn handle(State(st): State<AppState>, request: axum::extract::Request) -> Response {
    let peer = request.extensions().get::<ConnectInfo<std::net::SocketAddr>>().map(|c| c.0);
    let (parts, body) = request.into_parts();
    let headers = parts.headers;
    let Ok(body) = axum::body::to_bytes(body, MAX_BODY).await else {
        return Reply::error(StatusCode::PAYLOAD_TOO_LARGE, None, mcp::INVALID_REQUEST, "The message is larger than 1 MiB", None).into_response();
    };
    if headers.contains_key(ORIGIN) && !crate::auth::origin_allowed(&st, &headers) {
        return Reply::error(StatusCode::FORBIDDEN, None, mcp::INVALID_REQUEST, "Origin not allowed", None).into_response();
    }
    let keys = datadog_api::keys(&headers);
    if keys.app_key.is_none() {
        return unauthorized("An application key is required: Authorization: Bearer ocak_…");
    }
    let ip = crate::auth::client_ip(&st, &headers, peer);
    let ctx = match datadog_api::resolve(&st, &keys, ip).await {
        Ok(ctx) => ctx,
        Err(e) if e.status == StatusCode::FORBIDDEN => return unauthorized("Invalid or revoked application key"),
        Err(e) => return e.into_response(),
    };
    let request = match mcp::parse(&headers, &body) {
        Ok(Message::Request(r)) => r,
        Ok(Message::Accepted) => return StatusCode::ACCEPTED.into_response(),
        Err(reply) => return reply.into_response(),
    };
    mcp::dispatch(request, &mcp::tools::McpTools { st, ctx }).await.into_response()
}

/// The server sends nothing on its own, so there is no GET stream (and no session to DELETE).
#[utoipa::path(get, path = "/api/v1/mcp", operation_id = "mcp_stream", responses((status = 405, description = "No server-initiated stream")), tag = "mcp")]
pub async fn not_allowed() -> Response {
    let mut r = StatusCode::METHOD_NOT_ALLOWED.into_response();
    r.headers_mut().insert(axum::http::header::ALLOW, HeaderValue::from_static("POST"));
    r
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};

    /// The route over the live test databases (skipped without them): an org with a user and key.
    async fn server(prefix: &str) -> Option<datadog_api::replay::TestServer> {
        datadog_api::replay::TestServer::start(prefix).await
    }

    async fn post(base: &str, key: Option<&str>, body: Value) -> (StatusCode, Value) {
        let mut r = reqwest::Client::new().post(format!("{base}/api/v1/mcp")).header("accept", "application/json, text/event-stream").header("mcp-protocol-version", "2025-06-18").json(&body);
        if let Some(k) = key {
            r = r.bearer_auth(k);
        }
        let res = r.send().await.expect("request");
        let status = StatusCode::from_u16(res.status().as_u16()).unwrap();
        (status, res.json().await.unwrap_or(Value::Null))
    }

    fn call(name: &str, arguments: Value) -> Value {
        json!({"jsonrpc": "2.0", "id": 7, "method": "tools/call", "params": {"name": name, "arguments": arguments}})
    }

    #[tokio::test]
    async fn requires_an_application_key() {
        let Some(s) = server("mcp-auth").await else { return };
        let res = reqwest::Client::new().post(format!("{}/api/v1/mcp", s.base)).json(&json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"})).send().await.unwrap();
        assert_eq!(res.status().as_u16(), 401);
        assert!(res.headers()["www-authenticate"].to_str().unwrap().starts_with("Bearer"));
        let (status, _) = post(&s.base, Some("ocak_not_a_real_key"), json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"})).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        // A session cookie is not a credential here.
        let res = reqwest::Client::new().post(format!("{}/api/v1/mcp", s.base)).header("cookie", "oc_session=whatever").json(&json!({"jsonrpc": "2.0", "id": 1, "method": "ping"})).send().await.unwrap();
        assert_eq!(res.status().as_u16(), 401);
        // Datadog's key headers work as for the management API.
        let res = reqwest::Client::new().post(format!("{}/api/v1/mcp", s.base)).header("dd-application-key", &s.app_key).header("dd-api-key", &s.api_key).json(&json!({"jsonrpc": "2.0", "id": 1, "method": "ping"})).send().await.unwrap();
        assert_eq!(res.status().as_u16(), 200);
        let res = reqwest::Client::new().get(format!("{}/api/v1/mcp", s.base)).bearer_auth(&s.app_key).send().await.unwrap();
        assert_eq!(res.status().as_u16(), 405);
        let res = reqwest::Client::new().post(format!("{}/api/v1/mcp", s.base)).bearer_auth(&s.app_key).header("origin", "https://evil.example.com").json(&json!({"jsonrpc": "2.0", "id": 1, "method": "ping"})).send().await.unwrap();
        assert_eq!(res.status().as_u16(), 403);
    }

    #[tokio::test]
    async fn handshake_lists_and_calls_tools() {
        let Some(s) = server("mcp-handshake").await else { return };
        let (status, b) = post(&s.base, Some(&s.app_key), json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "test", "version": "1"}}})).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(b["result"]["protocolVersion"], "2025-06-18");
        let (status, _) = post(&s.base, Some(&s.app_key), json!({"jsonrpc": "2.0", "method": "notifications/initialized"})).await;
        assert_eq!(status, StatusCode::ACCEPTED);
        let (_, b) = post(&s.base, Some(&s.app_key), json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"})).await;
        let names: Vec<&str> = b["result"]["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect();
        assert!(names.contains(&"search_logs") && names.contains(&"search_monitors"), "{names:?}");
        let (_, b) = post(&s.base, Some(&s.app_key), call("search_monitors", json!({}))).await;
        assert_eq!(b["result"]["isError"], false, "{b}");
        // Invalid input is a tool error the model can correct, not a protocol error.
        let (_, b) = post(&s.base, Some(&s.app_key), call("search_logs", json!({"query": "*", "from": "90d"}))).await;
        assert_eq!(b["result"]["isError"], true, "{b}");
        assert!(b["result"]["content"][0]["text"].as_str().unwrap().contains("31 days"));
        let (_, b) = post(&s.base, Some(&s.app_key), call("get_trace", json!({"trace_id": "not hex; DROP TABLE"}))).await;
        assert_eq!(b["result"]["isError"], true, "{b}");
        // Every call is in the org's audit trail.
        let (n,): (i64,) = sqlx::query_as("SELECT count(*) FROM audit_events WHERE org_id = $1 AND action = 'mcp.tool_call'").bind(&s.org).fetch_one(&s.st.postgres).await.unwrap();
        assert_eq!(n, 3);
    }

    /// Each search runs against the real handlers (an empty org: no data, but no error either).
    #[tokio::test]
    async fn every_search_runs() {
        let Some(s) = server("mcp-every").await else { return };
        let calls = [
            ("search_logs", json!({"query": "*", "from": "1d"})),
            ("aggregate_logs", json!({"query": "status:error", "group_by": ["service"]})),
            ("query_metrics", json!({"queries": ["avg:system.cpu.user{*} by {host}"], "points": 10})),
            ("search_metrics", json!({"query": "system cpu"})),
            ("search_spans", json!({"query": "service:checkout"})),
            ("list_services", json!({})),
            ("search_monitors", json!({"statuses": ["alert"]})),
            ("search_incidents", json!({})),
            ("search_events", json!({})),
            ("search_hosts", json!({})),
            ("search_kubernetes", json!({"kind": "pods"})),
            ("search_dashboards", json!({})),
            ("search_error_issues", json!({})),
            ("search_security_signals", json!({})),
            ("search_vulnerabilities", json!({"severity": ["critical"]})),
            ("search_code_security_findings", json!({"kind": "sast"})),
        ];
        for (name, args) in calls {
            let (status, b) = post(&s.base, Some(&s.app_key), call(name, args)).await;
            assert_eq!(status, StatusCode::OK, "{name}");
            assert_eq!(b["result"]["isError"], false, "{name}: {b}");
        }
        let (_, b) = post(&s.base, Some(&s.app_key), call("get_dashboard", json!({"id": "no-such-dashboard"}))).await;
        assert_eq!(b["result"]["isError"], true, "{b}");
    }

    #[tokio::test]
    async fn tools_see_only_the_keys_organization() {
        let Some(s) = server("mcp-tenant-a").await else { return };
        // Another org with a monitor and a dashboard of its own.
        let other = datadog_api::replay::new_org_with_roles(&s.st, "mcp-tenant-b").await;
        let (_, other_key) = datadog_api::replay::user_with_app_key(&s.st, &other, "admin").await;
        let monitor = json!({"name": "mcp-tenancy-secret-monitor", "type": "metric alert", "query": "avg(last_5m):avg:system.cpu.user{*} > 90", "message": "x"});
        let res = reqwest::Client::new().post(format!("{}/api/v1/monitor", s.base)).header("dd-application-key", &other_key).json(&monitor).send().await.unwrap();
        assert!(res.status().is_success(), "{}", res.text().await.unwrap_or_default());
        let (_, b) = post(&s.base, Some(&other_key), call("search_monitors", json!({"query": "mcp-tenancy-secret"}))).await;
        assert!(b["result"]["content"][0]["text"].as_str().unwrap().contains("mcp-tenancy-secret-monitor"), "the owner sees it: {b}");
        let (_, b) = post(&s.base, Some(&s.app_key), call("search_monitors", json!({"query": "mcp-tenancy-secret"}))).await;
        let text = b["result"]["content"][0]["text"].as_str().unwrap();
        assert!(!text.contains("mcp-tenancy-secret-monitor"), "another org's monitor leaked: {text}");
        // Arguments can't pick the org.
        let (_, b) = post(&s.base, Some(&s.app_key), call("search_monitors", json!({"query": "mcp-tenancy-secret", "org_id": other}))).await;
        assert!(!b["result"]["content"][0]["text"].as_str().unwrap_or("").contains("mcp-tenancy-secret-monitor"));
    }
}
