//! Search syntax shared by Logs, Error Tracking, … (mirrors `web/src/lib/searchQuery.ts`):
//!
//! ```text
//! service:order-service            tag / reserved attribute
//! service:(a OR b)              union
//! service:order*              wildcard
//! @http.status_code:5*          event attribute (dotted path)
//! -status:info                  negation
//! "bad response"  timeout       free text on the message
//! ```
//! Terms are ANDed.

mod duck;
mod matcher;
mod metrics;
mod parse;
mod sql;
pub mod sql_editor;

pub use duck::{build_any_duckdb, build_where_duckdb, ArchiveLogsSchema};
pub use matcher::{glob, matches, matches_any, Fields};
pub use metrics::{
    metric_field, metric_filter, metric_index_filter, metric_tag, parse_metric_query, resource_alias, short_tag, span_field,
    IndexFilter, MetricQuery, Rollup, SpaceAgg, TagFilter, TAG_EFFECTIVE, TAG_POINT, TAG_RESOURCE, TAG_SERVICE,
};
pub use parse::{parse_query, tokenize, Term};
pub use sql::{build_any, build_where, LogsSchema, Param, Schema, SpansSchema, SqlFilter};
