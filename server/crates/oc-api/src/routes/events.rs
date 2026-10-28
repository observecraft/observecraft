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

use super::event_correlation::{correlate, Correlator, MaintenanceWindow, RawEvent};
use super::event_settings;
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
    /// The correlator that grouped its events (Event Management → Settings); none for the default
    /// grouping per affected object.
    pub correlator: Option<String>,
    /// The maintenance window all its events fall in: the work item is suppressed.
    pub maintenance: Option<String>,
    /// Work item attribute values, by attribute id.
    pub attributes: std::collections::BTreeMap<String, String>,
}

#[derive(Serialize, ToSchema)]
pub struct WorkItemsPage {
    pub items: Vec<WorkItem>,
    /// Open and closed count the work items that aren't suppressed.
    pub open: usize,
    pub closed: usize,
    /// Work items in a maintenance window.
    pub suppressed: usize,
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
        correlator: None,
        maintenance: None,
        attributes: Default::default(),
    }
}

/// Raw events correlated with the org's correlators and maintenance windows read at most this many.
const CORRELATED_EVENTS: u64 = 20_000;

/// Work item rows from correlated events (see `event_correlation`): the row, its correlator's name
/// and its maintenance window.
fn correlated_rows(events: &[RawEvent], correlators: &[Correlator], windows: &[MaintenanceWindow]) -> Vec<(WorkItemRow, Option<String>, Option<String>)> {
    correlate(events, correlators, windows)
        .into_iter()
        .filter_map(|c| {
            let first = &events[*c.events.first()?];
            let last = &events[*c.events.last()?];
            let mut statuses: Vec<String> = c.events.iter().map(|&i| events[i].status.clone()).collect();
            statuses.sort();
            statuses.dedup();
            let correlator = c.correlator.map(|i| &correlators[i]);
            let title = match correlator {
                Some(k) if c.values.iter().any(|v| !v.is_empty()) => format!("{}: {}", k.name, c.values.iter().filter(|v| !v.is_empty()).cloned().collect::<Vec<_>>().join(", ")),
                Some(k) => k.name.clone(),
                None => last.title.clone(),
            };
            let row = WorkItemRow { key: c.key, title, source: last.source.clone(), statuses, count: c.events.len() as u64, first: first.ts, last: last.ts, text: last.text.clone(), tags: last.tags.join(",") };
            Some((row, correlator.map(|k| k.name.clone()), c.maintenance))
        })
        .collect()
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
    let (correlators, windows) = tokio::try_join!(event_settings::active_correlators(&st, &org), event_settings::windows_between(&st, &org, req.range.from, req.range.to))?;
    let mut items: Vec<WorkItem> = if correlators.is_empty() && windows.is_empty() {
        q.fetch_all::<WorkItemRow>().await?.into_iter().map(|r| work_item(r, req.range.to)).collect()
    } else {
        let events = raw_events(&st, &org, &req, &conds).await?;
        let mut items: Vec<WorkItem> = correlated_rows(&events, &correlators, &windows)
            .into_iter()
            .map(|(row, correlator, maintenance)| WorkItem { correlator, maintenance, ..work_item(row, req.range.to) })
            .collect();
        items.sort_by(|a, b| b.last_seen.cmp(&a.last_seen));
        items.truncate(WORK_ITEMS as usize);
        items
    };
    let ids: Vec<String> = items.iter().map(|i| i.id.clone()).collect();
    let mut values = event_settings::values_of(&st, &org, &ids).await?;
    for i in &mut items {
        i.attributes = values.remove(&i.id).unwrap_or_default();
    }
    let suppressed = items.iter().filter(|i| i.maintenance.is_some()).count();
    let open = items.iter().filter(|i| i.maintenance.is_none() && i.state == "open").count();
    Ok(Json(WorkItemsPage { closed: items.len() - suppressed - open, open, suppressed, items }))
}

/// The range's warning and error events matching the inbox query, newest first (at most [`CORRELATED_EVENTS`]).
async fn raw_events(st: &AppState, org: &str, req: &WorkItemsRequest, conds: &[Cond]) -> Result<Vec<RawEvent>, crate::error::ApiError> {
    let mut where_sql = RANGE.to_string();
    for c in conds {
        where_sql.push_str(" AND ");
        where_sql.push_str(&c.sql);
    }
    #[derive(Row, Deserialize)]
    struct RawRow {
        ts: i64,
        title: String,
        text: String,
        status: String,
        priority: String,
        source: String,
        host: String,
        aggregation_key: String,
        tags: String,
    }
    let mut q = st
        .ch(org)
        .query(&format!(
            "SELECT toInt64(toUnixTimestamp64Milli(Timestamp)) AS ts, LogAttributes['title'] AS title, Body AS text, toString(SeverityText) AS status, LogAttributes['priority'] AS priority, \
             LogAttributes['source_type_name'] AS source, ResourceAttributes['host.name'] AS host, LogAttributes['aggregation_key'] AS aggregation_key, LogAttributes['tags'] AS tags \
             FROM otel.otel_events WHERE OrgId = {{oc_org:String}} AND {where_sql} AND SeverityText IN ('error', 'warning') ORDER BY Timestamp DESC LIMIT {CORRELATED_EVENTS}"
        ))
        .bind(req.range.from)
        .bind(req.range.to);
    for c in conds {
        for b in &c.binds {
            q = q.bind(b.as_str());
        }
    }
    Ok(q
        .fetch_all::<RawRow>()
        .await?
        .into_iter()
        .map(|r| RawEvent {
            ts: r.ts,
            title: r.title,
            text: r.text,
            status: r.status,
            priority: r.priority,
            source: r.source,
            host: r.host,
            aggregation_key: r.aggregation_key,
            tags: r.tags.split(',').filter(|t| !t.is_empty()).map(str::to_string).collect(),
        })
        .collect())
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
    fn correlated_work_items_keep_the_default_ids_and_name_their_correlator() {
        let ev = |ts: i64, key: &str, title: &str, ns: &str| RawEvent { ts, title: title.into(), text: "12 BackOff: Back-off restarting".into(), status: "warning".into(), source: "kubernetes".into(), aggregation_key: key.into(), tags: vec![format!("kube_namespace:{ns}")], ..Default::default() };
        let events = vec![ev(1_000, "pod-a", "Pod a", "shop"), ev(2_000, "pod-b", "Pod b", "billing")];
        // The default grouping gives the work items the ids the SQL grouping gives them.
        let rows = correlated_rows(&events, &[], &[]);
        assert_eq!(rows[0].0.key, "pod-a");
        let row = WorkItemRow { key: "pod-a".into(), title: "Pod a".into(), source: "kubernetes".into(), statuses: vec!["warning".into()], count: 1, first: 1_000, last: 1_000, text: String::new(), tags: String::new() };
        assert_eq!(work_item(rows[0].0.clone(), 5_000).id, work_item(row, 5_000).id);

        let shop = Correlator { id: "c1".into(), name: "Shop".into(), query: "kube_namespace:shop".into(), group_by: vec!["kube_namespace".into()], window_ms: 600_000 };
        let rows = correlated_rows(&events, &[shop], &[]);
        assert_eq!((rows[0].0.title.as_str(), rows[0].1.as_deref()), ("Shop: shop", Some("Shop")));
        assert_eq!((rows[1].0.title.as_str(), rows[1].1.as_deref()), ("Pod b", None));
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
