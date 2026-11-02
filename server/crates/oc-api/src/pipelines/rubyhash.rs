//! The grok `rubyhash` filter: a Ruby hash literal (`{name => "John", "job" => {"title" => "CTO"}}`, or Logstash's
//! `{:message=>"…", :count=>3}`) as JSON. Keys lose their quotes and symbol colon; strings, numbers, `true`,
//! `false`, `nil`, arrays and nested hashes keep their types; other bare values stay text.

use serde_json::{Map, Value};

/// The hash as an object, or `None` when the text isn't one.
pub fn to_json(text: &str) -> Option<Value> {
    let mut p = Parser { s: text.trim().as_bytes(), pos: 0 };
    let v = p.hash()?;
    p.ws();
    (p.pos == p.s.len()).then_some(v)
}

struct Parser<'a> {
    s: &'a [u8],
    pos: usize,
}

impl Parser<'_> {
    fn peek(&self) -> Option<u8> {
        self.s.get(self.pos).copied()
    }

    fn ws(&mut self) {
        while self.peek().is_some_and(|c| c.is_ascii_whitespace()) {
            self.pos += 1;
        }
    }

    fn eat(&mut self, c: u8) -> bool {
        self.ws();
        let ok = self.peek() == Some(c);
        self.pos += usize::from(ok);
        ok
    }

    fn hash(&mut self) -> Option<Value> {
        if !self.eat(b'{') {
            return None;
        }
        let mut out = Map::new();
        if self.eat(b'}') {
            return Some(Value::Object(out));
        }
        loop {
            let key = self.key()?;
            self.ws();
            if !self.s[self.pos..].starts_with(b"=>") {
                return None;
            }
            self.pos += 2;
            let value = self.value()?;
            out.insert(key, value);
            if self.eat(b'}') {
                return Some(Value::Object(out));
            }
            if !self.eat(b',') {
                return None;
            }
        }
    }

    fn array(&mut self) -> Option<Value> {
        self.eat(b'[');
        let mut out = Vec::new();
        if self.eat(b']') {
            return Some(Value::Array(out));
        }
        loop {
            out.push(self.value()?);
            if self.eat(b']') {
                return Some(Value::Array(out));
            }
            if !self.eat(b',') {
                return None;
            }
        }
    }

    /// `"name"`, `'name'`, `:name` or `name`.
    fn key(&mut self) -> Option<String> {
        self.ws();
        match self.peek()? {
            b'"' | b'\'' => self.string(),
            b':' => {
                self.pos += 1;
                if matches!(self.peek(), Some(b'"' | b'\'')) {
                    return self.string();
                }
                self.bare(b"=").map(|k| k.trim_end().to_string())
            }
            _ => self.bare(b"=").map(|k| k.trim_end().to_string()),
        }
    }

    fn value(&mut self) -> Option<Value> {
        self.ws();
        match self.peek()? {
            b'{' => self.hash(),
            b'[' => self.array(),
            b'"' | b'\'' => self.string().map(Value::from),
            _ => {
                let word = self.bare(b",}]")?.trim_end();
                Some(match word {
                    "nil" => Value::Null,
                    "true" => Value::Bool(true),
                    "false" => Value::Bool(false),
                    _ => word
                        .parse::<i64>()
                        .map(Value::from)
                        .ok()
                        .or_else(|| word.parse::<f64>().ok().and_then(|f| serde_json::Number::from_f64(f).map(Value::Number)))
                        .unwrap_or_else(|| Value::from(word.trim_start_matches(':'))),
                })
            }
        }
    }

    /// Text up to one of `stops` (not included); empty text isn't a value.
    fn bare(&mut self, stops: &[u8]) -> Option<&str> {
        let start = self.pos;
        while self.peek().is_some_and(|c| !stops.contains(&c)) {
            self.pos += 1;
        }
        let word = std::str::from_utf8(&self.s[start..self.pos]).ok()?;
        (!word.trim().is_empty()).then_some(word)
    }

    /// A quoted string; double-quoted ones take backslash escapes.
    fn string(&mut self) -> Option<String> {
        let quote = self.peek()?;
        self.pos += 1;
        let mut out = Vec::new();
        loop {
            let c = self.peek()?;
            self.pos += 1;
            match c {
                c if c == quote => return String::from_utf8(out).ok(),
                b'\\' => {
                    let e = self.peek()?;
                    self.pos += 1;
                    match (quote, e) {
                        (b'"', b'n') => out.push(b'\n'),
                        (b'"', b't') => out.push(b'\t'),
                        (b'"', b'r') => out.push(b'\r'),
                        (_, e) if e == quote || e == b'\\' => out.push(e),
                        (_, e) => out.extend([b'\\', e]),
                    }
                }
                c => out.push(c),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn hashes_with_quoted_and_bare_keys_nest() {
        let v = to_json(r#"{name => "John", "job" => {"company" => "Big Company", "title" => "CTO"}}"#);
        assert_eq!(v, Some(json!({"name": "John", "job": {"company": "Big Company", "title": "CTO"}})));
    }

    #[test]
    fn logstash_symbols_keep_their_value_types() {
        let v = to_json(r#"{:message=>"pipeline started", :count=>3, :ratio=>0.5, :ok=>true, :error=>nil, :ids=>[1, "two"], :klass=>LogStash::Inputs::Beats}"#);
        assert_eq!(v, Some(json!({"message": "pipeline started", "count": 3, "ratio": 0.5, "ok": true, "error": null, "ids": [1, "two"], "klass": "LogStash::Inputs::Beats"})));
    }

    #[test]
    fn escapes_and_single_quotes() {
        assert_eq!(to_json(r#"{"b" => "say \"hi\""}"#), Some(json!({"b": "say \"hi\""})));
        assert_eq!(to_json(r#"{'a' => 'x'}"#), Some(json!({"a": "x"})));
    }

    #[test]
    fn not_a_hash() {
        assert_eq!(to_json("plain text"), None);
        assert_eq!(to_json(r#"{"a" => "unterminated}"#), None);
        assert_eq!(to_json("{}"), Some(json!({})));
    }
}
