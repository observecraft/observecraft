//! Test helpers: a scripted Anthropic API and the events of a streamed reply.
use std::convert::Infallible;

use axum::response::sse::{Event, Sse};
use axum::Json;
use futures::Stream;
use serde_json::{json, Value};

/// The Anthropic Messages API, scripted: each request gets the next SSE body; the request bodies are kept,
/// with the request's `x-api-key` as `_api_key`.
pub async fn fake_anthropic(replies: Vec<String>) -> (String, std::sync::Arc<std::sync::Mutex<Vec<Value>>>) {
    use axum::response::IntoResponse;
    let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::<Value>::new()));
    let replies = std::sync::Arc::new(std::sync::Mutex::new(replies.into_iter().rev().collect::<Vec<_>>()));
    let (s, r) = (seen.clone(), replies.clone());
    let app = axum::Router::new().route(
        "/v1/messages",
        axum::routing::post(move |headers: axum::http::HeaderMap, Json(mut body): Json<Value>| {
            let (s, r) = (s.clone(), r.clone());
            async move {
                body["_api_key"] = headers.get("x-api-key").and_then(|v| v.to_str().ok()).map(Into::into).unwrap_or(Value::Null);
                s.lock().unwrap().push(body);
                let reply = r.lock().unwrap().pop().unwrap_or_default();
                ([("content-type", "text/event-stream")], reply).into_response()
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (format!("http://{addr}"), seen)
}

pub fn sse_text(text: &str) -> String {
    format!(
        "event: message_start\ndata: {}\n\nevent: content_block_start\ndata: {}\n\nevent: content_block_delta\ndata: {}\n\nevent: message_delta\ndata: {}\n\n",
        json!({"type": "message_start", "message": {"usage": {"input_tokens": 100}}}),
        json!({"type": "content_block_start", "index": 0, "content_block": {"type": "text", "text": ""}}),
        json!({"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": text}}),
        json!({"type": "message_delta", "delta": {"stop_reason": "end_turn"}, "usage": {"output_tokens": 7}}),
    )
}

pub fn sse_tool(id: &str, name: &str, input: Value) -> String {
    format!(
        "data: {}\n\ndata: {}\n\ndata: {}\n\ndata: {}\n\ndata: {}\n\n",
        json!({"type": "message_start", "message": {"usage": {"input_tokens": 90}}}),
        json!({"type": "content_block_start", "index": 0, "content_block": {"type": "tool_use", "id": id, "name": name, "input": {}}}),
        json!({"type": "content_block_delta", "index": 0, "delta": {"type": "input_json_delta", "partial_json": input.to_string()}}),
        json!({"type": "content_block_stop", "index": 0}),
        json!({"type": "message_delta", "delta": {"stop_reason": "tool_use"}, "usage": {"output_tokens": 5}}),
    )
}

/// The SSE events of a reply, by name, once it has ended.
pub async fn events(sse: Sse<impl Stream<Item = Result<Event, Infallible>> + Send + 'static>) -> Vec<(String, Value)> {
    use axum::response::IntoResponse;
    let body = tokio::time::timeout(std::time::Duration::from_secs(30), axum::body::to_bytes(sse.into_response().into_body(), usize::MAX)).await.expect("the reply ended").unwrap();
    let mut parser = crate::assist::llm::SseParser::default();
    parser.feed(&body).into_iter().map(|(e, d)| (e, serde_json::from_str(&d).unwrap())).collect()
}
