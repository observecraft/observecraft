use crate::Term;

/// A bound query parameter. Everything user-supplied — including map keys — is bound,
/// never interpolated into SQL text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Param {
    Str(String),
    Int(i64),
}

#[derive(Debug, Default, Clone)]
pub struct SqlFilter {
    /// Boolean SQL expression with `?` placeholders (ClickHouse client binding syntax).
    pub clause: String,
    pub params: Vec<Param>,
}

/// Maps query keys onto a table's columns. Expressions may contain one `?` for a bound map key.
pub trait Schema: Sync {
    /// SQL expression for `key`, plus the bound key param when the expression is a map lookup.
    fn field(&self, key: &str) -> (String, Option<Param>);
    /// Lower-cased expression free-text terms are matched against.
    fn text_expr(&self) -> &'static str;
    /// Keys accepted but not filtered on (e.g. `index:` while there is one index).
    fn ignored(&self, _key: &str) -> bool {
        false
    }
}

/// `oc.logs`: reserved columns (`index` is the log index), `@attr.path` → `attr[...]`, anything else → `tags[...]`.
pub struct LogsSchema;

impl Schema for LogsSchema {
    fn field(&self, key: &str) -> (String, Option<Param>) {
        match key {
            "service" | "status" | "host" | "source" | "trace_id" | "span_id" => (key.to_string(), None),
            "index" => ("index_name".into(), None),
            k if k.starts_with('@') => ("attr[?]".into(), Some(Param::Str(k[1..].to_string()))),
            k => ("tags[?]".into(), Some(Param::Str(k.to_string()))),
        }
    }
    fn text_expr(&self) -> &'static str {
        "lower(message)"
    }
}

/// `otel.otel_traces` (OTel Collector ClickHouse exporter schema).
pub struct SpansSchema;

impl Schema for SpansSchema {
    fn field(&self, key: &str) -> (String, Option<Param>) {
        let res = |k: &str| ("ResourceAttributes[?]".to_string(), Some(Param::Str(k.to_string())));
        match key {
            "service" => ("ServiceName".into(), None),
            "resource" | "operation" | "name" => ("SpanName".into(), None),
            "status" => ("lower(StatusCode)".into(), None),
            "kind" => ("lower(SpanKind)".into(), None),
            "trace_id" => ("TraceId".into(), None),
            "env" => res("deployment.environment"),
            "version" => res("service.version"),
            "host" => res("host.name"),
            k if k.starts_with('@') => ("SpanAttributes[?]".into(), Some(Param::Str(k[1..].to_string()))),
            k => res(k),
        }
    }
    fn text_expr(&self) -> &'static str {
        "lower(concat(SpanName, ' ', StatusMessage))"
    }
}

/// `*` wildcards → SQL LIKE pattern, with LIKE metacharacters escaped.
pub(crate) fn like_pattern(p: &str) -> String {
    let mut out = String::with_capacity(p.len() + 2);
    for c in p.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '%' => out.push_str("\\%"),
            '_' => out.push_str("\\_"),
            '*' => out.push('%'),
            c => out.push(c),
        }
    }
    out
}

/// Builds the WHERE fragment for the terms against `schema`. `skip_key` drops that facet's own
/// filters (used for facet value counts).
pub fn build_where(schema: &dyn Schema, terms: &[Term], skip_key: Option<&str>) -> SqlFilter {
    let mut parts = Vec::new();
    let mut params = Vec::new();
    for term in terms {
        match term {
            Term::Text { text, negate } => {
                parts.push(format!("{}({} LIKE ?)", if *negate { "NOT " } else { "" }, schema.text_expr()));
                params.push(Param::Str(format!("%{}%", like_pattern(&text.to_lowercase()))));
            }
            Term::Tag { key, values, negate } => {
                if schema.ignored(key) || Some(key.as_str()) == skip_key || values.is_empty() {
                    continue;
                }
                let (expr, key_param) = schema.field(key);
                let mut ors = Vec::new();
                for v in values {
                    if let Some(kp) = &key_param {
                        params.push(kp.clone());
                    }
                    if v.contains('*') {
                        ors.push(format!("{expr} ILIKE ?"));
                        params.push(Param::Str(like_pattern(v)));
                    } else {
                        ors.push(format!("{expr} = ?"));
                        params.push(Param::Str(v.clone()));
                    }
                }
                let joined = ors.join(" OR ");
                parts.push(if *negate { format!("NOT ({joined})") } else { format!("({joined})") });
            }
        }
    }
    SqlFilter { clause: if parts.is_empty() { "1".into() } else { parts.join(" AND ") }, params }
}

/// Rows matching at least one of `searches` (`(a) OR (b)`, each search's terms ANDed as in
/// [`build_where`]); no searches match nothing (`0`). Log restriction queries use it.
pub fn build_any(schema: &dyn Schema, searches: &[Vec<Term>]) -> SqlFilter {
    any_of(searches.iter().map(|s| build_where(schema, s, None)).collect(), "0")
}

/// ORs the filters; `none` is the clause when there are none.
pub(crate) fn any_of(filters: Vec<SqlFilter>, none: &str) -> SqlFilter {
    if filters.is_empty() {
        return SqlFilter { clause: none.into(), params: Vec::new() };
    }
    let mut params = Vec::new();
    let mut parts = Vec::new();
    for f in filters {
        parts.push(format!("({})", f.clause));
        params.extend(f.params);
    }
    SqlFilter { clause: parts.join(" OR "), params }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse_query;

    #[test]
    fn builds_bound_sql() {
        let f = build_where(&LogsSchema, &parse_query("service:(a OR b*) -env:qa @http.method:GET oops"), None);
        assert_eq!(f.clause, "(service = ? OR service ILIKE ?) AND NOT (tags[?] = ?) AND (attr[?] = ?) AND (lower(message) LIKE ?)");
        assert_eq!(
            f.params,
            vec![
                Param::Str("a".into()),
                Param::Str("b%".into()),
                Param::Str("env".into()),
                Param::Str("qa".into()),
                Param::Str("http.method".into()),
                Param::Str("GET".into()),
                Param::Str("%oops%".into()),
            ]
        );
    }

    #[test]
    fn escapes_like_metacharacters_and_skips_facet() {
        let f = build_where(&LogsSchema, &parse_query("service:x 100%_done"), Some("service"));
        assert_eq!(f.clause, "(lower(message) LIKE ?)");
        assert_eq!(f.params, vec![Param::Str("%100\\%\\_done%".into())]);
    }

    #[test]
    fn hostile_keys_are_bound_not_interpolated() {
        // A key cannot contain quotes (the parser rejects it), and accepted keys go through a bound param.
        let f = build_where(&LogsSchema, &parse_query("x']:y"), None);
        assert!(!f.clause.contains("x'"));
    }

    #[test]
    fn any_of_several_searches_is_an_or_of_bound_clauses() {
        let searches = vec![parse_query("service:billing"), parse_query("env:prod kube_namespace:payments")];
        let f = build_any(&LogsSchema, &searches);
        assert_eq!(f.clause, "((service = ?)) OR ((tags[?] = ?) AND (tags[?] = ?))");
        assert_eq!(
            f.params,
            vec![Param::Str("billing".into()), Param::Str("env".into()), Param::Str("prod".into()), Param::Str("kube_namespace".into()), Param::Str("payments".into())]
        );
        // Hostile values stay parameters.
        let f = build_any(&LogsSchema, &[parse_query("service:\"x') OR 1=1 --\"")]);
        assert!(!f.clause.contains("1=1"), "{}", f.clause);
    }

    #[test]
    fn any_of_no_searches_matches_nothing() {
        let f = build_any(&LogsSchema, &[]);
        assert_eq!(f.clause, "0");
        assert!(f.params.is_empty());
    }

    #[test]
    fn empty_query_is_true() {
        assert_eq!(build_where(&LogsSchema, &[], None).clause, "1");
    }

    #[test]
    fn spans_schema_maps_status_env_and_attributes() {
        let f = build_where(&SpansSchema, &parse_query("service:billing status:error env:prod @db.system:redis"), None);
        assert_eq!(f.clause, "(ServiceName = ?) AND (lower(StatusCode) = ?) AND (ResourceAttributes[?] = ?) AND (SpanAttributes[?] = ?)");
        assert_eq!(f.params[2], Param::Str("deployment.environment".into()));
    }
}
