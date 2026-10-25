/// One AND-ed clause of a search.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Term {
    /// `key:value`, `key:(a OR b)`, `-key:value`. Values may contain `*` wildcards.
    Tag { key: String, values: Vec<String>, negate: bool },
    /// Free text matched against the message.
    Text { text: String, negate: bool },
}

/// Splits on whitespace, keeping quoted strings and parenthesised groups intact.
pub fn tokenize(q: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut depth = 0usize;
    let mut quoted = false;
    for ch in q.chars() {
        match ch {
            '"' => quoted = !quoted,
            '(' if !quoted => depth += 1,
            ')' if !quoted => depth = depth.saturating_sub(1),
            c if c.is_whitespace() && !quoted && depth == 0 => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
                continue;
            }
            _ => {}
        }
        cur.push(ch);
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

fn unquote(s: &str) -> String {
    if s.len() >= 2 && s.starts_with('"') && s.ends_with('"') {
        s[1..s.len() - 1].to_string()
    } else {
        s.to_string()
    }
}

fn is_key(s: &str) -> bool {
    let body = s.strip_prefix('@').unwrap_or(s);
    !body.is_empty() && body.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))
}

fn parse_term(token: &str) -> Term {
    let (negate, body) = match token.strip_prefix('-') {
        Some(rest) if !rest.is_empty() => (true, rest),
        _ => (false, token),
    };
    if let Some(idx) = body.find(':') {
        let (key, raw) = (&body[..idx], &body[idx + 1..]);
        if is_key(key) && !raw.is_empty() {
            let values = if raw.starts_with('(') && raw.ends_with(')') {
                raw[1..raw.len() - 1]
                    .split(" OR ")
                    .flat_map(|v| v.split(" or "))
                    .map(|v| unquote(v.trim()))
                    .filter(|v| !v.is_empty())
                    .collect()
            } else {
                vec![unquote(raw)]
            };
            return Term::Tag { key: key.to_string(), values, negate };
        }
    }
    Term::Text { text: unquote(body), negate }
}

pub fn parse_query(q: &str) -> Vec<Term> {
    tokenize(q).iter().filter(|t| t.as_str() != "AND").map(|t| parse_term(t)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_tags_unions_negation_and_text() {
        let t = parse_query(r#"service:(a OR b) -status:info "bad response" @http.status_code:5* AND timeout"#);
        assert_eq!(
            t,
            vec![
                Term::Tag { key: "service".into(), values: vec!["a".into(), "b".into()], negate: false },
                Term::Tag { key: "status".into(), values: vec!["info".into()], negate: true },
                Term::Text { text: "bad response".into(), negate: false },
                Term::Tag { key: "@http.status_code".into(), values: vec!["5*".into()], negate: false },
                Term::Text { text: "timeout".into(), negate: false },
            ]
        );
    }

    #[test]
    fn urls_are_text_not_tags() {
        // Keys may only contain [A-Za-z0-9_.-] (plus a leading @), so "/api/v1:thing" stays free text.
        assert!(matches!(parse_query("GET /api/v1:thing").as_slice(), [Term::Text { .. }, Term::Text { .. }]));
    }
}
