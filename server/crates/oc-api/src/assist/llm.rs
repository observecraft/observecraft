//! The model behind Assist: Anthropic's Messages API or any OpenAI-compatible chat completions
//! endpoint (OpenAI, Azure OpenAI, vLLM, LiteLLM, Ollama…), both streamed. A conversation is kept
//! provider-neutral ([`Msg`]) and converted per request; each streamed turn becomes a [`Turn`] with its
//! text and tool calls. Anthropic content blocks (thinking with its signature, fallbacks) are kept as
//! sent and replayed unchanged on the next request of the tool loop.
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

/// A tool the model may call.
#[derive(Clone, Debug)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    /// JSON Schema of the input (an object).
    pub schema: Value,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    /// The parsed input; `Err` with the text received when it isn't a JSON object.
    pub input: Result<Value, String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ToolResult {
    pub id: String,
    pub content: String,
    pub is_error: bool,
}

/// A conversation message, independent of the provider.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Msg {
    User(String),
    Assistant {
        text: String,
        calls: Vec<ToolCall>,
        /// Anthropic's content blocks of this turn, replayed as they came.
        blocks: Option<Value>,
    },
    /// The results of the previous assistant turn's tool calls, all in one message.
    Results(Vec<ToolResult>),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Stop {
    EndTurn,
    ToolUse,
    MaxTokens,
    /// The model declined; the explanation when it gives one.
    Refusal(String),
    Other(String),
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Usage {
    pub input: u64,
    pub output: u64,
}

/// One streamed model turn, complete.
#[derive(Clone, Debug, PartialEq)]
pub struct Turn {
    pub text: String,
    pub calls: Vec<ToolCall>,
    pub blocks: Option<Value>,
    pub stop: Stop,
    pub usage: Usage,
}

impl Turn {
    pub fn into_msg(self) -> Msg {
        Msg::Assistant { text: self.text, calls: self.calls, blocks: self.blocks }
    }
}

#[derive(Clone, PartialEq)]
pub enum Provider {
    Anthropic {
        base_url: String,
        api_key: String,
        model: String,
        /// Server-side fallback to another model when the requested one declines (`fallbacks: "default"`).
        fallbacks: bool,
    },
    OpenAi {
        base_url: String,
        api_key: Option<String>,
        model: String,
    },
}

/// Without the API key (the install's `Config` is logged with `{:?}`).
impl std::fmt::Debug for Provider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Provider::Anthropic { base_url, model, fallbacks, .. } => f.debug_struct("Anthropic").field("base_url", base_url).field("model", model).field("fallbacks", fallbacks).finish_non_exhaustive(),
            Provider::OpenAi { base_url, model, .. } => f.debug_struct("OpenAi").field("base_url", base_url).field("model", model).finish_non_exhaustive(),
        }
    }
}

/// Anthropic's default model.
pub const ANTHROPIC_MODEL: &str = "claude-opus-5";
const ANTHROPIC_VERSION: &str = "2023-06-01";
const FALLBACK_BETA: &str = "server-side-fallback-2026-07-01";

/// One HTTP request to the provider.
#[derive(Debug)]
pub struct Request {
    pub url: String,
    pub headers: Vec<(&'static str, String)>,
    pub body: Value,
}

impl Provider {
    pub fn model(&self) -> &str {
        match self {
            Provider::Anthropic { model, .. } | Provider::OpenAi { model, .. } => model,
        }
    }

    /// The streamed request of one turn. The system prompt and the tools come first and don't change
    /// within a conversation, so the provider's prompt cache holds them.
    pub fn request(&self, system: &str, tools: &[ToolSpec], msgs: &[Msg], max_tokens: u32) -> Request {
        match self {
            Provider::Anthropic { base_url, api_key, model, fallbacks } => {
                let mut body = json!({
                    "model": model,
                    "max_tokens": max_tokens,
                    "stream": true,
                    "system": system,
                    "thinking": {"type": "adaptive"},
                    // Caches the longest stable prefix (tools, system, earlier turns).
                    "cache_control": {"type": "ephemeral"},
                    "tools": tools.iter().map(|t| json!({"name": t.name, "description": t.description, "input_schema": t.schema, "eager_input_streaming": true})).collect::<Vec<_>>(),
                    "messages": anthropic_messages(msgs),
                });
                let mut headers = vec![("x-api-key", api_key.clone()), ("anthropic-version", ANTHROPIC_VERSION.to_string())];
                if *fallbacks {
                    body["fallbacks"] = json!("default");
                    headers.push(("anthropic-beta", FALLBACK_BETA.to_string()));
                }
                Request { url: format!("{}/v1/messages", base_url.trim_end_matches('/')), headers, body }
            }
            Provider::OpenAi { base_url, api_key, model } => {
                let mut messages = vec![json!({"role": "system", "content": system})];
                messages.extend(openai_messages(msgs));
                let mut body = json!({
                    "model": model,
                    "max_tokens": max_tokens,
                    "stream": true,
                    "stream_options": {"include_usage": true},
                    "messages": messages,
                });
                if !tools.is_empty() {
                    body["tools"] = json!(tools.iter().map(|t| json!({"type": "function", "function": {"name": t.name, "description": t.description, "parameters": t.schema}})).collect::<Vec<_>>());
                }
                let headers = api_key.iter().map(|k| ("authorization", format!("Bearer {k}"))).collect();
                Request { url: format!("{}/chat/completions", base_url.trim_end_matches('/')), headers, body }
            }
        }
    }

    pub fn stream(&self) -> Box<dyn TurnStream + Send> {
        match self {
            Provider::Anthropic { .. } => Box::<AnthropicStream>::default(),
            Provider::OpenAi { .. } => Box::<OpenAiStream>::default(),
        }
    }
}

fn anthropic_messages(msgs: &[Msg]) -> Vec<Value> {
    msgs.iter()
        .map(|m| match m {
            Msg::User(text) => json!({"role": "user", "content": text}),
            Msg::Assistant { blocks: Some(blocks), .. } => json!({"role": "assistant", "content": blocks}),
            Msg::Assistant { text, calls, blocks: None } => {
                let mut content: Vec<Value> = Vec::new();
                if !text.is_empty() {
                    content.push(json!({"type": "text", "text": text}));
                }
                content.extend(calls.iter().map(|c| json!({"type": "tool_use", "id": c.id, "name": c.name, "input": c.input.clone().unwrap_or_else(|_| json!({}))})));
                json!({"role": "assistant", "content": content})
            }
            Msg::Results(results) => json!({"role": "user", "content": results.iter().map(|r| json!({"type": "tool_result", "tool_use_id": r.id, "content": r.content, "is_error": r.is_error})).collect::<Vec<_>>()}),
        })
        .collect()
}

fn openai_messages(msgs: &[Msg]) -> Vec<Value> {
    let mut out = Vec::new();
    for m in msgs {
        match m {
            Msg::User(text) => out.push(json!({"role": "user", "content": text})),
            Msg::Assistant { text, calls, .. } => {
                let mut msg = json!({"role": "assistant", "content": if text.is_empty() { Value::Null } else { json!(text) }});
                if !calls.is_empty() {
                    msg["tool_calls"] = json!(calls
                        .iter()
                        .map(|c| json!({"id": c.id, "type": "function", "function": {"name": c.name, "arguments": match &c.input { Ok(v) => v.to_string(), Err(raw) => raw.clone() }}}))
                        .collect::<Vec<_>>());
                }
                out.push(msg);
            }
            Msg::Results(results) => out.extend(results.iter().map(|r| json!({"role": "tool", "tool_call_id": r.id, "content": r.content}))),
        }
    }
    out
}

/// Splits a Server-Sent Events byte stream into `(event, data)` records, across chunk boundaries.
#[derive(Default)]
pub struct SseParser {
    buf: Vec<u8>,
    event: String,
    data: String,
}

impl SseParser {
    pub fn feed(&mut self, chunk: &[u8]) -> Vec<(String, String)> {
        self.buf.extend_from_slice(chunk);
        let mut out = Vec::new();
        while let Some(end) = self.buf.iter().position(|b| *b == b'\n') {
            let line: Vec<u8> = self.buf.drain(..=end).collect();
            let line = String::from_utf8_lossy(&line);
            let line = line.trim_end_matches(['\n', '\r']);
            if line.is_empty() {
                if !self.data.is_empty() || !self.event.is_empty() {
                    out.push((std::mem::take(&mut self.event), std::mem::take(&mut self.data)));
                }
            } else if let Some(v) = line.strip_prefix("data:") {
                if !self.data.is_empty() {
                    self.data.push('\n');
                }
                self.data.push_str(v.strip_prefix(' ').unwrap_or(v));
            } else if let Some(v) = line.strip_prefix("event:") {
                self.event = v.trim().to_string();
            }
        }
        out
    }
}

/// Turns a provider's stream events into text deltas and, at the end, the [`Turn`].
pub trait TurnStream {
    /// Handles one SSE record; returns the text it adds, if any.
    fn event(&mut self, event: &str, data: &str) -> Result<Option<String>, String>;
    fn finish(self: Box<Self>) -> Result<Turn, String>;
}

fn parse_input(raw: &str) -> Result<Value, String> {
    if raw.trim().is_empty() {
        return Ok(json!({}));
    }
    match serde_json::from_str::<Value>(raw) {
        Ok(v @ Value::Object(_)) => Ok(v),
        _ => Err(raw.to_string()),
    }
}

#[derive(Default)]
struct AnthropicStream {
    blocks: Vec<Value>,
    /// Streamed tool input JSON per block index.
    partial: Map<String, Value>,
    stop: Option<Stop>,
    usage: Usage,
}

impl TurnStream for AnthropicStream {
    fn event(&mut self, _event: &str, data: &str) -> Result<Option<String>, String> {
        let v: Value = serde_json::from_str(data).map_err(|e| format!("Bad stream event: {e}"))?;
        match v["type"].as_str().unwrap_or("") {
            "message_start" => self.usage.input = v["message"]["usage"]["input_tokens"].as_u64().unwrap_or(0),
            "content_block_start" => {
                let index = v["index"].as_u64().unwrap_or(self.blocks.len() as u64) as usize;
                if self.blocks.len() <= index {
                    self.blocks.resize(index + 1, Value::Null);
                }
                self.blocks[index] = v["content_block"].clone();
            }
            "content_block_delta" => {
                let index = v["index"].as_u64().unwrap_or(0) as usize;
                let delta = &v["delta"];
                let Some(block) = self.blocks.get_mut(index) else { return Ok(None) };
                match delta["type"].as_str().unwrap_or("") {
                    "text_delta" => {
                        let text = delta["text"].as_str().unwrap_or("");
                        block["text"] = json!(format!("{}{text}", block["text"].as_str().unwrap_or("")));
                        return Ok(Some(text.to_string()));
                    }
                    "input_json_delta" => {
                        let key = index.to_string();
                        let so_far = self.partial.get(&key).and_then(Value::as_str).unwrap_or("").to_string();
                        self.partial.insert(key, json!(so_far + delta["partial_json"].as_str().unwrap_or("")));
                    }
                    "thinking_delta" => block["thinking"] = json!(format!("{}{}", block["thinking"].as_str().unwrap_or(""), delta["thinking"].as_str().unwrap_or(""))),
                    "signature_delta" => block["signature"] = json!(delta["signature"].as_str().unwrap_or("")),
                    _ => {}
                }
            }
            "content_block_stop" => {
                let index = v["index"].as_u64().unwrap_or(0) as usize;
                if let (Some(block), Some(raw)) = (self.blocks.get_mut(index), self.partial.get(&index.to_string()).and_then(Value::as_str)) {
                    // A truncated or malformed input stays as text; the tool call is then answered with an error.
                    block["input"] = parse_input(raw).unwrap_or_else(|raw| json!({ "__raw": raw }));
                }
            }
            "message_delta" => {
                if let Some(reason) = v["delta"]["stop_reason"].as_str() {
                    self.stop = Some(match reason {
                        "end_turn" | "stop_sequence" => Stop::EndTurn,
                        "tool_use" => Stop::ToolUse,
                        "max_tokens" => Stop::MaxTokens,
                        "refusal" => Stop::Refusal(v["delta"]["stop_details"]["explanation"].as_str().unwrap_or("").to_string()),
                        other => Stop::Other(other.to_string()),
                    });
                }
                self.usage.output = v["usage"]["output_tokens"].as_u64().unwrap_or(self.usage.output);
            }
            "error" => return Err(v["error"]["message"].as_str().unwrap_or("The model returned an error").to_string()),
            _ => {}
        }
        Ok(None)
    }

    fn finish(self: Box<Self>) -> Result<Turn, String> {
        let stop = self.stop.ok_or("The model stream ended early")?;
        let blocks: Vec<Value> = self.blocks.into_iter().filter(|b| !b.is_null()).collect();
        let text = blocks.iter().filter(|b| b["type"] == "text").filter_map(|b| b["text"].as_str()).collect::<Vec<_>>().join("");
        let calls = blocks
            .iter()
            .filter(|b| b["type"] == "tool_use")
            .map(|b| ToolCall {
                id: b["id"].as_str().unwrap_or("").to_string(),
                name: b["name"].as_str().unwrap_or("").to_string(),
                input: match b["input"].get("__raw").and_then(Value::as_str) {
                    Some(raw) => Err(raw.to_string()),
                    None => Ok(b["input"].clone()),
                },
            })
            .collect();
        // A malformed input is replayed as an empty object (its error result tells the model why).
        let replay = blocks.into_iter().map(|mut b| {
            if b["input"].get("__raw").is_some() {
                b["input"] = json!({});
            }
            b
        });
        Ok(Turn { text, calls, blocks: Some(Value::Array(replay.collect())), stop, usage: self.usage })
    }
}

#[derive(Default)]
struct OpenAiStream {
    text: String,
    /// (id, name, arguments) per tool call index.
    calls: Vec<(String, String, String)>,
    stop: Option<Stop>,
    usage: Usage,
}

impl TurnStream for OpenAiStream {
    fn event(&mut self, _event: &str, data: &str) -> Result<Option<String>, String> {
        if data.trim() == "[DONE]" {
            return Ok(None);
        }
        let v: Value = serde_json::from_str(data).map_err(|e| format!("Bad stream event: {e}"))?;
        if let Some(err) = v.get("error") {
            return Err(err["message"].as_str().unwrap_or("The model returned an error").to_string());
        }
        if let Some(u) = v.get("usage").filter(|u| u.is_object()) {
            self.usage = Usage { input: u["prompt_tokens"].as_u64().unwrap_or(0), output: u["completion_tokens"].as_u64().unwrap_or(0) };
        }
        let Some(choice) = v["choices"].get(0) else { return Ok(None) };
        let delta = &choice["delta"];
        for c in delta["tool_calls"].as_array().into_iter().flatten() {
            let index = c["index"].as_u64().unwrap_or(0) as usize;
            if self.calls.len() <= index {
                self.calls.resize(index + 1, Default::default());
            }
            let call = &mut self.calls[index];
            if let Some(id) = c["id"].as_str() {
                call.0 = id.to_string();
            }
            if let Some(name) = c["function"]["name"].as_str() {
                call.1.push_str(name);
            }
            if let Some(args) = c["function"]["arguments"].as_str() {
                call.2.push_str(args);
            }
        }
        if let Some(reason) = choice["finish_reason"].as_str() {
            self.stop = Some(match reason {
                "stop" => Stop::EndTurn,
                "tool_calls" | "function_call" => Stop::ToolUse,
                "length" => Stop::MaxTokens,
                "content_filter" => Stop::Refusal(String::new()),
                other => Stop::Other(other.to_string()),
            });
        }
        let text = delta["content"].as_str().filter(|t| !t.is_empty()).map(str::to_string);
        if let Some(t) = &text {
            self.text.push_str(t);
        }
        Ok(text)
    }

    fn finish(self: Box<Self>) -> Result<Turn, String> {
        let stop = self.stop.ok_or("The model stream ended early")?;
        let calls: Vec<ToolCall> = self
            .calls
            .into_iter()
            .enumerate()
            .map(|(i, (id, name, args))| ToolCall { id: if id.is_empty() { format!("call_{i}") } else { id }, name, input: parse_input(&args) })
            .collect();
        // Some servers end a turn with tool calls as "stop".
        let stop = if stop == Stop::EndTurn && !calls.is_empty() { Stop::ToolUse } else { stop };
        Ok(Turn { text: self.text, calls, blocks: None, stop, usage: self.usage })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn anthropic() -> Provider {
        Provider::Anthropic { base_url: "https://api.anthropic.com".into(), api_key: "sk-test".into(), model: ANTHROPIC_MODEL.into(), fallbacks: true }
    }

    fn openai() -> Provider {
        Provider::OpenAi { base_url: "http://vllm.example.com/v1/".into(), api_key: Some("key".into()), model: "qwen".into() }
    }

    fn tools() -> Vec<ToolSpec> {
        vec![ToolSpec { name: "search_logs".into(), description: "Search logs".into(), schema: json!({"type": "object", "properties": {"query": {"type": "string"}}, "required": ["query"]}) }]
    }

    fn history() -> Vec<Msg> {
        vec![
            Msg::User("Why is checkout failing?".into()),
            Msg::Assistant { text: "Let me look.".into(), calls: vec![ToolCall { id: "t1".into(), name: "search_logs".into(), input: Ok(json!({"query": "service:checkout status:error"})) }], blocks: None },
            Msg::Results(vec![ToolResult { id: "t1".into(), content: "3 errors".into(), is_error: false }]),
        ]
    }

    /// Feeds an SSE body in uneven chunks.
    fn run(provider: &Provider, body: &str) -> (Vec<String>, Turn) {
        let mut parser = SseParser::default();
        let mut stream = provider.stream();
        let mut texts = Vec::new();
        for chunk in body.as_bytes().chunks(7) {
            for (event, data) in parser.feed(chunk) {
                if let Some(t) = stream.event(&event, &data).unwrap() {
                    texts.push(t);
                }
            }
        }
        (texts, stream.finish().unwrap())
    }

    #[test]
    fn an_anthropic_request_streams_with_cached_tools_and_the_fallback() {
        let r = anthropic().request("You are Assist.", &tools(), &history(), 64_000);
        assert_eq!(r.url, "https://api.anthropic.com/v1/messages");
        assert!(r.headers.contains(&("anthropic-version", "2023-06-01".into())));
        assert!(r.headers.contains(&("anthropic-beta", "server-side-fallback-2026-07-01".into())));
        let b = &r.body;
        assert_eq!((b["model"].as_str(), b["stream"].as_bool(), b["fallbacks"].as_str()), (Some("claude-opus-5"), Some(true), Some("default")));
        assert_eq!(b["cache_control"], json!({"type": "ephemeral"}));
        assert_eq!(b["thinking"], json!({"type": "adaptive"}));
        assert_eq!(b["tools"][0]["input_schema"]["required"], json!(["query"]));
        assert_eq!(b["tools"][0]["eager_input_streaming"], json!(true));
        assert_eq!(b["messages"][1]["content"][1], json!({"type": "tool_use", "id": "t1", "name": "search_logs", "input": {"query": "service:checkout status:error"}}));
        // Tool results go back in one user message.
        assert_eq!(b["messages"][2], json!({"role": "user", "content": [{"type": "tool_result", "tool_use_id": "t1", "content": "3 errors", "is_error": false}]}));
        let off = Provider::Anthropic { base_url: "https://api.anthropic.com".into(), api_key: "k".into(), model: ANTHROPIC_MODEL.into(), fallbacks: false };
        assert!(off.request("s", &[], &[], 1).body.get("fallbacks").is_none());
    }

    #[test]
    fn an_openai_request_uses_functions_and_tool_messages() {
        let r = openai().request("You are Assist.", &tools(), &history(), 4096);
        assert_eq!(r.url, "http://vllm.example.com/v1/chat/completions");
        assert_eq!(r.headers, vec![("authorization", "Bearer key".to_string())]);
        let b = &r.body;
        assert_eq!(b["messages"][0], json!({"role": "system", "content": "You are Assist."}));
        assert_eq!(b["tools"][0]["function"]["parameters"]["required"], json!(["query"]));
        assert_eq!(b["messages"][2]["tool_calls"][0]["function"], json!({"name": "search_logs", "arguments": "{\"query\":\"service:checkout status:error\"}"}));
        assert_eq!(b["messages"][3], json!({"role": "tool", "tool_call_id": "t1", "content": "3 errors"}));
        let keyless = Provider::OpenAi { base_url: "http://ollama:11434/v1".into(), api_key: None, model: "llama".into() };
        assert!(keyless.request("s", &[], &[], 1).headers.is_empty());
        assert!(keyless.request("s", &[], &[], 1).body.get("tools").is_none());
    }

    #[test]
    fn an_anthropic_stream_gives_text_tool_calls_and_replayable_blocks() {
        let body = concat!(
            "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":1200}}}\n\n",
            "event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"thinking\",\"thinking\":\"\"}}\n\n",
            "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"signature_delta\",\"signature\":\"sig==\"}}\n\n",
            "event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":1,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\n",
            "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":1,\"delta\":{\"type\":\"text_delta\",\"text\":\"Checking \"}}\n\n",
            "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":1,\"delta\":{\"type\":\"text_delta\",\"text\":\"logs.\"}}\n\n",
            "event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":2,\"content_block\":{\"type\":\"tool_use\",\"id\":\"toolu_1\",\"name\":\"search_logs\",\"input\":{}}}\n\n",
            "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":2,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"{\\\"query\\\": \\\"service:\"}}\n\n",
            "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":2,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"checkout\\\"}\"}}\n\n",
            "event: content_block_stop\ndata: {\"type\":\"content_block_stop\",\"index\":2}\n\n",
            "event: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"tool_use\"},\"usage\":{\"output_tokens\":42}}\n\n",
            "event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n",
        );
        let (texts, turn) = run(&anthropic(), body);
        assert_eq!(texts.concat(), "Checking logs.");
        assert_eq!(turn.text, "Checking logs.");
        assert_eq!(turn.stop, Stop::ToolUse);
        assert_eq!(turn.usage, Usage { input: 1200, output: 42 });
        assert_eq!(turn.calls, vec![ToolCall { id: "toolu_1".into(), name: "search_logs".into(), input: Ok(json!({"query": "service:checkout"})) }]);
        // The thinking block (with its signature) is replayed unchanged.
        let blocks = turn.blocks.clone().unwrap();
        assert_eq!(blocks[0], json!({"type": "thinking", "thinking": "", "signature": "sig=="}));
        let replay = anthropic().request("s", &[], &[Msg::User("q".into()), turn.into_msg()], 1);
        assert_eq!(replay.body["messages"][1]["content"], blocks);
    }

    #[test]
    fn a_malformed_tool_input_is_kept_as_an_error_and_replayed_empty() {
        let body = concat!(
            "data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"tool_use\",\"id\":\"t\",\"name\":\"search_logs\",\"input\":{}}}\n\n",
            "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"{\\\"query\\\": \\\"servi\"}}\n\n",
            "data: {\"type\":\"content_block_stop\",\"index\":0}\n\n",
            "data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"max_tokens\"},\"usage\":{\"output_tokens\":5}}\n\n",
        );
        let (_, turn) = run(&anthropic(), body);
        assert_eq!(turn.stop, Stop::MaxTokens);
        assert_eq!(turn.calls[0].input, Err("{\"query\": \"servi".to_string()));
        assert_eq!(turn.blocks.unwrap()[0]["input"], json!({}));
    }

    #[test]
    fn a_refusal_and_an_error_event_are_reported() {
        let (_, turn) = run(&anthropic(), "data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"refusal\",\"stop_details\":{\"explanation\":\"not allowed\"}},\"usage\":{}}\n\n");
        assert_eq!(turn.stop, Stop::Refusal("not allowed".into()));
        let mut s = anthropic().stream();
        assert_eq!(s.event("error", "{\"type\":\"error\",\"error\":{\"message\":\"Overloaded\"}}"), Err("Overloaded".into()));
        assert!(anthropic().stream().finish().is_err(), "a stream without a stop reason ended early");
    }

    #[test]
    fn an_openai_stream_assembles_tool_calls_from_fragments() {
        let body = concat!(
            "data: {\"choices\":[{\"delta\":{\"content\":\"Looking\"}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call_a\",\"function\":{\"name\":\"search_logs\",\"arguments\":\"{\\\"qu\"}}]}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"ery\\\":\\\"x\\\"}\"}}]}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"tool_calls\"}]}\n\n",
            "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":10,\"completion_tokens\":3}}\n\n",
            "data: [DONE]\n\n",
        );
        let (texts, turn) = run(&openai(), body);
        assert_eq!(texts, vec!["Looking"]);
        assert_eq!(turn.stop, Stop::ToolUse);
        assert_eq!(turn.usage, Usage { input: 10, output: 3 });
        assert_eq!(turn.calls, vec![ToolCall { id: "call_a".into(), name: "search_logs".into(), input: Ok(json!({"query": "x"})) }]);
    }

    #[test]
    fn sse_records_span_chunks_and_multi_line_data() {
        let mut p = SseParser::default();
        assert!(p.feed(b"event: a\r\ndata: one").is_empty());
        assert_eq!(p.feed(b"\r\ndata: two\r\n\r\n: keepalive\n\n"), vec![("a".to_string(), "one\ntwo".to_string())]);
    }
}
