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

    /// Every value a dotted `@` key reaches inside arrays of objects (the JSON of `attr[prefix]`), as the SQL's
    /// `element_values`: strings as they are, other values as JSON, empty ones left out.
    pub fn element_values(&self, key: &str) -> Vec<String> {
        let Some(path) = key.strip_prefix('@') else { return vec![] };
        let mut out = vec![];
        for (prefix, rest) in crate::sql::array_paths(path) {
            let Some(raw) = self.attr.get(&prefix).filter(|r| r.starts_with('[')) else { continue };
            let Ok(serde_json::Value::Array(items)) = serde_json::from_str::<serde_json::Value>(raw) else { continue };
            for item in &items {
                let v = rest.iter().try_fold(item, |v, k| v.get(k));
                match v {
                    Some(serde_json::Value::String(s)) if !s.is_empty() => out.push(s.clone()),
                    Some(serde_json::Value::String(_) | serde_json::Value::Null) | None => {}
                    Some(other) => out.push(other.to_string()),
                }
            }
        }
        out
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

/// A numeric comparison (`>46080`) of the value, as the SQL makes it; None when the pattern isn't one.
fn compare(pattern: &str, value: &str) -> Option<bool> {
    let (op, number) = crate::sql::comparison(pattern)?;
    let (v, n) = (value.trim().parse::<f64>().ok()?, number.parse::<f64>().ok()?);
    Some(match op {
        ">" => v > n,
        ">=" => v >= n,
        "<" => v < n,
        _ => v <= n,
    })
}

pub fn matches(terms: &[Term], f: &Fields<'_>) -> bool {
    terms.iter().all(|t| match t {
        Term::Text { text, negate } => f.message.to_lowercase().contains(&text.to_lowercase()) != *negate,
        Term::Tag { key, .. } if key == "index" && f.index.is_none() => true,
        Term::Tag { key, values, negate } => {
            let hit_one = |v: &str| values.iter().any(|p| compare(p, v).unwrap_or_else(|| glob(p, v)));
            let hit = f.get(key).is_some_and(hit_one) || f.element_values(key).iter().any(|v| hit_one(v));
            hit != *negate
        }
        Term::Any { alternatives, negate } => alternatives.iter().any(|a| matches(a, f)) != *negate,
    })
}

/// Whether the event matches at least one of `searches` (none: it matches nothing), like `build_any`.
pub fn matches_any(searches: &[Vec<Term>], f: &Fields<'_>) -> bool {
    searches.iter().any(|terms| matches(terms, f))
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
        assert!(matches(&parse_query("(@http.method:POST OR env:prod) service:order*"), &f));
        assert!(!matches(&parse_query("(@http.method:POST OR env:qa)"), &f));
        assert!(!matches(&parse_query("-(@http.method:POST OR env:prod)"), &f));
        assert!(matches(&parse_query("(env:qa OR (status:error bad))"), &f));
    }

    #[test]
    fn matches_any_of_several_searches() {
        let none = HashMap::new();
        let f = Fields { service: "billing", status: "info", host: "h1", source: "go", message: "charged", tags: &none, attr: &none, index: None };
        assert!(matches_any(&[parse_query("service:catalog-service"), parse_query("service:billing")], &f));
        assert!(!matches_any(&[parse_query("service:catalog-service")], &f));
        assert!(!matches_any(&[], &f), "no searches match nothing");
    }

    #[test]
    fn numeric_comparisons_compare_numbers() {
        let none = HashMap::new();
        let attr = HashMap::from([("auth.token_ttl".to_string(), "2764800".to_string())]);
        let f = Fields { service: "vault", status: "info", host: "h1", source: "vault", message: "", tags: &none, attr: &attr, index: None };
        assert!(matches(&parse_query("@auth.token_ttl:>46080"), &f));
        assert!(!matches(&parse_query("@auth.token_ttl:<=46080"), &f));
        assert!(!matches(&parse_query("@missing:>1"), &f));
    }

    /// Arrays of objects are flattened as Datadog does: any element matches (`@install_targets.name:urllib3`).
    #[test]
    fn dotted_keys_reach_into_arrays_of_objects() {
        let none = HashMap::new();
        let attr = HashMap::from([
            ("install_targets".to_string(), r#"[{"name":"left-pad","version":"1.3.0"},{"name":"urllib3","n":2,"meta":{"ok":true}}]"#.to_string()),
            ("evt.outcome".to_string(), "ALLOW".to_string()),
        ]);
        let f = Fields { service: "scfw", status: "info", host: "h1", source: "scfw", message: "", tags: &none, attr: &attr, index: None };
        assert!(matches(&parse_query("@install_targets.name:urllib3 @evt.outcome:ALLOW"), &f));
        assert!(matches(&parse_query("@install_targets.name:url* @install_targets.n:>1 @install_targets.meta.ok:true"), &f));
        assert!(!matches(&parse_query("@install_targets.name:requests"), &f));
        assert!(!matches(&parse_query("-@install_targets.name:urllib3"), &f));
        assert_eq!(f.element_values("@install_targets.name"), ["left-pad", "urllib3"]);
    }

    #[test]
    fn glob_edges() {
        assert!(glob("*service", "order-service"));
        assert!(glob("or*s*", "order-service"));
        assert!(!glob("or*z", "order-service"));
    }
}
