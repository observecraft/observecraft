//! Log archives: every closed hour of an org's logs (optionally filtered) is written as one Parquet
//! file to the org's S3-compatible bucket, under Hive-style partitions:
//!
//! ```text
//! <prefix>/org=<org>/dt=2026-10-25/hour=14/archive-<id>.parquet
//! ```
//!
//! - **Writing**: ClickHouse renders the hour as Parquet from `oc.logs_raw` (every ingested log, so
//!   archives also hold logs that no index keeps); the server uploads it (presigned PUT).
//!   Re-running an hour overwrites the same object, so retries are idempotent.
//! - **Archive Search**: downloads only the partitions inside the requested range into a local cache
//!   and queries them with DuckDB (same search syntax as the Log Explorer). Runs as a background job
//!   with progress; matching events are kept with the search.
//! - **Rehydration**: DuckDB filters the archived files; the result is loaded into
//!   `oc.logs_rehydrated` (search it with `index:rehydrated`) until the rehydration expires.
//!
//! All S3 access happens from the server, with the credentials stored per archive (secret encrypted
//! with `OC_SECRET_KEY`).

use std::path::{Path as FsPath, PathBuf};
use std::time::Duration;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use oc_query::{build_where, build_where_duckdb, parse_query, LogsSchema, Param};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::{IntoParams, ToSchema};

use super::logs::{bind, LogEvent, TimeRange};
use crate::auth::{Admin, AuthContext, Writer};
use crate::error::{ApiError, ApiResult};
use crate::s3::{prefix_dir, Store};
use crate::state::{AppState, OrgId};

const HOUR_MS: i64 = 3_600_000;
/// An hour is archived once it has been closed this long (late logs still arrive).
const GRACE_MS: i64 = 10 * 60_000;
/// Hours archived per archive per tick (manual runs do more).
const HOURS_PER_TICK: usize = 12;
const MAX_ATTEMPTS: i32 = 5;
const MAX_SEARCH_FILES: usize = 2_000;
const MAX_REHYDRATE_ROWS: i64 = 2_000_000;
/// Archived columns, in the order DuckDB reads them back.
const ARCHIVE_SELECT: &str = "org_id, toString(id) AS id, timestamp, status, service, host, source, message, tags, attr, attributes, trace_id, span_id, severity_number, scope";

fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis() as i64
}

fn hour_parts(hour_ms: i64) -> (String, String) {
    let secs = hour_ms / 1000;
    let days = secs.div_euclid(86_400);
    let hour = secs.rem_euclid(86_400) / 3600;
    // Civil date from days since 1970-01-01 (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    (format!("{y:04}-{m:02}-{d:02}"), format!("{hour:02}"))
}

/// Partition directory of one org-hour.
fn partition(prefix: &str, org: &str, hour_ms: i64) -> String {
    let (dt, hh) = hour_parts(hour_ms);
    format!("{}org={org}/dt={dt}/hour={hh}/", prefix_dir(prefix))
}

// ================================================================ archive configuration

#[derive(sqlx::FromRow, Clone)]
struct ArchiveRow {
    id: String,
    org_id: String,
    name: String,
    endpoint: String,
    region: String,
    bucket: String,
    prefix: String,
    path_style: bool,
    access_key: String,
    secret_key_enc: String,
    query: String,
    enabled: bool,
    start_hour: i64,
}

const ARCHIVE_COLS: &str = "a.id::text, a.org_id, a.name, a.endpoint, a.region, a.bucket, a.prefix, a.path_style, a.access_key, a.secret_key_enc, a.query, a.enabled, \
    (extract(epoch FROM a.start_hour) * 1000)::bigint AS start_hour";

impl ArchiveRow {
    fn store(&self, st: &AppState) -> Result<Store, ApiError> {
        Store::new(&self.endpoint, &self.region, &self.bucket, self.path_style, &self.access_key, &st.secrets.decrypt(&self.secret_key_enc)?)
    }
}

async fn load_archive(st: &AppState, org: &str, id: &str) -> Result<ArchiveRow, ApiError> {
    sqlx::query_as(&format!("SELECT {ARCHIVE_COLS} FROM log_archives a WHERE a.org_id = $1 AND a.id::text = $2"))
        .bind(org)
        .bind(id)
        .fetch_optional(&st.postgres)
        .await?
        .ok_or_else(|| ApiError::NotFound("Archive not found".into()))
}

#[derive(Serialize, ToSchema, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct LogArchive {
    pub id: String,
    pub name: String,
    pub endpoint: String,
    pub region: String,
    pub bucket: String,
    pub prefix: String,
    pub path_style: bool,
    pub access_key: String,
    pub query: String,
    pub enabled: bool,
    pub created_at: i64,
    /// Archived hours (with or without logs).
    pub hours_done: i64,
    pub hours_failed: i64,
    pub rows: i64,
    pub bytes: i64,
    /// Newest archived hour (epoch ms).
    pub last_hour: Option<i64>,
    pub last_error: Option<String>,
}

const ARCHIVE_SUMMARY: &str = "SELECT a.id::text, a.name, a.endpoint, a.region, a.bucket, a.prefix, a.path_style, a.access_key, a.query, a.enabled, \
    (extract(epoch FROM a.created_at) * 1000)::bigint AS created_at, \
    count(r.hour) FILTER (WHERE r.status = 'done') AS hours_done, count(r.hour) FILTER (WHERE r.status = 'failed') AS hours_failed, \
    COALESCE(sum(r.rows), 0)::bigint AS rows, COALESCE(sum(r.bytes), 0)::bigint AS bytes, \
    (extract(epoch FROM max(r.hour) FILTER (WHERE r.status = 'done')) * 1000)::bigint AS last_hour, \
    (array_agg(r.error ORDER BY r.finished_at DESC) FILTER (WHERE r.status = 'failed'))[1] AS last_error \
    FROM log_archives a LEFT JOIN archive_runs r ON r.archive_id = a.id";

#[utoipa::path(get, path = "/api/v1/logs/archives", operation_id = "list_log_archives", responses((status = 200, body = Vec<LogArchive>)), tag = "archives")]
pub async fn list(State(st): State<AppState>, OrgId(org): OrgId) -> ApiResult<Vec<LogArchive>> {
    let rows = sqlx::query_as(&format!("{ARCHIVE_SUMMARY} WHERE a.org_id = $1 GROUP BY a.id ORDER BY a.created_at"))
        .bind(&org)
        .fetch_all(&st.postgres)
        .await?;
    Ok(Json(rows))
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ArchiveTarget {
    pub endpoint: String,
    #[serde(default = "default_region")]
    pub region: String,
    pub bucket: String,
    #[serde(default)]
    pub prefix: String,
    #[serde(default = "default_true")]
    pub path_style: bool,
    pub access_key: String,
    pub secret_key: String,
}

fn default_region() -> String {
    "us-east-1".into()
}
fn default_true() -> bool {
    true
}

impl ArchiveTarget {
    fn validate(&self) -> Result<(), ApiError> {
        if !(self.endpoint.starts_with("http://") || self.endpoint.starts_with("https://")) {
            return Err(ApiError::BadRequest("Endpoint must be an http(s) URL".into()));
        }
        let b = &self.bucket;
        if b.len() < 3 || b.len() > 63 || !b.bytes().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-' || c == b'.') {
            return Err(ApiError::BadRequest("Bucket names are 3–63 lowercase letters, digits, dots or dashes".into()));
        }
        if self.prefix.len() > 200 || self.prefix.contains("..") {
            return Err(ApiError::BadRequest("Invalid prefix".into()));
        }
        if self.access_key.is_empty() || self.secret_key.is_empty() {
            return Err(ApiError::BadRequest("Access key and secret key are required".into()));
        }
        Ok(())
    }
    fn store(&self) -> Result<Store, ApiError> {
        Store::new(&self.endpoint, &self.region, &self.bucket, self.path_style, &self.access_key, &self.secret_key)
    }
}

#[derive(Serialize, ToSchema)]
pub struct ArchiveTestResult {
    pub ok: bool,
    pub error: Option<String>,
}

/// Checks that the bucket can be written, read and deleted from with these credentials.
#[utoipa::path(post, path = "/api/v1/logs/archives/test", operation_id = "test_log_archive", request_body = ArchiveTarget, responses((status = 200, body = ArchiveTestResult)), tag = "archives")]
pub async fn test(Admin(_ctx): Admin, Json(t): Json<ArchiveTarget>) -> ApiResult<ArchiveTestResult> {
    let r = async {
        t.validate()?;
        t.store()?.probe(&t.prefix).await
    }
    .await;
    Ok(Json(match r {
        Ok(()) => ArchiveTestResult { ok: true, error: None },
        Err(e) => ArchiveTestResult { ok: false, error: Some(e.to_string()) },
    }))
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct CreateArchive {
    pub name: String,
    #[serde(flatten)]
    pub target: ArchiveTarget,
    /// Which logs to archive (search syntax); empty = all.
    #[serde(default)]
    pub query: String,
    /// Also archive the logs of the last N days still held in the raw table (0–2 by default, see OC_RAW_LOGS_HOURS).
    #[serde(default)]
    pub backfill_days: i64,
}

/// Creates an archive after proving the bucket is writable.
#[utoipa::path(post, path = "/api/v1/logs/archives", operation_id = "create_log_archive", request_body = CreateArchive, responses((status = 200, body = LogArchive), (status = 400, body = crate::error::ErrorBody)), tag = "archives")]
pub async fn create(State(st): State<AppState>, Admin(ctx): Admin, Json(req): Json<CreateArchive>) -> ApiResult<LogArchive> {
    let name = req.name.trim();
    if name.is_empty() || name.chars().count() > 100 {
        return Err(ApiError::BadRequest("Name must be 1–100 characters".into()));
    }
    req.target.validate()?;
    parse_query(&req.query);
    req.target.store()?.probe(&req.target.prefix).await?;
    let max_backfill = (st.config.retention.raw_logs_hours as i64 / 24 - 1).max(0);
    let id = insert_archive(&st, &ctx.org_id, name, &req.target, &req.query, req.backfill_days.clamp(0, max_backfill), Some(&ctx.user_id)).await?;
    ctx.audit(&st, "log_archive.created", name, &format!("{}/{}", req.target.bucket, req.target.prefix)).await;
    get_summary(&st, &ctx.org_id, &id).await.map(Json)
}

async fn insert_archive(st: &AppState, org: &str, name: &str, t: &ArchiveTarget, query: &str, backfill_days: i64, created_by: Option<&str>) -> Result<String, ApiError> {
    let start = (now_ms() / HOUR_MS) * HOUR_MS - backfill_days * 24 * HOUR_MS;
    let (id,): (String,) = sqlx::query_as(
        "INSERT INTO log_archives (org_id, name, endpoint, region, bucket, prefix, path_style, access_key, secret_key_enc, query, start_hour, created_by) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, to_timestamp($11 / 1000.0), $12::uuid) RETURNING id::text",
    )
    .bind(org)
    .bind(name)
    .bind(t.endpoint.trim_end_matches('/'))
    .bind(&t.region)
    .bind(&t.bucket)
    .bind(t.prefix.trim_matches('/'))
    .bind(t.path_style)
    .bind(&t.access_key)
    .bind(st.secrets.encrypt(&t.secret_key))
    .bind(query.trim())
    .bind(start)
    .bind(created_by)
    .fetch_one(&st.postgres)
    .await
    .map_err(|e| match &e {
        sqlx::Error::Database(d) if d.is_unique_violation() => ApiError::BadRequest(format!("An archive named '{name}' already exists")),
        _ => e.into(),
    })?;
    Ok(id)
}

async fn get_summary(st: &AppState, org: &str, id: &str) -> Result<LogArchive, ApiError> {
    sqlx::query_as(&format!("{ARCHIVE_SUMMARY} WHERE a.org_id = $1 AND a.id::text = $2 GROUP BY a.id"))
        .bind(org)
        .bind(id)
        .fetch_optional(&st.postgres)
        .await?
        .ok_or_else(|| ApiError::NotFound("Archive not found".into()))
}

#[derive(Deserialize, ToSchema)]
pub struct ArchivePatch {
    pub name: Option<String>,
    pub enabled: Option<bool>,
    pub query: Option<String>,
}

#[utoipa::path(patch, path = "/api/v1/logs/archives/{id}", operation_id = "update_log_archive", params(("id" = String, Path)), request_body = ArchivePatch, responses((status = 200, body = LogArchive)), tag = "archives")]
pub async fn update(State(st): State<AppState>, Admin(ctx): Admin, Path(id): Path<String>, Json(p): Json<ArchivePatch>) -> ApiResult<LogArchive> {
    let a = load_archive(&st, &ctx.org_id, &id).await?;
    let name = p.name.as_deref().map(str::trim).filter(|n| !n.is_empty() && n.chars().count() <= 100);
    sqlx::query("UPDATE log_archives SET name = COALESCE($3, name), enabled = COALESCE($4, enabled), query = COALESCE($5, query), updated_at = now() WHERE org_id = $1 AND id::text = $2")
        .bind(&ctx.org_id)
        .bind(&id)
        .bind(name)
        .bind(p.enabled)
        .bind(p.query.as_deref().map(str::trim))
        .execute(&st.postgres)
        .await?;
    ctx.audit(&st, "log_archive.updated", &a.name, &format!("enabled={:?}", p.enabled)).await;
    get_summary(&st, &ctx.org_id, &id).await.map(Json)
}

/// Stops archiving and forgets the archive; files already in the bucket are left there.
#[utoipa::path(delete, path = "/api/v1/logs/archives/{id}", operation_id = "delete_log_archive", params(("id" = String, Path)), responses((status = 204)), tag = "archives")]
pub async fn delete(State(st): State<AppState>, Admin(ctx): Admin, Path(id): Path<String>) -> Result<StatusCode, ApiError> {
    let a = load_archive(&st, &ctx.org_id, &id).await?;
    sqlx::query("DELETE FROM log_archives WHERE org_id = $1 AND id::text = $2").bind(&ctx.org_id).bind(&id).execute(&st.postgres).await?;
    ctx.audit(&st, "log_archive.deleted", &a.name, "").await;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Serialize, ToSchema, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct ArchiveRun {
    pub hour: i64,
    pub status: String,
    pub rows: i64,
    pub bytes: i64,
    pub object_key: Option<String>,
    pub error: Option<String>,
    pub attempts: i32,
    pub finished_at: i64,
}

#[utoipa::path(get, path = "/api/v1/logs/archives/{id}/runs", operation_id = "log_archive_runs", params(("id" = String, Path)), responses((status = 200, body = Vec<ArchiveRun>)), tag = "archives")]
pub async fn runs(State(st): State<AppState>, OrgId(org): OrgId, Path(id): Path<String>) -> ApiResult<Vec<ArchiveRun>> {
    load_archive(&st, &org, &id).await?;
    let rows = sqlx::query_as(
        "SELECT (extract(epoch FROM hour) * 1000)::bigint AS hour, status, rows, bytes, object_key, error, attempts, (extract(epoch FROM finished_at) * 1000)::bigint AS finished_at \
         FROM archive_runs WHERE archive_id = $1::uuid ORDER BY hour DESC LIMIT 200",
    )
    .bind(&id)
    .fetch_all(&st.postgres)
    .await?;
    Ok(Json(rows))
}

#[derive(Serialize, ToSchema)]
pub struct ArchiveNowResult {
    /// Hours written by this run.
    pub hours: usize,
    pub rows: i64,
}

/// Archives every pending closed hour now (instead of waiting for the background job).
#[utoipa::path(post, path = "/api/v1/logs/archives/{id}/run", operation_id = "run_log_archive", params(("id" = String, Path)), responses((status = 200, body = ArchiveNowResult)), tag = "archives")]
pub async fn run_now(State(st): State<AppState>, Admin(ctx): Admin, Path(id): Path<String>) -> ApiResult<ArchiveNowResult> {
    let a = load_archive(&st, &ctx.org_id, &id).await?;
    if !a.enabled {
        return Err(ApiError::BadRequest("This archive is paused. Enable it first.".into()));
    }
    let (hours, rows) = archive_pending(&st, &a, 24 * 15, 0).await?;
    Ok(Json(ArchiveNowResult { hours, rows }))
}

// ================================================================ writing

/// Closed hours of `a` that still need writing, oldest first.
async fn pending_hours(st: &AppState, a: &ArchiveRow, grace_ms: i64, limit: usize) -> Result<Vec<i64>, ApiError> {
    let now = now_ms();
    // Archives read the raw table; hours older than it keeps can't be archived anymore.
    let raw_ms = st.config.retention.raw_logs_hours as i64 * HOUR_MS;
    let first = a.start_hour.max(((now - raw_ms) / HOUR_MS + 1) * HOUR_MS);
    let last = ((now - grace_ms) / HOUR_MS) * HOUR_MS - HOUR_MS;
    if last < first {
        return Ok(Vec::new());
    }
    let settled: Vec<i64> = sqlx::query_scalar(
        "SELECT (extract(epoch FROM hour) * 1000)::bigint FROM archive_runs WHERE archive_id = $1::uuid AND hour >= to_timestamp($2 / 1000.0) \
         AND (status = 'done' OR attempts >= $3)",
    )
    .bind(&a.id)
    .bind(first)
    .bind(MAX_ATTEMPTS)
    .fetch_all(&st.postgres)
    .await?;
    let settled: std::collections::HashSet<i64> = settled.into_iter().collect();
    Ok((0..).map(|i| first + i * HOUR_MS).take_while(|h| *h <= last).filter(|h| !settled.contains(h)).take(limit).collect())
}

/// Writes up to `limit` pending hours; returns (hours written, rows).
async fn archive_pending(st: &AppState, a: &ArchiveRow, limit: usize, grace_ms: i64) -> Result<(usize, i64), ApiError> {
    let store = a.store(st)?;
    let mut written = 0;
    let mut total = 0;
    for hour in pending_hours(st, a, grace_ms, limit).await? {
        match archive_hour(st, a, &store, hour).await {
            Ok((rows, bytes, key)) => {
                record_run(st, &a.id, hour, "done", rows, bytes, key.as_deref(), None).await?;
                written += 1;
                total += rows;
            }
            Err(e) => {
                tracing::warn!(archive = %a.name, org = %a.org_id, hour, error = %e, "archiving an hour failed");
                record_run(st, &a.id, hour, "failed", 0, 0, None, Some(&e.to_string())).await?;
                // The store is probably unreachable: try again on the next tick.
                break;
            }
        }
    }
    Ok((written, total))
}

#[allow(clippy::too_many_arguments)]
async fn record_run(st: &AppState, archive: &str, hour: i64, status: &str, rows: i64, bytes: i64, key: Option<&str>, error: Option<&str>) -> Result<(), ApiError> {
    sqlx::query(
        "INSERT INTO archive_runs (archive_id, hour, status, rows, bytes, object_key, error) VALUES ($1::uuid, to_timestamp($2 / 1000.0), $3, $4, $5, $6, $7) \
         ON CONFLICT (archive_id, hour) DO UPDATE SET status = EXCLUDED.status, rows = EXCLUDED.rows, bytes = EXCLUDED.bytes, object_key = EXCLUDED.object_key, \
         error = EXCLUDED.error, finished_at = now(), attempts = archive_runs.attempts + 1",
    )
    .bind(archive)
    .bind(hour)
    .bind(status)
    .bind(rows)
    .bind(bytes)
    .bind(key)
    .bind(error)
    .execute(&st.postgres)
    .await?;
    Ok(())
}

/// One hour → one Parquet object. Returns (rows, bytes, object key); empty hours write nothing.
async fn archive_hour(st: &AppState, a: &ArchiveRow, store: &Store, hour: i64) -> Result<(i64, i64, Option<String>), ApiError> {
    let f = build_where(&LogsSchema, &parse_query(&a.query), None);
    let clause = format!("org_id = ? AND timestamp >= fromUnixTimestamp64Milli(?) AND timestamp < fromUnixTimestamp64Milli(?) AND ({})", f.clause);
    let mut params = vec![Param::Str(a.org_id.clone()), Param::Int(hour), Param::Int(hour + HOUR_MS)];
    params.extend(f.params);
    let rows: u64 = bind(st.clickhouse.query(&format!("SELECT count() FROM logs_raw WHERE {clause}")), &params).fetch_one().await?;
    if rows == 0 {
        return Ok((0, 0, None));
    }
    // Strings as UTF-8 strings (not binary) so every Parquet reader sees text.
    let ch = st.clickhouse.clone().with_option("output_format_parquet_string_as_string", "1").with_option("output_format_parquet_compression_method", "zstd");
    let mut cursor = bind(ch.query(&format!("SELECT {ARCHIVE_SELECT} FROM logs_raw WHERE {clause} ORDER BY timestamp")), &params).fetch_bytes("Parquet")?;
    let mut body = Vec::new();
    while let Some(chunk) = cursor.next().await? {
        body.extend_from_slice(&chunk);
    }
    let key = format!("{}archive-{}.parquet", partition(&a.prefix, &a.org_id, hour), &a.id[..8]);
    let bytes = body.len() as i64;
    store.put(&key, body, "application/vnd.apache.parquet").await?;
    Ok((rows as i64, bytes, Some(key)))
}

// ================================================================ archive files → local cache

/// The archive's objects for the org-hours overlapping `range` (only Parquet files).
async fn objects_in_range(store: &Store, a: &ArchiveRow, range: TimeRange) -> Result<Vec<crate::s3::Object>, ApiError> {
    let mut out = Vec::new();
    let mut hour = (range.from / HOUR_MS) * HOUR_MS;
    while hour <= range.to {
        out.extend(store.list(&partition(&a.prefix, &a.org_id, hour)).await?.into_iter().filter(|o| o.key.ends_with(".parquet")));
        if out.len() > MAX_SEARCH_FILES {
            return Err(ApiError::BadRequest(format!("More than {MAX_SEARCH_FILES} archive files in this range; narrow it")));
        }
        hour += HOUR_MS;
    }
    Ok(out)
}

/// Local copy of an archive object (downloaded once, reused while the size matches).
async fn cached(st: &AppState, store: &Store, a: &ArchiveRow, o: &crate::s3::Object) -> Result<PathBuf, ApiError> {
    let safe: String = o.key.chars().map(|c| if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '=' | '/') { c } else { '_' }).collect();
    let path = PathBuf::from(&st.config.data_dir).join("archive-cache").join(&a.id).join(safe.trim_start_matches('/').replace("..", "_"));
    if tokio::fs::metadata(&path).await.map(|m| m.len() == o.size).unwrap_or(false) {
        return Ok(path);
    }
    let body = store.get(&o.key).await?;
    if let Some(dir) = path.parent() {
        tokio::fs::create_dir_all(dir).await.map_err(|e| ApiError::Internal(e.into()))?;
    }
    tokio::fs::write(&path, body).await.map_err(|e| ApiError::Internal(e.into()))?;
    Ok(path)
}

/// `['a.parquet', 'b.parquet']` for DuckDB (paths are ours; quotes doubled anyway).
fn file_list(files: &[PathBuf]) -> String {
    format!("[{}]", files.iter().map(|p| format!("'{}'", p.display().to_string().replace('\'', "''"))).collect::<Vec<_>>().join(", "))
}

fn duck_params(params: &[Param]) -> Vec<duckdb::types::Value> {
    params
        .iter()
        .map(|p| match p {
            Param::Str(s) => duckdb::types::Value::Text(s.clone()),
            Param::Int(i) => duckdb::types::Value::BigInt(*i),
        })
        .collect()
}

fn duck_err(e: duckdb::Error) -> ApiError {
    ApiError::BadRequest(format!("Archive query failed: {e}"))
}

// ================================================================ archive search

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ArchiveSearchRequest {
    pub archive_id: String,
    #[serde(default)]
    pub query: String,
    pub range: TimeRange,
    /// Events kept (newest first), 1–5000. Default 1000.
    pub max_results: Option<i32>,
}

#[derive(Serialize, ToSchema, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct ArchiveSearch {
    pub id: String,
    pub archive_id: String,
    pub archive_name: String,
    pub query: String,
    pub from: i64,
    pub to: i64,
    /// running | done | failed | canceled
    pub status: String,
    pub files_total: i32,
    pub files_scanned: i32,
    pub bytes_scanned: i64,
    pub matches: i64,
    pub max_results: i32,
    pub error: Option<String>,
    pub created_at: i64,
    pub finished_at: Option<i64>,
}

const SEARCH_SELECT: &str = "SELECT s.id::text, s.archive_id::text, a.name AS archive_name, s.query, (extract(epoch FROM s.range_from) * 1000)::bigint AS from, \
    (extract(epoch FROM s.range_to) * 1000)::bigint AS to, s.status, s.files_total, s.files_scanned, s.bytes_scanned, s.matches, s.max_results, s.error, \
    (extract(epoch FROM s.created_at) * 1000)::bigint AS created_at, (extract(epoch FROM s.finished_at) * 1000)::bigint AS finished_at \
    FROM archive_searches s JOIN log_archives a ON a.id = s.archive_id";

/// Starts a search over archived logs; poll it with GET /api/v1/logs/archive-searches/{id}.
#[utoipa::path(post, path = "/api/v1/logs/archive-searches", operation_id = "create_archive_search", request_body = ArchiveSearchRequest, responses((status = 200, body = ArchiveSearch)), tag = "archives")]
pub async fn create_search(State(st): State<AppState>, ctx: AuthContext, Json(req): Json<ArchiveSearchRequest>) -> ApiResult<ArchiveSearch> {
    req.range.validate()?;
    let a = load_archive(&st, &ctx.org_id, &req.archive_id).await?;
    let max = req.max_results.unwrap_or(1000).clamp(1, 5000);
    let (id,): (String,) = sqlx::query_as(
        "INSERT INTO archive_searches (org_id, archive_id, query, range_from, range_to, max_results, created_by) \
         VALUES ($1, $2::uuid, $3, to_timestamp($4 / 1000.0), to_timestamp($5 / 1000.0), $6, $7::uuid) RETURNING id::text",
    )
    .bind(&ctx.org_id)
    .bind(&a.id)
    .bind(req.query.trim())
    .bind(req.range.from)
    .bind(req.range.to)
    .bind(max)
    .bind(&ctx.user_id)
    .fetch_one(&st.postgres)
    .await?;
    let (st2, id2, query) = (st.clone(), id.clone(), req.query.clone());
    tokio::spawn(async move {
        let r = run_search(&st2, &id2, &a, &query, req.range, max).await;
        let (status, error) = match r {
            Ok(()) => ("done", None),
            Err(e) => ("failed", Some(e.to_string())),
        };
        let _ = sqlx::query("UPDATE archive_searches SET status = $2, error = $3, finished_at = now() WHERE id = $1::uuid AND status = 'running'")
            .bind(&id2)
            .bind(status)
            .bind(error)
            .execute(&st2.postgres)
            .await;
    });
    get_search(&st, &ctx.org_id, &id).await.map(Json)
}

async fn get_search(st: &AppState, org: &str, id: &str) -> Result<ArchiveSearch, ApiError> {
    sqlx::query_as(&format!("{SEARCH_SELECT} WHERE s.org_id = $1 AND s.id::text = $2"))
        .bind(org)
        .bind(id)
        .fetch_optional(&st.postgres)
        .await?
        .ok_or_else(|| ApiError::NotFound("Search not found".into()))
}

async fn canceled(st: &AppState, search: &str) -> bool {
    sqlx::query_scalar::<_, String>("SELECT status FROM archive_searches WHERE id = $1::uuid").bind(search).fetch_optional(&st.postgres).await.ok().flatten().as_deref() != Some("running")
}

async fn run_search(st: &AppState, id: &str, a: &ArchiveRow, query: &str, range: TimeRange, max: i32) -> Result<(), ApiError> {
    let store = a.store(st)?;
    let objects = objects_in_range(&store, a, range).await?;
    sqlx::query("UPDATE archive_searches SET files_total = $2 WHERE id = $1::uuid").bind(id).bind(objects.len() as i32).execute(&st.postgres).await?;
    let mut files = Vec::new();
    for o in &objects {
        if canceled(st, id).await {
            return Ok(());
        }
        files.push(cached(st, &store, a, o).await?);
        sqlx::query("UPDATE archive_searches SET files_scanned = files_scanned + 1, bytes_scanned = bytes_scanned + $2 WHERE id = $1::uuid")
            .bind(id)
            .bind(o.size as i64)
            .execute(&st.postgres)
            .await?;
    }
    if files.is_empty() {
        return Ok(());
    }
    let terms = parse_query(query);
    let (events, matches) = tokio::task::spawn_blocking(move || duck_search(&files, &terms, range, max as usize)).await.map_err(|e| ApiError::Internal(e.into()))??;
    let mut tx = st.postgres.begin().await?;
    for (seq, ev) in events.iter().enumerate() {
        sqlx::query("INSERT INTO archive_search_hits (search_id, seq, event) VALUES ($1::uuid, $2, $3)").bind(id).bind(seq as i32).bind(ev).execute(&mut *tx).await?;
    }
    sqlx::query("UPDATE archive_searches SET matches = $2 WHERE id = $1::uuid").bind(id).bind(matches).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(())
}

/// DuckDB over the downloaded files: newest `max` matching events (as LogEvent JSON) and the total.
fn duck_search(files: &[PathBuf], terms: &[oc_query::Term], range: TimeRange, max: usize) -> Result<(Vec<Value>, i64), ApiError> {
    let f = build_where_duckdb(terms);
    let from = format!("read_parquet({}, union_by_name = true)", file_list(files));
    let clause = format!("epoch_ms(timestamp) BETWEEN ? AND ? AND ({})", f.clause);
    let mut params = vec![Param::Int(range.from), Param::Int(range.to)];
    params.extend(f.params);
    let conn = duckdb::Connection::open_in_memory().map_err(duck_err)?;
    let total: i64 = conn.query_row(&format!("SELECT count(*) FROM {from} WHERE {clause}"), duckdb::params_from_iter(duck_params(&params)), |r| r.get(0)).map_err(duck_err)?;
    let mut stmt = conn
        .prepare(&format!(
            "SELECT id, epoch_ms(timestamp), status, service, host, source, message, to_json(tags)::VARCHAR, attributes, trace_id, span_id \
             FROM {from} WHERE {clause} ORDER BY timestamp DESC LIMIT {max}"
        ))
        .map_err(duck_err)?;
    let rows = stmt
        .query_map(duckdb::params_from_iter(duck_params(&params)), |r| {
            let tags: String = r.get(7)?;
            let attributes: String = r.get(8)?;
            let opt = |s: String| (!s.is_empty()).then_some(s);
            Ok(LogEvent {
                id: r.get(0)?,
                timestamp: r.get(1)?,
                status: r.get(2)?,
                service: r.get(3)?,
                host: r.get(4)?,
                source: r.get(5)?,
                message: r.get(6)?,
                tags: serde_json::from_str(&tags).unwrap_or_default(),
                attributes: serde_json::from_str(&attributes).unwrap_or(Value::Null),
                trace_id: r.get::<_, Option<String>>(9)?.and_then(opt),
                span_id: r.get::<_, Option<String>>(10)?.and_then(opt),
            })
        })
        .map_err(duck_err)?;
    let mut events = Vec::new();
    for r in rows {
        events.push(serde_json::to_value(r.map_err(duck_err)?).map_err(|e| ApiError::Internal(e.into()))?);
    }
    Ok((events, total))
}

#[utoipa::path(get, path = "/api/v1/logs/archive-searches", operation_id = "list_archive_searches", responses((status = 200, body = Vec<ArchiveSearch>)), tag = "archives")]
pub async fn list_searches(State(st): State<AppState>, OrgId(org): OrgId) -> ApiResult<Vec<ArchiveSearch>> {
    let rows = sqlx::query_as(&format!("{SEARCH_SELECT} WHERE s.org_id = $1 ORDER BY s.created_at DESC LIMIT 50")).bind(&org).fetch_all(&st.postgres).await?;
    Ok(Json(rows))
}

#[derive(Deserialize, IntoParams)]
pub struct HitsParams {
    #[serde(default)]
    pub offset: i64,
    pub limit: Option<i64>,
}

#[derive(Serialize, ToSchema)]
pub struct ArchiveSearchDetail {
    pub search: ArchiveSearch,
    pub events: Vec<LogEvent>,
}

#[utoipa::path(get, path = "/api/v1/logs/archive-searches/{id}", operation_id = "get_archive_search", params(("id" = String, Path), HitsParams), responses((status = 200, body = ArchiveSearchDetail)), tag = "archives")]
pub async fn get_search_detail(State(st): State<AppState>, OrgId(org): OrgId, Path(id): Path<String>, Query(p): Query<HitsParams>) -> ApiResult<ArchiveSearchDetail> {
    let search = get_search(&st, &org, &id).await?;
    let raw: Vec<Value> = sqlx::query_scalar("SELECT event FROM archive_search_hits WHERE search_id = $1::uuid ORDER BY seq OFFSET $2 LIMIT $3")
        .bind(&id)
        .bind(p.offset.max(0))
        .bind(p.limit.unwrap_or(100).clamp(1, 1000))
        .fetch_all(&st.postgres)
        .await?;
    let events = raw.into_iter().filter_map(|v| serde_json::from_value(v).ok()).collect();
    Ok(Json(ArchiveSearchDetail { search, events }))
}

/// Cancels a running search, or deletes a finished one with its results.
#[utoipa::path(delete, path = "/api/v1/logs/archive-searches/{id}", operation_id = "delete_archive_search", params(("id" = String, Path)), responses((status = 204)), tag = "archives")]
pub async fn delete_search(State(st): State<AppState>, OrgId(org): OrgId, Path(id): Path<String>) -> Result<StatusCode, ApiError> {
    let s = get_search(&st, &org, &id).await?;
    if s.status == "running" {
        sqlx::query("UPDATE archive_searches SET status = 'canceled', finished_at = now() WHERE id = $1::uuid").bind(&id).execute(&st.postgres).await?;
    } else {
        sqlx::query("DELETE FROM archive_searches WHERE id = $1::uuid").bind(&id).execute(&st.postgres).await?;
    }
    Ok(StatusCode::NO_CONTENT)
}

// ================================================================ rehydration

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct RehydrateRequest {
    pub archive_id: String,
    pub name: String,
    #[serde(default)]
    pub query: String,
    pub range: TimeRange,
    /// Days the rehydrated logs stay searchable (1–30). Default 7.
    pub retention_days: Option<i64>,
}

#[derive(Serialize, ToSchema, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct Rehydration {
    pub id: String,
    pub archive_id: String,
    pub archive_name: String,
    pub name: String,
    pub query: String,
    pub from: i64,
    pub to: i64,
    /// running | done | failed | expired
    pub status: String,
    pub rows: i64,
    pub error: Option<String>,
    pub expires_at: i64,
    pub created_at: i64,
}

const REHYDRATION_SELECT: &str = "SELECT r.id::text, r.archive_id::text, a.name AS archive_name, r.name, r.query, (extract(epoch FROM r.range_from) * 1000)::bigint AS from, \
    (extract(epoch FROM r.range_to) * 1000)::bigint AS to, r.status, r.rows, r.error, (extract(epoch FROM r.expires_at) * 1000)::bigint AS expires_at, \
    (extract(epoch FROM r.created_at) * 1000)::bigint AS created_at FROM rehydrations r JOIN log_archives a ON a.id = r.archive_id";

/// Loads archived logs back into ClickHouse; search them in the Log Explorer with `index:rehydrated`.
#[utoipa::path(post, path = "/api/v1/logs/rehydrations", operation_id = "create_rehydration", request_body = RehydrateRequest, responses((status = 200, body = Rehydration)), tag = "archives")]
pub async fn create_rehydration(State(st): State<AppState>, Writer(ctx): Writer, Json(req): Json<RehydrateRequest>) -> ApiResult<Rehydration> {
    req.range.validate()?;
    let name = req.name.trim();
    if name.is_empty() || name.chars().count() > 100 {
        return Err(ApiError::BadRequest("Name must be 1–100 characters".into()));
    }
    let a = load_archive(&st, &ctx.org_id, &req.archive_id).await?;
    let days = req.retention_days.unwrap_or(7).clamp(1, 30);
    let (id,): (String,) = sqlx::query_as(
        "INSERT INTO rehydrations (org_id, archive_id, name, query, range_from, range_to, expires_at, created_by) \
         VALUES ($1, $2::uuid, $3, $4, to_timestamp($5 / 1000.0), to_timestamp($6 / 1000.0), now() + make_interval(days => $7), $8::uuid) RETURNING id::text",
    )
    .bind(&ctx.org_id)
    .bind(&a.id)
    .bind(name)
    .bind(req.query.trim())
    .bind(req.range.from)
    .bind(req.range.to)
    .bind(days as i32)
    .bind(&ctx.user_id)
    .fetch_one(&st.postgres)
    .await?;
    ctx.audit(&st, "rehydration.created", name, &req.query).await;
    let (st2, id2, query) = (st.clone(), id.clone(), req.query.clone());
    tokio::spawn(async move {
        let r = run_rehydration(&st2, &id2, &a, &query, req.range).await;
        let (status, rows, error) = match r {
            Ok(rows) => ("done", rows, None),
            Err(e) => ("failed", 0, Some(e.to_string())),
        };
        let _ = sqlx::query("UPDATE rehydrations SET status = $2, rows = $3, error = $4, finished_at = now() WHERE id = $1::uuid")
            .bind(&id2)
            .bind(status)
            .bind(rows)
            .bind(error)
            .execute(&st2.postgres)
            .await;
    });
    get_rehydration(&st, &ctx.org_id, &id).await.map(Json)
}

async fn get_rehydration(st: &AppState, org: &str, id: &str) -> Result<Rehydration, ApiError> {
    sqlx::query_as(&format!("{REHYDRATION_SELECT} WHERE r.org_id = $1 AND r.id::text = $2"))
        .bind(org)
        .bind(id)
        .fetch_optional(&st.postgres)
        .await?
        .ok_or_else(|| ApiError::NotFound("Rehydration not found".into()))
}

async fn run_rehydration(st: &AppState, id: &str, a: &ArchiveRow, query: &str, range: TimeRange) -> Result<i64, ApiError> {
    let store = a.store(st)?;
    let mut files = Vec::new();
    for o in objects_in_range(&store, a, range).await? {
        files.push(cached(st, &store, a, &o).await?);
    }
    if files.is_empty() {
        return Ok(0);
    }
    let out = PathBuf::from(&st.config.data_dir).join("rehydrate").join(format!("{id}.parquet"));
    if let Some(dir) = out.parent() {
        tokio::fs::create_dir_all(dir).await.map_err(|e| ApiError::Internal(e.into()))?;
    }
    let (terms, org, rid, out2) = (parse_query(query), a.org_id.clone(), id.to_string(), out.clone());
    let rows = tokio::task::spawn_blocking(move || duck_extract(&files, &terms, range, &org, &rid, &out2)).await.map_err(|e| ApiError::Internal(e.into()))??;
    if rows > 0 {
        load_into_clickhouse(st, &out).await?;
    }
    let _ = tokio::fs::remove_file(&out).await;
    Ok(rows)
}

/// Filters the archive files into one Parquet file shaped like `oc.logs_rehydrated`. The org is set
/// from the archive (never from file contents).
fn duck_extract(files: &[PathBuf], terms: &[oc_query::Term], range: TimeRange, org: &str, rehydration: &str, out: &FsPath) -> Result<i64, ApiError> {
    let f = build_where_duckdb(terms);
    let mut params = vec![Param::Str(org.to_string()), Param::Str(rehydration.to_string()), Param::Int(range.from), Param::Int(range.to)];
    params.extend(f.params);
    let conn = duckdb::Connection::open_in_memory().map_err(duck_err)?;
    conn.execute(
        &format!(
            "CREATE TABLE r AS SELECT ?::VARCHAR AS org_id, id, timestamp, status, service, host, source, message, tags, attr, attributes, trace_id, span_id, \
             severity_number, scope, ?::VARCHAR AS rehydration_id FROM read_parquet({}, union_by_name = true) WHERE epoch_ms(timestamp) BETWEEN ? AND ? AND ({})",
            file_list(files),
            f.clause
        ),
        duckdb::params_from_iter(duck_params(&params)),
    )
    .map_err(duck_err)?;
    let rows: i64 = conn.query_row("SELECT count(*) FROM r", [], |r| r.get(0)).map_err(duck_err)?;
    if rows > MAX_REHYDRATE_ROWS {
        return Err(ApiError::BadRequest(format!("{rows} logs match; narrow the query or range (limit {MAX_REHYDRATE_ROWS})")));
    }
    if rows > 0 {
        conn.execute_batch(&format!("COPY r TO '{}' (FORMAT parquet)", out.display().to_string().replace('\'', "''"))).map_err(duck_err)?;
    }
    Ok(rows)
}

/// `INSERT … FORMAT Parquet` over ClickHouse's HTTP interface with the file as the body.
async fn load_into_clickhouse(st: &AppState, file: &FsPath) -> Result<(), ApiError> {
    let body = tokio::fs::read(file).await.map_err(|e| ApiError::Internal(e.into()))?;
    let res = reqwest::Client::new()
        .post(&st.config.clickhouse_url)
        .basic_auth(&st.config.clickhouse_user, Some(&st.config.clickhouse_password))
        .query(&[(
            "query",
            "INSERT INTO oc.logs_rehydrated (org_id, id, timestamp, status, service, host, source, message, tags, attr, attributes, trace_id, span_id, severity_number, scope, rehydration_id) FORMAT Parquet",
        )])
        .body(body)
        .send()
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;
    if !res.status().is_success() {
        let text = res.text().await.unwrap_or_default();
        return Err(ApiError::BadRequest(format!("Loading into ClickHouse failed: {}", text.chars().take(300).collect::<String>())));
    }
    Ok(())
}

#[utoipa::path(get, path = "/api/v1/logs/rehydrations", operation_id = "list_rehydrations", responses((status = 200, body = Vec<Rehydration>)), tag = "archives")]
pub async fn list_rehydrations(State(st): State<AppState>, OrgId(org): OrgId) -> ApiResult<Vec<Rehydration>> {
    let rows = sqlx::query_as(&format!("{REHYDRATION_SELECT} WHERE r.org_id = $1 ORDER BY r.created_at DESC LIMIT 100")).bind(&org).fetch_all(&st.postgres).await?;
    Ok(Json(rows))
}

/// Removes a rehydration's logs now.
#[utoipa::path(delete, path = "/api/v1/logs/rehydrations/{id}", operation_id = "expire_rehydration", params(("id" = String, Path)), responses((status = 204)), tag = "archives")]
pub async fn expire_rehydration(State(st): State<AppState>, Writer(ctx): Writer, Path(id): Path<String>) -> Result<StatusCode, ApiError> {
    let r = get_rehydration(&st, &ctx.org_id, &id).await?;
    expire(&st, &ctx.org_id, &r.id).await?;
    ctx.audit(&st, "rehydration.expired", &r.name, "").await;
    Ok(StatusCode::NO_CONTENT)
}

async fn expire(st: &AppState, org: &str, id: &str) -> Result<(), ApiError> {
    st.clickhouse.query("ALTER TABLE logs_rehydrated DELETE WHERE org_id = ? AND rehydration_id = ?").bind(org).bind(id).execute().await?;
    sqlx::query("UPDATE rehydrations SET status = 'expired' WHERE id = $1::uuid").bind(id).execute(&st.postgres).await?;
    Ok(())
}

// ================================================================ background job

/// Archives closed hours every 5 minutes and expires old rehydrations. One server at a time (advisory lock).
pub async fn run_archiver(st: AppState) {
    let mut tick = tokio::time::interval(Duration::from_secs(300));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tick.tick().await;
        if let Err(e) = archiver_tick(&st).await {
            tracing::warn!(error = %e, "archiver tick failed");
        }
    }
}

async fn archiver_tick(st: &AppState) -> Result<(), ApiError> {
    let mut conn = st.postgres.acquire().await?;
    let locked: bool = sqlx::query_scalar("SELECT pg_try_advisory_lock(7302)").fetch_one(&mut *conn).await?;
    if !locked {
        return Ok(());
    }
    let result = async {
        let archives: Vec<ArchiveRow> = sqlx::query_as(&format!("SELECT {ARCHIVE_COLS} FROM log_archives a JOIN orgs o ON o.id = a.org_id WHERE a.enabled AND o.status = 'active'"))
            .fetch_all(&st.postgres)
            .await?;
        for a in &archives {
            match archive_pending(st, a, HOURS_PER_TICK, GRACE_MS).await {
                Ok((n, rows)) if n > 0 => tracing::info!(archive = %a.name, org = %a.org_id, hours = n, rows, "archived logs"),
                Ok(_) => {}
                Err(e) => tracing::warn!(archive = %a.name, error = %e, "archive unavailable"),
            }
        }
        let due: Vec<(String, String)> = sqlx::query_as("SELECT org_id, id::text FROM rehydrations WHERE status = 'done' AND expires_at < now()").fetch_all(&st.postgres).await?;
        for (org, id) in due {
            expire(st, &org, &id).await?;
        }
        Ok::<_, ApiError>(())
    }
    .await;
    let _ = sqlx::query("SELECT pg_advisory_unlock(7302)").execute(&mut *conn).await;
    result
}

/// Development (`OC_DEMO_ARCHIVE_ENDPOINT`): creates the local bucket and an archive of the install's org.
pub async fn ensure_demo_archive(st: &AppState) -> anyhow::Result<()> {
    let Some(d) = &st.config.demo_archive else { return Ok(()) };
    let exists: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM log_archives WHERE org_id = $1)").bind(&st.config.dev_org).fetch_one(&st.postgres).await?;
    if exists {
        return Ok(());
    }
    let target = ArchiveTarget {
        endpoint: d.endpoint.clone(),
        region: default_region(),
        bucket: d.bucket.clone(),
        prefix: "logs".into(),
        path_style: true,
        access_key: d.access_key.clone(),
        secret_key: d.secret_key.clone(),
    };
    let setup = async {
        let store = target.store()?;
        store.ensure_bucket().await?;
        store.probe(&target.prefix).await?;
        insert_archive(st, &st.config.dev_org, "Local RustFS", &target, "", 1, None).await
    };
    match setup.await {
        Ok(_) => tracing::info!(endpoint = %d.endpoint, bucket = %d.bucket, "created the demo log archive"),
        Err(e) => tracing::warn!(error = %e, "demo log archive not created (is RustFS running?)"),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn partitions_are_hive_style_utc_hours() {
        // 2026-10-25T14:00:00Z
        let h = 1_792_936_800_000;
        assert_eq!(partition("logs", "acme", h), "logs/org=acme/dt=2026-10-25/hour=14/");
        assert_eq!(partition("", "acme", h), "org=acme/dt=2026-10-25/hour=14/");
        assert_eq!(hour_parts(0), ("1970-01-01".into(), "00".into()));
        assert_eq!(hour_parts(951_782_400_000), ("2000-02-29".into(), "00".into()));
    }

    #[test]
    fn file_lists_escape_quotes() {
        assert_eq!(file_list(&[PathBuf::from("/tmp/a'b.parquet")]), "['/tmp/a''b.parquet']");
    }
}
