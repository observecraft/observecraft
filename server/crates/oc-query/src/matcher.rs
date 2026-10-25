use crate::Term;
use std::collections::HashMap;

/// The fields of an event as seen by the matcher (live tail evaluates in memory, not in ClickHouse).
pub struct Fields<'a> {
    pub service: &'a str,
    pub status: &'a str,
    pub host: &'a str,
    pub source: &'a str,
    pub message: &'a str,
    pub tags: &'a HashMap<String, String>,
    pub attr: &'a HashMap<String, String>,
    /// The log's index, once it has one (unknown before indexing, e.g. live tail): `index:` terms
    /// then match everything.
    pub index: Option<&'a str>,
}

impl<'a> Fields<'a> {
    fn get(&self, key: &str) -> Option<&'a str> {
        match key {
            "service" => Some(self.service),
            "status" => Some(self.status),
            "host" => Some(self.host),
            "source" => Some(self.source),
            "index" => self.index,
            k if k.starts_with('@') => self.attr.get(&k[1..]).map(String::as_str),
            k => self.tags.get(k).map(String::as_str),
        }
    }
}

/// Case-insensitive glob match supporting `*`.
pub fn glob(pattern: &str, value: &str) -> bool {
    let p = pattern.to_lowercase();
    let v = value.to_lowercase();
    if !p.contains('*') {
        return p == v;
    }
    let parts: Vec<&str> = p.split('*').collect();
    let mut pos = 0;
    for (i, part) in parts.iter().enumerate() {
        if part.is_empty() {
            continue;
        }
        if i == 0 {
            if !v.starts_with(part) {
                return false;
            }
            pos = part.len();
        } else if let Some(found) = v[pos..].find(part) {
            pos += found + part.len();
        } else {
            return false;
        }
    }
    parts.last().is_some_and(|l| l.is_empty() || v.ends_with(l))
}

pub fn matches(terms: &[Term], f: &Fields<'_>) -> bool {
    terms.iter().all(|t| match t {
        Term::Text { text, negate } => f.message.to_lowercase().contains(&text.to_lowercase()) != *negate,
        Term::Tag { key, .. } if key == "index" && f.index.is_none() => true,
        Term::Tag { key, values, negate } => {
            let hit = f.get(key).is_some_and(|v| values.iter().any(|p| glob(p, v)));
            hit != *negate
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse_query;

    #[test]
    fn matches_like_the_sql() {
        let tags = HashMap::from([("env".to_string(), "prod".to_string())]);
        let attr = HashMap::from([("http.method".to_string(), "GET".to_string())]);
        let f = Fields { service: "order-service", status: "error", host: "h1", source: "go", message: "GetCustomer bad response", tags: &tags, attr: &attr, index: Some("main") };
        assert!(matches(&parse_query("service:order* env:prod @http.method:get \"bad response\""), &f));
        assert!(!matches(&parse_query("-status:error"), &f));
        assert!(matches(&parse_query("service:(a OR order-service)"), &f));
        assert!(!matches(&parse_query("env:qa"), &f));
    }

    #[test]
    fn glob_edges() {
        assert!(glob("*service", "order-service"));
        assert!(glob("or*s*", "order-service"));
        assert!(!glob("or*z", "order-service"));
    }
}
