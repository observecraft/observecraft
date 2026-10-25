//! The same search syntax compiled for DuckDB over archived log Parquet files (columns as in
//! `oc.logs`: `tags` / `attr` are `MAP(VARCHAR, VARCHAR)`).
//!
//! Differences from ClickHouse handled here: map lookups go through `map_extract` (a missing key
//! reads as '' like ClickHouse), LIKE needs an explicit escape character, and "no filter" is `true`.

use crate::sql::{build_where, Param, Schema, SqlFilter};
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse_query;

    #[test]
    fn duckdb_dialect() {
        let f = build_where_duckdb(&parse_query("service:(a OR b*) -env:qa @http.method:GET oops"));
        assert_eq!(
            f.clause,
            "(service = ? OR service ILIKE ? ESCAPE '\\') AND NOT (coalesce(map_extract(tags, ?)[1], '') = ?) AND (coalesce(map_extract(attr, ?)[1], '') = ?) AND (lower(message) LIKE ? ESCAPE '\\')"
        );
        assert_eq!(f.params.len(), 7);
        assert_eq!(build_where_duckdb(&[]).clause, "true");
    }

    #[test]
    fn trace_ids_are_columns() {
        let f = build_where_duckdb(&parse_query("trace_id:0af7651916cd43dd8448eb211c80319c"));
        assert_eq!(f.clause, "(trace_id = ?)");
    }
}
