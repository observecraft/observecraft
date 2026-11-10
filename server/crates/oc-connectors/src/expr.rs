//! The connectors' expression language: what goes between `{{` and `}}` in a manifest (docs/connectors.md,
//! "Expressions"). Values are JSON; there are no loops, assignments or I/O.
//!
//! - Literals: `'text'` or `"text"`, numbers, `true`, `false`, `null`, lists `[a, b]`, objects `{key: a}`.
//! - Paths: a root (`record`, `config`, …) then `.name`, `[0]` (negative from the end), `[expr]`. A missing step is
//!   `null`, never an error.
//! - Operators, loosest first: `c ? a : b`, `a ?? b` (b when a is null or ""), `||`, `&&`, `== != < <= > >= in`
//!   (`not in`), `+ -`, `* / %`, `! -` (unary), then `.`, `[]` and filters `| name(args)`, which bind tightest.
//! - `+` adds numbers, joins lists, merges objects and otherwise concatenates text; with a `null` side it is `null`
//!   (so `': ' + x ?? ''` needs no condition). Arithmetic on `null` is `null`.
//! - Truthiness: `null`, `false`, `0`, `""`, `[]` and `{}` are false.

use std::collections::BTreeSet;

use serde_json::{json, Map, Number, Value};

#[derive(Clone, Debug, PartialEq)]
pub enum Expr {
    Lit(Value),
    Var(String),
    Get(Box<Expr>, Box<Expr>),
    List(Vec<Expr>),
    Obj(Vec<(String, Expr)>),
    Not(Box<Expr>),
    Neg(Box<Expr>),
    Bin(Op, Box<Expr>, Box<Expr>),
    Cond(Box<Expr>, Box<Expr>, Box<Expr>),
    Filter(Box<Expr>, String, Vec<Expr>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op {
    Coalesce,
    Or,
    And,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    In,
    NotIn,
    Add,
    Sub,
    Mul,
    Div,
    Rem,
}

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    Num(f64),
    Str(String),
    Ident(String),
    P(&'static str),
}

const PUNCT: [&str; 26] = ["??", "||", "&&", "==", "!=", "<=", ">=", ".", "[", "]", "(", ")", "{", "}", ",", "?", ":", "!", "<", ">", "+", "-", "*", "/", "%", "|"];

fn lex(src: &str) -> Result<Vec<Tok>, String> {
    let chars: Vec<char> = src.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            i += 1;
        } else if c == '\'' || c == '"' {
            let mut s = String::new();
            i += 1;
            loop {
                match chars.get(i) {
                    None => return Err("a text literal is not closed".into()),
                    Some(&q) if q == c => break,
                    Some('\\') => {
                        i += 1;
                        match chars.get(i) {
                            Some('n') => s.push('\n'),
                            Some('t') => s.push('\t'),
                            Some(&e) => s.push(e),
                            None => return Err("a text literal is not closed".into()),
                        }
                    }
                    Some(&ch) => s.push(ch),
                }
                i += 1;
            }
            i += 1;
            out.push(Tok::Str(s));
        } else if c.is_ascii_digit() {
            let start = i;
            while i < chars.len() && (chars[i].is_ascii_digit() || (chars[i] == '.' && chars.get(i + 1).is_some_and(|d| d.is_ascii_digit()))) {
                i += 1;
            }
            let text: String = chars[start..i].iter().collect();
            out.push(Tok::Num(text.parse().map_err(|_| format!("{text} is not a number"))?));
        } else if c.is_alphabetic() || c == '_' || c == '$' {
            let start = i;
            while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_' || chars[i] == '$') {
                i += 1;
            }
            out.push(Tok::Ident(chars[start..i].iter().collect()));
        } else {
            let rest: String = chars[i..chars.len().min(i + 2)].iter().collect();
            let p = PUNCT.iter().find(|p| rest.starts_with(**p)).ok_or_else(|| format!("unexpected character {c:?}"))?;
            i += p.chars().count();
            out.push(Tok::P(p));
        }
    }
    Ok(out)
}

struct Parser {
    toks: Vec<Tok>,
    pos: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.pos)
    }

    fn is(&self, p: &str) -> bool {
        matches!(self.peek(), Some(Tok::P(q)) if *q == p)
    }

    fn is_word(&self, w: &str) -> bool {
        matches!(self.peek(), Some(Tok::Ident(q)) if q == w)
    }

    fn eat(&mut self, p: &str) -> bool {
        if self.is(p) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn expect(&mut self, p: &str) -> Result<(), String> {
        if self.eat(p) {
            Ok(())
        } else {
            Err(format!("expected {p:?}"))
        }
    }

    fn expr(&mut self) -> Result<Expr, String> {
        let cond = self.coalesce()?;
        if self.eat("?") {
            let a = self.expr()?;
            self.expect(":")?;
            let b = self.expr()?;
            return Ok(Expr::Cond(Box::new(cond), Box::new(a), Box::new(b)));
        }
        Ok(cond)
    }

    fn binary(&mut self, ops: &[(&str, Op)], next: fn(&mut Self) -> Result<Expr, String>) -> Result<Expr, String> {
        let mut left = next(self)?;
        'outer: loop {
            for (p, op) in ops {
                if self.eat(p) {
                    let right = next(self)?;
                    left = Expr::Bin(*op, Box::new(left), Box::new(right));
                    continue 'outer;
                }
            }
            return Ok(left);
        }
    }

    fn coalesce(&mut self) -> Result<Expr, String> {
        self.binary(&[("??", Op::Coalesce)], Self::or)
    }

    fn or(&mut self) -> Result<Expr, String> {
        self.binary(&[("||", Op::Or)], Self::and)
    }

    fn and(&mut self) -> Result<Expr, String> {
        self.binary(&[("&&", Op::And)], Self::cmp)
    }

    fn cmp(&mut self) -> Result<Expr, String> {
        let left = self.add()?;
        let op = if self.eat("==") {
            Op::Eq
        } else if self.eat("!=") {
            Op::Ne
        } else if self.eat("<=") {
            Op::Le
        } else if self.eat(">=") {
            Op::Ge
        } else if self.eat("<") {
            Op::Lt
        } else if self.eat(">") {
            Op::Gt
        } else if self.is_word("in") {
            self.pos += 1;
            Op::In
        } else if self.is_word("not") && matches!(self.toks.get(self.pos + 1), Some(Tok::Ident(w)) if w == "in") {
            self.pos += 2;
            Op::NotIn
        } else {
            return Ok(left);
        };
        let right = self.add()?;
        Ok(Expr::Bin(op, Box::new(left), Box::new(right)))
    }

    fn add(&mut self) -> Result<Expr, String> {
        self.binary(&[("+", Op::Add), ("-", Op::Sub)], Self::mul)
    }

    fn mul(&mut self) -> Result<Expr, String> {
        self.binary(&[("*", Op::Mul), ("/", Op::Div), ("%", Op::Rem)], Self::unary)
    }

    fn unary(&mut self) -> Result<Expr, String> {
        if self.eat("!") {
            return Ok(Expr::Not(Box::new(self.unary()?)));
        }
        if self.eat("-") {
            return Ok(Expr::Neg(Box::new(self.unary()?)));
        }
        self.postfix()
    }

    fn postfix(&mut self) -> Result<Expr, String> {
        let mut e = self.primary()?;
        loop {
            if self.eat(".") {
                match self.toks.get(self.pos).cloned() {
                    Some(Tok::Ident(name)) => {
                        self.pos += 1;
                        e = Expr::Get(Box::new(e), Box::new(Expr::Lit(Value::String(name))));
                    }
                    Some(Tok::Num(n)) if n.fract() == 0.0 => {
                        self.pos += 1;
                        e = Expr::Get(Box::new(e), Box::new(Expr::Lit(json!(n as i64))));
                    }
                    _ => return Err("expected a name after '.'".into()),
                }
            } else if self.eat("[") {
                let index = self.expr()?;
                self.expect("]")?;
                e = Expr::Get(Box::new(e), Box::new(index));
            } else if self.eat("|") {
                let Some(Tok::Ident(name)) = self.toks.get(self.pos).cloned() else { return Err("expected a filter name after '|'".into()) };
                self.pos += 1;
                let mut args = Vec::new();
                if self.eat("(") && !self.eat(")") {
                    loop {
                        args.push(self.expr()?);
                        if self.eat(")") {
                            break;
                        }
                        self.expect(",")?;
                    }
                }
                check_filter(&name, args.len())?;
                e = Expr::Filter(Box::new(e), name, args);
            } else {
                return Ok(e);
            }
        }
    }

    fn primary(&mut self) -> Result<Expr, String> {
        let Some(tok) = self.toks.get(self.pos).cloned() else { return Err("the expression ends too early".into()) };
        self.pos += 1;
        match tok {
            Tok::Num(n) => Ok(Expr::Lit(number(n))),
            Tok::Str(s) => Ok(Expr::Lit(Value::String(s))),
            Tok::Ident(w) => Ok(match w.as_str() {
                "true" => Expr::Lit(Value::Bool(true)),
                "false" => Expr::Lit(Value::Bool(false)),
                "null" => Expr::Lit(Value::Null),
                _ => Expr::Var(w),
            }),
            Tok::P("(") => {
                let e = self.expr()?;
                self.expect(")")?;
                Ok(e)
            }
            Tok::P("[") => {
                let mut items = Vec::new();
                if !self.eat("]") {
                    loop {
                        items.push(self.expr()?);
                        if self.eat("]") {
                            break;
                        }
                        self.expect(",")?;
                    }
                }
                Ok(Expr::List(items))
            }
            Tok::P("{") => {
                let mut fields = Vec::new();
                if !self.eat("}") {
                    loop {
                        let key = match self.toks.get(self.pos).cloned() {
                            Some(Tok::Ident(k)) | Some(Tok::Str(k)) => k,
                            _ => return Err("expected a key in an object literal".into()),
                        };
                        self.pos += 1;
                        self.expect(":")?;
                        fields.push((key, self.expr()?));
                        if self.eat("}") {
                            break;
                        }
                        self.expect(",")?;
                    }
                }
                Ok(Expr::Obj(fields))
            }
            Tok::P(p) => Err(format!("unexpected {p:?}")),
        }
    }
}

/// Parses an expression (the text between `{{` and `}}`).
pub fn parse(src: &str) -> Result<Expr, String> {
    let mut p = Parser { toks: lex(src)?, pos: 0 };
    if p.toks.is_empty() {
        return Err("empty expression".into());
    }
    let e = p.expr()?;
    if p.pos < p.toks.len() {
        return Err(format!("unexpected {:?} after the expression", p.toks[p.pos]));
    }
    Ok(e)
}

impl Expr {
    /// The root variables the expression reads (`record`, `config`, …).
    pub fn roots(&self, out: &mut BTreeSet<String>) {
        match self {
            Expr::Lit(_) => {}
            Expr::Var(v) => {
                out.insert(v.clone());
            }
            Expr::Get(a, b) | Expr::Bin(_, a, b) => {
                a.roots(out);
                b.roots(out);
            }
            Expr::List(items) => items.iter().for_each(|e| e.roots(out)),
            Expr::Obj(fields) => fields.iter().for_each(|(_, e)| e.roots(out)),
            Expr::Not(a) | Expr::Neg(a) => a.roots(out),
            Expr::Cond(a, b, c) => {
                a.roots(out);
                b.roots(out);
                c.roots(out);
            }
            Expr::Filter(a, name, args) => {
                a.roots(out);
                let mut inner = BTreeSet::new();
                args.iter().for_each(|e| e.roots(&mut inner));
                // `map(…)` reads each element as `it`.
                if name == "map" {
                    inner.remove("it");
                }
                out.extend(inner);
            }
        }
    }

    /// The variables read with their first key (`config.base_url` → `("config", Some("base_url"))`).
    pub fn paths(&self, out: &mut Vec<(String, Option<String>)>) {
        match self {
            Expr::Var(v) => out.push((v.clone(), None)),
            Expr::Get(base, key) => match (base.as_ref(), key.as_ref()) {
                (Expr::Var(root), Expr::Lit(Value::String(k))) => out.push((root.clone(), Some(k.clone()))),
                _ => {
                    base.paths(out);
                    key.paths(out);
                }
            },
            Expr::Lit(_) => {}
            Expr::List(items) => items.iter().for_each(|e| e.paths(out)),
            Expr::Obj(fields) => fields.iter().for_each(|(_, e)| e.paths(out)),
            Expr::Not(a) | Expr::Neg(a) => a.paths(out),
            Expr::Bin(_, a, b) => {
                a.paths(out);
                b.paths(out);
            }
            Expr::Cond(a, b, c) => {
                a.paths(out);
                b.paths(out);
                c.paths(out);
            }
            Expr::Filter(a, name, args) => {
                a.paths(out);
                let mut inner = Vec::new();
                args.iter().for_each(|e| e.paths(&mut inner));
                out.extend(inner.into_iter().filter(|(root, _)| name != "map" || root != "it"));
            }
        }
    }

    /// The expression's value in `scope` (a map of root variables).
    pub fn eval(&self, scope: &Map<String, Value>) -> Value {
        match self {
            Expr::Lit(v) => v.clone(),
            Expr::Var(name) => scope.get(name).cloned().unwrap_or(Value::Null),
            Expr::Get(base, key) => get(&base.eval(scope), &key.eval(scope)),
            Expr::List(items) => Value::Array(items.iter().map(|e| e.eval(scope)).collect()),
            Expr::Obj(fields) => Value::Object(fields.iter().map(|(k, e)| (k.clone(), e.eval(scope))).collect()),
            Expr::Not(a) => Value::Bool(!truthy(&a.eval(scope))),
            Expr::Neg(a) => a.eval(scope).as_f64().map_or(Value::Null, |n| number(-n)),
            Expr::Cond(c, a, b) => {
                if truthy(&c.eval(scope)) {
                    a.eval(scope)
                } else {
                    b.eval(scope)
                }
            }
            Expr::Bin(op, a, b) => binary(*op, a, b, scope),
            // `list | map(it.x)`: the expression for each element (`it`).
            Expr::Filter(base, name, args) if name == "map" => match base.eval(scope) {
                Value::Array(items) => {
                    let mut s = scope.clone();
                    Value::Array(
                        items
                            .into_iter()
                            .map(|it| {
                                s.insert("it".into(), it);
                                args[0].eval(&s)
                            })
                            .collect(),
                    )
                }
                _ => Value::Null,
            },
            Expr::Filter(base, name, args) => {
                let args: Vec<Value> = args.iter().map(|e| e.eval(scope)).collect();
                filter(name, base.eval(scope), &args)
            }
        }
    }
}

/// A number as JSON: integral values as integers.
pub fn number(n: f64) -> Value {
    if n.is_finite() && n.fract() == 0.0 && n.abs() < 9.0e15 {
        Value::Number(Number::from(n as i64))
    } else {
        Number::from_f64(n).map_or(Value::Null, Value::Number)
    }
}

pub fn truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().is_some_and(|f| f != 0.0),
        Value::String(s) => !s.is_empty(),
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
    }
}

/// A value as text: strings as they are, `null` as "", others as JSON.
pub fn text(v: &Value) -> String {
    match v {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

fn missing(v: &Value) -> bool {
    matches!(v, Value::Null) || v.as_str().is_some_and(str::is_empty)
}

fn get(base: &Value, key: &Value) -> Value {
    match (base, key) {
        (Value::Object(o), Value::String(k)) => o.get(k).cloned().unwrap_or(Value::Null),
        (Value::Object(o), Value::Number(n)) => o.get(&n.to_string()).cloned().unwrap_or(Value::Null),
        (Value::Array(a), Value::Number(n)) => {
            let Some(i) = n.as_i64() else { return Value::Null };
            let i = if i < 0 { a.len() as i64 + i } else { i };
            usize::try_from(i).ok().and_then(|i| a.get(i)).cloned().unwrap_or(Value::Null)
        }
        _ => Value::Null,
    }
}

fn equal(a: &Value, b: &Value) -> bool {
    match (a.as_f64(), b.as_f64()) {
        (Some(x), Some(y)) if a.is_number() && b.is_number() => x == y,
        _ => a == b,
    }
}

fn contains(hay: &Value, needle: &Value) -> bool {
    match hay {
        Value::Array(a) => a.iter().any(|v| equal(v, needle)),
        Value::Object(o) => needle.as_str().is_some_and(|k| o.contains_key(k)),
        Value::String(s) => !missing(needle) && s.contains(&text(needle)),
        _ => false,
    }
}

fn order(a: &Value, b: &Value) -> Option<std::cmp::Ordering> {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => x.as_f64()?.partial_cmp(&y.as_f64()?),
        (Value::String(x), Value::String(y)) => Some(x.cmp(y)),
        _ => None,
    }
}

fn binary(op: Op, a: &Expr, b: &Expr, scope: &Map<String, Value>) -> Value {
    let left = a.eval(scope);
    match op {
        Op::Coalesce => {
            if missing(&left) {
                b.eval(scope)
            } else {
                left
            }
        }
        Op::Or => Value::Bool(truthy(&left) || truthy(&b.eval(scope))),
        Op::And => Value::Bool(truthy(&left) && truthy(&b.eval(scope))),
        _ => {
            let right = b.eval(scope);
            match op {
                Op::Eq => Value::Bool(equal(&left, &right)),
                Op::Ne => Value::Bool(!equal(&left, &right)),
                Op::Lt => Value::Bool(order(&left, &right) == Some(std::cmp::Ordering::Less)),
                Op::Le => Value::Bool(matches!(order(&left, &right), Some(std::cmp::Ordering::Less | std::cmp::Ordering::Equal))),
                Op::Gt => Value::Bool(order(&left, &right) == Some(std::cmp::Ordering::Greater)),
                Op::Ge => Value::Bool(matches!(order(&left, &right), Some(std::cmp::Ordering::Greater | std::cmp::Ordering::Equal))),
                Op::In => Value::Bool(contains(&right, &left)),
                Op::NotIn => Value::Bool(!contains(&right, &left)),
                Op::Add => add(left, right),
                _ => {
                    let (Some(x), Some(y)) = (num(&left), num(&right)) else { return Value::Null };
                    match op {
                        Op::Sub => number(x - y),
                        Op::Mul => number(x * y),
                        Op::Div if y != 0.0 => number(x / y),
                        Op::Rem if y != 0.0 => number(x % y),
                        _ => Value::Null,
                    }
                }
            }
        }
    }
}

/// A number, from a number or numeric text.
fn num(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.trim().parse().ok(),
        Value::Bool(b) => Some(if *b { 1.0 } else { 0.0 }),
        _ => None,
    }
}

fn add(left: Value, right: Value) -> Value {
    match (left, right) {
        (Value::Null, _) | (_, Value::Null) => Value::Null,
        (Value::Number(x), Value::Number(y)) => number(x.as_f64().unwrap_or(0.0) + y.as_f64().unwrap_or(0.0)),
        (Value::Array(mut x), Value::Array(y)) => {
            x.extend(y);
            Value::Array(x)
        }
        (Value::Object(mut x), Value::Object(y)) => {
            x.extend(y);
            Value::Object(x)
        }
        (x, y) => Value::String(text(&x) + &text(&y)),
    }
}

// ================================================================ filters

/// Every filter, with its number of arguments (min, max).
pub const FILTERS: &[(&str, usize, usize)] = &[
    ("map", 1, 1),
    ("abs", 0, 0),
    ("base64", 0, 0),
    ("base64_decode", 0, 0),
    ("bool", 0, 0),
    ("ceil", 0, 0),
    ("compact", 0, 0),
    ("contains", 1, 1),
    ("default", 1, 1),
    ("endswith", 1, 1),
    ("entries", 0, 0),
    ("epoch_ms", 0, 1),
    ("epoch_s", 0, 1),
    ("explode", 1, 1),
    ("first", 0, 0),
    ("flatten", 0, 0),
    ("float", 0, 0),
    ("floor", 0, 0),
    ("from_entries", 0, 0),
    ("get", 1, 1),
    ("host", 0, 0),
    ("index_by", 1, 1),
    ("int", 0, 0),
    ("join", 1, 1),
    ("json", 0, 0),
    ("keys", 0, 0),
    ("last", 0, 0),
    ("len", 0, 0),
    ("lower", 0, 0),
    ("matches", 1, 1),
    ("max", 0, 1),
    ("min", 0, 1),
    ("nonempty", 0, 0),
    ("parse_json", 0, 0),
    ("pluck", 1, 1),
    ("replace", 2, 2),
    ("rfc3339", 0, 0),
    ("rfc3339_s", 0, 0),
    ("round", 0, 1),
    ("sha256", 0, 0),
    ("slice", 1, 2),
    ("sort", 0, 1),
    ("split", 1, 1),
    ("startswith", 1, 1),
    ("str", 0, 0),
    ("strftime", 1, 2),
    ("sum", 0, 0),
    ("to_objects", 1, 1),
    ("trim", 0, 0),
    ("unique", 0, 0),
    ("upper", 0, 0),
    ("urlencode", 0, 0),
    ("values", 0, 0),
    ("where", 1, 2),
    ("with", 2, 2),
];

fn check_filter(name: &str, args: usize) -> Result<(), String> {
    let (_, min, max) = FILTERS.iter().find(|(n, _, _)| *n == name).ok_or_else(|| format!("unknown filter {name:?}"))?;
    if args < *min || args > *max {
        return Err(if min == max { format!("filter {name} takes {min} argument(s)") } else { format!("filter {name} takes {min} to {max} arguments") });
    }
    Ok(())
}

/// Epoch milliseconds of a time: RFC 3339 / ISO 8601 text (with or without a zone, then UTC), `YYYY-MM-DD`, RFC 2822,
/// or a number of seconds, milliseconds, microseconds or nanoseconds since the epoch (told apart by size).
pub fn to_ms(v: &Value) -> Option<i64> {
    to_ms_in(v, None)
}

/// A fixed offset (`+02:00`, `-0500`, `Z`) or an IANA zone (`Europe/Bucharest`).
#[derive(Clone, Copy)]
pub enum Zone {
    Fixed(chrono::FixedOffset),
    Iana(chrono_tz::Tz),
}

pub fn zone(name: &str) -> Option<Zone> {
    let name = name.trim();
    if name.eq_ignore_ascii_case("utc") || name == "Z" {
        return chrono::FixedOffset::east_opt(0).map(Zone::Fixed);
    }
    if let Ok(tz) = name.parse::<chrono_tz::Tz>() {
        return Some(Zone::Iana(tz));
    }
    let probe = format!("2000-01-01T00:00:00{name}");
    chrono::DateTime::parse_from_rfc3339(&probe).ok().map(|t| Zone::Fixed(*t.offset())).or_else(|| chrono::DateTime::parse_from_str(&format!("2000-01-01 00:00:00 {name}"), "%Y-%m-%d %H:%M:%S %z").ok().map(|t| Zone::Fixed(*t.offset())))
}

fn local_ms(t: chrono::NaiveDateTime, tz: Option<Zone>) -> Option<i64> {
    use chrono::TimeZone;
    match tz {
        None => Some(t.and_utc().timestamp_millis()),
        Some(Zone::Fixed(o)) => o.from_local_datetime(&t).earliest().map(|t| t.timestamp_millis()),
        Some(Zone::Iana(z)) => z.from_local_datetime(&t).earliest().map(|t| t.timestamp_millis()),
    }
}

/// [`to_ms`], reading times without a zone in `tz` (UTC when None).
pub fn to_ms_in(v: &Value, tz: Option<Zone>) -> Option<i64> {
    let from_number = |n: f64| -> i64 {
        let a = n.abs();
        if a >= 1e17 {
            (n / 1e6) as i64
        } else if a >= 1e14 {
            (n / 1e3) as i64
        } else if a >= 1e11 {
            n as i64
        } else {
            (n * 1000.0) as i64
        }
    };
    match v {
        Value::Number(n) => n.as_f64().map(from_number),
        Value::String(s) => {
            let s = s.trim();
            if let Ok(n) = s.parse::<f64>() {
                return Some(from_number(n));
            }
            if let Ok(t) = chrono::DateTime::parse_from_rfc3339(s) {
                return Some(t.timestamp_millis());
            }
            for fmt in ["%Y-%m-%dT%H:%M:%S%.f", "%Y-%m-%d %H:%M:%S%.f", "%Y-%m-%dT%H:%M:%S", "%Y-%m-%d %H:%M:%S"] {
                if let Ok(t) = chrono::NaiveDateTime::parse_from_str(s, fmt) {
                    return local_ms(t, tz);
                }
            }
            for fmt in ["%Y-%m-%d %H:%M:%S%.f %z", "%Y-%m-%dT%H:%M:%S%.f%z"] {
                if let Ok(t) = chrono::DateTime::parse_from_str(s, fmt) {
                    return Some(t.timestamp_millis());
                }
            }
            if let Ok(d) = chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d") {
                return d.and_hms_opt(0, 0, 0).and_then(|t| local_ms(t, tz));
            }
            chrono::DateTime::parse_from_rfc2822(s).ok().map(|t| t.timestamp_millis())
        }
        _ => None,
    }
}

fn format_ms(ms: i64, fmt: &str) -> Value {
    format_in(ms, fmt, None)
}

fn format_in(ms: i64, fmt: &str, tz: Option<Zone>) -> Value {
    let Some(t) = chrono::DateTime::from_timestamp_millis(ms) else { return Value::Null };
    Value::String(match tz {
        None => t.format(fmt).to_string(),
        Some(Zone::Fixed(o)) => t.with_timezone(&o).format(fmt).to_string(),
        Some(Zone::Iana(z)) => t.with_timezone(&z).format(fmt).to_string(),
    })
}

fn list(v: Value) -> Vec<Value> {
    match v {
        Value::Array(a) => a,
        Value::Null => vec![],
        other => vec![other],
    }
}

fn filter(name: &str, v: Value, args: &[Value]) -> Value {
    let arg = |i: usize| args.get(i).cloned().unwrap_or(Value::Null);
    let s = || v.as_str().map(str::to_string);
    match name {
        "abs" => num(&v).map_or(Value::Null, |n| number(n.abs())),
        "base64" => {
            use base64::Engine;
            if v.is_null() {
                Value::Null
            } else {
                Value::String(base64::engine::general_purpose::STANDARD.encode(text(&v)))
            }
        }
        "base64_decode" => {
            use base64::Engine;
            s().and_then(|t| base64::engine::general_purpose::STANDARD.decode(t.trim()).ok()).and_then(|b| String::from_utf8(b).ok()).map_or(Value::Null, Value::String)
        }
        "bool" => Value::Bool(match &v {
            Value::String(t) => !matches!(t.trim().to_ascii_lowercase().as_str(), "" | "0" | "false" | "no" | "off"),
            other => truthy(other),
        }),
        "ceil" => num(&v).map_or(Value::Null, |n| number(n.ceil())),
        "compact" => match v {
            Value::Array(a) => Value::Array(a.into_iter().filter(|x| !missing(x)).collect()),
            Value::Object(o) => Value::Object(o.into_iter().filter(|(_, x)| !missing(x)).collect()),
            other => other,
        },
        "contains" => Value::Bool(contains(&v, &arg(0))),
        "default" => {
            if missing(&v) {
                arg(0)
            } else {
                v
            }
        }
        "endswith" => Value::Bool(s().is_some_and(|t| t.ends_with(&text(&arg(0))))),
        "entries" => match v {
            Value::Object(o) => Value::Array(o.into_iter().map(|(k, x)| json!({ "key": k, "value": x })).collect()),
            _ => Value::Array(vec![]),
        },
        "epoch_ms" => to_ms_in(&v, args.first().and_then(|z| zone(&text(z)))).map_or(Value::Null, |ms| json!(ms)),
        "epoch_s" => to_ms_in(&v, args.first().and_then(|z| zone(&text(z)))).map_or(Value::Null, |ms| json!(ms.div_euclid(1000))),
        "explode" => {
            let field = text(&arg(0));
            let mut out = Vec::new();
            for item in list(v) {
                match item.get(&field) {
                    Some(Value::Array(values)) => {
                        for x in values {
                            let mut copy = item.clone();
                            copy[&field] = x.clone();
                            out.push(copy);
                        }
                    }
                    _ => out.push(item),
                }
            }
            Value::Array(out)
        }
        "first" => match v {
            Value::Array(a) => a.into_iter().next().unwrap_or(Value::Null),
            other => other,
        },
        "flatten" => Value::Array(list(v).into_iter().flat_map(list).collect()),
        "float" => num(&v).and_then(Number::from_f64).map_or(Value::Null, Value::Number),
        "floor" => num(&v).map_or(Value::Null, |n| number(n.floor())),
        "from_entries" => Value::Object(list(v).into_iter().filter_map(|e| Some((text(e.get("key")?), e.get("value").cloned().unwrap_or(Value::Null)))).collect()),
        "get" => get(&v, &arg(0)),
        "host" => s().and_then(|t| url::Url::parse(&t).ok()).and_then(|u| u.host_str().map(|h| Value::String(h.to_string()))).unwrap_or(Value::Null),
        "index_by" => {
            let field = arg(0);
            let mut out = Map::new();
            for item in list(v) {
                let key = text(&get(&item, &field));
                if !key.is_empty() {
                    out.insert(key, item);
                }
            }
            Value::Object(out)
        }
        "int" => num(&v).map_or(Value::Null, |n| json!(n.trunc() as i64)),
        "join" => match v {
            Value::Array(a) => Value::String(a.iter().filter(|x| !x.is_null()).map(text).collect::<Vec<_>>().join(&text(&arg(0)))),
            other => other,
        },
        "json" => Value::String(v.to_string()),
        "keys" => match v {
            Value::Object(o) => Value::Array(o.into_iter().map(|(k, _)| Value::String(k)).collect()),
            _ => Value::Array(vec![]),
        },
        "last" => match v {
            Value::Array(mut a) => a.pop().unwrap_or(Value::Null),
            other => other,
        },
        "len" => json!(match &v {
            Value::Array(a) => a.len(),
            Value::Object(o) => o.len(),
            Value::String(t) => t.chars().count(),
            Value::Null => 0,
            _ => 1,
        }),
        "lower" => s().map_or(v, |t| Value::String(t.to_lowercase())),
        "matches" => Value::Bool(s().is_some_and(|t| regex::Regex::new(&text(&arg(0))).is_ok_and(|re| re.is_match(&t)))),
        "max" | "min" => {
            if args.len() == 1 && num(&v).is_none() {
                return Value::Null;
            }
            let values: Vec<f64> = if args.is_empty() { list(v).iter().filter_map(num).collect() } else { [num(&v), num(&arg(0))].into_iter().flatten().collect() };
            let pick = if name == "max" { values.into_iter().reduce(f64::max) } else { values.into_iter().reduce(f64::min) };
            pick.map_or(Value::Null, number)
        }
        "nonempty" => {
            if truthy(&v) || v.is_number() || v.is_boolean() {
                v
            } else {
                Value::Null
            }
        }
        "parse_json" => s().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or(Value::Null),
        "pluck" => {
            let field = arg(0);
            Value::Array(list(v).iter().map(|item| get(item, &field)).collect())
        }
        "replace" => s().map_or(v, |t| Value::String(t.replace(&text(&arg(0)), &text(&arg(1))))),
        "rfc3339" => to_ms(&v).map_or(Value::Null, |ms| format_ms(ms, "%Y-%m-%dT%H:%M:%S%.3fZ")),
        "rfc3339_s" => to_ms(&v).map_or(Value::Null, |ms| format_ms(ms, "%Y-%m-%dT%H:%M:%SZ")),
        "round" => {
            let digits = num(&arg(0)).unwrap_or(0.0) as i32;
            num(&v).map_or(Value::Null, |n| {
                let f = 10f64.powi(digits);
                number((n * f).round() / f)
            })
        }
        "sha256" => {
            use sha2::Digest;
            if v.is_null() {
                Value::Null
            } else {
                Value::String(sha2::Sha256::digest(text(&v).as_bytes()).iter().map(|b| format!("{b:02x}")).collect())
            }
        }
        "slice" => {
            let len = match &v {
                Value::Array(a) => a.len(),
                Value::String(t) => t.chars().count(),
                _ => return Value::Null,
            } as i64;
            let clamp = |i: i64| if i < 0 { (len + i).max(0) } else { i.min(len) } as usize;
            let start = clamp(num(&arg(0)).unwrap_or(0.0) as i64);
            let end = args.get(1).and_then(num).map_or(len as usize, |e| clamp(e as i64)).max(start);
            match v {
                Value::Array(a) => Value::Array(a[start..end].to_vec()),
                Value::String(t) => Value::String(t.chars().skip(start).take(end - start).collect()),
                _ => Value::Null,
            }
        }
        "sort" => {
            let mut items = list(v);
            let field = arg(0);
            items.sort_by(|a, b| {
                let (a, b) = if field.is_null() { (a.clone(), b.clone()) } else { (get(a, &field), get(b, &field)) };
                order(&a, &b).unwrap_or(std::cmp::Ordering::Equal)
            });
            Value::Array(items)
        }
        "split" => s().map_or(Value::Null, |t| Value::Array(t.split(&text(&arg(0))).map(|p| Value::String(p.to_string())).collect())),
        "startswith" => Value::Bool(s().is_some_and(|t| t.starts_with(&text(&arg(0))))),
        "str" => {
            if v.is_null() {
                Value::Null
            } else {
                Value::String(text(&v))
            }
        }
        "strftime" => to_ms(&v).map_or(Value::Null, |ms| format_in(ms, &text(&arg(0)), args.get(1).and_then(|z| zone(&text(z))))),
        "sum" => number(list(v).iter().filter_map(num).sum()),
        "to_objects" => {
            let names: Vec<String> = list(arg(0)).iter().map(|n| match n {
                Value::Object(o) => o.get("name").map(text).unwrap_or_default(),
                other => text(other),
            }).collect();
            Value::Array(list(v).into_iter().map(|row| Value::Object(names.iter().cloned().zip(list(row)).collect())).collect())
        }
        "trim" => s().map_or(v, |t| Value::String(t.trim().to_string())),
        "unique" => {
            let mut out: Vec<Value> = Vec::new();
            for x in list(v) {
                if !out.iter().any(|y| equal(y, &x)) {
                    out.push(x);
                }
            }
            Value::Array(out)
        }
        "upper" => s().map_or(v, |t| Value::String(t.to_uppercase())),
        "urlencode" => {
            if v.is_null() {
                Value::Null
            } else {
                Value::String(url::form_urlencoded::byte_serialize(text(&v).as_bytes()).collect())
            }
        }
        "values" => match v {
            Value::Object(o) => Value::Array(o.into_iter().map(|(_, x)| x).collect()),
            _ => Value::Array(vec![]),
        },
        "where" => {
            let field = arg(0);
            Value::Array(list(v).into_iter().filter(|item| if args.len() == 2 { equal(&get(item, &field), &arg(1)) } else { truthy(&get(item, &field)) }).collect())
        }
        "with" => {
            let (field, value) = (text(&arg(0)), arg(1));
            Value::Array(
                list(v)
                    .into_iter()
                    .map(|mut item| {
                        if let Value::Object(o) = &mut item {
                            o.insert(field.clone(), value.clone());
                        }
                        item
                    })
                    .collect(),
            )
        }
        _ => Value::Null,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn eval(src: &str, scope: Value) -> Value {
        parse(src).unwrap_or_else(|e| panic!("{src}: {e}")).eval(scope.as_object().unwrap())
    }

    #[test]
    fn paths_literals_and_missing_steps() {
        let scope = json!({ "record": { "a": { "b": [10, 20, 30] }, "k": "b", "n": null } });
        assert_eq!(eval("record.a.b[1]", scope.clone()), json!(20));
        assert_eq!(eval("record.a.b[-1]", scope.clone()), json!(30));
        assert_eq!(eval("record.a[record.k][0]", scope.clone()), json!(10));
        assert_eq!(eval("record.x.y.z", scope.clone()), Value::Null);
        assert_eq!(eval("[1, 'two', {k: 3}]", scope.clone()), json!([1, "two", { "k": 3 }]));
        assert_eq!(eval("\"it's\"", scope), json!("it's"));
    }

    #[test]
    fn operators_follow_their_precedence() {
        let scope = json!({ "r": { "name": "", "email": "sam@example.com", "code": "peer.user.add", "n": 7 } });
        assert_eq!(eval("r.name ?? r.email ?? 'nobody'", scope.clone()), json!("sam@example.com"));
        assert_eq!(eval("'by ' + r.missing ?? ''", scope.clone()), json!(""), "null-propagating +, then ??");
        assert_eq!(eval("r.code in ['a', 'peer.user.add'] ? 'error' : 'info'", scope.clone()), json!("error"));
        assert_eq!(eval("r.code not in ['a'] && r.n > 3", scope.clone()), json!(true));
        assert_eq!(eval("r.n * 2 + 1", scope.clone()), json!(15));
        assert_eq!(eval("(r.n + 1) / 4", scope.clone()), json!(2));
        assert_eq!(eval("r.n / 0", scope.clone()), Value::Null);
        assert_eq!(eval("!r.name", scope.clone()), json!(true));
        assert_eq!(eval("1 == 1.0", scope.clone()), json!(true));
        assert_eq!(eval("[1] + [2]", scope.clone()), json!([1, 2]));
        assert_eq!(eval("'a' + 1", scope), json!("a1"));
    }

    #[test]
    fn filters_bind_tightest_and_chain() {
        let scope = json!({ "r": { "code": "Peer.User.Add", "tags": ["b", "a", "b"], "t": "2026-11-06T10:00:00.5Z", "m": { "x": 1 } } });
        assert_eq!(eval("r.code | lower | split('.') | first", scope.clone()), json!("peer"));
        assert_eq!(eval("r.tags | unique | sort | join(',')", scope.clone()), json!("a,b"));
        assert_eq!(eval("r.t | epoch_ms", scope.clone()), json!(1_793_959_200_500i64));
        assert_eq!(eval("r.t | epoch_ms | rfc3339", scope.clone()), json!("2026-11-06T10:00:00.500Z"));
        assert_eq!(eval("r.t | strftime('%Y-%m-%d')", scope.clone()), json!("2026-11-06"));
        assert_eq!(eval("r.m | entries", scope.clone()), json!([{ "key": "x", "value": 1 }]));
        assert_eq!(eval("{} | nonempty", scope.clone()), Value::Null);
        assert_eq!(eval("1 + r.tags | len", scope.clone()), json!(4), "1 + (tags | len)");
        assert_eq!(eval("1793959200 | rfc3339_s", scope.clone()), json!("2026-11-06T10:00:00Z"));
        assert_eq!(eval("'https://api.example.com:8443/x' | host", scope), json!("api.example.com"));
    }

    #[test]
    fn list_filters_reshape_records() {
        let scope = json!({ "response": { "jane": ["Adobe", "Gawker"], "sam": ["Adobe"] }, "members": [{ "id": "u1", "name": "Jane" }, { "id": "u2", "name": "Sam" }] });
        assert_eq!(
            eval("response | entries | explode('value')", scope.clone()),
            json!([{ "key": "jane", "value": "Adobe" }, { "key": "jane", "value": "Gawker" }, { "key": "sam", "value": "Adobe" }])
        );
        assert_eq!(eval("(members | index_by('id')).u2.name", scope.clone()), json!("Sam"));
        assert_eq!(eval("members | pluck('name')", scope.clone()), json!(["Jane", "Sam"]));
        assert_eq!(eval("members | where('name', 'Sam') | len", scope.clone()), json!(1));
        assert_eq!(eval("members | with('kind', 'member') | pluck('kind') | first", scope.clone()), json!("member"));
        assert_eq!(eval("[3, 1, 2] | max", scope.clone()), json!(3));
        assert_eq!(eval("(0 - 5) | max(0)", scope.clone()), json!(0));
        assert_eq!(eval("-5 | max(0)", scope.clone()), json!(-5), "unary minus is looser than filters");
        assert_eq!(eval("null | max(0)", scope.clone()), Value::Null);
        // Snowflake's positional rows, by the result's column metadata.
        assert_eq!(eval("[['a', 1], ['b', 2]] | to_objects([{name: 'NAME'}, {name: 'N'}])", scope), json!([{ "NAME": "a", "N": 1 }, { "NAME": "b", "N": 2 }]));
    }

    #[test]
    fn times_in_every_common_form() {
        for (text, ms) in [
            (json!("2026-11-10T08:57:59.6413435Z"), 1_794_301_079_641i64),
            (json!("2026-11-10 08:57:59"), 1_794_301_079_000),
            (json!("2026-11-10"), 1_794_268_800_000),
            (json!(1_794_301_079), 1_794_301_079_000),
            (json!(1_794_301_079_641i64), 1_794_301_079_641),
            (json!("1794301079641000"), 1_794_301_079_641),
            (json!("Tue, 10 Nov 2026 08:57:59 +0000"), 1_794_301_079_000),
            // ISO 8601's basic offset, without a colon (Mimecast).
            (json!("2026-11-10T10:57:59+0200"), 1_794_301_079_000),
            (json!("2026-11-10T08:57:59.641+0000"), 1_794_301_079_641),
        ] {
            assert_eq!(to_ms(&text), Some(ms), "{text}");
        }
        assert_eq!(to_ms(&json!("yesterday")), None);
        // An account's zone for times without one.
        assert_eq!(to_ms_in(&json!("2026-07-01 12:00:00"), zone("Europe/Bucharest")), Some(1_782_896_400_000));
        assert_eq!(to_ms_in(&json!("2026-07-01 12:00:00"), zone("-05:00")), Some(1_782_925_200_000));
        assert_eq!(to_ms_in(&json!("2026-07-01T12:00:00Z"), zone("-05:00")), Some(1_782_907_200_000), "a zone in the text wins");
    }

    #[test]
    fn errors_name_the_problem() {
        assert!(parse("record.").unwrap_err().contains("name"));
        assert!(parse("x | nosuch").unwrap_err().contains("unknown filter"));
        assert!(parse("x | split").unwrap_err().contains("argument"));
        assert!(parse("'open").unwrap_err().contains("closed"));
        assert!(parse("a b").unwrap_err().contains("after"));
        assert!(parse("").is_err());
        let mut roots = BTreeSet::new();
        parse("record.a ?? config.b | default(item)").unwrap().roots(&mut roots);
        assert_eq!(roots.into_iter().collect::<Vec<_>>(), ["config", "item", "record"]);
    }
}
