//! Processors: what a pipeline does to each log it matches, in order. Definitions are JSON in
//! Datadog's pipeline format (camelCase as integration pipelines write it; the snake_case names of
//! Datadog's API, such as `is_enabled` or `match_rules`, are accepted too).

use std::collections::HashMap;

use serde::Deserialize;
use serde_json::{Map, Value};

use super::dates::parse_any;
use super::filter::Filter;
use super::grok::{as_f64, num, query_string, Grok};
use super::log::{text, Log};
use super::{Pipeline, MAX_NESTING};

/// A compiled processor.
pub enum Step {
    Grok { source: String, grok: Grok },
    Status(Vec<String>),
    Date(Vec<String>),
    Service(Vec<String>),
    Message(Vec<String>),
    TraceId(Vec<String>),
    Remap(Remap),
    Category { target: String, categories: Vec<(Filter, String)> },
    Template { pieces: Vec<Piece>, target: String, replace_missing: bool },
    Lookup { source: String, target: String, table: HashMap<String, String>, default: Option<String> },
    Arithmetic { expr: Expr, target: String, replace_missing: bool },
    Url { sources: Vec<String>, target: String },
    Pipeline(Box<Pipeline>),
}

pub struct Remap {
    sources: Vec<String>,
    from_tag: bool,
    target: String,
    to_tag: bool,
    format: Format,
    preserve_source: bool,
    override_on_conflict: bool,
}

enum Format {
    Auto,
    Text,
    Integer,
    Double,
}

/// A string builder template: text and `%{attribute}` references.
pub enum Piece {
    Text(String),
    Attr(String),
}

/// An arithmetic processor's expression.
pub enum Expr {
    Num(f64),
    Attr(String),
    Neg(Box<Expr>),
    Bin(char, Box<Expr>, Box<Expr>),
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GrokDef {
    #[serde(default)]
    source: Option<String>,
    #[serde(default)]
    grok: GrokRules,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct GrokRules {
    #[serde(default, alias = "support_rules")]
    support_rules: Option<String>,
    #[serde(default, alias = "match_rules")]
    match_rules: Option<String>,
}

#[derive(Deserialize)]
struct SourcesDef {
    #[serde(default)]
    sources: Vec<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RemapDef {
    sources: Vec<String>,
    #[serde(default, alias = "source_type")]
    source_type: Option<String>,
    target: String,
    #[serde(default, alias = "target_type")]
    target_type: Option<String>,
    #[serde(default, alias = "target_format")]
    target_format: Option<String>,
    #[serde(default, alias = "preserve_source")]
    preserve_source: Option<bool>,
    #[serde(default, alias = "override_on_conflict")]
    override_on_conflict: Option<bool>,
}

#[derive(Deserialize)]
struct CategoryDef {
    target: String,
    #[serde(default)]
    categories: Vec<CategoryRule>,
}

#[derive(Deserialize)]
struct CategoryRule {
    #[serde(default)]
    filter: Value,
    name: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TemplateDef {
    template: String,
    target: String,
    #[serde(default, alias = "isReplaceMissing", alias = "is_replace_missing")]
    replace_missing: Option<bool>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct LookupDef {
    source: String,
    target: String,
    #[serde(default, alias = "lookup_table")]
    lookup_table: Vec<String>,
    #[serde(default, alias = "default_lookup")]
    default_lookup: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ArithmeticDef {
    expression: String,
    target: String,
    #[serde(default, alias = "isReplaceMissing", alias = "is_replace_missing")]
    replace_missing: Option<bool>,
}

#[derive(Deserialize)]
struct UrlDef {
    #[serde(default)]
    sources: Vec<String>,
    #[serde(default)]
    target: Option<String>,
}

#[derive(Deserialize)]
struct NestedDef {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    filter: Value,
    #[serde(default)]
    processors: Vec<Value>,
}

/// The query of a filter written as `{"query": "…"}` (Datadog's format) or as a plain string.
pub fn filter_query(v: &Value) -> &str {
    match v {
        Value::String(s) => s,
        Value::Object(o) => o.get("query").and_then(Value::as_str).unwrap_or(""),
        _ => "",
    }
}

fn de<T: serde::de::DeserializeOwned>(def: &Value) -> Result<T, String> {
    serde_json::from_value(def.clone()).map_err(|e| e.to_string())
}

/// Whether a processor definition is enabled (`enabled` or Datadog API's `is_enabled`; default yes).
pub fn enabled(def: &Value) -> bool {
    def.get("enabled").or_else(|| def.get("is_enabled")).and_then(Value::as_bool).unwrap_or(true)
}

/// The processor types that run; others are kept in the definition but skipped.
pub const SUPPORTED: &[&str] = &[
    "grok-parser",
    "status-remapper",
    "date-remapper",
    "service-remapper",
    "message-remapper",
    "trace-id-remapper",
    "attribute-remapper",
    "category-processor",
    "string-builder-processor",
    "lookup-processor",
    "arithmetic-processor",
    "url-parser",
    "pipeline",
];

/// Compiles one processor: `None` when it is disabled or of a type that doesn't run here.
/// `depth` is how deep in nested pipelines it sits.
pub fn compile(def: &Value, depth: usize) -> Result<Option<Step>, String> {
    let ty = def.get("type").and_then(Value::as_str).unwrap_or("");
    if !enabled(def) || !SUPPORTED.contains(&ty) {
        return Ok(None);
    }
    let sources = || de::<SourcesDef>(def).map(|d| d.sources);
    Ok(Some(match ty {
        "grok-parser" => {
            let d: GrokDef = de(def)?;
            let grok = Grok::compile(d.grok.support_rules.as_deref().unwrap_or(""), d.grok.match_rules.as_deref().unwrap_or(""))?;
            Step::Grok { source: d.source.filter(|s| !s.is_empty()).unwrap_or_else(|| "message".into()), grok }
        }
        "status-remapper" => Step::Status(sources()?),
        "date-remapper" => Step::Date(sources()?),
        "service-remapper" => Step::Service(sources()?),
        "message-remapper" => Step::Message(sources()?),
        "trace-id-remapper" => Step::TraceId(sources()?),
        "attribute-remapper" => {
            let d: RemapDef = de(def)?;
            let format = match d.target_format.as_deref().unwrap_or("auto") {
                "auto" | "" => Format::Auto,
                "string" => Format::Text,
                "integer" => Format::Integer,
                "double" => Format::Double,
                f => return Err(format!("Unknown target format '{f}' (auto, string, integer or double)")),
            };
            if d.target.trim().is_empty() {
                return Err("The remapper needs a target".into());
            }
            Step::Remap(Remap {
                sources: d.sources,
                from_tag: d.source_type.as_deref() == Some("tag"),
                target: d.target,
                to_tag: d.target_type.as_deref() == Some("tag"),
                format,
                preserve_source: d.preserve_source.unwrap_or(false),
                override_on_conflict: d.override_on_conflict.unwrap_or(false),
            })
        }
        "category-processor" => {
            let d: CategoryDef = de(def)?;
            Step::Category { target: d.target, categories: d.categories.into_iter().map(|c| (Filter::parse(filter_query(&c.filter)), c.name)).collect() }
        }
        "string-builder-processor" => {
            let d: TemplateDef = de(def)?;
            Step::Template { pieces: template(&d.template), target: d.target, replace_missing: d.replace_missing.unwrap_or(false) }
        }
        "lookup-processor" => {
            let d: LookupDef = de(def)?;
            let table = d.lookup_table.iter().filter_map(|line| lookup_row(line)).collect();
            Step::Lookup { source: d.source, target: d.target, table, default: d.default_lookup.filter(|s| !s.is_empty()) }
        }
        "arithmetic-processor" => {
            let d: ArithmeticDef = de(def)?;
            Step::Arithmetic { expr: parse_expr(&d.expression)?, target: d.target, replace_missing: d.replace_missing.unwrap_or(false) }
        }
        "url-parser" => {
            let d: UrlDef = de(def)?;
            let sources = if d.sources.is_empty() { vec!["http.url".to_string()] } else { d.sources };
            Step::Url { sources, target: d.target.filter(|t| !t.is_empty()).unwrap_or_else(|| "http.url_details".into()) }
        }
        "pipeline" => {
            if depth >= MAX_NESTING {
                return Err("Pipelines nest one level deep".into());
            }
            let d: NestedDef = de(def)?;
            let (p, errors) = Pipeline::compile_at(d.name.as_deref().unwrap_or(""), filter_query(&d.filter), &d.processors, depth + 1);
            if !errors.is_empty() {
                return Err(format!("in nested pipeline '{}': {}", p.name, errors.join("; ")));
            }
            Step::Pipeline(Box::new(p))
        }
        _ => return Ok(None),
    }))
}

pub fn apply(step: &Step, log: &mut Log, now_ms: i64) {
    let first = |log: &Log, sources: &[String]| sources.iter().find_map(|s| log.get(s));
    match step {
        Step::Grok { source, grok } => {
            let Some(Value::String(input)) = log.get(source) else { return };
            let Some(parsed) = grok.parse(&input, now_ms) else { return };
            for (path, v) in parsed.values {
                match (path.is_empty(), v) {
                    (true, Value::Object(o)) => o.into_iter().for_each(|(k, v)| log.set(&k, v)),
                    (true, _) => {}
                    (false, v) => log.set(&path, v),
                }
            }
        }
        Step::Status(sources) => {
            if let Some(v) = first(log, sources) {
                log.status = status_of(&v).to_string();
            }
        }
        Step::Date(sources) => {
            if let Some(ms) = sources.iter().find_map(|s| log.get(s).and_then(|v| parse_any(&v, now_ms))) {
                log.timestamp_ms = Some(ms);
            }
        }
        Step::Service(sources) => {
            if let Some(s) = sources.iter().filter_map(|s| log.get(s)).map(|v| text(&v)).find(|s| !s.is_empty()) {
                log.service = s;
            }
        }
        Step::Message(sources) => {
            if let Some(v) = first(log, sources) {
                log.message = text(&v);
            }
        }
        Step::TraceId(sources) => {
            if let Some(id) = first(log, sources).and_then(|v| trace_id(&text(&v))) {
                log.trace_id = id;
            }
        }
        Step::Remap(r) => r.apply(log),
        Step::Category { target, categories } => {
            if let Some((_, name)) = categories.iter().find(|(f, _)| log.matches(f)) {
                log.set(target, Value::from(name.as_str()));
            }
        }
        Step::Template { pieces, target, replace_missing } => {
            let mut out = String::new();
            for p in pieces {
                match p {
                    Piece::Text(t) => out.push_str(t),
                    Piece::Attr(a) => match log.get(a) {
                        Some(v) => out.push_str(&text(&v)),
                        None if *replace_missing => {}
                        None => return,
                    },
                }
            }
            log.set(target, Value::from(out));
        }
        Step::Lookup { source, target, table, default } => {
            let Some(v) = log.get(source) else { return };
            if let Some(found) = table.get(&text(&v)).or(default.as_ref()) {
                log.set(target, Value::from(found.as_str()));
            }
        }
        Step::Arithmetic { expr, target, replace_missing } => {
            if let Some(v) = eval(expr, log, *replace_missing).and_then(num) {
                log.set(target, v);
            }
        }
        Step::Url { sources, target } => {
            if let Some(details) = sources.iter().find_map(|s| log.get(s)).and_then(|v| v.as_str().and_then(url_details)) {
                log.set(target, details);
            }
        }
        Step::Pipeline(p) => {
            p.apply(log, now_ms);
        }
    }
}

impl Remap {
    fn apply(&self, log: &mut Log) {
        let read = |log: &Log, s: &str| if self.from_tag { log.tags.get(s).map(|v| Value::from(v.as_str())) } else { log.get(s) };
        let Some((source, v)) = self.sources.iter().find_map(|s| read(log, s).map(|v| (s, v))) else { return };
        let v = match self.format {
            Format::Auto => v,
            Format::Text => Value::from(text(&v)),
            Format::Integer => match as_f64(&v).filter(|f| f.is_finite()) {
                Some(f) => Value::from(f.trunc() as i64),
                None => return,
            },
            Format::Double => match as_f64(&v).and_then(serde_json::Number::from_f64) {
                Some(n) => Value::Number(n),
                None => return,
            },
        };
        let exists = if self.to_tag { log.tags.contains_key(&self.target) } else { log.get(&self.target).is_some() };
        if exists && !self.override_on_conflict && *source != self.target {
            return;
        }
        if !self.preserve_source && *source != self.target {
            if self.from_tag {
                log.tags.remove(source);
            } else {
                log.remove(source);
            }
        }
        if self.to_tag {
            log.tags.insert(self.target.clone(), text(&v));
        } else {
            log.set(&self.target, v);
        }
    }
}

/// Datadog's status from a remapped value, as one of ours (error, warn, info, debug): syslog
/// severities 0–7 and words by their first letters (`emerg`/`fatal`/`alert`/`crit`/`err` → error,
/// `warn` → warn, `notice`/`info`/`ok`/`success` → info, `debug`/`trace`/`verbose` → debug).
pub fn status_of(v: &Value) -> &'static str {
    let s = text(v).trim().to_ascii_lowercase();
    if let Ok(n) = s.parse::<f64>() {
        return match n as i64 {
            0..=3 => "error",
            4 => "warn",
            7 => "debug",
            _ => "info",
        };
    }
    let error = ["emerg", "f", "a", "c", "err", "panic"].iter().any(|p| s.starts_with(p)) || s == "e";
    if error {
        "error"
    } else if s.starts_with('w') {
        "warn"
    } else if s.starts_with('d') || s.starts_with("trace") || s.starts_with("verbose") {
        "debug"
    } else {
        "info"
    }
}

/// A trace id as the 32 hex digits `trace_id` holds: decimal (Datadog's 64-bit ids) or hex.
fn trace_id(s: &str) -> Option<String> {
    let s = s.trim().to_ascii_lowercase();
    if s.is_empty() {
        return None;
    }
    if s.bytes().all(|b| b.is_ascii_digit()) {
        return s.parse::<u128>().ok().map(|n| format!("{n:032x}"));
    }
    (s.len() <= 32 && s.bytes().all(|b| b.is_ascii_hexdigit())).then(|| format!("{s:0>32}"))
}

fn template(t: &str) -> Vec<Piece> {
    let mut out = Vec::new();
    let mut rest = t;
    while let Some(i) = rest.find("%{") {
        let Some(end) = rest[i..].find('}') else { break };
        if i > 0 {
            out.push(Piece::Text(rest[..i].to_string()));
        }
        out.push(Piece::Attr(rest[i + 2..i + end].trim().to_string()));
        rest = &rest[i + end + 1..];
    }
    if !rest.is_empty() {
        out.push(Piece::Text(rest.to_string()));
    }
    out
}

/// One `key,value` line of a lookup table (either may be double-quoted).
fn lookup_row(line: &str) -> Option<(String, String)> {
    let line = line.trim();
    let (key, value) = match line.strip_prefix('"') {
        Some(rest) => {
            let end = rest.find('"')?;
            (rest[..end].to_string(), rest[end + 1..].trim_start().strip_prefix(',')?)
        }
        None => {
            let (k, v) = line.split_once(',')?;
            (k.trim().to_string(), v)
        }
    };
    let value = value.trim();
    let value = value.strip_prefix('"').and_then(|v| v.strip_suffix('"')).unwrap_or(value);
    Some((key, value.to_string()))
}

/// `scheme`, `host`, `port`, `path` and `queryString` of a URL (only the last two for a relative one).
pub fn url_details(s: &str) -> Option<Value> {
    let mut out = Map::new();
    let parsed = match url::Url::parse(s) {
        Ok(u) if u.has_host() => {
            out.insert("scheme".into(), Value::from(u.scheme()));
            out.insert("host".into(), Value::from(u.host_str().unwrap_or("")));
            if let Some(port) = u.port_or_known_default() {
                out.insert("port".into(), Value::from(port));
            }
            u
        }
        _ if s.starts_with('/') => url::Url::parse(&format!("http://relative.invalid{s}")).ok()?,
        _ => return None,
    };
    out.insert("path".into(), Value::from(parsed.path()));
    if let Some(q) = parsed.query().filter(|q| !q.is_empty()) {
        out.insert("queryString".into(), query_string(q));
    }
    Some(Value::Object(out))
}

// ---------------------------------------------------------------- arithmetic

#[derive(Debug, PartialEq)]
enum Token {
    Num(f64),
    Ident(String),
    Op(char),
}

fn tokens(s: &str) -> Result<Vec<Token>, String> {
    let chars: Vec<char> = s.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            i += 1;
        } else if c.is_ascii_digit() || (c == '.' && chars.get(i + 1).is_some_and(char::is_ascii_digit)) {
            let start = i;
            while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.') {
                i += 1;
            }
            let n: String = chars[start..i].iter().collect();
            out.push(Token::Num(n.parse().map_err(|_| format!("Bad number '{n}'"))?));
        } else if c.is_alphabetic() || c == '_' || c == '@' {
            // Attribute names: letters, digits, `_`, `.`, `@`, and `\-` for a dash.
            let mut name = String::new();
            while i < chars.len() {
                match chars[i] {
                    '\\' if chars.get(i + 1) == Some(&'-') => {
                        name.push('-');
                        i += 2;
                    }
                    ch if ch.is_alphanumeric() || matches!(ch, '_' | '.' | '@') => {
                        name.push(ch);
                        i += 1;
                    }
                    _ => break,
                }
            }
            out.push(Token::Ident(name.trim_start_matches('@').to_string()));
        } else if "+-*/()".contains(c) {
            out.push(Token::Op(c));
            i += 1;
        } else {
            return Err(format!("Unexpected '{c}' in the expression"));
        }
    }
    Ok(out)
}

fn parse_expr(s: &str) -> Result<Expr, String> {
    let toks = tokens(s)?;
    let mut pos = 0;
    let e = sum(&toks, &mut pos)?;
    if pos != toks.len() {
        return Err(format!("Unexpected {:?} in the expression", toks[pos]));
    }
    Ok(e)
}

fn sum(t: &[Token], pos: &mut usize) -> Result<Expr, String> {
    let mut left = product(t, pos)?;
    while let Some(Token::Op(op @ ('+' | '-'))) = t.get(*pos) {
        *pos += 1;
        left = Expr::Bin(*op, Box::new(left), Box::new(product(t, pos)?));
    }
    Ok(left)
}

fn product(t: &[Token], pos: &mut usize) -> Result<Expr, String> {
    let mut left = factor(t, pos)?;
    while let Some(Token::Op(op @ ('*' | '/'))) = t.get(*pos) {
        *pos += 1;
        left = Expr::Bin(*op, Box::new(left), Box::new(factor(t, pos)?));
    }
    Ok(left)
}

fn factor(t: &[Token], pos: &mut usize) -> Result<Expr, String> {
    let tok = t.get(*pos).ok_or("The expression ends too early")?;
    *pos += 1;
    match tok {
        Token::Num(n) => Ok(Expr::Num(*n)),
        Token::Ident(a) => Ok(Expr::Attr(a.clone())),
        Token::Op('-') => Ok(Expr::Neg(Box::new(factor(t, pos)?))),
        Token::Op('(') => {
            let e = sum(t, pos)?;
            if t.get(*pos) != Some(&Token::Op(')')) {
                return Err("Missing ')' in the expression".into());
            }
            *pos += 1;
            Ok(e)
        }
        Token::Op(c) => Err(format!("Unexpected '{c}' in the expression")),
    }
}

/// The expression's value; `None` when an attribute is missing (unless replaced by 0) or on division by zero.
fn eval(e: &Expr, log: &Log, replace_missing: bool) -> Option<f64> {
    match e {
        Expr::Num(n) => Some(*n),
        Expr::Attr(a) => log.get(a).and_then(|v| as_f64(&v)).or(replace_missing.then_some(0.0)),
        Expr::Neg(x) => eval(x, log, replace_missing).map(|v| -v),
        Expr::Bin(op, l, r) => {
            let (l, r) = (eval(l, log, replace_missing)?, eval(r, log, replace_missing)?);
            match op {
                '+' => Some(l + r),
                '-' => Some(l - r),
                '*' => Some(l * r),
                _ => (r != 0.0).then(|| l / r),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::{Log, Pipeline};
    use super::*;
    use serde_json::json;

    const NOW: i64 = 1_792_933_200_000;

    fn log(message: &str, attrs: Value) -> Log {
        Log::from_row(&json!({
            "status": "info", "service": "storefront-api", "host": "i-0a1b2c3d4e5f60001", "source": "nginx",
            "message": message, "tags": {"env": "prod"}, "attributes": attrs.to_string(),
        }))
    }

    /// Runs the processors (an unfiltered pipeline) on the log.
    fn run(processors: Value, mut l: Log) -> Log {
        let (p, errors) = Pipeline::compile("test", "", processors.as_array().unwrap());
        assert!(errors.is_empty(), "{errors:?}");
        p.apply(&mut l, NOW);
        l
    }

    #[test]
    fn grok_parser_extracts_nested_attributes() {
        let grok = json!([{
            "type": "grok-parser", "name": "access", "source": "message",
            "grok": {"supportRules": "_method %{word:http.method}", "matchRules": "access %{ip:network.client.ip} %{_method} %{notSpace:http.url} %{integer:http.status_code}"}
        }]);
        let out = run(grok.clone(), log("10.0.0.12 GET /cart 404", json!({})));
        assert_eq!(out.attrs, json!({"network": {"client": {"ip": "10.0.0.12"}}, "http": {"method": "GET", "url": "/cart", "status_code": 404}}).as_object().unwrap().clone());
        // No rule matches: the log is unchanged.
        let untouched = log("not an access log", json!({"a": 1}));
        assert_eq!(run(grok, untouched.clone()), untouched);
    }

    #[test]
    fn grok_parser_reads_other_attributes_and_merges_keyvalue_pairs() {
        let p = json!([{"type": "grok-parser", "source": "request", "grok": {"matchRules": "kv %{data::keyvalue}"}}]);
        let out = run(p, log("m", json!({"request": "user=jane cart=7"})));
        assert_eq!(out.get("user"), Some(json!("jane")));
        assert_eq!(out.get("cart"), Some(json!("7")));
        // Snake_case (Datadog API) definitions and disabled processors.
        let p = json!([
            {"type": "grok-parser", "is_enabled": false, "grok": {"match_rules": "r %{data:skipped}"}},
            {"type": "grok-parser", "grok": {"match_rules": "r %{word:verb} %{data}"}}
        ]);
        let out = run(p, log("GET /cart", json!({})));
        assert_eq!((out.get("verb"), out.get("skipped")), (Some(json!("GET")), None));
    }

    #[test]
    fn status_values() {
        for (v, want) in [
            (json!("ERROR"), "error"),
            (json!("err"), "error"),
            (json!("E"), "error"),
            (json!("critical"), "error"),
            (json!("FATAL"), "error"),
            (json!("emergency"), "error"),
            (json!("alert"), "error"),
            (json!("panic"), "error"),
            (json!("warn"), "warn"),
            (json!("WARNING"), "warn"),
            (json!("notice"), "info"),
            (json!("info"), "info"),
            (json!("OK"), "info"),
            (json!("success"), "info"),
            (json!("debug"), "debug"),
            (json!("trace"), "debug"),
            (json!("verbose"), "debug"),
            (json!(0), "error"),
            (json!(3), "error"),
            (json!("4"), "warn"),
            (json!(5), "info"),
            (json!(6), "info"),
            (json!(7), "debug"),
            (json!("something else"), "info"),
        ] {
            assert_eq!(status_of(&v), want, "{v}");
        }
    }

    #[test]
    fn reserved_attribute_remappers() {
        let p = json!([
            {"type": "status-remapper", "sources": ["missing", "level"]},
            {"type": "date-remapper", "sources": ["ts"]},
            {"type": "service-remapper", "sources": ["app"]},
            {"type": "message-remapper", "sources": ["msg"]},
            {"type": "trace-id-remapper", "sources": ["dd.trace_id"]}
        ]);
        let out = run(p, log("raw line", json!({"level": "WARN", "ts": "2026-10-25T13:00:00.123Z", "app": "billing", "msg": "card declined", "dd": {"trace_id": "255"}})));
        assert_eq!((out.status.as_str(), out.service.as_str(), out.message.as_str()), ("warn", "billing", "card declined"));
        assert_eq!(out.timestamp_ms, Some(1_792_933_200_123));
        assert_eq!(out.trace_id, "000000000000000000000000000000ff");
        // Sources stay; a date that doesn't parse leaves the date alone.
        assert_eq!(out.get("level"), Some(json!("WARN")));
        let out = run(json!([{"type": "date-remapper", "sources": ["ts"]}]), log("m", json!({"ts": "soon"})));
        assert_eq!(out.timestamp_ms, None);
    }

    #[test]
    fn attribute_remapper() {
        let base = || log("m", json!({"client": "10.0.0.5", "network": {"client": {"ip": "10.0.0.9"}}, "code": "404"}));
        let remap = |extra: Value| {
            let mut def = json!({"type": "attribute-remapper", "sources": ["client"], "target": "network.client.ip"});
            def.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
            json!([def])
        };
        // An existing target is kept unless overrideOnConflict (and then the source too).
        let out = run(remap(json!({})), base());
        assert_eq!((out.get("network.client.ip"), out.get("client")), (Some(json!("10.0.0.9")), Some(json!("10.0.0.5"))));
        let mut fresh = base();
        fresh.remove("network");
        let out = run(remap(json!({})), fresh);
        assert_eq!((out.get("network.client.ip"), out.get("client")), (Some(json!("10.0.0.5")), None));
        let out = run(remap(json!({"overrideOnConflict": true, "preserveSource": true})), base());
        assert_eq!((out.get("network.client.ip"), out.get("client")), (Some(json!("10.0.0.5")), Some(json!("10.0.0.5"))));
        // Types, and tags as sources or targets.
        let p = json!([
            {"type": "attribute-remapper", "sources": ["code"], "target": "http.status_code", "targetFormat": "integer"},
            {"type": "attribute-remapper", "sources": ["env"], "sourceType": "tag", "target": "deployment.env", "preserveSource": true},
            {"type": "attribute-remapper", "sources": ["network.client.ip"], "target": "client_ip", "targetType": "tag", "preserveSource": true}
        ]);
        let out = run(p, base());
        assert_eq!(out.get("http.status_code"), Some(json!(404)));
        assert_eq!(out.get("deployment.env"), Some(json!("prod")));
        assert_eq!(out.tags.get("client_ip").map(String::as_str), Some("10.0.0.9"));
        assert_eq!(out.tags.get("env").map(String::as_str), Some("prod"));
    }

    #[test]
    fn category_processor_first_match_wins() {
        let p = json!([{"type": "category-processor", "target": "http.status_category", "categories": [
            {"filter": {"query": "@http.status_code:[200 TO 299]"}, "name": "OK"},
            {"filter": {"query": "@http.status_code:[400 TO 499]"}, "name": "warning"},
            {"filter": {"query": "@http.status_code:[400 TO 599]"}, "name": "error"}
        ]}, {"type": "status-remapper", "sources": ["http.status_category"]}]);
        let out = run(p.clone(), log("m", json!({"http": {"status_code": 404}})));
        assert_eq!((out.get("http.status_category"), out.status.as_str()), (Some(json!("warning")), "warn"));
        let out = run(p, log("m", json!({"http": {"status_code": 302}})));
        assert_eq!(out.get("http.status_category"), None);
    }

    #[test]
    fn string_builder_lookup_and_arithmetic() {
        let p = json!([
            {"type": "string-builder-processor", "template": "%{http.method} %{http.url} by %{user}", "target": "summary"},
            {"type": "string-builder-processor", "template": "%{http.method} by %{user}", "target": "partial", "replaceMissing": true},
            {"type": "lookup-processor", "source": "country", "target": "country_name", "lookupTable": ["FR,France", "\"US\", United States"], "defaultLookup": "Unknown"},
            {"type": "lookup-processor", "source": "region", "target": "region_name", "lookupTable": ["eu,Europe"]},
            {"type": "arithmetic-processor", "expression": "(duration - queue) * 1000 / 2", "target": "work_us"},
            {"type": "arithmetic-processor", "expression": "duration + missing", "target": "skipped"},
            {"type": "arithmetic-processor", "expression": "duration + missing", "target": "zeroed", "isReplaceMissing": true}
        ]);
        let out = run(p, log("m", json!({"http": {"method": "GET", "url": "/cart"}, "country": "US", "region": "us", "duration": "1.5", "queue": 0.5})));
        assert_eq!(out.get("summary"), None, "a missing attribute skips the template");
        assert_eq!(out.get("partial"), Some(json!("GET by ")));
        assert_eq!(out.get("country_name"), Some(json!("United States")));
        assert_eq!(out.get("region_name"), None);
        assert_eq!(out.get("work_us"), Some(json!(500)));
        assert_eq!(out.get("skipped"), None);
        assert_eq!(out.get("zeroed"), Some(json!(1.5)));
    }

    #[test]
    fn url_parser() {
        let p = json!([{"type": "url-parser", "sources": ["http.url"], "target": "http.url_details"}]);
        let out = run(p.clone(), log("m", json!({"http": {"url": "https://shop.example.com:8443/cart?id=7&q=red%20shoes"}})));
        assert_eq!(out.get("http.url_details"), Some(json!({"scheme": "https", "host": "shop.example.com", "port": 8443, "path": "/cart", "queryString": {"id": "7", "q": "red shoes"}})));
        let out = run(p, log("m", json!({"http": {"url": "/cart"}})));
        assert_eq!(out.get("http.url_details"), Some(json!({"path": "/cart"})));
    }

    #[test]
    fn nested_pipelines_have_their_own_filter() {
        let p = json!([{"type": "pipeline", "name": "errors only", "filter": {"query": "@level:error"}, "processors": [
            {"type": "string-builder-processor", "template": "yes", "target": "flagged"}
        ]}]);
        assert_eq!(run(p.clone(), log("m", json!({"level": "error"}))).get("flagged"), Some(json!("yes")));
        assert_eq!(run(p, log("m", json!({"level": "info"}))).get("flagged"), None);
    }

    #[test]
    fn invalid_processors_are_reported_and_unknown_types_skipped() {
        let defs = json!([
            {"type": "grok-parser", "name": "broken", "grok": {"matchRules": "r %{nope:x}"}},
            {"type": "arithmetic-processor", "name": "math", "expression": "a +", "target": "b"},
            {"type": "user-agent-parser", "sources": ["http.useragent"]},
            {"type": "string-builder-processor", "template": "kept", "target": "t"}
        ]);
        let (p, errors) = Pipeline::compile("p", "", defs.as_array().unwrap());
        assert_eq!(errors.len(), 2, "{errors:?}");
        assert!(errors[0].contains("broken") && errors[0].contains("nope"));
        assert!(errors[1].contains("math"));
        let mut l = log("m", json!({}));
        p.apply(&mut l, NOW);
        assert_eq!(l.get("t"), Some(json!("kept")));
    }
}
