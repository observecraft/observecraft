//! Dashboard-wide execution: tenant-scoped shared scans, isolated query errors and incremental SSE.
use std::collections::{BTreeMap, HashMap, HashSet};
use std::convert::Infallible;
use std::sync::{Arc, OnceLock, Weak};
use std::time::Duration;

use axum::extract::State;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::Json;
use futures::{stream, Stream, StreamExt};
use oc_query::{metric_field, metric_filter, MetricQuery, Param};
use serde::{Deserialize, Serialize};
use tokio::sync::{watch, Mutex};
use utoipa::ToSchema;

use super::super::logs::TimeRange;
use super::{
    bind, group_array, group_label, inner_query_at_step, parse, resolve_alias, round, step_ms,
    Kind, Row, SPAN_METRICS,
};
use crate::error::{ApiError, ApiResult};
use crate::state::{AppState, OrgId};

#[derive(Clone, Deserialize, Serialize, ToSchema)]
pub struct DashboardMetricQuery {
    pub id: String,
    pub query: String,
    pub points: Option<u32>,
}

#[derive(Clone, Deserialize, Serialize, ToSchema)]
pub struct DashboardQueryRequest {
    pub queries: Vec<DashboardMetricQuery>,
    pub range: TimeRange,
}

#[derive(Clone, Serialize, ToSchema)]
pub struct DashboardMetricSeries {
    pub name: String,
    pub points: Vec<(i64, f64)>,
    pub tags: BTreeMap<String, String>,
}

#[derive(Clone, Serialize, ToSchema)]
pub struct DashboardQueryResult {
    pub id: String,
    pub series: Vec<DashboardMetricSeries>,
    /// gauge | counter | delta | updown | histogram | exponential_histogram | summary | span | unknown
    pub kind: String,
    pub unit: Option<String>,
    pub step_ms: i64,
    /// Replace existing points at or after this aligned timestamp (also removes vanished series).
    pub replace_from: i64,
    pub error: Option<String>,
}

#[derive(Clone, Serialize, ToSchema)]
pub struct DashboardQueryFrame {
    pub results: Vec<DashboardQueryResult>,
    pub range: TimeRange,
    /// Actual telemetry SELECTs, excluding metadata discovery; independent of panel count.
    pub data_scans: usize,
    pub unique_queries: usize,
    pub error: Option<String>,
}

#[derive(Clone)]
struct Prepared {
    query: MetricQuery,
    text: String,
    ids: Vec<String>,
    kind: String,
    unit: Option<String>,
    scalar_kind: Option<Kind>,
    weighted: bool,
    scale: f64,
    step: i64,
    error: Option<String>,
}

struct Plan {
    items: Vec<Prepared>,
    groups: Vec<Vec<usize>>,
    fallback: Vec<usize>,
    duration: i64,
}

#[derive(clickhouse::Row, Deserialize)]
struct Meta {
    name: String,
    kind: String,
    unit: String,
}

fn validate(req: &DashboardQueryRequest) -> Result<(), ApiError> {
    req.range.validate()?;
    if req.queries.is_empty() || req.queries.len() > 256 {
        return Err(ApiError::BadRequest(
            "dashboard requires 1 to 256 queries".into(),
        ));
    }
    let mut ids = HashSet::new();
    for q in &req.queries {
        if q.id.is_empty() || q.id.len() > 128 || q.query.len() > 4096 || !ids.insert(&q.id) {
            return Err(ApiError::BadRequest(
                "query IDs must be unique and bounded".into(),
            ));
        }
    }
    Ok(())
}

impl Plan {
    async fn prepare(
        st: &AppState,
        org: &str,
        req: &DashboardQueryRequest,
    ) -> Result<Self, ApiError> {
        let mut items: Vec<Prepared> = Vec::new();
        let mut seen = HashMap::new();
        for item in &req.queries {
            let key = (
                item.query.clone(),
                item.points.unwrap_or(120).clamp(10, 1000),
            );
            if let Some(&i) = seen.get(&key) {
                let i: usize = i;
                items[i].ids.push(item.id.clone());
                continue;
            }
            let weighted = item.query.ends_with(".weighted()");
            let parsed = parse(
                item.query
                    .strip_suffix(".weighted()")
                    .unwrap_or(&item.query),
            );
            let (query, error) = match parsed {
                Ok(q) => (q, None),
                Err(e) => (parse("avg:invalid{*}")?, Some(e.to_string())),
            };
            let (query, scale) = resolve_alias(&query);
            seen.insert(key.clone(), items.len());
            items.push(Prepared {
                query,
                text: item.query.clone(),
                ids: vec![item.id.clone()],
                kind: "unknown".into(),
                unit: None,
                scalar_kind: None,
                weighted,
                scale,
                step: step_ms(req.range, key.1, 10_000),
                error,
            });
        }
        let names: Vec<_> = items
            .iter()
            .filter(|i| i.error.is_none())
            .map(|i| i.query.metric.clone())
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();
        let mut metadata = HashMap::new();
        let mut units = HashMap::new();
        if !names.is_empty() {
            let marks = vec!["?"; names.len()].join(",");
            let mut params = Vec::new();
            let mut parts = Vec::new();
            for (table, kind) in [
                ("otel.otel_metrics_gauge", "'gauge'"),
                ("otel.otel_metrics_sum", "multiIf(any(AggregationTemporality) = 1, 'delta', any(IsMonotonic), 'counter', 'updown')"),
                ("otel.otel_metrics_histogram", "'histogram'"),
                ("otel.otel_metrics_exponential_histogram", "'exponential_histogram'"),
                ("otel.otel_metrics_summary", "'summary'"),
            ] {
                // Statistic queries use a base metric name. Include both, without guessing its table.
                let bases: Vec<_> = names.iter().map(|n| super::split_stat(n).map(|(b, _)| b.to_string()).unwrap_or(n.clone())).collect();
                parts.push(format!("SELECT MetricName AS name, {kind} AS kind, any(MetricUnit) AS unit FROM {table} WHERE OrgId = {{oc_org:String}} AND (MetricName IN ({marks}) OR MetricName IN ({marks})) GROUP BY MetricName"));
                params.extend(names.iter().chain(bases.iter()).cloned().map(Param::Str));
            }
            for meta in bind(st.ch(org).query(&parts.join(" UNION ALL ")), &params)
                .fetch_all::<Meta>()
                .await?
            {
                units.insert(meta.name.clone(), meta.unit);
                metadata.entry(meta.name).or_insert(meta.kind);
            }
        }
        let mut groups: Vec<Vec<usize>> = Vec::new();
        let mut fallback = Vec::new();
        for (index, p) in items.iter_mut().enumerate() {
            if p.error.is_some() {
                continue;
            }
            let points = req
                .queries
                .iter()
                .find(|q| q.id == p.ids[0])
                .and_then(|q| q.points)
                .unwrap_or(120)
                .clamp(10, 1000);
            p.kind = if SPAN_METRICS.iter().any(|m| m.0 == p.query.metric) {
                "span".into()
            } else if let Some(kind) = metadata.get(&p.query.metric) {
                kind.clone()
            } else if let Some((base, _)) = super::split_stat(&p.query.metric) {
                metadata
                    .get(base)
                    .cloned()
                    .unwrap_or_else(|| "unknown".into())
            } else {
                "unknown".into()
            };
            let original = p
                .text
                .split_once(':')
                .and_then(|(_, r)| r.split_once('{'))
                .map(|(name, _)| name);
            p.unit = super::ALIASES
                .iter()
                .find(|a| Some(a.0) == original)
                .map(|a| a.4.to_string())
                .or_else(|| units.get(&p.query.metric).cloned())
                .filter(|u| !u.is_empty());
            p.scalar_kind = match p.kind.as_str() {
                "gauge" => Some(Kind::Gauge),
                "counter" => Some(Kind::Counter),
                "delta" => Some(Kind::Delta),
                "updown" => Some(Kind::UpDown),
                _ => None,
            };
            if matches!(p.scalar_kind, Some(Kind::Counter | Kind::Delta)) {
                p.unit = p.unit.take().map(|u| {
                    if u.ends_with("/s") {
                        u
                    } else {
                        format!("{u}/s")
                    }
                });
            }
            p.step = step_ms(
                req.range,
                points,
                if matches!(
                    p.kind.as_str(),
                    "counter" | "histogram" | "exponential_histogram" | "summary"
                ) {
                    30_000
                } else {
                    10_000
                },
            );
            if p.kind == "unknown" {
                p.error = Some(format!("Unknown metric: {}", p.query.metric));
            } else if p.weighted && (p.kind != "gauge" || p.query.agg.sql() != "sum") {
                p.error = Some("weighted() requires a sum aggregation on a gauge".into());
            } else if p.scalar_kind.is_none() {
                fallback.push(index);
            }
        }
        // Group after classification, avoiding mutable borrows while comparing prepared items.
        for (i, p) in items
            .iter()
            .enumerate()
            .filter(|(_, p)| p.scalar_kind.is_some() && p.error.is_none())
        {
            if let Some(g) = groups
                .iter_mut()
                .find(|g| items[g[0]].scalar_kind == p.scalar_kind && items[g[0]].step == p.step)
            {
                g.push(i);
            } else {
                groups.push(vec![i]);
            }
        }
        Ok(Self {
            items,
            groups,
            fallback,
            duration: req.range.to - req.range.from,
        })
    }

    async fn execute(
        &self,
        st: &AppState,
        org: &str,
        range: TimeRange,
        previous_to: Option<i64>,
    ) -> DashboardQueryFrame {
        let mut results: Vec<_> = self
            .items
            .iter()
            .map(|p| DashboardQueryResult {
                id: String::new(),
                series: Vec::new(),
                kind: p.kind.clone(),
                unit: p.unit.clone(),
                step_ms: p.step,
                replace_from: query_range(range, previous_to, p.step).from,
                error: p.error.clone(),
            })
            .collect();
        let jobs: Vec<Vec<usize>> = self
            .groups
            .iter()
            .cloned()
            .chain(self.fallback.iter().map(|i| vec![*i]))
            .collect();
        let scans = jobs.len();
        let mut work = stream::iter(jobs.into_iter().map(|indices| async move {
            let step = self.items[indices[0]].step;
            let subrange = query_range(range, previous_to, step);
            let data = if self.items[indices[0]].scalar_kind.is_some() {
                let (sql, params) = shared_sql(&self.items, &indices, subrange);
                bind(st.ch(org).query(&sql), &params).fetch_all::<BatchRow>().await.map_err(ApiError::from)
            } else {
                let p = &self.items[indices[0]];
                match inner_query_at_step(st, org, &p.query, subrange, 120, Some(step)).await {
                    Ok((inner, params, _)) => {
                        let sql = format!("SELECT g, toInt64(toUnixTimestamp(t)) * 1000 AS ts, toFloat64({}(v)) AS value FROM ({inner}) WHERE isFinite(v) GROUP BY g, t ORDER BY g, t", p.query.agg.sql());
                        bind(st.ch(org).query(&sql), &params).fetch_all::<Row>().await.map(|rows| rows.into_iter().map(|r| BatchRow { qi: indices[0] as u32, g: r.g, ts: r.ts, value: r.value }).collect()).map_err(ApiError::from)
                    }
                    Err(e) => Err(e),
                }
            };
            (indices, data)
        })).buffer_unordered(4);
        while let Some((indices, data)) = work.next().await {
            match data {
                Ok(rows) => {
                    for r in rows {
                        let p = &self.items[r.qi as usize];
                        let tags = p
                            .query
                            .by
                            .iter()
                            .cloned()
                            .zip(
                                r.g.into_iter()
                                    .map(|v| if v.is_empty() { "N/A".into() } else { v }),
                            )
                            .collect::<BTreeMap<_, _>>();
                        let name = if p.query.by.is_empty() {
                            p.text.clone()
                        } else {
                            group_label(&tags)
                        };
                        let target = &mut results[r.qi as usize].series;
                        if let Some(s) = target.last_mut().filter(|s| s.name == name) {
                            s.points.push((r.ts, round(r.value)));
                        } else {
                            target.push(DashboardMetricSeries {
                                name,
                                tags,
                                points: vec![(r.ts, round(r.value))],
                            });
                        }
                    }
                }
                Err(e) => {
                    tracing::warn!(error = %e, "dashboard scan failed");
                    let message = match e {
                        ApiError::BadRequest(m) => m,
                        _ => "Query failed".into(),
                    };
                    for i in indices {
                        results[i].error = Some(message.clone());
                    }
                }
            }
        }
        let results = results
            .into_iter()
            .enumerate()
            .flat_map(|(i, r)| {
                self.items[i]
                    .ids
                    .iter()
                    .map(move |id| DashboardQueryResult {
                        id: id.clone(),
                        ..r.clone()
                    })
            })
            .collect();
        DashboardQueryFrame {
            results,
            range,
            data_scans: scans,
            unique_queries: self.items.len(),
            error: None,
        }
    }
}

fn query_range(range: TimeRange, previous_to: Option<i64>, step: i64) -> TimeRange {
    TimeRange {
        from: previous_to
            .map(|to| ((to - 2 * step).div_euclid(step) * step).max(range.from))
            .unwrap_or_else(|| range.from.div_euclid(step) * step),
        to: range.to,
    }
}

#[derive(clickhouse::Row, Deserialize)]
struct BatchRow {
    qi: u32,
    g: Vec<String>,
    ts: i64,
    value: f64,
}

fn shared_sql(items: &[Prepared], indices: &[usize], range: TimeRange) -> (String, Vec<Param>) {
    let first = &items[indices[0]];
    let kind = first
        .scalar_kind
        .expect("only scalar kinds form shared scans");
    let mut params = Vec::new();
    let mut grouping = Vec::new();
    let mut eligible = Vec::new();
    let mut scaling = Vec::new();
    let mut aggregates = Vec::new();
    for &i in indices {
        let p = &items[i];
        let mut fields = Vec::new();
        for k in &p.query.by {
            let (field, bound) = metric_field(k);
            fields.push(field);
            params.extend(bound);
        }
        grouping.push(format!("qi = {i}, {}", group_array(&fields)));
        scaling.push(format!("qi = {i}, {:e}", p.scale));
        // Infer a regular submission cadence from each series with multiple samples.
        // Count occupied submission slots over the observed bucket, not exact shared
        // timestamps: independent collectors need not have synchronized clocks.
        // With no measurable cadence, no time weighting is inferred.
        let aggregate = if p.weighted {
            "sum(v * samples) / if(countIf(samples > 1) > 0, greatest(round((max(last_seen) - min(first_seen)) / greatest(medianExactIf((last_seen - first_seen) / greatest(samples - 1, 1), samples > 1), 1)) + 1, 1), 1)".to_string()
        } else {
            format!("{}(v)", p.query.agg.sql())
        };
        aggregates.push(format!("qi = {i}, {aggregate}"));
    }
    for &i in indices {
        let p = &items[i];
        let filter = metric_filter(&p.query.filters, metric_field);
        params.push(Param::Str(p.query.metric.clone()));
        params.extend(filter.params);
        eligible.push(format!(
            "if(MetricName = ? AND ({}), toInt32({i}), toInt32(-1))",
            filter.clause
        ));
    }
    let metrics: Vec<_> = indices
        .iter()
        .map(|i| items[*i].query.metric.clone())
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();
    params.extend(metrics.iter().cloned().map(Param::Str));
    params.extend([Param::Int(range.from), Param::Int(range.to)]);
    let step_s = first.step / 1000;
    let value = match kind {
        Kind::Gauge | Kind::UpDown => "avg(Value)".into(),
        Kind::Counter => "greatest(max(Value) - min(Value), 0) * 1000 / greatest(dateDiff('millisecond', min(TimeUnix), max(TimeUnix)), 1)".to_string(),
        Kind::Delta => format!("sum(Value) / {step_s}"),
    };
    let sql = format!(
        "SELECT toUInt32(qi) AS qi, g, toInt64(toUnixTimestamp(t)) * 1000 AS ts, toFloat64(multiIf({aggregates}, avg(v))) AS value FROM (\
         SELECT qi, multiIf({grouping}, CAST([], 'Array(String)')) AS g, toStartOfInterval(TimeUnix, INTERVAL {step_s} SECOND) AS t, \
         cityHash64(toString(mapSort(ResourceAttributes)), toString(mapSort(Attributes))) AS sid, uniqExact(TimeUnix) AS samples, min(toUnixTimestamp64Milli(TimeUnix)) AS first_seen, max(toUnixTimestamp64Milli(TimeUnix)) AS last_seen, ({value}) * multiIf({scaling}, 1.0) AS v \
         FROM {table} ARRAY JOIN arrayFilter(x -> x >= 0, [{eligible}]) AS qi \
         WHERE OrgId = {{oc_org:String}} AND MetricName IN ({marks}) AND TimeUnix >= fromUnixTimestamp64Milli(?) AND TimeUnix <= fromUnixTimestamp64Milli(?) \
         GROUP BY qi, g, t, sid{having}) WHERE isFinite(v) GROUP BY qi, g, t ORDER BY qi, g, t",
        aggregates=aggregates.join(","), grouping=grouping.join(","), scaling=scaling.join(","), eligible=eligible.join(","),
        table=kind.table(), marks=vec!["?"; metrics.len()].join(","), having=if kind == Kind::Counter { " HAVING count() > 1" } else { "" }
    );
    (sql, params)
}

/// Execute a dashboard in one request, preserving a separate result/error for every query ID.
#[utoipa::path(post, path = "/api/v1/query/dashboard", request_body = DashboardQueryRequest,
    responses((status = 200, body = DashboardQueryFrame)), tag = "metrics")]
pub async fn dashboard_query(
    State(st): State<AppState>,
    OrgId(org): OrgId,
    Json(req): Json<DashboardQueryRequest>,
) -> ApiResult<DashboardQueryFrame> {
    validate(&req)?;
    let plan = Plan::prepare(&st, &org, &req).await?;
    Ok(Json(plan.execute(&st, &org, req.range, None).await))
}

struct Published {
    snapshot: DashboardQueryFrame,
    delta: DashboardQueryFrame,
}
struct Feed {
    tx: watch::Sender<Option<Arc<Published>>>,
}
type Feeds = Mutex<HashMap<String, Weak<Feed>>>;
static FEEDS: OnceLock<Feeds> = OnceLock::new();

/// A single POST SSE stream sends a full snapshot then aligned bucket replacements every 10s.
/// Identical subscriptions within the same organization share the plan, scans and retained history.
#[utoipa::path(post, path = "/api/v1/query/dashboard/live", request_body = DashboardQueryRequest,
    responses((status = 200, body = DashboardQueryFrame, content_type = "text/event-stream")), tag = "metrics")]
pub async fn dashboard_live(
    State(st): State<AppState>,
    OrgId(org): OrgId,
    Json(req): Json<DashboardQueryRequest>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, ApiError> {
    validate(&req)?;
    let mut ordered = req.queries.clone();
    ordered.sort_by(|a, b| a.id.cmp(&b.id));
    let key = format!(
        "{org}\0{}\0{}",
        req.range.to - req.range.from,
        serde_json::to_string(&ordered).map_err(|e| ApiError::Internal(e.into()))?
    );
    let mut feeds = FEEDS.get_or_init(Default::default).lock().await;
    feeds.retain(|_, f| f.strong_count() > 0);
    let mut rx = if let Some(feed) = feeds.get(&key).and_then(Weak::upgrade) {
        feed.tx.subscribe()
    } else {
        if feeds.len() >= 128 {
            return Err(ApiError::TooManyRequests(
                "too many active dashboard subscriptions".into(),
            ));
        }
        let (tx, rx) = watch::channel(None);
        let feed = Arc::new(Feed { tx });
        feeds.insert(key, Arc::downgrade(&feed));
        tokio::spawn(run_feed(feed, st, org, req));
        rx
    };
    drop(feeds);
    let (tx, out) = tokio::sync::mpsc::channel(2);
    tokio::spawn(async move {
        let mut initial = true;
        let mut last_to = None;
        loop {
            let current = rx.borrow_and_update().clone();
            if let Some(state) = current {
                let full = initial
                    || last_to.is_some_and(|last| {
                        state.delta.results.iter().any(|r| r.replace_from > last)
                    });
                let frame = if full { &state.snapshot } else { &state.delta };
                let event = Event::default()
                    .event(if full { "snapshot" } else { "delta" })
                    .json_data(frame);
                initial = false;
                last_to = Some(frame.range.to);
                if let Ok(event) = event {
                    if tx.send(Ok(event)).await.is_err() {
                        break;
                    }
                }
            }
            tokio::select! { _ = tx.closed() => break, changed = rx.changed() => if changed.is_err() { break; } }
        }
    });
    Ok(Sse::new(tokio_stream::wrappers::ReceiverStream::new(out)).keep_alive(KeepAlive::default()))
}

async fn run_feed(feed: Arc<Feed>, st: AppState, org: String, req: DashboardQueryRequest) {
    let plan = tokio::select! { _ = feed.tx.closed() => return, plan = Plan::prepare(&st, &org, &req) => plan };
    let mut plan = match plan {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!(error=%e, "dashboard planning failed");
            let frame = DashboardQueryFrame {
                results: Vec::new(),
                range: req.range,
                data_scans: 0,
                unique_queries: 0,
                error: Some("Dashboard query failed".into()),
            };
            let _ = feed.tx.send(Some(Arc::new(Published {
                snapshot: frame.clone(),
                delta: frame,
            })));
            return;
        }
    };
    let mut range = req.range;
    let snapshot = tokio::select! { _ = feed.tx.closed() => return, frame = plan.execute(&st, &org, range, None) => frame };
    let mut snapshot = snapshot;
    if feed
        .tx
        .send(Some(Arc::new(Published {
            snapshot: snapshot.clone(),
            delta: snapshot.clone(),
        })))
        .is_err()
    {
        return;
    }
    let mut last_replan = std::time::Instant::now();
    let mut tick = tokio::time::interval(Duration::from_secs(10));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    tick.tick().await;
    loop {
        tokio::select! { _ = feed.tx.closed() => return, _ = tick.tick() => {} }
        let mut reseed = false;
        if last_replan.elapsed() >= Duration::from_secs(60)
            && plan.items.iter().any(|p| p.kind == "unknown")
        {
            let missing_ids: HashSet<_> = plan
                .items
                .iter()
                .filter(|p| p.kind == "unknown")
                .flat_map(|p| p.ids.iter())
                .collect();
            let missing = DashboardQueryRequest {
                range: req.range,
                queries: req
                    .queries
                    .iter()
                    .filter(|q| missing_ids.contains(&q.id))
                    .cloned()
                    .collect(),
            };
            let discovered = tokio::select! { _ = feed.tx.closed() => return, p = Plan::prepare(&st, &org, &missing) => p };
            if discovered.is_ok_and(|p| p.items.iter().any(|p| p.kind != "unknown")) {
                let refreshed = tokio::select! { _ = feed.tx.closed() => return, p = Plan::prepare(&st, &org, &req) => p };
                if let Ok(refreshed) = refreshed {
                    plan = refreshed;
                    reseed = true;
                }
            }
            last_replan = std::time::Instant::now();
        }
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i64;
        let next = TimeRange {
            from: now - plan.duration,
            to: now,
        };
        let delta = tokio::select! { _ = feed.tx.closed() => return, frame = plan.execute(&st, &org, next, if reseed { None } else { Some(range.to) }) => frame };
        if reseed {
            snapshot = delta.clone();
        } else {
            merge_frame(&mut snapshot, &delta);
        }
        range = next;
        if feed
            .tx
            .send(Some(Arc::new(Published {
                snapshot: snapshot.clone(),
                delta,
            })))
            .is_err()
        {
            return;
        }
    }
}

fn merge_frame(snapshot: &mut DashboardQueryFrame, delta: &DashboardQueryFrame) {
    snapshot.range = delta.range;
    snapshot.data_scans = delta.data_scans;
    for update in &delta.results {
        let Some(old) = snapshot.results.iter_mut().find(|r| r.id == update.id) else {
            continue;
        };
        old.error = update.error.clone();
        if update.error.is_some() {
            continue;
        }
        for series in &mut old.series {
            series.points.retain(|(t, _)| {
                *t >= delta.range.from.div_euclid(update.step_ms) * update.step_ms
                    && *t < update.replace_from
            });
        }
        for series in &update.series {
            if let Some(existing) = old.series.iter_mut().find(|s| s.name == series.name) {
                existing.points.extend_from_slice(&series.points);
            } else {
                old.series.push(series.clone());
            }
        }
        old.series.retain(|s| !s.points.is_empty());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prepared(text: &str) -> Prepared {
        let (query, scale) = resolve_alias(&parse(text).unwrap());
        Prepared {
            query,
            text: text.into(),
            ids: vec!["q".into()],
            kind: "gauge".into(),
            unit: None,
            scalar_kind: Some(Kind::Gauge),
            weighted: false,
            scale,
            step: 30_000,
            error: None,
        }
    }

    #[test]
    fn many_groupings_and_filters_share_a_single_bound_scan() {
        let items = vec![
            prepared("sum:kubernetes.memory.usage{env:prod} by {kube_namespace}"),
            prepared("avg:kubernetes.memory.usage{!env:staging} by {host}"),
            prepared("max:kubernetes.cpu.requests{team:checkout} by {kube_deployment}"),
        ];
        let (sql, params) = shared_sql(
            &items,
            &[0, 1, 2],
            TimeRange {
                from: 0,
                to: 60_000,
            },
        );
        assert_eq!(sql.matches("otel.otel_metrics_gauge").count(), 1);
        assert!(sql.contains("OrgId = {oc_org:String}"));
        assert!(sql.contains("sum(v)") && sql.contains("avg(v)") && sql.contains("max(v)"));
        assert!(!sql.contains("checkout") && !sql.contains("staging"));
        assert_eq!(sql.matches('?').count(), params.len());
    }

    #[test]
    fn weighted_gauges_keep_coverage_in_the_shared_scan() {
        let mut query = prepared("sum:kubernetes_state.node.count{*} by {kube_cluster_name}");
        query.weighted = true;
        let (sql, params) = shared_sql(
            &[query],
            &[0],
            TimeRange {
                from: 0,
                to: 60_000,
            },
        );
        assert!(sql.contains("sum(v * samples)"));
        assert!(sql.contains("medianExactIf"));
        assert_eq!(sql.matches("otel.otel_metrics_gauge").count(), 1);
        assert_eq!(sql.matches('?').count(), params.len());
    }

    #[test]
    fn live_overlap_uses_full_buckets_at_the_original_resolution() {
        let range = query_range(
            TimeRange {
                from: 3_001,
                to: 3_600_555,
            },
            Some(3_590_000),
            30_000,
        );
        assert_eq!(range.from % 30_000, 0);
        assert!(range.from <= 3_590_000 - 60_000);
        assert!(range.to - range.from < 120_000);
    }

    #[test]
    fn delta_replaces_old_buckets_and_drops_groups_that_disappear() {
        let result = |series, replace_from| DashboardQueryResult {
            id: "q0".into(),
            series,
            kind: "gauge".into(),
            unit: None,
            step_ms: 10_000,
            replace_from,
            error: None,
        };
        let mut snapshot = DashboardQueryFrame {
            range: TimeRange {
                from: 0,
                to: 30_000,
            },
            data_scans: 1,
            unique_queries: 1,
            error: None,
            results: vec![result(
                vec![
                    DashboardMetricSeries {
                        tags: BTreeMap::new(),
                        name: "host:a".into(),
                        points: vec![(0, 1.0), (10_000, 2.0), (20_000, 3.0)],
                    },
                    DashboardMetricSeries {
                        tags: BTreeMap::new(),
                        name: "host:b".into(),
                        points: vec![(20_000, 4.0)],
                    },
                ],
                0,
            )],
        };
        let delta = DashboardQueryFrame {
            range: TimeRange {
                from: 5000,
                to: 40_000,
            },
            data_scans: 1,
            unique_queries: 1,
            error: None,
            results: vec![result(
                vec![DashboardMetricSeries {
                    tags: BTreeMap::new(),
                    name: "host:a".into(),
                    points: vec![(20_000, 8.0), (30_000, 9.0)],
                }],
                20_000,
            )],
        };
        merge_frame(&mut snapshot, &delta);
        assert_eq!(snapshot.results[0].series.len(), 1);
        assert_eq!(
            snapshot.results[0].series[0].points,
            vec![(0, 1.0), (10_000, 2.0), (20_000, 8.0), (30_000, 9.0)]
        );
    }

    #[test]
    fn duplicate_query_ids_are_rejected_before_execution() {
        let q = DashboardMetricQuery {
            id: "same".into(),
            query: "avg:test{*}".into(),
            points: None,
        };
        assert!(validate(&DashboardQueryRequest {
            queries: vec![q.clone(), q],
            range: TimeRange { from: 0, to: 1000 }
        })
        .is_err());
    }
}
