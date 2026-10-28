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
    /// Tag keys added as facets ("+ Add"), after env and service.
    #[serde(default)]
    pub extra: Vec<String>,
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
    /// `source`, `status`, `priority`, `host`, or a tag key (`env`, `service`, added ones).
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

const CORE_FACETS: [(&str, &str); 4] = [("source", "Source"), ("status", "Status"), ("priority", "Priority"), ("host", "Host")];
/// Tag facets every search shows.
const DEFAULT_TAG_FACETS: [&str; 2] = ["env", "service"];
/// Added tag facets per search.
const MAX_EXTRA_FACETS: usize = 10;

fn tag_key_ok(k: &str) -> bool {
    !k.is_empty() && k.len() <= 100 && k.chars().all(|c| c.is_ascii_alphanumeric() || "_.-/".contains(c))
}

/// The facets of a search, as (key, label, group): the core ones, env and service, then the added tags.
fn facet_list(extra: &[String]) -> Vec<(String, String, String)> {
    let mut out: Vec<(String, String, String)> = CORE_FACETS.iter().map(|(k, l)| (k.to_string(), l.to_string(), "CORE".to_string())).collect();
    let mut tags: Vec<&str> = DEFAULT_TAG_FACETS.to_vec();
    for k in extra.iter().map(String::as_str).filter(|k| tag_key_ok(k)).take(MAX_EXTRA_FACETS) {
        if !tags.contains(&k) && !CORE_FACETS.iter().any(|(c, _)| *c == k) {
            tags.push(k);
        }
    }
    for k in tags {
        let label = match k {
            "env" => "Env".to_string(),
            "service" => "Service".to_string(),
            other => other.to_string(),
        };
        out.push((k.to_string(), label, "TAGS".to_string()));
    }
    out
}

/// A tag's key and value from `key:value` in SQL (`t` is one tag).
const TAG_KEY_SQL: &str = "substring(t, 1, position(t, ':') - 1)";
const TAG_VALUE_SQL: &str = "substring(t, position(t, ':') + 1)";

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

    let list = facet_list(&req.extra);
    let tag_keys: Vec<String> = list.iter().filter(|(_, _, g)| g == "TAGS").map(|(k, ..)| k.clone()).collect();
    let tag_facets = bind_all(ch.query(&format!(
        "SELECT key, value, count() AS count FROM ( \
           SELECT arrayJoin(splitByChar(',', LogAttributes['tags'])) AS t, {TAG_KEY_SQL} AS key, {TAG_VALUE_SQL} AS value \
           FROM otel.otel_events WHERE OrgId = {{oc_org:String}} AND {where_sql}) \
         WHERE position(t, ':') > 0 AND value != '' AND has(?, key) GROUP BY key, value ORDER BY key, count DESC LIMIT 25 BY key"
    )))
    .bind(&tag_keys)
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
    let (rows, facet_rows, tag_rows, total, bucket_rows) = tokio::try_join!(rows, facets, tag_facets, total, buckets)?;
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

    let mut facets: Vec<EventFacet> = list.into_iter().map(|(key, label, group)| EventFacet { key, label, group, values: vec![] }).collect();
    for f in facet_rows.into_iter().chain(tag_rows) {
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

// ================================================================ triage inbox

/// Warning and error events of one object (the Agent's aggregation key), or of one source and title.
#[derive(Row, Deserialize, Clone)]
pub(crate) struct WorkItemRow {
    pub key: String,
    pub title: String,
    pub source: String,
    pub statuses: Vec<String>,
    pub count: u64,
    pub first: i64,
    pub last: i64,
    /// The newest event's text.
    pub text: String,
    pub tags: String,
}

/// Correlated events: one affected object's warnings and errors.
#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct WorkItem {
    pub id: String,
    pub title: String,
    pub source: String,
    /// error | warning
    pub status: String,
    /// The newest event's reason (`BackOff`, `OOMKilling`), when it names one.
    pub reason: String,
    /// The newest event's text.
    pub summary: String,
    pub count: u64,
    pub first_seen: i64,
    pub last_seen: i64,
    /// open (events in the last 30 minutes of the range) | closed
    pub state: String,
    /// Where it happened (cluster, namespace, service, env).
    pub tags: Vec<String>,
}

#[derive(Serialize, ToSchema)]
pub struct WorkItemsPage {
    pub items: Vec<WorkItem>,
    pub open: usize,
    pub closed: usize,
}

/// A work item stays open while its object keeps reporting.
const OPEN_FOR_MS: i64 = 30 * 60_000;
const WORK_ITEM_TAGS: [&str; 4] = ["kube_cluster_name", "kube_namespace", "service", "env"];

/// An event's first line of text without its Markdown (`%%%` wrappers, `**bold**`).
fn event_plain(text: &str) -> String {
    text.lines().map(|l| l.trim().trim_matches('%').trim()).find(|l| !l.is_empty()).unwrap_or("").replace("**", "")
}

/// The reason a Kubernetes event gives (`3100 BackOff: Back-off …` → `BackOff`), else empty.
fn event_reason(text: &str) -> String {
    let plain = event_plain(text);
    let first = plain.as_str();
    let rest = first.split_once(' ').filter(|(n, _)| n.chars().all(|c| c.is_ascii_digit())).map_or(first, |(_, r)| r);
    match rest.split_once(':') {
        Some((word, _)) if !word.is_empty() && word.chars().all(|c| c.is_ascii_alphanumeric()) => word.to_string(),
        _ => String::new(),
    }
}

fn worst_status(statuses: &[String]) -> String {
    if statuses.iter().any(|s| s == "error") { "error" } else { "warning" }.to_string()
}

fn work_item(r: WorkItemRow, range_end: i64) -> WorkItem {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in r.key.bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    let tags = r.tags.split(',').filter(|t| t.split_once(':').is_some_and(|(k, _)| WORK_ITEM_TAGS.contains(&k))).map(str::to_string).collect();
    WorkItem {
        id: format!("{h:016x}"),
        status: worst_status(&r.statuses),
        reason: event_reason(&r.text),
        summary: event_plain(&r.text).chars().take(300).collect(),
        state: if range_end - r.last <= OPEN_FOR_MS { "open" } else { "closed" }.into(),
        tags,
        title: r.title,
        source: r.source,
        count: r.count,
        first_seen: r.first,
        last_seen: r.last,
    }
}

#[derive(Deserialize, ToSchema)]
pub struct WorkItemsRequest {
    pub range: TimeRange,
    #[serde(default)]
    pub query: String,
}

/// Work items most recently active first.
const WORK_ITEMS: u64 = 500;

/// Triage Inbox: the range's warning and error events correlated into work items, one per affected
/// object (the Agent's aggregation key) or, without one, per source and title.
#[utoipa::path(post, path = "/api/v1/events/work-items", operation_id = "event_work_items", request_body = WorkItemsRequest, responses((status = 200, body = WorkItemsPage)), tag = "events")]
pub async fn work_items(State(st): State<AppState>, OrgId(org): OrgId, Json(req): Json<WorkItemsRequest>) -> ApiResult<WorkItemsPage> {
    req.range.validate()?;
    let conds = conditions(&req.query);
    let mut where_sql = RANGE.to_string();
    for c in &conds {
        where_sql.push_str(" AND ");
        where_sql.push_str(&c.sql);
    }
    let mut q = st
        .ch(&org)
        .query(&format!(
            "SELECT if(LogAttributes['aggregation_key'] != '', LogAttributes['aggregation_key'], concat(LogAttributes['source_type_name'], ':', LogAttributes['title'])) AS key, \
             argMax(LogAttributes['title'], Timestamp) AS title, any(LogAttributes['source_type_name']) AS source, groupUniqArray(toString(SeverityText)) AS statuses, \
             count() AS count, toInt64(toUnixTimestamp64Milli(min(Timestamp))) AS first, toInt64(toUnixTimestamp64Milli(max(Timestamp))) AS last, \
             argMax(Body, Timestamp) AS text, argMax(LogAttributes['tags'], Timestamp) AS tags \
             FROM otel.otel_events WHERE OrgId = {{oc_org:String}} AND {where_sql} AND SeverityText IN ('error', 'warning') \
             GROUP BY key ORDER BY last DESC LIMIT {WORK_ITEMS}"
        ))
        .bind(req.range.from)
        .bind(req.range.to);
    for c in &conds {
        for b in &c.binds {
            q = q.bind(b.as_str());
        }
    }
    let items: Vec<WorkItem> = q.fetch_all::<WorkItemRow>().await?.into_iter().map(|r| work_item(r, req.range.to)).collect();
    let open = items.iter().filter(|i| i.state == "open").count();
    Ok(Json(WorkItemsPage { closed: items.len() - open, open, items }))
}

#[derive(Deserialize, ToSchema)]
pub struct EventFieldsRequest {
    pub range: TimeRange,
    #[serde(default)]
    pub query: String,
}

#[derive(Serialize, ToSchema, Row, Deserialize)]
pub struct EventField {
    pub key: String,
    /// Events with the tag.
    pub count: u64,
}

/// Tag keys of the matching events, most common first: the facets "+ Add" offers.
#[utoipa::path(post, path = "/api/v1/events/fields", operation_id = "event_fields", request_body = EventFieldsRequest, responses((status = 200, body = Vec<EventField>)), tag = "events")]
pub async fn fields(State(st): State<AppState>, OrgId(org): OrgId, Json(req): Json<EventFieldsRequest>) -> ApiResult<Vec<EventField>> {
    req.range.validate()?;
    let conds = conditions(&req.query);
    let mut where_sql = RANGE.to_string();
    for c in &conds {
        where_sql.push_str(" AND ");
        where_sql.push_str(&c.sql);
    }
    let mut q = st
        .ch(&org)
        .query(&format!(
            "SELECT key, count() AS count FROM ( \
               SELECT arrayJoin(arrayDistinct(splitByChar(',', LogAttributes['tags']))) AS t, {TAG_KEY_SQL} AS key \
               FROM otel.otel_events WHERE OrgId = {{oc_org:String}} AND {where_sql}) \
             WHERE position(t, ':') > 0 AND key != '' GROUP BY key ORDER BY count DESC, key LIMIT 200"
        ))
        .bind(req.range.from)
        .bind(req.range.to);
    for c in &conds {
        for b in &c.binds {
            q = q.bind(b.as_str());
        }
    }
    Ok(Json(q.fetch_all::<EventField>().await?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_work_item_names_its_reason_worst_status_and_state() {
        assert_eq!(event_reason("3100 BackOff: Back-off restarting failed container alloy in pod alloy-xxf8p"), "BackOff");
        assert_eq!(event_reason("283 OOMKilling: Memory cgroup out of memory"), "OOMKilling");
        assert_eq!(event_reason("Container 0345 ran out of memory"), "");
        // The Agent's events are Markdown: %%% wrappers and **bold** reasons.
        assert_eq!(event_reason("%%% \n546439 **BackOff**: Back-off restarting failed container\n \n _Events emitted by the kubelet_"), "BackOff");
        assert_eq!(event_plain("%%% \n546439 **BackOff**: Back-off restarting\n \n _Events emitted_"), "546439 BackOff: Back-off restarting");
        assert_eq!(worst_status(&["warning".into(), "error".into(), "info".into()]), "error");
        assert_eq!(worst_status(&["warning".into()]), "warning");
        let row = WorkItemRow { key: "kubernetes_apiserver:abc".into(), title: "Events from the Pod shop/checkout-7d9".into(), source: "kubernetes".into(), statuses: vec!["warning".into()], count: 12, first: 1_000, last: 9_000_000, text: "12 BackOff: Back-off restarting".into(), tags: "kube_namespace:shop,kube_cluster_name:prod,pod_name:checkout-7d9".into() };
        let open = work_item(row.clone(), 10_000_000);
        assert_eq!((open.state.as_str(), open.reason.as_str(), open.count), ("open", "BackOff", 12));
        assert_eq!(open.tags, vec!["kube_namespace:shop".to_string(), "kube_cluster_name:prod".to_string()]);
        assert_eq!(open.id, work_item(row.clone(), 10_000_000).id);
        // Quiet for more than 30 minutes before the range's end: closed.
        assert_eq!(work_item(row, 9_000_000 + 31 * 60_000).state, "closed");
    }

    #[test]
    fn facets_are_the_core_ones_then_env_service_and_added_tags() {
        let keys = |extra: &[&str]| facet_list(&extra.iter().map(|s| s.to_string()).collect::<Vec<_>>()).into_iter().map(|(k, _, g)| format!("{g}:{k}")).collect::<Vec<_>>();
        assert_eq!(keys(&[]), vec!["CORE:source", "CORE:status", "CORE:priority", "CORE:host", "TAGS:env", "TAGS:service"]);
        // Added tags once each, after the defaults; core keys and invalid names aren't tags.
        assert_eq!(keys(&["kube_namespace", "env", "host", "bad key", "kube_namespace"])[6..], ["TAGS:kube_namespace".to_string()]);
    }

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
