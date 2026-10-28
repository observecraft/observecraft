//! Running one synthetic API test: an HTTP request, a TLS handshake (SSL), DNS lookups or a TCP
//! connection, then its assertions. Every connection goes to an address checked by
//! [`super::synthetics_net`]; HTTP redirects are followed by hand so each hop is checked again,
//! response bodies are read up to [`MAX_BODY_BYTES`] and every run is bounded by its timeout.

use std::net::IpAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;

use super::synthetics_net::{check_addr, resolve};

/// Response bodies are read (and asserted on) up to this size.
pub const MAX_BODY_BYTES: usize = 1 << 20;
pub const MAX_REDIRECTS: usize = 10;
pub const MAX_TIMEOUT_SECONDS: u32 = 60;
/// Body kept with a result for display.
pub const BODY_PREVIEW_BYTES: usize = 64 * 1024;
const MAX_KEPT_HEADERS: usize = 100;
const MAX_PATTERN_CHARS: usize = 1000;
const USER_AGENT: &str = "ObserveCraft-Synthetics/1.0";

#[derive(Serialize, Deserialize, ToSchema, Clone, Copy, PartialEq, Eq, Debug, Hash)]
#[serde(rename_all = "lowercase")]
pub enum SyntheticTestType {
    Http,
    Ssl,
    Dns,
    Tcp,
}

impl SyntheticTestType {
    pub fn as_str(self) -> &'static str {
        match self {
            SyntheticTestType::Http => "http",
            SyntheticTestType::Ssl => "ssl",
            SyntheticTestType::Dns => "dns",
            SyntheticTestType::Tcp => "tcp",
        }
    }
    pub fn parse(s: &str) -> Self {
        match s {
            "ssl" => SyntheticTestType::Ssl,
            "dns" => SyntheticTestType::Dns,
            "tcp" => SyntheticTestType::Tcp,
            _ => SyntheticTestType::Http,
        }
    }
}

#[derive(Serialize, Deserialize, ToSchema, Clone, Debug, PartialEq)]
pub struct SyntheticHeader {
    pub name: String,
    pub value: String,
}

#[derive(Serialize, Deserialize, ToSchema, Clone, Debug, PartialEq)]
pub struct SyntheticBasicAuth {
    pub username: String,
    pub password: String,
}

/// What a test sends. HTTP tests use `method`, `url`, `headers`, `body`, `basicAuth` and
/// `followRedirects`; SSL, DNS and TCP tests use `host` and `port`; DNS tests may name a server.
#[derive(Serialize, Deserialize, ToSchema, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct SyntheticRequest {
    /// HTTP: GET, POST, PUT, PATCH, DELETE, HEAD or OPTIONS.
    pub method: String,
    /// HTTP: an http or https URL.
    pub url: String,
    /// SSL, TCP: host name or IP address; DNS: the domain to look up.
    pub host: String,
    /// SSL (default 443) and TCP.
    pub port: Option<u16>,
    pub headers: Vec<SyntheticHeader>,
    pub body: String,
    /// Content type sent with a body (`application/json`, `text/plain`, …).
    pub body_type: String,
    pub basic_auth: Option<SyntheticBasicAuth>,
    /// 1–60 seconds.
    pub timeout_seconds: u32,
    /// HTTP: follow up to 10 redirects.
    pub follow_redirects: bool,
    /// HTTP(S) and SSL: accept self-signed and otherwise untrusted certificates.
    pub accept_self_signed: bool,
    /// DNS: IP address of the server to ask; empty for the ObserveCraft server's resolver.
    pub dns_server: String,
    /// DNS server port (default 53).
    pub dns_server_port: Option<u16>,
}

impl Default for SyntheticRequest {
    fn default() -> Self {
        SyntheticRequest {
            method: "GET".into(),
            url: String::new(),
            host: String::new(),
            port: None,
            headers: Vec::new(),
            body: String::new(),
            body_type: String::new(),
            basic_auth: None,
            timeout_seconds: 30,
            follow_redirects: true,
            accept_self_signed: false,
            dns_server: String::new(),
            dns_server_port: None,
        }
    }
}

#[derive(Serialize, Deserialize, ToSchema, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "camelCase")]
pub enum SyntheticAssertionType {
    StatusCode,
    /// Milliseconds.
    ResponseTime,
    /// `property` is the header name.
    Header,
    Body,
    /// `property` is a JSON path such as `$.items[0].status`.
    BodyJsonPath,
    /// Days until the certificate expires.
    Certificate,
    /// Every record of type `property` (A, AAAA, CNAME, MX, TXT, NS) passes; there must be one.
    RecordEvery,
    /// At least one record of type `property` passes.
    RecordSome,
}

#[derive(Serialize, Deserialize, ToSchema, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "camelCase")]
pub enum SyntheticOperator {
    Is,
    IsNot,
    LessThan,
    MoreThan,
    Contains,
    DoesNotContain,
    Matches,
    DoesNotMatch,
    /// Certificates: expires in more than `target` days.
    IsInMoreThan,
}

#[derive(Serialize, Deserialize, ToSchema, Clone, Debug, PartialEq)]
pub struct SyntheticAssertion {
    #[serde(rename = "type")]
    pub kind: SyntheticAssertionType,
    pub operator: SyntheticOperator,
    /// Header name, JSON path or DNS record type; empty for the other types.
    #[serde(default)]
    pub property: String,
    /// The expected value (a number for status codes, times and days; a regular expression for matches).
    pub target: String,
}

#[derive(Serialize, Deserialize, ToSchema, Clone, Debug, PartialEq)]
pub struct SyntheticAssertionResult {
    pub assertion: SyntheticAssertion,
    pub passed: bool,
    /// The value found (absent when there was none).
    pub actual: Option<String>,
    pub error: Option<String>,
}

#[derive(Serialize, Deserialize, ToSchema, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SyntheticCertificate {
    pub subject: String,
    pub issuer: String,
    /// Epoch milliseconds.
    pub not_before: i64,
    pub not_after: i64,
    pub days_left: f64,
    /// The chain verified against the public web roots for the host name.
    pub trusted: bool,
    pub trust_error: Option<String>,
    pub protocol: Option<String>,
    pub cipher: Option<String>,
}

#[derive(Serialize, Deserialize, ToSchema, Clone, Debug, PartialEq)]
pub struct SyntheticDnsRecord {
    #[serde(rename = "type")]
    pub kind: String,
    pub value: String,
    pub ttl: u32,
}

/// One run of a test, as stored with its result.
#[derive(Serialize, Deserialize, ToSchema, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct SyntheticRunDetail {
    pub passed: bool,
    /// Why the run failed before its assertions could be checked (timeout, refused address, …).
    pub error: Option<String>,
    /// Response time in milliseconds: the whole HTTP exchange, the TLS handshake, the DNS lookups or the TCP connect.
    pub duration_ms: f64,
    pub assertions: Vec<SyntheticAssertionResult>,
    /// The address connected to.
    pub resolved_ip: Option<String>,
    pub method: Option<String>,
    /// The URL that answered (after redirects).
    pub url: Option<String>,
    pub redirects: Vec<String>,
    pub status_code: Option<u16>,
    pub response_headers: Vec<SyntheticHeader>,
    /// The start of the body (up to 64 KiB); kept with failed runs and runs started by hand.
    pub body: Option<String>,
    pub body_size: Option<u64>,
    /// The body was larger than the 1 MiB read.
    pub body_truncated: bool,
    pub certificate: Option<SyntheticCertificate>,
    pub dns_server: Option<String>,
    pub records: Vec<SyntheticDnsRecord>,
}

// ================================================================ assertions

/// Operators an assertion type accepts.
pub fn operators(kind: SyntheticAssertionType) -> &'static [SyntheticOperator] {
    use SyntheticOperator::*;
    match kind {
        SyntheticAssertionType::StatusCode => &[Is, IsNot, LessThan, MoreThan, Matches, DoesNotMatch],
        SyntheticAssertionType::ResponseTime => &[LessThan, MoreThan],
        SyntheticAssertionType::Header | SyntheticAssertionType::BodyJsonPath => &[Is, IsNot, Contains, DoesNotContain, Matches, DoesNotMatch, LessThan, MoreThan],
        SyntheticAssertionType::Body | SyntheticAssertionType::RecordEvery | SyntheticAssertionType::RecordSome => &[Is, IsNot, Contains, DoesNotContain, Matches, DoesNotMatch],
        SyntheticAssertionType::Certificate => &[IsInMoreThan],
    }
}

/// Assertion types a test type accepts.
pub fn assertion_types(t: SyntheticTestType) -> &'static [SyntheticAssertionType] {
    use SyntheticAssertionType::*;
    match t {
        SyntheticTestType::Http => &[StatusCode, ResponseTime, Header, Body, BodyJsonPath],
        SyntheticTestType::Ssl => &[Certificate, ResponseTime],
        SyntheticTestType::Dns => &[RecordEvery, RecordSome, ResponseTime],
        SyntheticTestType::Tcp => &[ResponseTime],
    }
}

pub const RECORD_TYPES: [&str; 6] = ["A", "AAAA", "CNAME", "MX", "TXT", "NS"];

fn regex(pattern: &str) -> Result<fancy_regex::Regex, String> {
    if pattern.chars().count() > MAX_PATTERN_CHARS {
        return Err(format!("Patterns are at most {MAX_PATTERN_CHARS} characters"));
    }
    fancy_regex::RegexBuilder::new(pattern).backtrack_limit(1_000_000).delegate_size_limit(1 << 20).build().map_err(|e| format!("Invalid pattern '{pattern}': {e}"))
}

fn number(target: &str) -> Result<f64, String> {
    target.trim().parse::<f64>().ok().filter(|v| v.is_finite()).ok_or_else(|| format!("'{target}' is not a number"))
}

/// Checks an assertion's shape for a test type (operator, property, target); the API refuses the others.
pub fn check_assertion(t: SyntheticTestType, a: &SyntheticAssertion) -> Result<(), String> {
    if !assertion_types(t).contains(&a.kind) {
        return Err(format!("{:?} assertions don't apply to {} tests", a.kind, t.as_str().to_uppercase()));
    }
    if !operators(a.kind).contains(&a.operator) {
        return Err(format!("{:?} assertions can't use {:?}", a.kind, a.operator));
    }
    if a.target.chars().count() > 10_000 {
        return Err("Assertion targets are at most 10,000 characters".into());
    }
    match a.kind {
        SyntheticAssertionType::Header if a.property.trim().is_empty() => return Err("Header assertions name a header".into()),
        SyntheticAssertionType::BodyJsonPath => {
            parse_json_path(&a.property)?;
        }
        SyntheticAssertionType::RecordEvery | SyntheticAssertionType::RecordSome if !RECORD_TYPES.contains(&a.property.as_str()) => {
            return Err(format!("DNS record assertions name a record type: {}", RECORD_TYPES.join(", ")));
        }
        _ => {}
    }
    match a.operator {
        SyntheticOperator::Matches | SyntheticOperator::DoesNotMatch => {
            regex(&a.target)?;
        }
        SyntheticOperator::LessThan | SyntheticOperator::MoreThan | SyntheticOperator::IsInMoreThan => {
            number(&a.target)?;
        }
        _ if matches!(a.kind, SyntheticAssertionType::StatusCode) => {
            number(&a.target)?;
        }
        _ => {}
    }
    Ok(())
}

/// What a run observed, for the assertions.
#[derive(Default)]
pub struct Observed<'a> {
    pub status_code: Option<u16>,
    pub response_time_ms: f64,
    pub headers: &'a [SyntheticHeader],
    pub body: Option<&'a str>,
    pub cert_days_left: Option<f64>,
    pub records: &'a [SyntheticDnsRecord],
}

fn compare_number(op: SyntheticOperator, actual: f64, target: &str) -> Result<bool, String> {
    let t = number(target)?;
    Ok(match op {
        SyntheticOperator::Is => actual == t,
        SyntheticOperator::IsNot => actual != t,
        SyntheticOperator::LessThan => actual < t,
        SyntheticOperator::MoreThan | SyntheticOperator::IsInMoreThan => actual > t,
        _ => return Err(format!("{op:?} doesn't compare numbers")),
    })
}

fn compare_text(op: SyntheticOperator, actual: &str, target: &str) -> Result<bool, String> {
    Ok(match op {
        SyntheticOperator::Is => actual == target,
        SyntheticOperator::IsNot => actual != target,
        SyntheticOperator::Contains => actual.contains(target),
        SyntheticOperator::DoesNotContain => !actual.contains(target),
        SyntheticOperator::Matches => regex(target)?.is_match(actual).map_err(|e| e.to_string())?,
        SyntheticOperator::DoesNotMatch => !regex(target)?.is_match(actual).map_err(|e| e.to_string())?,
        SyntheticOperator::LessThan | SyntheticOperator::MoreThan | SyntheticOperator::IsInMoreThan => {
            compare_number(op, number(actual).map_err(|_| format!("'{}' is not a number", short(actual)))?, target)?
        }
    })
}

/// Negative operators pass when there is nothing to compare.
fn negative(op: SyntheticOperator) -> bool {
    matches!(op, SyntheticOperator::IsNot | SyntheticOperator::DoesNotContain | SyntheticOperator::DoesNotMatch)
}

fn short(s: &str) -> String {
    let mut out: String = s.chars().take(200).collect();
    if out.len() < s.len() {
        out.push('…');
    }
    out
}

/// A JSON path: `$.a.b[0]['c d']` (the leading `$` is optional).
#[derive(Debug, PartialEq)]
enum PathStep {
    Key(String),
    Index(usize),
}

fn parse_json_path(path: &str) -> Result<Vec<PathStep>, String> {
    let bad = || format!("Invalid JSON path '{path}': use $.field, $.list[0] or $['field name']");
    let p = path.trim();
    let mut rest = p.strip_prefix('$').unwrap_or(p);
    if p.is_empty() {
        return Err(bad());
    }
    let mut steps = Vec::new();
    // Without `$`, the path starts with a bare key.
    if !p.starts_with('$') && !rest.starts_with(['.', '[']) {
        let end = rest.find(['.', '[']).unwrap_or(rest.len());
        steps.push(PathStep::Key(rest[..end].to_string()));
        rest = &rest[end..];
    }
    while !rest.is_empty() {
        if let Some(r) = rest.strip_prefix('.') {
            let end = r.find(['.', '[']).unwrap_or(r.len());
            if end == 0 {
                return Err(bad());
            }
            steps.push(PathStep::Key(r[..end].to_string()));
            rest = &r[end..];
        } else if let Some(r) = rest.strip_prefix('[') {
            let close = r.find(']').ok_or_else(bad)?;
            let inner = r[..close].trim();
            let quoted = inner.len() >= 2 && ((inner.starts_with('\'') && inner.ends_with('\'')) || (inner.starts_with('"') && inner.ends_with('"')));
            if quoted {
                steps.push(PathStep::Key(inner[1..inner.len() - 1].to_string()));
            } else {
                steps.push(PathStep::Index(inner.parse().map_err(|_| bad())?));
            }
            rest = &r[close + 1..];
        } else {
            return Err(bad());
        }
    }
    Ok(steps)
}

fn json_at<'a>(v: &'a Value, steps: &[PathStep]) -> Option<&'a Value> {
    steps.iter().try_fold(v, |v, s| match s {
        PathStep::Key(k) => v.get(k.as_str()),
        PathStep::Index(i) => v.get(*i),
    })
}

fn json_text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// Checks one assertion against what a run observed.
pub fn evaluate(a: &SyntheticAssertion, o: &Observed) -> SyntheticAssertionResult {
    let op = a.operator;
    let (actual, outcome): (Option<String>, Result<bool, String>) = match a.kind {
        SyntheticAssertionType::StatusCode => match o.status_code {
            Some(code) if matches!(op, SyntheticOperator::Matches | SyntheticOperator::DoesNotMatch) => (Some(code.to_string()), compare_text(op, &code.to_string(), &a.target)),
            Some(code) => (Some(code.to_string()), compare_number(op, f64::from(code), &a.target)),
            None => (None, Ok(false)),
        },
        SyntheticAssertionType::ResponseTime => (Some(format!("{}", o.response_time_ms.round())), compare_number(op, o.response_time_ms, &a.target)),
        SyntheticAssertionType::Header => {
            let name = a.property.trim();
            let values: Vec<&str> = o.headers.iter().filter(|h| h.name.eq_ignore_ascii_case(name)).map(|h| h.value.as_str()).collect();
            if values.is_empty() {
                (None, Ok(negative(op)))
            } else {
                let joined = values.join(", ");
                (Some(short(&joined)), compare_text(op, &joined, &a.target))
            }
        }
        SyntheticAssertionType::Body => match o.body {
            Some(b) => (Some(short(b)), compare_text(op, b, &a.target)),
            None => (None, Ok(negative(op))),
        },
        SyntheticAssertionType::BodyJsonPath => match parse_json_path(&a.property) {
            Err(e) => (None, Err(e)),
            Ok(steps) => match o.body.map(serde_json::from_str::<Value>) {
                Some(Ok(doc)) => match json_at(&doc, &steps) {
                    Some(v) => {
                        let text = json_text(v);
                        (Some(short(&text)), compare_text(op, &text, &a.target))
                    }
                    None => (None, Ok(negative(op))),
                },
                Some(Err(_)) => (None, Err("The body is not JSON".into())),
                None => (None, Ok(negative(op))),
            },
        },
        SyntheticAssertionType::Certificate => match o.cert_days_left {
            Some(days) => (Some(format!("{:.1}", days)), compare_number(op, days, &a.target)),
            None => (None, Ok(false)),
        },
        SyntheticAssertionType::RecordEvery | SyntheticAssertionType::RecordSome => {
            let values: Vec<&str> = o.records.iter().filter(|r| r.kind == a.property).map(|r| r.value.as_str()).collect();
            if values.is_empty() {
                (None, Ok(false))
            } else {
                let checks: Result<Vec<bool>, String> = values.iter().map(|v| compare_text(op, v, &a.target)).collect();
                let every = a.kind == SyntheticAssertionType::RecordEvery;
                (Some(short(&values.join(", "))), checks.map(|c| if every { c.iter().all(|x| *x) } else { c.iter().any(|x| *x) }))
            }
        }
    };
    match outcome {
        Ok(passed) => SyntheticAssertionResult { assertion: a.clone(), passed, actual, error: None },
        Err(e) => SyntheticAssertionResult { assertion: a.clone(), passed: false, actual, error: Some(e) },
    }
}

/// Assertions to start from after sending a request once from the editor.
pub fn suggest(t: SyntheticTestType, run: &SyntheticRunDetail) -> Vec<SyntheticAssertion> {
    use SyntheticAssertionType as K;
    use SyntheticOperator as Op;
    let a = |kind, operator, property: &str, target: String| SyntheticAssertion { kind, operator, property: property.to_string(), target };
    // Under a second, or twice what this run took (rounded up to 100 ms) when it was slower.
    let time = a(K::ResponseTime, Op::LessThan, "", format!("{}", ((run.duration_ms * 2.0 / 100.0).ceil() as i64 * 100).max(1000)));
    let mut out = Vec::new();
    match t {
        SyntheticTestType::Http => {
            if let Some(code) = run.status_code {
                out.push(a(K::StatusCode, Op::Is, "", code.to_string()));
            }
            out.push(time);
            if let Some(ct) = run.response_headers.iter().find(|h| h.name.eq_ignore_ascii_case("content-type")) {
                out.push(a(K::Header, Op::Is, "content-type", ct.value.clone()));
            }
        }
        SyntheticTestType::Ssl => {
            if run.certificate.is_some() {
                out.push(a(K::Certificate, Op::IsInMoreThan, "", "10".into()));
            }
            out.push(time);
        }
        SyntheticTestType::Dns => {
            out.push(time);
            if let Some(r) = run.records.iter().find(|r| r.kind == "A").or(run.records.first()) {
                out.push(a(K::RecordSome, Op::Is, &r.kind, r.value.clone()));
            }
        }
        SyntheticTestType::Tcp => out.push(time),
    }
    out
}

// ================================================================ running

/// Runs a test once and checks its assertions.
pub async fn run(t: SyntheticTestType, req: &SyntheticRequest, assertions: &[SyntheticAssertion], allow_private: bool) -> SyntheticRunDetail {
    let timeout = Duration::from_secs(u64::from(req.timeout_seconds.clamp(1, MAX_TIMEOUT_SECONDS)));
    let mut out = SyntheticRunDetail::default();
    let started = Instant::now();
    let mut body = String::new();
    let outcome = tokio::time::timeout(timeout, async {
        match t {
            SyntheticTestType::Http => run_http(req, allow_private, &mut out, &mut body).await,
            SyntheticTestType::Ssl => run_ssl(req, allow_private, &mut out).await,
            SyntheticTestType::Dns => run_dns(req, allow_private, timeout, &mut out).await,
            SyntheticTestType::Tcp => run_tcp(req, allow_private, &mut out).await,
        }
    })
    .await
    .unwrap_or_else(|_| Err(format!("Timed out after {} s", timeout.as_secs())));
    if out.duration_ms == 0.0 {
        out.duration_ms = started.elapsed().as_secs_f64() * 1000.0;
    }
    if let Err(e) = outcome {
        out.error = Some(e);
        out.passed = false;
        if t == SyntheticTestType::Http && out.status_code.is_some() {
            out.body = Some(preview(&body));
        }
        return out;
    }
    let observed = Observed {
        status_code: out.status_code,
        response_time_ms: out.duration_ms,
        headers: &out.response_headers,
        body: (t == SyntheticTestType::Http).then_some(body.as_str()),
        cert_days_left: out.certificate.as_ref().map(|c| c.days_left),
        records: &out.records,
    };
    let results: Vec<SyntheticAssertionResult> = assertions.iter().map(|a| evaluate(a, &observed)).collect();
    out.passed = results.iter().all(|r| r.passed);
    out.assertions = results;
    if t == SyntheticTestType::Http {
        out.body = Some(preview(&body));
    }
    out
}

/// The start of a body, cut on a character boundary.
fn preview(body: &str) -> String {
    if body.len() <= BODY_PREVIEW_BYTES {
        return body.to_string();
    }
    let mut end = BODY_PREVIEW_BYTES;
    while !body.is_char_boundary(end) {
        end -= 1;
    }
    body[..end].to_string()
}

/// An error with its causes, as one line.
fn describe(e: &(dyn std::error::Error + 'static)) -> String {
    let mut out = e.to_string();
    let mut source = e.source();
    while let Some(s) = source {
        let text = s.to_string();
        if !out.contains(&text) {
            out.push_str(": ");
            out.push_str(&text);
        }
        source = s.source();
    }
    out
}

fn same_origin(a: &url::Url, b: &url::Url) -> bool {
    a.scheme() == b.scheme() && a.host_str() == b.host_str() && a.port_or_known_default() == b.port_or_known_default()
}

/// The method and body of the request that follows a redirect answered with `status`.
fn after_redirect(status: u16, method: &reqwest::Method) -> (reqwest::Method, bool) {
    let to_get = status == 303 && *method != reqwest::Method::HEAD || (matches!(status, 301 | 302) && *method == reqwest::Method::POST);
    if to_get {
        (reqwest::Method::GET, false)
    } else {
        (method.clone(), true)
    }
}

async fn run_http(r: &SyntheticRequest, allow_private: bool, out: &mut SyntheticRunDetail, body_out: &mut String) -> Result<(), String> {
    let first = url::Url::parse(r.url.trim()).map_err(|e| format!("Invalid URL: {e}"))?;
    let mut url = first.clone();
    let mut method = reqwest::Method::from_bytes(r.method.trim().to_ascii_uppercase().as_bytes()).map_err(|_| format!("Invalid method {}", r.method))?;
    let mut send_body = !r.body.is_empty();
    let started = Instant::now();
    out.method = Some(method.to_string());
    for hop in 0..=MAX_REDIRECTS {
        if !matches!(url.scheme(), "http" | "https") {
            return Err(format!("Only http and https URLs can be tested ({url})"));
        }
        let host = url.host_str().ok_or("The URL has no host")?.to_string();
        let port = url.port_or_known_default().ok_or("The URL has no port")?;
        let addr = resolve(&host, port, allow_private).await?;
        out.resolved_ip = Some(addr.ip().to_string());
        let mut builder = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .no_proxy()
            .user_agent(USER_AGENT)
            .danger_accept_invalid_certs(r.accept_self_signed)
            .connect_timeout(Duration::from_secs(u64::from(MAX_TIMEOUT_SECONDS)));
        // Connect to the checked address, never to a second lookup's answer.
        if matches!(url.host(), Some(url::Host::Domain(_))) {
            builder = builder.resolve(&host, addr);
        }
        let client = builder.build().map_err(|e| describe(&e))?;
        let mut rb = client.request(method.clone(), url.clone());
        // Credentials only go to the origin the test names.
        let trusted = same_origin(&url, &first);
        for h in &r.headers {
            let sensitive = ["authorization", "cookie", "proxy-authorization"].contains(&h.name.trim().to_ascii_lowercase().as_str());
            if trusted || !sensitive {
                rb = rb.header(h.name.trim(), h.value.as_str());
            }
        }
        if let (Some(auth), true) = (&r.basic_auth, trusted) {
            rb = rb.basic_auth(&auth.username, Some(&auth.password));
        }
        if send_body {
            if !r.body_type.is_empty() && !r.headers.iter().any(|h| h.name.trim().eq_ignore_ascii_case("content-type")) {
                rb = rb.header("content-type", r.body_type.as_str());
            }
            rb = rb.body(r.body.clone());
        }
        let mut resp = rb.send().await.map_err(|e| describe(&e))?;
        let status = resp.status().as_u16();
        let location = resp.headers().get(reqwest::header::LOCATION).and_then(|v| v.to_str().ok()).map(str::to_string);
        if r.follow_redirects && resp.status().is_redirection() {
            if let Some(loc) = location {
                if hop == MAX_REDIRECTS {
                    return Err(format!("More than {MAX_REDIRECTS} redirects"));
                }
                let next = url.join(&loc).map_err(|e| format!("Invalid redirect to {loc}: {e}"))?;
                out.redirects.push(next.to_string());
                let (m, keep_body) = after_redirect(status, &method);
                method = m;
                send_body &= keep_body;
                url = next;
                continue;
            }
        }
        out.url = Some(url.to_string());
        out.status_code = Some(status);
        out.response_headers = resp
            .headers()
            .iter()
            .take(MAX_KEPT_HEADERS)
            .map(|(k, v)| SyntheticHeader { name: k.as_str().to_string(), value: String::from_utf8_lossy(v.as_bytes()).chars().take(4096).collect() })
            .collect();
        let mut buf: Vec<u8> = Vec::new();
        loop {
            match resp.chunk().await {
                Ok(Some(chunk)) => {
                    let room = MAX_BODY_BYTES - buf.len();
                    if chunk.len() > room {
                        buf.extend_from_slice(&chunk[..room]);
                        out.body_truncated = true;
                        break;
                    }
                    buf.extend_from_slice(&chunk);
                }
                Ok(None) => break,
                Err(e) => {
                    out.duration_ms = started.elapsed().as_secs_f64() * 1000.0;
                    *body_out = String::from_utf8_lossy(&buf).into_owned();
                    return Err(format!("Reading the body failed: {}", describe(&e)));
                }
            }
        }
        out.duration_ms = started.elapsed().as_secs_f64() * 1000.0;
        out.body_size = Some(buf.len() as u64);
        *body_out = String::from_utf8_lossy(&buf).into_owned();
        return Ok(());
    }
    Err(format!("More than {MAX_REDIRECTS} redirects"))
}

/// Records the chain's verification against the web roots, then lets the handshake go on: an SSL
/// test reports on untrusted certificates instead of failing before it can see them.
#[derive(Debug)]
struct Inspect {
    inner: Arc<rustls::client::WebPkiServerVerifier>,
    verdict: Mutex<Option<Result<(), String>>>,
}

impl rustls::client::danger::ServerCertVerifier for Inspect {
    fn verify_server_cert(
        &self,
        end_entity: &rustls::pki_types::CertificateDer<'_>,
        intermediates: &[rustls::pki_types::CertificateDer<'_>],
        server_name: &rustls::pki_types::ServerName<'_>,
        ocsp: &[u8],
        now: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        let verdict = self.inner.verify_server_cert(end_entity, intermediates, server_name, ocsp, now).map(|_| ()).map_err(|e| e.to_string());
        *self.verdict.lock().unwrap() = Some(verdict);
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &rustls::pki_types::CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        self.inner.verify_tls12_signature(message, cert, dss)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &rustls::pki_types::CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        self.inner.verify_tls13_signature(message, cert, dss)
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        self.inner.supported_verify_schemes()
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

/// Subject, issuer and validity of a DER certificate.
fn certificate_info(der: &[u8], now: i64) -> Result<(String, String, i64, i64, f64), String> {
    use x509_cert::der::Decode;
    let cert = x509_cert::Certificate::from_der(der).map_err(|e| format!("Unreadable certificate: {e}"))?;
    let tbs = &cert.tbs_certificate;
    let not_before = tbs.validity.not_before.to_unix_duration().as_millis() as i64;
    let not_after = tbs.validity.not_after.to_unix_duration().as_millis() as i64;
    let days_left = (not_after - now) as f64 / 86_400_000.0;
    Ok((tbs.subject.to_string(), tbs.issuer.to_string(), not_before, not_after, days_left))
}

async fn run_ssl(r: &SyntheticRequest, allow_private: bool, out: &mut SyntheticRunDetail) -> Result<(), String> {
    let host = r.host.trim().trim_start_matches('[').trim_end_matches(']').to_string();
    let port = r.port.unwrap_or(443);
    let addr = resolve(&host, port, allow_private).await?;
    out.resolved_ip = Some(addr.ip().to_string());
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let mut roots = rustls::RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    let inner = rustls::client::WebPkiServerVerifier::builder_with_provider(Arc::new(roots), provider.clone()).build().map_err(|e| e.to_string())?;
    let inspect = Arc::new(Inspect { inner, verdict: Mutex::new(None) });
    let config = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|e| e.to_string())?
        .dangerous()
        .with_custom_certificate_verifier(inspect.clone())
        .with_no_client_auth();
    let name = rustls::pki_types::ServerName::try_from(host.clone()).map_err(|_| format!("Invalid host name {host}"))?;
    let started = Instant::now();
    let tcp = tokio::net::TcpStream::connect(addr).await.map_err(|e| format!("Connection to {addr} failed: {e}"))?;
    let tls = tokio_rustls::TlsConnector::from(Arc::new(config)).connect(name, tcp).await.map_err(|e| format!("TLS handshake failed: {e}"))?;
    out.duration_ms = started.elapsed().as_secs_f64() * 1000.0;
    let (_, conn) = tls.get_ref();
    let leaf = conn.peer_certificates().and_then(|c| c.first()).ok_or("The server sent no certificate")?;
    let now = now_ms();
    let (subject, issuer, not_before, not_after, days_left) = certificate_info(leaf.as_ref(), now)?;
    let verdict = inspect.verdict.lock().unwrap().clone().unwrap_or(Err("The certificate was not checked".into()));
    out.certificate = Some(SyntheticCertificate {
        subject,
        issuer,
        not_before,
        not_after,
        days_left,
        trusted: verdict.is_ok(),
        trust_error: verdict.clone().err(),
        protocol: conn.protocol_version().map(|v| format!("{v:?}").replace('_', ".").replace("TLSv1.", "TLS 1.")),
        cipher: conn.negotiated_cipher_suite().map(|c| format!("{:?}", c.suite())),
    });
    if now >= not_after {
        return Err("The certificate has expired".into());
    }
    if now < not_before {
        return Err("The certificate is not valid yet".into());
    }
    match verdict {
        Err(e) if !r.accept_self_signed => Err(format!("The certificate is not trusted: {e}")),
        _ => Ok(()),
    }
}

async fn run_dns(r: &SyntheticRequest, allow_private: bool, timeout: Duration, out: &mut SyntheticRunDetail) -> Result<(), String> {
    use hickory_resolver::config::{NameServerConfigGroup, ResolverConfig, ResolverOpts};
    use hickory_resolver::name_server::TokioConnectionProvider;
    use hickory_resolver::proto::rr::RecordType;
    use hickory_resolver::{ResolveErrorKind, Resolver};
    let domain = r.host.trim().trim_end_matches('.').to_string();
    let mut opts = ResolverOpts::default();
    opts.timeout = timeout.min(Duration::from_secs(10));
    opts.attempts = 1;
    opts.cache_size = 0;
    let resolver = if r.dns_server.trim().is_empty() {
        out.dns_server = Some("system resolver".into());
        Resolver::builder_tokio().map_err(|e| format!("No system resolver: {e}"))?.with_options(opts).build()
    } else {
        let ip: IpAddr = r.dns_server.trim().parse().map_err(|_| "The DNS server must be an IP address".to_string())?;
        check_addr(ip, allow_private)?;
        let port = r.dns_server_port.unwrap_or(53);
        out.dns_server = Some(format!("{ip}:{port}"));
        let config = ResolverConfig::from_parts(None, vec![], NameServerConfigGroup::from_ips_clear(&[ip], port, true));
        Resolver::builder_with_config(config, TokioConnectionProvider::default()).with_options(opts).build()
    };
    let started = Instant::now();
    let types = [RecordType::A, RecordType::AAAA, RecordType::CNAME, RecordType::MX, RecordType::TXT, RecordType::NS];
    let lookups = futures::future::join_all(types.iter().map(|t| resolver.lookup(format!("{domain}."), *t))).await;
    out.duration_ms = started.elapsed().as_secs_f64() * 1000.0;
    let mut failures = Vec::new();
    for (t, res) in types.iter().zip(lookups) {
        match res {
            Ok(lookup) => {
                for rec in lookup.record_iter().filter(|rec| rec.record_type() == *t) {
                    let value = rec.data().to_string();
                    let value = if matches!(t, RecordType::CNAME | RecordType::NS | RecordType::MX) { value.trim_end_matches('.').to_string() } else { value };
                    out.records.push(SyntheticDnsRecord { kind: t.to_string(), value, ttl: rec.ttl() });
                }
            }
            Err(e) if matches!(e.kind(), ResolveErrorKind::Proto(p) if p.is_no_records_found()) => {}
            Err(e) => failures.push(format!("{t}: {e}")),
        }
    }
    if out.records.is_empty() && !failures.is_empty() {
        return Err(format!("DNS lookup failed ({})", failures.join("; ")));
    }
    if out.records.is_empty() {
        return Err(format!("{domain} has no A, AAAA, CNAME, MX, TXT or NS records"));
    }
    Ok(())
}

async fn run_tcp(r: &SyntheticRequest, allow_private: bool, out: &mut SyntheticRunDetail) -> Result<(), String> {
    let port = r.port.ok_or("TCP tests need a port")?;
    let addr = resolve(r.host.trim(), port, allow_private).await?;
    out.resolved_ip = Some(addr.ip().to_string());
    let started = Instant::now();
    tokio::net::TcpStream::connect(addr).await.map_err(|e| format!("Connection to {addr} failed: {e}"))?;
    out.duration_ms = started.elapsed().as_secs_f64() * 1000.0;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use SyntheticAssertionType as K;
    use SyntheticOperator as Op;

    fn a(kind: SyntheticAssertionType, operator: SyntheticOperator, property: &str, target: &str) -> SyntheticAssertion {
        SyntheticAssertion { kind, operator, property: property.into(), target: target.into() }
    }

    fn header(name: &str, value: &str) -> SyntheticHeader {
        SyntheticHeader { name: name.into(), value: value.into() }
    }

    fn record(kind: &str, value: &str) -> SyntheticDnsRecord {
        SyntheticDnsRecord { kind: kind.into(), value: value.into(), ttl: 60 }
    }

    fn passes(assertion: SyntheticAssertion, o: &Observed) -> bool {
        let r = evaluate(&assertion, o);
        assert_eq!(r.error, None, "{assertion:?}");
        r.passed
    }

    #[test]
    fn status_codes_and_response_times() {
        let o = Observed { status_code: Some(200), response_time_ms: 120.4, ..Default::default() };
        assert!(passes(a(K::StatusCode, Op::Is, "", "200"), &o));
        assert!(!passes(a(K::StatusCode, Op::IsNot, "", "200"), &o));
        assert!(passes(a(K::StatusCode, Op::LessThan, "", "300"), &o));
        assert!(passes(a(K::StatusCode, Op::Matches, "", "^2\\d\\d$"), &o));
        assert!(!passes(a(K::StatusCode, Op::DoesNotMatch, "", "^2"), &o));
        assert!(passes(a(K::ResponseTime, Op::LessThan, "", "1000"), &o));
        assert!(!passes(a(K::ResponseTime, Op::LessThan, "", "100"), &o));
        assert_eq!(evaluate(&a(K::ResponseTime, Op::LessThan, "", "100"), &o).actual.as_deref(), Some("120"));
        // No response at all: status assertions fail.
        assert!(!passes(a(K::StatusCode, Op::Is, "", "200"), &Observed::default()));
    }

    #[test]
    fn headers_match_case_insensitively_and_missing_ones_pass_only_negative_operators() {
        let headers = [header("Content-Type", "application/json; charset=utf-8"), header("x-cache", "HIT")];
        let o = Observed { headers: &headers, ..Default::default() };
        assert!(passes(a(K::Header, Op::Contains, "content-type", "application/json"), &o));
        assert!(passes(a(K::Header, Op::Is, "X-Cache", "HIT"), &o));
        assert!(!passes(a(K::Header, Op::Is, "x-cache", "MISS"), &o));
        assert!(!passes(a(K::Header, Op::Is, "x-missing", "1"), &o));
        assert!(passes(a(K::Header, Op::DoesNotContain, "x-missing", "1"), &o));
        assert_eq!(evaluate(&a(K::Header, Op::Is, "x-missing", "1"), &o).actual, None);
    }

    #[test]
    fn bodies_and_json_paths() {
        let body = r#"{"status":"ok","items":[{"id":7,"name":"cart service"}],"count":3,"flags":{"beta":true}}"#;
        let o = Observed { body: Some(body), ..Default::default() };
        assert!(passes(a(K::Body, Op::Contains, "", "\"status\":\"ok\""), &o));
        assert!(passes(a(K::Body, Op::DoesNotContain, "", "error"), &o));
        assert!(passes(a(K::Body, Op::Matches, "", "\"count\":\\d+"), &o));
        assert!(passes(a(K::BodyJsonPath, Op::Is, "$.status", "ok"), &o));
        assert!(passes(a(K::BodyJsonPath, Op::Is, "status", "ok"), &o));
        assert!(passes(a(K::BodyJsonPath, Op::Is, "$.items[0].id", "7"), &o));
        assert!(passes(a(K::BodyJsonPath, Op::Contains, "$.items[0]['name']", "cart"), &o));
        assert!(passes(a(K::BodyJsonPath, Op::MoreThan, "$.count", "2"), &o));
        assert!(passes(a(K::BodyJsonPath, Op::Is, "$.flags.beta", "true"), &o));
        assert!(!passes(a(K::BodyJsonPath, Op::Is, "$.items[3].id", "7"), &o));
        let not_json = Observed { body: Some("<html>"), ..Default::default() };
        assert_eq!(evaluate(&a(K::BodyJsonPath, Op::Is, "$.status", "ok"), &not_json).error.as_deref(), Some("The body is not JSON"));
    }

    #[test]
    fn json_paths_parse() {
        use PathStep::*;
        assert_eq!(parse_json_path("$.a.b[2]['c d'][\"e\"]"), Ok(vec![Key("a".into()), Key("b".into()), Index(2), Key("c d".into()), Key("e".into())]));
        assert_eq!(parse_json_path("a[0]"), Ok(vec![Key("a".into()), Index(0)]));
        assert_eq!(parse_json_path("$"), Ok(vec![]));
        for bad in ["", "$..a", "$.a[", "$.a[x]", "$a"] {
            assert!(parse_json_path(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn certificates_and_dns_records() {
        let o = Observed { cert_days_left: Some(42.5), ..Default::default() };
        assert!(passes(a(K::Certificate, Op::IsInMoreThan, "", "30"), &o));
        assert!(!passes(a(K::Certificate, Op::IsInMoreThan, "", "60"), &o));
        let records = [record("A", "93.184.215.14"), record("A", "93.184.215.15"), record("MX", "10 mail.example.com")];
        let o = Observed { records: &records, ..Default::default() };
        assert!(passes(a(K::RecordSome, Op::Is, "A", "93.184.215.15"), &o));
        assert!(!passes(a(K::RecordEvery, Op::Is, "A", "93.184.215.15"), &o));
        assert!(passes(a(K::RecordEvery, Op::Matches, "A", "^93\\.184\\."), &o));
        assert!(passes(a(K::RecordSome, Op::Contains, "MX", "mail.example.com"), &o));
        assert!(!passes(a(K::RecordSome, Op::Is, "AAAA", "::1"), &o), "no record of the type fails");
    }

    #[test]
    fn assertions_are_checked_for_their_test_type() {
        let http = SyntheticTestType::Http;
        assert_eq!(check_assertion(http, &a(K::StatusCode, Op::Is, "", "200")), Ok(()));
        assert!(check_assertion(http, &a(K::StatusCode, Op::Is, "", "ok")).unwrap_err().contains("not a number"));
        assert!(check_assertion(http, &a(K::Certificate, Op::IsInMoreThan, "", "10")).unwrap_err().contains("don't apply to HTTP tests"));
        assert!(check_assertion(http, &a(K::ResponseTime, Op::Contains, "", "10")).unwrap_err().contains("can't use"));
        assert!(check_assertion(http, &a(K::Header, Op::Is, " ", "x")).unwrap_err().contains("name a header"));
        assert!(check_assertion(http, &a(K::Body, Op::Matches, "", "(unclosed")).unwrap_err().contains("Invalid pattern"));
        assert!(check_assertion(http, &a(K::BodyJsonPath, Op::Is, "$..x", "1")).unwrap_err().contains("Invalid JSON path"));
        assert!(check_assertion(SyntheticTestType::Dns, &a(K::RecordSome, Op::Is, "SRV", "x")).unwrap_err().contains("record type"));
        assert_eq!(check_assertion(SyntheticTestType::Dns, &a(K::RecordSome, Op::Is, "A", "1.2.3.4")), Ok(()));
        assert_eq!(check_assertion(SyntheticTestType::Ssl, &a(K::Certificate, Op::IsInMoreThan, "", "14")), Ok(()));
        assert!(check_assertion(SyntheticTestType::Tcp, &a(K::Body, Op::Contains, "", "x")).is_err());
    }

    #[test]
    fn suggestions_follow_the_response() {
        let run = SyntheticRunDetail { status_code: Some(201), duration_ms: 730.0, response_headers: vec![header("Content-Type", "application/json")], ..Default::default() };
        assert_eq!(
            suggest(SyntheticTestType::Http, &run),
            vec![a(K::StatusCode, Op::Is, "", "201"), a(K::ResponseTime, Op::LessThan, "", "1500"), a(K::Header, Op::Is, "content-type", "application/json")]
        );
        let fast = SyntheticRunDetail { duration_ms: 12.0, records: vec![record("CNAME", "shop.example.net"), record("A", "93.184.215.14")], ..Default::default() };
        assert_eq!(suggest(SyntheticTestType::Dns, &fast), vec![a(K::ResponseTime, Op::LessThan, "", "1000"), a(K::RecordSome, Op::Is, "A", "93.184.215.14")]);
        assert_eq!(suggest(SyntheticTestType::Tcp, &fast), vec![a(K::ResponseTime, Op::LessThan, "", "1000")]);
    }

    #[test]
    fn redirects_turn_posts_into_gets_like_browsers() {
        use reqwest::Method;
        assert_eq!(after_redirect(303, &Method::POST), (Method::GET, false));
        assert_eq!(after_redirect(302, &Method::POST), (Method::GET, false));
        assert_eq!(after_redirect(301, &Method::PUT), (Method::PUT, true));
        assert_eq!(after_redirect(307, &Method::POST), (Method::POST, true));
        assert_eq!(after_redirect(308, &Method::GET), (Method::GET, true));
        let u = |s: &str| url::Url::parse(s).unwrap();
        assert!(same_origin(&u("https://shop.example.com/a"), &u("https://shop.example.com:443/b")));
        assert!(!same_origin(&u("https://shop.example.com/a"), &u("https://cdn.example.com/a")));
        assert!(!same_origin(&u("https://shop.example.com/a"), &u("http://shop.example.com/a")));
    }

    #[test]
    fn previews_cut_on_a_character_boundary() {
        let long = "é".repeat(BODY_PREVIEW_BYTES);
        let p = preview(&long);
        assert!(p.len() <= BODY_PREVIEW_BYTES && p.len() >= BODY_PREVIEW_BYTES - 1);
        assert_eq!(preview("short"), "short");
    }

    #[tokio::test]
    async fn private_targets_are_refused_before_connecting() {
        let req = SyntheticRequest { url: "http://127.0.0.1:9/health".into(), ..Default::default() };
        let r = run(SyntheticTestType::Http, &req, &[], false).await;
        assert!(!r.passed);
        assert!(r.error.as_deref().unwrap().contains("loopback"), "{:?}", r.error);
        let tcp = SyntheticRequest { host: "169.254.169.254".into(), port: Some(80), ..Default::default() };
        let r = run(SyntheticTestType::Tcp, &tcp, &[], false).await;
        assert!(r.error.as_deref().unwrap().contains("link-local"), "{:?}", r.error);
        let dns = SyntheticRequest { host: "example.com".into(), dns_server: "10.0.0.2".into(), ..Default::default() };
        let r = run(SyntheticTestType::Dns, &dns, &[], false).await;
        assert!(r.error.as_deref().unwrap().contains("private"), "{:?}", r.error);
        let ftp = SyntheticRequest { url: "ftp://example.com/file".into(), ..Default::default() };
        assert!(run(SyntheticTestType::Http, &ftp, &[], true).await.error.unwrap().contains("Only http and https"));
    }

    /// A local server (allowed with the operator setting): redirects are followed and checked, bodies capped.
    #[tokio::test]
    async fn http_runs_against_a_local_server() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            loop {
                let Ok((mut s, _)) = listener.accept().await else { return };
                tokio::spawn(async move {
                    let mut buf = vec![0u8; 4096];
                    let n = s.read(&mut buf).await.unwrap_or(0);
                    let req = String::from_utf8_lossy(&buf[..n]).to_string();
                    let resp = if req.starts_with("GET /old") {
                        "HTTP/1.1 302 Found\r\nLocation: /health\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_string()
                    } else if req.starts_with("GET /away") {
                        "HTTP/1.1 302 Found\r\nLocation: ftp://files.example.com/latest\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_string()
                    } else if req.starts_with("GET /big") {
                        let body = "x".repeat(MAX_BODY_BYTES + 10);
                        format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len())
                    } else {
                        let body = r#"{"status":"ok"}"#;
                        format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len())
                    };
                    let _ = s.write_all(resp.as_bytes()).await;
                });
            }
        });
        let req = |path: &str| SyntheticRequest { url: format!("http://127.0.0.1:{port}{path}"), ..Default::default() };
        let checks = [a(K::StatusCode, Op::Is, "", "200"), a(K::BodyJsonPath, Op::Is, "$.status", "ok")];
        let r = run(SyntheticTestType::Http, &req("/old"), &checks, true).await;
        assert!(r.passed, "{r:?}");
        assert_eq!(r.redirects, vec![format!("http://127.0.0.1:{port}/health")]);
        assert_eq!(r.body.as_deref(), Some(r#"{"status":"ok"}"#));
        // Every hop is checked again (scheme, then address).
        let r = run(SyntheticTestType::Http, &req("/away"), &checks, true).await;
        assert!(r.error.unwrap().contains("Only http and https"));
        let r = run(SyntheticTestType::Http, &req("/old"), &checks, false).await;
        assert!(r.error.unwrap().contains("loopback"), "refused without the setting");
        let r = run(SyntheticTestType::Http, &SyntheticRequest { follow_redirects: false, ..req("/old") }, &checks, true).await;
        assert_eq!(r.status_code, Some(302));
        assert!(!r.passed);
        let r = run(SyntheticTestType::Http, &req("/big"), &[], true).await;
        assert!(r.body_truncated && r.body_size == Some(MAX_BODY_BYTES as u64), "{:?}", r.body_size);
        let tcp = SyntheticRequest { host: "127.0.0.1".into(), port: Some(port), ..Default::default() };
        assert!(run(SyntheticTestType::Tcp, &tcp, &[a(K::ResponseTime, Op::LessThan, "", "5000")], true).await.passed);
    }
}
