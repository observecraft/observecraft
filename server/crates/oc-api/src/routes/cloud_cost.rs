//! Cloud Cost: daily cost line items (ClickHouse `otel.cloud_costs`, FOCUS columns) and what reads them.
//!
//! - Costs come from custom costs files uploaded through Datadog's API (`datadog_api::cost`) and, in
//!   demo mode, from [`seed_demo`] (a fictional year of AWS, Azure, Google Cloud and SaaS bills).
//! - They are queried as Datadog's cost metrics ([`cost_metric`]): `all.cost`, `aws.cost.amortized`,
//!   `azure.cost.actual`, `custom.cost.basis`, … with the cost tags (`providername`, `servicename`,
//!   `subaccountname`, `regionname`, … and the bills' own tags), so dashboards (`data_source:
//!   cloud_cost`), monitors and budgets read them through the metric engine (`metrics.rs`).
//! - The Cloud Cost pages read [`query`] (totals per group and calendar interval, with the previous
//!   period) and [`tags`] (the tags costs can be broken down by).
//!
//! Rows are read with FINAL: deleting a file writes its rows again as deleted (see the migration).
//!
//! Tenancy: every ClickHouse query filters on `OrgId = {oc_org:String}` through `AppState::ch(org)`.

use std::collections::{BTreeMap, HashMap};
use std::time::Duration;

use axum::extract::State;
use axum::Json;
use chrono::{Datelike, NaiveDate, Utc};
use oc_query::{build_where, parse_query, MetricQuery, Param, Schema, SqlFilter, Term};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use super::logs::{bind, TimeRange};
use crate::error::{ApiError, ApiResult};
use crate::state::{AppState, OrgId};

pub const TABLE: &str = "otel.cloud_costs";
const DAY_MS: i64 = 86_400_000;
/// Groups a query returns at most; the rest are summed into one `Other` group.
const MAX_GROUPS: usize = 200;
/// The FileId of the demo seed's rows.
const DEMO_FILE: &str = "demo";
/// Days of costs the demo seed keeps.
const DEMO_DAYS: i64 = 400;

// ================================================================ the cost metrics

/// Which cost a cost metric reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CostColumn {
    /// Commitments spread over their term (`*.amortized`, `all.cost`).
    Effective,
    /// What the bill charged (`*.unblended`, `*.actual`, `custom.cost.basis`).
    Billed,
}

impl CostColumn {
    pub fn sql(self) -> &'static str {
        match self {
            CostColumn::Effective => "EffectiveCost",
            CostColumn::Billed => "BilledCost",
        }
    }
}

/// Which providers a cost metric covers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CostProviders {
    All,
    /// `aws`, `azure`, `gcp`, `oci`, or a custom provider's lower-cased name.
    One(String),
    /// Everything but the cloud bills (`custom.cost.*`).
    Custom,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CostMetric {
    pub providers: CostProviders,
    pub column: CostColumn,
}

const CLOUDS: &[&str] = &["aws", "azure", "gcp", "oci"];

/// Datadog's cost metric names: `all.cost`, `<provider>.cost.<kind>` (`aws.cost.amortized`,
/// `aws.cost.net.unblended`, `azure.cost.actual`, `gcp.cost.amortized`, `custom.cost.basis`, …).
pub fn cost_metric(name: &str) -> Option<CostMetric> {
    if name == "all.cost" {
        return Some(CostMetric { providers: CostProviders::All, column: CostColumn::Effective });
    }
    let (provider, kind) = name.split_once(".cost.")?;
    if provider.is_empty() || provider.contains('.') {
        return None;
    }
    let column = if kind.split('.').any(|p| p == "amortized") {
        CostColumn::Effective
    } else if ["unblended", "net.unblended", "blended", "actual", "basis", "list"].contains(&kind) {
        CostColumn::Billed
    } else {
        return None;
    };
    let providers = match provider {
        "custom" => CostProviders::Custom,
        p => CostProviders::One(p.to_string()),
    };
    Some(CostMetric { providers, column })
}

impl CostMetric {
    fn filter(&self) -> SqlFilter {
        match &self.providers {
            CostProviders::All => SqlFilter { clause: "1".into(), params: vec![] },
            CostProviders::One(p) => SqlFilter { clause: "lower(ProviderName) = ?".into(), params: vec![Param::Str(p.clone())] },
            CostProviders::Custom => SqlFilter { clause: format!("ProviderName NOT IN ({})", CLOUDS.iter().map(|c| format!("'{c}'")).collect::<Vec<_>>().join(", ")), params: vec![] },
        }
    }
}

/// The FOCUS columns, as the cost tags name them.
const COLUMNS: &[(&str, &str)] = &[
    ("providername", "ProviderName"),
    ("servicename", "ServiceName"),
    ("subaccountname", "SubAccountName"),
    ("regionname", "RegionName"),
    ("availabilityzone", "AvailabilityZone"),
    ("chargecategory", "ChargeCategory"),
    ("chargedescription", "ChargeDescription"),
    ("resourceid", "ResourceId"),
    ("pricingunit", "PricingUnit"),
    ("billingcurrency", "BillingCurrency"),
    ("pricingcategory", "PricingCategory"),
    ("commitmentdiscountid", "CommitmentDiscountId"),
    ("commitmentdiscounttype", "CommitmentDiscountType"),
    ("commitmentdiscountstatus", "CommitmentDiscountStatus"),
];

fn column(key: &str) -> Option<&'static str> {
    let k = key.to_ascii_lowercase().replace('_', "");
    COLUMNS.iter().find(|(name, _)| *name == k).map(|(_, c)| *c)
}

/// A cost tag as SQL: a FOCUS column, else the bill's tag.
pub fn cost_field(key: &str) -> (String, Vec<Param>) {
    match column(key) {
        Some(c) => (c.to_string(), vec![]),
        None => ("Tags[?]".into(), vec![Param::Str(key.to_string())]),
    }
}

/// The search syntax over costs (`providername:aws servicename:(AmazonEC2 OR AmazonRDS) -team:ml`).
struct CostSchema;

impl Schema for CostSchema {
    fn field(&self, key: &str) -> (String, Option<Param>) {
        let (expr, mut params) = cost_field(key);
        (expr, params.pop())
    }
    fn text_expr(&self) -> &'static str {
        "lower(concat(ServiceName, ' ', ChargeDescription, ' ', ProviderName, ' ', SubAccountName))"
    }
}

fn day(ms: i64) -> NaiveDate {
    chrono::DateTime::from_timestamp_millis(ms).unwrap_or_default().date_naive()
}

fn day_ms(d: NaiveDate) -> i64 {
    d.and_hms_opt(0, 0, 0).unwrap_or_default().and_utc().timestamp_millis()
}

/// The days of a range: the UTC days of its ends.
fn days(range: TimeRange) -> (NaiveDate, NaiveDate) {
    (day(range.from), day(range.to.max(range.from)))
}

/// A cost metric query's points, per series and interval, for the metric engine (the shape of its
/// inner queries: `g` group, `t` interval start, `sid` series, `v` value). Costs are daily: the
/// interval is at least a day, and a series' value is what it cost in the interval.
pub fn points_sql(metric: &CostMetric, q: &MetricQuery, range: TimeRange, step_ms: i64) -> (String, Vec<Param>) {
    let (from, to) = days(range);
    let mut params = Vec::new();
    let mut by = Vec::new();
    for k in &q.by {
        let (e, p) = cost_field(k);
        by.push(format!("toString({e})"));
        params.extend(p);
    }
    let group = if by.is_empty() { "CAST([], 'Array(String)')".to_string() } else { format!("[{}]", by.join(", ")) };
    let provider = metric.filter();
    let filter = oc_query::metric_filter(&q.filters, cost_field);
    params.extend([Param::Str(from.to_string()), Param::Str(to.to_string())]);
    params.extend(provider.params);
    params.extend(filter.params);
    let sql = format!(
        "SELECT {group} AS g, toStartOfInterval(toDateTime(ChargeDate, 'UTC'), INTERVAL {step} SECOND) AS t, \
         cityHash64(ProviderName, ServiceName, SubAccountName, RegionName, ChargeCategory, ChargeDescription, ResourceId, Tags) AS sid, sum({col}) AS v \
         FROM {TABLE} FINAL WHERE OrgId = {{oc_org:String}} AND IsDeleted = 0 AND ChargeDate >= toDate(?) AND ChargeDate <= toDate(?) AND {provider} AND {filter} \
         GROUP BY g, t, sid",
        step = step_ms / 1000,
        col = metric.column.sql(),
        provider = provider.clause,
        filter = filter.clause,
    );
    (sql, params)
}

/// The interval of a cost metric query: whole days, about `points` of them over the range.
pub fn step_ms(range: TimeRange, points: u32) -> i64 {
    let raw = (range.to - range.from) / points.max(1) as i64;
    ((raw + DAY_MS - 1) / DAY_MS).max(1) * DAY_MS
}

/// A budget's `metrics_query` (`aws.cost.amortized{service:ec2} by {service}`, no aggregator) as a
/// metric query: costs add up.
pub fn budget_query(text: &str) -> Result<MetricQuery, String> {
    let text = text.trim();
    let full = if text.split('{').next().is_some_and(|head| head.contains(':')) { text.to_string() } else { format!("sum:{text}") };
    let mut q = oc_query::parse_metric_query(&full)?;
    // A quoted value (`providername:"Confluent Cloud"`) is the text between the quotes.
    for f in &mut q.filters {
        if f.value.len() >= 2 && f.value.starts_with('"') && f.value.ends_with('"') {
            f.value = f.value[1..f.value.len() - 1].to_string();
        }
    }
    if cost_metric(&q.metric).is_none() {
        return Err(format!("'{}' is not a cost metric (all.cost, aws.cost.amortized, azure.cost.amortized, gcp.cost.amortized, custom.cost.basis, …)", q.metric));
    }
    Ok(q)
}

#[derive(clickhouse::Row, Deserialize)]
struct MonthRow {
    m: i64,
    g: Vec<String>,
    cost: f64,
}

/// What a cost query cost per month (`YYYYMM`) and group of its `by` tags, from `start` to `end`
/// (months, inclusive): budgets compare it with their amounts.
pub async fn monthly_costs(st: &AppState, org: &str, q: &MetricQuery, start: i64, end: i64) -> Result<Vec<(i64, Vec<String>, f64)>, ApiError> {
    let metric = cost_metric(&q.metric).ok_or_else(|| ApiError::BadRequest(format!("'{}' is not a cost metric", q.metric)))?;
    let month = |m: i64| NaiveDate::from_ymd_opt((m / 100) as i32, (m % 100) as u32, 1).ok_or_else(|| ApiError::BadRequest(format!("invalid month {m}")));
    let from = month(start)?;
    let to = month(end)?.checked_add_months(chrono::Months::new(1)).unwrap_or_default() - chrono::Duration::days(1);
    let mut params = Vec::new();
    let mut by = Vec::new();
    for k in &q.by {
        let (e, p) = cost_field(k);
        by.push(format!("toString({e})"));
        params.extend(p);
    }
    let group = if by.is_empty() { "CAST([], 'Array(String)')".to_string() } else { format!("[{}]", by.join(", ")) };
    let provider = metric.filter();
    let filter = oc_query::metric_filter(&q.filters, cost_field);
    params.extend([Param::Str(from.to_string()), Param::Str(to.to_string())]);
    params.extend(provider.params);
    params.extend(filter.params);
    let sql = format!(
        "SELECT toInt64(toYYYYMM(ChargeDate)) AS m, {group} AS g, sum({col}) AS cost FROM {TABLE} FINAL \
         WHERE OrgId = {{oc_org:String}} AND IsDeleted = 0 AND ChargeDate >= toDate(?) AND ChargeDate <= toDate(?) AND {p} AND {f} GROUP BY m, g",
        col = metric.column.sql(),
        p = provider.clause,
        f = filter.clause,
    );
    let rows: Vec<MonthRow> = bind(st.ch(org).query(&sql), &params).fetch_all().await?;
    Ok(rows.into_iter().map(|r| (r.m, r.g, round(r.cost))).collect())
}

// ================================================================ writing line items

/// One cost line item and day, as stored.
#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct CostLine {
    #[serde(rename = "OrgId")]
    pub org: String,
    #[serde(rename = "FileId")]
    pub file_id: String,
    #[serde(rename = "LineNo")]
    pub line_no: u32,
    #[serde(rename = "ChargeDate")]
    pub date: String,
    #[serde(rename = "ProviderName")]
    pub provider: String,
    #[serde(rename = "ServiceName")]
    pub service: String,
    #[serde(rename = "SubAccountName")]
    pub sub_account: String,
    #[serde(rename = "RegionName")]
    pub region: String,
    #[serde(rename = "AvailabilityZone")]
    pub zone: String,
    #[serde(rename = "ChargeCategory")]
    pub category: String,
    #[serde(rename = "ChargeDescription")]
    pub description: String,
    #[serde(rename = "ResourceId")]
    pub resource_id: String,
    #[serde(rename = "PricingUnit")]
    pub pricing_unit: String,
    #[serde(rename = "UsageQuantity")]
    pub quantity: f64,
    #[serde(rename = "BilledCost")]
    pub billed: f64,
    #[serde(rename = "EffectiveCost")]
    pub effective: f64,
    #[serde(rename = "ListCost")]
    pub list: f64,
    #[serde(rename = "PricingCategory")]
    pub pricing_category: String,
    #[serde(rename = "CommitmentDiscountId")]
    pub commitment_id: String,
    #[serde(rename = "CommitmentDiscountType")]
    pub commitment_type: String,
    #[serde(rename = "CommitmentDiscountStatus")]
    pub commitment_status: String,
    #[serde(rename = "BillingCurrency")]
    pub currency: String,
    #[serde(rename = "Tags")]
    pub tags: BTreeMap<String, String>,
    #[serde(rename = "Source")]
    pub source: String,
    #[serde(rename = "Version")]
    pub version: u64,
    #[serde(rename = "IsDeleted")]
    pub deleted: u8,
}

pub async fn insert(st: &AppState, lines: &[CostLine]) -> Result<(), ApiError> {
    for chunk in lines.chunks(20_000) {
        let mut ndjson = Vec::new();
        for l in chunk {
            serde_json::to_writer(&mut ndjson, l).map_err(anyhow::Error::from)?;
            ndjson.push(b'\n');
        }
        crate::logproc::insert_rows(st, TABLE, ndjson).await.map_err(|e| ApiError::Internal(anyhow::anyhow!("writing cost line items failed: {e}")))?;
    }
    Ok(())
}

/// Removes a file's line items: writes them again, deleted, with a newer version.
pub async fn delete_file(st: &AppState, org: &str, file_id: &str) -> Result<(), ApiError> {
    let version = Utc::now().timestamp_millis() as u64;
    let sql = format!(
        "INSERT INTO {TABLE} SELECT * REPLACE (toUInt64(?) AS Version, toUInt8(1) AS IsDeleted) FROM {TABLE} FINAL WHERE OrgId = {{oc_org:String}} AND FileId = ? AND IsDeleted = 0"
    );
    st.ch(org).query(&sql).bind(version).bind(file_id).execute().await?;
    Ok(())
}

#[derive(clickhouse::Row, Deserialize)]
pub struct StoredLine {
    #[serde(rename = "ChargeDate")]
    pub date: String,
    #[serde(rename = "ProviderName")]
    pub provider: String,
    #[serde(rename = "ChargeDescription")]
    pub description: String,
    #[serde(rename = "BilledCost")]
    pub billed: f64,
    #[serde(rename = "BillingCurrency")]
    pub currency: String,
    #[serde(rename = "Tags")]
    pub tags: Vec<(String, String)>,
    #[serde(rename = "LineNo")]
    pub line_no: u32,
}

/// A file's stored line items (one per day), in upload order.
pub async fn file_lines(st: &AppState, org: &str, file_id: &str) -> Result<Vec<StoredLine>, ApiError> {
    let sql = format!(
        "SELECT toString(ChargeDate) AS ChargeDate, ProviderName, ChargeDescription, BilledCost, BillingCurrency, CAST(Tags, 'Array(Tuple(String, String))') AS Tags, LineNo \
         FROM {TABLE} FINAL WHERE OrgId = {{oc_org:String}} AND FileId = ? AND IsDeleted = 0 ORDER BY LineNo"
    );
    Ok(st.ch(org).query(&sql).bind(file_id).fetch_all::<StoredLine>().await?)
}

// ================================================================ the Cloud Cost pages

#[derive(Deserialize, ToSchema, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum CostInterval {
    #[default]
    Day,
    Week,
    Month,
}

#[derive(Deserialize, ToSchema, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum CostType {
    #[default]
    Amortized,
    Billed,
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct CostQueryRequest {
    /// Search syntax over the cost tags (`providername:aws team:checkout`).
    #[serde(default)]
    pub query: String,
    #[serde(default)]
    pub cost_type: CostType,
    /// Only usage charges: no purchases, taxes, credits or adjustments.
    #[serde(default)]
    pub usage_only: bool,
    #[serde(default)]
    pub group_by: Vec<String>,
    pub range: TimeRange,
    #[serde(default)]
    pub interval: CostInterval,
    /// Groups returned (largest first); the others are summed into `Other`.
    pub limit: Option<usize>,
}

#[derive(Serialize, ToSchema, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CostSeries {
    /// The group's tag values ("N/A" when the cost has no such tag); `{"other": "Other"}` for the rest.
    pub tags: BTreeMap<String, String>,
    /// One value per bucket.
    pub values: Vec<f64>,
    pub total: f64,
    /// What the group cost over the previous period of the same length.
    pub previous_total: f64,
}

#[derive(Serialize, ToSchema, Debug)]
#[serde(rename_all = "camelCase")]
pub struct CostQueryResult {
    /// Interval starts (epoch ms, UTC midnight).
    pub buckets: Vec<i64>,
    pub series: Vec<CostSeries>,
    pub total: f64,
    pub previous_total: f64,
    /// The days compared with: the same number of days just before the range.
    pub previous_range: TimeRange,
    /// The last day with costs (epoch ms), if any: later days aren't billed yet.
    pub data_through: Option<i64>,
}

fn bucket_start(d: NaiveDate, interval: CostInterval) -> NaiveDate {
    match interval {
        CostInterval::Day => d,
        CostInterval::Week => d - chrono::Duration::days(d.weekday().num_days_from_monday() as i64),
        CostInterval::Month => d.with_day(1).unwrap_or(d),
    }
}

fn bucket_sql(interval: CostInterval) -> &'static str {
    match interval {
        CostInterval::Day => "ChargeDate",
        CostInterval::Week => "toMonday(ChargeDate)",
        CostInterval::Month => "toStartOfMonth(ChargeDate)",
    }
}

fn next_bucket(d: NaiveDate, interval: CostInterval) -> NaiveDate {
    match interval {
        CostInterval::Day => d + chrono::Duration::days(1),
        CostInterval::Week => d + chrono::Duration::days(7),
        CostInterval::Month => d.checked_add_months(chrono::Months::new(1)).unwrap_or(d),
    }
}

/// The previous period: as many days, just before.
fn previous(from: NaiveDate, to: NaiveDate) -> (NaiveDate, NaiveDate) {
    let len = (to - from).num_days() + 1;
    let prev_to = from - chrono::Duration::days(1);
    (prev_to - chrono::Duration::days(len - 1), prev_to)
}

/// The search over costs as SQL. `key:*` means the cost has the tag (a missing tag reads as '').
fn cost_where(query: &str) -> SqlFilter {
    let (exists, terms): (Vec<Term>, Vec<Term>) = parse_query(query).into_iter().partition(|t| matches!(t, Term::Tag { values, .. } if !values.is_empty() && values.iter().all(|v| v.chars().all(|c| c == '*'))));
    let mut f = build_where(&CostSchema, &terms, None);
    for t in exists {
        if let Term::Tag { key, negate, .. } = t {
            let (expr, params) = cost_field(&key);
            f.clause = format!("({}) AND {}notEmpty({expr})", f.clause, if negate { "NOT " } else { "" });
            f.params.extend(params);
        }
    }
    f
}

fn where_clause(req: &CostQueryRequest) -> SqlFilter {
    let mut f = cost_where(&req.query);
    if req.usage_only {
        f.clause = format!("({}) AND lower(ChargeCategory) = 'usage'", f.clause);
    }
    f
}

#[derive(clickhouse::Row, Deserialize)]
struct BucketRow {
    g: Vec<String>,
    t: i64,
    cost: f64,
}

#[derive(clickhouse::Row, Deserialize)]
struct TotalRow {
    g: Vec<String>,
    cost: f64,
}

/// Costs per group and interval over the range, with each group's cost over the previous period.
#[utoipa::path(post, path = "/api/v1/cloud-cost/query", operation_id = "query_cloud_cost", request_body = CostQueryRequest,
    responses((status = 200, body = CostQueryResult)), tag = "cloud-cost")]
pub async fn query(State(st): State<AppState>, OrgId(org): OrgId, Json(req): Json<CostQueryRequest>) -> ApiResult<CostQueryResult> {
    Ok(Json(run_query(&st, &org, &req).await?))
}

pub async fn run_query(st: &AppState, org: &str, req: &CostQueryRequest) -> Result<CostQueryResult, ApiError> {
    if req.group_by.len() > 4 {
        return Err(ApiError::BadRequest("group by at most 4 tags".into()));
    }
    let (from, to) = days(req.range);
    if (to - from).num_days() > 3 * 366 {
        return Err(ApiError::BadRequest("the range can't exceed 3 years".into()));
    }
    let (prev_from, prev_to) = previous(from, to);
    let mut group_params = Vec::new();
    let mut by = Vec::new();
    for k in &req.group_by {
        let (e, p) = cost_field(k);
        by.push(format!("toString({e})"));
        group_params.extend(p);
    }
    let group = if by.is_empty() { "CAST([], 'Array(String)')".to_string() } else { format!("[{}]", by.join(", ")) };
    let filter = where_clause(req);
    let col = match req.cost_type {
        CostType::Amortized => "EffectiveCost",
        CostType::Billed => "BilledCost",
    };
    let ch = st.ch(org);
    let range_sql = |b: &str| {
        format!(
            "SELECT {group} AS g, {b} FROM {TABLE} FINAL WHERE OrgId = {{oc_org:String}} AND IsDeleted = 0 AND ChargeDate >= toDate(?) AND ChargeDate <= toDate(?) AND {}",
            filter.clause
        )
    };
    let params_for = |a: NaiveDate, b: NaiveDate| {
        let mut p = group_params.clone();
        p.extend([Param::Str(a.to_string()), Param::Str(b.to_string())]);
        p.extend(filter.params.clone());
        p
    };
    let sql = format!(
        "{} GROUP BY g, t",
        range_sql(&format!("toInt64(toUnixTimestamp(toDateTime({}, 'UTC'))) * 1000 AS t, sum({col}) AS cost", bucket_sql(req.interval)))
    );
    let rows: Vec<BucketRow> = bind(ch.query(&sql), &params_for(from, to)).fetch_all().await?;
    let sql = format!("{} GROUP BY g", range_sql(&format!("sum({col}) AS cost")));
    let prev: Vec<TotalRow> = bind(ch.query(&sql), &params_for(prev_from, prev_to)).fetch_all().await?;
    let through: i64 = bind(
        ch.query(&format!(
            "SELECT toInt64(toUnixTimestamp(toDateTime(max(ChargeDate), 'UTC'))) * 1000 FROM {TABLE} WHERE OrgId = {{oc_org:String}} AND IsDeleted = 0 AND ChargeDate >= toDate(?)"
        )),
        &[Param::Str((to - chrono::Duration::days(62)).to_string())],
    )
    .fetch_one()
    .await?;

    let mut buckets = Vec::new();
    let mut b = bucket_start(from, req.interval);
    while b <= to {
        buckets.push(day_ms(b));
        b = next_bucket(b, req.interval);
    }
    let index: HashMap<i64, usize> = buckets.iter().enumerate().map(|(i, t)| (*t, i)).collect();
    let previous_by: HashMap<Vec<String>, f64> = prev.into_iter().map(|r| (r.g, r.cost)).collect();
    Ok(assemble(&req.group_by, buckets.clone(), &index, rows, &previous_by, req.limit.unwrap_or(MAX_GROUPS).min(MAX_GROUPS), from, to, (through > 0).then_some(through)))
}

#[allow(clippy::too_many_arguments)]
fn assemble(
    group_by: &[String],
    buckets: Vec<i64>,
    index: &HashMap<i64, usize>,
    rows: Vec<BucketRow>,
    previous_by: &HashMap<Vec<String>, f64>,
    limit: usize,
    from: NaiveDate,
    to: NaiveDate,
    data_through: Option<i64>,
) -> CostQueryResult {
    let mut groups: HashMap<Vec<String>, Vec<f64>> = HashMap::new();
    for r in rows {
        let values = groups.entry(r.g).or_insert_with(|| vec![0.0; buckets.len()]);
        if let Some(i) = index.get(&r.t) {
            values[*i] += r.cost;
        }
    }
    // Groups only seen before still show what they cost then.
    for g in previous_by.keys() {
        groups.entry(g.clone()).or_insert_with(|| vec![0.0; buckets.len()]);
    }
    let tags = |g: &[String]| -> BTreeMap<String, String> { group_by.iter().cloned().zip(g.iter().map(|v| if v.is_empty() { "N/A".to_string() } else { v.clone() })).collect() };
    let mut series: Vec<CostSeries> = groups
        .into_iter()
        .map(|(g, values)| CostSeries { tags: tags(&g), total: round(values.iter().sum()), previous_total: round(previous_by.get(&g).copied().unwrap_or(0.0)), values: values.into_iter().map(round).collect() })
        .collect();
    series.sort_by(|a, b| b.total.abs().total_cmp(&a.total.abs()).then(b.previous_total.abs().total_cmp(&a.previous_total.abs())).then(a.tags.cmp(&b.tags)));
    let total = round(series.iter().map(|s| s.total).sum());
    let previous_total = round(series.iter().map(|s| s.previous_total).sum());
    if series.len() > limit {
        let rest = series.split_off(limit);
        let mut values = vec![0.0; buckets.len()];
        for s in &rest {
            for (v, x) in values.iter_mut().zip(&s.values) {
                *v += x;
            }
        }
        series.push(CostSeries {
            tags: BTreeMap::from([("other".to_string(), "Other".to_string())]),
            total: round(rest.iter().map(|s| s.total).sum()),
            previous_total: round(rest.iter().map(|s| s.previous_total).sum()),
            values: values.into_iter().map(round).collect(),
        });
    }
    let (pf, pt) = previous(from, to);
    CostQueryResult { buckets, series, total, previous_total, previous_range: TimeRange { from: day_ms(pf), to: day_ms(pt) + DAY_MS - 1 }, data_through }
}

fn round(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct CostTagsRequest {
    /// Search syntax narrowing the costs (`providername:azure`).
    #[serde(default)]
    pub query: String,
    pub range: TimeRange,
    #[serde(default)]
    pub cost_type: CostType,
}

#[derive(Serialize, ToSchema, Debug)]
#[serde(rename_all = "camelCase")]
pub struct CostTagValue {
    pub value: String,
    pub cost: f64,
}

#[derive(Serialize, ToSchema, Debug)]
#[serde(rename_all = "camelCase")]
pub struct CostTag {
    pub key: String,
    /// `focus` (the bill's standard columns), `aws` / `azure` / `gcp` (provider tags), `kubernetes`, or
    /// `resource` (tags on the billed resources).
    pub source: String,
    /// Its values, costliest first (at most 50).
    pub values: Vec<CostTagValue>,
    pub distinct_values: u64,
    /// Share of the cost (0–1) that has the tag.
    pub coverage: f64,
    /// Sub-accounts whose costs have the tag.
    pub accounts: u64,
}

fn tag_source(key: &str) -> &'static str {
    if column(key).is_some() {
        "focus"
    } else if key.starts_with("aws_") {
        "aws"
    } else if key.starts_with("azure_") || key == "metersubcategory" || key == "metercategory" {
        "azure"
    } else if key.starts_with("gcp_") {
        "gcp"
    } else if key.starts_with("kube_") || key.starts_with("pod_") {
        "kubernetes"
    } else {
        "resource"
    }
}

#[derive(clickhouse::Row, Deserialize)]
struct TagRow {
    k: String,
    distinct: u64,
    covered: f64,
    accounts: u64,
    top: Vec<(String, f64)>,
}

/// The tags costs can be broken down by, with their values, coverage and sources.
#[utoipa::path(post, path = "/api/v1/cloud-cost/tags", operation_id = "cloud_cost_tags", request_body = CostTagsRequest,
    responses((status = 200, body = Vec<CostTag>)), tag = "cloud-cost")]
pub async fn tags(State(st): State<AppState>, OrgId(org): OrgId, Json(req): Json<CostTagsRequest>) -> ApiResult<Vec<CostTag>> {
    let (from, to) = days(req.range);
    let f = cost_where(&req.query);
    let col = match req.cost_type {
        CostType::Amortized => "EffectiveCost",
        CostType::Billed => "BilledCost",
    };
    let columns = COLUMNS.iter().map(|(k, c)| format!("('{k}', toString({c}))")).collect::<Vec<_>>().join(", ");
    let base = format!("FROM {TABLE} FINAL WHERE OrgId = {{oc_org:String}} AND IsDeleted = 0 AND ChargeDate >= toDate(?) AND ChargeDate <= toDate(?) AND {}", f.clause);
    let mut params = vec![Param::Str(from.to_string()), Param::Str(to.to_string())];
    params.extend(f.params.clone());
    let sql = format!(
        "SELECT k, uniqExact(v) AS distinct, sum(c) AS covered, toUInt64(length(arrayDistinct(arrayFlatten(groupArray(accts))))) AS accounts, \
         arraySlice(arraySort(x -> -x.2, groupArray((v, c))), 1, 50) AS top \
         FROM (SELECT kv.1 AS k, kv.2 AS v, sum(cost) AS c, groupUniqArray(acct) AS accts FROM ( \
             SELECT arrayJoin(arrayConcat([{columns}], arrayZip(mapKeys(Tags), mapValues(Tags)))) AS kv, {col} AS cost, SubAccountName AS acct {base}) \
           WHERE kv.2 != '' GROUP BY k, v) GROUP BY k ORDER BY k"
    );
    let ch = st.ch(&org);
    let rows: Vec<TagRow> = bind(ch.query(&sql), &params).fetch_all().await?;
    let total: f64 = bind(ch.query(&format!("SELECT toFloat64(sum({col})) {base}")), &params).fetch_one().await?;
    Ok(Json(
        rows.into_iter()
            .map(|r| CostTag {
                source: tag_source(&r.k).to_string(),
                coverage: if total.abs() > f64::EPSILON { (r.covered / total).clamp(0.0, 1.0) } else { 0.0 },
                values: r.top.into_iter().map(|(value, cost)| CostTagValue { value, cost: round(cost) }).collect(),
                distinct_values: r.distinct,
                accounts: r.accounts,
                key: r.k,
            })
            .collect(),
    ))
}

// ================================================================ unit costs

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct UnitCostsRequest {
    /// Search syntax narrowing the usage (`providername:azure metercategory:"Virtual Machines"`).
    #[serde(default)]
    pub query: String,
    pub range: TimeRange,
    /// The tags a row is (`metername`, `regionname`, `pricingunit`, …); at most 6.
    pub group_by: Vec<String>,
}

#[derive(Serialize, ToSchema, Debug)]
#[serde(rename_all = "camelCase")]
pub struct UnitCostRow {
    /// The row's tag values ("N/A" when the usage has no such tag).
    pub tags: BTreeMap<String, String>,
    /// Units used (in the row's pricing unit).
    pub usage_quantity: f64,
    /// What the usage cost, commitments amortized.
    pub amortized_cost: f64,
    /// What it would have cost at on-demand prices.
    pub on_demand_cost: f64,
}

#[derive(clickhouse::Row, Deserialize)]
struct UnitRow {
    g: Vec<String>,
    q: f64,
    e: f64,
    l: f64,
}

/// Unit prices: per group of tags, the usage and what it cost amortized and on demand (Plan → Unit
/// Costs divides them). Only usage with a quantity; unused commitments are left out.
#[utoipa::path(post, path = "/api/v1/cloud-cost/unit-costs", operation_id = "cloud_cost_unit_costs", request_body = UnitCostsRequest,
    responses((status = 200, body = Vec<UnitCostRow>)), tag = "cloud-cost")]
pub async fn unit_costs(State(st): State<AppState>, OrgId(org): OrgId, Json(req): Json<UnitCostsRequest>) -> ApiResult<Vec<UnitCostRow>> {
    if req.group_by.is_empty() || req.group_by.len() > 6 {
        return Err(ApiError::BadRequest("group by 1 to 6 tags".into()));
    }
    let (from, to) = days(req.range);
    let mut params = Vec::new();
    let mut by = Vec::new();
    for k in &req.group_by {
        let (e, p) = cost_field(k);
        by.push(format!("toString({e})"));
        params.extend(p);
    }
    let f = cost_where(&req.query);
    params.extend([Param::Str(from.to_string()), Param::Str(to.to_string())]);
    params.extend(f.params);
    let sql = format!(
        "SELECT [{by}] AS g, sum(UsageQuantity) AS q, sum(EffectiveCost) AS e, sum(ListCost) AS l FROM {TABLE} FINAL \
         WHERE OrgId = {{oc_org:String}} AND IsDeleted = 0 AND ChargeDate >= toDate(?) AND ChargeDate <= toDate(?) AND lower(ChargeCategory) = 'usage' \
         AND CommitmentDiscountStatus != 'Unused' AND UsageQuantity > 0 AND {} GROUP BY g ORDER BY e DESC LIMIT 500",
        f.clause,
        by = by.join(", "),
    );
    let rows: Vec<UnitRow> = bind(st.ch(&org).query(&sql), &params).fetch_all().await?;
    Ok(Json(
        rows.into_iter()
            .map(|r| UnitCostRow {
                tags: req.group_by.iter().cloned().zip(r.g.into_iter().map(|v| if v.is_empty() { "N/A".to_string() } else { v })).collect(),
                usage_quantity: r.q,
                amortized_cost: round(r.e),
                on_demand_cost: round(r.l),
            })
            .collect(),
    ))
}

// ================================================================ commitments

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct CommitmentsRequest {
    /// `aws`, `azure` or `gcp`.
    pub provider: String,
    /// Search syntax narrowing the costs (`subaccountname:ecommerce-eu`).
    #[serde(default)]
    pub query: String,
    pub range: TimeRange,
}

/// A commitment type's daily figures.
#[derive(Serialize, ToSchema, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct CommitmentTypeSeries {
    /// `Savings Plan`, `Reserved Instance`, `Pre-purchase Plan`, `Committed Use Discount`, or `On-Demand`.
    pub kind: String,
    /// On-demand value of the usage it covered (for `On-Demand`: the uncovered usage), per day.
    pub values: Vec<f64>,
}

#[derive(Serialize, ToSchema, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct CommitmentUtilization {
    pub kind: String,
    /// Amortized cost of the used and unused parts.
    pub used: f64,
    pub unused: f64,
}

#[derive(Serialize, ToSchema, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct Commitment {
    pub id: String,
    pub kind: String,
    pub account: String,
    pub region: String,
    pub service: String,
    pub term: String,
    pub purchase_option: String,
    pub start: String,
    pub end: String,
    /// Amortized cost over the range (used + unused).
    pub amortized_cost: f64,
    pub unused_cost: f64,
    /// Share used (0–1).
    pub utilization: f64,
    /// On-demand value of what it covered minus what it cost.
    pub savings: f64,
}

/// Coverage of one service and region.
#[derive(Serialize, ToSchema, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct CoverageRow {
    pub service: String,
    pub region: String,
    /// On-demand value of the covered usage, and what the uncovered usage cost.
    pub covered: f64,
    pub on_demand: f64,
}

#[derive(Serialize, ToSchema, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct CommitmentsResult {
    /// Days (epoch ms, UTC midnight).
    pub buckets: Vec<i64>,
    /// Amortized cost of the commitments (used and unused), per day.
    pub commitment_cost: Vec<f64>,
    /// What the covered usage would have cost on demand, per day.
    pub on_demand_equivalent: Vec<f64>,
    /// Effective savings rate (0–1) per day, when there was eligible usage.
    pub savings_rate: Vec<Option<f64>>,
    /// Over the range: 1 − what the eligible usage cost / what it would have cost on demand.
    pub effective_savings_rate: Option<f64>,
    pub realized_savings: f64,
    /// Share (0–1) of the eligible usage (services commitments cover) that commitments covered.
    pub coverage: f64,
    /// What the uncovered eligible usage cost.
    pub on_demand_spend: f64,
    pub coverage_by_type: Vec<CommitmentTypeSeries>,
    pub coverage_by_service: Vec<CoverageRow>,
    pub utilization: Vec<CommitmentUtilization>,
    pub commitments: Vec<Commitment>,
}

#[derive(clickhouse::Row, Deserialize)]
struct CommitRow {
    t: i64,
    service: String,
    region: String,
    kind: String,
    id: String,
    status: String,
    committed: u8,
    e: f64,
    l: f64,
}

#[derive(clickhouse::Row, Deserialize, Default)]
struct CommitMeta {
    id: String,
    kind: String,
    account: String,
    region: String,
    service: String,
    term: String,
    option: String,
    start: String,
    end: String,
}

/// Commitment programs of a provider: savings, coverage, utilization and the commitments.
#[utoipa::path(post, path = "/api/v1/cloud-cost/commitments", operation_id = "cloud_cost_commitments", request_body = CommitmentsRequest,
    responses((status = 200, body = CommitmentsResult)), tag = "cloud-cost")]
pub async fn commitments(State(st): State<AppState>, OrgId(org): OrgId, Json(req): Json<CommitmentsRequest>) -> ApiResult<CommitmentsResult> {
    let (from, to) = days(req.range);
    if (to - from).num_days() > 3 * 366 {
        return Err(ApiError::BadRequest("the range can't exceed 3 years".into()));
    }
    let f = cost_where(&req.query);
    let mut params = vec![Param::Str(from.to_string()), Param::Str(to.to_string()), Param::Str(req.provider.to_ascii_lowercase())];
    params.extend(f.params);
    let base = format!(
        "FROM {TABLE} FINAL WHERE OrgId = {{oc_org:String}} AND IsDeleted = 0 AND ChargeDate >= toDate(?) AND ChargeDate <= toDate(?) AND lower(ProviderName) = ? \
         AND lower(ChargeCategory) = 'usage' AND {}",
        f.clause
    );
    let ch = st.ch(&org);
    let sql = format!(
        "SELECT toInt64(toUnixTimestamp(toDateTime(ChargeDate, 'UTC'))) * 1000 AS t, toString(ServiceName) AS service, toString(RegionName) AS region, \
         toString(CommitmentDiscountType) AS kind, CommitmentDiscountId AS id, toString(CommitmentDiscountStatus) AS status, \
         toUInt8(PricingCategory = 'Committed' OR CommitmentDiscountId != '') AS committed, sum(EffectiveCost) AS e, sum(ListCost) AS l \
         {base} GROUP BY t, service, region, kind, id, status, committed"
    );
    let rows: Vec<CommitRow> = bind(ch.query(&sql), &params).fetch_all().await?;
    let sql = format!(
        "SELECT CommitmentDiscountId AS id, any(toString(CommitmentDiscountType)) AS kind, any(toString(SubAccountName)) AS account, any(toString(RegionName)) AS region, \
         any(toString(ServiceName)) AS service, argMax(Tags['commitment_term'], ChargeDate) AS term, argMax(Tags['commitment_purchase_option'], ChargeDate) AS option, \
         argMax(Tags['commitment_start'], ChargeDate) AS start, argMax(Tags['commitment_end'], ChargeDate) AS end {base} AND CommitmentDiscountId != '' GROUP BY id ORDER BY id"
    );
    let meta: Vec<CommitMeta> = bind(ch.query(&sql), &params).fetch_all().await?;
    let mut buckets = Vec::new();
    let mut d = from;
    while d <= to {
        buckets.push(day_ms(d));
        d += chrono::Duration::days(1);
    }
    Ok(Json(assemble_commitments(buckets, rows, meta)))
}

fn assemble_commitments(buckets: Vec<i64>, rows: Vec<CommitRow>, meta: Vec<CommitMeta>) -> CommitmentsResult {
    let index: HashMap<i64, usize> = buckets.iter().enumerate().map(|(i, t)| (*t, i)).collect();
    let n = buckets.len();
    // Eligible usage: the services any commitment covered.
    let eligible: std::collections::HashSet<&str> = rows.iter().filter(|r| r.committed == 1).map(|r| r.service.as_str()).collect();
    let mut commitment_cost = vec![0.0; n];
    let mut equivalent = vec![0.0; n];
    let mut eligible_cost = vec![0.0; n];
    let mut eligible_list = vec![0.0; n];
    let mut by_type: BTreeMap<String, Vec<f64>> = BTreeMap::new();
    let mut on_demand = vec![0.0; n];
    let mut by_service: BTreeMap<(String, String), (f64, f64)> = BTreeMap::new();
    let mut util: BTreeMap<String, (f64, f64)> = BTreeMap::new();
    let mut per_commitment: HashMap<String, (f64, f64, f64)> = HashMap::new();
    for r in &rows {
        let Some(&i) = index.get(&r.t) else { continue };
        if !eligible.contains(r.service.as_str()) {
            continue;
        }
        eligible_cost[i] += r.e;
        let unused = r.status.eq_ignore_ascii_case("unused");
        if r.committed == 1 {
            commitment_cost[i] += r.e;
            let kind = if r.kind.is_empty() { "Commitment".to_string() } else { r.kind.clone() };
            let u = util.entry(kind.clone()).or_default();
            let c = per_commitment.entry(r.id.clone()).or_default();
            if unused {
                u.1 += r.e;
                c.1 += r.e;
            } else {
                u.0 += r.e;
                c.0 += r.e;
                c.2 += r.l;
                equivalent[i] += r.l;
                eligible_list[i] += r.l;
                by_type.entry(kind).or_insert_with(|| vec![0.0; n])[i] += r.l;
                by_service.entry((r.service.clone(), r.region.clone())).or_default().0 += r.l;
            }
        } else {
            on_demand[i] += r.e;
            eligible_list[i] += r.l;
            by_service.entry((r.service.clone(), r.region.clone())).or_default().1 += r.e;
        }
    }
    let rate = |cost: f64, list: f64| (list > 0.0).then(|| ((1.0 - cost / list) * 10_000.0).round() / 10_000.0);
    let savings_rate = (0..n).map(|i| rate(eligible_cost[i], eligible_list[i])).collect();
    let total_cost: f64 = eligible_cost.iter().sum();
    let total_list: f64 = eligible_list.iter().sum();
    let covered: f64 = equivalent.iter().sum();
    let on_demand_spend: f64 = on_demand.iter().sum();
    let mut coverage_by_type: Vec<CommitmentTypeSeries> = by_type.into_iter().map(|(kind, values)| CommitmentTypeSeries { kind, values: values.into_iter().map(round).collect() }).collect();
    coverage_by_type.push(CommitmentTypeSeries { kind: "On-Demand".into(), values: on_demand.iter().copied().map(round).collect() });
    let mut coverage_by_service: Vec<CoverageRow> = by_service.into_iter().map(|((service, region), (c, o))| CoverageRow { service, region, covered: round(c), on_demand: round(o) }).collect();
    coverage_by_service.sort_by(|a, b| (b.covered + b.on_demand).total_cmp(&(a.covered + a.on_demand)));
    let commitments = meta
        .into_iter()
        .map(|m| {
            let (used, unused, list) = per_commitment.get(&m.id).copied().unwrap_or_default();
            Commitment {
                amortized_cost: round(used + unused),
                unused_cost: round(unused),
                utilization: if used + unused > 0.0 { used / (used + unused) } else { 0.0 },
                savings: round(list - used - unused),
                id: m.id,
                kind: m.kind,
                account: m.account,
                region: m.region,
                service: m.service,
                term: m.term,
                purchase_option: m.option,
                start: m.start,
                end: m.end,
            }
        })
        .collect::<Vec<_>>();
    CommitmentsResult {
        buckets,
        commitment_cost: commitment_cost.into_iter().map(round).collect(),
        on_demand_equivalent: equivalent.into_iter().map(round).collect(),
        savings_rate,
        effective_savings_rate: rate(total_cost, total_list),
        realized_savings: round(commitments.iter().map(|c| c.savings).sum()),
        coverage: if covered + on_demand_spend > 0.0 { covered / (covered + on_demand_spend) } else { 0.0 },
        on_demand_spend: round(on_demand_spend),
        coverage_by_type,
        coverage_by_service,
        utilization: util.into_iter().map(|(kind, (used, unused))| CommitmentUtilization { kind, used: round(used), unused: round(unused) }).collect(),
        commitments,
    }
}

// ================================================================ demo costs

/// A line of the fictional bills: (provider, service, sub-account, region, daily cost, weekend
/// factor, description, tags).
type DemoLine = (&'static str, &'static str, &'static str, &'static str, f64, f64, &'static str, &'static [(&'static str, &'static str)]);

const DEMO_LINES: &[DemoLine] = &[
    ("aws", "AmazonEC2", "ecommerce-prod", "us-east-1", 1400.0, 1.0, "EC2 instance usage", &[("aws_product", "ec2"), ("aws_instance_family", "m6i"), ("team", "platform"), ("service", "storefront-api"), ("env", "prod")]),
    ("aws", "AmazonEC2", "ecommerce-prod", "us-east-1", 420.0, 1.0, "EC2 GPU instance usage", &[("aws_product", "ec2"), ("aws_instance_family", "g5"), ("gpu_type", "a10g"), ("team", "ml"), ("service", "catalog-service"), ("env", "prod")]),
    ("aws", "AmazonEC2", "ecommerce-staging", "us-east-1", 380.0, 0.55, "EC2 instance usage", &[("aws_product", "ec2"), ("aws_instance_family", "m6i"), ("team", "platform"), ("service", "storefront-api"), ("env", "staging")]),
    ("aws", "AmazonRDS", "ecommerce-prod", "us-east-1", 620.0, 1.0, "RDS database instances", &[("aws_product", "rds"), ("team", "checkout"), ("service", "order-service"), ("env", "prod")]),
    ("aws", "AmazonS3", "ecommerce-prod", "us-east-1", 210.0, 1.0, "S3 storage and requests", &[("aws_product", "s3"), ("storage_class", "standard"), ("team", "platform"), ("service", "catalog-service"), ("env", "prod")]),
    ("aws", "AmazonS3", "ecommerce-prod", "us-east-1", 45.0, 1.0, "S3 Glacier storage", &[("aws_product", "s3"), ("storage_class", "glacier"), ("team", "data"), ("service", "billing"), ("env", "prod")]),
    ("aws", "AWSDataTransfer", "ecommerce-prod", "us-east-1", 70.0, 0.9, "Data transfer between availability zones", &[("aws_product", "datatransfer"), ("data_transfer_type", "cross-zone"), ("team", "platform"), ("env", "prod")]),
    ("aws", "AWSDataTransfer", "ecommerce-prod", "us-east-1", 40.0, 0.9, "Data transfer between regions", &[("aws_product", "datatransfer"), ("data_transfer_type", "cross-region"), ("team", "platform"), ("env", "prod")]),
    ("aws", "AWSDataTransfer", "ecommerce-prod", "us-east-1", 48.0, 0.8, "Data transfer out to the internet", &[("aws_product", "datatransfer"), ("data_transfer_type", "internet"), ("team", "storefront"), ("service", "api-gateway"), ("env", "prod")]),
    ("aws", "ElasticLoadBalancing", "ecommerce-prod", "us-east-1", 85.0, 1.0, "Load balancer hours and capacity units", &[("aws_product", "elb"), ("team", "platform"), ("service", "api-gateway"), ("env", "prod")]),
    ("aws", "AmazonVPC", "ecommerce-prod", "us-east-1", 95.0, 1.0, "NAT gateway hours and data processed", &[("aws_product", "natgateway"), ("team", "platform"), ("env", "prod")]),
    ("aws", "AmazonEKS", "ecommerce-prod", "us-east-1", 70.0, 1.0, "EKS cluster hours", &[("aws_product", "eks"), ("team", "platform"), ("env", "prod")]),
    ("aws", "AmazonDynamoDB", "ecommerce-prod", "us-east-1", 140.0, 0.9, "DynamoDB read and write capacity", &[("aws_product", "dynamodb"), ("team", "fulfillment"), ("service", "inventory-service"), ("env", "prod")]),
    ("aws", "AWSLambda", "ecommerce-prod", "us-east-1", 55.0, 0.8, "Lambda requests and duration", &[("aws_product", "lambda"), ("team", "payments"), ("service", "billing"), ("env", "prod")]),
    ("aws", "AmazonCloudFront", "ecommerce-prod", "us-east-1", 120.0, 1.1, "CloudFront data transfer and requests", &[("aws_product", "cloudfront"), ("team", "storefront"), ("service", "storefront-api"), ("env", "prod")]),
    ("aws", "AmazonBedrock", "ecommerce-prod", "us-east-1", 180.0, 0.7, "Model inference tokens", &[("aws_product", "bedrock"), ("ai_vendor", "Amazon Bedrock"), ("ai_model", "chat-large"), ("team", "ml"), ("service", "catalog-service"), ("env", "prod")]),
    ("azure", "VirtualMachines", "ecommerce-eu", "westeurope", 520.0, 1.0, "Virtual machine compute hours", &[("metercategory", "Virtual Machines"), ("team", "platform"), ("service", "storefront-api"), ("env", "prod")]),
    ("azure", "VirtualMachines", "ecommerce-eu", "westeurope", 260.0, 1.0, "NC-series GPU virtual machines", &[("metercategory", "Virtual Machines"), ("gpu_type", "t4"), ("team", "ml"), ("service", "catalog-service"), ("env", "prod")]),
    ("azure", "Storage", "ecommerce-eu", "westeurope", 78.0, 1.0, "Blob storage capacity and operations", &[("metercategory", "Storage"), ("metersubcategory", "Blob Storage"), ("team", "platform"), ("env", "prod")]),
    ("azure", "Storage", "ecommerce-eu", "westeurope", 6.0, 1.0, "File shares", &[("metercategory", "Storage"), ("metersubcategory", "Files"), ("team", "platform"), ("env", "prod")]),
    ("azure", "Storage", "ecommerce-eu", "westeurope", 2.5, 1.0, "Tiered block blob storage", &[("metercategory", "Storage"), ("metersubcategory", "Tiered Block Blob"), ("team", "data"), ("env", "prod")]),
    ("azure", "Storage", "ecommerce-eu", "westeurope", 0.6, 1.0, "Queue operations", &[("metercategory", "Storage"), ("metersubcategory", "Queues v2"), ("team", "fulfillment"), ("env", "prod")]),
    ("azure", "Storage", "ecommerce-eu", "westeurope", 0.25, 1.0, "Table storage", &[("metercategory", "Storage"), ("metersubcategory", "Tables"), ("team", "fulfillment"), ("env", "prod")]),
    ("azure", "SQLDatabase", "ecommerce-eu", "westeurope", 210.0, 1.0, "SQL Database vCore hours", &[("metercategory", "SQL Database"), ("team", "checkout"), ("service", "order-service"), ("env", "prod")]),
    ("azure", "LogAnalytics", "ecommerce-eu", "westeurope", 140.0, 0.9, "Log data ingestion", &[("metercategory", "Log Analytics"), ("team", "platform"), ("env", "prod")]),
    ("azure", "FoundryModels", "ecommerce-eu", "westeurope", 900.0, 0.6, "Model inference tokens", &[("metercategory", "Foundry Models"), ("ai_vendor", "Azure AI Foundry"), ("ai_model", "chat-large"), ("team", "ml"), ("service", "storefront-api"), ("env", "prod")]),
    ("azure", "FoundryModels", "ecommerce-eu", "westeurope", 120.0, 0.6, "Embedding tokens", &[("metercategory", "Foundry Models"), ("ai_vendor", "Azure AI Foundry"), ("ai_model", "embed-small"), ("team", "ml"), ("service", "catalog-service"), ("env", "prod")]),
    ("azure", "Bandwidth", "ecommerce-eu", "westeurope", 35.0, 0.9, "Data transfer out", &[("metercategory", "Bandwidth"), ("data_transfer_type", "internet"), ("team", "storefront"), ("env", "prod")]),
    ("azure", "AzureDevOps", "ecommerce-eu", "westeurope", 60.0, 0.3, "Pipelines parallel jobs", &[("metercategory", "Azure DevOps"), ("team", "platform"), ("env", "ci")]),
    ("gcp", "BigQuery", "ecommerce-analytics", "us-central1", 380.0, 0.5, "Analysis bytes scanned", &[("gcp_product", "bigquery"), ("team", "data"), ("service", "billing"), ("env", "prod")]),
    ("gcp", "Compute Engine", "ecommerce-analytics", "us-central1", 240.0, 1.0, "N2 instance core and RAM hours", &[("gcp_product", "compute"), ("team", "data"), ("env", "prod")]),
    ("gcp", "Compute Engine", "ecommerce-analytics", "us-central1", 150.0, 1.0, "A2 GPU instance hours", &[("gcp_product", "compute"), ("gpu_type", "a100"), ("team", "ml"), ("service", "catalog-service"), ("env", "prod")]),
    ("gcp", "Cloud Storage", "ecommerce-analytics", "us-central1", 75.0, 1.0, "Standard storage", &[("gcp_product", "storage"), ("storage_class", "standard"), ("team", "data"), ("env", "prod")]),
    ("gcp", "Kubernetes Engine", "ecommerce-analytics", "europe-west1", 110.0, 1.0, "GKE cluster management", &[("gcp_product", "gke"), ("team", "platform"), ("env", "prod")]),
    ("gcp", "Vertex AI", "ecommerce-analytics", "us-central1", 60.0, 0.7, "Generative model tokens", &[("gcp_product", "vertexai"), ("ai_vendor", "Vertex AI"), ("ai_model", "chat-medium"), ("team", "ml"), ("service", "catalog-service"), ("env", "prod")]),
    ("gcp", "Networking", "ecommerce-analytics", "us-central1", 40.0, 0.9, "Inter-region egress", &[("gcp_product", "networking"), ("data_transfer_type", "cross-region"), ("team", "data"), ("env", "prod")]),
    ("Snowflake", "Compute", "ecommerce-warehouse", "us-east-1", 260.0, 0.5, "Warehouse credits", &[("team", "data"), ("env", "prod")]),
    ("Snowflake", "Storage", "ecommerce-warehouse", "us-east-1", 30.0, 1.0, "Storage", &[("team", "data"), ("env", "prod")]),
    ("Confluent Cloud", "Kafka Cluster", "ecommerce-streaming", "us-east-1", 140.0, 1.0, "Dedicated cluster CKUs", &[("team", "fulfillment"), ("service", "order-service"), ("env", "prod")]),
    ("GitHub", "Actions", "example-org", "global", 45.0, 0.25, "Actions minutes", &[("team", "platform"), ("env", "ci")]),
    ("Fastly", "CDN", "ecommerce-edge", "global", 55.0, 1.1, "Edge bandwidth and requests", &[("team", "storefront"), ("service", "storefront-api"), ("env", "prod")]),
    ("MongoDB", "Atlas Cluster", "ecommerce-catalog", "us-east-1", 90.0, 1.0, "Dedicated cluster hours", &[("team", "catalog"), ("service", "catalog-service"), ("env", "prod")]),
    ("OpenAI", "API", "ecommerce-assistant", "global", 110.0, 0.6, "Model inference tokens", &[("ai_vendor", "OpenAI"), ("ai_model", "chat-small"), ("team", "ml"), ("service", "storefront-api"), ("env", "prod")]),
];

/// AWS Savings Plan: billed up front on the 1st, amortized daily into EC2 (`ecommerce-prod`).
const SAVINGS_PLAN_DAILY: f64 = 300.0;

/// How the demo bills price a line's usage: (service, description, pricing unit, on-demand price per
/// unit, the bill's tags for the unit: instance types, meters, SKUs).
type DemoPrice = (&'static str, &'static str, &'static str, f64, &'static [(&'static str, &'static str)]);

const DEMO_PRICES: &[DemoPrice] = &[
    ("AmazonEC2", "EC2 instance usage", "Hrs", 0.192, &[("aws_instance_type", "m6i.xlarge"), ("aws_usage_type", "BoxUsage:m6i.xlarge"), ("aws_operation", "RunInstances")]),
    ("AmazonEC2", "EC2 GPU instance usage", "Hrs", 1.006, &[("aws_instance_type", "g5.xlarge"), ("aws_usage_type", "BoxUsage:g5.xlarge"), ("aws_operation", "RunInstances")]),
    ("AmazonRDS", "RDS database instances", "Hrs", 0.52, &[("aws_instance_type", "db.r6g.xlarge"), ("aws_usage_type", "InstanceUsage:db.r6g.xl"), ("aws_operation", "CreateDBInstance")]),
    ("AmazonS3", "S3 storage and requests", "GB-Mo", 0.023, &[("aws_usage_type", "TimedStorage-ByteHrs")]),
    ("AmazonS3", "S3 Glacier storage", "GB-Mo", 0.004, &[("aws_usage_type", "TimedStorage-GlacierByteHrs")]),
    ("AWSDataTransfer", "Data transfer between availability zones", "GB", 0.01, &[("aws_usage_type", "DataTransfer-Regional-Bytes")]),
    ("AWSDataTransfer", "Data transfer between regions", "GB", 0.02, &[("aws_usage_type", "USE1-USW2-AWS-Out-Bytes")]),
    ("AWSDataTransfer", "Data transfer out to the internet", "GB", 0.09, &[("aws_usage_type", "DataTransfer-Out-Bytes")]),
    ("ElasticLoadBalancing", "Load balancer hours and capacity units", "LCU-Hrs", 0.008, &[("aws_usage_type", "LoadBalancerUsage")]),
    ("AmazonVPC", "NAT gateway hours and data processed", "GB", 0.045, &[("aws_usage_type", "NatGateway-Bytes")]),
    ("AmazonEKS", "EKS cluster hours", "Hrs", 0.10, &[("aws_usage_type", "AmazonEKS-Hours:perCluster")]),
    ("AmazonDynamoDB", "DynamoDB read and write capacity", "WriteCapacityUnit-Hrs", 0.00065, &[("aws_usage_type", "WriteCapacityUnit-Hrs")]),
    ("AWSLambda", "Lambda requests and duration", "Lambda-GB-Second", 0.0000166667, &[("aws_usage_type", "Lambda-GB-Second")]),
    ("AmazonCloudFront", "CloudFront data transfer and requests", "GB", 0.085, &[("aws_usage_type", "DataTransfer-Out-Bytes")]),
    ("AmazonBedrock", "Model inference tokens", "1K tokens", 0.003, &[("aws_usage_type", "USE1-InputTokenCount")]),
    ("VirtualMachines", "Virtual machine compute hours", "1 Hour", 0.192, &[("productname", "Virtual Machines Dsv5 Series"), ("metername", "D4s v5"), ("additionalinfo_vcpus", "4"), ("additionalinfo_servicetype", "Standard_D4s_v5"), ("consumedservice", "Microsoft.Compute")]),
    ("VirtualMachines", "NC-series GPU virtual machines", "1 Hour", 0.526, &[("productname", "Virtual Machines NCasT4 v3 Series"), ("metername", "NC4as T4 v3"), ("additionalinfo_vcpus", "4"), ("additionalinfo_servicetype", "Standard_NC4as_T4_v3"), ("consumedservice", "Microsoft.Compute")]),
    ("Storage", "Blob storage capacity and operations", "1 GB/Month", 0.0184, &[("productname", "Blob Storage"), ("metername", "Hot LRS Data Stored"), ("consumedservice", "Microsoft.Storage")]),
    ("Storage", "File shares", "1 GB/Month", 0.06, &[("productname", "Files v2"), ("metername", "Hot LRS Data Stored"), ("consumedservice", "Microsoft.Storage")]),
    ("Storage", "Tiered block blob storage", "1 GB/Month", 0.01, &[("productname", "Tiered Block Blob"), ("metername", "Cool LRS Data Stored"), ("consumedservice", "Microsoft.Storage")]),
    ("Storage", "Queue operations", "10K", 0.004, &[("productname", "Queues v2"), ("metername", "LRS Class 1 Operations"), ("consumedservice", "Microsoft.Storage")]),
    ("Storage", "Table storage", "1 GB/Month", 0.045, &[("productname", "Tables"), ("metername", "LRS Data Stored"), ("consumedservice", "Microsoft.Storage")]),
    ("SQLDatabase", "SQL Database vCore hours", "1 Hour", 0.505, &[("productname", "SQL Database Single General Purpose - Compute Gen5"), ("metername", "vCore"), ("consumedservice", "Microsoft.Sql")]),
    ("LogAnalytics", "Log data ingestion", "1 GB", 2.76, &[("productname", "Log Analytics"), ("metername", "Pay-as-you-go Data Ingestion"), ("consumedservice", "Microsoft.OperationalInsights")]),
    ("FoundryModels", "Model inference tokens", "1M", 2.5, &[("productname", "Foundry Models"), ("metername", "chat-large Input Tokens"), ("consumedservice", "Microsoft.CognitiveServices")]),
    ("FoundryModels", "Embedding tokens", "1M", 0.02, &[("productname", "Foundry Models"), ("metername", "embed-small Tokens"), ("consumedservice", "Microsoft.CognitiveServices")]),
    ("Bandwidth", "Data transfer out", "1 GB", 0.087, &[("productname", "Rtn Preference: MGN"), ("metername", "Standard Data Transfer Out"), ("consumedservice", "Microsoft.Network")]),
    ("AzureDevOps", "Pipelines parallel jobs", "1/Day", 1.33, &[("productname", "Azure Pipelines"), ("metername", "Microsoft-hosted CI/CD"), ("consumedservice", "Microsoft.VisualStudio")]),
    ("BigQuery", "Analysis bytes scanned", "tebibyte", 6.25, &[("gcp_sku_description", "Analysis (us-central1)")]),
    ("Compute Engine", "N2 instance core and RAM hours", "hour", 0.0316, &[("gcp_sku_description", "N2 Instance Core running in Americas"), ("gcp_machine_type", "n2-standard-4")]),
    ("Compute Engine", "A2 GPU instance hours", "hour", 2.93, &[("gcp_sku_description", "Nvidia Tesla A100 GPU running in Americas"), ("gcp_machine_type", "a2-highgpu-1g")]),
    ("Cloud Storage", "Standard storage", "gibibyte month", 0.020, &[("gcp_sku_description", "Standard Storage US Regional")]),
    ("Kubernetes Engine", "GKE cluster management", "hour", 0.10, &[("gcp_sku_description", "Regional Kubernetes Clusters")]),
    ("Vertex AI", "Generative model tokens", "1M tokens", 1.25, &[("gcp_sku_description", "Generative model input tokens")]),
    ("Networking", "Inter-region egress", "gibibyte", 0.01, &[("gcp_sku_description", "Network Inter Region Data Transfer Out from Americas to Europe")]),
    ("Compute", "Warehouse credits", "Credits", 3.0, &[]),
    ("Storage", "Storage", "TB-Mo", 23.0, &[]),
    ("Kafka Cluster", "Dedicated cluster CKUs", "CKU-Hours", 2.25, &[]),
    ("Actions", "Actions minutes", "Minutes", 0.008, &[]),
    ("CDN", "Edge bandwidth and requests", "GB", 0.12, &[]),
    ("Atlas Cluster", "Dedicated cluster hours", "Hours", 2.08, &[]),
    ("API", "Model inference tokens", "1M tokens", 0.6, &[]),
];

fn demo_price(service: &str, desc: &str) -> Option<&'static DemoPrice> {
    DEMO_PRICES.iter().find(|p| p.0 == service && p.1 == desc)
}

/// A commitment of the demo bills, covering one line's usage up to its daily amount.
struct DemoCommitment {
    id: &'static str,
    /// `Savings Plan`, `Reserved Instance`, `Pre-purchase Plan` or `Committed Use Discount`.
    kind: &'static str,
    service: &'static str,
    description: &'static str,
    account: &'static str,
    /// What it costs a day, amortized (used or not).
    daily: f64,
    /// Off the on-demand price.
    discount: f64,
    /// Months.
    term: u32,
    /// Paid up front (billed by the purchase line) rather than with the usage.
    upfront: bool,
}

const DEMO_COMMITMENTS: &[DemoCommitment] = &[
    DemoCommitment {
        id: "arn:aws:savingsplans::123456789012:savingsplan/0a1b2c3d-4e5f-4000-8000-000000000001",
        kind: "Savings Plan",
        service: "AmazonEC2",
        description: "EC2 instance usage",
        account: "ecommerce-prod",
        daily: SAVINGS_PLAN_DAILY,
        discount: 0.28,
        term: 12,
        upfront: true,
    },
    DemoCommitment { id: "ri-0a1b2c3d4e5f60001", kind: "Reserved Instance", service: "AmazonRDS", description: "RDS database instances", account: "ecommerce-prod", daily: 300.0, discount: 0.4, term: 12, upfront: false },
    DemoCommitment {
        id: "reservation/00000000-0000-4000-8000-000000000101",
        kind: "Reserved Instance",
        service: "VirtualMachines",
        description: "Virtual machine compute hours",
        account: "ecommerce-eu",
        daily: 230.0,
        discount: 0.36,
        term: 12,
        upfront: false,
    },
    DemoCommitment {
        id: "savingsplan/00000000-0000-4000-8000-000000000102",
        kind: "Savings Plan",
        service: "VirtualMachines",
        description: "NC-series GPU virtual machines",
        account: "ecommerce-eu",
        daily: 90.0,
        discount: 0.15,
        term: 36,
        upfront: false,
    },
    DemoCommitment {
        id: "projects/ecommerce-analytics/regions/us-central1/commitments/n2-commitment-1",
        kind: "Committed Use Discount",
        service: "Compute Engine",
        description: "N2 instance core and RAM hours",
        account: "ecommerce-analytics",
        daily: 110.0,
        discount: 0.37,
        term: 12,
        upfront: false,
    },
];

/// A deterministic number in [-1, 1) for a day and line.
fn noise(day: i64, line: usize, salt: u64) -> f64 {
    let mut x = (day as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ (line as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F) ^ salt;
    x ^= x >> 33;
    x = x.wrapping_mul(0xFF51_AFD7_ED55_8CCD);
    x ^= x >> 33;
    (x >> 11) as f64 / (1u64 << 52) as f64 - 1.0
}

/// One row's costs: (billed, effective, list), its usage, and the commitment it belongs to.
struct DemoCost {
    billed: f64,
    effective: f64,
    list: f64,
    quantity: f64,
    pricing: &'static str,
    commitment: Option<(&'static DemoCommitment, &'static str)>,
}

impl DemoCost {
    fn plain(billed: f64, effective: f64, list: f64) -> Self {
        DemoCost { billed, effective, list, quantity: 0.0, pricing: "", commitment: None }
    }
}

/// The fictional bills of one day: `today` is the day the data is generated (for recent changes).
pub fn demo_lines(org: &str, d: NaiveDate, today: NaiveDate) -> Vec<CostLine> {
    let epoch = NaiveDate::from_ymd_opt(2000, 1, 1).unwrap_or_default();
    let n = (d - epoch).num_days();
    let age = (today - d).num_days();
    let weekend = d.weekday().number_from_monday() >= 6;
    // A year of growth, and a slow monthly swing.
    let trend = 1.0 - 0.18 * (age as f64 / 365.0).min(1.5);
    let swing = 1.0 + 0.06 * ((n as f64) * std::f64::consts::TAU / 29.0).sin();
    // Commitments were bought 10 months ago: before that, the usage they cover ran on demand.
    let bought = today.with_day(1).unwrap_or(today).checked_sub_months(chrono::Months::new(10)).unwrap_or(today);
    let mut out = Vec::new();
    let mut push = |no: u32, (provider, service, account, region, _, _, desc, tags): DemoLine, c: DemoCost, category: &str| {
        let mut t: BTreeMap<String, String> = tags.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        if provider == "aws" {
            t.insert("aws_account".into(), "123456789012".into());
        }
        let price = demo_price(service, desc).filter(|_| category == "usage");
        if let Some((_, _, _, _, extra)) = price {
            t.extend(extra.iter().map(|(k, v)| (k.to_string(), v.to_string())));
        }
        if let Some((cm, _)) = c.commitment {
            let end = bought.checked_add_months(chrono::Months::new(cm.term)).unwrap_or(bought);
            t.insert("commitment_term".into(), if cm.term % 12 == 0 { format!("{} year{}", cm.term / 12, if cm.term == 12 { "" } else { "s" }) } else { format!("{} months", cm.term) });
            t.insert("commitment_purchase_option".into(), if cm.upfront { "All Upfront" } else { "No Upfront" }.into());
            t.insert("commitment_start".into(), bought.to_string());
            t.insert("commitment_end".into(), end.to_string());
        }
        out.push(CostLine {
            org: org.to_string(),
            file_id: DEMO_FILE.into(),
            line_no: (n as u32) * 100 + no,
            date: d.to_string(),
            provider: provider.into(),
            service: service.into(),
            sub_account: account.into(),
            region: region.into(),
            zone: String::new(),
            category: category.into(),
            description: desc.into(),
            resource_id: String::new(),
            pricing_unit: price.map(|p| p.2.to_string()).unwrap_or_default(),
            quantity: c.quantity,
            billed: round(c.billed),
            effective: round(c.effective),
            list: round(c.list),
            pricing_category: c.pricing.into(),
            commitment_id: c.commitment.map(|(cm, _)| cm.id.to_string()).unwrap_or_default(),
            commitment_type: c.commitment.map(|(cm, _)| cm.kind.to_string()).unwrap_or_default(),
            commitment_status: c.commitment.map(|(_, status)| status.to_string()).unwrap_or_default(),
            currency: "USD".into(),
            tags: t,
            source: "demo".into(),
            version: 1,
            deleted: 0,
        });
    };
    for (i, line) in DEMO_LINES.iter().enumerate() {
        let (_, service, account, _, base, weekend_factor, desc, _) = *line;
        let mut cost = base * trend * swing * (1.0 + 0.09 * noise(n, i, 1)) * if weekend { weekend_factor } else { 1.0 };
        // The last week: a new assistant feature drives Vertex AI up; the staging fleet was scaled down.
        if service == "Vertex AI" && age < 7 {
            cost *= 9.0;
        }
        if desc == "EC2 instance usage" && account == "ecommerce-staging" && age < 7 {
            cost *= 0.45;
        }
        // Spikes: a few days of heavy batch jobs each month.
        if service == "BigQuery" && noise(n, i, 2) > 0.85 {
            cost *= 2.2;
        }
        let unit = demo_price(service, desc).map(|p| p.3).unwrap_or_default();
        let quantity = |list: f64| if unit > 0.0 { list / unit } else { 0.0 };
        let cm = DEMO_COMMITMENTS.iter().enumerate().find(|(_, c)| c.service == service && c.description == desc && c.account == account && d >= bought);
        if let Some((j, cm)) = cm {
            // The commitment covers most of the usage, up to what it buys a day; the rest is on demand.
            let used = (cm.daily * (0.94 + 0.05 * noise(n, i, 3))).min(cost * 0.95);
            let unused = cm.daily - used;
            let covered = used / (1.0 - cm.discount);
            push(
                50 + j as u32,
                *line,
                DemoCost { billed: if cm.upfront { 0.0 } else { used }, effective: used, list: covered, quantity: quantity(covered), pricing: "Committed", commitment: Some((cm, "Used")) },
                "usage",
            );
            push(60 + j as u32, *line, DemoCost { billed: if cm.upfront { 0.0 } else { unused }, effective: unused, list: 0.0, quantity: 0.0, pricing: "Committed", commitment: Some((cm, "Unused")) }, "usage");
            cost -= used;
        }
        push(i as u32, *line, DemoCost { billed: cost, effective: cost, list: cost, quantity: quantity(cost), pricing: "Standard", commitment: None }, "usage");
    }
    if d.day() == 1 {
        let days_in_month = (d.checked_add_months(chrono::Months::new(1)).unwrap_or(d) - d).num_days() as f64;
        let plan: DemoLine = ("aws", "Savings Plans for AWS Compute usage", "ecommerce-prod", "global", 0.0, 1.0, "Savings Plan upfront fee", &[("aws_product", "savingsplans"), ("team", "platform"), ("env", "prod")]);
        push(90, plan, DemoCost::plain(SAVINGS_PLAN_DAILY * days_in_month, 0.0, 0.0), "purchase");
        let tax: DemoLine = ("azure", "Tax", "ecommerce-eu", "westeurope", 0.0, 1.0, "VAT", &[]);
        push(91, tax, DemoCost::plain(410.0 * trend, 410.0 * trend, 410.0 * trend), "tax");
    }
    let credit: DemoLine = ("gcp", "Compute Engine", "ecommerce-analytics", "us-central1", 0.0, 1.0, "Sustained use discount", &[("gcp_product", "compute"), ("team", "data"), ("env", "prod")]);
    push(92, credit, DemoCost::plain(-18.0 * trend, -18.0 * trend, -18.0 * trend), "credit");
    out
}

/// Writes the demo bills up to yesterday: the missing days of the last [`DEMO_DAYS`].
pub async fn seed_demo(st: &AppState, org: &str) -> Result<usize, ApiError> {
    let today = Utc::now().date_naive();
    let last: String = st
        .ch(org)
        .query(&format!("SELECT toString(max(ChargeDate)) FROM {TABLE} WHERE OrgId = {{oc_org:String}} AND FileId = ? AND IsDeleted = 0"))
        .bind(DEMO_FILE)
        .fetch_one()
        .await?;
    let first = today - chrono::Duration::days(DEMO_DAYS);
    let start = match NaiveDate::parse_from_str(&last, "%Y-%m-%d") {
        Ok(d) if d >= first && d.year() > 1970 => d + chrono::Duration::days(1),
        _ => first,
    };
    let mut lines = Vec::new();
    let mut d = start;
    while d < today {
        lines.extend(demo_lines(org, d, today));
        d += chrono::Duration::days(1);
    }
    insert(st, &lines).await?;
    Ok(lines.len())
}

/// Keeps the demo org's bills current (one new day a day).
pub async fn run_demo(st: AppState, org: String) {
    let mut tick = tokio::time::interval(Duration::from_secs(3 * 3600));
    loop {
        tick.tick().await;
        match seed_demo(&st, &org).await {
            Ok(0) => {}
            Ok(n) => tracing::info!(lines = n, "wrote demo cloud costs"),
            Err(e) => tracing::warn!(error = %e, "writing demo cloud costs failed"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn datadog_cost_metric_names() {
        assert_eq!(cost_metric("all.cost"), Some(CostMetric { providers: CostProviders::All, column: CostColumn::Effective }));
        assert_eq!(cost_metric("aws.cost.amortized"), Some(CostMetric { providers: CostProviders::One("aws".into()), column: CostColumn::Effective }));
        assert_eq!(cost_metric("aws.cost.net.amortized").unwrap().column, CostColumn::Effective);
        assert_eq!(cost_metric("aws.cost.net.unblended").unwrap().column, CostColumn::Billed);
        assert_eq!(cost_metric("azure.cost.actual").unwrap().column, CostColumn::Billed);
        assert_eq!(cost_metric("gcp.cost.amortized.shared.resources.allocated").unwrap().column, CostColumn::Effective);
        assert_eq!(cost_metric("custom.cost.basis"), Some(CostMetric { providers: CostProviders::Custom, column: CostColumn::Billed }));
        assert_eq!(cost_metric("system.cpu.user"), None);
        assert_eq!(cost_metric("aws.cost.unknown"), None);
    }

    #[test]
    fn cost_tags_map_to_focus_columns_or_bill_tags() {
        assert_eq!(cost_field("providername"), ("ProviderName".to_string(), vec![]));
        assert_eq!(cost_field("sub_account_name"), ("SubAccountName".to_string(), vec![]));
        assert_eq!(cost_field("team"), ("Tags[?]".to_string(), vec![Param::Str("team".into())]));
    }

    #[test]
    fn budget_queries_have_no_aggregator() {
        use oc_query::TagFilter;
        let q = budget_query("aws.cost.amortized{service:ec2} by {service}").unwrap();
        assert_eq!(q.metric, "aws.cost.amortized");
        assert_eq!(q.by, vec!["service".to_string()]);
        assert_eq!(q.filters, vec![TagFilter { key: "service".into(), value: "ec2".into(), negate: false }]);
        assert!(budget_query("sum:all.cost{*}").is_ok());
        assert!(budget_query("system.cpu.user{*}").is_err());
        // The budget page scopes SaaS providers by name, quoted when it has spaces.
        let q = budget_query("all.cost{providername:\"Confluent Cloud\"} by {team}").unwrap();
        assert_eq!(q.filters[0].value, "Confluent Cloud");
    }

    #[test]
    fn cost_points_are_daily_and_scoped_to_the_org() {
        let q = oc_query::parse_metric_query("sum:aws.cost.amortized{team:ml} by {servicename}.rollup(sum, monthly)").unwrap();
        let range = TimeRange { from: 1_760_000_000_000, to: 1_760_000_000_000 + 30 * DAY_MS };
        let (sql, params) = points_sql(&cost_metric(&q.metric).unwrap(), &q, range, step_ms(range, 300));
        assert!(sql.contains("OrgId = {oc_org:String}"));
        assert!(sql.contains("INTERVAL 86400 SECOND"));
        assert!(sql.contains("FINAL"));
        assert_eq!(params.len(), 5);
        assert_eq!(step_ms(TimeRange { from: 0, to: 365 * DAY_MS }, 100), 4 * DAY_MS);
    }

    #[test]
    fn a_wildcard_alone_means_the_tag_is_set() {
        let f = cost_where("gpu_type:* -team:* providername:aws");
        assert!(f.clause.contains("notEmpty(Tags[?])"), "{}", f.clause);
        assert!(f.clause.contains("NOT notEmpty(Tags[?])"), "{}", f.clause);
        assert!(f.clause.contains("ProviderName = ?"), "{}", f.clause);
        assert_eq!(f.params, vec![Param::Str("aws".into()), Param::Str("gpu_type".into()), Param::Str("team".into())]);
    }

    #[test]
    fn previous_period_has_the_same_length() {
        let d = |s: &str| NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap();
        assert_eq!(previous(d("2026-10-22"), d("2026-10-28")), (d("2026-10-15"), d("2026-10-21")));
    }

    #[test]
    fn groups_beyond_the_limit_become_other() {
        let rows = (0..5).map(|i| BucketRow { g: vec![format!("s{i}")], t: 0, cost: (i + 1) as f64 }).collect();
        let index = HashMap::from([(0, 0)]);
        let prev = HashMap::from([(vec!["gone".to_string()], 7.0)]);
        let d = NaiveDate::from_ymd_opt(1970, 1, 1).unwrap();
        let r = assemble(&["servicename".to_string()], vec![0], &index, rows, &prev, 3, d, d, None);
        assert_eq!(r.series.len(), 4);
        assert_eq!(r.series[0].tags["servicename"], "s4");
        // `gone` cost nothing now but 7 before: it ranks with the rest by total, then by its previous cost.
        let other = r.series.last().unwrap();
        assert_eq!(other.tags["other"], "Other");
        assert_eq!(other.total, 3.0);
        assert_eq!(other.previous_total, 7.0);
        assert_eq!(r.total, 15.0);
        assert_eq!(r.previous_total, 7.0);
    }

    #[test]
    fn demo_bills_are_deterministic_and_amortize_the_savings_plan() {
        let d = NaiveDate::from_ymd_opt(2026, 10, 1).unwrap();
        let a = demo_lines("default", d, d);
        assert_eq!(a, demo_lines("default", d, d));
        let purchase = a.iter().find(|l| l.category == "purchase").unwrap();
        assert_eq!(purchase.effective, 0.0);
        assert_eq!(purchase.billed, SAVINGS_PLAN_DAILY * 31.0);
        let mut seen = std::collections::HashSet::new();
        assert!(a.iter().all(|l| seen.insert(l.line_no)));
        assert!(a.iter().filter(|l| l.provider == "aws").all(|l| l.tags.get("aws_account").map(String::as_str) == Some("123456789012")));
    }
    #[test]
    fn demo_commitments_split_usage_into_covered_on_demand_and_unused() {
        let d = NaiveDate::from_ymd_opt(2026, 10, 15).unwrap();
        let a = demo_lines("default", d, d);
        let ec2: Vec<_> = a.iter().filter(|l| l.service == "AmazonEC2" && l.description == "EC2 instance usage" && l.sub_account == "ecommerce-prod").collect();
        assert_eq!(ec2.len(), 3);
        let used = ec2.iter().find(|l| l.commitment_status == "Used").unwrap();
        let unused = ec2.iter().find(|l| l.commitment_status == "Unused").unwrap();
        // Paid up front: the usage it covers bills nothing, its amortized cost is the commitment.
        assert_eq!(used.billed, 0.0);
        assert!(used.list > used.effective);
        assert!((used.effective + unused.effective - SAVINGS_PLAN_DAILY).abs() < 0.02);
        assert_eq!(unused.quantity, 0.0);
        assert!(a.iter().filter(|l| l.category == "usage" && l.commitment_status != "Unused").all(|l| !l.pricing_unit.is_empty() && l.quantity > 0.0));
        assert!(a.iter().filter(|l| l.commitment_status.is_empty() && l.category == "usage").all(|l| l.list == l.effective));
    }

    #[test]
    fn commitments_savings_coverage_and_utilization() {
        let row = |service: &str, kind: &str, id: &str, status: &str, committed: u8, e: f64, l: f64| CommitRow {
            t: 0,
            service: service.into(),
            region: "us-east-1".into(),
            kind: kind.into(),
            id: id.into(),
            status: status.into(),
            committed,
            e,
            l,
        };
        let rows = vec![
            row("AmazonEC2", "Savings Plan", "sp-1", "Used", 1, 72.0, 100.0),
            row("AmazonEC2", "Savings Plan", "sp-1", "Unused", 1, 8.0, 0.0),
            row("AmazonEC2", "", "", "", 0, 50.0, 50.0),
            // Not covered by any commitment: not eligible.
            row("AmazonS3", "", "", "", 0, 40.0, 40.0),
        ];
        let meta = vec![CommitMeta { id: "sp-1".into(), kind: "Savings Plan".into(), ..CommitMeta::default() }];
        let r = assemble_commitments(vec![0], rows, meta);
        assert_eq!(r.on_demand_equivalent, vec![100.0]);
        assert_eq!(r.commitment_cost, vec![80.0]);
        assert_eq!(r.on_demand_spend, 50.0);
        assert!((r.coverage - 100.0 / 150.0).abs() < 1e-9);
        // 1 - (72 + 8 + 50) / (100 + 50)
        assert_eq!(r.effective_savings_rate, Some(0.1333));
        assert_eq!(r.realized_savings, 20.0);
        assert_eq!(r.commitments[0].utilization, 0.9);
        assert_eq!(r.utilization[0].unused, 8.0);
        assert_eq!(r.coverage_by_type.iter().map(|t| t.kind.as_str()).collect::<Vec<_>>(), vec!["Savings Plan", "On-Demand"]);
    }

}
