//! A manifest's checks beyond its schema: every template parses and reads only the variables its place has
//! (secrets only in `auth`, `tls` and requests), `config.` and `secrets.` name declared fields, ids are well formed and
//! unique, durations parse, the files it names exist. Each problem names where it is (`streams[1].request.url`).

use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;

use crate::manifest::{duration_ms, Auth, ConfigField, FieldKind, Incremental, Manifest, Pagination, Request, Signer, Stream};
use crate::template::{Template, Tree};

const BASE: &[&str] = &["config", "tables", "now", "minute", "source", "connection"];
const AUTH_EXTRA: &[&str] = &["secrets"];
const REQUEST_EXTRA: &[&str] = &["secrets", "auth", "discovery", "page", "window", "cursor", "item", "lookups", "export", "chunk"];
const ANSWER_EXTRA: &[&str] = &["secrets", "auth", "discovery", "page", "window", "cursor", "item", "lookups", "export", "chunk", "response", "headers", "status", "records", "last"];
const RECORD_EXTRA: &[&str] = &["record", "vars", "lookups", "item", "window", "discovery", "entry", "parent"];

struct Checker<'a> {
    m: &'a Manifest,
    config: BTreeSet<&'a str>,
    secrets: BTreeSet<&'a str>,
    problems: Vec<String>,
}

fn id_ok(id: &str, max: usize) -> bool {
    (1..=max).contains(&id.len()) && id.as_bytes()[0].is_ascii_alphanumeric() && id.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"-_.".contains(&b))
}

fn key_ok(key: &str) -> bool {
    !key.is_empty() && key.as_bytes()[0].is_ascii_lowercase() && key.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

impl<'a> Checker<'a> {
    fn problem(&mut self, at: &str, what: impl std::fmt::Display) {
        self.problems.push(format!("{at}: {what}"));
    }

    fn paths(&mut self, at: &str, paths: Vec<(String, Option<String>)>, extra: &[&str]) {
        for (root, key) in paths {
            if !BASE.contains(&root.as_str()) && !extra.contains(&root.as_str()) {
                let mut allowed: Vec<&str> = BASE.iter().chain(extra).copied().collect();
                allowed.sort();
                self.problem(at, format!("unknown variable {root:?} here (variables: {})", allowed.join(", ")));
                continue;
            }
            match (root.as_str(), key.as_deref()) {
                ("config", Some(k)) if !self.config.contains(k) => {
                    let hint = if self.secrets.contains(k) { format!(" ({k} is a secret: secrets.{k})") } else { String::new() };
                    self.problem(at, format!("config.{k} is not a config field{hint}"));
                }
                ("secrets", Some(k)) if !self.secrets.contains(k) => self.problem(at, format!("secrets.{k} is not a secret config field")),
                ("tables", Some(k)) if !self.m.tables.contains_key(k) => self.problem(at, format!("tables.{k} is not in tables")),
                _ => {}
            }
        }
    }

    fn template(&mut self, at: &str, t: &str, extra: &[&str]) {
        match Template::parse(t) {
            Ok(tpl) => {
                let mut paths = Vec::new();
                tpl.paths(&mut paths);
                self.paths(at, paths, extra);
            }
            Err(e) => self.problem(at, e),
        }
    }

    fn opt(&mut self, at: &str, t: &Option<String>, extra: &[&str]) {
        if let Some(t) = t {
            self.template(at, t, extra);
        }
    }

    fn tree(&mut self, at: &str, v: &Value, extra: &[&str]) {
        match Tree::parse(v) {
            Ok(tree) => {
                let mut paths = Vec::new();
                tree.paths(&mut paths);
                self.paths(at, paths, extra);
            }
            Err(e) => self.problem(at, e),
        }
    }

    fn map(&mut self, at: &str, m: &BTreeMap<String, String>, extra: &[&str]) {
        for (k, v) in m {
            self.template(&format!("{at}.{k}"), v, extra);
        }
    }

    fn duration(&mut self, at: &str, d: &Option<String>) -> Option<i64> {
        let d = d.as_deref()?;
        match duration_ms(d) {
            Ok(ms) => Some(ms),
            Err(e) => {
                self.problem(at, e);
                None
            }
        }
    }

    fn request(&mut self, at: &str, r: &Request, extra: &[&str]) {
        self.template(&format!("{at}.url"), &r.url, extra);
        for (k, v) in &r.query {
            self.tree(&format!("{at}.query.{k}"), v, extra);
        }
        self.map(&format!("{at}.headers"), &r.headers, extra);
        if let Some(b) = &r.body {
            self.tree(&format!("{at}.body"), b, extra);
        }
        if let Some(f) = &r.form {
            self.map(&format!("{at}.form"), f, extra);
        }
        if r.body.is_some() && r.form.is_some() {
            self.problem(at, "a request has a body or a form, not both");
        }
    }

    fn auth(&mut self, at: &str, a: &Auth) {
        let x = AUTH_EXTRA;
        match a {
            Auth::None => {}
            Auth::ApiKey { name, value, .. } => {
                if name.trim().is_empty() {
                    self.problem(&format!("{at}.name"), "empty");
                }
                self.template(&format!("{at}.value"), value, x);
            }
            Auth::Bearer { token } => self.template(&format!("{at}.token"), token, x),
            Auth::Basic { username, password } => {
                self.template(&format!("{at}.username"), username, x);
                self.template(&format!("{at}.password"), password, x);
            }
            Auth::Headers { headers } => self.map(&format!("{at}.headers"), headers, x),
            Auth::Oauth2ClientCredentials(c) => {
                self.template(&format!("{at}.token_url"), &c.token_url, x);
                self.template(&format!("{at}.client_id"), &c.client_id, x);
                self.template(&format!("{at}.client_secret"), &c.client_secret, x);
                self.map(&format!("{at}.params"), &c.params, x);
            }
            Auth::Oauth2RefreshToken(c) => {
                self.template(&format!("{at}.token_url"), &c.token_url, x);
                self.template(&format!("{at}.client_id"), &c.client_id, x);
                self.opt(&format!("{at}.client_secret"), &c.client_secret, x);
                self.template(&format!("{at}.refresh_token"), &c.refresh_token, x);
                self.map(&format!("{at}.params"), &c.params, x);
                if let Some(r) = &c.rotate {
                    if !self.secrets.contains(r.as_str()) {
                        self.problem(&format!("{at}.rotate"), format!("{r} is not a secret config field"));
                    }
                }
            }
            Auth::JwtBearer(j) => {
                if j.service_account.is_none() && j.private_key.is_none() {
                    self.problem(at, "jwt_bearer needs service_account or private_key");
                }
                if j.service_account.is_none() && j.token_url.is_none() {
                    self.problem(at, "jwt_bearer needs token_url without a service_account");
                }
                self.opt(&format!("{at}.token_url"), &j.token_url, x);
                self.opt(&format!("{at}.service_account"), &j.service_account, x);
                self.opt(&format!("{at}.private_key"), &j.private_key, x);
                self.opt(&format!("{at}.key_id"), &j.key_id, x);
                for (k, v) in &j.claims {
                    self.tree(&format!("{at}.claims.{k}"), v, x);
                }
                self.map(&format!("{at}.params"), &j.params, x);
                self.duration(&format!("{at}.lifetime"), &j.lifetime);
            }
            Auth::TokenExchange(t) => {
                if let Some(j) = &t.jwt {
                    self.jwt(&format!("{at}.jwt"), j);
                    if !j.headers.is_empty() {
                        self.problem(&format!("{at}.jwt.headers"), "a token exchange's JWT goes in its request (auth.jwt)");
                    }
                }
                self.request(&format!("{at}.request"), &t.request, &["secrets", "auth"]);
                if t.capture.is_empty() {
                    self.problem(&format!("{at}.capture"), "captures nothing");
                }
                self.map(&format!("{at}.capture"), &t.capture, &["secrets", "response", "headers", "status"]);
                self.opt(&format!("{at}.expires_in"), &t.expires_in, &["secrets", "response", "headers", "status"]);
                self.duration(&format!("{at}.ttl"), &t.ttl);
                self.map(&format!("{at}.headers"), &t.headers, &["secrets", "auth"]);
                if t.headers.is_empty() && !t.capture.contains_key("token") {
                    self.problem(&format!("{at}.capture"), "without headers, the exchange must capture a token (sent as a bearer token)");
                }
            }
            Auth::Jwt(j) => {
                self.jwt(at, j);
                self.map(&format!("{at}.headers"), &j.headers, &["secrets", "auth"]);
            }
            Auth::Signer { signer, params } => {
                let needed: &[&str] = match signer {
                    Signer::AkamaiEdgegrid => &["client_token", "client_secret", "access_token"],
                    Signer::DuoHmacV2 | Signer::DuoHmacV5 => &["ikey", "skey"],
                    Signer::CortexXdrAdvanced => &["key_id", "key"],
                    Signer::TrendMicroApexCentral => &["application_id", "api_key"],
                };
                for n in needed {
                    if !params.contains_key(*n) {
                        self.problem(&format!("{at}.params"), format!("the signer needs {n}"));
                    }
                }
                self.map(&format!("{at}.params"), params, x);
            }
            Auth::Oauth2ClientAssertion(c) => {
                for (k, t) in [("token_url", &c.token_url), ("client_id", &c.client_id), ("private_key", &c.private_key)] {
                    self.template(&format!("{at}.{k}"), t, x);
                }
                self.opt(&format!("{at}.key_id"), &c.key_id, x);
                self.opt(&format!("{at}.audience"), &c.audience, x);
                self.map(&format!("{at}.params"), &c.params, x);
                self.duration(&format!("{at}.lifetime"), &c.lifetime);
            }
            Auth::Oauth2AuthorizationCode(c) => {
                self.template(&format!("{at}.authorize_url"), &c.authorize_url, &[]);
                self.template(&format!("{at}.token_url"), &c.token_url, &[]);
                self.map(&format!("{at}.authorize_params"), &c.authorize_params, &[]);
                self.map(&format!("{at}.params"), &c.params, x);
                self.opt(&format!("{at}.client_id"), &c.client_id, x);
                self.opt(&format!("{at}.client_secret"), &c.client_secret, x);
                if c.client_secret.is_some() && c.client_id.is_none() {
                    self.problem(&format!("{at}.client_secret"), "a connection's own client secret goes with its client_id");
                }
            }
            Auth::Choice { field, options } => {
                match self.m.config.iter().find(|f| &f.key == field) {
                    None => self.problem(&format!("{at}.field"), format!("{field} is not a config field")),
                    Some(f) if f.secret => self.problem(&format!("{at}.field"), format!("{field} is a secret field")),
                    Some(f) => {
                        for o in f.options.iter().filter(|o| !options.contains_key(&o.value)) {
                            self.problem(&format!("{at}.options"), format!("no auth for {field} = {}", o.value));
                        }
                    }
                }
                if options.is_empty() {
                    self.problem(&format!("{at}.options"), "empty");
                }
                for (k, a) in options {
                    if matches!(a, Auth::Choice { .. }) {
                        self.problem(&format!("{at}.options.{k}"), "a choice can't hold another choice");
                    }
                    self.auth(&format!("{at}.options.{k}"), a);
                }
            }
        }
    }

    fn jwt(&mut self, at: &str, j: &crate::manifest::JwtSpec) {
        self.template(&format!("{at}.private_key"), &j.private_key, AUTH_EXTRA);
        self.opt(&format!("{at}.key_id"), &j.key_id, AUTH_EXTRA);
        for (k, v) in &j.claims {
            self.tree(&format!("{at}.claims.{k}"), v, &["secrets", "key"]);
        }
        self.duration(&format!("{at}.lifetime"), &j.lifetime);
    }

    fn call(&mut self, at: &str, c: &crate::manifest::Call, extra: &[&str]) {
        let mut vars: Vec<&str> = REQUEST_EXTRA.to_vec();
        vars.extend(extra);
        self.request(&format!("{at}.request"), &c.request, &vars);
        vars.extend(["response", "status", "headers"]);
        self.opt(&format!("{at}.message"), &c.message, &vars);
        for (i, e) in c.errors.iter().enumerate() {
            self.template(&format!("{at}.errors[{i}].message"), &e.message, &vars);
        }
    }

    fn pagination(&mut self, at: &str, p: &Pagination) {
        match p {
            Pagination::Cursor { next } | Pagination::NextUrl { next, .. } => self.template(&format!("{at}.next"), next, ANSWER_EXTRA),
            Pagination::Offset { size, total, .. } => {
                if *size == 0 {
                    self.problem(&format!("{at}.size"), "must be at least 1");
                }
                self.opt(&format!("{at}.total"), total, ANSWER_EXTRA);
            }
            Pagination::Page { size, total_pages, .. } => {
                if *size == 0 {
                    self.problem(&format!("{at}.size"), "must be at least 1");
                }
                self.opt(&format!("{at}.total_pages"), total_pages, ANSWER_EXTRA);
            }
            Pagination::AsyncExport(x) => {
                self.request(&format!("{at}.create"), &x.create, REQUEST_EXTRA);
                self.template(&format!("{at}.id"), &x.id, ANSWER_EXTRA);
                self.request(&format!("{at}.status"), &x.status, REQUEST_EXTRA);
                self.template(&format!("{at}.ready"), &x.ready, ANSWER_EXTRA);
                self.opt(&format!("{at}.failed"), &x.failed, ANSWER_EXTRA);
                self.template(&format!("{at}.chunks"), &x.chunks, ANSWER_EXTRA);
                self.request(&format!("{at}.chunk"), &x.chunk, REQUEST_EXTRA);
            }
            Pagination::None | Pagination::LinkHeader => {}
        }
    }

    fn stream(&mut self, i: usize, s: &Stream) {
        let at = format!("streams[{i}]");
        if !key_ok(&s.id) {
            self.problem(&format!("{at}.id"), format!("{:?} must be lowercase letters, digits and _", s.id));
        }
        if let Some(ms) = self.duration(&format!("{at}.interval"), &s.interval) {
            if ms < 60_000 {
                self.problem(&format!("{at}.interval"), "must be at least 1m");
            }
        }
        let export = matches!(s.pagination, Pagination::AsyncExport(_));
        match (&s.request, self.m.native.is_some() || export) {
            (Some(r), _) => self.request(&format!("{at}.request"), r, REQUEST_EXTRA),
            (None, false) => self.problem(&at, "a stream needs a request"),
            (None, true) => {}
        }
        if let Some(a) = &s.auth {
            self.auth(&format!("{at}.auth"), a);
        }
        self.opt(&format!("{at}.for_each"), &s.for_each, &["discovery", "lookups"]);
        self.pagination(&format!("{at}.pagination"), &s.pagination);
        self.opt(&format!("{at}.records"), &s.records, ANSWER_EXTRA);
        if let Some(x) = &s.expand {
            let with_parent: Vec<&str> = REQUEST_EXTRA.iter().copied().chain(["parent"]).collect();
            self.request(&format!("{at}.expand.request"), &x.request, &with_parent);
            let answer: Vec<&str> = ANSWER_EXTRA.iter().copied().chain(["parent"]).collect();
            self.opt(&format!("{at}.expand.records"), &x.records, &answer);
        }
        let record: Vec<&str> = RECORD_EXTRA.iter().copied().filter(|r| *r != "vars" && *r != "entry").collect();
        self.opt(&format!("{at}.truncated"), &s.truncated, ANSWER_EXTRA);
        self.opt(&format!("{at}.record_id"), &s.record_id, &record);
        self.opt(&format!("{at}.filter"), &s.filter, &record);
        self.map(&format!("{at}.vars"), &s.vars, &record);
        let mut names = BTreeSet::new();
        for (j, l) in s.lookups.iter().enumerate() {
            let lat = format!("{at}.lookups[{j}]");
            if !key_ok(&l.name) || !names.insert(l.name.clone()) {
                self.problem(&format!("{lat}.name"), format!("{:?} must be unique, lowercase letters, digits and _", l.name));
            }
            self.request(&format!("{lat}.request"), &l.request, REQUEST_EXTRA);
            self.pagination(&format!("{lat}.pagination"), &l.pagination);
            self.opt(&format!("{lat}.records"), &l.records, ANSWER_EXTRA);
            self.template(&format!("{lat}.key"), &l.key, &["record"]);
            self.opt(&format!("{lat}.value"), &l.value, &["record"]);
            self.duration(&format!("{lat}.ttl"), &l.ttl);
            self.opt(&format!("{lat}.when"), &l.when, &["item", "discovery"]);
        }
        match &s.incremental {
            Incremental::NewRecords { .. } if s.record_id.is_none() => self.problem(&format!("{at}.incremental"), "new_records needs the stream's record_id"),
            Incremental::Time(t) => {
                let iat = format!("{at}.incremental");
                let with_vars: Vec<&str> = record.iter().copied().chain(["vars"]).collect();
                self.template(&format!("{iat}.record_time"), &t.record_time, &with_vars);
                for (name, d) in [("lookback", &t.lookback), ("overlap", &t.overlap), ("settle", &t.settle), ("slice", &t.slice), ("max_window", &t.max_window)] {
                    if self.duration(&format!("{iat}.{name}"), d) == Some(0) && (name == "slice" || name == "max_window") {
                        self.problem(&format!("{iat}.{name}"), "must be more than 0");
                    }
                }
            }
            Incremental::Token { initial, next } => {
                self.opt(&format!("{at}.incremental.initial"), initial, &[]);
                self.template(&format!("{at}.incremental.next"), next, ANSWER_EXTRA);
            }
            _ => {}
        }
        for (j, e) in s.errors.iter().enumerate() {
            self.template(&format!("{at}.errors[{j}].message"), &e.message, ANSWER_EXTRA);
            self.opt(&format!("{at}.errors[{j}].when"), &e.when, ANSWER_EXTRA);
        }
        let o = &s.output;
        if o.logs.is_none() && o.metrics.is_empty() && o.events.is_none() && o.costs.is_none() {
            self.problem(&format!("{at}.output"), "maps records to nothing (logs, metrics, events or costs)");
        }
        let rec = RECORD_EXTRA;
        if let Some(l) = &o.logs {
            let lat = format!("{at}.output.logs");
            self.template(&format!("{lat}.message"), &l.message, rec);
            for (name, t) in [("status", &l.status), ("service", &l.service), ("source", &l.source), ("host", &l.host), ("date", &l.date)] {
                self.opt(&format!("{lat}.{name}"), t, rec);
            }
            self.map(&format!("{lat}.tags"), &l.tags, rec);
            if let Some(a) = &l.attributes {
                if !a.is_object() && !a.is_string() {
                    self.problem(&format!("{lat}.attributes"), "must be an object, or a template giving one");
                }
                self.tree(&format!("{lat}.attributes"), a, rec);
            }
        }
        for (j, mo) in o.metrics.iter().enumerate() {
            let mat = format!("{at}.output.metrics[{j}]");
            self.template(&format!("{mat}.name"), &mo.name, rec);
            if Template::parse(&mo.name).is_ok_and(|t| t.is_static()) && !(mo.name.as_bytes().first().is_some_and(u8::is_ascii_alphabetic) && mo.name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'.')) {
                self.problem(&format!("{mat}.name"), format!("{:?} is not a metric name (letters, digits, _ and .)", mo.name));
            }
            self.template(&format!("{mat}.value"), &mo.value, rec);
            for (name, t) in [("each", &mo.each), ("tags_from", &mo.tags_from), ("host", &mo.host), ("time", &mo.time), ("filter", &mo.filter), ("unit", &mo.unit)] {
                self.opt(&format!("{mat}.{name}"), t, rec);
            }
            self.map(&format!("{mat}.tags"), &mo.tags, rec);
        }
        if let Some(e) = &o.events {
            let eat = format!("{at}.output.events");
            self.template(&format!("{eat}.title"), &e.title, rec);
            self.template(&format!("{eat}.text"), &e.text, rec);
            for (name, t) in [("alert_type", &e.alert_type), ("priority", &e.priority), ("host", &e.host), ("aggregation_key", &e.aggregation_key), ("date", &e.date)] {
                self.opt(&format!("{eat}.{name}"), t, rec);
            }
            self.map(&format!("{eat}.tags"), &e.tags, rec);
        }
        if let Some(c) = &o.costs {
            let cat = format!("{at}.output.costs");
            if !id_ok(&c.provider, 50) {
                self.problem(&format!("{cat}.provider"), "lowercase letters, digits, -, _ and .");
            }
            self.template(&format!("{cat}.date"), &c.date, rec);
            self.template(&format!("{cat}.cost"), &c.cost, rec);
            self.template(&format!("{cat}.service"), &c.service, rec);
            for (name, t) in [("currency", &c.currency), ("account", &c.account), ("usage_type", &c.usage_type), ("usage_amount", &c.usage_amount)] {
                self.opt(&format!("{cat}.{name}"), t, rec);
            }
            self.map(&format!("{cat}.tags"), &c.tags, rec);
        }
    }

    fn field(&mut self, i: usize, f: &ConfigField) {
        let at = format!("config[{i}]");
        if !key_ok(&f.key) {
            self.problem(&format!("{at}.key"), format!("{:?} must be lowercase letters, digits and _", f.key));
        }
        if f.secret && !matches!(f.kind, FieldKind::String | FieldKind::Text) {
            self.problem(&at, "only string and text fields can be secret");
        }
        if matches!(f.kind, FieldKind::Select | FieldKind::Multiselect) && f.options.is_empty() {
            self.problem(&at, "a select needs options");
        }
        if let Some(p) = &f.pattern {
            if regex::Regex::new(p).is_err() {
                self.problem(&format!("{at}.pattern"), "not a regular expression");
            }
        }
        if f.secret && f.default.is_some() {
            self.problem(&at, "a secret can't have a default");
        }
    }
}

/// Every problem of `m` (with its package's `files`), empty when it is valid.
pub fn validate(m: &Manifest, files: &BTreeMap<String, String>) -> Vec<String> {
    let mut c = Checker { m, config: BTreeSet::new(), secrets: BTreeSet::new(), problems: Vec::new() };
    if m.schema_version != 1 {
        c.problem("schema_version", "this server reads schema_version 1");
    }
    if !id_ok(&m.id, 100) {
        c.problem("id", format!("{:?} must be lowercase letters, digits, -, _ and .", m.id));
    }
    for (name, v) in [("name", &m.name), ("publisher", &m.publisher), ("vendor", &m.vendor), ("description", &m.description)] {
        if v.trim().is_empty() {
            c.problem(name, "empty");
        }
    }
    for t in &m.tiles {
        if !id_ok(t, 100) {
            c.problem("tiles", format!("{t:?} is not a catalog id"));
        }
    }
    let mut keys = BTreeSet::new();
    for (i, f) in m.config.iter().enumerate() {
        if !keys.insert(f.key.as_str()) {
            c.problem(&format!("config[{i}].key"), format!("{} is declared twice", f.key));
        }
        c.field(i, f);
        if f.secret {
            c.secrets.insert(&f.key);
        } else {
            c.config.insert(&f.key);
        }
    }
    c.opt("instance", &m.instance, &[]);
    for (i, l) in m.links.iter().enumerate() {
        c.template(&format!("links[{i}].url"), &l.url, &[]);
    }
    c.auth("auth", &m.auth);
    if let Some(t) = &m.tls {
        c.template("tls.certificate", &t.certificate, AUTH_EXTRA);
        c.template("tls.key", &t.key, AUTH_EXTRA);
    }
    c.map("headers", &m.headers, &["secrets", "auth"]);
    for (i, e) in m.errors.iter().enumerate() {
        c.template(&format!("errors[{i}].message"), &e.message, ANSWER_EXTRA);
        c.opt(&format!("errors[{i}].when"), &e.when, ANSWER_EXTRA);
    }
    for (name, r) in [("rate_limit", &m.rate_limit)] {
        if let Some(r) = r {
            c.duration(&format!("{name}.min_interval"), &r.min_interval);
        }
    }
    let mut names = BTreeSet::new();
    for (i, d) in m.discover.iter().enumerate() {
        if !key_ok(&d.name) || !names.insert(&d.name) {
            c.problem(&format!("discover[{i}].name"), format!("{:?} must be unique, lowercase letters, digits and _", d.name));
        }
        c.request(&format!("discover[{i}].request"), &d.request, &["secrets", "auth", "discovery", "page"]);
        c.opt(&format!("discover[{i}].value"), &d.value, ANSWER_EXTRA);
        c.duration(&format!("discover[{i}].ttl"), &d.ttl);
        c.pagination(&format!("discover[{i}].pagination"), &d.pagination);
        c.opt(&format!("discover[{i}].records"), &d.records, ANSWER_EXTRA);
    }
    let mut ids = BTreeSet::new();
    for (i, s) in m.streams.iter().enumerate() {
        if !ids.insert(s.id.as_str()) {
            c.problem(&format!("streams[{i}].id"), format!("{} is declared twice", s.id));
        }
        c.stream(i, s);
        if let Some(r) = &s.rate_limit {
            c.duration(&format!("streams[{i}].rate_limit.min_interval"), &r.min_interval);
        }
    }
    if m.streams.is_empty() && m.push.is_none() && m.webhook.is_none() {
        c.problem("streams", "a package needs streams, push or webhook");
    }
    match &m.test {
        None if !m.streams.is_empty() && m.native.is_none() => c.problem("test", "a package with streams needs a connection test"),
        None => {}
        Some(t) => {
            match (&t.request, &t.stream) {
                (Some(r), None) => c.request("test.request", r, REQUEST_EXTRA),
                (None, Some(s)) if !ids.contains(s.as_str()) => c.problem("test.stream", format!("{s} is not a stream")),
                (None, Some(_)) => {}
                _ => c.problem("test", "a test has a request or a stream"),
            }
            c.opt("test.success", &t.success, ANSWER_EXTRA);
            c.template("test.message", &t.message, ANSWER_EXTRA);
            c.opt("test.failure", &t.failure, ANSWER_EXTRA);
            for (i, e) in t.errors.iter().enumerate() {
                c.template(&format!("test.errors[{i}].message"), &e.message, ANSWER_EXTRA);
            }
        }
    }
    for (i, call) in m.on_connect.iter().enumerate() {
        c.call(&format!("on_connect[{i}]"), call, &[]);
    }
    if let Some(p) = &m.push {
        c.opt("push.url", &p.url, &["intake", "api_key"]);
        c.map("push.headers", &p.headers, &["intake", "api_key"]);
        if let Some(create) = &p.create {
            c.call("push.create", create, &["push_url"]);
        }
        for (i, e) in p.endpoints.iter().enumerate() {
            c.opt(&format!("push.endpoints[{i}].url"), &e.url, &["intake", "api_key"]);
            c.map(&format!("push.endpoints[{i}].headers"), &e.headers, &["intake", "api_key"]);
        }
        if let Some(s) = &p.signature {
            if !c.secrets.contains(s.secret.as_str()) {
                c.problem("push.signature.secret", format!("{} is not a secret config field", s.secret));
            }
            c.duration("push.signature.tolerance", &s.tolerance);
        }
    }
    if let Some(w) = &m.webhook {
        const WEBHOOK: &[&str] = &["record", "body", "headers"];
        c.opt("webhook.records", &w.records, &["body", "headers"]);
        c.opt("webhook.record_id", &w.record_id, WEBHOOK);
        c.opt("webhook.filter", &w.filter, WEBHOOK);
        if w.logs.is_none() && w.events.is_none() {
            c.problem("webhook", "a webhook makes logs or events");
        }
        if let Some(l) = &w.logs {
            c.template("webhook.logs.message", &l.message, WEBHOOK);
            for (k, t) in [("status", &l.status), ("service", &l.service), ("source", &l.source), ("host", &l.host), ("date", &l.date)] {
                c.opt(&format!("webhook.logs.{k}"), t, WEBHOOK);
            }
            c.map("webhook.logs.tags", &l.tags, WEBHOOK);
            if let Some(a) = &l.attributes {
                c.tree("webhook.logs.attributes", a, WEBHOOK);
            }
        }
        if let Some(e) = &w.events {
            c.template("webhook.events.title", &e.title, WEBHOOK);
            c.template("webhook.events.text", &e.text, WEBHOOK);
            for (k, t) in [("alert_type", &e.alert_type), ("priority", &e.priority), ("host", &e.host), ("aggregation_key", &e.aggregation_key), ("date", &e.date)] {
                c.opt(&format!("webhook.events.{k}"), t, WEBHOOK);
            }
            c.map("webhook.events.tags", &e.tags, WEBHOOK);
        }
    }
    // Assets and fixtures name files of the package.
    let json_file = |c: &mut Checker, at: &str, path: &str| match files.get(path) {
        None => c.problem(at, format!("{path} is not in the package")),
        Some(text) => {
            if serde_json::from_str::<Value>(text).is_err() {
                c.problem(at, format!("{path} is not JSON"));
            }
        }
    };
    let a = &m.assets;
    for (i, p) in a.dashboards.iter().enumerate() {
        json_file(&mut c, &format!("assets.dashboards[{i}]"), p);
    }
    if !a.dashboards.is_empty() {
        match files.get("dist/dashboards.json").map(|t| serde_json::from_str::<Value>(t)) {
            Some(Ok(Value::Array(d))) if d.len() == a.dashboards.len() => {}
            _ => c.problem("assets.dashboards", "dist/dashboards.json is missing or out of date: run node scripts/connector.mjs build <package>"),
        }
    }
    if let Some(p) = &a.pipeline {
        json_file(&mut c, "assets.pipeline", p);
        if files.get(p).and_then(|t| serde_json::from_str::<Value>(t).ok()).is_some_and(|v| !v["processors"].is_array()) {
            c.problem("assets.pipeline", format!("{p} has no processors (Datadog's pipeline JSON)"));
        }
    }
    for (name, list) in [("monitors", &a.monitors), ("security_rules", &a.security_rules), ("log_metrics", &a.log_metrics)] {
        for (i, p) in list.iter().enumerate() {
            json_file(&mut c, &format!("assets.{name}[{i}]"), p);
        }
    }
    if let Some(p) = &a.metadata {
        if !files.contains_key(p) {
            c.problem("assets.metadata", format!("{p} is not in the package"));
        }
    }
    if let Some(mock) = &m.mock {
        if duration_ms("0s").is_ok() && crate::expr::to_ms(&Value::String(mock.now.clone())).is_none() {
            c.problem("mock.now", "not a time (RFC 3339)");
        }
        for (i, r) in mock.responses.iter().enumerate() {
            if let Some(f) = &r.file {
                if !files.contains_key(f) {
                    c.problem(&format!("mock.responses[{i}].file"), format!("{f} is not in the package"));
                }
            }
            if r.file.is_some() && r.body.is_some() {
                c.problem(&format!("mock.responses[{i}]"), "a body or a file, not both");
            }
        }
        for k in mock.expect.keys() {
            if k != "test" && !ids.contains(k.as_str()) {
                c.problem(&format!("mock.expect.{k}"), "not a stream (or test)");
            }
        }
        for k in mock.secrets.keys() {
            if !c.secrets.contains(k.as_str()) {
                c.problem(&format!("mock.secrets.{k}"), "not a secret config field");
            }
        }
        for k in mock.config.keys() {
            if !c.config.contains(k.as_str()) {
                c.problem(&format!("mock.config.{k}"), "not a config field");
            }
        }
    }
    c.problems
}
