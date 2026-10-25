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

mod matcher;
mod metrics;
mod parse;
mod sql;

pub use matcher::{matches, Fields};
pub use metrics::{metric_field, metric_filter, parse_metric_query, resource_alias, short_tag, span_field, MetricQuery, SpaceAgg, TagFilter};
pub use parse::{parse_query, tokenize, Term};
pub use sql::{build_where, LogsSchema, Param, Schema, SpansSchema, SqlFilter};
