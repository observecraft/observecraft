//! The same search syntax compiled for DuckDB over archived log Parquet files (columns as in
//! `oc.logs`: `tags` / `attr` are `MAP(VARCHAR, VARCHAR)`).
//!
//! Differences from ClickHouse handled here: map lookups go through `map_extract` (a missing key
//! reads as '' like ClickHouse), LIKE needs an explicit escape character, and "no filter" is `true`.

use crate::sql::{any_of, array_paths, build_where, Param, Schema, SqlFilter};
use crate::Term;

/// Archived `oc.logs` rows read by DuckDB.
pub struct ArchiveLogsSchema;

impl Schema for ArchiveLogsSchema {
    fn field(&self, key: &str) -> (String, Option<Param>) {
        match key {
            "service" | "status" | "host" | "source" | "trace_id" | "span_id" => (key.to_string(), None),
            k if k.starts_with('@') => ("coalesce(map_extract(attr, ?)[1], '')".into(), Some(Param::Str(k[1..].to_string()))),
            k => ("coalesce(map_extract(tags, ?)[1], '')".into(), Some(Param::Str(k.to_string()))),
        }
    }
    fn text_expr(&self) -> &'static str {
        "lower(message)"
    }
    fn ignored(&self, key: &str) -> bool {
        key == "index"
    }
    fn compare(&self, expr: &str, op: &str) -> String {
        format!("TRY_CAST({expr} AS DOUBLE) {op} CAST(? AS DOUBLE)")
    }
    fn element_match(&self, key: &str, cond: &str, cond_params: &[Param]) -> Option<(String, Vec<Param>)> {
        let paths = array_paths(key.strip_prefix('@')?);
        if paths.is_empty() {
            return None;
        }
        let (mut parts, mut params) = (vec![], vec![]);
        let held = "coalesce(map_extract(attr, ?)[1], '')";
        for (prefix, rest) in paths {
            // Only valid JSON arrays are read: json_extract fails on other text.
            parts.push(format!("(CASE WHEN starts_with({held}, '[') AND json_valid({held}) THEN len(list_filter(list_transform(json_extract({held}, '$[*]'), lambda _e: json_extract_string(_e, ?)), lambda _v: {cond})) > 0 ELSE false END)"));
            params.extend((0..3).map(|_| Param::Str(prefix.clone())));
            params.push(Param::Str(format!("${}", rest.iter().map(|k| format!(".\"{}\"", k.replace('"', "\\\""))).collect::<String>())));
            params.extend_from_slice(cond_params);
        }
        Some((parts.join(" OR "), params))
    }
}

/// WHERE fragment for DuckDB with `?` placeholders (DuckDB's positional parameters).
pub fn build_where_duckdb(terms: &[Term]) -> SqlFilter {
    let f = build_where(&ArchiveLogsSchema, terms, None);
    if f.clause == "1" {
        return SqlFilter { clause: "true".into(), params: f.params };
    }
    // Patterns come from `like_pattern`, which escapes with a backslash.
    let clause = f.clause.replace(" ILIKE ?", " ILIKE ? ESCAPE '\\'").replace(" LIKE ?", " LIKE ? ESCAPE '\\'");
    SqlFilter { clause, params: f.params }
}

/// Rows matching at least one of `searches`, for DuckDB; no searches match nothing (`false`).
pub fn build_any_duckdb(searches: &[Vec<Term>]) -> SqlFilter {
    any_of(searches.iter().map(|s| build_where_duckdb(s)).collect(), "false")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse_query;

    #[test]
    fn duckdb_numeric_comparisons() {
        let f = build_where_duckdb(&parse_query("@token_ttl:>=46080"));
        assert_eq!(f.clause, "(TRY_CAST(coalesce(map_extract(attr, ?)[1], '') AS DOUBLE) >= CAST(? AS DOUBLE))");
    }

    #[test]
    fn duckdb_dialect() {
        let f = build_where_duckdb(&parse_query("service:(a OR b*) -env:qa @method:GET oops"));
        assert_eq!(
            f.clause,
            "(service = ? OR service ILIKE ? ESCAPE '\\') AND NOT (coalesce(map_extract(tags, ?)[1], '') = ?) AND (coalesce(map_extract(attr, ?)[1], '') = ?) AND (lower(message) LIKE ? ESCAPE '\\')"
        );
        assert_eq!(f.params.len(), 7);
        assert_eq!(build_where_duckdb(&[]).clause, "true");
    }

    #[test]
    fn any_of_several_searches_in_duckdb() {
        let f = build_any_duckdb(&[parse_query("service:bill*"), parse_query("env:prod")]);
        assert_eq!(f.clause, "((service ILIKE ? ESCAPE '\\')) OR ((coalesce(map_extract(tags, ?)[1], '') = ?))");
        assert_eq!(f.params.len(), 3);
        assert_eq!(build_any_duckdb(&[]).clause, "false");
    }

    #[test]
    fn duckdb_reaches_into_arrays_of_objects() {
        let f = build_where_duckdb(&parse_query("@install_targets.name:url*"));
        assert!(f.clause.contains("list_filter(") && f.clause.contains("json_extract_string(_e, ?)") && f.clause.contains("_v ILIKE ? ESCAPE '\\'"), "{}", f.clause);
        assert_eq!(f.clause.matches('?').count(), f.params.len());
        assert!(f.params.contains(&Param::Str("$.\"name\"".into())), "{:?}", f.params);
    }

    #[test]
    fn trace_ids_are_columns() {
        let f = build_where_duckdb(&parse_query("trace_id:0af7651916cd43dd8448eb211c80319c"));
        assert_eq!(f.clause, "(trace_id = ?)");
    }
}
