//! The MCP server (ADR-0009): JSON-RPC over Streamable HTTP for AI agents (Claude Code, IDE assistants),
//! with the tools of [`tools`]. This module is the protocol only: parsing a POST, checking its protocol
//! version and headers, and answering `initialize`, `server/discover`, `ping`, `tools/list` and
//! `tools/call` over a [`Toolbox`]. The route is `routes/mcp.rs`.
//!
//! Dual-era: a request carrying `_meta["io.modelcontextprotocol/protocolVersion"]` follows revision
//! 2026-07-28 (stateless, headers mirrored from the body); `initialize` and requests without it follow
//! the earlier revisions. Nothing is kept between requests, so there are no sessions in either.
pub mod tools;

use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use base64::Engine;
use serde_json::{json, Value};

/// The stateless revision.
pub const MODERN: &str = "2026-07-28";
/// Revisions that open with `initialize`, newest first.
pub const LEGACY: [&str; 3] = ["2025-11-25", "2025-06-18", "2025-03-26"];
/// What a legacy request without `MCP-Protocol-Version` speaks.
const UNSTATED: &str = "2025-03-26";
const VERSION_META: &str = "io.modelcontextprotocol/protocolVersion";
/// How long a 2026-07-28 client may cache `tools/list`: the tools change only with a release.
pub const LIST_TTL_MS: u64 = 3_600_000;

pub const PARSE_ERROR: i64 = -32700;
pub const INVALID_REQUEST: i64 = -32600;
pub const METHOD_NOT_FOUND: i64 = -32601;
pub const INVALID_PARAMS: i64 = -32602;
pub const HEADER_MISMATCH: i64 = -32020;
pub const UNSUPPORTED_VERSION: i64 = -32022;

const INSTRUCTIONS: &str = "ObserveCraft: logs, metrics, traces, monitors, incidents, events, hosts, Kubernetes, dashboards and security findings of \
    the organization the key belongs to. Tools only read. Start broad (list_services, search_monitors with statuses [\"alert\"], aggregate_logs \
    by service) and narrow down; times are relative (15m, 1h, 1d) or epoch milliseconds, at most 31 days. Log and span queries use the search \
    syntax `service:checkout status:error @http.status_code:>=500 \"timed out\" -env:staging`; metric queries `avg:system.cpu.user{env:prod} by {host}`.";

/// Every protocol version served.
pub fn supported() -> Vec<&'static str> {
    std::iter::once(MODERN).chain(LEGACY).collect()
}

/// The two ways a request can be served.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Era {
    /// 2026-07-28: per-request metadata, `resultType` on results, 404 for unknown methods.
    Modern,
    /// `initialize`-based revisions.
    Legacy,
}

/// A JSON-RPC request to answer.
#[derive(Debug)]
pub struct Request {
    pub id: Value,
    pub method: String,
    pub params: Value,
    pub era: Era,
}

/// A POST's message: a request, or something answered with 202 (notifications, client responses).
#[derive(Debug)]
pub enum Message {
    Request(Request),
    Accepted,
}

/// An HTTP answer: a status and a JSON-RPC body (none for 202).
#[derive(Debug)]
pub struct Reply {
    pub status: StatusCode,
    pub body: Option<Value>,
}

impl Reply {
    fn ok(body: Value) -> Self {
        Reply { status: StatusCode::OK, body: Some(body) }
    }

    /// A JSON-RPC error (`id` null when the request had none or couldn't be read).
    pub fn error(status: StatusCode, id: Option<&Value>, code: i64, message: impl Into<String>, data: Option<Value>) -> Self {
        let mut error = json!({"code": code, "message": message.into()});
        if let Some(d) = data {
            error["data"] = d;
        }
        Reply { status, body: Some(json!({"jsonrpc": "2.0", "id": id.cloned().unwrap_or(Value::Null), "error": error})) }
    }
}

impl IntoResponse for Reply {
    fn into_response(self) -> Response {
        match self.body {
            Some(b) => (self.status, Json(b)).into_response(),
            None => self.status.into_response(),
        }
    }
}

/// A tool's answer: text for the model, and whether it is an error the model can correct.
#[derive(Debug, PartialEq)]
pub struct ToolOutcome {
    pub text: String,
    pub is_error: bool,
}

/// The tools a request may list and call.
pub trait Toolbox {
    /// Tool definitions (`name`, `description`, `inputSchema`, `annotations`), in a stable order.
    fn tools(&self) -> Vec<Value>;
    /// None when there is no such tool.
    fn call(&self, name: &str, args: &Value) -> impl std::future::Future<Output = Option<ToolOutcome>> + Send;
}

fn header<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers.get(name).and_then(|v| v.to_str().ok()).map(str::trim)
}

/// A mirrored header's value: as is, or decoded from `=?base64?…?=`.
fn decode_header(v: &str) -> Option<String> {
    match v.strip_prefix("=?base64?").and_then(|r| r.strip_suffix("?=")) {
        Some(b64) => base64::engine::general_purpose::STANDARD.decode(b64).ok().and_then(|b| String::from_utf8(b).ok()),
        None => Some(v.to_string()),
    }
}

/// Reads a POST body, checking its protocol version and (2026-07-28) the headers mirrored from it.
pub fn parse(headers: &HeaderMap, body: &[u8]) -> Result<Message, Reply> {
    let bad = |id: Option<&Value>, code, message: &str| Reply::error(StatusCode::BAD_REQUEST, id, code, message, None);
    let v: Value = serde_json::from_slice(body).map_err(|e| bad(None, PARSE_ERROR, &format!("Parse error: {e}")))?;
    if v.is_array() {
        return Err(bad(None, INVALID_REQUEST, "Batches are not supported: send one message per request"));
    }
    if v["jsonrpc"] != "2.0" {
        return Err(bad(v.get("id"), INVALID_REQUEST, "Not a JSON-RPC 2.0 message"));
    }
    let Some(method) = v["method"].as_str() else {
        // A client's response (legacy revisions allowed them): nothing to answer.
        return if v.get("result").is_some() || v.get("error").is_some() { Ok(Message::Accepted) } else { Err(bad(v.get("id"), INVALID_REQUEST, "Missing method")) };
    };
    let Some(id) = v.get("id").filter(|i| !i.is_null()) else {
        return Ok(Message::Accepted);
    };
    if !(id.is_string() || id.is_i64() || id.is_u64()) {
        return Err(bad(None, INVALID_REQUEST, "The id must be a string or an integer"));
    }
    let params = v.get("params").cloned().unwrap_or_else(|| json!({}));
    if !params.is_object() {
        return Err(bad(Some(id), INVALID_PARAMS, "params must be an object"));
    }
    let stated = header(headers, "mcp-protocol-version");
    let unsupported = |requested: &str| {
        Reply::error(StatusCode::BAD_REQUEST, Some(id), UNSUPPORTED_VERSION, "Unsupported protocol version", Some(json!({"supported": supported(), "requested": requested})))
    };
    let meta_version = params["_meta"][VERSION_META].as_str();
    let era = match meta_version {
        Some(version) if method != "initialize" && !LEGACY.contains(&version) => {
            let mismatch = |m: String| Reply::error(StatusCode::BAD_REQUEST, Some(id), HEADER_MISMATCH, m, None);
            if stated != Some(version) {
                return Err(mismatch(format!("Header mismatch: MCP-Protocol-Version {:?} does not match the body's {version}", stated.unwrap_or(""))));
            }
            if version != MODERN {
                return Err(unsupported(version));
            }
            if header(headers, "mcp-method") != Some(method) {
                return Err(mismatch(format!("Header mismatch: Mcp-Method does not match the body's {method}")));
            }
            let named = match method {
                "tools/call" | "prompts/get" => params["name"].as_str(),
                "resources/read" => params["uri"].as_str(),
                _ => None,
            };
            if let Some(name) = named {
                if header(headers, "mcp-name").and_then(decode_header).as_deref() != Some(name) {
                    return Err(mismatch(format!("Header mismatch: Mcp-Name does not match the body's {name}")));
                }
            }
            Era::Modern
        }
        _ => {
            if method != "initialize" {
                let version = stated.unwrap_or(UNSTATED);
                if !LEGACY.contains(&version) {
                    return Err(unsupported(version));
                }
            }
            Era::Legacy
        }
    };
    Ok(Message::Request(Request { id: id.clone(), method: method.to_string(), params, era }))
}

fn server_info() -> Value {
    json!({"name": "observecraft", "title": "ObserveCraft", "version": env!("CARGO_PKG_VERSION")})
}

fn capabilities() -> Value {
    json!({"tools": {"listChanged": false}})
}

/// Answers a request.
pub async fn dispatch(req: Request, toolbox: &(impl Toolbox + Sync)) -> Reply {
    let id = &req.id;
    let invalid = |m: String| Reply::error(StatusCode::OK, Some(id), INVALID_PARAMS, m, None);
    let mut result = match req.method.as_str() {
        "initialize" => {
            let asked = req.params["protocolVersion"].as_str().unwrap_or("");
            let version = LEGACY.iter().find(|v| **v == asked).copied().unwrap_or(LEGACY[0]);
            json!({"protocolVersion": version, "capabilities": capabilities(), "serverInfo": server_info(), "instructions": INSTRUCTIONS})
        }
        "server/discover" => json!({"supportedVersions": supported(), "capabilities": capabilities(), "_meta": {"io.modelcontextprotocol/serverInfo": server_info()}, "instructions": INSTRUCTIONS}),
        "ping" => json!({}),
        "tools/list" => json!({"tools": toolbox.tools()}),
        "tools/call" => {
            let Some(name) = req.params["name"].as_str() else { return invalid("tools/call needs the tool's name".into()) };
            let args = req.params.get("arguments").filter(|a| !a.is_null()).cloned().unwrap_or_else(|| json!({}));
            if !args.is_object() {
                return invalid("arguments must be an object".into());
            }
            match toolbox.call(name, &args).await {
                None => return invalid(format!("Unknown tool: {name}")),
                Some(out) => json!({"content": [{"type": "text", "text": out.text}], "isError": out.is_error}),
            }
        }
        other => {
            let status = if req.era == Era::Modern { StatusCode::NOT_FOUND } else { StatusCode::OK };
            return Reply::error(status, Some(id), METHOD_NOT_FOUND, format!("Method not found: {other}"), None);
        }
    };
    if req.era == Era::Modern {
        result["resultType"] = json!("complete");
        if req.method == "tools/list" {
            result["ttlMs"] = json!(LIST_TTL_MS);
            result["cacheScope"] = json!("private");
        }
    }
    Reply::ok(json!({"jsonrpc": "2.0", "id": id, "result": result}))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    struct Echo;

    impl Toolbox for Echo {
        fn tools(&self) -> Vec<Value> {
            vec![json!({"name": "echo", "description": "Echoes", "inputSchema": {"type": "object"}})]
        }
        async fn call(&self, name: &str, args: &Value) -> Option<ToolOutcome> {
            (name == "echo").then(|| match args["say"].as_str() {
                Some(s) => ToolOutcome { text: s.to_string(), is_error: false },
                None => ToolOutcome { text: "Give `say`".into(), is_error: true },
            })
        }
    }

    fn headers(pairs: &[(&'static str, &str)]) -> HeaderMap {
        let mut h = HeaderMap::new();
        for (k, v) in pairs {
            h.insert(*k, HeaderValue::from_str(v).unwrap());
        }
        h
    }

    async fn answer(h: &HeaderMap, body: Value) -> Reply {
        match parse(h, body.to_string().as_bytes()) {
            Ok(Message::Request(r)) => dispatch(r, &Echo).await,
            Ok(Message::Accepted) => Reply { status: StatusCode::ACCEPTED, body: None },
            Err(e) => e,
        }
    }

    fn modern(method: &str, name: Option<&str>) -> HeaderMap {
        let mut h = headers(&[("mcp-protocol-version", MODERN), ("mcp-method", method)]);
        if let Some(n) = name {
            h.insert("mcp-name", HeaderValue::from_str(n).unwrap());
        }
        h
    }

    fn meta() -> Value {
        json!({VERSION_META: MODERN, "io.modelcontextprotocol/clientInfo": {"name": "test", "version": "1"}, "io.modelcontextprotocol/clientCapabilities": {}})
    }

    #[tokio::test]
    async fn legacy_handshake_lists_and_calls_tools() {
        let r = answer(&HeaderMap::new(), json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "claude-code", "version": "2"}}})).await;
        assert_eq!(r.status, StatusCode::OK);
        let b = r.body.unwrap();
        assert_eq!(b["id"], 1);
        assert_eq!(b["result"]["protocolVersion"], "2025-06-18");
        assert_eq!(b["result"]["serverInfo"]["name"], "observecraft");
        assert!(b["result"]["capabilities"]["tools"].is_object());
        assert!(b["result"].get("resultType").is_none());

        let h = headers(&[("mcp-protocol-version", "2025-06-18")]);
        let r = answer(&h, json!({"jsonrpc": "2.0", "method": "notifications/initialized"})).await;
        assert_eq!((r.status, r.body.is_none()), (StatusCode::ACCEPTED, true));

        let b = answer(&h, json!({"jsonrpc": "2.0", "id": "two", "method": "tools/list"})).await.body.unwrap();
        assert_eq!(b["id"], "two");
        assert_eq!(b["result"]["tools"][0]["name"], "echo");
        assert!(b["result"].get("ttlMs").is_none());

        let b = answer(&h, json!({"jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": {"name": "echo", "arguments": {"say": "hi"}}})).await.body.unwrap();
        assert_eq!(b["result"], json!({"content": [{"type": "text", "text": "hi"}], "isError": false}));

        let b = answer(&h, json!({"jsonrpc": "2.0", "id": 4, "method": "ping"})).await.body.unwrap();
        assert_eq!(b["result"], json!({}));
    }

    #[tokio::test]
    async fn initialize_offers_the_newest_legacy_version_for_unknown_ones() {
        let b = answer(&HeaderMap::new(), json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2024-11-05"}})).await.body.unwrap();
        assert_eq!(b["result"]["protocolVersion"], LEGACY[0]);
    }

    #[tokio::test]
    async fn modern_requests_are_stateless_and_typed() {
        let b = answer(&modern("server/discover", None), json!({"jsonrpc": "2.0", "id": 1, "method": "server/discover", "params": {"_meta": meta()}})).await.body.unwrap();
        assert_eq!(b["result"]["supportedVersions"], json!(supported()));
        assert_eq!(b["result"]["resultType"], "complete");
        assert_eq!(b["result"]["_meta"]["io.modelcontextprotocol/serverInfo"]["name"], "observecraft");

        // List results say how long a client may cache them, and that they are the key's own.
        let b = answer(&modern("tools/list", None), json!({"jsonrpc": "2.0", "id": 4, "method": "tools/list", "params": {"_meta": meta()}})).await.body.unwrap();
        assert_eq!(b["result"]["tools"][0]["name"], "echo");
        assert_eq!((b["result"]["ttlMs"].clone(), b["result"]["cacheScope"].clone()), (json!(LIST_TTL_MS), json!("private")));

        let b = answer(&modern("tools/call", Some("echo")), json!({"jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": {"name": "echo", "arguments": {"say": "hi"}, "_meta": meta()}})).await.body.unwrap();
        assert_eq!(b["result"]["content"][0]["text"], "hi");
        assert_eq!(b["result"]["resultType"], "complete");

        // Mcp-Name may be Base64 in the sentinel form.
        let h = modern("tools/call", Some("=?base64?ZWNobw==?="));
        let r = answer(&h, json!({"jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": {"name": "echo", "arguments": {"say": "b64"}, "_meta": meta()}})).await;
        assert_eq!(r.status, StatusCode::OK, "{:?}", r.body);
    }

    #[tokio::test]
    async fn modern_headers_must_match_the_body() {
        let call = |name: &str| json!({"jsonrpc": "2.0", "id": 9, "method": "tools/call", "params": {"name": name, "arguments": {}, "_meta": meta()}});
        for h in [modern("tools/call", Some("other")), modern("tools/call", None), modern("tools/list", Some("echo")), headers(&[("mcp-method", "tools/call"), ("mcp-name", "echo")])] {
            let r = answer(&h, call("echo")).await;
            assert_eq!(r.status, StatusCode::BAD_REQUEST);
            let b = r.body.unwrap();
            assert_eq!((b["id"].clone(), b["error"]["code"].clone()), (json!(9), json!(HEADER_MISMATCH)), "{b}");
        }
    }

    #[tokio::test]
    async fn unsupported_versions_list_the_supported_ones() {
        let mut m = meta();
        m[VERSION_META] = json!("2031-01-01");
        let h = headers(&[("mcp-protocol-version", "2031-01-01"), ("mcp-method", "tools/list")]);
        let r = answer(&h, json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list", "params": {"_meta": m}})).await;
        assert_eq!(r.status, StatusCode::BAD_REQUEST);
        let b = r.body.unwrap();
        assert_eq!(b["error"]["code"], UNSUPPORTED_VERSION);
        assert_eq!(b["error"]["data"], json!({"supported": supported(), "requested": "2031-01-01"}));

        // A legacy request naming an unknown version in its header.
        let r = answer(&headers(&[("mcp-protocol-version", "1999-01-01")]), json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"})).await;
        assert_eq!(r.status, StatusCode::BAD_REQUEST);
        assert_eq!(r.body.unwrap()["error"]["code"], UNSUPPORTED_VERSION);
    }

    #[tokio::test]
    async fn unknown_methods_and_tools_are_json_rpc_errors() {
        let b = answer(&modern("resources/list", None), json!({"jsonrpc": "2.0", "id": 1, "method": "resources/list", "params": {"_meta": meta()}})).await;
        assert_eq!(b.status, StatusCode::NOT_FOUND);
        assert_eq!(b.body.unwrap()["error"]["code"], METHOD_NOT_FOUND);
        let b = answer(&HeaderMap::new(), json!({"jsonrpc": "2.0", "id": 1, "method": "prompts/list"})).await;
        assert_eq!(b.status, StatusCode::OK);
        assert_eq!(b.body.unwrap()["error"]["code"], METHOD_NOT_FOUND);
        let b = answer(&HeaderMap::new(), json!({"jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": {"name": "nope"}})).await.body.unwrap();
        assert_eq!(b["error"]["code"], INVALID_PARAMS);
        assert_eq!(b["error"]["message"], "Unknown tool: nope");
        let b = answer(&HeaderMap::new(), json!({"jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": {"name": "echo", "arguments": [1]}})).await.body.unwrap();
        assert_eq!(b["error"]["code"], INVALID_PARAMS);
    }

    #[tokio::test]
    async fn invalid_arguments_are_tool_errors_the_model_can_fix() {
        let b = answer(&HeaderMap::new(), json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": "echo"}})).await.body.unwrap();
        assert_eq!(b["result"]["isError"], true);
        assert_eq!(b["result"]["content"][0]["text"], "Give `say`");
    }

    #[test]
    fn malformed_messages_are_rejected() {
        let h = HeaderMap::new();
        let err = |body: &str| match parse(&h, body.as_bytes()) {
            Err(r) => (r.status, r.body.unwrap()["error"]["code"].as_i64().unwrap()),
            Ok(m) => panic!("accepted {body}: {m:?}"),
        };
        assert_eq!(err("{not json"), (StatusCode::BAD_REQUEST, PARSE_ERROR));
        assert_eq!(err(r#"[{"jsonrpc":"2.0","id":1,"method":"ping"}]"#), (StatusCode::BAD_REQUEST, INVALID_REQUEST));
        assert_eq!(err(r#"{"id":1,"method":"ping"}"#), (StatusCode::BAD_REQUEST, INVALID_REQUEST));
        assert_eq!(err(r#"{"jsonrpc":"2.0","id":{"x":1},"method":"ping"}"#), (StatusCode::BAD_REQUEST, INVALID_REQUEST));
        assert_eq!(err(r#"{"jsonrpc":"2.0","id":1,"method":"ping","params":[1]}"#), (StatusCode::BAD_REQUEST, INVALID_PARAMS));
        assert!(matches!(parse(&h, br#"{"jsonrpc":"2.0","id":1,"result":{}}"#), Ok(Message::Accepted)));
    }
}
