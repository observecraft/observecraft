//! Log indexes (Log Configuration → Indexes) and the org's data retention settings.
//!
//! Each org routes incoming logs to the first index whose filter matches (see `logproc`). An index
//! sets the retention of its logs, an optional daily quota, and exclusion filters. Retention changes
//! apply to logs indexed from then on. The org also chooses its span and metric retention (metrics:
//! full resolution, 5-minute and 1-hour rollups, from fixed tiers). The instance admin bounds what an
//! org may choose (`org_data_settings`: longest log / span / metric retention, number of indexes).

use std::collections::HashMap;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use oc_query::parse_query;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;

use crate::auth::{Admin, AuthContext};
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;

const DAY_MS: i64 = 86_400_000;

#[derive(Serialize, Deserialize, ToSchema, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ExclusionFilter {
    pub name: String,
    /// Logs matching this search are excluded…
    pub query: String,
    /// …at this rate (0–100 %).
    pub sample_rate: f64,
    pub enabled: bool,
}

#[derive(Serialize, ToSchema, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct LogIndex {
    pub id: String,
    pub name: String,
    pub filter: String,
    pub retention_days: i32,
    pub daily_quota: Option<i64>,
    pub quota_reset_hour: i16,
    #[sqlx(json)]
    pub exclusion_filters: Vec<ExclusionFilter>,
    pub position: i32,
    /// Logs indexed since the current quota day started.
    #[sqlx(skip)]
    pub indexed_today: u64,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct LogIndexes {
    pub indexes: Vec<LogIndex>,
    /// Logs received since 00:00 UTC, indexed or not.
    pub ingested_today: u64,
    pub max_retention_days: i32,
    pub max_indexes: i32,
}

#[derive(sqlx::FromRow)]
pub(crate) struct DataSettings {
    pub max_log_retention_days: i32,
    pub max_span_retention_days: i32,
    pub max_indexes: i32,
    pub span_retention_days: i32,
    #[sqlx(flatten)]
    pub metrics: StoredMetricRetention,
}

pub(crate) async fn data_settings(st: &AppState, org: &str) -> Result<DataSettings, ApiError> {
    let r = &st.config.retention;
    sqlx::query("INSERT INTO org_data_settings (org_id, max_log_retention_days, max_span_retention_days, span_retention_days) VALUES ($1, $2, $3, $4) ON CONFLICT DO NOTHING")
        .bind(org)
        .bind(r.logs_max_days.min(30) as i32)
        .bind(r.traces_max_days.min(30) as i32)
        .bind(r.traces_days as i32)
        .execute(&st.postgres)
        .await?;
    Ok(sqlx::query_as(
        "SELECT max_log_retention_days, max_span_retention_days, max_indexes, span_retention_days, metrics_retention_days, metrics_5m_days, metrics_1h_days, \
         max_metrics_retention_days, max_metrics_5m_days, max_metrics_1h_days FROM org_data_settings WHERE org_id = $1",
    )
    .bind(org)
    .fetch_one(&st.postgres)
    .await?)
}

/// New orgs: data settings within the install's ceilings and one catch-all index.
pub(crate) async fn provision(st: &AppState, org: &str) -> Result<(), ApiError> {
    data_settings(st, org).await?;
    sqlx::query("INSERT INTO log_indexes (org_id, name, filter, retention_days, position) VALUES ($1, 'main', '', $2, 0) ON CONFLICT DO NOTHING")
        .bind(org)
        .bind(st.config.retention.logs_days.min(st.config.retention.logs_max_days) as i32)
        .execute(&st.postgres)
        .await?;
    Ok(())
}

const INDEX_COLS: &str = "id::text, name, filter, retention_days, daily_quota, quota_reset_hour, exclusion_filters, position";

#[derive(clickhouse::Row, Deserialize)]
struct HourCount {
    index_name: String,
    hour: i64,
    n: u64,
}

#[utoipa::path(get, path = "/api/v1/logs/indexes", operation_id = "list_log_indexes", responses((status = 200, body = LogIndexes)), tag = "logs")]
pub async fn list(State(st): State<AppState>, ctx: AuthContext) -> ApiResult<LogIndexes> {
    let settings = data_settings(&st, &ctx.org_id).await?;
    let mut indexes: Vec<LogIndex> = sqlx::query_as(&format!("SELECT {INDEX_COLS} FROM log_indexes WHERE org_id = $1 ORDER BY position, created_at"))
        .bind(&ctx.org_id)
        .fetch_all(&st.postgres)
        .await?;
    let hours = st
        .clickhouse
        .query(
            "SELECT toString(index_name) AS index_name, toInt64(toUnixTimestamp(toStartOfHour(timestamp))) * 1000 AS hour, count() AS n \
             FROM logs WHERE org_id = ? AND timestamp >= now() - INTERVAL 25 HOUR GROUP BY index_name, hour",
        )
        .bind(&ctx.org_id)
        .fetch_all::<HourCount>();
    let raw = st.clickhouse.query("SELECT count() FROM logs_raw WHERE org_id = ? AND timestamp >= toStartOfDay(now())").bind(&ctx.org_id).fetch_one::<u64>();
    let (hours, ingested_today) = tokio::try_join!(hours, raw)?;
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis() as i64;
    let mut by_index: HashMap<String, Vec<(i64, u64)>> = HashMap::new();
    for h in hours {
        by_index.entry(h.index_name).or_default().push((h.hour, h.n));
    }
    for ix in &mut indexes {
        let offset = ix.quota_reset_hour as i64 * 3_600_000;
        let start = (now - offset).div_euclid(DAY_MS) * DAY_MS + offset;
        ix.indexed_today = by_index.get(&ix.name).map(|v| v.iter().filter(|(h, _)| *h >= start).map(|(_, n)| n).sum()).unwrap_or(0);
    }
    Ok(Json(LogIndexes { indexes, ingested_today, max_retention_days: settings.max_log_retention_days, max_indexes: settings.max_indexes }))
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct LogIndexInput {
    pub name: String,
    #[serde(default)]
    pub filter: String,
    pub retention_days: i32,
    pub daily_quota: Option<i64>,
    #[serde(default)]
    pub quota_reset_hour: i16,
    #[serde(default)]
    pub exclusion_filters: Vec<ExclusionFilter>,
}

fn validate(input: &mut LogIndexInput, max_retention: i32) -> Result<(), ApiError> {
    input.name = input.name.trim().to_lowercase();
    let n = &input.name;
    if n.is_empty() || n.len() > 64 || !n.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_') || n == "rehydrated" {
        return Err(ApiError::BadRequest("Index names are 1–64 lowercase letters, digits, dashes or underscores (not \"rehydrated\")".into()));
    }
    if !(1..=max_retention).contains(&input.retention_days) {
        return Err(ApiError::BadRequest(format!("Retention must be between 1 and {max_retention} days for this organization")));
    }
    if input.daily_quota.is_some_and(|q| q <= 0) {
        return Err(ApiError::BadRequest("The daily quota must be positive (or empty for no limit)".into()));
    }
    if !(0..=23).contains(&input.quota_reset_hour) {
        return Err(ApiError::BadRequest("The quota reset hour is 0–23 (UTC)".into()));
    }
    if input.exclusion_filters.len() > 20 {
        return Err(ApiError::BadRequest("At most 20 exclusion filters per index".into()));
    }
    for e in &mut input.exclusion_filters {
        e.name = e.name.trim().chars().take(100).collect();
        if !(0.0..=100.0).contains(&e.sample_rate) || !e.sample_rate.is_finite() {
            return Err(ApiError::BadRequest("Exclusion rates are 0–100 %".into()));
        }
        parse_query(&e.query);
    }
    parse_query(&input.filter);
    Ok(())
}

async fn get_index(st: &AppState, org: &str, id: &str) -> Result<LogIndex, ApiError> {
    sqlx::query_as(&format!("SELECT {INDEX_COLS} FROM log_indexes WHERE org_id = $1 AND id::text = $2"))
        .bind(org)
        .bind(id)
        .fetch_optional(&st.postgres)
        .await?
        .ok_or_else(|| ApiError::NotFound("Index not found".into()))
}

fn unique(e: sqlx::Error, name: &str) -> ApiError {
    match &e {
        sqlx::Error::Database(d) if d.is_unique_violation() => ApiError::BadRequest(format!("An index named '{name}' already exists")),
        _ => e.into(),
    }
}

#[utoipa::path(post, path = "/api/v1/logs/indexes", operation_id = "create_log_index", request_body = LogIndexInput, responses((status = 200, body = LogIndex)), tag = "logs")]
pub async fn create(State(st): State<AppState>, Admin(ctx): Admin, Json(mut input): Json<LogIndexInput>) -> ApiResult<LogIndex> {
    let settings = data_settings(&st, &ctx.org_id).await?;
    validate(&mut input, settings.max_log_retention_days)?;
    let (count,): (i64,) = sqlx::query_as("SELECT count(*) FROM log_indexes WHERE org_id = $1").bind(&ctx.org_id).fetch_one(&st.postgres).await?;
    if count >= settings.max_indexes as i64 {
        return Err(ApiError::BadRequest(format!("This organization can have at most {} indexes", settings.max_indexes)));
    }
    let (id,): (String,) = sqlx::query_as(
        "INSERT INTO log_indexes (org_id, name, filter, retention_days, daily_quota, quota_reset_hour, exclusion_filters, position) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, (SELECT COALESCE(max(position), -1) + 1 FROM log_indexes WHERE org_id = $1)) RETURNING id::text",
    )
    .bind(&ctx.org_id)
    .bind(&input.name)
    .bind(input.filter.trim())
    .bind(input.retention_days)
    .bind(input.daily_quota)
    .bind(input.quota_reset_hour)
    .bind(serde_json::to_value(&input.exclusion_filters).unwrap_or(Value::Array(vec![])))
    .fetch_one(&st.postgres)
    .await
    .map_err(|e| unique(e, &input.name))?;
    ctx.audit(&st, "log_index.created", &input.name, &format!("{} days", input.retention_days)).await;
    Ok(Json(get_index(&st, &ctx.org_id, &id).await?))
}

#[utoipa::path(put, path = "/api/v1/logs/indexes/{id}", operation_id = "update_log_index", params(("id" = String, Path)), request_body = LogIndexInput, responses((status = 200, body = LogIndex)), tag = "logs")]
pub async fn update(State(st): State<AppState>, Admin(ctx): Admin, Path(id): Path<String>, Json(mut input): Json<LogIndexInput>) -> ApiResult<LogIndex> {
    let settings = data_settings(&st, &ctx.org_id).await?;
    validate(&mut input, settings.max_log_retention_days)?;
    let n = sqlx::query(
        "UPDATE log_indexes SET name = $3, filter = $4, retention_days = $5, daily_quota = $6, quota_reset_hour = $7, exclusion_filters = $8, updated_at = now() \
         WHERE org_id = $1 AND id::text = $2",
    )
    .bind(&ctx.org_id)
    .bind(&id)
    .bind(&input.name)
    .bind(input.filter.trim())
    .bind(input.retention_days)
    .bind(input.daily_quota)
    .bind(input.quota_reset_hour)
    .bind(serde_json::to_value(&input.exclusion_filters).unwrap_or(Value::Array(vec![])))
    .execute(&st.postgres)
    .await
    .map_err(|e| unique(e, &input.name))?
    .rows_affected();
    if n == 0 {
        return Err(ApiError::NotFound("Index not found".into()));
    }
    ctx.audit(&st, "log_index.updated", &input.name, &format!("{} days, quota {:?}", input.retention_days, input.daily_quota)).await;
    Ok(Json(get_index(&st, &ctx.org_id, &id).await?))
}

/// Stops indexing into it; its logs stay searchable (`index:<name>`) until they expire.
#[utoipa::path(delete, path = "/api/v1/logs/indexes/{id}", operation_id = "delete_log_index", params(("id" = String, Path)), responses((status = 204)), tag = "logs")]
pub async fn delete(State(st): State<AppState>, Admin(ctx): Admin, Path(id): Path<String>) -> Result<StatusCode, ApiError> {
    let row: Option<(String,)> = sqlx::query_as("DELETE FROM log_indexes WHERE org_id = $1 AND id::text = $2 RETURNING name").bind(&ctx.org_id).bind(&id).fetch_optional(&st.postgres).await?;
    let Some((name,)) = row else { return Err(ApiError::NotFound("Index not found".into())) };
    ctx.audit(&st, "log_index.deleted", &name, "").await;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize, ToSchema)]
pub struct ReorderIndexes {
    /// Every index id, in routing order.
    pub ids: Vec<String>,
}

#[utoipa::path(post, path = "/api/v1/logs/indexes/reorder", operation_id = "reorder_log_indexes", request_body = ReorderIndexes, responses((status = 204)), tag = "logs")]
pub async fn reorder(State(st): State<AppState>, Admin(ctx): Admin, Json(req): Json<ReorderIndexes>) -> Result<StatusCode, ApiError> {
    let mut tx = st.postgres.begin().await?;
    for (pos, id) in req.ids.iter().enumerate() {
        sqlx::query("UPDATE log_indexes SET position = $3 WHERE org_id = $1 AND id::text = $2").bind(&ctx.org_id).bind(id).bind(pos as i32).execute(&mut *tx).await?;
    }
    tx.commit().await?;
    ctx.audit(&st, "log_index.reordered", "", "").await;
    Ok(StatusCode::NO_CONTENT)
}

// ================================================================ org data settings

/// Metric retentions an org may choose, in days. Retention is part of the metric tables' partition
/// key, so the values come from a short fixed list (395: a year plus a month for year-over-year).
pub(crate) const METRIC_RETENTION_TIERS: [i32; 7] = [7, 15, 30, 90, 180, 395, 760];

/// Days metrics are kept at full resolution and as 5-minute and 1-hour rollups.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct MetricRetention {
    pub raw: i32,
    pub rollup_5m: i32,
    pub rollup_1h: i32,
}

impl MetricRetention {
    fn tiers_in_order(self) -> Result<(), String> {
        let tiers = METRIC_RETENTION_TIERS.map(|d| d.to_string()).join(", ");
        if [self.raw, self.rollup_5m, self.rollup_1h].iter().any(|d| !METRIC_RETENTION_TIERS.contains(d)) {
            return Err(format!("Metric retention must be one of {tiers} days"));
        }
        if self.raw > self.rollup_5m || self.rollup_5m > self.rollup_1h {
            return Err("Metrics can't be kept longer at a finer resolution: full resolution ≤ 5-minute ≤ 1-hour rollups".into());
        }
        Ok(())
    }
}

/// What `org_data_settings` stores for metrics: NULL = the install's default.
#[derive(Clone, Copy, Debug, Default, sqlx::FromRow)]
pub(crate) struct StoredMetricRetention {
    #[sqlx(rename = "metrics_retention_days")]
    pub raw: Option<i32>,
    #[sqlx(rename = "metrics_5m_days")]
    pub rollup_5m: Option<i32>,
    #[sqlx(rename = "metrics_1h_days")]
    pub rollup_1h: Option<i32>,
    #[sqlx(rename = "max_metrics_retention_days")]
    pub max_raw: Option<i32>,
    #[sqlx(rename = "max_metrics_5m_days")]
    pub max_5m: Option<i32>,
    #[sqlx(rename = "max_metrics_1h_days")]
    pub max_1h: Option<i32>,
}

impl StoredMetricRetention {
    /// (the org's retention, its limits), unset values taking the install's default.
    pub(crate) fn effective(self, install: MetricRetention) -> (MetricRetention, MetricRetention) {
        let or = |v: Option<i32>, d: i32| v.unwrap_or(d);
        (
            MetricRetention { raw: or(self.raw, install.raw), rollup_5m: or(self.rollup_5m, install.rollup_5m), rollup_1h: or(self.rollup_1h, install.rollup_1h) },
            MetricRetention { raw: or(self.max_raw, install.raw), rollup_5m: or(self.max_5m, install.rollup_5m), rollup_1h: or(self.max_1h, install.rollup_1h) },
        )
    }
}

/// The install's default metric retention (OC_RETENTION_METRICS_*).
pub(crate) fn install_metric_retention(r: &crate::config::Retention) -> MetricRetention {
    MetricRetention { raw: r.metrics_days as i32, rollup_5m: r.metrics_5m_days as i32, rollup_1h: r.metrics_1h_days as i32 }
}

/// An org's choice: tiers, in order, within its limits.
pub(crate) fn validate_metric_retention(v: MetricRetention, limits: MetricRetention) -> Result<(), String> {
    v.tiers_in_order()?;
    if v.raw > limits.raw || v.rollup_5m > limits.rollup_5m || v.rollup_1h > limits.rollup_1h {
        return Err(format!(
            "Metric retention can be up to {} / {} / {} days for this organization",
            limits.raw, limits.rollup_5m, limits.rollup_1h
        ));
    }
    Ok(())
}

/// The instance admin's limits for an org: tiers, in order.
pub(crate) fn validate_metric_limits(limits: MetricRetention) -> Result<(), String> {
    limits.tiers_in_order()
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct OrgDataSettings {
    /// Days the org's spans are kept.
    pub span_retention_days: i32,
    /// Ceilings set by the instance admin.
    pub max_span_retention_days: i32,
    pub max_log_retention_days: i32,
    pub max_indexes: i32,
    /// Days the org's metrics are kept at full resolution and as rollups, and its limits.
    pub metrics: MetricRetention,
    pub max_metrics: MetricRetention,
    /// What metric retentions can be chosen.
    pub metric_retention_tiers: Vec<i32>,
    /// Install-wide (not per org): CI retention, raw-log window feeding archives.
    pub ci_retention_days: u32,
    pub raw_logs_hours: u32,
}

async fn org_data_settings(st: &AppState, org: &str) -> Result<OrgDataSettings, ApiError> {
    let s = data_settings(st, org).await?;
    let r = &st.config.retention;
    let (metrics, max_metrics) = s.metrics.effective(install_metric_retention(r));
    Ok(OrgDataSettings {
        span_retention_days: s.span_retention_days,
        max_span_retention_days: s.max_span_retention_days,
        max_log_retention_days: s.max_log_retention_days,
        max_indexes: s.max_indexes,
        metrics,
        max_metrics,
        metric_retention_tiers: METRIC_RETENTION_TIERS.to_vec(),
        ci_retention_days: r.ci_days,
        raw_logs_hours: r.raw_logs_hours,
    })
}

#[utoipa::path(get, path = "/api/v1/org/data-settings", operation_id = "get_org_data_settings", responses((status = 200, body = OrgDataSettings)), tag = "logs")]
pub async fn get_settings(State(st): State<AppState>, ctx: AuthContext) -> ApiResult<OrgDataSettings> {
    Ok(Json(org_data_settings(&st, &ctx.org_id).await?))
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct OrgDataSettingsPatch {
    pub span_retention_days: Option<i32>,
    pub metrics: Option<MetricRetention>,
}

/// Span and metric retention apply to data received from then on (within about a minute).
#[utoipa::path(patch, path = "/api/v1/org/data-settings", operation_id = "update_org_data_settings", request_body = OrgDataSettingsPatch, responses((status = 200, body = OrgDataSettings)), tag = "logs")]
pub async fn update_settings(State(st): State<AppState>, Admin(ctx): Admin, Json(p): Json<OrgDataSettingsPatch>) -> ApiResult<OrgDataSettings> {
    let s = data_settings(&st, &ctx.org_id).await?;
    if let Some(days) = p.span_retention_days {
        if !(1..=s.max_span_retention_days).contains(&days) {
            return Err(ApiError::BadRequest(format!("Span retention must be between 1 and {} days for this organization", s.max_span_retention_days)));
        }
        sqlx::query("UPDATE org_data_settings SET span_retention_days = $2, updated_at = now() WHERE org_id = $1").bind(&ctx.org_id).bind(days).execute(&st.postgres).await?;
        ctx.audit(&st, "org.span_retention_changed", "", &format!("{days} days")).await;
    }
    if let Some(m) = p.metrics {
        let (_, limits) = s.metrics.effective(install_metric_retention(&st.config.retention));
        validate_metric_retention(m, limits).map_err(ApiError::BadRequest)?;
        sqlx::query("UPDATE org_data_settings SET metrics_retention_days = $2, metrics_5m_days = $3, metrics_1h_days = $4, updated_at = now() WHERE org_id = $1")
            .bind(&ctx.org_id)
            .bind(m.raw)
            .bind(m.rollup_5m)
            .bind(m.rollup_1h)
            .execute(&st.postgres)
            .await?;
        ctx.audit(&st, "org.metric_retention_changed", "", &format!("{} / {} / {} days", m.raw, m.rollup_5m, m.rollup_1h)).await;
    }
    Ok(Json(org_data_settings(&st, &ctx.org_id).await?))
}

#[cfg(test)]
mod tests {
    use super::*;

    const fn r(raw: i32, rollup_5m: i32, rollup_1h: i32) -> MetricRetention {
        MetricRetention { raw, rollup_5m, rollup_1h }
    }

    #[test]
    fn metric_retention_is_a_tier_within_the_limits() {
        let limits = r(30, 90, 395);
        assert!(validate_metric_retention(r(15, 90, 395), limits).is_ok());
        assert!(validate_metric_retention(r(30, 30, 30), limits).is_ok());
        // Not a tier: every distinct value is a partition, so they come from a short list.
        assert!(validate_metric_retention(r(20, 90, 395), limits).is_err());
        // Over the org's limit.
        assert!(validate_metric_retention(r(30, 90, 760), limits).is_err());
        // A finer resolution kept longer than a coarser one isn't useful.
        assert!(validate_metric_retention(r(30, 15, 395), limits).is_err());
        assert!(validate_metric_retention(r(7, 180, 90), r(30, 180, 395)).is_err());
    }

    #[test]
    fn unset_metric_retention_is_the_install_default() {
        let install = r(30, 90, 395);
        let unset = StoredMetricRetention::default();
        assert_eq!(unset.effective(install), (install, install));
        let chosen = StoredMetricRetention { raw: Some(15), rollup_5m: None, rollup_1h: Some(760), max_raw: None, max_5m: Some(180), max_1h: Some(760) };
        assert_eq!(chosen.effective(install), (r(15, 90, 760), r(30, 180, 760)));
    }

    #[test]
    fn metric_limits_are_ordered_tiers() {
        assert!(validate_metric_limits(r(90, 395, 760)).is_ok());
        assert!(validate_metric_limits(r(30, 90, 100)).is_err());
        assert!(validate_metric_limits(r(90, 30, 395)).is_err());
    }

    fn input(name: &str, days: i32) -> LogIndexInput {
        LogIndexInput { name: name.into(), filter: String::new(), retention_days: days, daily_quota: None, quota_reset_hour: 0, exclusion_filters: vec![] }
    }

    #[test]
    fn index_limits() {
        assert!(validate(&mut input("Main", 15), 30).is_ok());
        assert!(validate(&mut input("main", 31), 30).is_err());
        assert!(validate(&mut input("rehydrated", 7), 30).is_err());
        assert!(validate(&mut input("bad name", 7), 30).is_err());
        let mut i = input("x", 7);
        i.exclusion_filters.push(ExclusionFilter { name: "debug".into(), query: "status:debug".into(), sample_rate: 120.0, enabled: true });
        assert!(validate(&mut i, 30).is_err());
    }
}
