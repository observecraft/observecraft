//! The grok `xml` filter: an XML document as JSON, as Datadog's docs describe it. The root element is the
//! one key; attributes and child elements are keys of their element, a repeated child element is an array,
//! an element with only text is that text, and text next to attributes or children is `value`.

use serde_json::{Map, Value};

/// The document as `{root: element}`, or `None` when it isn't well-formed XML.
pub fn to_json(xml: &str) -> Option<Value> {
    let mut p = Parser { s: xml.as_bytes(), pos: 0 };
    p.skip_misc();
    let (name, value) = p.element()?;
    p.skip_misc();
    (p.pos == p.s.len()).then(|| Value::Object(Map::from_iter([(name, value)])))
}

struct Parser<'a> {
    s: &'a [u8],
    pos: usize,
}

impl Parser<'_> {
    fn rest(&self) -> &[u8] {
        &self.s[self.pos..]
    }

    fn skip_spaces(&mut self) {
        while self.rest().first().is_some_and(u8::is_ascii_whitespace) {
            self.pos += 1;
        }
    }

    /// Skips `end`-terminated text after `start`; false when it never ends.
    fn skip_between(&mut self, start: &[u8], end: &[u8]) -> bool {
        if !self.rest().starts_with(start) {
            return false;
        }
        match find(&self.rest()[start.len()..], end) {
            Some(i) => {
                self.pos += start.len() + i + end.len();
                true
            }
            None => false,
        }
    }

    /// Whitespace, the prolog, comments and the doctype around elements.
    fn skip_misc(&mut self) {
        loop {
            self.skip_spaces();
            if !(self.skip_between(b"<?", b"?>") || self.skip_between(b"<!--", b"-->") || (!self.rest().starts_with(b"<![CDATA[") && self.skip_between(b"<!", b">"))) {
                return;
            }
        }
    }

    fn name(&mut self) -> Option<String> {
        let start = self.pos;
        while self.rest().first().is_some_and(|c| !c.is_ascii_whitespace() && !b"/>=\"'".contains(c)) {
            self.pos += 1;
        }
        (self.pos > start).then(|| String::from_utf8_lossy(&self.s[start..self.pos]).into_owned())
    }

    /// One element at `<`: its name and value.
    fn element(&mut self) -> Option<(String, Value)> {
        if self.rest().first() != Some(&b'<') {
            return None;
        }
        self.pos += 1;
        let name = self.name()?;
        let mut fields = Map::new();
        loop {
            self.skip_spaces();
            match self.rest().first()? {
                b'/' if self.rest().starts_with(b"/>") => {
                    self.pos += 2;
                    return Some((name, if fields.is_empty() { Value::from("") } else { Value::Object(fields) }));
                }
                b'>' => {
                    self.pos += 1;
                    break;
                }
                _ => {
                    let key = self.name()?;
                    self.skip_spaces();
                    if self.rest().first() != Some(&b'=') {
                        return None;
                    }
                    self.pos += 1;
                    self.skip_spaces();
                    let quote = *self.rest().first().filter(|q| **q == b'"' || **q == b'\'')?;
                    let end = self.rest()[1..].iter().position(|c| *c == quote)?;
                    let raw = String::from_utf8_lossy(&self.rest()[1..1 + end]).into_owned();
                    self.pos += end + 2;
                    fields.insert(key, Value::from(unescape(&raw)));
                }
            }
        }
        let has_attributes = !fields.is_empty();
        let mut text = String::new();
        let mut children = false;
        loop {
            if self.rest().starts_with(b"</") {
                self.pos += 2;
                if self.name()? != name {
                    return None;
                }
                self.skip_spaces();
                if self.rest().first() != Some(&b'>') {
                    return None;
                }
                self.pos += 1;
                break;
            }
            if self.rest().starts_with(b"<![CDATA[") {
                let start = self.pos + 9;
                let end = find(&self.s[start..], b"]]>")?;
                text.push_str(&String::from_utf8_lossy(&self.s[start..start + end]));
                self.pos = start + end + 3;
            } else if self.rest().starts_with(b"<!--") || self.rest().starts_with(b"<?") {
                self.skip_misc();
            } else if self.rest().first() == Some(&b'<') {
                let (child, value) = self.element()?;
                children = true;
                match fields.get_mut(&child) {
                    Some(Value::Array(items)) => items.push(value),
                    Some(existing) => *existing = Value::Array(vec![existing.take(), value]),
                    None => {
                        fields.insert(child, value);
                    }
                }
            } else {
                let end = self.rest().iter().position(|c| *c == b'<')?;
                text.push_str(&unescape(&String::from_utf8_lossy(&self.rest()[..end])));
                self.pos += end;
            }
        }
        let text = text.trim();
        if !has_attributes && !children {
            return Some((name, Value::from(text)));
        }
        if !text.is_empty() {
            fields.insert("value".into(), Value::from(text));
        }
        Some((name, Value::Object(fields)))
    }
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

/// The five predefined entities and numeric character references.
fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        let tail = &rest[i..];
        let Some(end) = tail.find(';') else {
            out.push_str(tail);
            return out;
        };
        let entity = &tail[1..end];
        let decoded = match entity {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            _ => entity
                .strip_prefix("#x")
                .map(|h| u32::from_str_radix(h, 16))
                .or_else(|| entity.strip_prefix('#').map(str::parse::<u32>))
                .and_then(Result::ok)
                .and_then(char::from_u32),
        };
        match decoded {
            Some(c) => out.push(c),
            None => out.push_str(&tail[..=end]),
        }
        rest = &tail[end + 1..];
    }
    out.push_str(rest);
    out
}
