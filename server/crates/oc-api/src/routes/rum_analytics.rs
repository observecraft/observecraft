//! Product Analytics (Digital Experience → Product Analytics → Product Pulse): who uses the RUM
//! applications, on what, how they arrive, what they do and where they struggle, from the RUM events
//! of `otel.otel_rum` ([`super::rum`]).
//!
//! A user is `usr.id`, else the SDK's `usr.anonymous_id`, else the session. Views count once each
//! (their latest version). New users were first seen in the range (looking back 30 days before it);
//! returning users were seen before it too.

use axum::extract::State;
use axum::Json;
use oc_query::Param;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use super::logs::{bind, TimeRange};
use super::rum::RumBucket;
use crate::auth::AuthContext;
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;

const DAY: i64 = 86_400_000;
/// The user of an event.
const UID: &str = "if(LogAttributes['usr.id'] != '', LogAttributes['usr.id'], if(LogAttributes['usr.anonymous_id'] != '', LogAttributes['usr.anonymous_id'], SessionId))";

#[derive(Deserialize, ToSchema, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum AnalyticsPage {
    Home,
    Users,
    Devices,
    Traffic,
    Engagement,
    Performance,
}

#[derive(Deserialize, ToSchema, Debug)]
#[serde(rename_all = "camelCase")]
pub struct AnalyticsRequest {
    pub range: TimeRange,
    pub page: AnalyticsPage,
    #[serde(default)]
    pub application_id: Option<String>,
}

/// A value and its change from the previous period of the same length (percent; absent without one).
#[derive(Serialize, ToSchema, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct AnalyticsValue {
    pub value: Option<f64>,
    pub change: Option<f64>,
}

#[derive(Serialize, ToSchema, Debug, Default, Clone)]
#[serde(rename_all = "camelCase")]
pub struct AnalyticsItem {
    pub name: String,
    pub value: f64,
}

/// One page's numbers; the fields of other pages stay empty.
#[derive(Serialize, ToSchema, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct AnalyticsReport {
    pub active_users: AnalyticsValue,
    pub new_users: AnalyticsValue,
    pub returning_users: AnalyticsValue,
    pub views_per_user: AnalyticsValue,
    /// 75th percentile of the views' loading time, ms.
    pub load_time: AnalyticsValue,
    /// Average, ms.
    pub session_duration: AnalyticsValue,
    /// Sessions with a single view, percent.
    pub bounce_rate: AnalyticsValue,
    /// Percent.
    pub sessions_with_errors: AnalyticsValue,
    pub sessions: AnalyticsValue,
    pub sessions_per_user: AnalyticsValue,
    pub views_per_session: AnalyticsValue,
    /// Average time on a page, ms.
    pub time_on_page: AnalyticsValue,
    /// Users with an event in the last 5 minutes.
    pub users_online: AnalyticsValue,
    pub versions_in_use: AnalyticsValue,
    /// Sessions on desktop, percent (the rest: mobile and other devices).
    pub desktop: AnalyticsValue,
    pub issue_count: AnalyticsValue,
    /// Views with at least one frustration signal, percent.
    pub views_with_frustration: AnalyticsValue,
    /// Users (home, users), page views (traffic) per bucket.
    pub over_time: Vec<RumBucket>,
    pub bucket_ms: i64,
    pub countries: Vec<AnalyticsItem>,
    pub power_users: Vec<AnalyticsItem>,
    pub versions: Vec<AnalyticsItem>,
    pub browsers: Vec<AnalyticsItem>,
    pub sources: Vec<AnalyticsItem>,
    pub campaigns: Vec<AnalyticsItem>,
    pub popular_views: Vec<AnalyticsItem>,
    /// Frustration signals per view, on average, for the views that had some.
    pub frustration_by_view: Vec<AnalyticsItem>,
    /// Error messages by count.
    pub issues: Vec<AnalyticsItem>,
}

struct Scope {
    clause: String,
    params: Vec<Param>,
}

fn scope(range: TimeRange, app: Option<&str>) -> Scope {
    // Queries put `OrgId = {oc_org:String}` in front ([`AppState::ch`] binds it).
    let mut clause = "Timestamp >= fromUnixTimestamp64Milli(?) AND Timestamp <= fromUnixTimestamp64Milli(?)".to_string();
    let mut params = vec![Param::Int(range.from), Param::Int(range.to)];
    if let Some(a) = app.filter(|a| !a.is_empty()) {
        clause.push_str(" AND ApplicationId = ?");
        params.push(Param::Str(a.to_string()));
    }
    Scope { clause, params }
}

#[derive(clickhouse::Row, Deserialize, Default)]
struct Kpis {
    users: u64,
    sessions: u64,
    views: u64,
    load: f64,
    duration: f64,
    bounce: f64,
    with_errors: f64,
    time_on_page: f64,
    versions: u64,
    desktop: f64,
    issues: u64,
    frustrated_views: f64,
}

#[derive(clickhouse::Row, Deserialize)]
struct Item {
    name: String,
    value: f64,
}

#[derive(clickhouse::Row, Deserialize)]
struct Bucket {
    t: i64,
    n: u64,
}

#[derive(clickhouse::Row, Deserialize)]
struct UserSplit {
    new_users: u64,
    returning: u64,
}

#[derive(clickhouse::Row, Deserialize)]
struct Count {
    n: u64,
}

/// The KPIs of a range (all pages share them).
async fn kpis(ch: &clickhouse::Client, range: TimeRange, app: Option<&str>) -> Result<Kpis, ApiError> {
    let s = scope(range, app);
    // Sessions: their views and errors; views: their latest version. Scalar subqueries answer
    // Nullable types: each is unwrapped (nothing to average gives 0; the report leaves those out).
    let sql = format!(
        "WITH sessions AS (SELECT SessionId, uniqExactIf(ViewId, EventType = 'view') AS v, countIf(EventType = 'error') AS e, \
             toFloat64(dateDiff('millisecond', min(Timestamp), max(Timestamp))) AS d, argMin(LogAttributes['device.type'], Timestamp) AS dev \
             FROM otel.otel_rum WHERE OrgId = {{oc_org:String}} AND {c} AND SessionId != '' GROUP BY SessionId), \
         views AS (SELECT argMax(LogAttributes, toUInt64OrZero(LogAttributes['_dd.document_version'])) AS a FROM otel.otel_rum WHERE OrgId = {{oc_org:String}} AND {c} AND EventType = 'view' GROUP BY ViewId) \
         SELECT ifNull((SELECT uniqExact({UID}) FROM otel.otel_rum WHERE OrgId = {{oc_org:String}} AND {c}), 0) AS users, ifNull((SELECT count() FROM sessions), 0) AS sessions, ifNull((SELECT count() FROM views), 0) AS views, \
         ifNull((SELECT quantileIf(0.75)(toFloat64OrZero(a['view.loading_time']) / 1e6, a['view.loading_time'] != '') FROM views), 0) AS load, \
         ifNull((SELECT avg(d) FROM sessions), 0) AS duration, ifNull((SELECT 100 * countIf(v = 1) / greatest(countIf(v >= 1), 1) FROM sessions), 0) AS bounce, \
         ifNull((SELECT 100 * countIf(e > 0) / greatest(count(), 1) FROM sessions), 0) AS with_errors, \
         ifNull((SELECT avgIf(toFloat64OrZero(a['view.time_spent']) / 1e6, a['view.time_spent'] != '') FROM views), 0) AS time_on_page, \
         ifNull((SELECT uniqExact(ResourceAttributes['service.version']) FROM otel.otel_rum WHERE OrgId = {{oc_org:String}} AND {c} AND ResourceAttributes['service.version'] != ''), 0) AS versions, \
         ifNull((SELECT 100 * countIf(dev = 'Desktop') / greatest(count(), 1) FROM sessions), 0) AS desktop, \
         ifNull((SELECT uniqExact(LogAttributes['error.message']) FROM otel.otel_rum WHERE OrgId = {{oc_org:String}} AND {c} AND EventType = 'error'), 0) AS issues, \
         ifNull((SELECT 100 * countIf(toUInt64OrZero(a['view.frustration.count']) > 0) / greatest(count(), 1) FROM views), 0) AS frustrated_views",
        c = s.clause
    );
    // The scope's parameters for each of its uses, in order.
    let uses = sql.matches("Timestamp >= fromUnixTimestamp64Milli(?)").count();
    let params: Vec<Param> = (0..uses).flat_map(|_| s.params.clone()).collect();
    let k: Kpis = bind(ch.query(&sql), &params).fetch_one().await?;
    Ok(k)
}

fn value(now: f64, before: Option<f64>) -> AnalyticsValue {
    let change = before.filter(|b| *b != 0.0 && b.is_finite() && now.is_finite()).map(|b| 100.0 * (now - b) / b);
    AnalyticsValue { value: now.is_finite().then_some(now), change }
}

async fn items(ch: &clickhouse::Client, s: &Scope, select: &str, rest: &str) -> Result<Vec<AnalyticsItem>, ApiError> {
    let sql = format!("SELECT {select} FROM otel.otel_rum WHERE OrgId = {{oc_org:String}} AND {} {rest}", s.clause);
    let rows: Vec<Item> = bind(ch.query(&sql), &s.params).fetch_all().await?;
    Ok(rows.into_iter().map(|r| AnalyticsItem { name: r.name, value: r.value }).collect())
}

/// One Product Analytics page for the range (and an application, or all of them).
#[utoipa::path(post, path = "/api/v1/product-analytics/report", operation_id = "product_analytics_report", request_body = AnalyticsRequest,
    responses((status = 200, body = AnalyticsReport), (status = 400, body = crate::error::ErrorBody)), tag = "rum")]
pub async fn report(State(st): State<AppState>, ctx: AuthContext, Json(req): Json<AnalyticsRequest>) -> ApiResult<AnalyticsReport> {
    req.range.validate()?;
    let ch = st.ch(&ctx.org_id);
    let app = req.application_id.as_deref();
    let span = req.range.to - req.range.from;
    let k = kpis(&ch, req.range, app).await?;
    let p = kpis(&ch, TimeRange { from: req.range.from - span, to: req.range.from }, app).await?;
    let mut r = AnalyticsReport::default();
    let has = |x: u64| x > 0;
    let f = |x: u64| x as f64;
    r.active_users = value(f(k.users), has(p.users).then(|| f(p.users)));
    r.sessions = value(f(k.sessions), has(p.sessions).then(|| f(p.sessions)));
    let or_none = |x: f64, n: u64| if n > 0 { x } else { f64::NAN };
    r.load_time = value(or_none(k.load, k.views), has(p.views).then_some(p.load));
    r.session_duration = value(or_none(k.duration, k.sessions), has(p.sessions).then_some(p.duration));
    r.bounce_rate = value(k.bounce, has(p.sessions).then_some(p.bounce));
    r.sessions_with_errors = value(k.with_errors, has(p.sessions).then_some(p.with_errors));
    let ratio = |a: u64, b: u64| if b == 0 { f64::NAN } else { a as f64 / b as f64 };
    r.views_per_user = value(ratio(k.views, k.users), has(p.users).then(|| ratio(p.views, p.users)));
    r.sessions_per_user = value(ratio(k.sessions, k.users), has(p.users).then(|| ratio(p.sessions, p.users)));
    r.views_per_session = value(ratio(k.views, k.sessions), has(p.sessions).then(|| ratio(p.views, p.sessions)));
    r.time_on_page = value(or_none(k.time_on_page, k.views), has(p.views).then_some(p.time_on_page));
    r.versions_in_use = value(f(k.versions), None);
    r.desktop = value(if k.sessions > 0 { k.desktop } else { f64::NAN }, None);
    r.issue_count = value(f(k.issues), has(p.issues).then(|| f(p.issues)));
    r.views_with_frustration = value(if k.views > 0 { k.frustrated_views } else { f64::NAN }, None);
    if k.users == 0 {
        return Ok(Json(r));
    }

    let s = scope(req.range, app);
    let daily = span >= 2 * DAY;
    let size = if daily { DAY } else { super::apm::bucket_ms(req.range, 60) };
    r.bucket_ms = size;
    let bucket_of = format!("intDiv(toInt64(toUnixTimestamp64Milli(Timestamp)), {size}) * {size}");
    match req.page {
        AnalyticsPage::Home | AnalyticsPage::Users => {
            let rows: Vec<Bucket> = bind(ch.query(&format!("SELECT {bucket_of} AS t, uniqExact({UID}) AS n FROM otel.otel_rum WHERE OrgId = {{oc_org:String}} AND {} GROUP BY t ORDER BY t", s.clause)), &s.params).fetch_all().await?;
            r.over_time = rows.into_iter().map(|b| RumBucket { t: b.t, count: b.n }).collect();
        }
        AnalyticsPage::Traffic => {
            let rows: Vec<Bucket> =
                bind(ch.query(&format!("SELECT {bucket_of} AS t, uniqExact(ViewId) AS n FROM otel.otel_rum WHERE OrgId = {{oc_org:String}} AND {} AND EventType = 'view' GROUP BY t ORDER BY t", s.clause)), &s.params).fetch_all().await?;
            r.over_time = rows.into_iter().map(|b| RumBucket { t: b.t, count: b.n }).collect();
        }
        _ => {}
    }
    match req.page {
        AnalyticsPage::Users => {
            // First seen within 30 days before the range: new if in it, returning if before.
            let lookback = scope(TimeRange { from: req.range.from - 30 * DAY, to: req.range.to }, app);
            let params: Vec<Param> = [vec![Param::Int(req.range.from); 3], lookback.params].concat();
            let split: UserSplit = bind(
                ch.query(&format!(
                    "SELECT countIf(first >= fromUnixTimestamp64Milli(?)) AS new_users, countIf(first < fromUnixTimestamp64Milli(?) AND last >= fromUnixTimestamp64Milli(?)) AS returning \
                     FROM (SELECT {UID} AS u, min(Timestamp) AS first, max(Timestamp) AS last FROM otel.otel_rum WHERE OrgId = {{oc_org:String}} AND {} GROUP BY u)",
                    lookback.clause
                )),
                &params,
            )
            .fetch_one()
            .await?;
            r.new_users = value(f(split.new_users), None);
            r.returning_users = value(f(split.returning), None);
            r.countries = items(&ch, &s, &format!("LogAttributes['geo.country'] AS name, toFloat64(uniqExact({UID})) AS value"), "AND name != '' GROUP BY name ORDER BY value DESC LIMIT 10").await?;
            r.power_users = items(&ch, &s, &format!("{UID} AS name, toFloat64(uniqExact(ViewId)) AS value"), "AND EventType = 'view' GROUP BY name ORDER BY value DESC LIMIT 10").await?;
        }
        AnalyticsPage::Devices => {
            r.versions = items(&ch, &s, &format!("ResourceAttributes['service.version'] AS name, toFloat64(uniqExact({UID})) AS value"), "AND name != '' GROUP BY name ORDER BY value DESC LIMIT 10").await?;
            r.browsers = items(
                &ch,
                &s,
                &format!("if(LogAttributes['browser.name'] != '', LogAttributes['browser.name'], LogAttributes['os.name']) AS name, toFloat64(uniqExact({UID})) AS value"),
                "AND name != '' GROUP BY name ORDER BY value DESC LIMIT 10",
            )
            .await?;
        }
        AnalyticsPage::Traffic => {
            r.sources = items(
                &ch,
                &s,
                "if(LogAttributes['view.referrer'] = '', 'Direct', domain(LogAttributes['view.referrer'])) AS name, toFloat64(uniqExact(ViewId)) AS value",
                "AND EventType = 'view' GROUP BY name ORDER BY value DESC LIMIT 10",
            )
            .await?;
            r.campaigns = items(&ch, &s, "extractURLParameter(LogAttributes['view.url'], 'utm_source') AS name, toFloat64(uniqExact(ViewId)) AS value", "AND EventType = 'view' AND name != '' GROUP BY name ORDER BY value DESC LIMIT 10")
                .await?;
        }
        AnalyticsPage::Engagement => {
            let now = crate::routes::metrics::now_ms();
            let recent = scope(TimeRange { from: now - 5 * 60_000, to: now }, app);
            let online: Count = bind(ch.query(&format!("SELECT uniqExact({UID}) AS n FROM otel.otel_rum WHERE OrgId = {{oc_org:String}} AND {}", recent.clause)), &recent.params).fetch_one().await?;
            r.users_online = value(f(online.n), None);
            r.popular_views = items(&ch, &s, "LogAttributes['view.name'] AS name, toFloat64(uniqExact(ViewId)) AS value", "AND EventType = 'view' AND name != '' GROUP BY name ORDER BY value DESC LIMIT 10").await?;
        }
        AnalyticsPage::Performance => {
            r.frustration_by_view = items(
                &ch,
                &s,
                "LogAttributes['view.name'] AS name, countIf(EventType = 'action' AND LogAttributes['action.frustration.type'] != '') / greatest(uniqExactIf(ViewId, EventType = 'view'), 1) AS value",
                "AND name != '' GROUP BY name HAVING value > 0 ORDER BY value DESC LIMIT 12",
            )
            .await?;
            r.issues = items(&ch, &s, "LogAttributes['error.message'] AS name, toFloat64(count()) AS value", "AND EventType = 'error' GROUP BY name ORDER BY value DESC LIMIT 20").await?;
        }
        AnalyticsPage::Home => {}
    }
    Ok(Json(r))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn changes_compare_with_the_previous_period() {
        let v = value(150.0, Some(100.0));
        assert_eq!((v.value, v.change), (Some(150.0), Some(50.0)));
        assert_eq!(value(5.0, Some(0.0)).change, None);
        assert_eq!(value(f64::NAN, None).value, None);
    }

    #[tokio::test]
    async fn every_page_answers_against_live_data() {
        let Some((_, states)) = crate::test_support::replicas(1).await else { return };
        let st = states.into_iter().next().unwrap();
        let ctx = crate::test_support::admin_of_new_org(&st, "pa").await;
        let now = crate::routes::metrics::now_ms();
        for (offset, kind, session, view, extra) in [
            (-60_000, "view", "s1", "v1", "('view.name','/'),('view.loading_time','800000000'),('usr.id','u-1'),('geo.country','Germany'),('device.type','Desktop')"),
            (-50_000, "view", "s1", "v2", "('view.name','/cart'),('view.loading_time','1200000000'),('usr.id','u-1'),('view.referrer','https://search.example.com/q')"),
            (-40_000, "error", "s1", "v2", "('view.name','/cart'),('error.message','boom'),('usr.id','u-1')"),
            (-30_000, "view", "s2", "v3", "('view.name','/'),('device.type','Mobile'),('view.url','https://shop.example.com/\\x3Futm_source=newsletter')"),
        ] {
            st.clickhouse
                .query(&format!(
                    "INSERT INTO otel.otel_rum (Timestamp, Body, ResourceAttributes, LogAttributes) VALUES (fromUnixTimestamp64Milli(?), '{{}}', map('oc.org_id', ?, 'service.version', '2.0'), \
                     mapFromArrays(arrayMap(x -> x.1, [('type', ?), ('session.id', ?), ('view.id', ?), {extra}]), arrayMap(x -> x.2, [('type', ?), ('session.id', ?), ('view.id', ?), {extra}])))"
                ))
                .bind(now + offset)
                .bind(&ctx.org_id)
                .bind(kind)
                .bind(session)
                .bind(view)
                .bind(kind)
                .bind(session)
                .bind(view)
                .execute()
                .await
                .unwrap();
        }
        let get = |page| {
            let st = st.clone();
            let ctx = ctx.clone();
            async move { report(State(st), ctx, Json(AnalyticsRequest { range: TimeRange { from: now - 3_600_000, to: now }, page, application_id: None })).await.unwrap().0 }
        };
        let home = get(AnalyticsPage::Home).await;
        assert_eq!(home.active_users.value, Some(2.0), "u-1 and the anonymous session s2");
        assert_eq!(home.sessions.value, Some(2.0));
        assert_eq!(home.bounce_rate.value, Some(50.0));
        assert_eq!(home.sessions_with_errors.value, Some(50.0));
        assert!(!home.over_time.is_empty());
        let users = get(AnalyticsPage::Users).await;
        assert_eq!((users.new_users.value, users.countries[0].name.as_str()), (Some(2.0), "Germany"));
        let devices = get(AnalyticsPage::Devices).await;
        assert_eq!((devices.desktop.value, devices.versions[0].name.as_str()), (Some(50.0), "2.0"));
        let traffic = get(AnalyticsPage::Traffic).await;
        assert!(traffic.sources.iter().any(|x| x.name == "search.example.com") && traffic.campaigns[0].name == "newsletter");
        let perf = get(AnalyticsPage::Performance).await;
        assert_eq!((perf.issue_count.value, perf.issues[0].name.as_str()), (Some(1.0), "boom"));
        get(AnalyticsPage::Engagement).await;
    }
}
