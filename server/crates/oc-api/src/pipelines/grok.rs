//! Grok parsing with Datadog's rule syntax.
//!
//! A parser has match rules and optional support rules, one per line: `name pattern`. A pattern is
//! a regular expression in which `%{MATCHER:attribute:filter}` matches text with a named matcher
//! (`word`, `integer`, `date("dd/MMM/yyyy:HH:mm:ss Z")`, `regex("[a-z]+")`, …), stores it under
//! `attribute` (dots make nested attributes) and optionally converts it with a filter (`number`,
//! `nullIf("-")`, `keyvalue`, `json`, …). `%{rule}` includes another rule. A rule must match the
//! whole value; the first match rule that does wins.

use fancy_regex::Regex;
use serde_json::{Map, Value};

use super::dates::DatePattern;

/// What a successful parse extracted: `(attribute path, value)`, in pattern order. An empty path
/// (`%{data::keyvalue}`) carries an object whose keys go to the top level.
pub struct Parsed {
    pub rule: String,
    pub values: Vec<(String, Value)>,
}

pub struct Grok {
    rules: Vec<Rule>,
}

struct Rule {
    name: String,
    regex: Regex,
    captures: Vec<Capture>,
}

/// One `%{matcher:attribute:filter}` that stores what it matched.
struct Capture {
    group: String,
    path: String,
    kind: Kind,
    filter: Option<Filter>,
}

/// How a matcher's text becomes a value.
enum Kind {
    Str,
    Int,
    Num,
    /// `true` when the text is this word (case-insensitive).
    Bool(String),
    Date(Box<DatePattern>),
    /// A quoted string, without its quotes.
    Unquote,
}

enum Filter {
    Number,
    Integer,
    Boolean,
    NullIf(String),
    Lowercase,
    Uppercase,
    Json,
    Scale(f64),
    Array { open: String, close: String, sep: String },
    KeyValue(Regex),
    DecodeUri,
    QueryString,
}

/// Rules may include each other; deeper nesting than this is taken for a loop.
const MAX_DEPTH: usize = 16;
/// Steps the backtracking engine may take per rule and value before the rule counts as not matching.
const BACKTRACK_LIMIT: usize = 200_000;

const INT: &str = r"[+-]?\d+";
const INT_EXT: &str = r"[+-]?\d+(?:[eE][+-]?\d+)?";
const NUM: &str = r"[+-]?(?:\d+(?:\.\d*)?|\.\d+)";
const NUM_EXT: &str = r"[+-]?(?:\d+(?:\.\d*)?|\.\d+)(?:[eE][+-]?\d+)?";
const IPV4: &str = r"(?:(?:25[0-5]|2[0-4]\d|1\d\d|[1-9]?\d)\.){3}(?:25[0-5]|2[0-4]\d|1\d\d|[1-9]?\d)";
const IPV6: &str = r"(?:[0-9A-Fa-f]{0,4}:){2,7}[0-9A-Fa-f]{0,4}";
const HOSTNAME: &str = r"[A-Za-z0-9][A-Za-z0-9_-]{0,62}(?:\.[A-Za-z0-9_-]{1,63})*\.?";
const DOUBLE_QUOTED: &str = r#""(?:[^"\\]|\\.)*""#;
const SINGLE_QUOTED: &str = r"'(?:[^'\\]|\\.)*'";

impl std::fmt::Debug for Grok {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_list().entries(self.rules.iter().map(|r| &r.name)).finish()
    }
}

impl Grok {
    pub fn compile(support_rules: &str, match_rules: &str) -> Result<Grok, String> {
        let support = rule_lines(support_rules)?;
        let matching = rule_lines(match_rules)?;
        if matching.is_empty() {
            return Err("Add at least one match rule".into());
        }
        // Match rules can include support rules and other match rules.
        let defs: std::collections::HashMap<&str, &str> = support.iter().chain(&matching).map(|(n, p)| (n.as_str(), p.as_str())).collect();
        let mut rules = Vec::with_capacity(matching.len());
        for (name, pattern) in &matching {
            let mut ex = Expander { defs: &defs, captures: Vec::new() };
            let body = ex.expand(pattern, 0).map_err(|e| format!("Rule '{name}': {e}"))?;
            let regex = fancy_regex::RegexBuilder::new(&format!("^(?:{body})$")).backtrack_limit(BACKTRACK_LIMIT).build().map_err(|e| format!("Rule '{name}': invalid pattern ({e})"))?;
            rules.push(Rule { name: name.clone(), regex, captures: ex.captures });
        }
        Ok(Grok { rules })
    }

    /// The first match rule matching the whole `text`, and what it extracted.
    pub fn parse(&self, text: &str, now_ms: i64) -> Option<Parsed> {
        for rule in &self.rules {
            let Ok(Some(c)) = rule.regex.captures(text) else { continue };
            let mut values: Vec<(String, Value)> = Vec::new();
            let mut repeated: Vec<bool> = Vec::new();
            for cap in &rule.captures {
                let Some(m) = c.name(&cap.group) else { continue };
                let Some(v) = cap.kind.convert(m.as_str(), now_ms) else { continue };
                let Some(v) = (match &cap.filter {
                    Some(f) => f.apply(v),
                    None => Some(v),
                }) else {
                    continue;
                };
                // The same attribute matched twice: an array of both.
                match values.iter().position(|(p, _)| *p == cap.path) {
                    Some(i) if repeated[i] => {
                        if let Value::Array(a) = &mut values[i].1 {
                            a.push(v);
                        }
                    }
                    Some(i) => {
                        let first = std::mem::take(&mut values[i].1);
                        values[i].1 = Value::Array(vec![first, v]);
                        repeated[i] = true;
                    }
                    None => {
                        values.push((cap.path.clone(), v));
                        repeated.push(false);
                    }
                }
            }
            return Some(Parsed { rule: rule.name.clone(), values });
        }
        None
    }
}

/// `name pattern` lines; blank lines and `#` comments are skipped.
fn rule_lines(text: &str) -> Result<Vec<(String, String)>, String> {
    let mut out = Vec::new();
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')) {
        let (name, pattern) = line.split_once(char::is_whitespace).unwrap_or((line, ""));
        let pattern = pattern.trim();
        if pattern.is_empty() {
            return Err(format!("Rule '{name}' has no pattern (write one rule per line: name pattern)"));
        }
        out.push((name.to_string(), pattern.to_string()));
    }
    Ok(out)
}

struct Expander<'a> {
    defs: &'a std::collections::HashMap<&'a str, &'a str>,
    captures: Vec<Capture>,
}

/// The parts of `%{matcher(args):attribute:filter(args)}`.
struct Spec {
    matcher: String,
    args: Vec<String>,
    attr: String,
    filter: Option<(String, Vec<String>)>,
}

impl Expander<'_> {
    /// The pattern as a regex, with every `%{…}` replaced.
    fn expand(&mut self, pattern: &str, depth: usize) -> Result<String, String> {
        if depth > MAX_DEPTH {
            return Err("rules include each other in a loop".into());
        }
        let mut out = String::new();
        let mut rest = pattern;
        while let Some(i) = rest.find("%{") {
            out.push_str(&rest[..i]);
            let after = &rest[i + 2..];
            let end = closing_brace(after).ok_or("a %{ is never closed")?;
            let spec = parse_spec(&after[..end])?;
            out.push_str(&self.reference(spec, depth)?);
            rest = &after[end + 1..];
        }
        out.push_str(rest);
        Ok(out)
    }

    fn reference(&mut self, spec: Spec, depth: usize) -> Result<String, String> {
        let (re, kind) = match self.defs.get(spec.matcher.as_str()) {
            Some(pattern) if spec.args.is_empty() => (self.expand(pattern, depth + 1)?, Kind::Str),
            _ => matcher(&spec.matcher, &spec.args)?,
        };
        if spec.attr.is_empty() && spec.filter.is_none() {
            return Ok(format!("(?:{re})"));
        }
        let filter = spec.filter.map(|(name, args)| compile_filter(&name, &args)).transpose()?;
        let group = format!("oc{}", self.captures.len());
        let out = format!("(?P<{group}>{re})");
        self.captures.push(Capture { group, path: spec.attr, kind, filter });
        Ok(out)
    }
}

/// Index of the `}` closing a `%{`, skipping quoted arguments.
fn closing_brace(s: &str) -> Option<usize> {
    let mut quote: Option<char> = None;
    let mut escaped = false;
    for (i, c) in s.char_indices() {
        match quote {
            Some(_) if escaped => escaped = false,
            Some(_) if c == '\\' => escaped = true,
            Some(q) if c == q => quote = None,
            Some(_) => {}
            None if c == '"' || c == '\'' => quote = Some(c),
            None if c == '}' => return Some(i),
            None => {}
        }
    }
    None
}

fn parse_spec(s: &str) -> Result<Spec, String> {
    let mut cur = Cursor { chars: s.chars().collect(), pos: 0 };
    // `%{?>matcher…}` (an atomic marker some upstream rules carry) reads as the matcher.
    cur.skip_prefix("?>");
    let matcher = cur.until(&['(', ':']).trim().to_string();
    if matcher.is_empty() {
        return Err(format!("empty matcher in %{{{s}}}"));
    }
    let args = cur.args()?;
    let mut attr = String::new();
    let mut filter = None;
    if cur.eat(':') {
        attr = cur.until(&[':']).trim().to_string();
        if cur.eat(':') {
            let name = cur.until(&['(']).trim().to_string();
            let fargs = cur.args()?;
            if !name.is_empty() {
                filter = Some((name, fargs));
            }
        }
    }
    if cur.pos < cur.chars.len() {
        return Err(format!("unexpected text in %{{{s}}}"));
    }
    Ok(Spec { matcher, args, attr, filter })
}

struct Cursor {
    chars: Vec<char>,
    pos: usize,
}

impl Cursor {
    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn eat(&mut self, c: char) -> bool {
        let hit = self.peek() == Some(c);
        if hit {
            self.pos += 1;
        }
        hit
    }

    fn skip_prefix(&mut self, prefix: &str) {
        let p: Vec<char> = prefix.chars().collect();
        if self.chars[self.pos..].starts_with(&p) {
            self.pos += p.len();
        }
    }

    fn skip_spaces(&mut self) {
        while self.peek().is_some_and(char::is_whitespace) {
            self.pos += 1;
        }
    }

    fn until(&mut self, stops: &[char]) -> String {
        let start = self.pos;
        while self.peek().is_some_and(|c| !stops.contains(&c)) {
            self.pos += 1;
        }
        self.chars[start..self.pos].iter().collect()
    }

    /// `("a", "b", 3)` → its arguments; nothing when there is no `(`.
    fn args(&mut self) -> Result<Vec<String>, String> {
        let mut out = Vec::new();
        if !self.eat('(') {
            return Ok(out);
        }
        loop {
            self.skip_spaces();
            if self.eat(')') {
                return Ok(out);
            }
            match self.peek() {
                Some(q @ ('"' | '\'')) => {
                    self.pos += 1;
                    out.push(self.quoted(q)?);
                }
                Some(_) => out.push(self.until(&[',', ')']).trim().to_string()),
                None => return Err("unclosed argument list".into()),
            }
            self.skip_spaces();
            if !self.eat(',') && self.peek() != Some(')') {
                return Err("arguments are separated by commas".into());
            }
        }
    }

    /// A quoted argument after its opening quote: `\\` and `\"` are escapes, other backslashes stay.
    fn quoted(&mut self, q: char) -> Result<String, String> {
        let mut s = String::new();
        while let Some(c) = self.peek() {
            self.pos += 1;
            if c == q {
                return Ok(s);
            }
            if c == '\\' {
                match self.peek() {
                    Some(n) if n == '\\' || n == q => {
                        s.push(n);
                        self.pos += 1;
                    }
                    _ => s.push('\\'),
                }
            } else {
                s.push(c);
            }
        }
        Err("unclosed quoted argument".into())
    }
}

fn matcher(name: &str, args: &[String]) -> Result<(String, Kind), String> {
    let s = |re: &str| Ok((re.to_string(), Kind::Str));
    match name {
        "word" => s(r"\b\w+\b"),
        "notSpace" => s(r"\S+"),
        "data" => s(r"[\s\S]*?"),
        "integer" => Ok((INT.into(), Kind::Int)),
        "integerStr" => s(INT),
        "integerExt" => Ok((INT_EXT.into(), Kind::Int)),
        "integerExtStr" => s(INT_EXT),
        "number" => Ok((NUM.into(), Kind::Num)),
        "numberStr" => s(NUM),
        "numberExt" => Ok((NUM_EXT.into(), Kind::Num)),
        "numberExtStr" => s(NUM_EXT),
        "boolean" => {
            let (yes, no) = match args {
                [] => ("true", "false"),
                [yes, no] => (yes.as_str(), no.as_str()),
                _ => return Err("boolean takes no arguments or two (the true and false words)".into()),
            };
            Ok((format!("(?i:{}|{})", fancy_regex::escape(yes), fancy_regex::escape(no)), Kind::Bool(yes.to_string())))
        }
        "ipv4" => s(IPV4),
        "ipv6" => s(IPV6),
        "ip" => s(&format!("{IPV4}|{IPV6}")),
        "hostname" => s(HOSTNAME),
        "ipOrHost" => s(&format!("{IPV4}|{IPV6}|{HOSTNAME}")),
        "port" => s(r"\d{1,5}"),
        "quotedString" => Ok((format!("{DOUBLE_QUOTED}|{SINGLE_QUOTED}"), Kind::Unquote)),
        "doubleQuotedString" => Ok((DOUBLE_QUOTED.into(), Kind::Unquote)),
        "singleQuotedString" => Ok((SINGLE_QUOTED.into(), Kind::Unquote)),
        "uuid" => s(r"[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}"),
        "mac" => s(r"(?:[0-9A-Fa-f]{2}[:-]){5}[0-9A-Fa-f]{2}"),
        "regex" => match args {
            [re] => s(re),
            _ => Err("regex takes one argument: regex(\"pattern\")".into()),
        },
        "date" => match args {
            [p] | [p, _] => {
                let d = DatePattern::compile(p, args.get(1).map(String::as_str))?;
                Ok((d.fragment().to_string(), Kind::Date(Box::new(d))))
            }
            _ => Err("date takes a pattern and an optional time zone: date(\"yyyy-MM-dd HH:mm:ss\", \"UTC\")".into()),
        },
        _ => Err(format!("Unknown matcher '{name}'")),
    }
}

fn compile_filter(name: &str, args: &[String]) -> Result<Filter, String> {
    let arg = |i: usize| args.get(i).map(String::as_str);
    Ok(match name {
        "number" => Filter::Number,
        "integer" => Filter::Integer,
        "boolean" => Filter::Boolean,
        "lowercase" => Filter::Lowercase,
        "uppercase" => Filter::Uppercase,
        "json" => Filter::Json,
        "decodeuricomponent" => Filter::DecodeUri,
        "querystring" => Filter::QueryString,
        "nullIf" => Filter::NullIf(arg(0).ok_or("nullIf takes the value to drop: nullIf(\"-\")")?.to_string()),
        "scale" => Filter::Scale(arg(0).and_then(|a| a.parse().ok()).ok_or("scale takes a number: scale(1000)")?),
        "array" => match args {
            [sep] => Filter::Array { open: String::new(), close: String::new(), sep: sep.clone() },
            [brackets, sep] => {
                let mut b = brackets.chars();
                let (open, close) = (b.next().map(String::from).unwrap_or_default(), b.next().map(String::from).unwrap_or_default());
                Filter::Array { open, close, sep: sep.clone() }
            }
            _ => return Err("array takes a separator, optionally after the brackets: array(\"[]\", \",\")".into()),
        },
        "keyvalue" => Filter::KeyValue(keyvalue_regex(arg(0).unwrap_or("="), arg(1).unwrap_or(""))?),
        "url" | "useragent" | "xml" | "rubyhash" | "geoip" | "cidr" => return Err(format!("The '{name}' filter isn't supported yet")),
        _ => return Err(format!("Unknown filter '{name}'")),
    })
}

/// `key<sep>value` pairs: values are quoted (`"…"`, `'…'`, `<…>`) or runs of word characters,
/// `.-_@` and the extra characters allowed.
fn keyvalue_regex(sep: &str, allowed: &str) -> Result<Regex, String> {
    let extra: String = allowed.chars().map(|c| if matches!(c, '\\' | ']' | '[' | '^' | '-') { format!("\\{c}") } else { c.to_string() }).collect();
    let re = format!(r#"([\w.\-@]+){}(?:"([^"]*)"|'([^']*)'|<([^>]*)>|([\w.\-@{extra}]+))"#, fancy_regex::escape(sep));
    Regex::new(&re).map_err(|e| format!("keyvalue: {e}"))
}

/// A number as JSON: integral values as integers (so `200` doesn't read as `200.0`).
pub fn num(f: f64) -> Option<Value> {
    if f.is_finite() && f.fract() == 0.0 && f.abs() < 9.0e15 {
        Some(Value::from(f as i64))
    } else {
        serde_json::Number::from_f64(f).map(Value::Number)
    }
}

/// A number from a JSON number or a numeric string.
pub fn as_f64(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

impl Kind {
    fn convert(&self, s: &str, now_ms: i64) -> Option<Value> {
        match self {
            Kind::Str => Some(Value::from(s)),
            Kind::Int => s.parse::<i64>().ok().map(Value::from).or_else(|| s.parse::<f64>().ok().filter(|f| f.is_finite()).map(|f| Value::from(f.trunc() as i64))),
            Kind::Num => num(s.parse().ok()?),
            Kind::Bool(yes) => Some(Value::Bool(s.eq_ignore_ascii_case(yes))),
            Kind::Date(p) => p.parse(s, now_ms).map(Value::from),
            Kind::Unquote => {
                let inner = s.get(1..s.len().saturating_sub(1))?;
                let mut out = String::with_capacity(inner.len());
                let mut chars = inner.chars();
                while let Some(c) = chars.next() {
                    out.push(if c == '\\' { chars.next().unwrap_or('\\') } else { c });
                }
                Some(Value::from(out))
            }
        }
    }
}

impl Filter {
    fn apply(&self, v: Value) -> Option<Value> {
        let text = |v: &Value| v.as_str().map(str::to_string).unwrap_or_else(|| v.to_string());
        match self {
            Filter::Number => num(as_f64(&v)?),
            Filter::Integer => as_f64(&v).filter(|f| f.is_finite()).map(|f| Value::from(f.trunc() as i64)),
            Filter::Boolean => match &v {
                Value::Bool(_) => Some(v),
                _ => match text(&v).to_ascii_lowercase().as_str() {
                    "true" => Some(Value::Bool(true)),
                    "false" => Some(Value::Bool(false)),
                    _ => None,
                },
            },
            Filter::NullIf(x) => (text(&v) != *x).then_some(v),
            Filter::Lowercase => Some(v.as_str().map(|s| Value::from(s.to_lowercase())).unwrap_or(v)),
            Filter::Uppercase => Some(v.as_str().map(|s| Value::from(s.to_uppercase())).unwrap_or(v)),
            Filter::Json => serde_json::from_str(v.as_str()?).ok(),
            Filter::Scale(k) => num(as_f64(&v)? * k),
            Filter::Array { open, close, sep } => {
                let s = text(&v);
                let s = s.trim();
                let inner = s.strip_prefix(open.as_str()).unwrap_or(s);
                let inner = inner.strip_suffix(close.as_str()).unwrap_or(inner);
                let items = if sep.is_empty() { vec![inner] } else { inner.split(sep.as_str()).collect() };
                Some(Value::Array(items.into_iter().map(str::trim).filter(|x| !x.is_empty()).map(Value::from).collect()))
            }
            Filter::KeyValue(re) => {
                let s = text(&v);
                let mut out = Map::new();
                for c in re.captures_iter(&s).flatten() {
                    let key = c.get(1).map_or("", |m| m.as_str());
                    let value = (2..=4).find_map(|i| c.get(i)).map(|m| m.as_str()).or_else(|| c.get(5).map(|m| m.as_str().trim_end_matches([',', ';'])));
                    if let Some(value) = value.filter(|x| !x.is_empty()) {
                        out.insert(key.to_string(), Value::from(value));
                    }
                }
                Some(Value::Object(out))
            }
            Filter::DecodeUri => Some(Value::from(decode_uri(&text(&v)))),
            Filter::QueryString => Some(query_string(&text(&v))),
        }
    }
}

/// `%XX` escapes decoded (invalid UTF-8 replaced).
pub fn decode_uri(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = |b: u8| (b as char).to_digit(16);
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(hi), Some(lo)) = (hex(bytes[i + 1]), hex(bytes[i + 2])) {
                out.push((hi * 16 + lo) as u8);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// `a=1&b=x%20y` (with or without a leading `?`) as an object of strings.
pub fn query_string(s: &str) -> Value {
    Value::Object(url::form_urlencoded::parse(s.trim_start_matches('?').as_bytes()).map(|(k, v)| (k.into_owned(), Value::from(v.into_owned()))).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const NOW: i64 = 1_792_933_200_000;

    fn grok(rules: &str) -> Grok {
        Grok::compile("", rules).unwrap()
    }

    /// The extracted values of `rules` on `text` as one JSON object (paths as keys).
    fn parse(rules: &str, text: &str) -> Option<Value> {
        grok(rules).parse(text, NOW).map(|p| Value::Object(p.values.into_iter().collect::<Map<_, _>>()))
    }

    #[test]
    fn matchers_and_their_types() {
        let v = parse(
            r#"r %{word:verb} %{notSpace:path} %{integer:status} %{number:secs} %{numberStr:raw} %{boolean:cached} %{ip:client} %{hostname:host}"#,
            "GET /cart?id=7 200 0.25 1.50 TRUE 10.0.0.12 storefront-api.example.com",
        )
        .unwrap();
        assert_eq!(v, json!({"verb": "GET", "path": "/cart?id=7", "status": 200, "secs": 0.25, "raw": "1.50", "cached": true, "client": "10.0.0.12", "host": "storefront-api.example.com"}));
        let v = parse(
            r#"r %{ipv6:a} %{ipv4:b} %{quotedString:q} %{doubleQuotedString:d} %{regex("[a-f0-9]{6}"):sha} %{data:rest}"#,
            r#"fe80::1:2 192.168.0.1 'it\'s' "say \"hi\"" c0ffee the rest, of it"#,
        );
        assert_eq!(v, Some(json!({"a": "fe80::1:2", "b": "192.168.0.1", "q": "it's", "d": "say \"hi\"", "sha": "c0ffee", "rest": "the rest, of it"})));
        // Integral numbers stay integers; dates become epoch milliseconds.
        assert_eq!(parse("r %{number:n} %{date(\"yyyy-MM-dd HH:mm:ss\"):at}", "3 2026-10-25 13:00:00"), Some(json!({"n": 3, "at": NOW})));
    }

    #[test]
    fn data_is_lazy_and_rules_match_the_whole_value() {
        assert_eq!(parse("r %{data:a}, %{data:b}", "x, y, z"), Some(json!({"a": "x", "b": "y, z"})));
        assert!(parse("r %{word:a}", "two words").is_none());
        assert!(parse("r %{integer:a}", "12abc").is_none());
    }

    #[test]
    fn filters() {
        let v = parse(
            r#"r %{notSpace:a:number} %{notSpace:b:integer} %{notSpace:c:boolean} %{notSpace:d:nullIf("-")} %{word:e:lowercase} %{word:f:uppercase} %{number:g:scale(1000)}"#,
            "1.5 42 false - GET post 0.25",
        )
        .unwrap();
        assert_eq!(v, json!({"a": 1.5, "b": 42, "c": false, "e": "get", "f": "POST", "g": 250}));
        assert_eq!(parse(r#"r %{data:j:json}"#, r#"{"user":{"id":7}}"#), Some(json!({"j": {"user": {"id": 7}}})));
        assert_eq!(parse(r#"r %{data:list:array("[]", ",")}"#, "[a, b,c]"), Some(json!({"list": ["a", "b", "c"]})));
        assert_eq!(parse(r#"r %{notSpace:flags:array(",")}"#, "qr,rd,ra"), Some(json!({"flags": ["qr", "rd", "ra"]})));
        assert_eq!(parse("r %{notSpace:p:decodeuricomponent} %{notSpace:q:querystring}", "a%20b%2Fc%é ?id=7&q=red%20shoes"), Some(json!({"p": "a b/c%é", "q": {"id": "7", "q": "red shoes"}})));
    }

    #[test]
    fn keyvalue_filter() {
        // Default: key=value, quoted values unquoted; an empty attribute merges at the top level.
        assert_eq!(parse("r %{data::keyvalue}", r#"user=jane id=42 note="two words""#), Some(json!({"": {"user": "jane", "id": "42", "note": "two words"}})));
        assert_eq!(parse("r %{data:kv:keyvalue(\":\")}", "a:1 b:2"), Some(json!({"kv": {"a": "1", "b": "2"}})));
        // Custom separator, with commas allowed in values (trailing delimiters are dropped).
        let v = parse(r#"r %{data::keyvalue(": ", ",")}"#, r#"client: 10.0.0.5, server: localhost, request: "GET / HTTP/1.1""#);
        assert_eq!(v, Some(json!({"": {"client": "10.0.0.5", "server": "localhost", "request": "GET / HTTP/1.1"}})));
    }

    #[test]
    fn dotted_attributes_are_paths() {
        let p = grok("r %{integer:http.status_code} %{word:http.method}").parse("404 GET", NOW).unwrap();
        assert_eq!(p.values, vec![("http.status_code".to_string(), json!(404)), ("http.method".to_string(), json!("GET"))]);
    }

    #[test]
    fn first_matching_rule_wins() {
        let g = grok("errors %{word:level}: %{data:msg}\n\n# comment\nany %{data:msg}\nlater %{word:level}: %{data:other}");
        let p = g.parse("ERROR: disk full", NOW).unwrap();
        assert_eq!(p.rule, "errors");
        let p = g.parse("just text", NOW).unwrap();
        assert_eq!(p.rule, "any");
    }

    #[test]
    fn support_rules_and_rule_references() {
        let support = "_ts %{date(\"dd/MMM/yyyy:HH:mm:ss Z\"):date}\n_method %{word:http.method}";
        let rules = "access.common \\[%{_ts}\\] \"(?>%{_method} |)%{notSpace:http.url}\"\naccess.combined %{access.common} %{integer:http.status_code}";
        let g = Grok::compile(support, rules).unwrap();
        let p = g.parse("[13/Jul/2016:10:55:36 +0000] \"GET /cart\" 200", NOW).unwrap();
        assert_eq!(p.rule, "access.combined");
        assert_eq!(p.values.len(), 4);
        assert_eq!(p.values[0], ("date".to_string(), json!(1_468_407_336_000_i64)));
        // Lookarounds work too.
        assert!(Grok::compile("", r#"r %{regex("(?!AUDIT)[A-Z]{5}"):code}"#).unwrap().parse("AUDIT", NOW).is_none());
    }

    #[test]
    fn matches_without_an_attribute_are_not_extracted_and_repeats_make_arrays() {
        assert_eq!(parse("r %{word} %{integer:n}", "abc 5"), Some(json!({"n": 5})));
        assert_eq!(parse("r %{word:tag} %{word:tag}", "a b"), Some(json!({"tag": ["a", "b"]})));
        // An optional part that didn't take part extracts nothing.
        assert_eq!(parse("r %{word:a}( %{word:b})?", "x"), Some(json!({"a": "x"})));
    }

    #[test]
    fn invalid_rules_are_errors() {
        assert!(Grok::compile("", "r %{nope:a}").unwrap_err().contains("nope"));
        assert!(Grok::compile("", "r %{word:a:nope}").unwrap_err().contains("nope"));
        assert!(Grok::compile("", "r %{regex(\"[a-\"):a}").is_err());
        assert!(Grok::compile("loop %{loop}", "r %{loop}").is_err());
        assert!(Grok::compile("", "").is_err(), "at least one match rule");
        assert!(Grok::compile("", "lonely").is_err(), "a rule needs a pattern");
    }
}
