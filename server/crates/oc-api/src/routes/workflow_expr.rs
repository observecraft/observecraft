//! Workflow Automation: the small expression language of if/else conditions and "for each" lists.
//!
//! An expression is a value or a test over the run's data, with or without `{{ }}` around it:
//!
//! - **paths** name data: `Trigger.monitor.name`, `Steps.check.output.value`, `Steps.list.output.items[0].id`,
//!   `inputs.env`, `Loop.item`; a path that leads nowhere is `null`;
//! - **literals**: numbers, `"strings"` or `'strings'`, `true`, `false`, `null`;
//! - **operators**: `==` `!=` `<` `<=` `>` `>=`, `contains` (text or list), `&&` / `and`, `||` / `or`,
//!   `!` / `not`, and parentheses.
//!
//! Nothing is executed: an expression is parsed into a tree and evaluated against a JSON document.
//! Text that reads as a number compares as one (`Trigger.value > 5` holds for `"7.5"`). Values are
//! truthy unless `null`, `false`, `0`, `""` or empty.

use serde_json::Value;

pub const MAX_EXPRESSION_CHARS: usize = 2000;
const MAX_TOKENS: usize = 400;
const MAX_DEPTH: usize = 32;

#[derive(Debug, Clone, PartialEq)]
pub enum Segment {
    Key(String),
    Index(usize),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BinOp {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    Contains,
    And,
    Or,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    Literal(Value),
    Path(Vec<Segment>),
    Not(Box<Expr>),
    Neg(Box<Expr>),
    Binary(BinOp, Box<Expr>, Box<Expr>),
}

#[derive(Debug, Clone, PartialEq)]
enum Token {
    Num(f64),
    Str(String),
    Ident(String),
    Dot,
    LBracket,
    RBracket,
    LParen,
    RParen,
    Op(&'static str),
}

fn tokenize(src: &str) -> Result<Vec<Token>, String> {
    let chars: Vec<char> = src.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            i += 1;
            continue;
        }
        if out.len() >= MAX_TOKENS {
            return Err(format!("An expression has at most {MAX_TOKENS} parts"));
        }
        let two: String = chars[i..(i + 2).min(chars.len())].iter().collect();
        if let Some(op) = ["==", "!=", "<=", ">=", "&&", "||"].into_iter().find(|o| *o == two) {
            out.push(Token::Op(op));
            i += 2;
            continue;
        }
        match c {
            '<' => out.push(Token::Op("<")),
            '>' => out.push(Token::Op(">")),
            '!' => out.push(Token::Op("!")),
            '-' => out.push(Token::Op("-")),
            '.' => out.push(Token::Dot),
            '[' => out.push(Token::LBracket),
            ']' => out.push(Token::RBracket),
            '(' => out.push(Token::LParen),
            ')' => out.push(Token::RParen),
            '"' | '\'' => {
                let quote = c;
                let mut s = String::new();
                i += 1;
                loop {
                    let Some(&ch) = chars.get(i) else { return Err("A text value is missing its closing quote".into()) };
                    if ch == quote {
                        break;
                    }
                    if ch == '\\' {
                        i += 1;
                        match chars.get(i) {
                            Some('n') => s.push('\n'),
                            Some('t') => s.push('\t'),
                            Some(&other) => s.push(other),
                            None => return Err("A text value is missing its closing quote".into()),
                        }
                    } else {
                        s.push(ch);
                    }
                    i += 1;
                }
                out.push(Token::Str(s));
            }
            d if d.is_ascii_digit() => {
                let start = i;
                while i < chars.len() && (chars[i].is_ascii_digit() || (chars[i] == '.' && chars.get(i + 1).is_some_and(|n| n.is_ascii_digit()))) {
                    i += 1;
                }
                let text: String = chars[start..i].iter().collect();
                out.push(Token::Num(text.parse().map_err(|_| format!("{text} isn't a number"))?));
                continue;
            }
            a if a.is_ascii_alphabetic() || a == '_' => {
                let start = i;
                while i < chars.len() && (chars[i].is_ascii_alphanumeric() || chars[i] == '_') {
                    i += 1;
                }
                out.push(Token::Ident(chars[start..i].iter().collect()));
                continue;
            }
            other => return Err(format!("Unexpected character “{other}” in the expression")),
        }
        i += 1;
    }
    Ok(out)
}

struct Parser {
    tokens: Vec<Token>,
    pos: usize,
    depth: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.pos)
    }

    fn next(&mut self) -> Option<Token> {
        let t = self.tokens.get(self.pos).cloned();
        self.pos += 1;
        t
    }

    fn is_word(&self, w: &str) -> bool {
        matches!(self.peek(), Some(Token::Ident(i)) if i == w)
    }

    fn is_op(&self, op: &str) -> bool {
        matches!(self.peek(), Some(Token::Op(o)) if *o == op)
    }

    fn enter(&mut self) -> Result<(), String> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            return Err("The expression is nested too deeply".into());
        }
        Ok(())
    }

    fn or(&mut self) -> Result<Expr, String> {
        let mut left = self.and()?;
        while self.is_op("||") || self.is_word("or") {
            self.pos += 1;
            let right = self.and()?;
            left = Expr::Binary(BinOp::Or, Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn and(&mut self) -> Result<Expr, String> {
        let mut left = self.comparison()?;
        while self.is_op("&&") || self.is_word("and") {
            self.pos += 1;
            let right = self.comparison()?;
            left = Expr::Binary(BinOp::And, Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn comparison(&mut self) -> Result<Expr, String> {
        let left = self.unary()?;
        let op = match self.peek() {
            Some(Token::Op("==")) => BinOp::Eq,
            Some(Token::Op("!=")) => BinOp::Ne,
            Some(Token::Op("<")) => BinOp::Lt,
            Some(Token::Op("<=")) => BinOp::Le,
            Some(Token::Op(">")) => BinOp::Gt,
            Some(Token::Op(">=")) => BinOp::Ge,
            Some(Token::Ident(w)) if w == "contains" => BinOp::Contains,
            _ => return Ok(left),
        };
        self.pos += 1;
        let right = self.unary()?;
        if matches!(self.peek(), Some(Token::Op("==" | "!=" | "<" | "<=" | ">" | ">="))) || self.is_word("contains") {
            return Err("Comparisons can't be chained; join them with && or ||".into());
        }
        Ok(Expr::Binary(op, Box::new(left), Box::new(right)))
    }

    fn unary(&mut self) -> Result<Expr, String> {
        self.enter()?;
        let e = if self.is_op("!") || self.is_word("not") {
            self.pos += 1;
            Expr::Not(Box::new(self.unary()?))
        } else if self.is_op("-") {
            self.pos += 1;
            Expr::Neg(Box::new(self.unary()?))
        } else {
            self.primary()?
        };
        self.depth -= 1;
        Ok(e)
    }

    fn primary(&mut self) -> Result<Expr, String> {
        match self.next() {
            Some(Token::Num(n)) => Ok(Expr::Literal(number(n))),
            Some(Token::Str(s)) => Ok(Expr::Literal(Value::String(s))),
            Some(Token::LParen) => {
                let e = self.or()?;
                match self.next() {
                    Some(Token::RParen) => Ok(e),
                    _ => Err("A parenthesis isn't closed".into()),
                }
            }
            Some(Token::Ident(w)) => match w.as_str() {
                "true" => Ok(Expr::Literal(Value::Bool(true))),
                "false" => Ok(Expr::Literal(Value::Bool(false))),
                "null" => Ok(Expr::Literal(Value::Null)),
                "and" | "or" | "not" | "contains" => Err(format!("“{w}” needs a value before and after it")),
                _ => self.path(w),
            },
            Some(Token::Op(op)) => Err(format!("“{op}” needs a value before it")),
            Some(Token::RParen) => Err("Unexpected “)”".into()),
            Some(_) => Err("Unexpected “.” or “[”: a path starts with a name (Trigger, Steps, inputs, Loop)".into()),
            None => Err("The expression ends too early".into()),
        }
    }

    fn path(&mut self, root: String) -> Result<Expr, String> {
        let mut segs = vec![Segment::Key(root)];
        loop {
            match self.peek() {
                Some(Token::Dot) => {
                    self.pos += 1;
                    match self.next() {
                        Some(Token::Ident(k)) => segs.push(Segment::Key(k)),
                        Some(Token::Num(n)) if n.fract() == 0.0 && n >= 0.0 => segs.push(Segment::Index(n as usize)),
                        _ => return Err("A “.” must be followed by a name".into()),
                    }
                }
                Some(Token::LBracket) => {
                    self.pos += 1;
                    let seg = match self.next() {
                        Some(Token::Num(n)) if n.fract() == 0.0 && n >= 0.0 => Segment::Index(n as usize),
                        Some(Token::Str(s)) => Segment::Key(s),
                        _ => return Err("Brackets hold a position ([0]) or a quoted name ([\"key\"])".into()),
                    };
                    if self.next() != Some(Token::RBracket) {
                        return Err("A “[” isn't closed".into());
                    }
                    segs.push(seg);
                }
                _ => return Ok(Expr::Path(segs)),
            }
        }
    }
}

fn number(n: f64) -> Value {
    if n.fract() == 0.0 && n.abs() < 1e15 {
        Value::from(n as i64)
    } else {
        serde_json::Number::from_f64(n).map(Value::Number).unwrap_or(Value::Null)
    }
}

/// `{{ x }}` → `x`.
fn unwrap(src: &str) -> &str {
    let t = src.trim();
    t.strip_prefix("{{").and_then(|r| r.strip_suffix("}}")).map(str::trim).unwrap_or(t)
}

/// Parses an expression.
pub fn parse(src: &str) -> Result<Expr, String> {
    if src.chars().count() > MAX_EXPRESSION_CHARS {
        return Err(format!("An expression is longer than {MAX_EXPRESSION_CHARS} characters"));
    }
    let inner = unwrap(src);
    if inner.is_empty() {
        return Err("The expression is empty".into());
    }
    let mut p = Parser { tokens: tokenize(inner)?, pos: 0, depth: 0 };
    let e = p.or()?;
    if p.pos < p.tokens.len() {
        return Err("Unexpected text after the end of the expression; join conditions with && or ||".into());
    }
    Ok(e)
}

fn paths<'a>(e: &'a Expr, out: &mut Vec<&'a [Segment]>) {
    match e {
        Expr::Literal(_) => {}
        Expr::Path(p) => out.push(p),
        Expr::Not(x) | Expr::Neg(x) => paths(x, out),
        Expr::Binary(_, a, b) => {
            paths(a, out);
            paths(b, out);
        }
    }
}

/// Parses an expression and checks that its paths start at one of `roots`.
pub fn check(src: &str, roots: &[&str]) -> Result<Expr, String> {
    let e = parse(src)?;
    let mut found = Vec::new();
    paths(&e, &mut found);
    for p in found {
        if let Some(Segment::Key(root)) = p.first() {
            if !roots.contains(&root.as_str()) {
                return Err(format!("Unknown name {root}: start a path with {}", roots.join(", ")));
            }
        }
    }
    Ok(e)
}

/// The value at `path` in `vars`, null when absent.
pub fn lookup<'a>(vars: &'a Value, path: &[Segment]) -> Option<&'a Value> {
    let mut cur = vars;
    for seg in path {
        cur = match (seg, cur) {
            (Segment::Key(k), Value::Object(m)) => m.get(k)?,
            (Segment::Key(k), Value::Array(a)) => a.get(k.parse::<usize>().ok()?)?,
            (Segment::Index(i), Value::Array(a)) => a.get(*i)?,
            (Segment::Index(i), Value::Object(m)) => m.get(&i.to_string())?,
            _ => return None,
        };
    }
    Some(cur)
}

/// Splits a dotted template path (`Steps.check.output.items.0`) into segments.
pub fn segments(path: &str) -> Vec<Segment> {
    path.split('.').map(|s| s.parse::<usize>().map(Segment::Index).unwrap_or_else(|_| Segment::Key(s.to_string()))).collect()
}

pub fn truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().is_some_and(|f| f != 0.0),
        Value::String(s) => !s.is_empty(),
        Value::Array(a) => !a.is_empty(),
        Value::Object(m) => !m.is_empty(),
    }
}

fn as_number(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.trim().parse::<f64>().ok().filter(|f| f.is_finite()),
        _ => None,
    }
}

fn equal(a: &Value, b: &Value) -> bool {
    if a == b {
        return true;
    }
    match (a, b) {
        (Value::Number(_), Value::String(_)) | (Value::String(_), Value::Number(_)) | (Value::Number(_), Value::Number(_)) => matches!((as_number(a), as_number(b)), (Some(x), Some(y)) if x == y),
        _ => false,
    }
}

fn order(a: &Value, b: &Value) -> Option<std::cmp::Ordering> {
    if let (Some(x), Some(y)) = (as_number(a), as_number(b)) {
        return x.partial_cmp(&y);
    }
    match (a, b) {
        (Value::String(x), Value::String(y)) => Some(x.cmp(y)),
        _ => None,
    }
}

fn contains(hay: &Value, needle: &Value) -> bool {
    match hay {
        Value::String(s) => match needle {
            Value::String(n) => s.contains(n.as_str()),
            Value::Number(_) | Value::Bool(_) => s.contains(&needle.to_string()),
            _ => false,
        },
        Value::Array(a) => a.iter().any(|x| equal(x, needle)),
        Value::Object(m) => needle.as_str().is_some_and(|k| m.contains_key(k)),
        _ => false,
    }
}

/// Evaluates a parsed expression against the run's data.
pub fn eval(e: &Expr, vars: &Value) -> Value {
    match e {
        Expr::Literal(v) => v.clone(),
        Expr::Path(p) => lookup(vars, p).cloned().unwrap_or(Value::Null),
        Expr::Not(x) => Value::Bool(!truthy(&eval(x, vars))),
        Expr::Neg(x) => as_number(&eval(x, vars)).map(|n| number(-n)).unwrap_or(Value::Null),
        Expr::Binary(op, a, b) => {
            let left = eval(a, vars);
            match op {
                BinOp::And => return Value::Bool(truthy(&left) && truthy(&eval(b, vars))),
                BinOp::Or => return Value::Bool(truthy(&left) || truthy(&eval(b, vars))),
                _ => {}
            }
            let right = eval(b, vars);
            Value::Bool(match op {
                BinOp::Eq => equal(&left, &right),
                BinOp::Ne => !equal(&left, &right),
                BinOp::Lt => order(&left, &right) == Some(std::cmp::Ordering::Less),
                BinOp::Le => matches!(order(&left, &right), Some(std::cmp::Ordering::Less | std::cmp::Ordering::Equal)),
                BinOp::Gt => order(&left, &right) == Some(std::cmp::Ordering::Greater),
                BinOp::Ge => matches!(order(&left, &right), Some(std::cmp::Ordering::Greater | std::cmp::Ordering::Equal)),
                BinOp::Contains => contains(&left, &right),
                BinOp::And | BinOp::Or => unreachable!("handled above"),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn vars() -> Value {
        json!({
            "Trigger": {"monitor": {"name": "High error rate on order-service", "tags": ["team:checkout", "env:prod"]}, "value": "7.5", "transition": {"to": "alert"}},
            "Steps": {"check": {"status": "succeeded", "output": {"value": 3, "items": [{"id": "a1"}, {"id": "b2"}]}}},
            "inputs": {"env": "production", "dry_run": false, "replicas": 0},
        })
    }

    fn run(src: &str) -> Value {
        eval(&parse(src).unwrap_or_else(|e| panic!("{src}: {e}")), &vars())
    }

    #[test]
    fn paths_and_literals() {
        assert_eq!(run("Trigger.monitor.name"), json!("High error rate on order-service"));
        assert_eq!(run("{{ Steps.check.output.items[1].id }}"), json!("b2"));
        assert_eq!(run("Steps.check.output.items.0.id"), json!("a1"));
        assert_eq!(run("Steps.missing.output"), Value::Null, "a path that leads nowhere is null");
        assert_eq!(run("'it\\'s'"), json!("it's"));
        assert_eq!(run("-2.5"), json!(-2.5));
        assert_eq!(run("null"), Value::Null);
    }

    #[test]
    fn comparisons_coerce_numeric_text() {
        assert_eq!(run("Trigger.value > 5"), json!(true));
        assert_eq!(run("Trigger.value == 7.5"), json!(true));
        assert_eq!(run("Steps.check.output.value <= 3"), json!(true));
        assert_eq!(run("Steps.check.output.value != 3"), json!(false));
        assert_eq!(run("inputs.env == \"production\""), json!(true));
        assert_eq!(run("inputs.env < 'staging'"), json!(true));
        assert_eq!(run("inputs.env > 3"), json!(false), "text and numbers don't order");
        assert_eq!(run("Steps.missing == null"), json!(true));
    }

    #[test]
    fn logic_and_contains() {
        assert_eq!(run("Trigger.transition.to == 'alert' && Trigger.value > 5"), json!(true));
        assert_eq!(run("Trigger.transition.to == 'ok' or inputs.env == 'production'"), json!(true));
        assert_eq!(run("not inputs.dry_run"), json!(true));
        assert_eq!(run("!inputs.replicas"), json!(true), "0 is falsy");
        assert_eq!(run("Trigger.monitor.tags contains 'team:checkout'"), json!(true));
        assert_eq!(run("Trigger.monitor.name contains 'order-service'"), json!(true));
        assert_eq!(run("(inputs.env == 'staging' || Steps.check.status == 'succeeded') && !(Trigger.value < 1)"), json!(true));
    }

    #[test]
    fn mistakes_are_explained() {
        assert!(parse("").unwrap_err().contains("empty"));
        assert!(parse("a ==").unwrap_err().contains("ends too early"));
        assert!(parse("(a == 1").unwrap_err().contains("parenthesis"));
        assert!(parse("a == 1 b").unwrap_err().contains("&&"));
        assert!(parse("1 < a < 3").unwrap_err().contains("chained"));
        assert!(parse("'open").unwrap_err().contains("quote"));
        assert!(parse("a; drop table").unwrap_err().contains("Unexpected character"));
        assert!(parse(&"(".repeat(40)).unwrap_err().contains("nested"));
        assert!(parse(&"a".repeat(MAX_EXPRESSION_CHARS + 1)).unwrap_err().contains("longer"));
    }

    #[test]
    fn roots_are_checked() {
        let roots = ["Trigger", "Steps", "inputs"];
        assert!(check("Trigger.value > 1 && Steps.a.status == 'succeeded'", &roots).is_ok());
        assert!(check("Secrets.token == 'x'", &roots).unwrap_err().contains("Secrets"));
    }

    #[test]
    fn truthiness() {
        for v in [json!(null), json!(false), json!(0), json!(""), json!([]), json!({})] {
            assert!(!truthy(&v), "{v}");
        }
        for v in [json!(true), json!(1), json!("x"), json!([0]), json!({"a": 1})] {
            assert!(truthy(&v), "{v}");
        }
    }
}
