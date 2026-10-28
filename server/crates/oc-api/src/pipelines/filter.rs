//! Pipeline and category filters: the log search syntax (`oc_query`, as index filters use it),
//! plus what integration pipelines also rely on: numeric ranges and comparisons on attributes
//! (`@http.status_code:[400 TO 499]`, `@duration:>=1000`) and quoted values matched literally
//! (`@severity: "*"`). `*` alone matches every log.

use std::sync::OnceLock;

use fancy_regex::Regex;
use oc_query::{matches, parse_query, Fields, Term};

pub struct Filter {
    terms: Vec<Term>,
    clauses: Vec<Clause>,
}

/// A clause `oc_query` doesn't express, on one key.
struct Clause {
    key: String,
    test: Test,
    negate: bool,
}

enum Test {
    /// `key:[lo TO hi]` (`{…}` excludes the bounds, `*` leaves one open) or `key:>=n`.
    Range(Bound, Bound),
    /// `key:"value"`: equal, ignoring case, with no wildcards.
    Literal(String),
}

#[derive(Clone, Copy)]
enum Bound {
    Open,
    Incl(f64),
    Excl(f64),
}

fn clause_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r#"(?:^|(?<=\s))(-?)(@?[\w.\-]+):(?:([\[{])\s*(\S+)\s+TO\s+(\S+?)\s*([\]}])|(>=|<=|>|<)([^\s)]+)|\s*"((?:[^"\\]|\\.)*)")(?=\s|$)"#).expect("filter clause regex"))
}

impl Filter {
    pub fn parse(query: &str) -> Filter {
        let mut clauses = Vec::new();
        let mut rest = String::new();
        let mut last = 0;
        for c in clause_re().captures_iter(query).flatten() {
            let whole = c.get(0).expect("match");
            rest.push_str(&query[last..whole.start()]);
            last = whole.end();
            let g = |i: usize| c.get(i).map_or("", |m| m.as_str());
            let bound = |v: &str, inclusive: bool| match v.parse::<f64>() {
                Ok(n) if inclusive => Bound::Incl(n),
                Ok(n) => Bound::Excl(n),
                Err(_) => Bound::Open,
            };
            let test = if let Some(lit) = c.get(9) {
                Test::Literal(lit.as_str().replace("\\\"", "\"").to_lowercase())
            } else if !g(3).is_empty() {
                Test::Range(bound(g(4), g(3) == "["), bound(g(5), g(6) == "]"))
            } else {
                match g(7) {
                    ">=" => Test::Range(bound(g(8), true), Bound::Open),
                    ">" => Test::Range(bound(g(8), false), Bound::Open),
                    "<=" => Test::Range(Bound::Open, bound(g(8), true)),
                    _ => Test::Range(Bound::Open, bound(g(8), false)),
                }
            };
            clauses.push(Clause { key: g(2).to_string(), test, negate: g(1) == "-" });
        }
        rest.push_str(&query[last..]);
        let terms = parse_query(&rest).into_iter().filter(|t| !matches!(t, Term::Text { text, negate: false } if text == "*")).collect();
        Filter { terms, clauses }
    }

    pub fn matches(&self, f: &Fields<'_>) -> bool {
        matches(&self.terms, f) && self.clauses.iter().all(|c| c.matches(f))
    }
}

impl Clause {
    fn matches(&self, f: &Fields<'_>) -> bool {
        let value = match self.key.as_str() {
            "service" => Some(f.service),
            "status" => Some(f.status),
            "host" => Some(f.host),
            "source" => Some(f.source),
            k if k.starts_with('@') => f.attr.get(&k[1..]).map(String::as_str),
            k => f.tags.get(k).map(String::as_str),
        };
        let hit = match &self.test {
            Test::Literal(want) => value.is_some_and(|v| v.to_lowercase() == *want),
            Test::Range(lo, hi) => value.and_then(|v| v.trim().parse::<f64>().ok()).is_some_and(|n| {
                let above = match *lo {
                    Bound::Open => true,
                    Bound::Incl(lo) => n >= lo,
                    Bound::Excl(lo) => n > lo,
                };
                let below = match *hi {
                    Bound::Open => true,
                    Bound::Incl(hi) => n <= hi,
                    Bound::Excl(hi) => n < hi,
                };
                above && below
            }),
        };
        hit != self.negate
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn check(query: &str, attr: &[(&str, &str)]) -> bool {
        let tags = HashMap::from([("env".to_string(), "prod".to_string())]);
        let attr: HashMap<String, String> = attr.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        let f = Fields { service: "storefront-api", status: "info", host: "i-0a1b2c3d4e5f60001", source: "nginx", message: "GET /cart 200", tags: &tags, attr: &attr, index: None };
        Filter::parse(query).matches(&f)
    }

    #[test]
    fn search_syntax() {
        assert!(check("", &[]));
        assert!(check("*", &[]));
        assert!(check("source:nginx env:prod", &[]));
        assert!(check("service:(billing OR storefront-*) cart", &[]));
        assert!(!check("source:nginx -env:prod", &[]));
        assert!(check("@http.method:GET", &[("http.method", "GET")]));
        assert!(!check("@http.method:*", &[]));
    }

    #[test]
    fn quoted_values_are_literal() {
        // Redis's pipeline tells severities apart by symbol, `*` included.
        assert!(check(r#"@severity: "*""#, &[("severity", "*")]));
        assert!(!check(r#"@severity: "*""#, &[("severity", "#")]));
        assert!(check(r##"@severity:"#" source:nginx"##, &[("severity", "#")]));
        assert!(!check(r##"-@severity:"#""##, &[("severity", "#")]));
        assert!(check(r#"@msg:"Disk Full""#, &[("msg", "disk full")]));
    }

    #[test]
    fn numeric_ranges_and_comparisons() {
        fn code(c: &str) -> Vec<(&str, &str)> {
            vec![("http.status_code", c)]
        }
        assert!(check("@http.status_code:[200 TO 299]", &code("200")));
        assert!(check("@http.status_code:[200 TO 299]", &code("299")));
        assert!(!check("@http.status_code:{200 TO 299}", &code("299")));
        assert!(!check("@http.status_code:[200 TO 299]", &code("404")));
        assert!(!check("@http.status_code:[200 TO 299]", &code("abc")));
        assert!(!check("@http.status_code:[200 TO 299]", &[]));
        assert!(check("-@http.status_code:[200 TO 299] source:nginx", &code("500")));
        assert!(check("@http.status_code:[500 TO *]", &code("503")));
        assert!(check("@duration:>=1000", &[("duration", "1000")]));
        assert!(!check("@duration:>1000", &[("duration", "1000")]));
        assert!(check("@duration:<10.5", &[("duration", "3")]));
        assert!(check("@duration:<=3 env:prod", &[("duration", "3")]));
    }
}
