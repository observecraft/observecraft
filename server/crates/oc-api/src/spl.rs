//! Splunk SPL → ObserveCraft log search translation (Logs → Configuration → Migration).
//!
//! - The search part (`index=web status=500 NOT host=db* "timed out"`) becomes our search syntax
//!   (`index:web @http.status_code:500 -host:db* "timed out"`), with Splunk's common field names
//!   mapped to our standard attributes ([`map_field`]).
//! - Transforming commands (`stats`, `timechart`, `top`, `rare`, `chart`, `bin`) become an
//!   [`SplAggregation`]: measures, group-by fields, interval and limit, which the Log Explorer and
//!   log monitors understand.
//! - Macros (`` `name` ``, `` `name(a,b)` ``) expand from the definitions of an uploaded macros file.
//! - Everything else is explained step by step; what has no equivalent (`eval`, `rex`, `join`, OR
//!   across different fields, numeric comparisons, …) is reported as a warning, and the translation
//!   is then not [`complete`](SplTranslation::complete).

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Serialize, Deserialize, ToSchema, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SplStep {
    /// The SPL this step is about.
    pub spl: String,
    /// What it became (empty when it was dropped).
    pub result: String,
    pub note: String,
}

#[derive(Serialize, Deserialize, ToSchema, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SplMeasure {
    /// count | avg | sum | min | max | cardinality | pc50 | pc75 | pc90 | pc95 | pc99
    pub function: String,
    /// The attribute measured (none for `count`).
    pub field: Option<String>,
    /// The SPL name of the result (`count`, `avg(duration)`, or its `as` alias).
    pub name: String,
}

#[derive(Serialize, Deserialize, ToSchema, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SplCondition {
    /// The measure's SPL name (`count`).
    pub measure: String,
    /// > | >= | < | <= | = | !=
    pub op: String,
    pub value: f64,
}

#[derive(Serialize, Deserialize, ToSchema, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SplAggregation {
    /// timeseries | toplist | table | value
    pub viz: String,
    pub measures: Vec<SplMeasure>,
    pub group_by: Vec<String>,
    /// Rollup interval of a timeseries (`5m`, `1h`).
    pub interval: Option<String>,
    pub limit: Option<u32>,
    /// Smallest groups first (`rare`, `sort count`).
    pub ascending: bool,
    /// A `where <measure> > N` after the aggregation: the alert condition of a migrated alert.
    pub condition: Option<SplCondition>,
}

#[derive(Serialize, Deserialize, ToSchema, Clone, Debug, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct SplTranslation {
    /// The search, in our search syntax.
    pub query: String,
    pub aggregation: Option<SplAggregation>,
    /// `earliest=` of the search (`-24h`), if it set one.
    pub earliest: Option<String>,
    /// Attributes listed by `table` / `fields`.
    pub columns: Vec<String>,
    pub explanation: Vec<SplStep>,
    pub warnings: Vec<String>,
    /// Every part was translated (no warnings).
    pub complete: bool,
}

/// A macro definition: its argument names and body (`$arg$` placeholders).
#[derive(Clone, Debug, Default)]
pub struct Macro {
    pub args: Vec<String>,
    pub definition: String,
}

/// Macros by Splunk title: `name` or `name(2)` for one taking two arguments.
pub type Macros = HashMap<String, Macro>;

// ================================================================ entry point

pub fn translate(spl: &str, macros: &Macros) -> SplTranslation {
    let mut t = Tr::default();
    let expanded = expand_macros(spl.trim(), macros, &mut t, 0);
    let segments = split_pipes(&expanded);
    for (i, seg) in segments.iter().enumerate() {
        let seg = seg.trim();
        if seg.is_empty() {
            continue;
        }
        if i == 0 {
            // A pipeline that starts with `|` begins with a generating command (`| tstats`, `| rest`, …).
            if expanded.trim_start().starts_with('|') {
                command(&mut t, seg);
            } else {
                let body = strip_word(seg, "search").unwrap_or(seg);
                search(&mut t, body, seg);
            }
        } else {
            command(&mut t, seg);
        }
    }
    let query = t.terms.join(" ");
    let complete = t.warnings.is_empty();
    SplTranslation { query, aggregation: t.agg, earliest: t.earliest, columns: t.columns, explanation: t.steps, warnings: t.warnings, complete }
}

#[derive(Default)]
struct Tr {
    terms: Vec<String>,
    agg: Option<SplAggregation>,
    earliest: Option<String>,
    columns: Vec<String>,
    steps: Vec<SplStep>,
    warnings: Vec<String>,
    /// `bin _time span=…` before a `stats … by _time`.
    bin_span: Option<String>,
}

impl Tr {
    fn step(&mut self, spl: &str, result: &str, note: &str) {
        self.steps.push(SplStep { spl: spl.to_string(), result: result.to_string(), note: note.to_string() });
    }
    fn warn(&mut self, spl: &str, msg: &str) {
        self.steps.push(SplStep { spl: spl.to_string(), result: String::new(), note: msg.to_string() });
        self.warnings.push(format!("`{}`: {msg}", shorten(spl)));
    }
}

fn shorten(s: &str) -> String {
    let s = s.trim();
    if s.chars().count() > 80 {
        format!("{}…", s.chars().take(79).collect::<String>())
    } else {
        s.to_string()
    }
}

// ================================================================ macros

fn expand_macros(spl: &str, macros: &Macros, t: &mut Tr, depth: usize) -> String {
    if !spl.contains('`') {
        return spl.to_string();
    }
    if depth > 8 {
        t.warnings.push("macros nest more than 8 levels deep: stopped expanding".into());
        return spl.replace('`', "");
    }
    let mut out = String::new();
    let mut rest = spl;
    while let Some(start) = rest.find('`') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        let Some(end) = after.find('`') else {
            out.push_str(&rest[start..]);
            rest = "";
            break;
        };
        let call = after[..end].trim();
        rest = &after[end + 1..];
        let (name, args) = match call.find('(') {
            Some(p) if call.ends_with(')') => (call[..p].trim(), split_args(&call[p + 1..call.len() - 1])),
            _ => (call, vec![]),
        };
        let key = if args.is_empty() { name.to_string() } else { format!("{name}({})", args.len()) };
        match macros.get(&key) {
            Some(m) => {
                let mut body = m.definition.clone();
                for (k, v) in m.args.iter().zip(&args) {
                    body = body.replace(&format!("${k}$"), v);
                }
                let body = expand_macros(&body, macros, t, depth + 1);
                t.step(&format!("`{call}`"), &body, "Macro expanded from the macros file.");
                out.push_str(&body);
            }
            None => {
                t.warn(&format!("`{call}`"), &format!("unknown macro `{key}`: upload the macros file that defines it"));
            }
        }
    }
    out.push_str(rest);
    out
}

fn split_args(s: &str) -> Vec<String> {
    let mut out = vec![];
    let mut cur = String::new();
    let mut quoted = false;
    for c in s.chars() {
        match c {
            '"' => {
                quoted = !quoted;
                cur.push(c);
            }
            ',' if !quoted => out.push(std::mem::take(&mut cur).trim().to_string()),
            c => cur.push(c),
        }
    }
    if !cur.trim().is_empty() || !out.is_empty() {
        out.push(cur.trim().to_string());
    }
    out
}

// ================================================================ lexing

/// Splits on `|` outside quotes, brackets and parentheses.
fn split_pipes(s: &str) -> Vec<String> {
    let mut out = vec![];
    let mut cur = String::new();
    let (mut quoted, mut depth) = (false, 0i32);
    for c in s.chars() {
        match c {
            '"' => quoted = !quoted,
            '(' | '[' if !quoted => depth += 1,
            ')' | ']' if !quoted => depth -= 1,
            '|' if !quoted && depth <= 0 => {
                out.push(std::mem::take(&mut cur));
                continue;
            }
            _ => {}
        }
        cur.push(c);
    }
    out.push(cur);
    out
}

fn strip_word<'a>(s: &'a str, word: &str) -> Option<&'a str> {
    let s = s.trim_start();
    let head = s.get(..word.len())?;
    if head.eq_ignore_ascii_case(word) && s[word.len()..].chars().next().is_none_or(char::is_whitespace) {
        Some(s[word.len()..].trim_start())
    } else {
        None
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Word(String),
    Quoted(String),
    LParen,
    RParen,
    Op(String),
    Comma,
}

fn lex(s: &str) -> Vec<Tok> {
    let mut out = vec![];
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    let mut word = String::new();
    let flush = |word: &mut String, out: &mut Vec<Tok>| {
        if !word.is_empty() {
            out.push(Tok::Word(std::mem::take(word)));
        }
    };
    while i < chars.len() {
        let c = chars[i];
        match c {
            '"' => {
                flush(&mut word, &mut out);
                let mut q = String::new();
                i += 1;
                while i < chars.len() && chars[i] != '"' {
                    if chars[i] == '\\' && i + 1 < chars.len() {
                        i += 1;
                    }
                    q.push(chars[i]);
                    i += 1;
                }
                out.push(Tok::Quoted(q));
            }
            '(' => {
                flush(&mut word, &mut out);
                out.push(Tok::LParen);
            }
            ')' => {
                flush(&mut word, &mut out);
                out.push(Tok::RParen);
            }
            ',' => {
                flush(&mut word, &mut out);
                out.push(Tok::Comma);
            }
            '=' | '!' | '<' | '>' => {
                flush(&mut word, &mut out);
                let mut op = c.to_string();
                if i + 1 < chars.len() && chars[i + 1] == '=' {
                    op.push('=');
                    i += 1;
                }
                if op == "!" {
                    // A lone `!` isn't an SPL operator: keep it in the word.
                    word.push('!');
                } else {
                    out.push(Tok::Op(if op == "==" { "=".into() } else { op }));
                }
            }
            c if c.is_whitespace() => flush(&mut word, &mut out),
            c => word.push(c),
        }
        i += 1;
    }
    flush(&mut word, &mut out);
    out
}

// ================================================================ search expressions

#[derive(Debug, Clone)]
enum Node {
    And(Vec<Node>),
    Or(Vec<Node>),
    Not(Box<Node>),
    Cmp { field: String, op: String, value: String },
    Text(String),
}

struct Parser {
    toks: Vec<Tok>,
    pos: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.pos)
    }
    fn is_word(&self, w: &str) -> bool {
        matches!(self.peek(), Some(Tok::Word(x)) if x == w)
    }
    fn or(&mut self) -> Node {
        let mut parts = vec![self.and()];
        while self.is_word("OR") {
            self.pos += 1;
            parts.push(self.and());
        }
        if parts.len() == 1 {
            parts.pop().unwrap()
        } else {
            Node::Or(parts)
        }
    }
    fn and(&mut self) -> Node {
        let mut parts = vec![];
        while let Some(t) = self.peek() {
            match t {
                Tok::RParen => break,
                Tok::Word(w) if w == "OR" => break,
                Tok::Word(w) if w == "AND" => self.pos += 1,
                _ => match self.unary() {
                    Some(n) => parts.push(n),
                    None => break,
                },
            }
        }
        if parts.len() == 1 {
            parts.pop().unwrap()
        } else {
            Node::And(parts)
        }
    }
    fn unary(&mut self) -> Option<Node> {
        if self.is_word("NOT") {
            self.pos += 1;
            return self.unary().map(|n| Node::Not(Box::new(n)));
        }
        match self.peek()?.clone() {
            Tok::LParen => {
                self.pos += 1;
                let n = self.or();
                if self.peek() == Some(&Tok::RParen) {
                    self.pos += 1;
                }
                Some(n)
            }
            Tok::Word(w) => {
                self.pos += 1;
                if let Some(Tok::Op(op)) = self.peek().cloned() {
                    self.pos += 1;
                    let value = match self.peek().cloned() {
                        Some(Tok::Word(v)) | Some(Tok::Quoted(v)) => {
                            self.pos += 1;
                            v
                        }
                        _ => String::new(),
                    };
                    return Some(Node::Cmp { field: w, op, value });
                }
                Some(Node::Text(w))
            }
            Tok::Quoted(q) => {
                self.pos += 1;
                Some(Node::Text(format!("\"{q}\"")))
            }
            Tok::Op(_) | Tok::Comma | Tok::RParen => {
                self.pos += 1;
                None
            }
        }
    }
}

fn parse_expr(s: &str) -> Node {
    let mut p = Parser { toks: lex(s), pos: 0 };
    let mut parts = vec![];
    while p.pos < p.toks.len() {
        parts.push(p.or());
        if p.peek() == Some(&Tok::RParen) {
            p.pos += 1;
        }
    }
    if parts.len() == 1 {
        parts.pop().unwrap()
    } else {
        Node::And(parts)
    }
}

/// Splunk field → our attribute. `value` disambiguates `status` (an HTTP code or a log level).
pub fn map_field(field: &str, value: Option<&str>) -> (String, &'static str) {
    let f = field.trim();
    let lower = f.to_ascii_lowercase();
    match lower.as_str() {
        "index" => ("index".into(), "Splunk indexes map to log indexes."),
        "sourcetype" => ("source".into(), "The source type becomes the log source."),
        "source" => ("@filename".into(), "Splunk's source (the file or input) becomes @filename."),
        "host" => ("host".into(), ""),
        "service" | "app" => ("service".into(), ""),
        "env" | "environment" => ("env".into(), ""),
        "status" if value.is_none_or(|v| v.chars().next().is_some_and(|c| c.is_ascii_digit())) => ("@http.status_code".into(), "`status` with a numeric value is read as the HTTP status code."),
        "status" | "log_level" | "loglevel" | "level" | "severity" => ("status".into(), "The log level becomes the log status."),
        "method" | "http_method" => ("@http.method".into(), ""),
        "uri" | "uri_path" => ("@http.url_details.path".into(), ""),
        "url" => ("@http.url".into(), ""),
        "useragent" | "http_user_agent" | "user_agent" => ("@http.useragent".into(), ""),
        "clientip" | "client_ip" | "src_ip" | "src" => ("@network.client.ip".into(), ""),
        "dest" | "dest_ip" => ("@network.destination.ip".into(), ""),
        "user" | "username" | "user_id" => ("@usr.id".into(), ""),
        "duration" | "response_time" => ("@duration".into(), ""),
        _ => (format!("@{f}"), ""),
    }
}

fn map_value(attr: &str, v: &str) -> String {
    if attr == "status" {
        return match v.to_ascii_lowercase().as_str() {
            "warning" | "warn" => "warn".into(),
            "err" | "error" => "error".into(),
            "fatal" | "critical" | "crit" | "emerg" => "critical".into(),
            other => other.to_string(),
        };
    }
    v.to_string()
}

fn quote_value(v: &str) -> String {
    if v.is_empty() || v.chars().any(|c| c.is_whitespace() || c == '(' || c == ')') {
        format!("\"{}\"", v.replace('"', ""))
    } else {
        v.to_string()
    }
}

/// Renders a search node as AND-ed terms of our syntax, or explains why it can't.
fn render(n: &Node, t: &mut Tr, out: &mut Vec<String>) {
    match n {
        Node::And(parts) => parts.iter().for_each(|p| render(p, t, out)),
        Node::Text(w) if w == "*" => t.step("*", "", "Matches every event: nothing to add."),
        Node::Text(w) => {
            out.push(w.clone());
            t.step(w, w, "Free text, matched against the log message.");
        }
        Node::Cmp { field, op, value } => cmp(field, op, value, false, t, out),
        Node::Not(inner) => match inner.as_ref() {
            Node::Cmp { field, op, value } => cmp(field, op, value, true, t, out),
            Node::Text(w) => {
                out.push(format!("-{w}"));
                t.step(&format!("NOT {w}"), &format!("-{w}"), "Excluded text.");
            }
            Node::Or(parts) => parts.iter().for_each(|p| render(&Node::Not(Box::new(p.clone())), t, out)),
            other => t.warn(&spl_of(other), "NOT of a group of conditions has no equivalent: negate each term instead"),
        },
        Node::Or(parts) => or(parts, t, out),
    }
}

fn cmp(field: &str, op: &str, value: &str, negate: bool, t: &mut Tr, out: &mut Vec<String>) {
    let spl = format!("{}{field}{op}{value}", if negate { "NOT " } else { "" });
    let lower = field.to_ascii_lowercase();
    if matches!(lower.as_str(), "earliest" | "latest") {
        if lower == "earliest" {
            t.earliest = Some(value.to_string());
        }
        t.step(&spl, "", "The time range comes from the time picker (set from `earliest`).");
        return;
    }
    if (lower == "index" && value == "*") || lower.starts_with('_') || lower == "splunk_server" {
        t.step(&spl, "", "Not needed in ObserveCraft.");
        return;
    }
    let (attr, note) = map_field(field, Some(value));
    let neg = match op {
        "=" => negate,
        "!=" => !negate,
        _ => {
            t.warn(&spl, "numeric comparisons aren't supported in log searches: use a log-based metric or a monitor threshold");
            return;
        }
    };
    let term = format!("{}{attr}:{}", if neg { "-" } else { "" }, quote_value(&map_value(&attr, value)));
    t.step(&spl, &term, note);
    out.push(term);
}

fn or(parts: &[Node], t: &mut Tr, out: &mut Vec<String>) {
    let spl = parts.iter().map(spl_of).collect::<Vec<_>>().join(" OR ");
    // Same field, equality only: key:(a OR b).
    let mut attr: Option<String> = None;
    let mut values = vec![];
    for p in parts {
        match p {
            Node::Cmp { field, op, value } if op == "=" => {
                let (a, _) = map_field(field, Some(value));
                if attr.as_ref().is_some_and(|x| *x != a) {
                    attr = None;
                    values.clear();
                    break;
                }
                values.push(quote_value(&map_value(&a, value)));
                attr = Some(a);
            }
            _ => {
                attr = None;
                values.clear();
                break;
            }
        }
    }
    match attr {
        Some(a) if !values.is_empty() => {
            let term = format!("{a}:({})", values.join(" OR "));
            t.step(&spl, &term, "Values of one attribute, any of them.");
            out.push(term);
        }
        _ => t.warn(&spl, "OR between different fields or free text isn't supported in one search: split it into several searches"),
    }
}

fn spl_of(n: &Node) -> String {
    match n {
        Node::And(p) => p.iter().map(spl_of).collect::<Vec<_>>().join(" "),
        Node::Or(p) => format!("({})", p.iter().map(spl_of).collect::<Vec<_>>().join(" OR ")),
        Node::Not(i) => format!("NOT {}", spl_of(i)),
        Node::Cmp { field, op, value } => format!("{field}{op}{value}"),
        Node::Text(w) => w.clone(),
    }
}

fn search(t: &mut Tr, body: &str, _seg: &str) {
    let node = parse_expr(body);
    let mut out = vec![];
    render(&node, t, &mut out);
    t.terms.extend(out);
}

// ================================================================ commands

fn command(t: &mut Tr, seg: &str) {
    let seg = seg.trim();
    let (name, args) = match seg.find(char::is_whitespace) {
        Some(i) => (&seg[..i], seg[i..].trim()),
        None => (seg, ""),
    };
    match name.to_ascii_lowercase().as_str() {
        "search" => search(t, args, seg),
        "where" => where_cmd(t, args, seg),
        "stats" | "eventstats" | "streamstats" => {
            if name != "stats" {
                t.warnings.push(format!("`{name}` keeps every event: translated as `stats`"));
            }
            stats(t, args, seg, "table")
        }
        "timechart" => timechart(t, args, seg),
        "chart" => chart(t, args, seg),
        "top" | "rare" => top(t, args, seg, name.eq_ignore_ascii_case("rare")),
        "bin" | "bucket" => {
            if let Some(span) = option(args, "span") {
                t.bin_span = Some(span.clone());
                t.step(seg, &format!("interval {span}"), "Time buckets: the rollup interval of the time series.");
            } else {
                t.warn(seg, "only `bin _time span=…` is supported");
            }
        }
        "head" | "limit" => {
            let n = args.split_whitespace().find_map(|w| w.trim_start_matches("limit=").parse::<u32>().ok()).unwrap_or(10);
            if let Some(a) = t.agg.as_mut() {
                a.limit = Some(n);
            }
            t.step(seg, &format!("first {n}"), "The explorer lists the newest events first.");
        }
        "sort" => {
            if let Some(a) = t.agg.as_mut() {
                a.ascending = !args.trim_start().starts_with('-') && !args.contains("desc");
            }
            t.step(seg, "", "Results are sorted by the measure; click a column header to change it.");
        }
        "table" | "fields" => {
            if args.trim_start().starts_with('-') {
                t.step(seg, "", "Hidden columns: not needed, the explorer only shows the columns you add.");
            } else {
                let cols: Vec<String> = args.split([' ', ',']).filter(|c| !c.is_empty() && !c.starts_with('_')).map(|c| map_field(c, None).0).collect();
                t.step(seg, &cols.join(", "), "Columns shown in the log list.");
                t.columns = cols;
            }
        }
        "rename" => t.step(seg, "", "Renamed fields: our results keep the attribute names."),
        "spath" | "kv" | "extract" => t.step(seg, "", "JSON and key=value attributes are parsed at ingestion: nothing to do at search time."),
        "dedup" => t.warn(seg, "dedup has no equivalent in log searches: group by the field instead"),
        "eval" => t.warn(seg, "computed fields aren't supported at search time: compute them at ingestion with a pipeline processor (Arithmetic, String Builder)"),
        "rex" | "regex" => t.warn(seg, "regular expression extraction isn't supported at search time: parse the attribute with a Grok parser in a pipeline"),
        "lookup" | "inputlookup" => t.warn(seg, "lookups aren't supported at search time: enrich logs with a Reference Table lookup processor in a pipeline"),
        "transaction" | "join" | "append" | "appendcols" | "union" | "map" | "multisearch" => t.warn(seg, "combining searches isn't supported: rebuild it as separate searches or a notebook"),
        "tstats" | "rest" | "metadata" | "datamodel" | "pivot" | "makeresults" => t.warn(seg, "generating commands have no log search equivalent"),
        "fillnull" | "makemv" | "mvexpand" | "convert" | "replace" | "addtotals" | "outputlookup" | "collect" | "sendemail" | "iplocation" | "geostats" => {
            t.warn(seg, "no equivalent in log searches")
        }
        _ => t.warn(seg, &format!("unknown command `{name}`")),
    }
}

fn option(args: &str, key: &str) -> Option<String> {
    args.split_whitespace().find_map(|w| w.strip_prefix(&format!("{key}=")).map(|v| v.trim_matches('"').to_string()))
}

fn where_cmd(t: &mut Tr, args: &str, seg: &str) {
    // After an aggregation: `where count > 10` is the alert condition.
    if let Some(a) = t.agg.as_mut() {
        let toks = lex(args);
        if let [Tok::Word(m), Tok::Op(op), Tok::Word(v)] = toks.as_slice() {
            if let (true, Ok(value)) = (a.measures.iter().any(|x| x.name == *m), v.parse::<f64>()) {
                a.condition = Some(SplCondition { measure: m.clone(), op: op.clone(), value });
                t.step(seg, &format!("{m} {op} {v}"), "Condition on the aggregated value: the alert threshold of a monitor.");
                return;
            }
        }
        t.warn(seg, "only `where <measure> <op> <number>` is supported after an aggregation");
        return;
    }
    // Functions: like(), isnull(), isnotnull(), match() (as a wildcard when it's a prefix).
    let a = args.trim();
    let call = |f: &str| a.strip_prefix(f).and_then(|r| r.trim_start().strip_prefix('(')).and_then(|r| r.strip_suffix(')'));
    if let Some(inner) = call("like") {
        let parts = split_args(inner);
        if let [f, pat] = parts.as_slice() {
            let pat = pat.trim_matches('"').replace('%', "*").replace('_', "?");
            let (attr, note) = map_field(f, Some(&pat));
            let term = format!("{attr}:{}", quote_value(&pat));
            t.step(seg, &term, note);
            t.terms.push(term);
            return;
        }
    }
    if let Some(f) = call("isnotnull") {
        let (attr, _) = map_field(f.trim(), None);
        let term = format!("{attr}:*");
        t.step(seg, &term, "The attribute is present.");
        t.terms.push(term);
        return;
    }
    if let Some(f) = call("isnull") {
        let (attr, _) = map_field(f.trim(), None);
        let term = format!("-{attr}:*");
        t.step(seg, &term, "The attribute is missing.");
        t.terms.push(term);
        return;
    }
    if a.contains('(') && !a.starts_with('(') {
        t.warn(seg, "eval functions in `where` aren't supported");
        return;
    }
    // Plain comparisons: same as the search, with == allowed.
    search(t, a, seg);
}

/// Parses `count`, `count(x)`, `avg(x) as y`, … up to `by`. Returns the measures and the group-by fields.
fn measures(args: &str, t: &mut Tr, seg: &str) -> (Vec<SplMeasure>, Vec<String>, Vec<(String, String)>) {
    let toks = lex(args);
    let mut out = vec![];
    let mut by = vec![];
    let mut opts = vec![];
    let mut i = 0;
    while i < toks.len() {
        match &toks[i] {
            Tok::Word(w) if w.eq_ignore_ascii_case("by") || w.eq_ignore_ascii_case("over") => {
                for tk in &toks[i + 1..] {
                    if let Tok::Word(f) = tk {
                        if !f.eq_ignore_ascii_case("by") {
                            by.push(f.clone());
                        }
                    }
                }
                break;
            }
            Tok::Word(k) if matches!(toks.get(i + 1), Some(Tok::Op(o)) if o == "=") => {
                if let Some(Tok::Word(v)) | Some(Tok::Quoted(v)) = toks.get(i + 2) {
                    opts.push((k.to_ascii_lowercase(), v.clone()));
                }
                i += 3;
                continue;
            }
            Tok::Word(f) => {
                let func = f.to_ascii_lowercase();
                let mut field = None;
                let mut j = i + 1;
                if toks.get(j) == Some(&Tok::LParen) {
                    if let Some(Tok::Word(x)) = toks.get(j + 1) {
                        field = Some(x.clone());
                    }
                    while j < toks.len() && toks[j] != Tok::RParen {
                        j += 1;
                    }
                    j += 1;
                }
                let mut name = match &field {
                    Some(x) => format!("{f}({x})"),
                    None => f.clone(),
                };
                if matches!(toks.get(j), Some(Tok::Word(a)) if a.eq_ignore_ascii_case("as")) {
                    if let Some(Tok::Word(alias)) | Some(Tok::Quoted(alias)) = toks.get(j + 1) {
                        name = alias.clone();
                    }
                    j += 2;
                }
                match function(&func) {
                    Some(fun) => {
                        let field = field.filter(|_| fun != "count").map(|x| map_field(&x, None).0);
                        if fun != "count" && field.is_none() {
                            t.warn(seg, &format!("`{f}` needs a field"));
                        } else {
                            out.push(SplMeasure { function: fun.into(), field, name });
                        }
                    }
                    None => t.warn(seg, &format!("the `{f}` function has no equivalent (supported: count, dc, avg, sum, min, max, median, p50–p99)")),
                }
                i = j;
                continue;
            }
            _ => {}
        }
        i += 1;
    }
    (out, by, opts)
}

fn function(f: &str) -> Option<&'static str> {
    Some(match f {
        "count" | "c" => "count",
        "dc" | "distinct_count" | "estdc" => "cardinality",
        "avg" | "mean" => "avg",
        "sum" => "sum",
        "min" => "min",
        "max" => "max",
        "median" | "p50" | "perc50" => "pc50",
        "p75" | "perc75" => "pc75",
        "p90" | "perc90" => "pc90",
        "p95" | "perc95" => "pc95",
        "p99" | "perc99" => "pc99",
        _ => return None,
    })
}

fn set_agg(t: &mut Tr, seg: &str, agg: SplAggregation) {
    if t.agg.is_some() {
        t.warn(seg, "a second aggregation over aggregated results isn't supported");
        return;
    }
    let desc = describe(&agg);
    t.step(seg, &desc, "Aggregation: shown in the Log Explorer's analytics views and used by log monitors.");
    t.agg = Some(agg);
}

fn describe(a: &SplAggregation) -> String {
    let m = a.measures.iter().map(|m| match &m.field {
        Some(f) => format!("{}({f})", m.function),
        None => m.function.clone(),
    });
    let mut s = m.collect::<Vec<_>>().join(", ");
    if !a.group_by.is_empty() {
        s.push_str(&format!(" by {}", a.group_by.join(", ")));
    }
    if let Some(i) = &a.interval {
        s.push_str(&format!(" every {i}"));
    }
    if let Some(l) = a.limit {
        s.push_str(&format!(", top {l}"));
    }
    format!("{} — {s}", a.viz)
}

fn group_fields(by: &[String]) -> (Vec<String>, bool) {
    let time = by.iter().any(|b| b == "_time");
    (by.iter().filter(|b| *b != "_time").map(|b| map_field(b, None).0).collect(), time)
}

fn stats(t: &mut Tr, args: &str, seg: &str, viz: &str) {
    let (ms, by, _) = measures(args, t, seg);
    if ms.is_empty() {
        return;
    }
    let (group_by, time) = group_fields(&by);
    let interval = if time { Some(t.bin_span.clone().unwrap_or_else(|| "1h".into())) } else { None };
    let viz = if time {
        "timeseries"
    } else if group_by.is_empty() {
        "value"
    } else {
        viz
    };
    set_agg(t, seg, SplAggregation { viz: viz.into(), measures: ms, group_by, interval, limit: None, ascending: false, condition: None });
}

fn timechart(t: &mut Tr, args: &str, seg: &str) {
    let (ms, by, opts) = measures(args, t, seg);
    let ms = if ms.is_empty() { vec![SplMeasure { function: "count".into(), field: None, name: "count".into() }] } else { ms };
    let get = |k: &str| opts.iter().find(|(o, _)| o == k).map(|(_, v)| v.clone());
    let interval = get("span").or_else(|| t.bin_span.clone()).or(Some("auto".into()));
    let limit = get("limit").and_then(|v| v.parse().ok());
    let (group_by, _) = group_fields(&by);
    set_agg(t, seg, SplAggregation { viz: "timeseries".into(), measures: ms, group_by, interval, limit, ascending: false, condition: None });
}

fn chart(t: &mut Tr, args: &str, seg: &str) {
    let (ms, by, opts) = measures(args, t, seg);
    if ms.is_empty() {
        return;
    }
    let limit = opts.iter().find(|(o, _)| o == "limit").and_then(|(_, v)| v.parse().ok());
    let (group_by, time) = group_fields(&by);
    let viz = if time { "timeseries" } else { "table" };
    let interval = if time { Some(t.bin_span.clone().unwrap_or_else(|| "auto".into())) } else { None };
    set_agg(t, seg, SplAggregation { viz: viz.into(), measures: ms, group_by, interval, limit, ascending: false, condition: None });
}

fn top(t: &mut Tr, args: &str, seg: &str, rare: bool) {
    let toks = lex(args);
    let mut fields = vec![];
    let mut limit = Some(10);
    let mut i = 0;
    while i < toks.len() {
        match &toks[i] {
            Tok::Word(k) if matches!(toks.get(i + 1), Some(Tok::Op(o)) if o == "=") => {
                if k.eq_ignore_ascii_case("limit") {
                    if let Some(Tok::Word(v)) = toks.get(i + 2) {
                        limit = v.parse().ok().filter(|n| *n > 0);
                    }
                }
                i += 3;
                continue;
            }
            Tok::Word(w) if w.eq_ignore_ascii_case("by") => {}
            Tok::Word(w) => fields.push(map_field(w, None).0),
            _ => {}
        }
        i += 1;
    }
    if fields.is_empty() {
        t.warn(seg, "top needs a field");
        return;
    }
    let ms = vec![SplMeasure { function: "count".into(), field: None, name: "count".into() }];
    set_agg(t, seg, SplAggregation { viz: "toplist".into(), measures: ms, group_by: fields, interval: None, limit, ascending: rare, condition: None });
}

// ================================================================ macros file

/// Macros from the export CSV (`title,definition,args`): `name` or `name(2)` with `args` "a,b".
pub fn macros_from_rows(rows: &[HashMap<String, String>]) -> Macros {
    let mut out = Macros::new();
    for r in rows {
        let Some(title) = r.get("title").filter(|t| !t.trim().is_empty()) else { continue };
        let args: Vec<String> = r.get("args").map(|a| a.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect()).unwrap_or_default();
        out.insert(title.trim().to_string(), Macro { args, definition: r.get("definition").cloned().unwrap_or_default() });
    }
    out
}

// ================================================================ CSV

/// RFC 4180 CSV: quoted fields with doubled quotes and line breaks. Rows as header → value maps.
pub fn parse_csv(text: &str) -> Vec<HashMap<String, String>> {
    let mut rows: Vec<Vec<String>> = vec![];
    let mut row = vec![];
    let mut field = String::new();
    let mut quoted = false;
    let mut chars = text.trim_start_matches('\u{feff}').chars().peekable();
    while let Some(c) = chars.next() {
        if quoted {
            match c {
                '"' if chars.peek() == Some(&'"') => {
                    field.push('"');
                    chars.next();
                }
                '"' => quoted = false,
                c => field.push(c),
            }
            continue;
        }
        match c {
            '"' => quoted = true,
            ',' => row.push(std::mem::take(&mut field)),
            '\r' => {}
            '\n' => {
                row.push(std::mem::take(&mut field));
                rows.push(std::mem::take(&mut row));
            }
            c => field.push(c),
        }
    }
    if !field.is_empty() || !row.is_empty() {
        row.push(field);
        rows.push(row);
    }
    let mut it = rows.into_iter().filter(|r| !(r.len() == 1 && r[0].trim().is_empty()));
    let Some(header) = it.next() else { return vec![] };
    let header: Vec<String> = header.iter().map(|h| h.trim().to_string()).collect();
    it.map(|r| header.iter().cloned().zip(r.into_iter().chain(std::iter::repeat(String::new()))).collect()).collect()
}

// ================================================================ dashboards (Simple XML)

#[derive(Debug, Clone, PartialEq)]
pub struct Panel {
    pub title: String,
    /// chart | table | single | event | map | viz
    pub kind: String,
    pub query: String,
    pub earliest: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SimpleDashboard {
    pub label: Option<String>,
    pub panels: Vec<Panel>,
}

fn xml_text(s: &str) -> String {
    let s = s.trim();
    let s = s.strip_prefix("<![CDATA[").and_then(|x| x.strip_suffix("]]>")).unwrap_or(s);
    s.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'").replace("&amp;", "&").trim().to_string()
}

/// The text of the first `<tag>…</tag>` in `s` (attributes allowed on the opening tag).
fn element<'a>(s: &'a str, tag: &str) -> Option<(&'a str, usize)> {
    let open = format!("<{tag}");
    let mut from = 0;
    loop {
        let start = from + s[from..].find(&open)?;
        let after = &s[start + open.len()..];
        // `<searchString>` must not match `<search`.
        if !after.starts_with(['>', ' ', '\n', '\t', '/']) {
            from = start + open.len();
            continue;
        }
        let gt = start + open.len() + after.find('>')?;
        if s[..gt].ends_with('/') {
            return Some(("", gt + 1));
        }
        let close = format!("</{tag}>");
        let end = gt + 1 + s[gt + 1..].find(&close)?;
        return Some((&s[gt + 1..end], end + close.len()));
    }
}

/// Panels of a Simple XML dashboard: each `<chart|table|single|event|map|viz>` with its `<search><query>`.
pub fn parse_simple_xml(xml: &str) -> Option<SimpleDashboard> {
    if !xml.contains("<dashboard") && !xml.contains("<form") {
        return None;
    }
    let label = element(xml, "label").map(|(t, _)| xml_text(t)).filter(|t| !t.is_empty());
    let mut panels = vec![];
    let kinds = ["chart", "table", "single", "event", "map", "viz"];
    // A visualization takes its own title, or its panel's.
    let chunks: Vec<&str> = if xml.contains("<panel") { xml.split("<panel").skip(1).collect() } else { vec![xml] };
    for chunk in chunks {
        let first_viz = |s: &str| {
            kinds
                .iter()
                .filter_map(|k| {
                    let open = format!("<{k}");
                    let mut from = 0;
                    loop {
                        let i = from + s[from..].find(&open)?;
                        if s[i + open.len()..].starts_with(['>', ' ', '\n', '\t']) {
                            return Some((i, *k));
                        }
                        from = i + open.len();
                    }
                })
                .min_by_key(|(i, _)| *i)
        };
        let panel_title = match (element(chunk, "title"), first_viz(chunk)) {
            (Some((t, end)), Some((i, _))) if end <= i => xml_text(t),
            _ => String::new(),
        };
        let mut rest = chunk;
        while let Some((i, kind)) = first_viz(rest) {
            let Some((body, end)) = element(&rest[i..], kind) else { break };
            let own = element(body, "title").map(|(t, _)| xml_text(t)).filter(|t| !t.is_empty());
            let search = element(body, "search").map(|(s, _)| s).unwrap_or(body);
            if let Some((q, _)) = element(search, "query").or_else(|| element(body, "searchString")) {
                let earliest = element(search, "earliest").map(|(e, _)| xml_text(e)).filter(|e| !e.is_empty());
                panels.push(Panel { title: own.unwrap_or_else(|| panel_title.clone()), kind: kind.to_string(), query: xml_text(q), earliest });
            }
            rest = &rest[i + end..];
        }
    }
    Some(SimpleDashboard { label, panels })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tr(s: &str) -> SplTranslation {
        translate(s, &Macros::new())
    }

    #[test]
    fn plain_searches() {
        assert_eq!(tr("index=web sourcetype=nginx error").query, "index:web source:nginx error");
        assert_eq!(tr("search host=web-01 \"connection refused\"").query, "host:web-01 \"connection refused\"");
        assert_eq!(tr("index=* service=checkout").query, "service:checkout");
        assert_eq!(tr("host=web-* NOT host=web-test").query, "host:web-* -host:web-test");
        assert_eq!(tr("service!=billing").query, "-service:billing");
        assert_eq!(tr("NOT \"health check\"").query, "-\"health check\"");
        let t = tr("index=web status=500");
        assert_eq!(t.query, "index:web @http.status_code:500");
        assert!(t.complete);
    }

    #[test]
    fn log_levels_and_fields() {
        assert_eq!(tr("log_level=ERROR").query, "status:error");
        assert_eq!(tr("level=WARNING service=api").query, "status:warn service:api");
        assert_eq!(tr("clientip=203.0.113.9 method=POST").query, "@network.client.ip:203.0.113.9 @http.method:POST");
        assert_eq!(tr("order_id=A-1").query, "@order_id:A-1");
        assert_eq!(tr("message=\"payment declined\"").query, "@message:\"payment declined\"");
    }

    #[test]
    fn or_of_one_field_is_a_union() {
        assert_eq!(tr("host=a OR host=b").query, "host:(a OR b)");
        assert_eq!(tr("(status=500 OR status=503) index=web").query, "@http.status_code:(500 OR 503) index:web");
        assert_eq!(tr("NOT (host=a OR host=b)").query, "-host:a -host:b");
        let t = tr("host=a OR service=b");
        assert!(!t.complete);
        assert_eq!(t.query, "");
    }

    #[test]
    fn comparisons_and_time() {
        let t = tr("index=web earliest=-24h latest=now duration>500");
        assert_eq!(t.query, "index:web");
        assert_eq!(t.earliest.as_deref(), Some("-24h"));
        assert_eq!(t.warnings.len(), 1);
        assert!(tr("_index_earliest=-1h service=x").complete);
    }

    #[test]
    fn stats_become_aggregations() {
        let t = tr("index=web | stats count by host");
        assert_eq!(t.query, "index:web");
        let a = t.aggregation.unwrap();
        assert_eq!(a.viz, "table");
        assert_eq!(a.measures, vec![SplMeasure { function: "count".into(), field: None, name: "count".into() }]);
        assert_eq!(a.group_by, vec!["host"]);

        let a = tr("service=api | stats avg(duration) as avg_ms, p95(duration), dc(clientip) by service, host").aggregation.unwrap();
        assert_eq!(a.measures.iter().map(|m| m.function.as_str()).collect::<Vec<_>>(), ["avg", "pc95", "cardinality"]);
        assert_eq!(a.measures[0].name, "avg_ms");
        assert_eq!(a.measures[0].field.as_deref(), Some("@duration"));
        assert_eq!(a.measures[2].field.as_deref(), Some("@network.client.ip"));
        assert_eq!(a.group_by, vec!["service", "host"]);

        let a = tr("* | stats count").aggregation.unwrap();
        assert_eq!(a.viz, "value");
    }

    #[test]
    fn timecharts_and_bins() {
        let a = tr("index=web | timechart span=5m count by status").aggregation.unwrap();
        assert_eq!((a.viz.as_str(), a.interval.as_deref()), ("timeseries", Some("5m")));
        assert_eq!(a.group_by, vec!["@http.status_code"]);
        let a = tr("index=web | timechart avg(response_time)").aggregation.unwrap();
        assert_eq!(a.measures[0].field.as_deref(), Some("@duration"));
        assert_eq!(a.interval.as_deref(), Some("auto"));
        let a = tr("index=web | bin _time span=1h | stats count by _time, host").aggregation.unwrap();
        assert_eq!((a.viz.as_str(), a.interval.as_deref()), ("timeseries", Some("1h")));
        assert_eq!(a.group_by, vec!["host"]);
    }

    #[test]
    fn top_rare_head_sort() {
        let a = tr("index=web | top limit=5 uri").aggregation.unwrap();
        assert_eq!((a.viz.as_str(), a.limit, a.ascending), ("toplist", Some(5), false));
        assert_eq!(a.group_by, vec!["@http.url_details.path"]);
        let a = tr("index=web | rare useragent").aggregation.unwrap();
        assert!(a.ascending);
        assert_eq!(a.limit, Some(10));
        let a = tr("index=web | stats count by host | sort -count | head 3").aggregation.unwrap();
        assert_eq!((a.limit, a.ascending), (Some(3), false));
    }

    #[test]
    fn where_clauses() {
        let a = tr("index=web status>=500 | stats count by host | where count > 10");
        let c = a.aggregation.unwrap().condition.unwrap();
        assert_eq!((c.measure.as_str(), c.op.as_str(), c.value), ("count", ">", 10.0));
        assert_eq!(tr("index=web | where host==\"web-1\"").query, "index:web host:web-1");
        assert_eq!(tr("index=web | where like(uri, \"/api/%\")").query, "index:web @http.url_details.path:/api/*");
        assert_eq!(tr("index=web | where isnotnull(user)").query, "index:web @usr.id:*");
        assert_eq!(tr("index=web | where isnull(user)").query, "index:web -@usr.id:*");
        assert!(!tr("index=web | where len(uri) > 3").complete);
    }

    #[test]
    fn unsupported_commands_warn() {
        for q in ["index=web | eval x=1", "index=web | rex field=_raw \"id=(?<id>\\d+)\"", "index=web | dedup host", "index=web | join host [search index=db]", "| tstats count where index=web"] {
            let t = tr(q);
            assert!(!t.complete, "{q}");
            assert!(!t.warnings.is_empty());
        }
        let t = tr("index=web | spath | table host, uri");
        assert!(t.complete);
        assert_eq!(t.columns, vec!["host", "@http.url_details.path"]);
        assert!(!tr("index=web | stats values(host)").complete);
        assert!(!tr("index=web | stats count | stats count").complete);
    }

    #[test]
    fn macros_expand() {
        let mut m = Macros::new();
        m.insert("web_errors".into(), Macro { args: vec![], definition: "index=web status=500".into() });
        m.insert("svc(1)".into(), Macro { args: vec!["name".into()], definition: "service=$name$".into() });
        m.insert("outer".into(), Macro { args: vec![], definition: "`web_errors` host=a".into() });
        assert_eq!(translate("`web_errors` | stats count", &m).query, "index:web @http.status_code:500");
        assert_eq!(translate("`svc(checkout)` error", &m).query, "service:checkout error");
        assert_eq!(translate("`outer`", &m).query, "index:web @http.status_code:500 host:a");
        assert!(!translate("`missing`", &m).complete);
        let rows = parse_csv("title,definition,args\nsvc(1),service=$name$,name\nweb,index=web,\n");
        let m = macros_from_rows(&rows);
        assert_eq!(m["svc(1)"].args, vec!["name"]);
        assert_eq!(translate("`svc(api)` `web`", &m).query, "service:api index:web");
    }

    #[test]
    fn csv_with_quotes_and_newlines() {
        let rows = parse_csv("title,search\r\n\"Errors, web\",\"index=web \"\"x\"\"\nerror\"\nplain,host=a\n");
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0]["title"], "Errors, web");
        assert_eq!(rows[0]["search"], "index=web \"x\"\nerror");
        assert_eq!(rows[1]["search"], "host=a");
    }

    #[test]
    fn simple_xml_panels() {
        let xml = r#"<dashboard version="1.1"><label>Web &amp; API</label><row><panel><title>Errors</title>
            <chart><search><query>index=web status&gt;=500 | timechart count</query><earliest>-24h</earliest></search></chart></panel>
            <panel><table><title>Top hosts</title><search><query><![CDATA[index=web | top host]]></query></search></table></panel>
            <panel><single><searchString>index=web | stats count</searchString></single></panel></row></dashboard>"#;
        let d = parse_simple_xml(xml).unwrap();
        assert_eq!(d.label.as_deref(), Some("Web & API"));
        assert_eq!(d.panels.len(), 3);
        assert_eq!(d.panels[0], Panel { title: "Errors".into(), kind: "chart".into(), query: "index=web status>=500 | timechart count".into(), earliest: Some("-24h".into()) });
        assert_eq!(d.panels[1].title, "Top hosts");
        assert_eq!(d.panels[1].query, "index=web | top host");
        assert_eq!(d.panels[2].kind, "single");
        assert!(parse_simple_xml("{}").is_none());
    }
}
