//! Runs a package: one stream for one connection ([`run`]), or the connection test ([`test`]). A run reads pages
//! through the [`Transport`], keeps the records its incremental strategy hasn't sent yet, maps them
//! ([`crate::output`]) and returns them with the stream's next state; the caller publishes them and saves the state
//! only once they are stored, so a failed publish reads the same records again.
//!
//! - **Pages**: [`Pager`] follows the stream's pagination; a next page never leaves the first request's host.
//! - **Time windows**: from the cursor minus `overlap` (the first run: `lookback`) to now minus `settle`, cut into
//!   `slice`s read oldest first. A window read to its end moves the cursor. One left unread (out of pages, quota, a
//!   failed page) carries on: `newest_first` / `unordered` windows from the page they stopped at (the backlog),
//!   `oldest_first` ones from their last record. Records already sent (their ids, kept while inside the overlap) are
//!   skipped, so no record is lost or sent twice by the engine.
//! - **Errors**: a refused first request fails the run (nothing is published, the job backs off); a failure after
//!   some pages keeps what was read and saves the progress, with the error as the run's warning.

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use crate::auth::{self, Cache, Prepared};
use crate::expr::{text, to_ms, truthy};
use crate::http::{parse_body, ClientTls, HttpRequest, HttpResponse, Transport};
use crate::manifest::{duration_ms, AsyncExport, Auth, CursorValue, ErrorRule, Incremental, IssueKind, Manifest, Order, Pagination, RateLimit, Request, Stream};
use crate::output::{id_hash, Item, Mapped, Mapper};
use crate::template::{Template, Tree};

pub const USER_AGENT: &str = "ObserveCraft-Connectors/1.0";
/// Pages a run reads unless the stream says otherwise.
pub const MAX_PAGES: u32 = 20;
/// Ids remembered by `new_records` unless the stream says otherwise.
pub const MAX_SEEN: usize = 100_000;

/// A connection as the engine sees it: its configuration and unsealed secrets.
#[derive(Clone, Debug, Default)]
pub struct Connection {
    pub id: String,
    pub name: String,
    pub config: Map<String, Value>,
    pub secrets: Map<String, Value>,
}

/// What identifies the connected account (`connection.instance`): the manifest's `instance` over `config`, else the
/// connector id.
pub fn instance(m: &Manifest, config: &Map<String, Value>) -> String {
    let mut scope = Map::new();
    scope.insert("config".into(), Value::Object(config.clone()));
    scope.insert("tables".into(), json!(m.tables));
    m.instance.as_deref().and_then(|t| Template::parse(t).ok()).map(|t| t.text(&scope)).filter(|s| !s.is_empty()).unwrap_or_else(|| m.id.clone())
}

/// `config` with the manifest's defaults for the fields left out or empty.
pub fn effective_config(m: &Manifest, config: &Map<String, Value>) -> Map<String, Value> {
    let mut out = config.clone();
    for f in m.config.iter().filter(|f| !f.secret) {
        if let Some(d) = &f.default {
            if out.get(&f.key).is_none_or(|v| v.is_null() || v.as_str().is_some_and(str::is_empty)) {
                out.insert(f.key.clone(), d.clone());
            }
        }
    }
    out
}

/// What runs share: the transport, cached tokens / lookups / discoveries, and the pacing of each connection.
pub struct Runtime {
    pub transport: Arc<dyn Transport>,
    pub cache: Arc<Cache>,
    /// Sleep between requests as rate limits ask (off in replay tests).
    pub pace: bool,
    last: Mutex<HashMap<String, Instant>>,
}

impl Runtime {
    pub fn new(transport: Arc<dyn Transport>, pace: bool) -> Runtime {
        Runtime { transport, cache: Arc::new(Cache::default()), pace, last: Mutex::default() }
    }

    /// Forgets a connection's tokens and lookups (its credentials changed).
    pub fn forget(&self, connection: &str) {
        self.cache.forget_prefix(&format!("{connection}\0"));
    }
}

/// A run's problem.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Issue {
    pub kind: IssueKind,
    pub message: String,
}

impl Issue {
    fn other(message: impl Into<String>) -> Issue {
        Issue { kind: IssueKind::Other, message: message.into() }
    }
}

/// A record id remembered, with its time when it has one.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Seen {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub t: Option<i64>,
}

/// A time window not read to its end.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Backlog {
    pub start: i64,
    pub end: i64,
    /// Where its pages carry on.
    pub page: Value,
    #[serde(default)]
    pub max_time: Option<i64>,
}

/// An asynchronous export in progress.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Export {
    pub id: Value,
    #[serde(default)]
    pub start: Option<i64>,
    #[serde(default)]
    pub end: Option<i64>,
    /// Chunks already read.
    #[serde(default)]
    pub done: Vec<String>,
    pub created: i64,
}

/// One partition's state (a stream, or one of its `for_each` items).
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Partition {
    /// Epoch ms: time windows start here (minus the overlap).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backlog: Option<Backlog>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub seen: Vec<Seen>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub export: Option<Export>,
}

/// A stream's state, by partition (`""` without `for_each`).
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct StreamState {
    #[serde(default)]
    pub partitions: BTreeMap<String, Partition>,
}

impl StreamState {
    /// Where the stream is read up to: its partitions' oldest cursor.
    pub fn collected_until(&self) -> Option<i64> {
        self.partitions.values().filter_map(|p| p.cursor).min()
    }
}

/// What a run read.
#[derive(Debug, Default)]
pub struct Run {
    pub records: usize,
    pub mapped: Mapped,
    pub state: StreamState,
    pub requests: u32,
    pub warning: Option<Issue>,
    /// Rotated refresh tokens to save: (secret field, value).
    pub rotated: Vec<(String, String)>,
}

/// One fetched page.
struct Page {
    response: Value,
    headers: Value,
    records: Vec<Value>,
}


/// Where the next page is.
#[derive(Clone, Debug, PartialEq)]
enum PageAt {
    Start,
    Cursor(Value),
    Url(String),
    Offset(u64),
    Number(u64),
}

impl PageAt {
    fn to_value(&self) -> Value {
        match self {
            PageAt::Start => Value::Null,
            PageAt::Cursor(c) => json!({ "cursor": c }),
            PageAt::Url(u) => json!({ "url": u }),
            PageAt::Offset(o) => json!({ "offset": o }),
            PageAt::Number(n) => json!({ "number": n }),
        }
    }

    fn from_value(v: &Value) -> PageAt {
        if let Some(c) = v.get("cursor") {
            PageAt::Cursor(c.clone())
        } else if let Some(u) = v.get("url").and_then(Value::as_str) {
            PageAt::Url(u.to_string())
        } else if let Some(o) = v.get("offset").and_then(Value::as_u64) {
            PageAt::Offset(o)
        } else if let Some(n) = v.get("number").and_then(Value::as_u64) {
            PageAt::Number(n)
        } else {
            PageAt::Start
        }
    }
}

struct Pager<'a> {
    req: &'a Request,
    pagination: &'a Pagination,
    records: Option<Template>,
    at: PageAt,
    origin: Option<String>,
    done: bool,
    /// The next offset is past the API's maximum.
    capped: bool,
}

fn as_list(v: Value) -> Vec<Value> {
    match v {
        Value::Array(a) => a,
        Value::Null => vec![],
        other => vec![other],
    }
}

fn missing(v: &Value) -> bool {
    v.is_null() || v.as_str().is_some_and(str::is_empty)
}

impl<'a> Pager<'a> {
    fn new(req: &'a Request, pagination: &'a Pagination, records: &Option<String>, at: PageAt) -> Result<Pager<'a>, Issue> {
        let records = records.as_deref().map(Template::parse).transpose().map_err(Issue::other)?;
        Ok(Pager { req, pagination, records, at, origin: None, done: false, capped: false })
    }

    fn page_vars(&self) -> Value {
        let size = match self.pagination {
            Pagination::Offset { size, .. } | Pagination::Page { size, .. } => json!(size),
            _ => Value::Null,
        };
        match (&self.at, self.pagination) {
            (PageAt::Cursor(c), _) => json!({ "cursor": c, "size": size }),
            (PageAt::Offset(o), _) => json!({ "offset": o, "size": size }),
            (PageAt::Number(n), _) => json!({ "number": n, "size": size }),
            (PageAt::Start, Pagination::Offset { start, .. }) => json!({ "offset": start, "size": size }),
            (PageAt::Start, Pagination::Page { start, .. }) => json!({ "number": start, "size": size }),
            _ => json!({ "size": size }),
        }
    }

    async fn fetch(&mut self, cx: &mut Cx<'_>, scope: &Map<String, Value>, errors: &[ErrorRule]) -> Result<Page, Issue> {
        let mut s = scope.clone();
        s.insert("page".into(), self.page_vars());
        let mut req = cx.build(self.req, &s)?;
        if let PageAt::Url(u) = &self.at {
            req.url = u.clone();
            if let Pagination::NextUrl { method: Some(method), .. } = self.pagination {
                if *method != req.method {
                    req.method = *method;
                    req.body = None;
                    req.headers.retain(|(k, _)| !k.eq_ignore_ascii_case("content-type"));
                }
            }
            cx.reapply(&mut req)?;
        }
        let origin = crate::request::origin(&req.url);
        match &self.origin {
            None => self.origin = origin,
            Some(first) if Some(first) != origin.as_ref() => return Err(Issue::other(format!("the next page is on another host ({})", origin.unwrap_or_default()))),
            Some(_) => {}
        }
        let resp = cx.send(&req, errors, &s).await?;
        let response = parse_body(&resp.body, self.req.format).map_err(Issue::other)?;
        let headers = resp.headers_value();
        s.insert("response".into(), response.clone());
        s.insert("headers".into(), headers.clone());
        s.insert("status".into(), json!(resp.status));
        let records = as_list(match &self.records {
            Some(t) => t.render(&s),
            None => response.clone(),
        });
        s.insert("records".into(), Value::Array(records.clone()));
        s.insert("last".into(), records.last().cloned().unwrap_or(Value::Null));
        let render = |t: &str| Template::parse(t).map(|t| t.render(&s)).map_err(Issue::other);
        let next = match self.pagination {
            Pagination::None | Pagination::AsyncExport(_) => None,
            Pagination::Cursor { next } => {
                let c = render(next)?;
                (!missing(&c) && !records.is_empty()).then_some(PageAt::Cursor(c))
            }
            Pagination::NextUrl { next, .. } => {
                let u = text(&render(next)?);
                (!u.is_empty() && !records.is_empty()).then(|| crate::request::resolve(&req.url, &u)).flatten().map(PageAt::Url)
            }
            Pagination::LinkHeader => resp.header("link").and_then(crate::http::link_next).and_then(|u| crate::request::resolve(&req.url, &u)).map(PageAt::Url),
            Pagination::Offset { size, start, total, max } => {
                let current = match self.at {
                    PageAt::Offset(o) => o,
                    _ => *start,
                };
                let next = current + size;
                let total = total.as_deref().map(render).transpose()?.and_then(|t| t.as_u64());
                if (records.len() as u64) < *size || total.is_some_and(|t| next >= t) {
                    None
                } else if max.is_some_and(|m| next > m) {
                    self.capped = true;
                    None
                } else {
                    Some(PageAt::Offset(next))
                }
            }
            Pagination::Page { size, start, total_pages } => {
                let current = match self.at {
                    PageAt::Number(n) => n,
                    _ => *start,
                };
                let total = total_pages.as_deref().map(render).transpose()?.and_then(|t| t.as_u64());
                if (records.len() as u64) < *size || total.is_some_and(|t| current + 1 > (t + start).saturating_sub(1)) {
                    None
                } else {
                    Some(PageAt::Number(current + 1))
                }
            }
        };
        match next {
            Some(at) if at != self.at => self.at = at,
            _ => self.done = true,
        }
        Ok(Page { response, headers, records })
    }
}

/// A run's context: the connection, its credentials, its scope and what it used.
struct Cx<'a> {
    m: &'a Manifest,
    conn: &'a Connection,
    rt: &'a Runtime,
    now_ms: i64,
    scope: Map<String, Value>,
    prepared: Prepared,
    extra_headers: Vec<(String, String)>,
    tls: Option<ClientTls>,
    rate: Option<RateLimit>,
    budget: Option<u32>,
    requests: u32,
    secrets: Vec<String>,
    rotated: Vec<(String, String)>,
    /// An error rule with `reset` matched: the partition's position is dropped.
    reset: std::sync::atomic::AtomicBool,
}

fn default_message(status: u16, name: &str) -> (IssueKind, String) {
    match status {
        401 => (IssueKind::Auth, format!("{name} refused the credentials (HTTP 401)")),
        403 => (IssueKind::AccessDenied, format!("{name} refused access (HTTP 403): the credentials lack a permission, or the plan doesn't include this API")),
        404 => (IssueKind::AccessDenied, format!("Not found at this URL (HTTP 404): check the {name} URL and the account's plan")),
        429 => (IssueKind::Throttled, format!("{name} is rate limiting requests (HTTP 429)")),
        s => (IssueKind::Other, format!("{name} answered HTTP {s}")),
    }
}

/// The vendor's own error text in an answer, shortened.
fn vendor_message(body: &Value) -> Option<String> {
    let pick = |v: &Value| -> Option<String> {
        match v {
            Value::String(s) if !s.is_empty() => Some(s.clone()),
            Value::Object(o) => ["message", "detail", "error_description", "description"].iter().find_map(|k| o.get(*k).and_then(Value::as_str).map(str::to_string)),
            Value::Array(a) => a.first().and_then(|e| e.get("message").or(e.get("detail")).and_then(Value::as_str).map(str::to_string)),
            _ => None,
        }
    };
    ["message", "error", "errors", "detail", "error_description", "title"].iter().find_map(|k| body.get(*k).and_then(pick)).map(|m| m.chars().take(200).collect())
}

impl<'a> Cx<'a> {
    async fn new(m: &'a Manifest, stream: Option<&'a Stream>, conn: &'a Connection, rt: &'a Runtime, now_ms: i64, budget: Option<u32>) -> Result<Cx<'a>, Issue> {
        let mut scope = Map::new();
        scope.insert("config".into(), Value::Object(conn.config.clone()));
        scope.insert("secrets".into(), Value::Object(conn.secrets.clone()));
        scope.insert("connection".into(), json!({ "id": conn.id, "name": conn.name, "instance": instance(m, &conn.config) }));
        scope.insert("tables".into(), json!(m.tables));
        scope.insert("now".into(), json!(now_ms));
        scope.insert("minute".into(), json!(now_ms - now_ms.rem_euclid(60_000)));
        scope.insert("source".into(), json!(m.source.clone().unwrap_or_else(|| m.id.clone())));
        let mut secrets: Vec<String> = conn.secrets.values().map(text).filter(|s| s.len() >= 4).collect();
        let tls = match &m.tls {
            Some(t) => {
                let render = |s: &str| Template::parse(s).map(|t| t.text(&scope)).map_err(Issue::other);
                Some(ClientTls { certificate: render(&t.certificate)?, key: render(&t.key)? })
            }
            None => None,
        };
        let mut cx = Cx {
            m,
            conn,
            rt,
            now_ms,
            scope,
            prepared: Prepared::default(),
            extra_headers: vec![("User-Agent".into(), USER_AGENT.into())],
            tls,
            rate: stream.and_then(|s| s.rate_limit.clone()).or_else(|| m.rate_limit.clone()),
            budget,
            requests: 0,
            secrets: Vec::new(),
            rotated: Vec::new(),
            reset: std::sync::atomic::AtomicBool::new(false),
        };
        let (auth, key): (&Auth, String) = match stream.and_then(|s| s.auth.as_ref()) {
            Some(a) => (a, format!("{}\0{}", conn.id, stream.map(|s| s.id.as_str()).unwrap_or_default())),
            None => (&m.auth, conn.id.clone()),
        };
        let auth = auth.chosen(&conn.config).map_err(|message| Issue { kind: IssueKind::Auth, message })?;
        let prepared = auth::prepare(auth, &cx.scope, &key, rt.transport.as_ref(), &rt.cache, now_ms).await.map_err(|e| Issue { kind: IssueKind::Auth, message: cx.scrub_with(&e, &secrets) })?;
        secrets.extend(prepared.secrets());
        cx.secrets = secrets;
        if let Some(r) = &prepared.rotated {
            cx.rotated.push(r.clone());
        }
        cx.scope.insert("auth".into(), Value::Object(prepared.values.clone()));
        cx.prepared = prepared;
        for (k, v) in &m.headers {
            let value = Template::parse(v).map(|t| t.text(&cx.scope)).map_err(Issue::other)?;
            cx.extra_headers.push((k.clone(), value));
        }
        cx.discover().await?;
        Ok(cx)
    }

    fn scrub_with(&self, message: &str, secrets: &[String]) -> String {
        let mut out = message.to_string();
        for s in secrets {
            out = out.replace(s.as_str(), "[secret]");
        }
        out
    }

    fn scrub(&self, message: &str) -> String {
        self.scrub_with(message, &self.secrets)
    }

    async fn discover(&mut self) -> Result<(), Issue> {
        let mut found = Map::new();
        for d in &self.m.discover {
            let key = format!("{}\0discovery\0{}", self.conn.id, d.name);
            let value = match self.rt.cache.get(&key, self.now_ms) {
                Some(v) => v,
                None => {
                    let mut s = self.scope.clone();
                    s.insert("discovery".into(), Value::Object(found.clone()));
                    let paged = !matches!(d.pagination, Pagination::None);
                    if paged {
                        // Every page's records, as `records`; the last page's answer as `response`.
                        let mut pager = Pager::new(&d.request, &d.pagination, &d.records, PageAt::Start)?;
                        let (mut all, mut pages) = (Vec::new(), 0);
                        let scope = s.clone();
                        while !pager.done && pages < d.max_pages.unwrap_or(MAX_PAGES) {
                            let page = pager.fetch(self, &scope, &[]).await?;
                            pages += 1;
                            s.insert("response".into(), page.response);
                            s.insert("headers".into(), page.headers);
                            all.extend(page.records);
                        }
                        s.insert("records".into(), Value::Array(all));
                    } else {
                        let req = self.build(&d.request, &s)?;
                        let resp = self.send(&req, &[], &s).await?;
                        s.insert("response".into(), parse_body(&resp.body, d.request.format).map_err(Issue::other)?);
                        s.insert("headers".into(), resp.headers_value());
                    }
                    let v = match &d.value {
                        Some(t) => Template::parse(t).map_err(Issue::other)?.render(&s),
                        None if paged => s["records"].clone(),
                        None => s["response"].clone(),
                    };
                    let ttl = d.ttl.as_deref().map(duration_ms).transpose().map_err(Issue::other)?.unwrap_or(3_600_000);
                    self.rt.cache.put(&key, v.clone(), self.now_ms + ttl);
                    v
                }
            };
            found.insert(d.name.clone(), value);
        }
        self.scope.insert("discovery".into(), Value::Object(found));
        Ok(())
    }

    fn build(&self, req: &Request, scope: &Map<String, Value>) -> Result<HttpRequest, Issue> {
        let mut out = crate::request::build(req, scope, &self.extra_headers).map_err(|e| Issue::other(self.scrub(&e)))?;
        self.reapply(&mut out)?;
        Ok(out)
    }

    fn reapply(&self, req: &mut HttpRequest) -> Result<(), Issue> {
        self.prepared.apply(req, self.now_ms).map_err(|e| Issue { kind: IssueKind::Auth, message: self.scrub(&e) })?;
        req.tls = self.tls.clone();
        Ok(())
    }

    async fn pace(&self) {
        let Some(rate) = &self.rate else { return };
        let interval = match (rate.min_interval.as_deref().and_then(|d| duration_ms(d).ok()), rate.requests_per_minute) {
            (Some(ms), _) => ms,
            (None, Some(rpm)) if rpm > 0 => 60_000 / rpm as i64,
            _ => return,
        };
        let wait = {
            let mut last = self.rt.last.lock().unwrap();
            let now = Instant::now();
            let next = last.get(&self.conn.id).map_or(now, |t| (*t + Duration::from_millis(interval as u64)).max(now));
            last.insert(self.conn.id.clone(), next);
            next.saturating_duration_since(now)
        };
        if self.rt.pace && !wait.is_zero() {
            tokio::time::sleep(wait).await;
        }
    }

    /// Sends a request; a failed answer becomes the issue its error rules (then the manifest's, then the defaults)
    /// make of it.
    async fn send(&mut self, req: &HttpRequest, errors: &[ErrorRule], scope: &Map<String, Value>) -> Result<HttpResponse, Issue> {
        let resp = self.send_raw(req).await?;
        if (200..300).contains(&resp.status) {
            match self.failure_2xx(&resp, errors, scope) {
                Some(issue) => Err(issue),
                None => Ok(resp),
            }
        } else {
            Err(self.failure(&resp, errors, scope))
        }
    }

    /// An error in a successful answer (GraphQL's `errors` with 200): a rule naming the 2xx status with a `when`.
    fn failure_2xx(&self, resp: &HttpResponse, errors: &[ErrorRule], scope: &Map<String, Value>) -> Option<Issue> {
        let mut rules = errors.iter().chain(&self.m.errors).filter(|r| r.when.is_some() && r.status.contains(&resp.status)).peekable();
        rules.peek()?;
        let body: Value = serde_json::from_slice(&resp.body).unwrap_or(Value::Null);
        let mut s = scope.clone();
        s.insert("response".into(), body);
        s.insert("status".into(), json!(resp.status));
        s.insert("headers".into(), resp.headers_value());
        let rule = rules.find(|r| r.when.as_deref().and_then(|w| Template::parse(w).ok()).is_some_and(|t| truthy(&t.render(&s))))?;
        let message = Template::parse(&rule.message).map(|t| t.text(&s)).unwrap_or_default();
        Some(Issue { kind: rule.kind, message: self.scrub(&message) })
    }

    /// Sends a request within the quota and the pacing; any answer is returned.
    async fn send_raw(&mut self, req: &HttpRequest) -> Result<HttpResponse, Issue> {
        if let Some(left) = self.budget.as_mut() {
            if *left == 0 {
                return Err(Issue { kind: IssueKind::Throttled, message: "the connection's request quota is used up for now".into() });
            }
            *left -= 1;
        }
        self.pace().await;
        self.requests += 1;
        self.rt.transport.send(req).await.map_err(|e| Issue::other(self.scrub(&e)))
    }

    /// The issue a failed answer is.
    fn failure(&self, resp: &HttpResponse, errors: &[ErrorRule], scope: &Map<String, Value>) -> Issue {
        let body: Value = serde_json::from_slice(&resp.body).unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&resp.body).chars().take(200).collect()));
        let mut s = scope.clone();
        s.insert("response".into(), body.clone());
        s.insert("status".into(), json!(resp.status));
        s.insert("headers".into(), resp.headers_value());
        for rule in errors.iter().chain(&self.m.errors) {
            if !rule.status.is_empty() && !rule.status.contains(&resp.status) {
                continue;
            }
            if let Some(w) = &rule.when {
                if !Template::parse(w).is_ok_and(|t| truthy(&t.render(&s))) {
                    continue;
                }
            }
            let message = Template::parse(&rule.message).map(|t| t.text(&s)).unwrap_or_default();
            if rule.reset {
                self.reset.store(true, std::sync::atomic::Ordering::Relaxed);
            }
            return Issue { kind: rule.kind, message: self.scrub(&message) };
        }
        let (kind, mut message) = default_message(resp.status, &self.m.name);
        if let Some(detail) = vendor_message(&body) {
            message.push_str(&format!(": {detail}"));
        }
        if resp.status == 429 {
            if let Some(after) = resp.header("retry-after") {
                message.push_str(&format!(" (retry after {after}s)"));
            }
        }
        Issue { kind, message: self.scrub(&message) }
    }

    /// The stream's lookups that read `item` (`per_item`) or don't, into `lookups` of `scope`.
    async fn lookups(&mut self, stream: &Stream, scope: &mut Map<String, Value>, per_item: bool) -> Result<(), Issue> {
        let mut out = scope.get("lookups").and_then(Value::as_object).cloned().unwrap_or_default();
        let item_key = scope.get("item").map(partition_key).unwrap_or_default();
        for l in &stream.lookups {
            if reads_item(l) != per_item {
                continue;
            }
            if let Some(w) = &l.when {
                if !truthy(&Template::parse(w).map_err(Issue::other)?.render(scope)) {
                    out.insert(l.name.clone(), Value::Object(Map::new()));
                    continue;
                }
            }
            let key = format!("{}\0lookup\0{}\0{}\0{}", self.conn.id, stream.id, l.name, if per_item { item_key.as_str() } else { "" });
            let table = match self.rt.cache.get(&key, self.now_ms) {
                Some(v) => v,
                None => {
                    let mut table = Map::new();
                    let key_t = Template::parse(&l.key).map_err(Issue::other)?;
                    let value_t = l.value.as_deref().map(Template::parse).transpose().map_err(Issue::other)?;
                    let scope = scope.clone();
                    let mut pager = Pager::new(&l.request, &l.pagination, &l.records, PageAt::Start)?;
                    let mut pages = 0;
                    while !pager.done && pages < l.max_pages.unwrap_or(MAX_PAGES) {
                        let page = pager.fetch(self, &scope, &stream.errors).await?;
                        pages += 1;
                        for r in page.records {
                            let mut s = scope.clone();
                            s.insert("record".into(), r.clone());
                            let k = key_t.text(&s);
                            if !k.is_empty() {
                                table.insert(k, value_t.as_ref().map_or(r, |t| t.render(&s)));
                            }
                        }
                    }
                    let v = Value::Object(table);
                    let ttl = l.ttl.as_deref().map(duration_ms).transpose().map_err(Issue::other)?.unwrap_or(3_600_000);
                    self.rt.cache.put(&key, v.clone(), self.now_ms + ttl);
                    v
                }
            };
            out.insert(l.name.clone(), table);
        }
        scope.insert("lookups".into(), Value::Object(out));
        Ok(())
    }
}

/// Whether a lookup's request (or `when`) reads the `for_each` item: it is then read per item.
fn reads_item(l: &crate::manifest::Lookup) -> bool {
    let mut roots = std::collections::BTreeSet::new();
    if let Ok(t) = serde_json::to_value(&l.request).as_ref().map_err(|_| ()).and_then(|v| Tree::parse(v).map_err(|_| ())) {
        t.roots(&mut roots);
    }
    if let Some(t) = l.when.as_deref().and_then(|w| Template::parse(w).ok()) {
        t.roots(&mut roots);
    }
    roots.contains("item")
}

/// A stream's per-record templates, compiled.
struct RecordFns {
    id: Option<Template>,
    filter: Option<Template>,
    vars: Vec<(String, Template)>,
    time: Option<Template>,
    mapper: Mapper,
}

impl RecordFns {
    fn new(m: &Manifest, stream: &Stream) -> Result<RecordFns, Issue> {
        let p = |t: &Option<String>| t.as_deref().map(Template::parse).transpose().map_err(Issue::other);
        Ok(RecordFns {
            id: p(&stream.record_id)?,
            filter: p(&stream.filter)?,
            vars: stream.vars.iter().map(|(k, v)| Ok((k.clone(), Template::parse(v).map_err(Issue::other)?))).collect::<Result<_, Issue>>()?,
            time: match &stream.incremental {
                Incremental::Time(t) => Some(Template::parse(&t.record_time).map_err(Issue::other)?),
                _ => None,
            },
            mapper: Mapper::compile(m, stream).map_err(Issue::other)?,
        })
    }
}

/// A native connector's records (`native:`, SQS files, WebSocket frames) mapped as a package stream maps them: its
/// `record_id`, `filter`, `vars` and `output` over the record, with `parent` (what the record came in: an S3 object)
/// and the connection's `config`; never its secrets.
pub struct Records {
    f: RecordFns,
    scope: Map<String, Value>,
    instance: String,
    now_ms: i64,
}

impl Records {
    pub fn new(m: &Manifest, stream: &Stream, conn: &Connection, now_ms: i64) -> Result<Records, Issue> {
        let mut scope = Map::new();
        scope.insert("config".into(), Value::Object(conn.config.clone()));
        scope.insert("connection".into(), json!({ "id": conn.id, "name": conn.name, "instance": instance(m, &conn.config) }));
        scope.insert("tables".into(), json!(m.tables));
        scope.insert("now".into(), json!(now_ms));
        scope.insert("minute".into(), json!(now_ms - now_ms.rem_euclid(60_000)));
        scope.insert("source".into(), json!(m.source.clone().unwrap_or_else(|| m.id.clone())));
        Ok(Records { f: RecordFns::new(m, stream)?, scope, instance: instance(m, &conn.config), now_ms })
    }

    /// Maps one record into `out`; false when the stream's filter skips it.
    pub fn map(&self, record: Value, parent: Option<&Value>, out: &mut Mapped) -> bool {
        let record = match parent {
            Some(p) => json!({ EXPANDED: true, "parent": p, "record": record }),
            None => record,
        };
        let Some(rec) = prepare_record(&self.f, &self.scope, record) else { return false };
        self.f.mapper.map(&Item { scope: &rec.scope, time_ms: rec.time, id: &rec.id, instance: &self.instance, now_ms: self.now_ms }, out);
        true
    }
}

/// A record in its scope, with its id and time; None when the filter skips it.
struct Rec {
    scope: Map<String, Value>,
    id: String,
    time: Option<i64>,
}

fn prepare_record(f: &RecordFns, scope: &Map<String, Value>, record: Value) -> Option<Rec> {
    let mut s = scope.clone();
    // Mappings never see credentials.
    s.remove("secrets");
    s.remove("auth");
    // An expanded record carries its index entry.
    let record = match record {
        Value::Object(mut o) if o.contains_key(EXPANDED) => {
            s.insert("parent".into(), o.remove("parent").unwrap_or(Value::Null));
            o.remove("record").unwrap_or(Value::Null)
        }
        other => other,
    };
    let id = match &f.id {
        Some(t) => {
            s.insert("record".into(), record.clone());
            t.text(&s)
        }
        None => record.to_string(),
    };
    s.insert("record".into(), record);
    if f.filter.as_ref().is_some_and(|t| !truthy(&t.render(&s))) {
        return None;
    }
    if !f.vars.is_empty() {
        let vars: Map<String, Value> = f.vars.iter().map(|(k, t)| (k.clone(), t.render(&s))).collect();
        s.insert("vars".into(), Value::Object(vars));
    }
    // After the vars: a record's time may be one of them.
    let time = f.time.as_ref().and_then(|t| to_ms(&t.render(&s)));
    Some(Rec { scope: s, id, time })
}

struct Emit<'r> {
    f: &'r RecordFns,
    instance: String,
    now_ms: i64,
}

impl Emit<'_> {
    fn emit(&self, rec: &Rec, out: &mut Run) {
        self.f.mapper.map(&Item { scope: &rec.scope, time_ms: rec.time, id: &rec.id, instance: &self.instance, now_ms: self.now_ms }, &mut out.mapped);
        out.records += 1;
    }
}

/// Before index entries are downloaded (`expand`): the entries whose id and time read only `parent` (the entry) are
/// checked with `skip(id, time)` first, so records sent before or outside the window aren't requested again. `hit`
/// is set when one is skipped. Without `expand`, or templates reading the downloaded `record`, nothing is skipped.
fn before_expand(f: &RecordFns, stream: &Stream, scope: &Map<String, Value>, entries: Vec<Value>, skip: impl Fn(&str, Option<i64>) -> bool, hit: &mut bool) -> Vec<Value> {
    let reads_parent_only = |t: &Option<Template>| {
        t.as_ref().is_none_or(|t| {
            let mut roots = std::collections::BTreeSet::new();
            t.roots(&mut roots);
            !roots.contains("record")
        })
    };
    if stream.expand.is_none() || f.id.is_none() || !reads_parent_only(&f.id) || !reads_parent_only(&f.time) {
        return entries;
    }
    entries
        .into_iter()
        .filter(|entry| {
            let mut s = scope.clone();
            s.insert("parent".into(), entry.clone());
            let id = f.id.as_ref().map(|t| t.text(&s)).unwrap_or_default();
            let time = f.time.as_ref().and_then(|t| to_ms(&t.render(&s)));
            let skipped = skip(&id, time);
            *hit |= skipped;
            !skipped
        })
        .collect()
}

/// Marks an expanded record (`{EXPANDED: true, parent, record}`) between [`expand`] and [`prepare_record`].
const EXPANDED: &str = "\u{0}expanded";

/// Index-then-download: each index entry's request read, its records in its place (with the entry as `parent`).
async fn expand(cx: &mut Cx<'_>, stream: &Stream, scope: &Map<String, Value>, records: Vec<Value>) -> Result<Vec<Value>, Issue> {
    let Some(x) = &stream.expand else { return Ok(records) };
    let records_t = x.records.as_deref().map(Template::parse).transpose().map_err(Issue::other)?;
    let mut out = Vec::new();
    for parent in records {
        let mut s = scope.clone();
        s.insert("parent".into(), parent.clone());
        let req = cx.build(&x.request, &s)?;
        let resp = cx.send(&req, &stream.errors, &s).await?;
        s.insert("response".into(), parse_body(&resp.body, x.request.format).map_err(Issue::other)?);
        s.insert("headers".into(), resp.headers_value());
        let children = as_list(match &records_t {
            Some(t) => t.render(&s),
            None => s["response"].clone(),
        });
        out.extend(children.into_iter().map(|record| json!({ EXPANDED: true, "parent": parent, "record": record })));
    }
    Ok(out)
}

/// The key of a `for_each` item's partition.
fn partition_key(item: &Value) -> String {
    match item {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        other => other.get("id").map(text).filter(|s| !s.is_empty()).unwrap_or_else(|| id_hash(&other.to_string())),
    }
}

/// Runs `stream` once for `conn` from `state`. `budget`: requests left in the connection's (and stream's) quota.
pub async fn run(m: &Manifest, stream: &Stream, conn: &Connection, state: StreamState, now_ms: i64, rt: &Runtime, budget: Option<u32>) -> Result<Run, Issue> {
    let mut cx = Cx::new(m, Some(stream), conn, rt, now_ms, budget).await?;
    let mut base = cx.scope.clone();
    cx.lookups(stream, &mut base, false).await?;
    cx.scope = base;
    let items = match &stream.for_each {
        Some(t) => as_list(Template::parse(t).map_err(Issue::other)?.render(&cx.scope)),
        None => vec![Value::Null],
    };
    let f = RecordFns::new(m, stream)?;
    let emit = Emit { f: &f, instance: instance(m, &conn.config), now_ms };
    let mut out = Run::default();
    let mut failures = Vec::new();
    for item in items.iter() {
        let key = partition_key(item);
        let part = state.partitions.get(&key).cloned().unwrap_or_default();
        let mut scope = cx.scope.clone();
        if !item.is_null() {
            scope.insert("item".into(), item.clone());
        }
        let looked_up = if item.is_null() { Ok(()) } else { cx.lookups(stream, &mut scope, true).await };
        let result = match looked_up {
            Ok(()) => partition(&mut cx, stream, &emit, scope, part.clone(), &mut out).await,
            Err(issue) => Err(issue),
        };
        let reset = |mut p: Partition| {
            p.token = None;
            p.backlog = None;
            p.export = None;
            p
        };
        match result {
            Ok(p) if cx.reset.swap(false, std::sync::atomic::Ordering::Relaxed) => {
                out.state.partitions.insert(key, reset(p));
            }
            Ok(p) => {
                out.state.partitions.insert(key, p);
            }
            Err(issue) if cx.reset.swap(false, std::sync::atomic::Ordering::Relaxed) => {
                out.state.partitions.insert(key, reset(part));
                keep(&mut out, Issue { kind: issue.kind, message: format!("{} (the stream starts again from its start)", issue.message) });
            }
            Err(issue) => {
                out.state.partitions.insert(key, part);
                failures.push(issue);
            }
        }
    }
    if !failures.is_empty() {
        if failures.len() == items.len() && out.records == 0 {
            return Err(failures.swap_remove(0));
        }
        out.warning = out.warning.or_else(|| Some(failures.swap_remove(0)));
    }
    out.requests = cx.requests;
    out.rotated = std::mem::take(&mut cx.rotated);
    out.mapped.points = crate::output::aggregate(std::mem::take(&mut out.mapped.points));
    Ok(out)
}

/// Whether the stream's `truncated` says the API cut this page short.
fn cut_short(stream: &Stream, scope: &Map<String, Value>, page: &Page) -> bool {
    let Some(t) = stream.truncated.as_deref().and_then(|t| Template::parse(t).ok()) else { return false };
    let mut s = scope.clone();
    s.insert("response".into(), page.response.clone());
    s.insert("headers".into(), page.headers.clone());
    s.insert("records".into(), Value::Array(page.records.clone()));
    truthy(&t.render(&s))
}

fn truncated_issue() -> Issue {
    Issue::other("The API's answer was truncated: some records may be missing (it answers fewer than there are; read more often)")
}

/// The issue of a page after the first: kept as the run's warning, the progress saved.
fn keep(out: &mut Run, issue: Issue) {
    if out.warning.is_none() {
        out.warning = Some(issue);
    }
}

async fn partition(cx: &mut Cx<'_>, stream: &Stream, emit: &Emit<'_>, scope: Map<String, Value>, mut part: Partition, out: &mut Run) -> Result<Partition, Issue> {
    if let Pagination::AsyncExport(x) = &stream.pagination {
        return export(cx, stream, x, emit, scope, part, out).await;
    }
    let req = stream.request.as_ref().ok_or_else(|| Issue::other("the stream has no request"))?;
    let max_pages = stream.max_pages.unwrap_or(MAX_PAGES);
    match &stream.incremental {
        Incremental::Time(tc) => time_windows(cx, stream, req, tc, emit, scope, part, out).await,
        kind => {
            let new_only = matches!(kind, Incremental::NewRecords { .. });
            // Newest first: a page with a record sent before ends the paging.
            let stop_at_seen = matches!(kind, Incremental::NewRecords { order: Order::NewestFirst, .. });
            let mut seen: HashSet<String> = if new_only { part.seen.iter().map(|s| s.id.clone()).collect() } else { HashSet::new() };
            let mut s = scope.clone();
            let token_next = match kind {
                Incremental::Token { initial, next } => {
                    if part.token.is_none() {
                        part.token = initial.as_deref().map(Template::parse).transpose().map_err(Issue::other)?.map(|t| t.render(&scope)).filter(|v| !missing(v));
                    }
                    s.insert("cursor".into(), json!({ "token": part.token }));
                    Some(Template::parse(next).map_err(Issue::other)?)
                }
                _ => None,
            };
            let mut pager = Pager::new(req, &stream.pagination, &stream.records, PageAt::Start)?;
            let mut pages = 0;
            let mut pending = part.token.clone();
            while !pager.done && pages < max_pages {
                let page = match pager.fetch(cx, &s, &stream.errors).await {
                    Ok(p) => p,
                    Err(issue) if pages == 0 => return Err(issue),
                    Err(issue) => {
                        keep(out, issue);
                        break;
                    }
                };
                pages += 1;
                if cut_short(stream, &s, &page) {
                    keep(out, truncated_issue());
                }
                if let Some(t) = &token_next {
                    let mut ps = s.clone();
                    ps.insert("response".into(), page.response.clone());
                    ps.insert("headers".into(), page.headers.clone());
                    ps.insert("last".into(), page.records.last().cloned().unwrap_or(Value::Null));
                    ps.insert("records".into(), Value::Array(page.records.clone()));
                    let v = t.render(&ps);
                    if !missing(&v) {
                        pending = Some(v);
                    }
                }
                let mut known = false;
                let listed = if new_only { before_expand(emit.f, stream, &s, page.records, |id, _| seen.contains(&id_hash(id)), &mut known) } else { page.records };
                let records = match expand(cx, stream, &s, listed).await {
                    Ok(r) => r,
                    Err(issue) => {
                        keep(out, issue);
                        break;
                    }
                };
                for r in records {
                    let Some(rec) = prepare_record(emit.f, &s, r) else { continue };
                    if new_only {
                        let h = id_hash(&rec.id);
                        if !seen.insert(h.clone()) {
                            known = true;
                            continue;
                        }
                        part.seen.push(Seen { id: h, t: None });
                    }
                    emit.emit(&rec, out);
                }
                if stop_at_seen && known {
                    break;
                }
            }
            if pager.capped {
                keep(out, Issue::other("more records than the API pages through: the rest were skipped"));
            }
            // A list read again every run, cut at max_pages: its rest is never read.
            if !pager.done && pages >= max_pages && token_next.is_none() && !stop_at_seen {
                keep(out, Issue::other(format!("More than {max_pages} pages: the rest was skipped (raise the stream's max_pages)")));
            }
            if let Incremental::NewRecords { max_seen, .. } = kind {
                let cap = max_seen.map_or(MAX_SEEN, |n| n as usize);
                if part.seen.len() > cap {
                    part.seen.drain(..part.seen.len() - cap);
                }
            }
            if token_next.is_some() {
                part.token = pending;
            }
            Ok(part)
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn time_windows(cx: &mut Cx<'_>, stream: &Stream, req: &Request, tc: &crate::manifest::TimeCursor, emit: &Emit<'_>, scope: Map<String, Value>, mut part: Partition, out: &mut Run) -> Result<Partition, Issue> {
    let d = |s: &Option<String>, default: i64| s.as_deref().map(duration_ms).transpose().map(|v| v.unwrap_or(default)).map_err(Issue::other);
    let (lookback, overlap, settle) = (d(&tc.lookback, 86_400_000)?, d(&tc.overlap, 0)?, d(&tc.settle, 0)?);
    let slice = tc.slice.as_deref().map(duration_ms).transpose().map_err(Issue::other)?;
    let max_window = tc.max_window.as_deref().map(duration_ms).transpose().map_err(Issue::other)?;
    let now = cx.now_ms;
    let mut queue: VecDeque<(i64, i64, PageAt, Option<i64>)> = VecDeque::new();
    let resuming = part.backlog.is_some();
    if let Some(b) = part.backlog.take() {
        queue.push_back((b.start, b.end, PageAt::from_value(&b.page), b.max_time));
    } else {
        let floor = now - lookback;
        let start = part.cursor.map_or(floor, |c| (c - overlap).max(floor));
        let mut end = now - settle;
        if let Some(mw) = max_window {
            end = end.min(start + mw);
        }
        if end <= start {
            return Ok(part);
        }
        let mut s = start;
        while s < end {
            let e = slice.map_or(end, |w| (s + w).min(end));
            queue.push_back((s, e, PageAt::Start, None));
            s = e;
        }
    }
    let mut seen: HashSet<String> = part.seen.iter().map(|s| s.id.clone()).collect();
    let mut pages_left = stream.max_pages.unwrap_or(MAX_PAGES);
    let mut first = true;
    while let Some((ws, we, at, max_before)) = queue.pop_front() {
        let mut s = scope.clone();
        s.insert("window".into(), json!({ "start": ws, "end": we }));
        let mut pager = Pager::new(req, &stream.pagination, &stream.records, at)?;
        let mut window_max = max_before;
        let mut complete = true;
        let mut split = false;
        while !pager.done {
            if pages_left == 0 {
                complete = false;
                break;
            }
            let page = match pager.fetch(cx, &s, &stream.errors).await {
                Ok(p) => p,
                Err(issue) if first => return Err(issue),
                Err(issue) => {
                    keep(out, issue);
                    complete = false;
                    break;
                }
            };
            first = false;
            pages_left -= 1;
            let page_meta = Page { response: page.response.clone(), headers: page.headers.clone(), records: if stream.truncated.is_some() { page.records.clone() } else { vec![] } };
            let early = std::cell::Cell::new(false);
            let listed = before_expand(
                emit.f,
                stream,
                &s,
                page.records,
                |id, t| {
                    early.set(early.get() || t.is_some_and(|t| t < ws));
                    seen.contains(&id_hash(id)) || t.is_some_and(|t| t < ws || t > we)
                },
                &mut false,
            );
            let mut older = early.get();
            let records = match expand(cx, stream, &s, listed).await {
                Ok(r) => r,
                Err(issue) => {
                    keep(out, issue);
                    complete = false;
                    break;
                }
            };
            for r in records {
                let Some(rec) = prepare_record(emit.f, &s, r) else { continue };
                if let Some(t) = rec.time {
                    if t < ws {
                        older = true;
                    }
                    if t < ws || t > we {
                        continue;
                    }
                    window_max = Some(window_max.map_or(t, |m: i64| m.max(t)));
                }
                let h = id_hash(&rec.id);
                if !seen.insert(h.clone()) {
                    continue;
                }
                part.seen.push(Seen { id: h, t: rec.time });
                emit.emit(&rec, out);
            }
            if !older && cut_short(stream, &s, &page_meta) {
                keep(out, truncated_issue());
            }
            // Newest first: a record older than the window means the rest of the list is too.
            if older && tc.order == Order::NewestFirst {
                pager.capped = false;
                break;
            }
            if pager.capped {
                if we - ws > 1000 {
                    split = true;
                } else {
                    keep(out, Issue::other("more records in one second than the API pages through: the rest were skipped"));
                }
            }
        }
        if split {
            let mid = ws + (we - ws) / 2;
            queue.push_front((mid, we, PageAt::Start, None));
            queue.push_front((ws, mid, PageAt::Start, None));
            continue;
        }
        if complete {
            let progress = match tc.cursor {
                CursorValue::WindowEnd => Some(we),
                CursorValue::LastRecord => window_max.or(part.cursor).or(Some(ws)),
            };
            part.cursor = part.cursor.max(progress);
            continue;
        }
        match (tc.order, window_max) {
            (Order::OldestFirst, Some(t)) if t > ws => part.cursor = part.cursor.max(Some(t)),
            _ => part.backlog = Some(Backlog { start: ws, end: we, page: pager.at.to_value(), max_time: window_max }),
        }
        break;
    }
    let _ = resuming;
    // Ids are kept while their record can be read again: inside the next window's overlap, or the backlog's window.
    let keep_from = part.backlog.as_ref().map(|b| b.start).into_iter().chain(part.cursor.map(|c| c - overlap)).min();
    if let Some(from) = keep_from {
        part.seen.retain(|s| s.t.is_none_or(|t| t >= from));
    }
    if part.seen.len() > MAX_SEEN {
        part.seen.drain(..part.seen.len() - MAX_SEEN);
    }
    Ok(part)
}

async fn export(cx: &mut Cx<'_>, stream: &Stream, x: &AsyncExport, emit: &Emit<'_>, scope: Map<String, Value>, mut part: Partition, out: &mut Run) -> Result<Partition, Issue> {
    let render = |t: &str, s: &Map<String, Value>| Template::parse(t).map(|t| t.render(s)).map_err(Issue::other);
    let window = match &stream.incremental {
        Incremental::Time(tc) => {
            let lookback = tc.lookback.as_deref().map(duration_ms).transpose().map_err(Issue::other)?.unwrap_or(86_400_000);
            let start = part.cursor.unwrap_or(cx.now_ms - lookback);
            Some((start, cx.now_ms))
        }
        _ => None,
    };
    let mut s = scope.clone();
    if part.export.is_none() {
        if let Some((start, end)) = window {
            s.insert("window".into(), json!({ "start": start, "end": end }));
        }
        let req = cx.build(&x.create, &s)?;
        let resp = cx.send(&req, &stream.errors, &s).await?;
        s.insert("response".into(), parse_body(&resp.body, x.create.format).map_err(Issue::other)?);
        let id = render(&x.id, &s)?;
        if missing(&id) {
            return Err(Issue::other("the export request answered without an export id"));
        }
        part.export = Some(Export { id, start: window.map(|w| w.0), end: window.map(|w| w.1), done: vec![], created: cx.now_ms });
    }
    let ex = part.export.clone().expect("export");
    s.insert("export".into(), json!({ "id": ex.id }));
    // The export's own window, also in the runs after the one that created it.
    if let (Some(start), Some(end)) = (ex.start, ex.end) {
        s.insert("window".into(), json!({ "start": start, "end": end }));
    }
    let req = cx.build(&x.status, &s)?;
    let resp = cx.send(&req, &stream.errors, &s).await?;
    let mut ss = s.clone();
    ss.insert("response".into(), parse_body(&resp.body, x.status.format).map_err(Issue::other)?);
    if x.failed.as_deref().map(|t| render(t, &ss)).transpose()?.is_some_and(|v| truthy(&v)) {
        part.export = None;
        keep(out, Issue::other("the export failed on the vendor's side: a new one starts next run"));
        return Ok(part);
    }
    if !truthy(&render(&x.ready, &ss)?) {
        return Ok(part);
    }
    let chunks = as_list(render(&x.chunks, &ss)?);
    let mut seen: HashSet<String> = part.seen.iter().map(|s| s.id.clone()).collect();
    let mut done: Vec<String> = ex.done.clone();
    let mut pages_left = stream.max_pages.unwrap_or(MAX_PAGES);
    for chunk in &chunks {
        let key = text(chunk);
        if done.contains(&key) {
            continue;
        }
        if pages_left == 0 {
            break;
        }
        pages_left -= 1;
        let mut cs = s.clone();
        cs.insert("chunk".into(), chunk.clone());
        let req = cx.build(&x.chunk, &cs)?;
        let resp = match cx.send(&req, &stream.errors, &cs).await {
            Ok(r) => r,
            Err(issue) => {
                keep(out, issue);
                break;
            }
        };
        cs.insert("response".into(), parse_body(&resp.body, x.chunk.format).map_err(Issue::other)?);
        let records = as_list(match &stream.records {
            Some(t) => render(t, &cs)?,
            None => cs["response"].clone(),
        });
        for r in records {
            let Some(rec) = prepare_record(emit.f, &s, r) else { continue };
            // Records outside the export's window (APIs exporting whole days) aren't its.
            if let (Some(t), Some(start), Some(end)) = (rec.time, ex.start, ex.end) {
                if t < start || t > end {
                    continue;
                }
            }
            let h = id_hash(&rec.id);
            if !seen.insert(h.clone()) {
                continue;
            }
            part.seen.push(Seen { id: h, t: rec.time });
            emit.emit(&rec, out);
        }
        done.push(key);
    }
    if chunks.iter().all(|c| done.contains(&text(c))) {
        part.cursor = part.cursor.max(ex.end);
        part.export = None;
        // Ids stay (an API exporting again what overlaps sends nothing twice), the oldest forgotten first.
        if part.seen.len() > MAX_SEEN {
            part.seen.drain(..part.seen.len() - MAX_SEEN);
        }
    } else if let Some(e) = part.export.as_mut() {
        e.done = done;
    }
    Ok(part)
}

/// Sends a one-off request (`on_connect`, `push.create`) for `conn`: the message it gives, or why it failed.
/// `extra`: more variables (`push_url`).
pub async fn call(m: &Manifest, call: &crate::manifest::Call, conn: &Connection, extra: Map<String, Value>, now_ms: i64, rt: &Runtime) -> Result<String, Issue> {
    let mut cx = Cx::new(m, None, conn, rt, now_ms, None).await?;
    let mut s = cx.scope.clone();
    s.extend(extra);
    let req = cx.build(&call.request, &s)?;
    let resp = cx.send_raw(&req).await?;
    if !(200..300).contains(&resp.status) && !call.ok_status.contains(&resp.status) {
        return Err(cx.failure(&resp, &call.errors, &s));
    }
    s.insert("response".into(), parse_body(&resp.body, call.request.format).unwrap_or(Value::Null));
    s.insert("status".into(), json!(resp.status));
    let message = call.message.as_deref().map(|t| Template::parse(t).map(|t| t.text(&s))).transpose().map_err(Issue::other)?.unwrap_or_else(|| "Done".into());
    Ok(cx.scrub(&message))
}

/// A connection test's result.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Tested {
    pub ok: bool,
    pub message: String,
    /// Records the test read (a stream's first page).
    pub records: Option<usize>,
}

/// Tests a connection: the manifest's `test` request (or its stream's first page) and message.
pub async fn test(m: &Manifest, conn: &Connection, now_ms: i64, rt: &Runtime) -> Tested {
    let failed = |message: String| Tested { ok: false, message, records: None };
    let Some(t) = &m.test else { return failed("This connector has no connection test".into()) };
    let stream = t.stream.as_ref().and_then(|id| m.streams.iter().find(|s| &s.id == id));
    let mut cx = match Cx::new(m, stream, conn, rt, now_ms, None).await {
        Ok(cx) => cx,
        Err(issue) => return failed(issue.message),
    };
    let mut s = cx.scope.clone();
    let (response, records) = match (stream, &t.request) {
        (Some(stream), _) => {
            if let Err(issue) = cx.lookups(stream, &mut s, false).await {
                return failed(issue.message);
            }
            if let Some(item) = stream.for_each.as_deref().and_then(|t| Template::parse(t).ok()).map(|t| as_list(t.render(&s))).and_then(|l| l.into_iter().next()) {
                s.insert("item".into(), item);
                if let Err(issue) = cx.lookups(stream, &mut s, true).await {
                    return failed(issue.message);
                }
            }
            if let Incremental::Time(tc) = &stream.incremental {
                let lookback = tc.lookback.as_deref().and_then(|d| duration_ms(d).ok()).unwrap_or(86_400_000);
                s.insert("window".into(), json!({ "start": now_ms - lookback, "end": now_ms }));
            }
            s.insert("cursor".into(), json!({ "token": null }));
            let Some(req) = &stream.request else { return failed("the test's stream has no request".into()) };
            let mut pager = match Pager::new(req, &stream.pagination, &stream.records, PageAt::Start) {
                Ok(p) => p,
                Err(i) => return failed(i.message),
            };
            let errors: Vec<ErrorRule> = t.errors.iter().chain(&stream.errors).cloned().collect();
            match pager.fetch(&mut cx, &s, &errors).await {
                Ok(p) => (p.response, Some(p.records)),
                Err(issue) => return failed(issue.message),
            }
        }
        (None, Some(req)) => {
            let built = match cx.build(req, &s) {
                Ok(b) => b,
                Err(i) => return failed(i.message),
            };
            match cx.send(&built, &t.errors, &s).await {
                Ok(resp) => match parse_body(&resp.body, req.format) {
                    Ok(v) => (v, None),
                    Err(e) => return failed(e),
                },
                Err(issue) => return failed(issue.message),
            }
        }
        (None, None) => return failed("the test names no request or stream".into()),
    };
    s.insert("response".into(), response);
    s.insert("records".into(), records.clone().map_or(Value::Null, Value::Array));
    let ok = t.success.as_deref().and_then(|w| Template::parse(w).ok()).is_none_or(|w| truthy(&w.render(&s)));
    let message = if ok { Template::parse(&t.message).map(|t| t.text(&s)) } else { Template::parse(t.failure.as_deref().unwrap_or("The answer isn't what the connector expects")).map(|t| t.text(&s)) }.unwrap_or_default();
    Tested { ok, message: cx.scrub(&message), records: records.map(|r| r.len()) }
}
