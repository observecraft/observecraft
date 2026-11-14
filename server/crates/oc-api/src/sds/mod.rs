//! Sensitive Data Scanner: finds sensitive values in logs (credit card numbers, credentials, personal data) and
//! redacts, hashes or only tags them, before the logs are stored.
//!
//! Each org's scanning groups and rules ([`config`], Postgres `sds_*`, edited through Datadog's API and the UI) are
//! compiled into a [`Scanner`]. The log indexer ([`crate::logproc`]) runs it on every log after the pipelines and
//! standard attributes, so indexes, generated metrics, forwarding, archives and the 72-hour copy all get the
//! scanned log; the Kafka ingester also runs it on live tail's copy ([`crate::log_ingest`]), which is taken before
//! the pipelines. Scanners are rebuilt when an org's configuration version moves (checked every 10 s).
//!
//! A group applies to the logs its filter matches (log search syntax), sampled at its `logs` rate. Its enabled rules
//! run in order, each on what the earlier ones left: on the message and the attributes (`namespaces` narrows, and
//! `excluded_namespaces` removes, attribute paths; `message` names the message). A match counts when its keyword
//! condition holds (one of the keywords within `character_count` characters before it, or in the attribute's
//! path) and no suppression names it. A matching log gets the rule's tags (a standard pattern's
//! `sensitive_data:<pattern>` / `sensitive_data_category:<category>` when the rule has none); tags hold one value,
//! so on a key two rules set, the higher priority (lower number) rule's value stays.
//!
//! Matches are counted per org, group, rule and service, and written every minute as the count metric
//! `sds.matches` (tags `sds_group`, `sds_group_id`, `sds_rule`, `sds_rule_id`, `service`).

pub mod config;
pub mod patterns;

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock, RwLock};

use oc_query::Fields;
use regex::Regex;
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::pipelines::Filter;
use patterns::{Candidate, Check, StandardPattern};

/// The metric matches are counted in.
pub const MATCH_METRIC: &str = "sds.matches";
/// Default `character_count` of a keyword condition.
pub const DEFAULT_KEYWORD_CHARS: usize = 30;
/// Largest compiled program a rule's regular expression may have (bytes).
const REGEX_SIZE_LIMIT: usize = 1 << 20;
/// Values longer than this are scanned up to here (the rest is left as is).
const MAX_SCANNED_BYTES: usize = 256 * 1024;
/// Priority of a rule that sets none.
const DEFAULT_PRIORITY: i64 = 3;

/// How a rule replaces what it finds (`text_replacement`).
#[derive(Clone, Debug, PartialEq)]
pub enum Replacement {
    /// Tags the log only.
    None,
    /// A 16-hex-digit hash of the value, the same for the same value in the org.
    Hash,
    /// A fixed text.
    Text(String),
    /// `*` over the first n characters.
    FromBeginning(usize),
    /// `*` over the last n characters.
    FromEnd(usize),
}

/// A rule's regular expression, compiled with the `regex` crate (linear time).
pub fn compile_pattern(pattern: &str) -> Result<Regex, String> {
    regex::RegexBuilder::new(pattern).size_limit(REGEX_SIZE_LIMIT).build().map_err(|e| format!("Invalid pattern: {e}"))
}

struct Rule {
    id: String,
    name: String,
    regex: Regex,
    /// The capture group holding the sensitive part.
    group: usize,
    check: Option<Check>,
    namespaces: Vec<String>,
    excluded: Vec<String>,
    /// Lowercased; empty: no keyword condition.
    keywords: Vec<String>,
    keyword_chars: usize,
    starts_with: Vec<String>,
    ends_with: Vec<String>,
    exact: Vec<String>,
    replacement: Replacement,
    tags: Vec<(String, String)>,
    priority: i64,
}

struct Group {
    id: String,
    name: String,
    filter: Filter,
    /// Percent of the logs scanned (0 to 100).
    rate: f64,
    rules: Vec<Rule>,
}

/// One org's compiled configuration.
pub struct Scanner {
    groups: Vec<Group>,
}

/// What a rule found in one log.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hit {
    pub group_id: String,
    pub group: String,
    pub rule_id: String,
    pub rule: String,
    pub count: u64,
}

/// The log fields a scan reads and rewrites.
pub struct Target<'a> {
    pub id: &'a str,
    pub service: &'a str,
    pub status: &'a str,
    pub host: &'a str,
    pub source: &'a str,
    pub message: &'a mut String,
    pub tags: &'a mut HashMap<String, String>,
    /// Flattened attributes (`http.url` → value), as stored.
    pub attr: &'a mut HashMap<String, String>,
}

fn strings(v: Option<&Value>) -> Vec<String> {
    v.and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect()).unwrap_or_default()
}

/// `key:value` tags (a bare `key` gets an empty value).
pub fn parse_tags(tags: &[String]) -> Vec<(String, String)> {
    tags.iter()
        .map(|t| t.trim())
        .filter(|t| !t.is_empty())
        .map(|t| match t.split_once(':') {
            Some((k, v)) => (k.to_string(), v.to_string()),
            None => (t.to_string(), String::new()),
        })
        .collect()
}

/// A rule's `text_replacement`.
pub fn replacement(v: Option<&Value>) -> Replacement {
    let Some(v) = v else { return Replacement::None };
    let n = || v.get("number_of_chars").and_then(Value::as_u64).unwrap_or(0) as usize;
    match v.get("type").and_then(Value::as_str).unwrap_or("none") {
        "hash" => Replacement::Hash,
        "replacement_string" => Replacement::Text(v.get("replacement_string").and_then(Value::as_str).unwrap_or("[sensitive_data]").to_string()),
        "partial_replacement_from_beginning" => Replacement::FromBeginning(n()),
        "partial_replacement_from_end" => Replacement::FromEnd(n()),
        _ => Replacement::None,
    }
}

impl Rule {
    /// A stored rule; None when it's disabled or doesn't compile.
    fn compile(r: &config::RuleRow) -> Option<Rule> {
        let a = &r.attributes;
        if !a.get("is_enabled").and_then(Value::as_bool).unwrap_or(false) {
            return None;
        }
        let standard: Option<&StandardPattern> = r.standard_pattern_id.as_deref().and_then(patterns::find);
        let (regex, group, check) = match standard {
            Some(p) => (compile_pattern(p.pattern).ok()?, p.group, p.check),
            None => (compile_pattern(a.get("pattern").and_then(Value::as_str)?).ok()?, 0, None),
        };
        let kw = a.get("included_keyword_configuration");
        let mut keywords = strings(kw.and_then(|k| k.get("keywords")));
        if kw.and_then(|k| k.get("use_recommended_keywords")).and_then(Value::as_bool).unwrap_or(false) {
            keywords.extend(standard.map(|p| p.keywords.iter().map(|k| k.to_string()).collect::<Vec<_>>()).unwrap_or_default());
        }
        let suppress = a.get("suppressions");
        let mut tags = parse_tags(&strings(a.get("tags")));
        if tags.is_empty() {
            tags = standard.map(|p| parse_tags(&p.tags())).unwrap_or_default();
        }
        Some(Rule {
            id: r.id.clone(),
            name: a.get("name").and_then(Value::as_str).unwrap_or("").to_string(),
            regex,
            group,
            check,
            namespaces: strings(a.get("namespaces")),
            excluded: strings(a.get("excluded_namespaces")),
            keywords: keywords.iter().map(|k| k.to_lowercase()).filter(|k| !k.is_empty()).collect(),
            keyword_chars: kw.and_then(|k| k.get("character_count")).and_then(Value::as_u64).map_or(DEFAULT_KEYWORD_CHARS, |n| n as usize),
            starts_with: strings(suppress.and_then(|s| s.get("starts_with"))),
            ends_with: strings(suppress.and_then(|s| s.get("ends_with"))),
            exact: strings(suppress.and_then(|s| s.get("exact_match"))),
            replacement: replacement(a.get("text_replacement")),
            tags,
            priority: a.get("priority").and_then(Value::as_i64).or(standard.map(|p| p.priority)).unwrap_or(DEFAULT_PRIORITY),
        })
    }

    /// Whether the rule scans the value at `path` (`message`, or an attribute path).
    fn scans(&self, path: &str) -> bool {
        let under = |ns: &String| path == ns || path.strip_prefix(ns.as_str()).is_some_and(|rest| rest.starts_with('.'));
        (self.namespaces.is_empty() || self.namespaces.iter().any(under)) && !self.excluded.iter().any(under)
    }

    fn suppressed(&self, text: &str) -> bool {
        self.exact.iter().any(|s| s == text)
            || self.starts_with.iter().any(|s| !s.is_empty() && text.starts_with(s.as_str()))
            || self.ends_with.iter().any(|s| !s.is_empty() && text.ends_with(s.as_str()))
    }

    fn keyword_near(&self, path: &str, before: &str) -> bool {
        if self.keywords.is_empty() {
            return true;
        }
        let start = before.char_indices().rev().nth(self.keyword_chars.saturating_sub(1)).map_or(0, |(i, _)| i);
        let window = before[start..].to_lowercase();
        let path = path.to_lowercase();
        self.keywords.iter().any(|k| window.contains(k.as_str()) || path.contains(k.as_str()))
    }

    /// Scans one value: the matches' spans, after checks, keywords and suppressions.
    fn find(&self, path: &str, text: &str) -> Vec<(usize, usize)> {
        let scanned = &text[..floor_char_boundary(text, MAX_SCANNED_BYTES)];
        self.regex
            .captures_iter(scanned)
            .filter_map(|c| c.get(self.group))
            .filter(|m| !m.as_str().is_empty())
            .filter(|m| self.check.is_none_or(|check| check(&Candidate { text: m.as_str(), before: &text[..m.start()], after: &text[m.end()..] })))
            .filter(|m| self.keyword_near(path, &text[..m.start()]))
            .filter(|m| !self.suppressed(m.as_str()))
            .map(|m| (m.start(), m.end()))
            .collect()
    }
}

fn floor_char_boundary(s: &str, max: usize) -> usize {
    if s.len() <= max {
        return s.len();
    }
    (0..=max).rev().find(|i| s.is_char_boundary(*i)).unwrap_or(0)
}

/// FNV-1a: a log's stable sampling point (0 to 9 999), the same on every server.
fn sample_point(id: &str) -> u64 {
    id.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ b as u64).wrapping_mul(0x0100_0000_01b3)) % 10_000
}

fn hash_value(salt: &[u8], value: &str) -> String {
    let mut h = Sha256::new();
    h.update(salt);
    h.update(value.as_bytes());
    h.finalize().iter().take(8).map(|b| format!("{b:02x}")).collect()
}

/// `text` with the spans replaced.
fn redact(text: &str, spans: &[(usize, usize)], how: &Replacement, salt: &[u8]) -> String {
    let mut out = String::with_capacity(text.len());
    let mut last = 0;
    for &(start, end) in spans {
        out.push_str(&text[last..start]);
        let found = &text[start..end];
        let n = found.chars().count();
        match how {
            Replacement::None => out.push_str(found),
            Replacement::Hash => out.push_str(&hash_value(salt, found)),
            Replacement::Text(t) => out.push_str(t),
            Replacement::FromBeginning(k) => {
                let k = (*k).min(n);
                out.extend(std::iter::repeat_n('*', k));
                out.extend(found.chars().skip(k));
            }
            Replacement::FromEnd(k) => {
                let k = (*k).min(n);
                out.extend(found.chars().take(n - k));
                out.extend(std::iter::repeat_n('*', k));
            }
        }
        last = end;
    }
    out.push_str(&text[last..]);
    out
}

impl Scanner {
    /// Compiles an org's configuration: its enabled groups scanning logs, and their enabled rules that compile.
    pub fn compile(cfg: &config::Config) -> Scanner {
        let mut groups = Vec::new();
        for g in &cfg.groups {
            let a = &g.attributes;
            let logs = a.get("product_list").and_then(Value::as_array).is_none_or(|p| p.is_empty() || p.iter().any(|v| v.as_str() == Some("logs")));
            if !a.get("is_enabled").and_then(Value::as_bool).unwrap_or(false) || !logs {
                continue;
            }
            let rate = a
                .get("samplings")
                .and_then(Value::as_array)
                .and_then(|s| s.iter().find(|s| s.get("product").and_then(Value::as_str) == Some("logs")))
                .and_then(|s| s.get("rate").and_then(Value::as_f64))
                .unwrap_or(100.0)
                .clamp(0.0, 100.0);
            let rules: Vec<Rule> = cfg.rules.iter().filter(|r| r.group_id == g.id).filter_map(Rule::compile).collect();
            if rules.is_empty() {
                continue;
            }
            let query = a.get("filter").and_then(|f| f.get("query")).and_then(Value::as_str).unwrap_or("*").trim();
            groups.push(Group {
                id: g.id.clone(),
                name: a.get("name").and_then(Value::as_str).unwrap_or("").to_string(),
                filter: Filter::parse(if query.is_empty() { "*" } else { query }),
                rate,
                rules,
            });
        }
        Scanner { groups }
    }

    pub fn is_empty(&self) -> bool {
        self.groups.is_empty()
    }

    /// Scans a log, redacting in place; returns what each rule found.
    pub fn scan(&self, t: &mut Target<'_>, salt: &[u8]) -> Vec<Hit> {
        let mut hits = Vec::new();
        // The priority of the rule that set each tag in this scan.
        let mut set_by: HashMap<String, i64> = HashMap::new();
        for g in &self.groups {
            if g.rate < 100.0 && (sample_point(t.id) as f64) >= g.rate * 100.0 {
                continue;
            }
            let applies = g.filter.matches(&Fields { service: t.service, status: t.status, host: t.host, source: t.source, message: t.message, tags: t.tags, attr: t.attr, index: None });
            if !applies {
                continue;
            }
            for r in &g.rules {
                let mut count = 0u64;
                if r.scans("message") {
                    let spans = r.find("message", t.message);
                    if !spans.is_empty() {
                        count += spans.len() as u64;
                        if r.replacement != Replacement::None {
                            *t.message = redact(t.message, &spans, &r.replacement, salt);
                        }
                    }
                }
                for (path, value) in t.attr.iter_mut() {
                    if !r.scans(path) {
                        continue;
                    }
                    let spans = r.find(path, value);
                    if !spans.is_empty() {
                        count += spans.len() as u64;
                        if r.replacement != Replacement::None {
                            *value = redact(value, &spans, &r.replacement, salt);
                        }
                    }
                }
                if count == 0 {
                    continue;
                }
                for (k, v) in &r.tags {
                    if set_by.get(k).is_none_or(|p| r.priority < *p) {
                        set_by.insert(k.clone(), r.priority);
                        t.tags.insert(k.clone(), v.clone());
                    }
                }
                hits.push(Hit { group_id: g.id.clone(), group: g.name.clone(), rule_id: r.id.clone(), rule: r.name.clone(), count });
            }
        }
        hits
    }

    /// Scans a log the indexer holds as JSON (Vector's rows: `message`, `tags`, `attr` and the typed `attributes`).
    pub fn scan_row(&self, row: &mut Value, salt: &[u8]) -> Vec<Hit> {
        let s = |k: &str| row.get(k).and_then(Value::as_str).unwrap_or("").to_string();
        let (id, service, status, host, source, mut message) = (s("id"), s("service"), s("status"), s("host"), s("source"), s("message"));
        let map = |k: &str| -> HashMap<String, String> {
            row.get(k).and_then(Value::as_object).map(|o| o.iter().map(|(k, v)| (k.clone(), v.as_str().map(str::to_string).unwrap_or_else(|| v.to_string()))).collect()).unwrap_or_default()
        };
        let (mut tags, mut attr) = (map("tags"), map("attr"));
        let before = attr.clone();
        let hits = self.scan(&mut Target { id: &id, service: &service, status: &status, host: &host, source: &source, message: &mut message, tags: &mut tags, attr: &mut attr }, salt);
        if hits.is_empty() {
            return hits;
        }
        let Some(o) = row.as_object_mut() else {
            return hits;
        };
        o.insert("message".into(), Value::from(message));
        o.insert("tags".into(), Value::Object(tags.into_iter().map(|(k, v)| (k, Value::from(v))).collect()));
        if attr != before {
            // The typed attributes are rebuilt from the scanned text ones (the matched values become strings).
            o.insert("attributes".into(), Value::from(crate::pipelines::log::attributes_json("", attr.clone()).to_string()));
            o.insert("attr".into(), Value::Object(attr.into_iter().map(|(k, v)| (k, Value::from(v))).collect()));
        }
        hits
    }
}

/// (org, group id, group, rule id, rule, service) → matches.
type MatchKey = (String, String, String, String, String, String);

/// Every org's scanner, and the matches not written yet.
#[derive(Default)]
pub struct Scanners {
    by_org: RwLock<HashMap<String, (i64, Arc<Scanner>)>>,
    matches: Mutex<HashMap<MatchKey, u64>>,
    salt: OnceLock<Vec<u8>>,
}

impl Scanners {
    /// Hashes are keyed with the server's secret, so they can't be compared across installs.
    pub fn set_secret(&self, secret: &str) {
        let _ = self.salt.set(Sha256::digest(format!("sds:{secret}").as_bytes()).to_vec());
    }

    fn salt_for(&self, org: &str) -> Vec<u8> {
        let mut s = self.salt.get().cloned().unwrap_or_default();
        s.extend_from_slice(org.as_bytes());
        s.push(0);
        s
    }

    /// The org's scanner, when it scans something.
    pub fn get(&self, org: &str) -> Option<Arc<Scanner>> {
        self.by_org.read().unwrap().get(org).map(|(_, s)| s.clone()).filter(|s| !s.is_empty())
    }

    /// Installs an org's scanner compiled from configuration `version`.
    pub fn put(&self, org: &str, version: i64, scanner: Scanner) {
        self.by_org.write().unwrap().insert(org.to_string(), (version, Arc::new(scanner)));
    }

    /// Rebuilds the scanners of the orgs whose configuration changed, and drops those of orgs without groups.
    pub async fn reload(&self, pg: &sqlx::PgPool) -> Result<(), sqlx::Error> {
        let versions: Vec<(String, i64)> =
            sqlx::query_as("SELECT c.org_id, c.version FROM sds_configs c JOIN orgs o ON o.id = c.org_id WHERE o.status = 'active' AND EXISTS (SELECT 1 FROM sds_groups g WHERE g.org_id = c.org_id)")
                .fetch_all(pg)
                .await?;
        let current: HashMap<String, i64> = self.by_org.read().unwrap().iter().map(|(k, (v, _))| (k.clone(), *v)).collect();
        let mut seen = std::collections::HashSet::new();
        for (org, version) in versions {
            seen.insert(org.clone());
            if current.get(&org) == Some(&version) {
                continue;
            }
            let cfg = config::load(pg, &org).await?;
            // Kept even when it scans nothing, so it isn't loaded again until it changes.
            self.put(&org, cfg.version, Scanner::compile(&cfg));
        }
        self.by_org.write().unwrap().retain(|org, _| seen.contains(org));
        Ok(())
    }

    /// Scans a log of `org`, counting its matches; false when the org scans nothing.
    pub fn scan(&self, org: &str, t: &mut Target<'_>) -> bool {
        let Some(scanner) = self.get(org) else {
            return false;
        };
        let service = t.service.to_string();
        let hits = scanner.scan(t, &self.salt_for(org));
        self.count(org, &service, hits);
        true
    }

    /// [`Scanners::scan`] of a JSON row (Vector's path).
    pub fn scan_row(&self, org: &str, row: &mut Value) {
        let Some(scanner) = self.get(org) else { return };
        let service = row.get("service").and_then(Value::as_str).unwrap_or("").to_string();
        let hits = scanner.scan_row(row, &self.salt_for(org));
        self.count(org, &service, hits);
    }

    /// Scans live tail's copy of a log: redacted the same way, not counted.
    pub fn scan_uncounted(&self, org: &str, t: &mut Target<'_>) {
        if let Some(scanner) = self.get(org) {
            scanner.scan(t, &self.salt_for(org));
        }
    }

    fn count(&self, org: &str, service: &str, hits: Vec<Hit>) {
        if hits.is_empty() {
            return;
        }
        let mut m = self.matches.lock().unwrap();
        for h in hits {
            *m.entry((org.to_string(), h.group_id, h.group, h.rule_id, h.rule, service.to_string())).or_insert(0) += h.count;
        }
    }

    /// The matches counted since the last call, as `sds.matches` points at `time_ms`.
    pub fn drain_points(&self, time_ms: i64) -> Vec<crate::log_metrics::Point> {
        let drained = std::mem::take(&mut *self.matches.lock().unwrap());
        let description: Arc<str> = Arc::from("Sensitive data matches found by the Sensitive Data Scanner");
        drained
            .into_iter()
            .map(|((org, group_id, group, rule_id, rule, service), n)| crate::log_metrics::Point {
                event_id: String::new(),
                org,
                metric: MATCH_METRIC.into(),
                description: description.clone(),
                retention: crate::log_metrics::Retention::default(),
                tags: vec![("sds_group".into(), group), ("sds_group_id".into(), group_id), ("sds_rule".into(), rule), ("sds_rule_id".into(), rule_id), ("service".into(), service)],
                time_ms,
                value: crate::log_metrics::PointValue::Count(n),
            })
            .collect()
    }
}

/// Writes the counted matches every minute (`sds.matches`); points are kept for the next minute when ClickHouse fails.
pub async fn write_matches(st: crate::state::AppState, scanners: Arc<Scanners>) {
    let mut tick = tokio::time::interval(std::time::Duration::from_secs(60));
    let mut pending: Vec<crate::log_metrics::Point> = Vec::new();
    loop {
        tick.tick().await;
        let now = chrono::Utc::now().timestamp_millis();
        pending.extend(scanners.drain_points(now - now % 60_000));
        if pending.is_empty() {
            continue;
        }
        let (sums, _) = crate::log_metrics::rows_in_scope(&pending, "observecraft.sds");
        match crate::logproc::insert_rows(&st, "otel.metrics_ingest_sum", sums).await {
            Ok(()) => pending.clear(),
            Err(e) => {
                tracing::warn!(error = %e, points = pending.len(), "writing sensitive data scanner matches failed");
                // Bounded: a long outage loses the oldest minutes' counts.
                if pending.len() > 100_000 {
                    pending.drain(..pending.len() - 100_000);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::config::{Config, GroupRow, RuleRow};
    use super::*;
    use serde_json::json;

    fn group(id: &str, attrs: Value) -> GroupRow {
        GroupRow { id: id.into(), attributes: attrs }
    }

    fn rule(id: &str, group: &str, standard: Option<&str>, attrs: Value) -> RuleRow {
        RuleRow { id: id.into(), group_id: group.into(), standard_pattern_id: standard.map(str::to_string), attributes: attrs }
    }

    fn scanner(groups: Vec<GroupRow>, rules: Vec<RuleRow>) -> Scanner {
        Scanner::compile(&Config { version: 1, groups, rules })
    }

    fn on(query: &str) -> Value {
        json!({ "name": "PCI", "is_enabled": true, "product_list": ["logs"], "filter": { "query": query } })
    }

    struct Log {
        message: String,
        tags: HashMap<String, String>,
        attr: HashMap<String, String>,
    }

    fn log(message: &str, attr: &[(&str, &str)]) -> Log {
        Log { message: message.into(), tags: HashMap::new(), attr: attr.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect() }
    }

    fn run(s: &Scanner, l: &mut Log, id: &str) -> Vec<Hit> {
        s.scan(&mut Target { id, service: "billing", status: "info", host: "", source: "", message: &mut l.message, tags: &mut l.tags, attr: &mut l.attr }, b"salt")
    }

    #[test]
    fn standard_patterns_redact_and_tag() {
        let s = scanner(
            vec![group("g", on("*"))],
            vec![rule("r", "g", Some("credit_card"), json!({ "name": "cards", "is_enabled": true, "text_replacement": { "type": "replacement_string", "replacement_string": "[card]" } }))],
        );
        let mut l = log("paid with 4111 1111 1111 1111", &[("payment.card", "5555-5555-5555-4444"), ("order", "4111 1111 1111 1112")]);
        let hits = run(&s, &mut l, "id-1");
        assert_eq!(l.message, "paid with [card]");
        assert_eq!(l.attr["payment.card"], "[card]");
        assert_eq!(l.attr["order"], "4111 1111 1111 1112", "fails Luhn");
        assert_eq!(l.tags["sensitive_data"], "credit_card");
        assert_eq!(l.tags["sensitive_data_category"], "finance");
        assert_eq!(hits, vec![Hit { group_id: "g".into(), group: "PCI".into(), rule_id: "r".into(), rule: "cards".into(), count: 2 }]);
    }

    #[test]
    fn replacements() {
        let salt = b"k";
        assert_eq!(redact("card 4111111111111111 ok", &[(5, 21)], &Replacement::FromEnd(12), salt), "card 4111************ ok");
        assert_eq!(redact("card 4111111111111111 ok", &[(5, 21)], &Replacement::FromBeginning(12), salt), "card ************1111 ok");
        assert_eq!(redact("ab", &[(0, 2)], &Replacement::FromBeginning(5), salt), "**");
        let hashed = redact("x secret y", &[(2, 8)], &Replacement::Hash, salt);
        assert_eq!(hashed.len(), "x  y".len() + 16);
        assert_eq!(hashed, redact("x secret y", &[(2, 8)], &Replacement::Hash, salt), "stable");
        assert_ne!(hashed, redact("x secret y", &[(2, 8)], &Replacement::Hash, b"other"), "keyed");
        assert_eq!(replacement(Some(&json!({"type": "partial_replacement_from_end", "number_of_chars": 4}))), Replacement::FromEnd(4));
        assert_eq!(replacement(None), Replacement::None);
    }

    #[test]
    fn custom_patterns_namespaces_keywords_and_suppressions() {
        let s = scanner(
            vec![group("g", on("service:billing"))],
            vec![rule(
                "r",
                "g",
                None,
                json!({
                    "name": "customer ids", "is_enabled": true, "pattern": r"C-\d{6}", "namespaces": ["customer"], "excluded_namespaces": ["customer.public"],
                    "tags": ["sensitive_data:customer_id"], "text_replacement": { "type": "hash" },
                    "included_keyword_configuration": { "keywords": ["customer"], "character_count": 10 },
                    "suppressions": { "exact_match": ["C-000000"] }
                }),
            )],
        );
        let mut l = log("customer C-123456 logged in", &[("customer.id", "C-123456"), ("customer.public.id", "C-123456"), ("customer.test", "C-000000"), ("order", "C-123456")]);
        let hits = run(&s, &mut l, "id");
        assert_eq!(l.message, "customer C-123456 logged in", "the message isn't in the namespaces");
        assert_eq!(l.attr["customer.id"].len(), 16, "hashed: {}", l.attr["customer.id"]);
        assert_eq!(l.attr["customer.public.id"], "C-123456", "excluded");
        assert_eq!(l.attr["customer.test"], "C-000000", "suppressed");
        assert_eq!(l.attr["order"], "C-123456");
        assert_eq!(hits[0].count, 1);
        assert_eq!(l.tags["sensitive_data"], "customer_id");

        // The keyword must be within 10 characters before the match (or in the attribute's path).
        let s2 = scanner(
            vec![group("g", on("*"))],
            vec![rule(
                "r",
                "g",
                None,
                json!({ "name": "n", "is_enabled": true, "pattern": r"\d{4}", "text_replacement": { "type": "replacement_string", "replacement_string": "X" }, "included_keyword_configuration": { "keywords": ["pin"], "character_count": 10 } }),
            )],
        );
        let mut l = log("pin: 1234, and much later a number 5678", &[("pin_code", "9999")]);
        run(&s2, &mut l, "id");
        assert_eq!(l.message, "pin: X, and much later a number 5678");
        assert_eq!(l.attr["pin_code"], "X");
    }

    #[test]
    fn group_filters_sampling_and_disabled_rules() {
        let rules =
            || vec![rule("r", "g", Some("email_address"), json!({ "name": "emails", "is_enabled": true, "text_replacement": { "type": "replacement_string", "replacement_string": "[email]" } }))];
        let s = scanner(vec![group("g", on("service:catalog-service"))], rules());
        let mut l = log("from alice@example.com", &[]);
        assert!(run(&s, &mut l, "id").is_empty(), "billing isn't in the filter");
        let mut sampled = on("*");
        sampled["samplings"] = json!([{ "product": "logs", "rate": 0.0 }]);
        assert!(run(&scanner(vec![group("g", sampled)], rules()), &mut l, "id").is_empty());
        let mut half = on("*");
        half["samplings"] = json!([{ "product": "logs", "rate": 50.0 }]);
        let s = scanner(vec![group("g", half)], rules());
        let scanned = (0..1000).filter(|i| !run(&s, &mut log("from alice@example.com", &[]), &format!("log-{i}")).is_empty()).count();
        assert!((400..600).contains(&scanned), "{scanned}");
        let mut disabled = on("*");
        disabled["is_enabled"] = json!(false);
        assert!(scanner(vec![group("g", disabled)], rules()).is_empty());
        assert!(scanner(vec![group("g", on("*"))], vec![rule("r", "g", None, json!({ "name": "n", "is_enabled": false, "pattern": "x" }))]).is_empty());
        let mut only_rum = on("*");
        only_rum["product_list"] = json!(["rum"]);
        assert!(scanner(vec![group("g", only_rum)], rules()).is_empty());
    }

    #[test]
    fn rules_run_in_order_and_the_higher_priority_tag_wins() {
        let s = scanner(
            vec![group("g", on("*"))],
            vec![
                rule("low", "g", None, json!({ "name": "low", "is_enabled": true, "pattern": "secret", "priority": 4, "tags": ["sensitive_data:low"], "text_replacement": { "type": "none" } })),
                rule(
                    "high",
                    "g",
                    None,
                    json!({ "name": "high", "is_enabled": true, "pattern": "secret", "priority": 1, "tags": ["sensitive_data:high"], "text_replacement": { "type": "replacement_string", "replacement_string": "[x]" } }),
                ),
                rule("after", "g", None, json!({ "name": "after", "is_enabled": true, "pattern": "secret", "priority": 1, "tags": ["sensitive_data:after"] })),
            ],
        );
        let mut l = log("a secret", &[]);
        let hits = run(&s, &mut l, "id");
        assert_eq!(l.message, "a [x]");
        assert_eq!(l.tags["sensitive_data"], "high");
        assert_eq!(hits.iter().map(|h| h.rule_id.as_str()).collect::<Vec<_>>(), ["low", "high"], "the third rule sees the redacted text");
    }

    #[test]
    fn json_rows_are_scanned_with_their_typed_attributes() {
        let s = scanner(
            vec![group("g", on("*"))],
            vec![rule("r", "g", Some("us_ssn"), json!({ "name": "ssn", "is_enabled": true, "text_replacement": { "type": "partial_replacement_from_beginning", "number_of_chars": 7 } }))],
        );
        let mut row = json!({ "id": "x", "service": "billing", "message": "ssn 123-45-6789", "tags": {}, "attr": { "user.ssn": "123-45-6789", "n": "1" }, "attributes": "{\"user\":{\"ssn\":\"123-45-6789\"},\"n\":1}" });
        let hits = s.scan_row(&mut row, b"");
        assert_eq!(hits.len(), 1);
        assert_eq!(row["message"], "ssn *******6789");
        assert_eq!(row["attr"]["user.ssn"], "*******6789");
        assert_eq!(row["tags"]["sensitive_data"], "us_ssn");
        let attributes: Value = serde_json::from_str(row["attributes"].as_str().unwrap()).unwrap();
        assert_eq!(attributes["user"]["ssn"], "*******6789");
    }

    /// Against the live test databases: scanners follow the stored configuration, and match points fit the metric table.
    #[tokio::test]
    async fn scanners_follow_postgres_and_matches_are_written() {
        // Inserts go to the test ClickHouse (the server's own setting names another).
        let Some((_, states)) = crate::test_support::replicas_with(1, |c| {
            c.clickhouse_url = std::env::var("OC_TEST_CLICKHOUSE_URL").unwrap_or_default();
            c.clickhouse_user = std::env::var("OC_TEST_CLICKHOUSE_USER").unwrap_or_else(|_| "default".into());
            c.clickhouse_password = std::env::var("OC_TEST_CLICKHOUSE_PASSWORD").unwrap_or_default();
        })
        .await
        else {
            return;
        };
        let st = &states[0];
        let org = crate::test_support::new_org(st, "sds-live").await;
        let attrs = config::group_attributes(&json!({ "name": "PCI", "is_enabled": true, "filter": { "query": "*" } }), None).unwrap();
        let (g, v1) = config::create_group(&st.postgres, &org, attrs).await.unwrap();
        let r = config::rule_attributes(&json!({ "name": "cards", "is_enabled": true, "text_replacement": { "type": "hash" } }), None, Some("credit_card")).unwrap();
        let (_, v2) = config::create_rule(&st.postgres, &org, &g, Some("credit_card"), r).await.unwrap();
        assert!(v2 > v1);
        let scanners = Scanners::default();
        scanners.reload(&st.postgres).await.unwrap();
        let mut l = log("card 4111 1111 1111 1111", &[]);
        assert!(scanners.scan(&org, &mut Target { id: "1", service: "billing", status: "", host: "", source: "", message: &mut l.message, tags: &mut l.tags, attr: &mut l.attr }));
        assert_eq!(l.message.len(), "card ".len() + 16);
        let now = chrono::Utc::now().timestamp_millis();
        let (sums, _) = crate::log_metrics::rows_in_scope(&scanners.drain_points(now - now % 60_000), "observecraft.sds");
        crate::logproc::insert_rows(st, "otel.metrics_ingest_sum", sums).await.unwrap();

        config::delete_group(&st.postgres, &org, &g).await.unwrap();
        scanners.reload(&st.postgres).await.unwrap();
        assert!(scanners.get(&org).is_none(), "an org without groups scans nothing");
        assert!(config::find_group(&st.postgres, &org, &g).await.is_err());
        assert!(matches!(config::reorder_groups(&st.postgres, &org, std::slice::from_ref(&g)).await, Err(config::Error::Invalid(_))));
    }

    #[test]
    fn matches_become_points() {
        let scanners = Scanners::default();
        scanners.put("acme", 1, scanner(vec![group("g", on("*"))], vec![rule("r", "g", Some("email_address"), json!({ "name": "emails", "is_enabled": true }))]));
        let mut l = log("a@example.com b@example.com", &[]);
        assert!(scanners.scan("acme", &mut Target { id: "1", service: "billing", status: "", host: "", source: "", message: &mut l.message, tags: &mut l.tags, attr: &mut l.attr }));
        assert!(!scanners.scan("other", &mut Target { id: "1", service: "billing", status: "", host: "", source: "", message: &mut l.message, tags: &mut l.tags, attr: &mut l.attr }));
        let points = scanners.drain_points(60_000);
        assert_eq!(points.len(), 1);
        assert_eq!(points[0].metric, MATCH_METRIC);
        assert_eq!(points[0].value, crate::log_metrics::PointValue::Count(2));
        assert!(points[0].tags.contains(&("sds_rule".to_string(), "emails".to_string())));
        assert!(scanners.drain_points(60_000).is_empty());
    }
}
