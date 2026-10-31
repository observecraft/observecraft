//! Logs → Configuration → Migration: Quick Translation of a Splunk (SPL) search, and imports of
//! exported saved searches, alerts, dashboards and macros, reviewed and migrated to log monitors
//! and dashboards.
//!
//! - The translation itself is [`crate::spl`].
//! - An import stores each asset with its translation (Postgres `log_imports`, `log_import_assets`);
//!   migrating an alert creates a log monitor (`logs("…") by {…}` counting events), migrating a
//!   dashboard creates a dashboard with a log stream per panel (and a note with the panel's
//!   aggregation, which needs a log-based metric to be charted).
//! - Tenancy: every query filters on the org from the session.

use std::collections::HashMap;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use utoipa::ToSchema;

use crate::auth::{AuthContext, Writer};
use crate::error::{ApiError, ApiResult};
use crate::spl::{self, Macro, Macros, SplTranslation};
use crate::state::AppState;

/// Upload limit per file (CSV text).
const MAX_FILE: usize = 10 * 1024 * 1024;
/// Assets per import.
const MAX_ASSETS: usize = 2000;

// ================================================================ translate

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct TranslateRequest {
    /// spl
    #[serde(default = "spl_source")]
    pub source: String,
    pub query: String,
    /// The macros export (CSV with `title,definition,args`), if the query uses macros.
    pub macros: Option<String>,
}

fn spl_source() -> String {
    "spl".into()
}

#[utoipa::path(post, path = "/api/v1/logs/translate", operation_id = "translate_log_query", request_body = TranslateRequest, responses((status = 200, body = SplTranslation)), tag = "logs")]
pub async fn translate(_: AuthContext, Json(req): Json<TranslateRequest>) -> ApiResult<SplTranslation> {
    if req.source != "spl" {
        return Err(ApiError::BadRequest("only Splunk SPL can be translated".into()));
    }
    if req.query.trim().is_empty() || req.query.len() > 100_000 {
        return Err(ApiError::BadRequest("the query must be 1–100,000 characters".into()));
    }
    let macros = req.macros.as_deref().map(|m| spl::macros_from_rows(&spl::parse_csv(m))).unwrap_or_default();
    Ok(Json(spl::translate(&req.query, &macros)))
}

// ================================================================ imports: model

#[derive(Serialize, Deserialize, ToSchema, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ImportSearch {
    /// The panel title (dashboards) or the asset's title.
    pub title: String,
    pub spl: String,
    pub translation: SplTranslation,
}

/// What a migrated alert becomes.
#[derive(Serialize, Deserialize, ToSchema, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ImportAlert {
    /// above | above_or_equal | below | below_or_equal
    pub comparator: Option<String>,
    pub threshold: Option<f64>,
    pub window_minutes: i64,
    pub recipients: Vec<String>,
    /// The original schedule (`*/5 * * * *`), for reference: monitors evaluate every minute.
    pub schedule: Option<String>,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ImportAsset {
    pub id: String,
    /// alert | dashboard | search
    pub kind: String,
    pub title: String,
    pub description: Option<String>,
    /// ready | partial | unsupported | migrated | failed
    pub status: String,
    pub searches: Vec<ImportSearch>,
    pub alert: Option<ImportAlert>,
    /// Why it can't be migrated as is.
    pub issues: Vec<String>,
    pub target_kind: Option<String>,
    pub target_id: Option<String>,
    pub error: Option<String>,
    pub migrated_at: Option<i64>,
}

#[derive(Serialize, ToSchema, Default)]
#[serde(rename_all = "camelCase")]
pub struct ImportCounts {
    pub total: i64,
    pub alerts: i64,
    pub dashboards: i64,
    pub searches: i64,
    pub ready: i64,
    pub partial: i64,
    pub unsupported: i64,
    pub migrated: i64,
    pub failed: i64,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct LogImport {
    pub id: String,
    pub name: String,
    /// splunk
    pub source: String,
    pub created_at: i64,
    pub created_by: Option<String>,
    pub macros: usize,
    pub counts: ImportCounts,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct LogImportDetail {
    #[serde(flatten)]
    pub import: LogImport,
    pub assets: Vec<ImportAsset>,
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct CreateImport {
    pub name: String,
    /// splunk
    #[serde(default = "splunk")]
    pub source: String,
    /// Saved searches and alerts export (CSV).
    pub alerts: Option<String>,
    /// Dashboards export (CSV with the Simple XML in `eai:data`).
    pub dashboards: Option<String>,
    /// Macros export (CSV with `title,definition,args`).
    pub macros: Option<String>,
}

fn splunk() -> String {
    "splunk".into()
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct MigrateRequest {
    /// The assets to migrate; empty: every ready or partial one not migrated yet.
    #[serde(default)]
    pub asset_ids: Vec<String>,
}

// ================================================================ imports: building

struct NewAsset {
    kind: &'static str,
    title: String,
    original: Value,
    searches: Vec<ImportSearch>,
    alert: Option<ImportAlert>,
    issues: Vec<String>,
}

impl NewAsset {
    fn status(&self) -> &'static str {
        if self.searches.is_empty() || self.searches.iter().all(|s| s.translation.query.is_empty() && s.translation.aggregation.is_none()) {
            "unsupported"
        } else if self.issues.is_empty() && self.searches.iter().all(|s| s.translation.complete) {
            "ready"
        } else {
            "partial"
        }
    }
}

fn get<'a>(r: &'a HashMap<String, String>, k: &str) -> Option<&'a str> {
    r.get(k).map(|s| s.trim()).filter(|s| !s.is_empty())
}

/// `-15m`, `-1h@h`, `rt-5m`, `-7d` → minutes.
fn window_minutes(earliest: Option<&str>) -> Option<i64> {
    let e = earliest?.trim().trim_start_matches("rt");
    let e = e.split('@').next()?.trim_start_matches('-');
    let split = e.find(|c: char| !c.is_ascii_digit())?;
    let n: i64 = if split == 0 { 1 } else { e[..split].parse().ok()? };
    let unit = &e[split..];
    let m = match unit {
        "s" | "sec" | "secs" | "second" | "seconds" => (n + 59) / 60,
        "m" | "min" | "mins" | "minute" | "minutes" => n,
        "h" | "hr" | "hrs" | "hour" | "hours" => n * 60,
        "d" | "day" | "days" => n * 1440,
        "w" | "week" | "weeks" => n * 10_080,
        _ => return None,
    };
    Some(m.max(1))
}

fn comparator(c: &str) -> Option<&'static str> {
    Some(match c.to_ascii_lowercase().as_str() {
        "greater than" | ">" => "above",
        ">=" => "above_or_equal",
        "less than" | "<" => "below",
        "<=" => "below_or_equal",
        _ => return None,
    })
}

fn alert_asset(r: &HashMap<String, String>, macros: &Macros) -> NewAsset {
    let title = get(r, "title").unwrap_or("Untitled search").to_string();
    let spl = get(r, "search").or_else(|| get(r, "qualifiedSearch")).unwrap_or_default().to_string();
    let translation = spl::translate(&spl, macros);
    let alert_type = get(r, "alert_type").unwrap_or_default().to_ascii_lowercase();
    let scheduled = get(r, "is_scheduled").is_some_and(|v| v == "1" || v.eq_ignore_ascii_case("true"));
    let is_alert = (!alert_type.is_empty() && alert_type != "always") || get(r, "alert_comparator").is_some() || (alert_type == "always" && scheduled);
    let mut issues = vec![];
    let mut alert = None;
    if is_alert {
        let mut comparator_ = get(r, "alert_comparator").and_then(comparator).map(String::from);
        let mut threshold = get(r, "alert_threshold").and_then(|t| t.parse::<f64>().ok());
        if alert_type == "always" {
            comparator_ = Some("above".into());
            threshold = Some(0.0);
        }
        // `… | stats count | where count > 10`: the condition is in the search.
        if let Some(c) = translation.aggregation.as_ref().and_then(|a| a.condition.as_ref()) {
            comparator_ = comparator(&c.op).map(String::from);
            threshold = Some(c.value);
        }
        if let Some(c) = get(r, "alert_comparator").filter(|c| comparator(c).is_none()) {
            issues.push(format!("the \"{c}\" condition has no log monitor equivalent: set the threshold on the monitor"));
        }
        if alert_type.starts_with("number of hosts") || alert_type.starts_with("number of sources") {
            issues.push(format!("\"{alert_type}\" alerts count events: group the monitor by host or source instead"));
        }
        if let Some(a) = &translation.aggregation {
            if a.measures.iter().any(|m| m.function != "count") {
                issues.push("log monitors count events: this alert measures another value, so it needs a log-based metric and a metric monitor".into());
            }
            if a.group_by.len() > 4 {
                issues.push("log monitors group by at most 4 attributes".into());
            }
        }
        if threshold.is_none() {
            issues.push("no alert threshold: set one on the monitor".into());
        }
        let recipients = get(r, "action.email.to").map(|t| t.split([',', ';', ' ']).map(str::trim).filter(|s| s.contains('@')).map(String::from).collect()).unwrap_or_default();
        alert = Some(ImportAlert {
            comparator: comparator_,
            threshold,
            window_minutes: window_minutes(get(r, "dispatch.earliest_time")).unwrap_or(15).clamp(1, 1440),
            recipients,
            schedule: get(r, "cron_schedule").map(String::from),
        });
    }
    NewAsset {
        kind: if is_alert { "alert" } else { "search" },
        title: title.clone(),
        original: json!(r),
        searches: vec![ImportSearch { title, spl, translation }],
        alert,
        issues,
    }
}

fn dashboard_asset(r: &HashMap<String, String>, macros: &Macros) -> Option<NewAsset> {
    let xml = get(r, "eai:data").or_else(|| get(r, "data"))?;
    let mut original = json!(r);
    original["eai:data"] = json!(format!("{} characters of Simple XML", xml.len()));
    let mut issues = vec![];
    let Some(d) = spl::parse_simple_xml(xml) else {
        return Some(NewAsset {
            kind: "dashboard",
            title: get(r, "label").or_else(|| get(r, "title")).unwrap_or("Untitled dashboard").to_string(),
            original,
            searches: vec![],
            alert: None,
            issues: vec!["only Simple XML dashboards can be imported (Dashboard Studio JSON isn't supported)".into()],
        });
    };
    let title = d.label.clone().or_else(|| get(r, "label").map(String::from)).or_else(|| get(r, "title").map(String::from)).unwrap_or_else(|| "Untitled dashboard".into());
    let searches: Vec<ImportSearch> = d
        .panels
        .iter()
        .enumerate()
        .map(|(i, p)| ImportSearch { title: if p.title.is_empty() { format!("Panel {}", i + 1) } else { p.title.clone() }, spl: p.query.clone(), translation: spl::translate(&p.query, macros) })
        .collect();
    if searches.is_empty() {
        issues.push("no panel with a search".into());
    }
    if searches.iter().any(|s| s.translation.aggregation.is_some()) {
        issues.push("charted panels need log-based metrics: they're migrated as log streams with a note describing the aggregation".into());
    }
    Some(NewAsset { kind: "dashboard", title, original, searches, alert: None, issues })
}

fn check_file(f: &Option<String>, what: &str) -> Result<(), ApiError> {
    if f.as_ref().is_some_and(|f| f.len() > MAX_FILE) {
        return Err(ApiError::BadRequest(format!("the {what} file is larger than 10 MB")));
    }
    Ok(())
}

// ================================================================ imports: storage

#[derive(sqlx::FromRow)]
struct ImportRow {
    id: String,
    name: String,
    source: String,
    created_at: i64,
    created_by: Option<String>,
    macros: Value,
}

const IMPORT_SELECT: &str = "SELECT i.id::text AS id, i.name, i.source, (extract(epoch FROM i.created_at) * 1000)::bigint AS created_at, u.name AS created_by, i.macros \
    FROM log_imports i LEFT JOIN users u ON u.id = i.created_by";

#[derive(sqlx::FromRow)]
struct AssetRow {
    id: String,
    kind: String,
    title: String,
    original: Value,
    translation: Value,
    status: String,
    target_kind: Option<String>,
    target_id: Option<String>,
    error: Option<String>,
    migrated_at: Option<i64>,
}

const ASSET_SELECT: &str = "SELECT id::text AS id, kind, title, original, translation, status, target_kind, target_id, error, \
    (extract(epoch FROM migrated_at) * 1000)::bigint AS migrated_at FROM log_import_assets";

fn asset(r: AssetRow) -> ImportAsset {
    let t = &r.translation;
    let searches: Vec<ImportSearch> = serde_json::from_value(t["searches"].clone()).unwrap_or_default();
    let alert: Option<ImportAlert> = serde_json::from_value(t["alert"].clone()).ok();
    let issues: Vec<String> = serde_json::from_value(t["issues"].clone()).unwrap_or_default();
    let description = r.original.get("description").and_then(Value::as_str).filter(|d| !d.trim().is_empty()).map(String::from);
    ImportAsset {
        id: r.id,
        kind: r.kind,
        title: r.title,
        description,
        status: r.status,
        searches,
        alert,
        issues,
        target_kind: r.target_kind,
        target_id: r.target_id,
        error: r.error,
        migrated_at: r.migrated_at,
    }
}

fn counts(assets: &[ImportAsset]) -> ImportCounts {
    let mut c = ImportCounts { total: assets.len() as i64, ..Default::default() };
    for a in assets {
        match a.kind.as_str() {
            "alert" => c.alerts += 1,
            "dashboard" => c.dashboards += 1,
            _ => c.searches += 1,
        }
        match a.status.as_str() {
            "ready" => c.ready += 1,
            "partial" => c.partial += 1,
            "unsupported" => c.unsupported += 1,
            "migrated" => c.migrated += 1,
            _ => c.failed += 1,
        }
    }
    c
}

async fn load(st: &AppState, org: &str, id: &str) -> Result<LogImportDetail, ApiError> {
    let row: ImportRow = sqlx::query_as(&format!("{IMPORT_SELECT} WHERE i.org_id = $1 AND i.id::text = $2"))
        .bind(org)
        .bind(id)
        .fetch_optional(&st.postgres)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("Import {id} not found")))?;
    let rows: Vec<AssetRow> = sqlx::query_as(&format!("{ASSET_SELECT} WHERE org_id = $1 AND import_id::text = $2 ORDER BY position"))
        .bind(org)
        .bind(id)
        .fetch_all(&st.postgres)
        .await?;
    let assets: Vec<ImportAsset> = rows.into_iter().map(asset).collect();
    Ok(LogImportDetail { import: summary(row, counts(&assets)), assets })
}

fn summary(r: ImportRow, counts: ImportCounts) -> LogImport {
    let macros = r.macros.as_object().map_or(0, |m| m.len());
    LogImport { id: r.id, name: r.name, source: r.source, created_at: r.created_at, created_by: r.created_by, macros, counts }
}

// ================================================================ imports: routes

#[utoipa::path(get, path = "/api/v1/logs/imports", operation_id = "list_log_imports", responses((status = 200, body = Vec<LogImport>)), tag = "logs")]
pub async fn list(State(st): State<AppState>, ctx: AuthContext) -> ApiResult<Vec<LogImport>> {
    let rows: Vec<ImportRow> = sqlx::query_as(&format!("{IMPORT_SELECT} WHERE i.org_id = $1 ORDER BY i.created_at DESC")).bind(&ctx.org_id).fetch_all(&st.postgres).await?;
    let stats: Vec<(String, String, String, i64)> = sqlx::query_as("SELECT import_id::text, kind, status, count(*) FROM log_import_assets WHERE org_id = $1 GROUP BY 1, 2, 3")
        .bind(&ctx.org_id)
        .fetch_all(&st.postgres)
        .await?;
    Ok(Json(
        rows.into_iter()
            .map(|r| {
                let mut c = ImportCounts::default();
                for (_, kind, status, n) in stats.iter().filter(|s| s.0 == r.id) {
                    c.total += n;
                    match kind.as_str() {
                        "alert" => c.alerts += n,
                        "dashboard" => c.dashboards += n,
                        _ => c.searches += n,
                    }
                    match status.as_str() {
                        "ready" => c.ready += n,
                        "partial" => c.partial += n,
                        "unsupported" => c.unsupported += n,
                        "migrated" => c.migrated += n,
                        _ => c.failed += n,
                    }
                }
                summary(r, c)
            })
            .collect(),
    ))
}

#[utoipa::path(get, path = "/api/v1/logs/imports/{id}", operation_id = "get_log_import", params(("id" = String, Path)), responses((status = 200, body = LogImportDetail)), tag = "logs")]
pub async fn get_one(State(st): State<AppState>, ctx: AuthContext, Path(id): Path<String>) -> ApiResult<LogImportDetail> {
    Ok(Json(load(&st, &ctx.org_id, &id).await?))
}

#[utoipa::path(post, path = "/api/v1/logs/imports", operation_id = "create_log_import", request_body = CreateImport, responses((status = 200, body = LogImportDetail)), tag = "logs")]
pub async fn create(State(st): State<AppState>, Writer(ctx): Writer, Json(req): Json<CreateImport>) -> ApiResult<LogImportDetail> {
    if req.source != "splunk" {
        return Err(ApiError::BadRequest("only Splunk exports can be imported".into()));
    }
    let name = req.name.trim();
    if name.is_empty() || name.chars().count() > 200 {
        return Err(ApiError::BadRequest("the import name must be 1–200 characters".into()));
    }
    check_file(&req.alerts, "alerts")?;
    check_file(&req.dashboards, "dashboards")?;
    check_file(&req.macros, "macros")?;
    let macros = req.macros.as_deref().map(|m| spl::macros_from_rows(&spl::parse_csv(m))).unwrap_or_default();
    let mut assets = vec![];
    for r in req.alerts.as_deref().map(spl::parse_csv).unwrap_or_default() {
        if get(&r, "search").is_some() || get(&r, "qualifiedSearch").is_some() {
            assets.push(alert_asset(&r, &macros));
        }
    }
    for r in req.dashboards.as_deref().map(spl::parse_csv).unwrap_or_default() {
        assets.extend(dashboard_asset(&r, &macros));
    }
    if assets.is_empty() {
        return Err(ApiError::BadRequest("no searches, alerts or dashboards found: upload the CSV files exported with the searches shown".into()));
    }
    if assets.len() > MAX_ASSETS {
        return Err(ApiError::BadRequest(format!("an import holds at most {MAX_ASSETS} assets")));
    }
    let macros_json: serde_json::Map<String, Value> = macros.iter().map(|(k, m): (&String, &Macro)| (k.clone(), json!({ "args": m.args, "definition": m.definition }))).collect();
    let mut tx = st.postgres.begin().await?;
    let (id,): (String,) = sqlx::query_as("INSERT INTO log_imports (org_id, name, source, created_by, macros) VALUES ($1, $2, $3, $4::uuid, $5) RETURNING id::text")
        .bind(&ctx.org_id)
        .bind(name)
        .bind(&req.source)
        .bind(&ctx.user_id)
        .bind(Value::Object(macros_json))
        .fetch_one(&mut *tx)
        .await?;
    for (i, a) in assets.iter().enumerate() {
        let translation = json!({ "searches": a.searches, "alert": a.alert, "issues": a.issues });
        sqlx::query("INSERT INTO log_import_assets (org_id, import_id, kind, title, original, translation, status, position) VALUES ($1, $2::uuid, $3, $4, $5, $6, $7, $8)")
            .bind(&ctx.org_id)
            .bind(&id)
            .bind(a.kind)
            .bind(a.title.chars().take(500).collect::<String>())
            .bind(&a.original)
            .bind(translation)
            .bind(a.status())
            .bind(i as i32)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    ctx.audit(&st, "log_import.create", &format!("log_import:{id}"), name).await;
    Ok(Json(load(&st, &ctx.org_id, &id).await?))
}

#[utoipa::path(delete, path = "/api/v1/logs/imports/{id}", operation_id = "delete_log_import", params(("id" = String, Path)), responses((status = 204)), tag = "logs")]
pub async fn delete(State(st): State<AppState>, Writer(ctx): Writer, Path(id): Path<String>) -> Result<StatusCode, ApiError> {
    let n = sqlx::query("DELETE FROM log_imports WHERE org_id = $1 AND id::text = $2").bind(&ctx.org_id).bind(&id).execute(&st.postgres).await?.rows_affected();
    if n == 0 {
        return Err(ApiError::NotFound(format!("Import {id} not found")));
    }
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(post, path = "/api/v1/logs/imports/{id}/migrate", operation_id = "migrate_log_import", params(("id" = String, Path)), request_body = MigrateRequest, responses((status = 200, body = LogImportDetail)), tag = "logs")]
pub async fn migrate(State(st): State<AppState>, Writer(ctx): Writer, Path(id): Path<String>, Json(req): Json<MigrateRequest>) -> ApiResult<LogImportDetail> {
    let detail = load(&st, &ctx.org_id, &id).await?;
    let wanted: Vec<&ImportAsset> = detail
        .assets
        .iter()
        .filter(|a| if req.asset_ids.is_empty() { matches!(a.status.as_str(), "ready" | "partial" | "failed") } else { req.asset_ids.contains(&a.id) })
        .filter(|a| a.kind != "search" && a.status != "migrated" && a.status != "unsupported")
        .collect();
    for a in wanted {
        let result = match a.kind.as_str() {
            "alert" => migrate_alert(&st, &ctx, a).await.map(|id| ("monitor", id)),
            _ => migrate_dashboard(&st, &ctx, a).await.map(|id| ("dashboard", id)),
        };
        match result {
            Ok((kind, target)) => {
                sqlx::query("UPDATE log_import_assets SET status = 'migrated', target_kind = $4, target_id = $5, error = NULL, migrated_at = now() WHERE org_id = $1 AND import_id::text = $2 AND id::text = $3")
                    .bind(&ctx.org_id)
                    .bind(&id)
                    .bind(&a.id)
                    .bind(kind)
                    .bind(target)
                    .execute(&st.postgres)
                    .await?;
            }
            Err(e) => {
                sqlx::query("UPDATE log_import_assets SET status = 'failed', error = $4 WHERE org_id = $1 AND import_id::text = $2 AND id::text = $3")
                    .bind(&ctx.org_id)
                    .bind(&id)
                    .bind(&a.id)
                    .bind(e.to_string())
                    .execute(&st.postgres)
                    .await?;
            }
        }
    }
    Ok(Json(load(&st, &ctx.org_id, &id).await?))
}

/// A log monitor counting the alert's events: `logs("<query>") by {…}`.
async fn migrate_alert(st: &AppState, ctx: &AuthContext, a: &ImportAsset) -> Result<String, ApiError> {
    let s = a.searches.first().ok_or_else(|| ApiError::BadRequest("the alert has no search".into()))?;
    let alert = a.alert.as_ref().ok_or_else(|| ApiError::BadRequest("not an alert".into()))?;
    let agg = s.translation.aggregation.as_ref();
    if agg.is_some_and(|g| g.measures.iter().any(|m| m.function != "count")) {
        return Err(ApiError::BadRequest("log monitors count events; this alert measures another value".into()));
    }
    let threshold = alert.threshold.ok_or_else(|| ApiError::BadRequest("the alert has no threshold".into()))?;
    let by: Vec<String> = agg.map(|g| g.group_by.iter().take(4).cloned().collect()).unwrap_or_default();
    let escaped = s.translation.query.replace('\\', "\\\\").replace('"', "\\\"");
    let query = if by.is_empty() { format!("logs(\"{escaped}\")") } else { format!("logs(\"{escaped}\") by {{{}}}", by.join(",")) };
    let mut message = a.description.clone().unwrap_or_default();
    if !alert.recipients.is_empty() {
        message.push_str(&format!("\n\n{}", alert.recipients.iter().map(|r| format!("@{r}")).collect::<Vec<_>>().join(" ")));
    }
    message.push_str(&format!("\n\nImported from Splunk. Original search:\n{}", s.spl));
    let input = json!({
        "name": a.title, "type": "log", "query": query, "priority": null, "message": message.trim(),
        "tags": ["source:splunk-import"],
        "options": {"detection": "threshold", "timeAggregation": "sum", "windowMinutes": alert.window_minutes,
                    "comparator": alert.comparator.clone().unwrap_or_else(|| "above".into()),
                    "thresholds": {"critical": threshold, "warning": null, "criticalRecovery": null, "warningRecovery": null}}
    });
    let mut input: super::monitors::MonitorInput = serde_json::from_value(input).map_err(|e| ApiError::BadRequest(e.to_string()))?;
    super::monitors::validate(&mut input)?;
    let id = super::monitors::store_new(st, ctx, &input, None).await?;
    Ok(id.to_string())
}

fn widget_id(i: usize) -> String {
    format!("w{}{i}", uuid_part())
}

fn uuid_part() -> String {
    let mut b = [0u8; 6];
    rand::RngCore::fill_bytes(&mut rand::rngs::OsRng, &mut b);
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn aggregation_note(s: &ImportSearch) -> Option<String> {
    let a = s.translation.aggregation.as_ref()?;
    let measures = a.measures.iter().map(|m| m.field.as_ref().map_or(m.function.clone(), |f| format!("{}({f})", m.function))).collect::<Vec<_>>().join(", ");
    let by = if a.group_by.is_empty() { String::new() } else { format!(" by {}", a.group_by.join(", ")) };
    Some(format!(
        "**{}** was a {} of {measures}{by}.\n\nTo chart it, create a log-based metric for `{}` (Logs → Generate Metrics) and replace this note with a {} widget.\n\nOriginal search:\n\n`{}`",
        s.title,
        a.viz,
        if s.translation.query.is_empty() { "*" } else { &s.translation.query },
        if a.viz == "timeseries" { "timeseries" } else { "top list" },
        s.spl.replace('`', "'"),
    ))
}

/// A dashboard with a log stream per panel, and a note for the panels that charted an aggregation.
async fn migrate_dashboard(st: &AppState, ctx: &AuthContext, a: &ImportAsset) -> Result<String, ApiError> {
    let mut widgets = vec![];
    let mut y = 0;
    for (i, s) in a.searches.iter().enumerate() {
        let note = aggregation_note(s);
        let w = if note.is_some() { 8 } else { 12 };
        widgets.push(json!({
            "id": widget_id(i * 2),
            "definition": { "type": "log_stream", "title": s.title, "query": s.translation.query, "limit": 50 },
            "layout": { "x": 0, "y": y, "w": w, "h": 8 }
        }));
        if let Some(n) = note {
            widgets.push(json!({
                "id": widget_id(i * 2 + 1),
                "definition": { "type": "note", "content": n, "background": "yellow", "align": "left", "fontSize": "sm" },
                "layout": { "x": 8, "y": y, "w": 4, "h": 8 }
            }));
        }
        y += 8;
    }
    let id = super::dashboards::new_id();
    let title: String = a.title.chars().take(200).collect();
    let def = json!({ "layoutType": "grid", "templateVariables": [], "widgets": widgets, "description": "Imported from Splunk." });
    sqlx::query("INSERT INTO dashboards (id, org_id, title, kind, author_id, teams, definition) VALUES ($1, $2, $3, 'custom', $4::uuid, $5, $6)")
        .bind(&id)
        .bind(&ctx.org_id)
        .bind(&title)
        .bind(&ctx.user_id)
        .bind(Vec::<String>::new())
        .bind(&def)
        .execute(&st.postgres)
        .await?;
    ctx.audit(st, "dashboard.create", &format!("dashboard:{id}"), &title).await;
    Ok(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    #[test]
    fn windows() {
        assert_eq!(window_minutes(Some("-15m")), Some(15));
        assert_eq!(window_minutes(Some("-1h@h")), Some(60));
        assert_eq!(window_minutes(Some("rt-5m")), Some(5));
        assert_eq!(window_minutes(Some("-d")), Some(1440));
        assert_eq!(window_minutes(Some("-30s")), Some(1));
        assert_eq!(window_minutes(Some("now")), None);
    }

    #[test]
    fn alerts_become_monitor_specs() {
        let a = alert_asset(
            &row(&[
                ("title", "Checkout errors"),
                ("search", "index=web service=checkout status=500 | stats count by host"),
                ("alert_type", "number of events"),
                ("alert_comparator", "greater than"),
                ("alert_threshold", "25"),
                ("dispatch.earliest_time", "-10m"),
                ("action.email.to", "ops@example.com, sre@example.com"),
            ]),
            &Macros::new(),
        );
        assert_eq!(a.kind, "alert");
        assert_eq!(a.status(), "ready");
        let al = a.alert.unwrap();
        assert_eq!((al.comparator.as_deref(), al.threshold, al.window_minutes), (Some("above"), Some(25.0), 10));
        assert_eq!(al.recipients, vec!["ops@example.com", "sre@example.com"]);

        let a = alert_asset(&row(&[("title", "Slow"), ("search", "index=web | stats avg(duration)"), ("alert_type", "custom"), ("alert_comparator", "rises by")]), &Macros::new());
        assert_eq!(a.status(), "partial");
        assert!(a.issues.len() >= 2);

        let a = alert_asset(&row(&[("title", "Saved"), ("search", "index=web error")]), &Macros::new());
        assert_eq!(a.kind, "search");

        let a = alert_asset(&row(&[("title", "Where"), ("search", "index=web | stats count | where count >= 3"), ("alert_type", "custom"), ("is_scheduled", "1")]), &Macros::new());
        let al = a.alert.unwrap();
        assert_eq!((al.comparator.as_deref(), al.threshold), (Some("above_or_equal"), Some(3.0)));
    }

    #[test]
    fn dashboards_become_panels() {
        let xml = "<dashboard><label>Web</label><row><panel><title>Errors</title><chart><search><query>index=web status=500 | timechart count</query></search></chart></panel><panel><event><search><query>index=web error</query></search></event></panel></row></dashboard>";
        let a = dashboard_asset(&row(&[("title", "web_overview"), ("eai:data", xml)]), &Macros::new()).unwrap();
        assert_eq!(a.title, "Web");
        assert_eq!(a.searches.len(), 2);
        assert_eq!(a.searches[1].title, "Panel 2");
        assert_eq!(a.status(), "partial");
        assert!(aggregation_note(&a.searches[0]).unwrap().contains("log-based metric"));
        assert!(aggregation_note(&a.searches[1]).is_none());
        let a = dashboard_asset(&row(&[("title", "studio"), ("eai:data", "{\"visualizations\":{}}")]), &Macros::new()).unwrap();
        assert_eq!(a.status(), "unsupported");
    }

    /// Import → review → migrate against live Postgres: an alert becomes a log monitor, a
    /// dashboard a dashboard of log streams. Skipped unless OC_TEST_CLICKHOUSE_URL and OC_TEST_POSTGRES_URL are set.
    #[tokio::test]
    async fn imports_migrate_to_monitors_and_dashboards() {
        let Some((_, states)) = crate::test_support::replicas(1).await else { return };
        let st = states[0].clone();
        let mut ctx = crate::test_support::admin_of_new_org(&st, "logimport").await;
        let (user,): (String,) = sqlx::query_as("INSERT INTO users (id, org_id, email, name) VALUES (gen_random_uuid(), $1, $2, 'Migration Tester') RETURNING id::text")
            .bind(&ctx.org_id)
            .bind(format!("{}@example.com", ctx.org_id))
            .fetch_one(&st.postgres)
            .await
            .unwrap();
        ctx.user_id = user;
        let alerts = "title,search,alert_type,alert_comparator,alert_threshold,dispatch.earliest_time,action.email.to,description\n\
            Checkout errors,`errs` service=checkout | stats count by host,number of events,greater than,5,-10m,ops@example.com,Too many errors\n\
            Saved search,index=web error,,,,,,\n";
        let macros = "title,definition,args\nerrs,index=web status=500,\n";
        let xml = "<dashboard><label>Web</label><row><panel><title>Errors</title><event><search><query>index=web error</query></search></event></panel></row></dashboard>";
        let dashboards = format!("title,eai:data\nweb,\"{}\"\n", xml.replace('"', "\"\""));
        let req = CreateImport { name: "Splunk prod".into(), source: "splunk".into(), alerts: Some(alerts.into()), dashboards: Some(dashboards), macros: Some(macros.into()) };
        let d = create(State(st.clone()), Writer(ctx.clone()), Json(req)).await.expect("create").0;
        assert_eq!((d.import.counts.total, d.import.counts.alerts, d.import.counts.dashboards, d.import.counts.searches, d.import.macros), (3, 1, 1, 1, 1));
        let alert = d.assets.iter().find(|a| a.kind == "alert").unwrap();
        assert_eq!(alert.searches[0].translation.query, "index:web @http.status_code:500 service:checkout");
        assert_eq!(alert.status, "ready");
        assert_eq!(list(State(st.clone()), ctx.clone()).await.unwrap().0[0].counts.total, 3);

        let d = migrate(State(st.clone()), Writer(ctx.clone()), Path(d.import.id.clone()), Json(MigrateRequest { asset_ids: vec![] })).await.expect("migrate").0;
        let alert = d.assets.iter().find(|a| a.kind == "alert").unwrap();
        assert_eq!(alert.status, "migrated", "{:?}", alert.error);
        let (query, message): (String, String) = sqlx::query_as("SELECT query, message FROM monitors WHERE org_id = $1 AND id = $2::bigint")
            .bind(&ctx.org_id)
            .bind(alert.target_id.as_deref().unwrap())
            .fetch_one(&st.postgres)
            .await
            .unwrap();
        assert_eq!(query, "logs(\"index:web @http.status_code:500 service:checkout\") by {host}");
        assert!(message.contains("@ops@example.com"));
        let dash = d.assets.iter().find(|a| a.kind == "dashboard").unwrap();
        assert_eq!(dash.status, "migrated");
        let (title, def): (String, Value) = sqlx::query_as("SELECT title, definition FROM dashboards WHERE org_id = $1 AND id = $2").bind(&ctx.org_id).bind(dash.target_id.as_deref().unwrap()).fetch_one(&st.postgres).await.unwrap();
        assert_eq!(title, "Web");
        assert_eq!(def["widgets"][0]["definition"]["query"], "index:web error");
        assert_eq!(d.assets.iter().find(|a| a.kind == "search").unwrap().status, "ready");

        // Another org sees nothing.
        let other = crate::test_support::admin_of_new_org(&st, "logimport-other").await;
        assert!(get_one(State(st.clone()), other.clone(), Path(d.import.id.clone())).await.is_err());
        assert!(delete(State(st.clone()), Writer(other), Path(d.import.id.clone())).await.is_err());
        assert_eq!(delete(State(st.clone()), Writer(ctx.clone()), Path(d.import.id.clone())).await.unwrap(), StatusCode::NO_CONTENT);
    }
}

