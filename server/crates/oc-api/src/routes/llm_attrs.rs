//! LLM spans: how ObserveCraft recognizes them among stored spans and reads their model, tokens
//! and messages. No separate pipeline: LLM spans are ordinary spans in `otel.otel_traces` that
//! carry one of these attribute schemes:
//!
//! - OpenTelemetry GenAI semantic conventions (`gen_ai.provider.name` / `gen_ai.system`,
//!   `gen_ai.operation.name`, `gen_ai.request.model`, `gen_ai.usage.*`, messages in
//!   `gen_ai.input.messages` / `gen_ai.output.messages` or in `gen_ai.content.*` events);
//! - OpenLLMetry (`gen_ai.prompt.N.content`, `llm.request.type`, `traceloop.span.kind`, …);
//! - OpenInference (`openinference.span.kind`, `llm.model_name`, `input.value`, …);
//! - dd-trace LLM Observability tags, when they reach the APM span (`_ml_obs.meta.span.kind`,
//!   `_ml_obs.meta.model_name`, `_ml_obs.meta.input.messages`, `_ml_obs.metrics`, …). The
//!   Datadog intake keeps span meta as attributes unchanged.
//!
//! The same key lists build the ClickHouse expressions (facets, aggregates) and the Rust
//! extraction (messages for the trace panel and the evaluations).

use std::collections::HashMap;

use serde::Serialize;
use serde_json::Value;
use utoipa::ToSchema;

/// An attribute whose presence marks a span as an LLM span.
pub const MARKER_KEYS: &[&str] = &[
    "gen_ai.provider.name",
    "gen_ai.system",
    "gen_ai.operation.name",
    "gen_ai.request.model",
    "llm.request.type",
    "traceloop.span.kind",
    "openinference.span.kind",
    "_ml_obs.meta.span.kind",
    "_ml_obs.meta.ml_app",
];
pub const MODEL_KEYS: &[&str] = &["gen_ai.response.model", "gen_ai.request.model", "_ml_obs.meta.model_name", "llm.model_name", "llm.response.model", "llm.request.model"];
pub const PROVIDER_KEYS: &[&str] = &["gen_ai.provider.name", "gen_ai.system", "_ml_obs.meta.model_provider", "llm.provider", "llm.system"];
pub const ML_APP_KEYS: &[&str] = &["_ml_obs.meta.ml_app", "ml_app"];
pub const OPERATION_KEYS: &[&str] = &["gen_ai.operation.name", "llm.request.type"];
/// Explicit span kinds (Datadog, OpenLLMetry, OpenInference), before the operation name.
pub const KIND_KEYS: &[&str] = &["_ml_obs.meta.span.kind", "traceloop.span.kind", "openinference.span.kind"];
pub const INPUT_TOKEN_KEYS: &[&str] = &["gen_ai.usage.input_tokens", "gen_ai.usage.prompt_tokens", "llm.usage.prompt_tokens", "llm.token_count.prompt"];
pub const OUTPUT_TOKEN_KEYS: &[&str] = &["gen_ai.usage.output_tokens", "gen_ai.usage.completion_tokens", "llm.usage.completion_tokens", "llm.token_count.completion"];
/// Datadog's LLM Observability keeps token counts in one JSON tag.
pub const DD_METRICS_KEY: &str = "_ml_obs.metrics";

/// Raw kind / operation names → the kinds shown in the UI.
pub const KIND_NAMES: &[(&str, &str)] = &[
    ("llm", "llm"),
    ("chat", "llm"),
    ("completion", "llm"),
    ("text_completion", "llm"),
    ("generate_content", "llm"),
    ("embedding", "embedding"),
    ("embeddings", "embedding"),
    ("retrieval", "retrieval"),
    ("retriever", "retrieval"),
    ("reranker", "retrieval"),
    ("tool", "tool"),
    ("execute_tool", "tool"),
    ("agent", "agent"),
    ("invoke_agent", "agent"),
    ("create_agent", "agent"),
    ("workflow", "workflow"),
    ("chain", "workflow"),
    ("task", "task"),
    ("guardrail", "task"),
];

/// Kinds that call a model: requests, latency and errors count these, tokens and cost too.
pub const CALL_KINDS: &[&str] = &["llm", "embedding"];

// ================================================================ ClickHouse expressions

fn attr(key: &str) -> String {
    format!("SpanAttributes['{key}']")
}

/// The first non-empty attribute of `keys` ('' when none is set). Keys are constants.
pub fn first_sql(keys: &[&str]) -> String {
    let mut out = String::from("multiIf(");
    for k in keys {
        out.push_str(&format!("{a} != '', {a}, ", a = attr(k)));
    }
    out.push_str("'')");
    out
}

/// The span is an LLM span.
pub fn is_llm_sql() -> String {
    let mut parts: Vec<String> = MARKER_KEYS.iter().map(|k| format!("{} != ''", attr(k))).collect();
    parts.push(format!("{} = 'llm'", attr("span.type")));
    format!("({})", parts.join(" OR "))
}

/// The span carries an evaluation result (an event or the attributes of an evaluator span).
pub fn has_evaluation_sql() -> String {
    format!("(has(Events.Name, '{EVALUATION_EVENT}') OR {} != '')", attr("gen_ai.evaluation.name"))
}

/// The normalized kind (`llm`, `embedding`, `retrieval`, `tool`, `agent`, `workflow`, `task`).
pub fn kind_sql() -> String {
    // No explicit kind or operation (e.g. dd-trace `span.type: llm`): a model call.
    let raw = format!("lower(multiIf({explicit} != '', {explicit}, {op} != '', {op}, 'llm'))", explicit = first_sql(KIND_KEYS), op = first_sql(OPERATION_KEYS));
    let from: Vec<String> = KIND_NAMES.iter().map(|(f, _)| format!("'{f}'")).collect();
    let to: Vec<String> = KIND_NAMES.iter().map(|(_, t)| format!("'{t}'")).collect();
    format!("transform({raw}, [{}], [{}], {raw})", from.join(", "), to.join(", "))
}

/// The span calls a model (kind llm or embedding).
pub fn is_call_sql() -> String {
    format!("({} IN ('llm', 'embedding'))", kind_sql())
}

/// The ML application: the Datadog `ml_app` tag, else the service.
pub fn ml_app_sql() -> String {
    format!("if({app} != '', {app}, toString(ServiceName))", app = first_sql(ML_APP_KEYS))
}

pub fn input_tokens_sql() -> String {
    tokens_sql(INPUT_TOKEN_KEYS, &["input_tokens", "prompt_tokens"])
}

pub fn output_tokens_sql() -> String {
    tokens_sql(OUTPUT_TOKEN_KEYS, &["output_tokens", "completion_tokens"])
}

fn tokens_sql(keys: &[&str], dd: &[&str]) -> String {
    let dd_expr = dd.iter().map(|k| format!("JSONExtractFloat({}, '{k}')", attr(DD_METRICS_KEY))).collect::<Vec<_>>().join(" + ");
    format!("if({v} != '', toFloat64OrZero({v}), {dd_expr})", v = first_sql(keys))
}

/// The attributes the extraction reads, as a JSON object (prefix filter keeps rows small).
pub fn llm_attrs_json_sql() -> String {
    "toJSONString(mapFilter((k, v) -> startsWith(k, 'gen_ai.') OR startsWith(k, 'llm.') OR startsWith(k, '_ml_obs.') OR startsWith(k, 'traceloop.') \
     OR k IN ('input.value', 'output.value', 'openinference.span.kind', 'ml_app', 'span.type', 'error.message'), SpanAttributes))"
        .to_string()
}

// ================================================================ extraction

pub const EVALUATION_EVENT: &str = "gen_ai.evaluation.result";

#[derive(Serialize, ToSchema, Clone, Debug, PartialEq)]
pub struct LlmMessage {
    /// system | user | assistant | tool | document …
    pub role: String,
    pub content: String,
}

/// An evaluation result recorded on the span by the application or an evaluator
/// (`gen_ai.evaluation.result` events or `gen_ai.evaluation.*` attributes).
#[derive(Serialize, ToSchema, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RecordedEvaluation {
    pub name: String,
    pub score: Option<f64>,
    pub label: String,
    pub explanation: String,
    /// From the label: pass / fail words; null when the label says neither.
    pub passed: Option<bool>,
}

/// What a span says about its LLM work.
#[derive(Serialize, ToSchema, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LlmSpanInfo {
    pub kind: String,
    pub ml_app: String,
    pub model: String,
    pub provider: String,
    pub operation: String,
    pub input_tokens: f64,
    pub output_tokens: f64,
    pub input: Vec<LlmMessage>,
    pub output: Vec<LlmMessage>,
    pub evaluations: Vec<RecordedEvaluation>,
}

/// A span event: name and attributes.
pub type Event<'a> = (&'a str, &'a HashMap<String, String>);

fn first<'a>(attrs: &'a HashMap<String, String>, keys: &[&str]) -> &'a str {
    keys.iter().filter_map(|k| attrs.get(*k)).map(String::as_str).find(|v| !v.is_empty()).unwrap_or("")
}

pub fn is_llm(attrs: &HashMap<String, String>) -> bool {
    MARKER_KEYS.iter().any(|k| attrs.get(*k).is_some_and(|v| !v.is_empty())) || attrs.get("span.type").is_some_and(|v| v == "llm")
}

pub fn normalize_kind(raw: &str) -> String {
    let raw = raw.to_lowercase();
    KIND_NAMES.iter().find(|(f, _)| *f == raw).map(|(_, t)| t.to_string()).unwrap_or(raw)
}

fn tokens(attrs: &HashMap<String, String>, keys: &[&str], dd: &[&str]) -> f64 {
    let v = first(attrs, keys);
    if !v.is_empty() {
        return v.parse().unwrap_or(0.0);
    }
    let Some(metrics) = attrs.get(DD_METRICS_KEY).and_then(|m| serde_json::from_str::<Value>(m).ok()) else { return 0.0 };
    dd.iter().filter_map(|k| metrics.get(*k).and_then(Value::as_f64)).sum()
}

/// The LLM view of a span, or None when it isn't an LLM span.
pub fn extract(service: &str, attrs: &HashMap<String, String>, events: &[Event]) -> Option<LlmSpanInfo> {
    let evaluated = events.iter().any(|(n, _)| *n == EVALUATION_EVENT) || attrs.get("gen_ai.evaluation.name").is_some_and(|v| !v.is_empty());
    if !is_llm(attrs) && !evaluated {
        return None;
    }
    let explicit = first(attrs, KIND_KEYS);
    let operation = first(attrs, OPERATION_KEYS).to_string();
    let kind = normalize_kind(if !explicit.is_empty() {
        explicit
    } else if !operation.is_empty() {
        &operation
    } else {
        "llm"
    });
    let app = first(attrs, ML_APP_KEYS);
    Some(LlmSpanInfo {
        kind,
        ml_app: if app.is_empty() { service.to_string() } else { app.to_string() },
        model: first(attrs, MODEL_KEYS).to_string(),
        provider: first(attrs, PROVIDER_KEYS).to_string(),
        operation,
        input_tokens: tokens(attrs, INPUT_TOKEN_KEYS, &["input_tokens", "prompt_tokens"]),
        output_tokens: tokens(attrs, OUTPUT_TOKEN_KEYS, &["output_tokens", "completion_tokens"]),
        input: input_messages(attrs, events),
        output: output_messages(attrs, events),
        evaluations: recorded_evaluations(attrs, events),
    })
}

/// Text of a message's content: a string, a list of parts (`{type, content|text}`, tool calls)
/// or an object with `text` / `content`.
fn content_text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Array(parts) => parts.iter().map(part_text).filter(|s| !s.is_empty()).collect::<Vec<_>>().join("\n"),
        Value::Object(_) => part_text(v),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

fn part_text(p: &Value) -> String {
    if let Value::String(s) = p {
        return s.clone();
    }
    let ty = p.get("type").and_then(Value::as_str).unwrap_or("");
    match ty {
        "tool_call" | "function_call" => {
            let name = p.get("name").and_then(Value::as_str).unwrap_or("tool");
            let args = p.get("arguments").map(|a| a.as_str().map(str::to_string).unwrap_or_else(|| a.to_string())).unwrap_or_default();
            format!("{name}({args})")
        }
        "tool_call_response" => p.get("response").map(content_text).unwrap_or_default(),
        _ => ["content", "text", "value"].iter().find_map(|k| p.get(*k)).map(content_text).unwrap_or_default(),
    }
}

/// One message object (`{role, content | parts}`); tool calls of OpenAI-style messages too.
fn message(v: &Value, default_role: &str) -> Option<LlmMessage> {
    let Value::Object(o) = v else {
        let text = content_text(v);
        return (!text.is_empty()).then(|| LlmMessage { role: default_role.into(), content: text });
    };
    let role = o.get("role").and_then(Value::as_str).unwrap_or(default_role).to_string();
    let mut text = o.get("parts").or_else(|| o.get("content")).or_else(|| o.get("text")).or_else(|| o.get("message").and_then(|m| m.get("content"))).map(content_text).unwrap_or_default();
    if let Some(calls) = o.get("tool_calls").and_then(Value::as_array) {
        for c in calls {
            let f = c.get("function").unwrap_or(c);
            let name = f.get("name").and_then(Value::as_str).unwrap_or("tool");
            let args = f.get("arguments").map(|a| a.as_str().map(str::to_string).unwrap_or_else(|| a.to_string())).unwrap_or_default();
            if !text.is_empty() {
                text.push('\n');
            }
            text.push_str(&format!("{name}({args})"));
        }
    }
    (!text.is_empty()).then_some(LlmMessage { role, content: text })
}

/// Messages from one attribute value: a JSON list of messages, `{messages: [...]}`, a list of
/// documents, a JSON string, or plain text.
fn parse_messages(raw: &str, default_role: &str) -> Vec<LlmMessage> {
    match serde_json::from_str::<Value>(raw) {
        Ok(Value::Array(items)) => items.iter().filter_map(|m| message(m, default_role)).collect(),
        Ok(Value::Object(o)) if o.get("messages").is_some_and(Value::is_array) => o["messages"].as_array().unwrap().iter().filter_map(|m| message(m, default_role)).collect(),
        Ok(v @ Value::Object(_)) => message(&v, default_role).into_iter().collect(),
        Ok(Value::String(s)) if !s.is_empty() => vec![LlmMessage { role: default_role.into(), content: s }],
        _ if !raw.trim().is_empty() => vec![LlmMessage { role: default_role.into(), content: raw.to_string() }],
        _ => vec![],
    }
}

/// OpenLLMetry / OpenInference indexed messages: `{prefix}.{i}.{role_key}` and `.{content_key}`.
fn indexed(attrs: &HashMap<String, String>, prefix: &str, role_key: &str, content_key: &str, default_role: &str) -> Vec<LlmMessage> {
    let mut out = Vec::new();
    for i in 0..256 {
        let content = attrs.get(&format!("{prefix}.{i}.{content_key}"));
        let role = attrs.get(&format!("{prefix}.{i}.{role_key}"));
        if content.is_none() && role.is_none() {
            break;
        }
        if let Some(c) = content.filter(|c| !c.is_empty()) {
            out.push(LlmMessage { role: role.cloned().unwrap_or_else(|| default_role.into()), content: c.clone() });
        }
    }
    out
}

fn input_messages(attrs: &HashMap<String, String>, events: &[Event]) -> Vec<LlmMessage> {
    let mut out = Vec::new();
    if let Some(sys) = attrs.get("gen_ai.system_instructions").filter(|s| !s.is_empty()) {
        out.extend(parse_messages(sys, "system").into_iter().map(|m| LlmMessage { role: "system".into(), ..m }));
    }
    for key in ["gen_ai.input.messages", "_ml_obs.meta.input.messages"] {
        if let Some(v) = attrs.get(key).filter(|v| !v.is_empty()) {
            out.extend(parse_messages(v, "user"));
            return out;
        }
    }
    let idx = indexed(attrs, "gen_ai.prompt", "role", "content", "user");
    if !idx.is_empty() {
        return [out, idx].concat();
    }
    let idx = indexed(attrs, "llm.input_messages", "message.role", "message.content", "user");
    if !idx.is_empty() {
        return [out, idx].concat();
    }
    // Per-message events (gen_ai.system.message, gen_ai.user.message, …) or the older prompt event.
    let from_events: Vec<LlmMessage> = events
        .iter()
        .filter_map(|(name, a)| {
            if *name == "gen_ai.content.prompt" {
                return a.get("gen_ai.prompt").map(|p| parse_messages(p, "user"));
            }
            let role = name.strip_prefix("gen_ai.")?.strip_suffix(".message")?;
            let body = a.get("content").or_else(|| a.get("gen_ai.event.content")).or_else(|| a.get("message"))?;
            Some(parse_messages(body, role).into_iter().map(|m| LlmMessage { role: role.to_string(), ..m }).collect())
        })
        .flatten()
        .collect();
    if !from_events.is_empty() {
        return [out, from_events].concat();
    }
    for key in ["gen_ai.prompt", "_ml_obs.meta.input.value", "input.value", "traceloop.entity.input"] {
        if let Some(v) = attrs.get(key).filter(|v| !v.is_empty()) {
            out.extend(parse_messages(v, "user"));
            return out;
        }
    }
    out
}

fn output_messages(attrs: &HashMap<String, String>, events: &[Event]) -> Vec<LlmMessage> {
    for key in ["gen_ai.output.messages", "_ml_obs.meta.output.messages"] {
        if let Some(v) = attrs.get(key).filter(|v| !v.is_empty()) {
            return parse_messages(v, "assistant");
        }
    }
    let idx = indexed(attrs, "gen_ai.completion", "role", "content", "assistant");
    if !idx.is_empty() {
        return idx;
    }
    let idx = indexed(attrs, "llm.output_messages", "message.role", "message.content", "assistant");
    if !idx.is_empty() {
        return idx;
    }
    let from_events: Vec<LlmMessage> = events
        .iter()
        .filter_map(|(name, a)| match *name {
            "gen_ai.content.completion" => a.get("gen_ai.completion").map(|c| parse_messages(c, "assistant")),
            "gen_ai.choice" => a.get("message").or_else(|| a.get("content")).or_else(|| a.get("gen_ai.event.content")).map(|c| parse_messages(c, "assistant")),
            _ => None,
        })
        .flatten()
        .collect();
    if !from_events.is_empty() {
        return from_events;
    }
    if let Some(docs) = attrs.get("_ml_obs.meta.output.documents").filter(|v| !v.is_empty()) {
        return parse_messages(docs, "document").into_iter().map(|m| LlmMessage { role: "document".into(), ..m }).collect();
    }
    for key in ["gen_ai.completion", "_ml_obs.meta.output.value", "output.value", "traceloop.entity.output"] {
        if let Some(v) = attrs.get(key).filter(|v| !v.is_empty()) {
            return parse_messages(v, "assistant");
        }
    }
    vec![]
}

const PASS_WORDS: &[&str] = &["pass", "passed", "true", "yes", "good", "correct", "relevant", "faithful", "safe", "ok"];
const FAIL_WORDS: &[&str] = &["fail", "failed", "false", "no", "bad", "incorrect", "irrelevant", "unfaithful", "unsafe", "toxic", "hallucinated"];

fn label_passed(label: &str) -> Option<bool> {
    let l = label.trim().to_lowercase();
    if PASS_WORDS.contains(&l.as_str()) {
        Some(true)
    } else if FAIL_WORDS.contains(&l.as_str()) {
        Some(false)
    } else {
        None
    }
}

fn recorded(a: &HashMap<String, String>) -> Option<RecordedEvaluation> {
    let name = a.get("gen_ai.evaluation.name").filter(|n| !n.is_empty())?.clone();
    let label = a.get("gen_ai.evaluation.score.label").cloned().unwrap_or_default();
    Some(RecordedEvaluation {
        name,
        score: a.get("gen_ai.evaluation.score.value").and_then(|v| v.parse().ok()),
        passed: label_passed(&label),
        label,
        explanation: a.get("gen_ai.evaluation.explanation").cloned().unwrap_or_default(),
    })
}

fn recorded_evaluations(attrs: &HashMap<String, String>, events: &[Event]) -> Vec<RecordedEvaluation> {
    let mut out: Vec<RecordedEvaluation> = events.iter().filter(|(n, _)| *n == EVALUATION_EVENT).filter_map(|(_, a)| recorded(a)).collect();
    out.extend(recorded(attrs));
    out
}

/// The text of the messages with one of `roles` (all when empty), joined.
pub fn text_of(messages: &[LlmMessage], roles: &[&str]) -> String {
    messages.iter().filter(|m| roles.is_empty() || roles.contains(&m.role.as_str())).map(|m| m.content.as_str()).collect::<Vec<_>>().join("\n")
}

// ================================================================ cost estimate

/// List prices in USD per million (input, output) tokens, kept by hand for a rough estimate:
/// no cached-input, batch or negotiated discounts, and they go stale. Longest prefix wins.
pub const PRICES: &[(&str, f64, f64)] = &[
    ("gpt-4o-mini", 0.15, 0.60),
    ("gpt-4o", 2.50, 10.00),
    ("gpt-4.1-nano", 0.10, 0.40),
    ("gpt-4.1-mini", 0.40, 1.60),
    ("gpt-4.1", 2.00, 8.00),
    ("gpt-3.5-turbo", 0.50, 1.50),
    ("o4-mini", 1.10, 4.40),
    ("o3-mini", 1.10, 4.40),
    ("o3", 2.00, 8.00),
    ("text-embedding-3-small", 0.02, 0.0),
    ("text-embedding-3-large", 0.13, 0.0),
    ("claude-fable-5", 10.00, 50.00),
    ("claude-opus-5-5", 4.00, 20.00),
    ("claude-opus-5", 5.00, 25.00),
    ("claude-opus-4", 5.00, 25.00),
    ("claude-sonnet-5", 2.00, 10.00),
    ("claude-sonnet-4", 3.00, 15.00),
    ("claude-haiku-4", 1.00, 5.00),
    ("gemini-2.0-flash", 0.10, 0.40),
    ("gemini-1.5-flash", 0.075, 0.30),
    ("gemini-1.5-pro", 1.25, 5.00),
    ("mistral-large", 2.00, 6.00),
    ("mistral-small", 0.20, 0.60),
];

/// (input, output) USD per million tokens for a model name, ignoring provider prefixes
/// (`openai/gpt-4o`, `anthropic.claude-sonnet-4-6`).
pub fn price(model: &str) -> Option<(f64, f64)> {
    let m = model.to_lowercase();
    let m = m.rsplit(['/', ':']).next().unwrap_or(&m);
    let m = m.strip_prefix("anthropic.").or_else(|| m.strip_prefix("us.anthropic.")).unwrap_or(m);
    PRICES.iter().filter(|(p, _, _)| m.starts_with(p)).max_by_key(|(p, _, _)| p.len()).map(|(_, i, o)| (*i, *o))
}

/// Estimated USD for the tokens, None for a model without a price.
pub fn estimate_cost(model: &str, input_tokens: f64, output_tokens: f64) -> Option<f64> {
    price(model).map(|(i, o)| (input_tokens * i + output_tokens * o) / 1e6)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    #[test]
    fn plain_spans_are_not_llm_spans() {
        assert!(extract("checkout", &map(&[("http.route", "/cart")]), &[]).is_none());
    }

    #[test]
    fn reads_genai_semantic_conventions() {
        let attrs = map(&[
            ("gen_ai.provider.name", "openai"),
            ("gen_ai.operation.name", "chat"),
            ("gen_ai.request.model", "gpt-4o-mini"),
            ("gen_ai.response.model", "gpt-4o-mini-2024-07-18"),
            ("gen_ai.usage.input_tokens", "812"),
            ("gen_ai.usage.output_tokens", "96"),
            ("gen_ai.system_instructions", r#"[{"type":"text","content":"You are the shop's support assistant."}]"#),
            ("gen_ai.input.messages", r#"[{"role":"user","parts":[{"type":"text","content":"Where is order 1042?"}]}]"#),
            (
                "gen_ai.output.messages",
                r#"[{"role":"assistant","parts":[{"type":"tool_call","name":"lookup_order","arguments":{"id":1042}},{"type":"text","content":"Checking."}],"finish_reason":"tool_call"}]"#,
            ),
        ]);
        let info = extract("support-assistant", &attrs, &[]).unwrap();
        assert_eq!(info.kind, "llm");
        assert_eq!(info.ml_app, "support-assistant");
        assert_eq!(info.model, "gpt-4o-mini-2024-07-18");
        assert_eq!(info.provider, "openai");
        assert_eq!((info.input_tokens, info.output_tokens), (812.0, 96.0));
        assert_eq!(
            info.input,
            vec![LlmMessage { role: "system".into(), content: "You are the shop's support assistant.".into() }, LlmMessage { role: "user".into(), content: "Where is order 1042?".into() }]
        );
        assert_eq!(info.output[0].role, "assistant");
        assert_eq!(info.output[0].content, "lookup_order({\"id\":1042})\nChecking.");
    }

    #[test]
    fn reads_openllmetry_indexed_prompts() {
        let attrs = map(&[
            ("gen_ai.system", "Anthropic"),
            ("llm.request.type", "chat"),
            ("gen_ai.request.model", "claude-sonnet-4-6"),
            ("gen_ai.usage.prompt_tokens", "40"),
            ("gen_ai.usage.completion_tokens", "12"),
            ("gen_ai.prompt.0.role", "system"),
            ("gen_ai.prompt.0.content", "Be brief."),
            ("gen_ai.prompt.1.role", "user"),
            ("gen_ai.prompt.1.content", "Summarize the review."),
            ("gen_ai.completion.0.role", "assistant"),
            ("gen_ai.completion.0.content", "Customers like the fit."),
        ]);
        let info = extract("review-summarizer", &attrs, &[]).unwrap();
        assert_eq!(info.kind, "llm");
        assert_eq!((info.input_tokens, info.output_tokens), (40.0, 12.0));
        assert_eq!(info.input.len(), 2);
        assert_eq!(text_of(&info.input, &["user"]), "Summarize the review.");
        assert_eq!(text_of(&info.output, &[]), "Customers like the fit.");
    }

    #[test]
    fn reads_datadog_llm_observability_tags() {
        let attrs = map(&[
            ("_ml_obs.meta.span.kind", "workflow"),
            ("_ml_obs.meta.ml_app", "product-search"),
            ("_ml_obs.meta.input.value", "running shoes under 100"),
            ("_ml_obs.meta.output.value", "3 products"),
        ]);
        let info = extract("catalog-service", &attrs, &[]).unwrap();
        assert_eq!(info.kind, "workflow");
        assert_eq!(info.ml_app, "product-search");
        assert_eq!(info.input, vec![LlmMessage { role: "user".into(), content: "running shoes under 100".into() }]);

        let llm = map(&[
            ("_ml_obs.meta.span.kind", "llm"),
            ("_ml_obs.meta.model_name", "gpt-4o"),
            ("_ml_obs.meta.model_provider", "openai"),
            ("_ml_obs.meta.input.messages", r#"[{"role":"user","content":"hi"}]"#),
            ("_ml_obs.meta.output.messages", r#"[{"role":"assistant","content":"hello"}]"#),
            ("_ml_obs.metrics", r#"{"input_tokens":5,"output_tokens":2,"total_tokens":7}"#),
        ]);
        let info = extract("svc", &llm, &[]).unwrap();
        assert_eq!((info.model.as_str(), info.provider.as_str()), ("gpt-4o", "openai"));
        assert_eq!((info.input_tokens, info.output_tokens), (5.0, 2.0));
        assert_eq!(text_of(&info.output, &[]), "hello");
    }

    #[test]
    fn reads_messages_from_events() {
        let attrs = map(&[("gen_ai.system", "openai"), ("gen_ai.operation.name", "chat")]);
        let sys = map(&[("content", "Be nice.")]);
        let user = map(&[("content", "Do you ship to Norway?")]);
        let choice = map(&[("message", r#"{"role":"assistant","content":"Yes, in 5 days."}"#)]);
        let events = [("gen_ai.system.message", &sys), ("gen_ai.user.message", &user), ("gen_ai.choice", &choice)];
        let info = extract("svc", &attrs, &events).unwrap();
        assert_eq!(info.input.iter().map(|m| m.role.as_str()).collect::<Vec<_>>(), ["system", "user"]);
        assert_eq!(text_of(&info.output, &[]), "Yes, in 5 days.");

        let prompt = map(&[("gen_ai.prompt", r#"[{"role":"user","content":"old style"}]"#)]);
        let completion = map(&[("gen_ai.completion", "answer")]);
        let info = extract("svc", &attrs, &[("gen_ai.content.prompt", &prompt), ("gen_ai.content.completion", &completion)]).unwrap();
        assert_eq!(text_of(&info.input, &[]), "old style");
        assert_eq!(text_of(&info.output, &[]), "answer");
    }

    #[test]
    fn normalizes_kinds_from_operations() {
        for (op, kind) in [("embeddings", "embedding"), ("execute_tool", "tool"), ("invoke_agent", "agent"), ("text_completion", "llm")] {
            let info = extract("s", &map(&[("gen_ai.operation.name", op)]), &[]).unwrap();
            assert_eq!(info.kind, kind, "{op}");
        }
        assert_eq!(extract("s", &map(&[("openinference.span.kind", "RETRIEVER")]), &[]).unwrap().kind, "retrieval");
        assert_eq!(extract("s", &map(&[("traceloop.span.kind", "workflow")]), &[]).unwrap().kind, "workflow");
    }

    #[test]
    fn reads_recorded_evaluations() {
        let attrs = map(&[("gen_ai.operation.name", "chat")]);
        let ev =
            map(&[("gen_ai.evaluation.name", "relevance"), ("gen_ai.evaluation.score.value", "0.31"), ("gen_ai.evaluation.score.label", "irrelevant"), ("gen_ai.evaluation.explanation", "off topic")]);
        let info = extract("s", &attrs, &[(EVALUATION_EVENT, &ev)]).unwrap();
        assert_eq!(info.evaluations, vec![RecordedEvaluation { name: "relevance".into(), score: Some(0.31), label: "irrelevant".into(), explanation: "off topic".into(), passed: Some(false) }]);
    }

    #[test]
    fn prices_by_longest_prefix() {
        assert_eq!(price("gpt-4o-mini-2024-07-18"), Some((0.15, 0.60)));
        assert_eq!(price("gpt-4o-2024-08-06"), Some((2.50, 10.00)));
        assert_eq!(price("openai/gpt-4o"), Some((2.50, 10.00)));
        assert_eq!(price("claude-opus-5-5"), Some((4.00, 20.00)));
        assert_eq!(price("anthropic.claude-sonnet-4-6"), Some((3.00, 15.00)));
        assert_eq!(price("my-finetune"), None);
        let cost = estimate_cost("gpt-4o", 1_000_000.0, 100_000.0).unwrap();
        assert!((cost - 3.5).abs() < 1e-9);
    }

    #[test]
    fn sql_expressions_cover_every_key() {
        let k = kind_sql();
        assert!(k.starts_with("transform(lower(multiIf("));
        assert!(k.contains("'execute_tool'") && k.contains("SpanAttributes['_ml_obs.meta.span.kind']"));
        let m = first_sql(MODEL_KEYS);
        assert_eq!(m.matches("!= ''").count(), MODEL_KEYS.len());
        assert!(is_llm_sql().contains("SpanAttributes['span.type'] = 'llm'"));
        assert!(input_tokens_sql().contains("JSONExtractFloat(SpanAttributes['_ml_obs.metrics'], 'input_tokens')"));
    }
}
