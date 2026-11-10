/// One AND-ed clause of a search.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Term {
    /// `key:value`, `key:(a OR b)`, `-key:value`. Values may contain `*` wildcards.
    Tag { key: String, values: Vec<String>, negate: bool },
    /// Free text matched against the message.
    Text { text: String, negate: bool },
    /// `(a OR b)`, `-(@x:1 OR @y:2 z)`: a group of searches, any of which matches (each one's terms ANDed).
    Any { alternatives: Vec<Vec<Term>>, negate: bool },
}

/// Splits on whitespace, keeping quoted strings (with their escaped `\"` and `\\`), parenthesised groups and escaped
/// spaces (`\ `) intact.
pub fn tokenize(q: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut depth = 0usize;
    let mut quoted = false;
    let mut chars = q.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '\\' if !quoted && chars.peek().is_some_and(|c| c.is_whitespace()) => {
                cur.push(ch);
                cur.extend(chars.next());
                continue;
            }
            '\\' if quoted => {
                cur.push(ch);
                cur.extend(chars.next());
                continue;
            }
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

/// A value as written: without its quotes (`\"` and `\\` inside them decoded), or with its escaped spaces (`\ `) as
/// spaces.
fn unquote(s: &str) -> String {
    if s.len() >= 2 && s.starts_with('"') && s.ends_with('"') {
        let mut out = String::new();
        let mut chars = s[1..s.len() - 1].chars().peekable();
        while let Some(ch) = chars.next() {
            match chars.peek() {
                Some(&next @ ('"' | '\\')) if ch == '\\' => {
                    out.push(next);
                    chars.next();
                }
                _ => out.push(ch),
            }
        }
        out
    } else {
        s.replace("\\ ", " ")
    }
}

fn is_key(s: &str) -> bool {
    let body = s.strip_prefix('@').unwrap_or(s);
    !body.is_empty() && body.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))
}

/// Splits a group's inside on its top-level ` OR `s (not those of nested groups or quotes).
fn split_or(inner: &str) -> Vec<String> {
    let mut out = vec![String::new()];
    for tok in tokenize(inner) {
        if tok == "OR" || tok == "or" {
            out.push(String::new());
        } else {
            let cur = out.last_mut().expect("one part");
            if !cur.is_empty() {
                cur.push(' ');
            }
            cur.push_str(&tok);
        }
    }
    out
}

fn parse_term(token: &str) -> Term {
    let (negate, body) = match token.strip_prefix('-') {
        Some(rest) if !rest.is_empty() => (true, rest),
        _ => (false, token),
    };
    if body.len() > 2 && body.starts_with('(') && body.ends_with(')') {
        let parts = split_or(&body[1..body.len() - 1]);
        let alternatives: Vec<Vec<Term>> = parts.iter().map(|p| parse_query(p)).filter(|t| !t.is_empty()).collect();
        if !alternatives.is_empty() {
            return Term::Any { alternatives, negate };
        }
    }
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
    fn groups_across_attributes() {
        let t = parse_query("source:scfw (@warning:true OR @warned:true) -(@a:1 OR b c)");
        let tag = |k: &str, v: &str| Term::Tag { key: k.into(), values: vec![v.into()], negate: false };
        assert_eq!(
            t,
            vec![
                tag("source", "scfw"),
                Term::Any { alternatives: vec![vec![tag("@warning", "true")], vec![tag("@warned", "true")]], negate: false },
                Term::Any { alternatives: vec![vec![tag("@a", "1")], vec![Term::Text { text: "b".into(), negate: false }, Term::Text { text: "c".into(), negate: false }]], negate: true },
            ]
        );
        assert_eq!(parse_query("(timeout)"), vec![Term::Any { alternatives: vec![vec![Term::Text { text: "timeout".into(), negate: false }]], negate: false }]);
    }

    #[test]
    fn escaped_spaces_stay_in_values() {
        // Datadog's syntax (its integration pipelines use it): `\ ` is a space inside a value.
        let t = parse_query(r"@evt.name:Add\ *member\ to\ role* -@role:*Global\ Administrator* timeout");
        assert_eq!(
            t,
            vec![
                Term::Tag { key: "@evt.name".into(), values: vec!["Add *member to role*".into()], negate: false },
                Term::Tag { key: "@role".into(), values: vec!["*Global Administrator*".into()], negate: true },
                Term::Text { text: "timeout".into(), negate: false },
            ]
        );
        assert_eq!(parse_query(r"@a:(x\ y OR z)"), vec![Term::Tag { key: "@a".into(), values: vec!["x y".into(), "z".into()], negate: false }]);
    }

    #[test]
    fn escaped_quotes_and_backslashes_stay_in_quoted_values() {
        // Datadog's syntax: inside a quoted value `\"` is a quote and `\\` a backslash.
        let t = parse_query(r#"@msg:"say \"hi\": a\\b" host:web-1 "C:\\dir \"x\"""#);
        assert_eq!(
            t,
            vec![
                Term::Tag { key: "@msg".into(), values: vec![r#"say "hi": a\b"#.into()], negate: false },
                Term::Tag { key: "host".into(), values: vec!["web-1".into()], negate: false },
                Term::Text { text: r#"C:\dir "x""#.into(), negate: false },
            ]
        );
        // A value ending in a backslash closes its quotes.
        assert_eq!(parse_query(r#"@path:"a\\" b"#), vec![Term::Tag { key: "@path".into(), values: vec![r"a\".into()], negate: false }, Term::Text { text: "b".into(), negate: false }]);
        assert_eq!(parse_query(r#"@file:"C:\temp""#), vec![Term::Tag { key: "@file".into(), values: vec![r"C:\temp".into()], negate: false }]);
        assert_eq!(parse_query(r#"@a:("x \"y\"" OR z)"#), vec![Term::Tag { key: "@a".into(), values: vec![r#"x "y""#.into(), "z".into()], negate: false }]);
    }

    #[test]
    fn urls_are_text_not_tags() {
        // Keys may only contain [A-Za-z0-9_.-] (plus a leading @), so "/api/v1:thing" stays free text.
        assert!(matches!(parse_query("GET /api/v1:thing").as_slice(), [Term::Text { .. }, Term::Text { .. }]));
    }
}
