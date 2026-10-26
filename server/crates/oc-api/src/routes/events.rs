//! Events (Datadog's event stream): what the Datadog Agent and the events API send through the
//! intake, stored in `otel.otel_events`.
//!
//! Search syntax: free text (title and text), and `source:`, `host:`, `status:` (alert type),
//! `priority:` and any `key:value` tag, each optionally negated with `-`.

use axum::extract::State;
use axum::Json;
use clickhouse::Row;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use super::logs::TimeRange;
use crate::error::ApiResult;
use crate::state::{AppState, OrgId};

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct EventSearchRequest {
    pub range: TimeRange,
    #[serde(default)]
    pub query: String,
    #[serde(default)]
    pub offset: usize,
    pub limit: usize,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct Event {
    /// Stable within a search (timestamp, host and title hash).
    pub id: String,
    /// Epoch milliseconds.
    pub timestamp: i64,
    pub title: String,
    pub text: String,
    /// error, warning, info or success.
    pub alert_type: String,
    pub priority: String,
    pub source: String,
    pub host: String,
    pub aggregation_key: String,
    pub tags: Vec<String>,
}

#[derive(Serialize, ToSchema)]
pub struct EventFacetValue {
    pub value: String,
    pub count: u64,
}

#[derive(Serialize, ToSchema)]
pub struct EventFacet {
    /// `source`, `status`, `priority` or `host`.
    pub key: String,
    pub label: String,
    pub group: String,
    pub values: Vec<EventFacetValue>,
}

/// Event counts in one time bucket, by status (error, warning, info, success).
#[derive(Serialize, ToSchema)]
pub struct EventBucket {
    /// Bucket start, epoch milliseconds.
    pub t: i64,
    pub counts: std::collections::BTreeMap<String, u64>,
}

#[derive(Serialize, ToSchema)]
pub struct EventPage {
    pub items: Vec<Event>,
    pub total: u64,
    pub facets: Vec<EventFacet>,
    pub histogram: Vec<EventBucket>,
}

/// One condition of the query: the SQL and its bound values.
struct Cond {
    sql: String,
    binds: Vec<String>,
}

/// The WHERE conditions for a query (all values bound, never interpolated).
fn conditions(query: &str) -> Vec<Cond> {
    let mut out = Vec::new();
    let mut text = Vec::new();
    for raw in query.split_whitespace() {
        let (neg, tok) = raw.strip_prefix('-').map_or((false, raw), |t| (true, t));
        let not = if neg { "NOT " } else { "" };
        let cond = match tok.split_once(':') {
            Some(("source", v)) => Cond { sql: format!("{not}LogAttributes['source_type_name'] = ?"), binds: vec![v.into()] },
            Some(("host", v)) => Cond { sql: format!("{not}ResourceAttributes['host.name'] = ?"), binds: vec![v.into()] },
            Some(("status" | "alert_type", v)) => Cond { sql: format!("{not}SeverityText = ?"), binds: vec![v.into()] },
            Some(("priority", v)) => Cond { sql: format!("{not}LogAttributes['priority'] = ?"), binds: vec![v.into()] },
            Some((k, v)) if !k.is_empty() && !v.is_empty() => Cond { sql: format!("{not}has(splitByChar(',', LogAttributes['tags']), ?)"), binds: vec![format!("{k}:{v}")] },
            _ if neg => Cond { sql: "NOT positionCaseInsensitive(concat(LogAttributes['title'], ' ', Body), ?) > 0".into(), binds: vec![tok.into()] },
            _ => {
                text.push(tok.to_string());
                continue;
            }
        };
        out.push(cond);
    }
    if !text.is_empty() {
        out.push(Cond { sql: "positionCaseInsensitive(concat(LogAttributes['title'], ' ', Body), ?) > 0".into(), binds: vec![text.join(" ")] });
    }
    out
}

/// The time range and query conditions; every query puts the org filter in front of it.
const RANGE: &str = "Timestamp >= fromUnixTimestamp64Milli(?) AND Timestamp <= fromUnixTimestamp64Milli(?)";

/// Search events, newest first, with facet counts over the same filter.
#[utoipa::path(post, path = "/api/v1/events/search", operation_id = "search_events", request_body = EventSearchRequest, responses((status = 200, body = EventPage)), tag = "events")]
pub async fn search(State(st): State<AppState>, OrgId(org): OrgId, Json(req): Json<EventSearchRequest>) -> ApiResult<EventPage> {
    req.range.validate()?;
    let conds = conditions(&req.query);
    let mut where_sql = RANGE.to_string();
    for c in &conds {
        where_sql.push_str(" AND ");
        where_sql.push_str(&c.sql);
    }
    let bind_all = |mut q: clickhouse::query::Query| {
        q = q.bind(req.range.from).bind(req.range.to);
        for c in &conds {
            for b in &c.binds {
                q = q.bind(b.as_str());
            }
        }
        q
    };

    #[derive(Row, Deserialize)]
    struct EventRow {
        id: String,
        ts: i64,
        title: String,
        text: String,
        alert_type: String,
        priority: String,
        source: String,
        host: String,
        aggregation_key: String,
        tags: String,
    }
    let ch = st.ch(&org);
    let rows = bind_all(ch.query(&format!(
        "SELECT toString(cityHash64(Timestamp, ResourceAttributes['host.name'], LogAttributes['title'], Body)) AS id, \
         toInt64(toUnixTimestamp64Milli(Timestamp)) AS ts, LogAttributes['title'] AS title, Body AS text, SeverityText AS alert_type, \
         LogAttributes['priority'] AS priority, LogAttributes['source_type_name'] AS source, ResourceAttributes['host.name'] AS host, \
         LogAttributes['aggregation_key'] AS aggregation_key, LogAttributes['tags'] AS tags \
         FROM otel.otel_events WHERE OrgId = {{oc_org:String}} AND {where_sql} ORDER BY Timestamp DESC LIMIT ? OFFSET ?"
    )))
    .bind(req.limit.min(500) as u64)
    .bind(req.offset as u64)
    .fetch_all::<EventRow>();

    #[derive(Row, Deserialize)]
    struct FacetRow {
        key: String,
        value: String,
        count: u64,
    }
    let facets = bind_all(ch.query(&format!(
        "SELECT key, value, count() AS count FROM ( \
           SELECT arrayJoin([('source', LogAttributes['source_type_name']), ('status', SeverityText), ('priority', LogAttributes['priority']), ('host', ResourceAttributes['host.name'])]) AS kv, \
           kv.1 AS key, kv.2 AS value FROM otel.otel_events WHERE OrgId = {{oc_org:String}} AND {where_sql}) \
         WHERE value != '' GROUP BY key, value ORDER BY key, count DESC LIMIT 25 BY key"
    )))
    .fetch_all::<FacetRow>();

    let total = bind_all(ch.query(&format!("SELECT count() FROM otel.otel_events WHERE OrgId = {{oc_org:String}} AND {where_sql}"))).fetch_one::<u64>();

    // About 60 bars over the range, on a round step.
    let span = (req.range.to - req.range.from).max(60_000);
    let step = [10_000i64, 30_000, 60_000, 300_000, 600_000, 1_800_000, 3_600_000, 10_800_000, 21_600_000, 43_200_000, 86_400_000].into_iter().find(|s| span / s <= 90).unwrap_or(86_400_000);
    #[derive(Row, Deserialize)]
    struct BucketRow {
        t: i64,
        status: String,
        count: u64,
    }
    let buckets = bind_all(ch.query(&format!(
        "SELECT intDiv(toInt64(toUnixTimestamp64Milli(Timestamp)), {step}) * {step} AS t, toString(SeverityText) AS status, count() AS count \
         FROM otel.otel_events WHERE OrgId = {{oc_org:String}} AND {where_sql} GROUP BY t, status ORDER BY t"
    )))
    .fetch_all::<BucketRow>();
    let (rows, facet_rows, total, bucket_rows) = tokio::try_join!(rows, facets, total, buckets)?;
    let mut histogram: Vec<EventBucket> = Vec::new();
    let mut t = req.range.from.div_euclid(step) * step;
    while t <= req.range.to {
        histogram.push(EventBucket { t, counts: Default::default() });
        t += step;
    }
    for b in bucket_rows {
        if let Some(h) = histogram.iter_mut().find(|h| h.t == b.t) {
            h.counts.insert(b.status, b.count);
        }
    }

    let mut facets: Vec<EventFacet> = Vec::new();
    for key in ["source", "status", "priority", "host"] {
        let label = match key {
            "source" => "Source",
            "status" => "Status",
            "priority" => "Priority",
            _ => "Host",
        };
        facets.push(EventFacet { key: key.into(), label: label.into(), group: "CORE".into(), values: vec![] });
    }
    for f in facet_rows {
        if let Some(x) = facets.iter_mut().find(|x| x.key == f.key) {
            x.values.push(EventFacetValue { value: f.value, count: f.count });
        }
    }
    let items = rows
        .into_iter()
        .map(|r| Event {
            id: r.id,
            timestamp: r.ts,
            title: r.title,
            text: r.text,
            alert_type: r.alert_type,
            priority: r.priority,
            source: r.source,
            host: r.host,
            aggregation_key: r.aggregation_key,
            tags: r.tags.split(',').filter(|t| !t.is_empty()).map(str::to_string).collect(),
        })
        .collect();
    Ok(Json(EventPage { items, total, facets, histogram }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_conditions_are_bound() {
        let c = conditions("source:kubernetes -status:info env:prod deploy failed");
        let sql: Vec<&str> = c.iter().map(|c| c.sql.as_str()).collect();
        assert_eq!(sql[0], "LogAttributes['source_type_name'] = ?");
        assert_eq!(sql[1], "NOT SeverityText = ?");
        assert_eq!(sql[2], "has(splitByChar(',', LogAttributes['tags']), ?)");
        assert_eq!(c[2].binds, vec!["env:prod"]);
        assert_eq!(c[3].binds, vec!["deploy failed"]);
        // Values never reach the SQL text.
        assert!(conditions("host:x'); DROP TABLE t; --").iter().all(|c| !c.sql.contains("DROP")));
    }
}
