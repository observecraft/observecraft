//! Dashboard-wide execution: one query per widget query, run concurrently (at most
//! [`CONCURRENT_QUERIES`] per request), each through a short result cache with request coalescing
//! ([`results_cache`]: viewers of the same dashboard, and its live feed, share executions); isolated query
//! errors and incremental SSE.
use std::collections::{BTreeMap, HashMap, HashSet};
use std::convert::Infallible;
use std::sync::{Arc, OnceLock, Weak};
use std::time::Duration;

use axum::extract::State;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::Json;
use futures::{stream, Stream, StreamExt};
use oc_query::{MetricQuery, Param};
use serde::{Deserialize, Serialize};
use tokio::sync::{watch, Mutex};
use utoipa::ToSchema;

use super::super::logs::TimeRange;
use super::{bind, group_label, inner_query_at_step, parse, round, step_ms, Kind, Row, Source, SPAN_METRICS};
use crate::error::{ApiError, ApiResult};
use crate::ttl_cache::TtlCache;
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
    source: Source,
    error: Option<String>,
    /// A derived metric's query: evaluated on its own as its formula, over the whole range.
    formula: Option<String>,
}

struct Plan {
    items: Vec<Prepared>,
    duration: i64,
}

/// Widget queries of one request run at once.
const CONCURRENT_QUERIES: usize = 8;

#[derive(clickhouse::Row, Deserialize)]
struct Meta {
    name: String,
    kind: String,
    unit: String,
}

fn validate(req: &DashboardQueryRequest) -> Result<(), ApiError> {
    req.range.validate_within(u32::MAX)?;
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
        let tiers = super::org_tiers(st, org).await?;
        req.range.validate_within(
            tiers
                .raw_days
                .max(tiers.rollup_5m_days)
                .max(tiers.rollup_1h_days),
        )?;
        let mut items: Vec<Prepared> = Vec::new();
        let mut seen = HashMap::new();
        let derived = super::super::derived_metrics::org_derived(st, org).await?;
        let index = super::super::metric_rules::index_rules(st, org).await?;
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
            let (query, mut error) = match parsed {
                Ok(q) => (q, None),
                Err(e) => (parse("avg:invalid{*}")?, Some(e.to_string())),
            };
            if let (None, Err(e)) = (&error, super::super::metric_rules::check_indexed(&query, &index)) {
                error = Some(e);
            }
            let mut formula = None;
            if error.is_none() && derived.contains_key(&query.metric) {
                match super::super::derived_metrics::expand(&item.query, &derived) {
                    Ok(_) if weighted => error = Some("weighted() isn't supported on derived metrics".into()),
                    Ok(_) => formula = Some(item.query.clone()),
                    Err(e) => error = Some(e),
                }
            }
            let (query, scale) = super::resolve_alias_if_missing(st, org, &query).await?;
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
                source: Source::Samples,
                error,
                formula,
            });
        }
        let names: Vec<_> = items
            .iter()
            .filter(|i| i.error.is_none() && i.formula.is_none())
            .map(|i| i.query.metric.clone())
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();
        let mut metadata = HashMap::new();
        let mut units = HashMap::new();
        if !names.is_empty() {
            let marks = vec!["?"; names.len()].join(",");
            let mut params = Vec::new();
            let bases: Vec<_> = names
                .iter()
                .map(|n| {
                    super::split_stat(n)
                        .map(|(b, _)| b.to_string())
                        .unwrap_or(n.clone())
                })
                .collect();
            let sql = format!("SELECT MetricName AS name, multiIf(any(MetricType) = 'sum', multiIf(any(AggregationTemporality) = 1, 'delta', any(IsMonotonic), 'counter', 'updown'), toString(any(MetricType))) AS kind, any(MetricUnit) AS unit FROM otel.metrics_series WHERE OrgId = {{oc_org:String}} AND (MetricName IN ({marks}) OR MetricName IN ({marks})) GROUP BY MetricName");
            params.extend(names.iter().chain(bases.iter()).cloned().map(Param::Str));
            for meta in bind(st.ch(org).query(&sql), &params)
                .fetch_all::<Meta>()
                .await?
            {
                if let Some(unit) = metric_unit(&meta.name, &meta.unit) {
                    units.insert(meta.name.clone(), unit);
                }
                metadata.entry(meta.name).or_insert(meta.kind);
            }
        }
        for p in items.iter_mut() {
            if p.error.is_some() {
                continue;
            }
            if p.formula.is_some() {
                p.kind = "derived".into();
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
                // The alias's unit only when the alias stood in (its values are scaled to it).
                .find(|a| Some(a.0) == original && p.query.metric != a.0)
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
            if p.kind != "span" {
                (p.source, p.step) = super::resolution(req.range, p.step, super::now_ms(), tiers);
            }
            if p.kind == "unknown" {
                p.error = Some(format!("Unknown metric: {}", p.query.metric));
            } else if p.weighted && (p.kind != "gauge" || p.query.agg.sql() != "sum") {
                p.error = Some("weighted() requires a sum aggregation on a gauge".into());
            }
        }
        Ok(Self {
            items,
            duration: req.range.to - req.range.from,
        })
    }

    /// Runs item `i` over `subrange` (cached). Returns what it found and whether it ran a query.
    async fn run_item(&self, st: &AppState, org: &str, i: usize, range: TimeRange, subrange: TimeRange, done: crate::metric_rollups::Progress) -> (Result<Fetched, ApiError>, bool) {
        let p = &self.items[i];
        if let Some(text) = &p.formula {
            let derived = super::evaluate_formula(st, org, text, range, 120).await.map(|(_, groups)| {
                groups.into_iter().map(|g| DashboardMetricSeries { name: if g.tags.is_empty() { text.clone() } else { group_label(&g.tags) }, tags: g.tags, points: g.points }).collect::<Vec<_>>()
            });
            return (derived.map(Fetched::Series), true);
        }
        let built = match p.scalar_kind {
            Some(kind) => Ok(widget_sql(p, kind, subrange, done)),
            None => inner_query_at_step(st, org, &p.query, subrange, 120, Some((p.source, p.step)))
                .await
                .map(|(inner, params, _)| (format!("SELECT g, toInt64(toUnixTimestamp(t)) * 1000 AS ts, toFloat64({}(v)) AS value FROM ({inner}) WHERE isFinite(v) GROUP BY g, t ORDER BY g, t", p.query.agg.sql()), params)),
        };
        let (sql, params) = match built {
            Ok(b) => b,
            Err(e) => return (Err(e), false),
        };
        // The SQL and its values say everything about the result (metric, filters, groups, range,
        // step, rollup progress); the org keeps orgs apart.
        let key = format!("{org}\0{sql}\0{params:?}");
        let ch = super::points_client(st, org);
        let ran = std::sync::atomic::AtomicBool::new(false);
        let rows = results_cache(p.step)
            .get_or_load(&key, || async {
                ran.store(true, std::sync::atomic::Ordering::Relaxed);
                bind(ch.query(&sql), &params).fetch_all::<Row>().await.map_err(ApiError::from)
            })
            .await;
        (rows.map(Fetched::Rows), ran.into_inner())
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
                // A derived metric is evaluated over the whole range every time.
                replace_from: if p.formula.is_some() { range.from } else { query_range(range, previous_to, p.step).from },
                error: p.error.clone(),
            })
            .collect();
        let jobs: Vec<usize> = (0..self.items.len()).filter(|i| self.items[*i].error.is_none()).collect();
        // Without the job's progress every rollup is read on the fly from the samples (slower, same values).
        let done = crate::metric_rollups::progress(st, org).await.unwrap_or_default();
        let mut work = stream::iter(jobs.into_iter().map(|i| async move {
            let p = &self.items[i];
            // Whole steps: a query issued a little later in the same step is the same query.
            let mut subrange = query_range(range, previous_to, p.step);
            subrange.to = (subrange.to + p.step - 1).div_euclid(p.step) * p.step;
            let (data, ran) = self.run_item(st, org, i, range, subrange, done).await;
            (i, data, ran)
        }))
        .buffer_unordered(CONCURRENT_QUERIES);
        let mut scans = 0;
        while let Some((i, data, ran)) = work.next().await {
            scans += usize::from(ran);
            match data {
                Ok(Fetched::Series(series)) => results[i].series = series,
                Ok(Fetched::Rows(rows)) => {
                    let p = &self.items[i];
                    let target = &mut results[i].series;
                    for r in rows.iter() {
                        let tags = p
                            .query
                            .by
                            .iter()
                            .cloned()
                            .zip(r.g.iter().map(|v| if v.is_empty() { "N/A".to_string() } else { v.clone() }))
                            .collect::<BTreeMap<_, _>>();
                        let name = if p.query.by.is_empty() { p.text.clone() } else { group_label(&tags) };
                        if let Some(s) = target.last_mut().filter(|s| s.name == name) {
                            s.points.push((r.ts, round(r.value)));
                        } else {
                            target.push(DashboardMetricSeries { name, tags, points: vec![(r.ts, round(r.value))] });
                        }
                    }
                }
                Err(e) => {
                    tracing::warn!(error = %e, "dashboard query failed");
                    results[i].error = Some(match e {
                        ApiError::BadRequest(m) => m,
                        _ => "Query failed".into(),
                    });
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

/// Widget query results, kept about min(step, 30 s) (`crate::ttl_cache`: concurrent identical
/// queries wait for one execution; keys start with the org).
fn results_cache(step_ms: i64) -> &'static TtlCache<Vec<Row>> {
    static CACHES: OnceLock<[TtlCache<Vec<Row>>; 3]> = OnceLock::new();
    let caches = CACHES.get_or_init(|| [10, 20, 30].map(|s| TtlCache::new(Duration::from_secs(s))));
    &caches[if step_ms <= 10_000 { 0 } else if step_ms <= 20_000 { 1 } else { 2 }]
}

/// What one query returned: rows (shared through the cache), or a derived metric's series.
enum Fetched {
    Rows(Arc<Vec<Row>>),
    Series(Vec<DashboardMetricSeries>),
}

/// A `.weighted()` sum: each series counts for the share of the interval it reported in. Its
/// samples per interval (count, first and last time) give its cadence; the interval holds
/// (span of the group's samples / mean cadence + 1) submission slots.
const WEIGHTED: &str = "sum(v * samples) / if(countIf(samples > 1) > 0, \
     greatest(round((max(last_seen) - min(first_seen)) / greatest(sumIf(last_seen - first_seen, samples > 1) / sumIf(samples - 1, samples > 1), 1)) + 1, 1), 1)";

/// One widget query of a gauge or sum metric: the per-series values of [`super::points_query`]
/// combined per group and interval with the query's space aggregator (or [`WEIGHTED`]).
fn widget_sql(p: &Prepared, kind: Kind, range: TimeRange, done: crate::metric_rollups::Progress) -> (String, Vec<Param>) {
    let options = super::PointsOptions { scale: p.scale, done, coverage: p.weighted };
    let (inner, params) = super::points_query(kind, p.source, &p.query, range, p.step / 1000, options);
    let aggregate = if p.weighted { WEIGHTED.to_string() } else { format!("{}(v)", p.query.agg.sql()) };
    (format!("SELECT g, toInt64(toUnixTimestamp(t)) * 1000 AS ts, toFloat64({aggregate}) AS value FROM ({inner}) WHERE isFinite(v) GROUP BY g, t ORDER BY g, t"), params)
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

/// Units of integration metrics from integrations-core's metadata.csv (BSD-3-Clause, see
/// server/seeds/integrations/kubernetes/LICENSE): the Datadog Agent sends none.
const INTEGRATION_UNITS: &str = include_str!("../../../../../seeds/integrations/kubernetes/units.csv");
const INTEGRATION_DESCRIPTIONS: &str = include_str!("../../../../../seeds/integrations/kubernetes/descriptions.tsv");

/// A metric's unit: what its sender reported, else the integration's, else none.
pub(super) fn metric_unit(metric: &str, reported: &str) -> Option<String> {
    static UNITS: OnceLock<HashMap<&'static str, &'static str>> = OnceLock::new();
    if !reported.is_empty() {
        return Some(reported.to_string());
    }
    let units = UNITS.get_or_init(|| INTEGRATION_UNITS.lines().filter(|l| !l.starts_with('#')).filter_map(|l| l.split_once(',')).collect());
    units.get(metric).map(|u| u.to_string())
}

/// What a metric measures: as its sender described it, else integrations-core's description.
pub(super) fn metric_description(metric: &str, reported: &str) -> String {
    static DESCRIPTIONS: OnceLock<HashMap<&'static str, &'static str>> = OnceLock::new();
    if !reported.is_empty() {
        return reported.to_string();
    }
    let descriptions = DESCRIPTIONS.get_or_init(|| INTEGRATION_DESCRIPTIONS.lines().filter(|l| !l.starts_with('#')).filter_map(|l| l.split_once('\t')).collect());
    descriptions.get(metric).map(|d| d.to_string()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::super::{group_array, resolve_alias};
    use super::*;
    use oc_query::{metric_field, metric_filter};
    use std::collections::HashSet;

    // The oracle: the dashboard's previous execution, every widget of a kind, step and source in one
    // shared scan with per-series uniqExact sample counts and a median cadence for weighted().
    #[derive(clickhouse::Row, Deserialize)]
    struct BatchRow {
        qi: u32,
        g: Vec<String>,
        ts: i64,
        value: f64,
    }

    fn oracle_shared_sql(items: &[Prepared], indices: &[usize], range: TimeRange, done: crate::metric_rollups::Progress) -> (String, Vec<Param>) {
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
        // The series side binds grouping/filter values first, then the point side's names/window.
        params.extend(metrics.iter().cloned().map(Param::Str));
        params.push(Param::Int(range.from));
        let step_s = first.step / 1000;
        let rollup = first.source != Source::Samples;
        // Rollups (up to the rollup job's progress, then the rest on the fly) come first in the text.
        let (points, filter) = if rollup {
            let (rows, rows_params) = super::super::rollup_rows(first.source, false, &metrics, range, done, None);
            params.splice(0..0, rows_params);
            (format!("{rows} AS x"), String::new())
        } else {
            params.extend(metrics.iter().cloned().map(Param::Str));
            params.extend([Param::Int(range.from), Param::Int(range.to)]);
            (
                "otel.metrics_samples AS x".to_string(),
                format!(
                    "WHERE x.OrgId = {{oc_org:String}} AND x.MetricName IN ({}) AND x.TimeUnix >= toStartOfInterval(fromUnixTimestamp64Milli(?), INTERVAL 1 SECOND) AND x.TimeUnix <= fromUnixTimestamp64Milli(?)",
                    vec!["?"; metrics.len()].join(",")
                ),
            )
        };
        let value = match (kind, rollup) {
            (Kind::Gauge | Kind::UpDown, false) => "avg(x.Value)".to_string(),
            (Kind::Gauge | Kind::UpDown, true) => "sum(x.Sum) / sum(x.Count)".to_string(),
            (Kind::Counter, false) => "greatest(max(x.Value) - min(x.Value), 0) * 1000 / greatest(dateDiff('millisecond', min(x.TimeUnix), max(x.TimeUnix)), 1)".to_string(),
            (Kind::Counter, true) => "greatest(max(x.Max) - min(x.Min), 0) * 1000 / greatest(dateDiff('millisecond', min(x.Start), max(x.End)), 1)".to_string(),
            (Kind::Delta, false) => format!("sum(x.Value) / {step_s}"),
            (Kind::Delta, true) => format!("sum(x.Sum) / {step_s}"),
        };
        let time = if rollup { "Bucket" } else { "TimeUnix" };
        let samples = if rollup {
            "sum(x.Count)"
        } else {
            "uniqExact(x.TimeUnix)"
        };
        let (start, end) = if rollup {
            ("x.Start", "x.End")
        } else {
            ("x.TimeUnix", "x.TimeUnix")
        };
        let metric_type = if kind == Kind::Gauge { "gauge" } else { "sum" };
        let sql = format!(
            "SELECT toUInt32(qi) AS qi, g, toInt64(toUnixTimestamp(t)) * 1000 AS ts, toFloat64(multiIf({aggregates}, avg(v))) AS value FROM (\
             SELECT s.qi AS qi, s.g AS g, toStartOfInterval(x.{time}, INTERVAL {step_s} SECOND) AS t, x.SeriesId AS sid, \
             {samples} AS samples, min(toUnixTimestamp64Milli({start})) AS first_seen, max(toUnixTimestamp64Milli({end})) AS last_seen, ({value}) * multiIf({scaling}, 1.0) AS v \
             FROM {points} INNER JOIN (SELECT SeriesId, qi, any(multiIf({grouping}, CAST([], 'Array(String)'))) AS g \
             FROM otel.metrics_series ARRAY JOIN arrayFilter(x -> x >= 0, [{eligible}]) AS qi \
             WHERE OrgId = {{oc_org:String}} AND MetricName IN ({marks}) AND MetricType = '{metric_type}' AND {seen} GROUP BY SeriesId, qi) AS s ON x.SeriesId = s.SeriesId \
             {filter} \
             GROUP BY qi, g, t, sid{having}) WHERE isFinite(v) GROUP BY qi, g, t ORDER BY qi, g, t",
            seen = super::super::SEEN_SINCE,
            aggregates=aggregates.join(","), grouping=grouping.join(","), scaling=scaling.join(","), eligible=eligible.join(","),
            marks=vec!["?"; metrics.len()].join(","), having=if kind == Kind::Counter { format!(" HAVING {samples} > 1") } else { String::new() }
        );
        (sql, params)
    }


    #[test]
    fn integration_metrics_get_their_descriptions_when_senders_send_none() {
        assert_eq!(metric_description("kubernetes.cpu.usage.total", ""), "The number of cores used");
        assert_eq!(metric_description("kubernetes.cpu.usage.total", "Sent by the app"), "Sent by the app");
        assert_eq!(metric_description("my.custom.metric", ""), "");
    }

    #[test]
    fn integration_metrics_get_their_units_when_senders_send_none() {
        // The Datadog Agent sends no units; integrations-core's metadata.csv has them.
        assert_eq!(metric_unit("kubernetes.cpu.usage.total", ""), Some("nanocore".to_string()));
        assert_eq!(metric_unit("kubernetes.memory.usage", ""), Some("byte".to_string()));
        assert_eq!(metric_unit("kubernetes.network.rx_bytes", ""), Some("byte/second".to_string()));
        // The Agent reports host memory in MiB (Datadog's metadata: mebibyte).
        assert_eq!(metric_unit("system.mem.used", ""), Some("mebibyte".to_string()));
        // What a sender reported wins; counts have no unit.
        assert_eq!(metric_unit("kubernetes.cpu.usage.total", "{core}"), Some("{core}".to_string()));
        assert_eq!(metric_unit("kubernetes_state.deployment.replicas", ""), None);
    }

    /// One query per widget gives the previous shared scan's results, for every kind (rates,
    /// deltas, gauges, up-down sums), filters, groups, weighted() and all three resolutions.
    /// weighted() now counts samples with count() (not uniqExact) and takes the mean cadence (not
    /// the median): equal for series with one point per timestamp and a regular cadence, as here.
    #[tokio::test]
    async fn widget_queries_match_the_shared_scan_on_samples_and_rollups() {
        let Ok(url) = std::env::var("OC_TEST_CLICKHOUSE_URL") else {
            return;
        };
        let user = std::env::var("OC_TEST_CLICKHOUSE_USER").unwrap_or_else(|_| "default".into());
        let password = std::env::var("OC_TEST_CLICKHOUSE_PASSWORD").unwrap_or_default();
        oc_store::migrate_clickhouse(
            &oc_store::ClickHouseConfig {
                url: url.clone(),
                user: user.clone(),
                password: password.clone(),
                database: "oc".into(),
                cluster: None,
            },
            false,
        )
        .await
        .unwrap();
        let org = format!(
            "dashboard-parity-{}-{}",
            std::process::id(),
            super::super::now_ms()
        );
        let ch = clickhouse::Client::default()
            .with_url(url)
            .with_user(user)
            .with_password(password)
            .with_option("param_oc_org", &org);
        let from = super::super::now_ms().div_euclid(3_600_000) * 3_600_000;
        let range = TimeRange {
            from,
            to: from + 3_600_000,
        };
        for (kind, name, temporality, monotonic) in [
            (Kind::Gauge, "dashboard.test.gauge", 0, false),
            (Kind::Counter, "dashboard.test.counter", 2, true),
            (Kind::Delta, "dashboard.test.delta", 1, true),
            (Kind::UpDown, "dashboard.test.updown", 2, false),
        ] {
            let table = if kind == Kind::Gauge {
                "otel.metrics_ingest_gauge"
            } else {
                "otel.metrics_ingest_sum"
            };
            let (extra_cols, extra_values) = if kind == Kind::Gauge {
                (String::new(), String::new())
            } else {
                (
                    ", AggregationTemporality, IsMonotonic".into(),
                    format!(", {temporality}, {monotonic}"),
                )
            };
            ch.query(&format!("INSERT INTO {table} (ResourceAttributes, MetricName, MetricUnit, Attributes, TimeUnix, Value{extra_cols}) SELECT map('oc.org_id', ?, 'host.name', if(number < 6, 'storefront-node', 'checkout-node'), 'deployment.environment', if(number < 6, 'prod', 'staging')), ?, '1', map('state', 'running'), fromUnixTimestamp64Milli(?) + toIntervalSecond((number % 6) * 10), toFloat64(10 + number * 2){extra_values} FROM numbers(12)"))
                .bind(&org).bind(name).bind(from).execute().await.unwrap();
            for tier in [crate::metric_rollups::Tier::FiveMinutes, crate::metric_rollups::Tier::Hour] {
                crate::metric_rollups::roll(&ch, false, tier, from, from + 3_600_000).await.unwrap();
            }
            let done = crate::metric_rollups::Progress { five_minutes: i64::MAX, hour: i64::MAX };
            for (source, step) in [
                (Source::Samples, 60_000),
                (Source::Rollup5m, 300_000),
                (Source::Rollup1h, 3_600_000),
            ] {
                let mut items = vec![
                    prepared(&format!("sum:{name}{{*}} by {{host}}")),
                    prepared(&format!("avg:{name}{{env:prod}}")),
                    prepared(&format!("max:{name}{{*}}")),
                    prepared(&format!("sum:{name}{{state:*}} by {{host}}")),
                    prepared(&format!("sum:{name}{{missing:*}} by {{host}}")),
                    prepared(&format!("sum:{name}{{!missing:*}} by {{host}}")),
                ];
                if kind == Kind::Gauge {
                    for text in [format!("sum:{name}{{*}} by {{host}}"), format!("sum:{name}{{env:prod}}")] {
                        let mut weighted = prepared(&text);
                        weighted.weighted = true;
                        items.push(weighted);
                    }
                }
                for p in &mut items {
                    p.scalar_kind = Some(kind);
                    p.source = source;
                    p.step = step;
                }
                let indices: Vec<usize> = (0..items.len()).collect();
                let (sql, params) = oracle_shared_sql(&items, &indices, range, done);
                let actual = bind(ch.query(&sql), &params)
                    .fetch_all::<BatchRow>()
                    .await
                    .unwrap_or_else(|e| panic!("{sql}: {e}"));
                for (index, p) in items.iter().enumerate() {
                    let (sql, params) = widget_sql(p, kind, range, done);
                    let expected = bind(ch.query(&sql), &params)
                        .fetch_all::<Row>()
                        .await
                        .unwrap_or_else(|e| panic!("{sql}: {e}"));
                    let actual: Vec<_> = actual.iter().filter(|r| r.qi == index as u32).collect();
                    if index == 4 {
                        assert!(
                            actual.is_empty(),
                            "a wildcard tag must exclude missing tags"
                        );
                    } else if index >= 3 {
                        assert!(
                            !actual.is_empty(),
                            "present tags and negated missing tags must match"
                        );
                    }
                    assert_eq!(
                        actual.len(),
                        expected.len(),
                        "{kind:?} {source:?} query {index}"
                    );
                    for (a, e) in actual.iter().zip(&expected) {
                        assert_eq!((&a.g, a.ts), (&e.g, e.ts));
                        assert!(
                            (a.value - e.value).abs() < 1e-9 * a.value.abs().max(1.0),
                            "{kind:?} {source:?} query {index}: {} != {}",
                            a.value,
                            e.value
                        );
                    }
                }
            }
        }
        let (sql, params) = widget_sql(&prepared("sum:dashboard.test.gauge{*}"), Kind::Gauge, range, Default::default());
        let other = ch
            .clone()
            .with_option("param_oc_org", format!("{org}-empty"));
        assert!(bind(other.query(&sql), &params)
            .fetch_all::<Row>()
            .await
            .unwrap()
            .is_empty());
    }

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
            source: Source::Samples,
            error: None,
            formula: None,
        }
    }

    #[test]
    fn a_widget_query_reads_its_own_metric_with_bound_values() {
        let p = prepared("sum:kubernetes.memory.usage{env:prod,team:checkout} by {kube_namespace}");
        let (sql, params) = widget_sql(&p, Kind::Gauge, TimeRange { from: 0, to: 60_000 }, Default::default());
        assert!(sql.contains("OrgId = {oc_org:String}") && sql.contains("sum(v)"));
        assert!(!sql.contains("checkout") && !sql.contains("prod") && !sql.contains("kubernetes.memory.usage"));
        assert_eq!(sql.matches('?').count(), params.len());
    }

    #[test]
    fn weighted_widgets_derive_cadence_from_sample_counts_and_spans() {
        for source in [Source::Samples, Source::Rollup5m] {
            let mut p = prepared("sum:kubernetes_state.node.count{*} by {kube_cluster_name}");
            p.weighted = true;
            p.source = source;
            p.step = 300_000;
            let (sql, params) = widget_sql(&p, Kind::Gauge, TimeRange { from: 0, to: 600_000 }, crate::metric_rollups::Progress { five_minutes: 300_000, hour: 0 });
            assert!(sql.contains("sum(v * samples)"));
            // No per-series, per-interval exact distinct counts or medians.
            assert!(!sql.contains("uniqExact") && !sql.contains("median"), "{sql}");
            assert_eq!(sql.matches('?').count(), params.len());
        }
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
