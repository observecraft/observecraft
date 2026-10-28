//! Self-service actions: their form inputs, step templates and how a run fills them.
//!
//! Templates name values with `{{inputs.<key>}}`, `{{user.email}}`, `{{user.name}}`,
//! `{{secrets.<name>}}` and, in HTTP requests after the first, `{{steps.<n>.<json.path>}}` (a value of
//! request n's JSON response). Two filters exist: `{{inputs.x | base64}}` and `{{inputs.x | path}}`
//! (a URL path whose `/` are kept). Every value is escaped for where it lands:
//!
//! - **URLs**: percent-encoded (a value never adds a path segment, query parameter or host); the
//!   scheme and host are literal in the template, so inputs can't choose where a request goes.
//! - **JSON bodies**: the template is JSON and placeholders sit inside its strings, so a value can
//!   never change the document's structure; a string that is just one placeholder takes the value's
//!   type (`"private": "{{inputs.private}}"` sends a boolean).
//! - **Headers**: values with line breaks are refused.
//!
//! Secrets are only used by HTTP requests and are redacted from what a run keeps.

use std::collections::{BTreeMap, HashSet};

use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use utoipa::ToSchema;

pub const MAX_INPUTS: usize = 30;
pub const MAX_REQUESTS: usize = 5;
pub const MAX_HEADERS: usize = 20;
pub const MAX_TEMPLATE_CHARS: usize = 20_000;
pub const MAX_TEXT_INPUT_CHARS: usize = 5000;
pub const REDACTED: &str = "[REDACTED]";
const METHODS: &[&str] = &["GET", "POST", "PUT", "PATCH", "DELETE"];

#[derive(Serialize, Deserialize, ToSchema, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum ActionInputKind {
    Text,
    /// One of `options`.
    Select,
    Number,
    Boolean,
}

/// A field of the action's form.
#[derive(Serialize, Deserialize, ToSchema, Clone, Debug, PartialEq)]
pub struct ActionInput {
    /// Name used in templates (`{{inputs.<key>}}`): `[a-z0-9_]`.
    pub key: String,
    pub label: String,
    pub kind: ActionInputKind,
    #[serde(default)]
    pub required: bool,
    /// Select only.
    #[serde(default)]
    pub options: Vec<String>,
    /// Pre-filled value (`true` / `false` for booleans).
    #[serde(default)]
    pub default: String,
    #[serde(default)]
    pub description: String,
}

#[derive(Serialize, Deserialize, ToSchema, Clone, Debug, PartialEq)]
pub struct ActionHeader {
    pub name: String,
    /// Template.
    pub value: String,
}

/// One outbound request; its response must be 2xx for the run to go on.
#[derive(Serialize, Deserialize, ToSchema, Clone, Debug, PartialEq)]
pub struct ActionHttpRequest {
    /// GET, POST, PUT, PATCH or DELETE.
    pub method: String,
    /// Template; scheme and host are literal.
    pub url: String,
    #[serde(default)]
    pub headers: Vec<ActionHeader>,
    /// JSON template sent as `application/json`; empty for no body.
    #[serde(default)]
    pub body: String,
}

#[derive(Serialize, Deserialize, ToSchema, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum ActionStepKind {
    /// HTTP requests sent in order (1–5).
    Http,
    /// Declares an incident: fields `title`, `severity`, `summary`, `teams`, `services`.
    CreateIncident,
    /// Pages an on-call team: fields `team`, `title`, `description`, `urgency`.
    PageOnCall,
    /// Adds a note to an incident's timeline: fields `incident` (its number), `body`.
    IncidentComment,
    /// Runs a synthetic test now: field `test` (its id).
    SyntheticRun,
    /// Starts a published workflow with an app trigger: field `workflow` (its id); the action's
    /// inputs fill the workflow inputs of the same keys.
    RunWorkflow,
}

impl ActionStepKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ActionStepKind::Http => "http",
            ActionStepKind::CreateIncident => "create_incident",
            ActionStepKind::PageOnCall => "page_on_call",
            ActionStepKind::IncidentComment => "incident_comment",
            ActionStepKind::SyntheticRun => "synthetic_run",
            ActionStepKind::RunWorkflow => "run_workflow",
        }
    }

    /// The fields of an internal step, and whether each is required.
    pub fn fields(self) -> &'static [(&'static str, bool)] {
        match self {
            ActionStepKind::Http => &[],
            ActionStepKind::CreateIncident => &[("title", true), ("severity", false), ("summary", false), ("teams", false), ("services", false)],
            ActionStepKind::PageOnCall => &[("team", true), ("title", true), ("description", false), ("urgency", false)],
            ActionStepKind::IncidentComment => &[("incident", true), ("body", true)],
            ActionStepKind::SyntheticRun => &[("test", true)],
            ActionStepKind::RunWorkflow => &[("workflow", true)],
        }
    }
}

#[derive(Serialize, Deserialize, ToSchema, Clone, Debug, PartialEq)]
pub struct ActionStep {
    pub kind: ActionStepKind,
    /// Http only.
    #[serde(default)]
    pub requests: Vec<ActionHttpRequest>,
    /// Internal steps: a template per field.
    #[serde(default)]
    pub fields: BTreeMap<String, String>,
}

/// What templates can read during a run.
pub struct RenderContext<'a> {
    pub inputs: &'a Map<String, Value>,
    pub user_email: &'a str,
    pub user_name: &'a str,
    pub secrets: &'a BTreeMap<String, String>,
    /// JSON responses of the requests sent so far.
    pub steps: Vec<Value>,
    /// Workflows: the run's data under the names of [`TemplateScope::roots`] (`Trigger`, `Steps`, …).
    pub vars: Option<&'a Value>,
}

/// Where a template is used, for validation: which names it may refer to.
pub struct TemplateScope<'a> {
    pub inputs: &'a [ActionInput],
    pub secrets: &'a HashSet<String>,
    /// Secrets are allowed (HTTP requests only).
    pub allow_secrets: bool,
    /// Requests answered before this one (`steps.1` … `steps.<n>`).
    pub prior_steps: usize,
    /// Workflows: further names a placeholder may start with (`Trigger`, `Steps`, …).
    pub roots: &'a [&'a str],
}

/// The first segment of a path, when it is one of the workflow names (not `inputs` or `user`, which
/// self-service actions have too).
fn workflow_root<'r>(path: &str, roots: &[&'r str]) -> Option<&'r str> {
    let first = path.split('.').next()?;
    roots.iter().copied().find(|r| *r == first && !matches!(*r, "inputs" | "user"))
}

// ================================================================ templates

/// A `{{ path | filter }}` placeholder.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Placeholder<'a> {
    path: &'a str,
    filter: Option<&'a str>,
}

#[derive(Debug, PartialEq)]
enum Piece<'a> {
    Literal(&'a str),
    Value(Placeholder<'a>),
}

fn pieces(template: &str) -> Result<Vec<Piece<'_>>, String> {
    let mut out = Vec::new();
    let mut rest = template;
    while let Some(start) = rest.find("{{") {
        if start > 0 {
            out.push(Piece::Literal(&rest[..start]));
        }
        let after = &rest[start + 2..];
        let end = after.find("}}").ok_or("A placeholder is missing its closing }}")?;
        let inner = &after[..end];
        let mut parts = inner.split('|');
        let path = parts.next().unwrap_or("").trim();
        let filter = parts.next().map(str::trim);
        if parts.next().is_some() {
            return Err(format!("{{{{{}}}}} has more than one filter", inner.trim()));
        }
        out.push(Piece::Value(Placeholder { path, filter }));
        rest = &after[end + 2..];
    }
    if !rest.is_empty() {
        out.push(Piece::Literal(rest));
    }
    Ok(out)
}

/// `steps.<n>.<rest>` → (n, rest).
fn step_ref(path: &str) -> Option<(usize, &str)> {
    let rest = path.strip_prefix("steps.")?;
    let (n, field) = rest.split_once('.').unwrap_or((rest, ""));
    Some((n.parse().ok()?, field))
}

/// Checks every placeholder of a template against what it may refer to.
pub fn check_template(template: &str, scope: &TemplateScope) -> Result<(), String> {
    if template.chars().count() > MAX_TEMPLATE_CHARS {
        return Err(format!("A template is longer than {MAX_TEMPLATE_CHARS} characters"));
    }
    for piece in pieces(template)? {
        let Piece::Value(p) = piece else { continue };
        if let Some(f) = p.filter.filter(|f| !matches!(*f, "base64" | "path")) {
            return Err(format!("Unknown filter {f} in {{{{{}}}}} (use base64 or path)", p.path));
        }
        let path = p.path;
        let known = if let Some(key) = path.strip_prefix("inputs.") {
            scope.inputs.iter().any(|i| i.key == key)
        } else if let Some(name) = path.strip_prefix("secrets.") {
            if !scope.allow_secrets {
                return Err(format!("{{{{{path}}}}}: secrets can only be used in HTTP requests"));
            }
            if !scope.secrets.contains(name) {
                return Err(format!("There is no secret named {name} ({{{{{path}}}}})"));
            }
            true
        } else if workflow_root(path, scope.roots).is_some() {
            true
        } else if let Some((n, field)) = step_ref(path) {
            if n == 0 || n > scope.prior_steps || field.is_empty() {
                return Err(format!("{{{{{path}}}}}: steps.{n} isn't available here; a request can read the JSON responses of the requests before it (steps.1.<field>)"));
            }
            true
        } else {
            matches!(path, "user.email" | "user.name")
        };
        if !known {
            return Err(format!("Unknown value {{{{{path}}}}} (inputs.<key>, user.email, user.name, secrets.<name> or steps.<n>.<field>)"));
        }
    }
    Ok(())
}

/// A value as text: strings as they are, numbers and booleans written out, null empty.
fn as_text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        Value::Bool(_) | Value::Number(_) => v.to_string(),
        other => other.to_string(),
    }
}

fn lookup(ctx: &RenderContext, p: &Placeholder) -> Result<Value, String> {
    let path = p.path;
    let value = if let Some(key) = path.strip_prefix("inputs.") {
        ctx.inputs.get(key).cloned().unwrap_or(Value::Null)
    } else if let Some(name) = path.strip_prefix("secrets.") {
        Value::String(ctx.secrets.get(name).cloned().ok_or_else(|| format!("There is no secret named {name}"))?)
    } else if let Some(vars) = ctx.vars.filter(|_| !path.starts_with("steps.") && !path.starts_with("user.")) {
        super::workflow_expr::lookup(vars, &super::workflow_expr::segments(path)).cloned().unwrap_or(Value::Null)
    } else if let Some((n, field)) = step_ref(path) {
        let missing = || format!("{path} is missing from the response of request {n}");
        let mut cur = ctx.steps.get(n.wrapping_sub(1)).ok_or_else(missing)?;
        for seg in field.split('.') {
            cur = match cur {
                Value::Object(m) => m.get(seg),
                Value::Array(a) => seg.parse::<usize>().ok().and_then(|i| a.get(i)),
                _ => None,
            }
            .ok_or_else(missing)?;
        }
        cur.clone()
    } else {
        match path {
            "user.email" => Value::String(ctx.user_email.to_string()),
            "user.name" => Value::String(ctx.user_name.to_string()),
            _ => return Err(format!("Unknown value {{{{{path}}}}}")),
        }
    };
    Ok(match p.filter {
        Some("base64") => Value::String(STANDARD.encode(as_text(&value))),
        _ => value,
    })
}

/// Percent-encodes everything but RFC 3986 unreserved characters.
pub fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// Plain text (headers, internal step fields): values as text; line breaks refused when `single_line`.
pub fn render_text(template: &str, ctx: &RenderContext, single_line: bool) -> Result<String, String> {
    let mut out = String::new();
    for piece in pieces(template)? {
        match piece {
            Piece::Literal(s) => out.push_str(s),
            Piece::Value(p) => {
                let text = as_text(&lookup(ctx, &p)?);
                if single_line && text.contains(['\r', '\n']) {
                    return Err(format!("{{{{{}}}}} contains a line break, which a header can't carry", p.path));
                }
                out.push_str(&text);
            }
        }
    }
    Ok(out)
}

/// A URL: every value percent-encoded.
pub fn render_url(template: &str, ctx: &RenderContext) -> Result<String, String> {
    let mut out = String::new();
    for piece in pieces(template)? {
        match piece {
            Piece::Literal(s) => out.push_str(s),
            Piece::Value(p) => {
                let text = as_text(&lookup(ctx, &p)?);
                if p.filter == Some("path") {
                    let mut segments = Vec::new();
                    for seg in text.split('/') {
                        if seg == "." || seg == ".." {
                            return Err(format!("{{{{{}}}}} can't contain the path segment “{seg}”", p.path));
                        }
                        segments.push(percent_encode(seg));
                    }
                    out.push_str(&segments.join("/"));
                } else {
                    if text == "." || text == ".." {
                        return Err(format!("{{{{{}}}}} can't be “{text}” in a URL", p.path));
                    }
                    out.push_str(&percent_encode(&text));
                }
            }
        }
    }
    Ok(out)
}

fn render_value(v: &Value, ctx: &RenderContext) -> Result<Value, String> {
    Ok(match v {
        Value::String(s) => {
            let parts = pieces(s)?;
            match parts.as_slice() {
                // A string that is only a placeholder takes the value's type.
                [Piece::Value(p)] => match lookup(ctx, p)? {
                    Value::Null => Value::String(String::new()),
                    other => other,
                },
                _ => Value::String(render_text(s, ctx, false)?),
            }
        }
        Value::Array(a) => Value::Array(a.iter().map(|x| render_value(x, ctx)).collect::<Result<_, _>>()?),
        Value::Object(m) => Value::Object(m.iter().map(|(k, x)| Ok((k.clone(), render_value(x, ctx)?))).collect::<Result<_, String>>()?),
        other => other.clone(),
    })
}

/// A JSON body: placeholders filled inside the template's strings.
pub fn render_json(template: &str, ctx: &RenderContext) -> Result<String, String> {
    if template.trim().is_empty() {
        return Ok(String::new());
    }
    let doc: Value = serde_json::from_str(template).map_err(|e| format!("The body template isn't JSON: {e}"))?;
    serde_json::to_string(&render_value(&doc, ctx)?).map_err(|e| e.to_string())
}

// ================================================================ validation

fn trimmed(value: &str, what: &str, max: usize, required: bool) -> Result<String, String> {
    let v = value.trim();
    if required && v.is_empty() {
        return Err(format!("{what} is required"));
    }
    if v.chars().count() > max {
        return Err(format!("{what} is longer than {max} characters"));
    }
    Ok(v.to_string())
}

fn header_name_ok(name: &str) -> bool {
    !name.is_empty() && name.len() <= 100 && name.bytes().all(|b| b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b))
}

/// Checks an HTTP request's templates (URL with literal scheme and host, header names, JSON body).
pub fn check_request(r: &ActionHttpRequest, scope: &TemplateScope) -> Result<ActionHttpRequest, String> {
    let method = r.method.trim().to_ascii_uppercase();
    if !METHODS.contains(&method.as_str()) {
        return Err(format!("Unknown method {method}; use GET, POST, PUT, PATCH or DELETE"));
    }
    let url = trimmed(&r.url, "The URL", 2048, true)?;
    let lower = url.to_ascii_lowercase();
    let after_scheme = lower.strip_prefix("https://").or_else(|| lower.strip_prefix("http://")).ok_or("The URL must start with http:// or https://")?;
    let authority = &after_scheme[..after_scheme.find(['/', '?', '#']).unwrap_or(after_scheme.len())];
    if authority.is_empty() || authority.contains("{{") {
        return Err("Write the URL's host out: it can't come from a placeholder".into());
    }
    check_template(&url, scope)?;
    let probe: String = pieces(&url)?.iter().map(|p| if let Piece::Literal(s) = p { s } else { "x" }).collect();
    url::Url::parse(&probe).map_err(|e| format!("Invalid URL {url}: {e}"))?;
    if r.headers.len() > MAX_HEADERS {
        return Err(format!("A request has at most {MAX_HEADERS} headers"));
    }
    let mut headers = Vec::with_capacity(r.headers.len());
    for h in &r.headers {
        let name = h.name.trim();
        if !header_name_ok(name) || name.eq_ignore_ascii_case("host") || name.eq_ignore_ascii_case("content-length") {
            return Err(format!("{name} isn't a header name a request can set"));
        }
        if h.value.contains(['\r', '\n']) {
            return Err(format!("The {name} header has a line break"));
        }
        check_template(&h.value, scope)?;
        headers.push(ActionHeader { name: name.to_string(), value: h.value.trim().to_string() });
    }
    let body = r.body.trim().to_string();
    if !body.is_empty() {
        check_template(&body, scope)?;
        serde_json::from_str::<Value>(&body).map_err(|_| "The body must be JSON, with placeholders inside its strings (\"name\": \"{{inputs.name}}\")".to_string())?;
    }
    Ok(ActionHttpRequest { method, url, headers, body })
}

fn number(s: &str) -> Option<Value> {
    let f: f64 = s.trim().parse().ok().filter(|f: &f64| f.is_finite())?;
    Some(if f.fract() == 0.0 && f.abs() < 1e15 { Value::from(f as i64) } else { Value::from(f) })
}

/// Checks the form fields.
pub fn check_inputs(inputs: &[ActionInput]) -> Result<Vec<ActionInput>, String> {
    if inputs.len() > MAX_INPUTS {
        return Err(format!("An action has at most {MAX_INPUTS} inputs"));
    }
    let mut keys = HashSet::new();
    let mut out = Vec::with_capacity(inputs.len());
    for i in inputs {
        let key = i.key.trim();
        if key.is_empty() || key.len() > 40 || !key.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_') {
            return Err(format!("{key} isn't an input key (a–z, 0–9 and _, up to 40)"));
        }
        if !keys.insert(key.to_string()) {
            return Err(format!("The input key {key} is used twice"));
        }
        let label = trimmed(&i.label, &format!("The label of {key}"), 200, false)?;
        let label = if label.is_empty() { key.to_string() } else { label };
        let description = trimmed(&i.description, &format!("The description of {key}"), 1000, false)?;
        let default = trimmed(&i.default, &format!("The default of {key}"), MAX_TEXT_INPUT_CHARS, false)?;
        let mut options = Vec::new();
        match i.kind {
            ActionInputKind::Select => {
                options = i.options.iter().map(|o| o.trim().to_string()).collect();
                let unique: HashSet<&String> = options.iter().collect();
                if options.is_empty() || options.len() > 50 || unique.len() != options.len() || options.iter().any(|o| o.is_empty() || o.chars().count() > 200) {
                    return Err(format!("The select input {key} needs 1–50 distinct options"));
                }
                if !default.is_empty() && !options.contains(&default) {
                    return Err(format!("The default {default} isn't an option of {key}"));
                }
            }
            ActionInputKind::Boolean if !matches!(default.as_str(), "" | "true" | "false") => return Err(format!("The default of {key} is true or false")),
            ActionInputKind::Number if !default.is_empty() && number(&default).is_none() => return Err(format!("The default of {key} must be a number")),
            _ => {}
        }
        out.push(ActionInput { key: key.to_string(), label, kind: i.kind, required: i.required, options, default, description });
    }
    Ok(out)
}

/// Checks a step against the inputs and secret names.
pub fn check_step(step: &ActionStep, inputs: &[ActionInput], secrets: &HashSet<String>) -> Result<ActionStep, String> {
    check_step_in(step, inputs, secrets, &[])
}

/// [`check_step`] where templates may also start with `roots` (a workflow's `Trigger`, `Steps`, …).
pub fn check_step_in(step: &ActionStep, inputs: &[ActionInput], secrets: &HashSet<String>, roots: &[&str]) -> Result<ActionStep, String> {
    if step.kind == ActionStepKind::Http {
        if step.requests.is_empty() || step.requests.len() > MAX_REQUESTS {
            return Err(format!("An HTTP step sends 1–{MAX_REQUESTS} requests"));
        }
        let requests = step
            .requests
            .iter()
            .enumerate()
            .map(|(i, r)| {
                check_request(r, &TemplateScope { inputs, secrets, allow_secrets: true, prior_steps: i, roots }).map_err(|e| if step.requests.len() > 1 { format!("Request {}: {e}", i + 1) } else { e })
            })
            .collect::<Result<_, _>>()?;
        return Ok(ActionStep { kind: step.kind, requests, fields: BTreeMap::new() });
    }
    let allowed = step.kind.fields();
    let scope = TemplateScope { inputs, secrets, allow_secrets: false, prior_steps: 0, roots };
    if let Some(unknown) = step.fields.keys().find(|k| !allowed.iter().any(|(f, _)| f == k)) {
        return Err(format!("This step has no field {unknown} (fields: {})", allowed.iter().map(|(f, _)| *f).collect::<Vec<_>>().join(", ")));
    }
    let mut fields = BTreeMap::new();
    for (name, required) in allowed {
        let value = trimmed(step.fields.get(*name).map(String::as_str).unwrap_or(""), &format!("The {name} field"), MAX_TEMPLATE_CHARS, *required)?;
        check_template(&value, &scope)?;
        if !value.is_empty() {
            fields.insert(name.to_string(), value);
        }
    }
    Ok(ActionStep { kind: step.kind, requests: Vec::new(), fields })
}

/// The values a run uses: given ones checked against the form, defaults for the others.
pub fn check_run_inputs(inputs: &[ActionInput], given: &Map<String, Value>) -> Result<Map<String, Value>, String> {
    if let Some(unknown) = given.keys().find(|k| !inputs.iter().any(|i| &i.key == *k)) {
        return Err(format!("Unknown input {unknown}"));
    }
    let mut out = Map::new();
    for def in inputs {
        let label = &def.label;
        let value = given.get(&def.key).filter(|v| !v.is_null() && v.as_str().is_none_or(|s| !s.trim().is_empty()));
        let value = match value {
            Some(v) => v.clone(),
            None if !def.default.is_empty() => Value::String(def.default.clone()),
            None if def.required => return Err(format!("{label} is required")),
            None => continue,
        };
        let checked = match def.kind {
            ActionInputKind::Text => {
                let s = value.as_str().map(str::trim).ok_or_else(|| format!("{label} must be text"))?;
                if s.chars().count() > MAX_TEXT_INPUT_CHARS {
                    return Err(format!("{label} is longer than {MAX_TEXT_INPUT_CHARS} characters"));
                }
                Value::String(s.to_string())
            }
            ActionInputKind::Select => {
                let s = value.as_str().map(str::trim).unwrap_or_default();
                if !def.options.iter().any(|o| o == s) {
                    return Err(format!("{s} isn't an option of {label}"));
                }
                Value::String(s.to_string())
            }
            ActionInputKind::Number => match &value {
                Value::Number(_) => value.clone(),
                Value::String(s) => number(s).ok_or_else(|| format!("{label} must be a number"))?,
                _ => return Err(format!("{label} must be a number")),
            },
            ActionInputKind::Boolean => match &value {
                Value::Bool(_) => value.clone(),
                Value::String(s) if s.trim() == "true" => Value::Bool(true),
                Value::String(s) if s.trim() == "false" => Value::Bool(false),
                _ => return Err(format!("{label} is true or false")),
            },
        };
        out.insert(def.key.clone(), checked);
    }
    Ok(out)
}

/// `text` with every secret value (raw, percent-encoded or base64) replaced by [`REDACTED`].
pub fn redact(text: &str, secrets: &BTreeMap<String, String>) -> String {
    let mut forms: Vec<String> = secrets.values().filter(|s| !s.is_empty()).flat_map(|s| [s.clone(), percent_encode(s), STANDARD.encode(s)]).collect();
    forms.sort_by_key(|f| std::cmp::Reverse(f.len()));
    forms.dedup();
    let mut out = text.to_string();
    for f in forms {
        out = out.replace(&f, REDACTED);
    }
    out
}

fn web_url(v: Option<&Value>) -> Option<String> {
    v.and_then(Value::as_str).filter(|s| s.starts_with("https://") || s.starts_with("http://")).map(str::to_string)
}

/// Link to what a response created: `html_url` (GitHub), `web_url` (GitLab) or a Jira issue's page.
pub fn result_link(response: &Value) -> Option<String> {
    let obj = response.as_object()?;
    if let Some(u) = web_url(obj.get("html_url")).or_else(|| web_url(obj.get("web_url"))) {
        return Some(u);
    }
    let key = obj.get("key").and_then(Value::as_str)?;
    let base = web_url(obj.get("self"))?;
    let (site, _) = base.split_once("/rest/api/")?;
    Some(format!("{site}/browse/{}", percent_encode(key)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn input(key: &str, kind: ActionInputKind) -> ActionInput {
        ActionInput { key: key.into(), label: key.into(), kind, required: false, options: vec![], default: String::new(), description: String::new() }
    }

    fn ctx<'a>(inputs: &'a Map<String, Value>, secrets: &'a BTreeMap<String, String>) -> RenderContext<'a> {
        RenderContext { inputs, user_email: "dana@example.com", user_name: "Dana Lee", secrets, steps: vec![], vars: None }
    }

    fn map(v: Value) -> Map<String, Value> {
        v.as_object().unwrap().clone()
    }

    fn secrets() -> BTreeMap<String, String> {
        BTreeMap::from([("github_token".to_string(), "ghp_s3cr3t/t0ken".to_string())])
    }

    #[test]
    fn urls_percent_encode_every_value() {
        let i = map(json!({"owner": "example-org", "repo": "web shop?x=1#frag", "path": "envs/prod/main.tf", "n": 3}));
        let s = BTreeMap::new();
        let c = ctx(&i, &s);
        assert_eq!(render_url("https://api.github.com/repos/{{inputs.owner}}/{{ inputs.repo }}/generate", &c).unwrap(), "https://api.github.com/repos/example-org/web%20shop%3Fx%3D1%23frag/generate");
        // A value can't add path segments, but the path filter keeps its slashes.
        assert_eq!(render_url("https://h.example.com/c/{{inputs.path}}", &c).unwrap(), "https://h.example.com/c/envs%2Fprod%2Fmain.tf");
        assert_eq!(render_url("https://h.example.com/c/{{inputs.path | path}}?n={{inputs.n}}", &c).unwrap(), "https://h.example.com/c/envs/prod/main.tf?n=3");
        assert_eq!(render_url("https://h.example.com/u?e={{user.email}}", &c).unwrap(), "https://h.example.com/u?e=dana%40example.com");
        // Dot segments would walk up the path.
        let dots = map(json!({"repo": "..", "path": "a/../b"}));
        let c = ctx(&dots, &s);
        assert!(render_url("https://h.example.com/r/{{inputs.repo}}/x", &c).unwrap_err().contains(".."));
        assert!(render_url("https://h.example.com/r/{{inputs.path | path}}", &c).unwrap_err().contains(".."));
    }

    #[test]
    fn json_bodies_keep_their_structure() {
        let i = map(json!({"name": "shop\", \"admin\": true, \"x\": \"", "private": true, "count": 2, "desc": "line\nbreak"}));
        let s = secrets();
        let c = ctx(&i, &s);
        let body = render_json(
            r#"{"name": "{{inputs.name}}", "private": "{{inputs.private}}", "n": "{{inputs.count}}", "about": "By {{user.name}}: {{inputs.desc}}", "tags": ["{{inputs.count}} repos"]}"#,
            &c,
        )
        .unwrap();
        let v: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(v["name"], json!("shop\", \"admin\": true, \"x\": \""), "quotes stay inside the string");
        assert!(v.get("admin").is_none());
        assert_eq!(v["private"], json!(true), "a lone placeholder keeps the value's type");
        assert_eq!(v["n"], json!(2));
        assert_eq!(v["about"], json!("By Dana Lee: line\nbreak"));
        assert_eq!(v["tags"], json!(["2 repos"]));
        let b64 = render_json(r#"{"content": "{{inputs.desc | base64}}"}"#, &c).unwrap();
        assert_eq!(serde_json::from_str::<Value>(&b64).unwrap()["content"], json!("bGluZQpicmVhaw=="));
        assert_eq!(render_json("", &c).unwrap(), "");
    }

    #[test]
    fn later_requests_read_earlier_responses() {
        let i = map(json!({"branch": "add-bucket"}));
        let s = BTreeMap::new();
        let mut c = ctx(&i, &s);
        c.steps.push(json!({"object": {"sha": "abc123"}, "items": [{"id": 7}]}));
        let body = render_json(r#"{"ref": "refs/heads/{{inputs.branch}}", "sha": "{{steps.1.object.sha}}", "id": "{{steps.1.items.0.id}}"}"#, &c).unwrap();
        assert_eq!(serde_json::from_str::<Value>(&body).unwrap(), json!({"ref": "refs/heads/add-bucket", "sha": "abc123", "id": 7}));
        assert!(render_text("{{steps.1.object.missing}}", &c, true).unwrap_err().contains("steps.1.object.missing"));
        assert!(render_text("{{steps.2.x}}", &c, true).is_err());
    }

    #[test]
    fn headers_refuse_line_breaks() {
        let i = map(json!({"v": "a\r\nX-Injected: 1"}));
        let s = secrets();
        let c = ctx(&i, &s);
        assert_eq!(render_text("Bearer {{secrets.github_token}}", &c, true).unwrap(), "Bearer ghp_s3cr3t/t0ken");
        assert!(render_text("{{inputs.v}}", &c, true).unwrap_err().contains("line break"));
        assert_eq!(render_text("{{inputs.v}}", &c, false).unwrap(), "a\r\nX-Injected: 1");
        assert_eq!(render_text("{{inputs.absent}}!", &c, true).unwrap(), "!", "unset optional inputs are empty");
    }

    #[test]
    fn templates_only_refer_to_known_names() {
        let inputs = vec![input("repo", ActionInputKind::Text)];
        let names: HashSet<String> = ["github_token".to_string()].into();
        let http = TemplateScope { inputs: &inputs, secrets: &names, allow_secrets: true, prior_steps: 1, roots: &[] };
        assert!(check_template("{{inputs.repo}} {{user.email}} {{user.name}} {{secrets.github_token}} {{steps.1.sha}} {{inputs.repo | path}}", &http).is_ok());
        assert!(check_template("{{inputs.other}}", &http).unwrap_err().contains("inputs.other"));
        assert!(check_template("{{secrets.aws}}", &http).unwrap_err().contains("secrets.aws"));
        assert!(check_template("{{steps.2.sha}}", &http).unwrap_err().contains("steps.2"));
        assert!(check_template("{{inputs.repo | upper}}", &http).unwrap_err().contains("upper"));
        assert!(check_template("{{user.password}}", &http).is_err());
        assert!(check_template("{{inputs.repo", &http).unwrap_err().contains("}}"));
        let internal = TemplateScope { allow_secrets: false, prior_steps: 0, ..http };
        assert!(check_template("{{secrets.github_token}}", &internal).unwrap_err().contains("HTTP"));
    }

    #[test]
    fn workflow_templates_read_the_run_data() {
        let roots = ["Trigger", "Steps", "inputs", "user"];
        let names = HashSet::new();
        let scope = TemplateScope { inputs: &[], secrets: &names, allow_secrets: true, prior_steps: 0, roots: &roots };
        assert!(check_template("{{ Trigger.monitor.name }} {{Steps.check.output.items.0.id}} {{user.email}}", &scope).is_ok());
        assert!(check_template("{{ Loop.item }}", &scope).unwrap_err().contains("Loop"), "only the given roots");
        assert!(check_template("{{ inputs.missing }}", &scope).is_err(), "inputs are still checked against the form");
        let vars = json!({"Trigger": {"monitor": {"name": "High error rate", "id": 7}}, "Steps": {"check": {"output": {"items": [{"id": "a b/c"}]}}}});
        let i = Map::new();
        let s = BTreeMap::new();
        let c = RenderContext { vars: Some(&vars), ..ctx(&i, &s) };
        assert_eq!(render_text("{{ Trigger.monitor.name }} by {{user.name}}", &c, true).unwrap(), "High error rate by Dana Lee");
        assert_eq!(render_url("https://h.example.com/x/{{Steps.check.output.items.0.id}}", &c).unwrap(), "https://h.example.com/x/a%20b%2Fc");
        assert_eq!(serde_json::from_str::<Value>(&render_json(r#"{"id": "{{Trigger.monitor.id}}"}"#, &c).unwrap()).unwrap(), json!({"id": 7}));
        assert_eq!(render_text("[{{ Steps.absent.output }}]", &c, true).unwrap(), "[]");
    }

    #[test]
    fn requests_have_literal_hosts_and_json_bodies() {
        let inputs = vec![input("host", ActionInputKind::Text)];
        let names = HashSet::new();
        let scope = TemplateScope { inputs: &inputs, secrets: &names, allow_secrets: true, prior_steps: 0, roots: &[] };
        let req = |method: &str, url: &str, body: &str| ActionHttpRequest { method: method.into(), url: url.into(), headers: vec![], body: body.into() };
        let ok = check_request(&req("post", " https://api.github.com/repos/{{inputs.host}} ", r#"{"a": "{{inputs.host}}"}"#), &scope).unwrap();
        assert_eq!(ok.method, "POST");
        assert_eq!(ok.url, "https://api.github.com/repos/{{inputs.host}}");
        assert!(check_request(&req("POST", "https://{{inputs.host}}/x", ""), &scope).unwrap_err().contains("host"));
        assert!(check_request(&req("POST", "https://api.{{inputs.host}}.com/x", ""), &scope).unwrap_err().contains("host"));
        assert!(check_request(&req("POST", "ftp://example.com/x", ""), &scope).unwrap_err().contains("http"));
        assert!(check_request(&req("TRACE", "https://example.com/x", ""), &scope).unwrap_err().contains("method"));
        assert!(check_request(&req("POST", "https://example.com/x", r#"{"private": {{inputs.host}}}"#), &scope).unwrap_err().contains("JSON"));
        let mut bad_header = req("GET", "https://example.com/x", "");
        bad_header.headers.push(ActionHeader { name: "X Bad".into(), value: String::new() });
        assert!(check_request(&bad_header, &scope).unwrap_err().contains("X Bad"));
    }

    #[test]
    fn steps_are_checked() {
        let inputs = vec![input("title", ActionInputKind::Text)];
        let names: HashSet<String> = ["token".to_string()].into();
        let internal = |kind, fields: &[(&str, &str)]| ActionStep { kind, requests: vec![], fields: fields.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect() };
        let ok = check_step(&internal(ActionStepKind::CreateIncident, &[("title", " {{inputs.title}} "), ("severity", "SEV-2")]), &inputs, &names).unwrap();
        assert_eq!(ok.fields["title"], "{{inputs.title}}");
        assert!(check_step(&internal(ActionStepKind::CreateIncident, &[("severity", "SEV-2")]), &inputs, &names).unwrap_err().contains("title"));
        assert!(check_step(&internal(ActionStepKind::PageOnCall, &[("team", "x"), ("title", "y"), ("color", "z")]), &inputs, &names).unwrap_err().contains("color"));
        assert!(check_step(&internal(ActionStepKind::SyntheticRun, &[("test", "{{secrets.token}}")]), &inputs, &names).unwrap_err().contains("HTTP"));
        assert!(check_step(&internal(ActionStepKind::RunWorkflow, &[]), &inputs, &names).unwrap_err().contains("workflow"));
        assert!(check_step(&internal(ActionStepKind::RunWorkflow, &[("workflow", "7a3e1c20-5b6d-4e7f-9a1b-2c3d4e5f6a04")]), &inputs, &names).is_ok());
        let none = ActionStep { kind: ActionStepKind::Http, requests: vec![], fields: BTreeMap::new() };
        assert!(check_step(&none, &inputs, &names).unwrap_err().contains("1–5"));
        let two = ActionStep {
            kind: ActionStepKind::Http,
            requests: vec![ActionHttpRequest { method: "GET".into(), url: "https://example.com/a/{{steps.1.x}}".into(), headers: vec![], body: String::new() }],
            fields: BTreeMap::new(),
        };
        assert!(check_step(&two, &inputs, &names).unwrap_err().contains("steps.1"), "a request can't read its own response");
    }

    #[test]
    fn form_fields_are_checked() {
        let mut select = input("env", ActionInputKind::Select);
        assert!(check_inputs(&[select.clone()]).unwrap_err().contains("options"));
        select.options = vec!["staging".into(), "production".into()];
        select.default = "qa".into();
        assert!(check_inputs(&[select.clone()]).unwrap_err().contains("qa"));
        select.default = "staging".into();
        assert!(check_inputs(&[select.clone()]).is_ok());
        assert!(check_inputs(&[input("Bad-Key", ActionInputKind::Text)]).unwrap_err().contains("Bad-Key"));
        assert!(check_inputs(&[input("a", ActionInputKind::Text), input("a", ActionInputKind::Number)]).unwrap_err().contains("twice"));
        let mut flag = input("private", ActionInputKind::Boolean);
        flag.default = "yes".into();
        assert!(check_inputs(&[flag]).unwrap_err().contains("true or false"));
        let mut n = input("count", ActionInputKind::Number);
        n.default = "two".into();
        assert!(check_inputs(&[n]).unwrap_err().contains("number"));
    }

    #[test]
    fn run_inputs_follow_the_form() {
        let mut env = input("env", ActionInputKind::Select);
        env.options = vec!["staging".into(), "production".into()];
        env.default = "staging".into();
        let mut name = input("name", ActionInputKind::Text);
        name.required = true;
        let mut private = input("private", ActionInputKind::Boolean);
        private.default = "true".into();
        let defs = vec![env, name, private, input("replicas", ActionInputKind::Number)];
        let out = check_run_inputs(&defs, &map(json!({"name": " checkout-web ", "replicas": "3"}))).unwrap();
        assert_eq!(Value::Object(out), json!({"env": "staging", "name": "checkout-web", "private": true, "replicas": 3}));
        let out = check_run_inputs(&defs, &map(json!({"name": "x", "private": "false", "replicas": 2.5, "env": "production"}))).unwrap();
        assert_eq!(Value::Object(out), json!({"env": "production", "name": "x", "private": false, "replicas": 2.5}));
        let err = |v: Value| check_run_inputs(&defs, &map(v)).unwrap_err();
        assert!(err(json!({})).contains("name"));
        assert!(err(json!({"name": "x", "env": "qa"})).contains("qa"));
        assert!(err(json!({"name": "x", "replicas": "many"})).contains("number"));
        assert!(err(json!({"name": "x", "private": "maybe"})).contains("true or false"));
        assert!(err(json!({"name": "x", "extra": 1})).contains("extra"));
        assert!(err(json!({"name": "x".repeat(5001)})).contains("5000"));
    }

    #[test]
    fn secrets_are_redacted_in_every_form() {
        let s = secrets();
        let raw = "token=ghp_s3cr3t/t0ken";
        assert_eq!(redact(raw, &s), "token=[REDACTED]");
        assert_eq!(redact("u=ghp_s3cr3t%2Ft0ken", &s), "u=[REDACTED]");
        assert_eq!(redact(&format!("basic {}", STANDARD.encode("ghp_s3cr3t/t0ken")), &s), "basic [REDACTED]");
        assert_eq!(redact("nothing here", &s), "nothing here");
        assert_eq!(redact("x", &BTreeMap::from([("empty".to_string(), String::new())])), "x");
    }

    #[test]
    fn links_to_what_was_created() {
        assert_eq!(result_link(&json!({"html_url": "https://github.com/example-org/shop"})).as_deref(), Some("https://github.com/example-org/shop"));
        assert_eq!(result_link(&json!({"web_url": "https://gitlab.example.com/g/p"})).as_deref(), Some("https://gitlab.example.com/g/p"));
        assert_eq!(result_link(&json!({"key": "SHOP-12", "self": "https://example.atlassian.net/rest/api/2/issue/10012"})).as_deref(), Some("https://example.atlassian.net/browse/SHOP-12"));
        assert_eq!(result_link(&json!({"html_url": "javascript:alert(1)"})), None);
        assert_eq!(result_link(&json!([1, 2])), None);
    }
}
