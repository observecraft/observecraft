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
    /// `expr <op> ?` as numbers, the bound value a number's text (`@duration:>1000`); NULL (no match) when not a number.
    fn compare(&self, expr: &str, op: &str) -> String {
        format!("toFloat64OrNull({expr}) {op} toFloat64(?)")
    }
    /// `cond` (on `_v`, with its params) for any element value `key` reaches inside an array of objects
    /// (`@install_targets.name`), ORed after the key's own match; None for tables without such arrays.
    fn element_match(&self, _key: &str, _cond: &str, _cond_params: &[Param]) -> Option<(String, Vec<Param>)> {
        None
    }
}

/// The arrays a dotted attribute path may reach into, as Datadog flattens arrays of objects: each strict prefix
/// (`install_targets` of `install_targets.name`) with the path inside its elements (`name`).
pub(crate) fn array_paths(path: &str) -> Vec<(String, Vec<String>)> {
    let segments: Vec<&str> = path.split('.').collect();
    if segments.iter().any(|s| s.is_empty()) {
        return vec![];
    }
    (1..segments.len()).map(|i| (segments[..i].join("."), segments[i..].iter().map(|s| s.to_string()).collect())).collect()
}

/// The values of `rest` in the elements of the JSON array `attr[prefix]`, as `Array(String)`: strings as they
/// are, other values as JSON (`2`, `true`), as the log normalizer writes attributes.
fn ch_elements(prefix: &str, rest: &[String]) -> (String, Vec<Param>) {
    let keys = vec!["?"; rest.len()].join(", ");
    let mut params: Vec<Param> = (0..3).flat_map(|_| rest.iter().map(|k| Param::Str(k.clone()))).collect();
    params.push(Param::Str(prefix.to_string()));
    (format!("arrayMap(_e -> if(JSONType(_e, {keys}) = 'String', JSONExtractString(_e, {keys}), JSONExtractRaw(_e, {keys})), JSONExtractArrayRaw(attr[?]))"), params)
}

impl LogsSchema {
    /// Every value `key` reaches inside arrays of objects, as `Array(String)` without empty values (a group-by
    /// counts a log in each element's group); None when the key can't reach into one (not a dotted `@` path).
    pub fn element_values(&self, key: &str) -> Option<(String, Vec<Param>)> {
        let paths = array_paths(key.strip_prefix('@')?);
        if paths.is_empty() {
            return None;
        }
        let (mut parts, mut params) = (vec![], vec![]);
        for (prefix, rest) in &paths {
            let (e, p) = ch_elements(prefix, rest);
            parts.push(e);
            params.extend(p);
        }
        Some((format!("arrayFilter(_v -> _v != '', arrayConcat({}))", parts.join(", ")), params))
    }
}

/// `>46080`, `<=1.5`: a numeric comparison (operator, number text) of a search value.
pub fn comparison(value: &str) -> Option<(&'static str, &str)> {
    let (op, rest) = [">=", "<=", ">", "<"].iter().find_map(|op| value.strip_prefix(op).map(|rest| (*op, rest)))?;
    rest.parse::<f64>().ok().filter(|n| n.is_finite()).map(|_| (op, rest))
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
    fn element_match(&self, key: &str, cond: &str, cond_params: &[Param]) -> Option<(String, Vec<Param>)> {
        let paths = array_paths(key.strip_prefix('@')?);
        if paths.is_empty() {
            return None;
        }
        let (mut parts, mut params) = (vec![], vec![]);
        for (prefix, rest) in &paths {
            let (e, p) = ch_elements(prefix, rest);
            parts.push(format!("(mapContains(attr, ?) AND arrayExists(_v -> {cond}, {e}))"));
            params.push(Param::Str(prefix.clone()));
            params.extend_from_slice(cond_params);
            params.extend(p);
        }
        Some((parts.join(" OR "), params))
    }
}

/// `otel.otel_traces` (OTel Collector ClickHouse exporter schema).
pub struct SpansSchema;

/// A span's Kubernetes cluster and namespace (the service page's primary tags).
pub const KUBE_CLUSTER: &str = "if(ResourceAttributes['k8s.cluster.name'] != '', ResourceAttributes['k8s.cluster.name'], ResourceAttributes['kube_cluster_name'])";
pub const KUBE_NAMESPACE: &str = "if(ResourceAttributes['k8s.namespace.name'] != '', ResourceAttributes['k8s.namespace.name'], ResourceAttributes['kube_namespace'])";

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
            // Datadog's Kubernetes tags: mapped to OpenTelemetry's attributes by the intake, else kept as sent.
            "kube_cluster_name" => (KUBE_CLUSTER.into(), None),
            "kube_namespace" => (KUBE_NAMESPACE.into(), None),
            k if k.starts_with('@') => ("SpanAttributes[?]".into(), Some(Param::Str(k[1..].to_string()))),
            k => res(k),
        }
    }
    fn text_expr(&self) -> &'static str {
        "lower(concat(SpanName, ' ', StatusMessage))"
    }
}

/// `otel.otel_rum` (RUM events): `@path` → the event's flattened field, `type` the event type, `service`,
/// `env` and `version` as Datadog's RUM facets name them; other keys are event fields too (`application.id`,
/// `session.type`, …), since RUM events carry no separate tags.
pub struct RumSchema;

impl Schema for RumSchema {
    fn field(&self, key: &str) -> (String, Option<Param>) {
        let attr = |k: &str| ("LogAttributes[?]".to_string(), Some(Param::Str(k.to_string())));
        match key {
            "type" | "@type" => ("EventType".into(), None),
            "service" | "@service" => ("ServiceName".into(), None),
            "env" => ("ResourceAttributes[?]".into(), Some(Param::Str("deployment.environment".into()))),
            "version" | "@version" => ("ResourceAttributes[?]".into(), Some(Param::Str("service.version".into()))),
            k => attr(k.strip_prefix('@').unwrap_or(k)),
        }
    }
    fn text_expr(&self) -> &'static str {
        "lower(Body)"
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
                    let (op, value) = match comparison(v) {
                        Some((op, number)) => (op, number.to_string()),
                        None if v.contains('*') => ("ILIKE", like_pattern(v)),
                        None => ("=", v.clone()),
                    };
                    let cond = |e: &str| if matches!(op, "=" | "ILIKE") { format!("{e} {op} ?") } else { schema.compare(e, op) };
                    let value = Param::Str(value);
                    params.push(value.clone());
                    match schema.element_match(key, &cond("_v"), std::slice::from_ref(&value)) {
                        Some((elements, p)) => {
                            ors.push(format!("{} OR {elements}", cond(&expr)));
                            params.extend(p);
                        }
                        None => ors.push(cond(&expr)),
                    }
                }
                let joined = ors.join(" OR ");
                parts.push(if *negate { format!("NOT ({joined})") } else { format!("({joined})") });
            }
            Term::Any { alternatives, negate } => {
                let any = any_of(alternatives.iter().map(|a| build_where(schema, a, skip_key)).collect(), "0");
                parts.push(format!("{}({})", if *negate { "NOT " } else { "" }, any.clause));
                params.extend(any.params);
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
    fn rum_fields_are_event_attributes() {
        let f = build_where(&RumSchema, &crate::parse_query("@type:view @view.name:/checkout* service:storefront-web -@session.type:synthetics"), None);
        assert_eq!(f.clause, "(EventType = ?) AND (LogAttributes[?] ILIKE ?) AND (ServiceName = ?) AND NOT (LogAttributes[?] = ?)");
        assert_eq!(f.params[0], Param::Str("view".into()));
        assert_eq!(f.params[1], Param::Str("view.name".into()));
        assert_eq!(f.params[5], Param::Str("synthetics".into()));
    }

    #[test]
    fn groups_across_attributes() {
        let f = build_where(&LogsSchema, &parse_query("source:scfw (@warning:true OR @warned:true) -(env:qa OR oops)"), None);
        assert_eq!(f.clause, "(source = ?) AND (((attr[?] = ?)) OR ((attr[?] = ?))) AND NOT (((tags[?] = ?)) OR ((lower(message) LIKE ?)))");
        assert_eq!(f.params.len(), 8);
        assert_eq!((&f.params[1], &f.params[2], &f.params[3]), (&Param::Str("warning".into()), &Param::Str("true".into()), &Param::Str("warned".into())));
    }

    #[test]
    fn builds_bound_sql() {
        let f = build_where(&LogsSchema, &parse_query("service:(a OR b*) -env:qa @method:GET oops"), None);
        assert_eq!(f.clause, "(service = ? OR service ILIKE ?) AND NOT (tags[?] = ?) AND (attr[?] = ?) AND (lower(message) LIKE ?)");
        assert_eq!(
            f.params,
            vec![
                Param::Str("a".into()),
                Param::Str("b%".into()),
                Param::Str("env".into()),
                Param::Str("qa".into()),
                Param::Str("method".into()),
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
    fn numeric_comparisons_compare_numbers() {
        let f = build_where(&LogsSchema, &parse_query("source:vault @token_ttl:>46080 @duration:<=1.5"), None);
        assert_eq!(f.clause, "(source = ?) AND (toFloat64OrNull(attr[?]) > toFloat64(?)) AND (toFloat64OrNull(attr[?]) <= toFloat64(?))");
        assert_eq!(f.params[1..3], [Param::Str("token_ttl".into()), Param::Str("46080".into())]);
        // Not a number: compared as text, as before.
        assert_eq!(build_where(&LogsSchema, &parse_query("@path:>x"), None).clause, "(attr[?] = ?)");
    }

    /// Datadog flattens arrays of objects: `@install_targets.name:urllib3` matches a log whose `install_targets`
    /// array (stored as JSON) has an element named urllib3. The array's key guards it, for the key bloom filter.
    #[test]
    fn dotted_keys_reach_into_arrays_of_objects() {
        let f = build_where(&LogsSchema, &parse_query("@install_targets.name:urllib3"), None);
        let element = "if(JSONType(_e, ?) = 'String', JSONExtractString(_e, ?), JSONExtractRaw(_e, ?))";
        assert_eq!(f.clause, format!("(attr[?] = ? OR (mapContains(attr, ?) AND arrayExists(_v -> _v = ?, arrayMap(_e -> {element}, JSONExtractArrayRaw(attr[?])))))"));
        let s = |v: &str| Param::Str(v.into());
        assert_eq!(f.params, [s("install_targets.name"), s("urllib3"), s("install_targets"), s("urllib3"), s("name"), s("name"), s("name"), s("install_targets")]);
        // Every array a longer path may start at; a plain key has none.
        let f = build_where(&LogsSchema, &parse_query("-@a.b.c:>2"), None);
        assert_eq!(f.clause.matches("mapContains(attr, ?)").count(), 2, "{}", f.clause);
        assert!(f.clause.starts_with("NOT (toFloat64OrNull(attr[?]) > toFloat64(?) OR (mapContains(attr, ?) AND arrayExists(_v -> toFloat64OrNull(_v) > toFloat64(?), "), "{}", f.clause);
        assert_eq!(f.clause.matches('?').count(), f.params.len());
        assert_eq!(build_where(&LogsSchema, &parse_query("@warning:true"), None).clause, "(attr[?] = ?)");
    }

    /// Group-by over an element path: every element's value (a log counts in each group).
    #[test]
    fn element_values_of_a_dotted_key() {
        let (expr, params) = LogsSchema.element_values("@install_targets.name").unwrap();
        assert_eq!(expr, "arrayFilter(_v -> _v != '', arrayConcat(arrayMap(_e -> if(JSONType(_e, ?) = 'String', JSONExtractString(_e, ?), JSONExtractRaw(_e, ?)), JSONExtractArrayRaw(attr[?]))))");
        assert_eq!(params.len(), 4);
        assert!(LogsSchema.element_values("@warning").is_none() && LogsSchema.element_values("env").is_none());
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

    /// The service page's primary tags, as Datadog names them: the OpenTelemetry attribute the intake maps them to,
    /// else the tag kept under its own name.
    #[test]
    fn spans_schema_maps_kubernetes_primary_tags() {
        let f = build_where(&SpansSchema, &parse_query("kube_cluster_name:prod-eu kube_namespace:shop"), None);
        assert_eq!(
            f.clause,
            "(if(ResourceAttributes['k8s.cluster.name'] != '', ResourceAttributes['k8s.cluster.name'], ResourceAttributes['kube_cluster_name']) = ?) \
             AND (if(ResourceAttributes['k8s.namespace.name'] != '', ResourceAttributes['k8s.namespace.name'], ResourceAttributes['kube_namespace']) = ?)"
        );
        assert_eq!(f.params, [Param::Str("prod-eu".into()), Param::Str("shop".into())]);
    }
}
