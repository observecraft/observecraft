//! Cloud SIEM's own pages: the content packs (log sources with default rules), the log sources
//! table, and the overview (setup, coverage, logs analyzed, important signals).
//!
//! A pack's status comes from its logs (counted with the reader's log access): **active** when on
//! and its logs arrived in the last 24 hours, **warning** when on and they stopped (seen in the last
//! 7 days, not the last 24 hours), **available** otherwise (or **disabled** when turned off).

use std::collections::{BTreeMap, HashMap};

use axum::extract::{Path, State};
use axum::Json;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use super::logs::{log_counts, TimeRange};
use super::security_catalog::{pack, PACKS};
use super::security_rules::{ensure_defaults, load_rules};
use super::security_signals::{signals_in, SecuritySignal};
use crate::auth::{AuthContext, Writer};
use crate::error::{ApiError, ApiResult};
use crate::log_access::LogAccess;
use crate::state::AppState;

const HOUR: i64 = 3_600_000;
const DAY: i64 = 24 * HOUR;

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct PackRule {
    pub id: String,
    pub name: String,
    /// The most severe case.
    pub severity: String,
    pub enabled: bool,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ContentPackStatus {
    pub id: String,
    pub name: String,
    pub category: String,
    pub platform: String,
    pub logo: String,
    pub description: String,
    pub query: String,
    pub setup: String,
    pub enabled: bool,
    /// active | warning | available | disabled
    pub status: String,
    pub logs24h: u64,
    /// Logs per hour over the last 24 hours.
    pub hourly: Vec<u64>,
    /// Epoch ms of the latest hour with logs in the last 7 days.
    pub last_log_at: Option<i64>,
    pub rules: Vec<PackRule>,
    /// Signals of the pack's rules in the last 7 days.
    pub signals7d: usize,
}

fn source_of(query: &str) -> &str {
    query.trim_start_matches("source:")
}

/// Per source of the packs: hourly counts over the last 24 hours and the last hour seen in 7 days.
async fn source_volumes(st: &AppState, org: &str, access: &LogAccess, now: i64) -> Result<HashMap<String, (Vec<u64>, Option<i64>)>, ApiError> {
    let sources: Vec<&str> = PACKS.iter().map(|p| source_of(p.query)).collect();
    let search = format!("source:({})", sources.join(" OR "));
    let by = vec!["source".to_string()];
    let to = now - now.rem_euclid(HOUR) + HOUR - 1;
    let day = log_counts(st, org, access, &search, &by, TimeRange { from: to + 1 - DAY, to }, HOUR).await?;
    let week = log_counts(st, org, access, &search, &by, TimeRange { from: to + 1 - 7 * DAY, to }, 6 * HOUR).await?;
    let mut out: HashMap<String, (Vec<u64>, Option<i64>)> = HashMap::new();
    for g in day {
        let src = g.tags.get("source").cloned().unwrap_or_default();
        out.entry(src).or_default().0 = g.points.iter().map(|(_, v)| *v as u64).collect();
    }
    for g in week {
        let src = g.tags.get("source").cloned().unwrap_or_default();
        out.entry(src).or_default().1 = g.points.iter().filter(|(_, v)| *v > 0.0).map(|(t, _)| *t).max();
    }
    Ok(out)
}

pub async fn pack_statuses(st: &AppState, org: &str, access: &LogAccess, now: i64) -> Result<Vec<ContentPackStatus>, ApiError> {
    ensure_defaults(st, org).await?;
    let disabled: Vec<(String, bool)> = sqlx::query_as("SELECT pack, enabled FROM security_content_packs WHERE org_id = $1").bind(org).fetch_all(&st.postgres).await?;
    let enabled: HashMap<String, bool> = disabled.into_iter().collect();
    let rules = load_rules(st, org).await?;
    let volumes = source_volumes(st, org, access, now).await?;
    let signals = signals_in(st, org, TimeRange { from: now - 7 * DAY, to: now }).await?;
    Ok(PACKS
        .iter()
        .map(|p| {
            let on = enabled.get(p.id).copied().unwrap_or(true);
            let (hourly, last) = volumes.get(source_of(p.query)).cloned().unwrap_or_default();
            let logs24h: u64 = hourly.iter().sum();
            let status = match (on, logs24h > 0, last.is_some()) {
                (false, _, _) => "disabled",
                (true, true, _) => "active",
                (true, false, true) => "warning",
                _ => "available",
            };
            let pack_rules: Vec<PackRule> = rules
                .iter()
                .filter(|r| r.pack.as_deref() == Some(p.id))
                .map(|r| PackRule {
                    id: r.id.clone(),
                    name: r.name.clone(),
                    severity: r.cases.iter().map(|c| c.status.clone()).max_by_key(|s| super::security_rules::severity_rank(s)).unwrap_or_default(),
                    enabled: r.is_enabled,
                })
                .collect();
            let signals7d = signals.iter().filter(|s| pack_rules.iter().any(|r| r.id == s.rule_id)).count();
            ContentPackStatus {
                id: p.id.into(),
                name: p.name.into(),
                category: p.category.into(),
                platform: p.platform.into(),
                logo: p.logo.into(),
                description: p.description.into(),
                query: p.query.into(),
                setup: p.setup.into(),
                enabled: on,
                status: status.into(),
                logs24h,
                hourly: if hourly.is_empty() { vec![0; 24] } else { hourly },
                last_log_at: last,
                rules: pack_rules,
                signals7d,
            }
        })
        .collect())
}

#[utoipa::path(get, path = "/api/v1/security/siem/packs", operation_id = "list_siem_content_packs", responses((status = 200, body = Vec<ContentPackStatus>)), tag = "security")]
pub async fn packs(State(st): State<AppState>, ctx: AuthContext, access: LogAccess) -> ApiResult<Vec<ContentPackStatus>> {
    Ok(Json(pack_statuses(&st, &ctx.org_id, &access, chrono::Utc::now().timestamp_millis()).await?))
}

#[derive(Deserialize, ToSchema)]
pub struct PackEnabled {
    pub enabled: bool,
}

#[utoipa::path(put, path = "/api/v1/security/siem/packs/{id}", operation_id = "set_siem_content_pack", params(("id" = String, Path)), request_body = PackEnabled,
    responses((status = 204), (status = 404, body = crate::error::ErrorBody)), tag = "security")]
pub async fn set_pack(State(st): State<AppState>, Writer(ctx): Writer, Path(id): Path<String>, Json(req): Json<PackEnabled>) -> Result<axum::http::StatusCode, ApiError> {
    pack(&id).ok_or_else(|| ApiError::NotFound(format!("no content pack {id}")))?;
    sqlx::query("INSERT INTO security_content_packs (org_id, pack, enabled) VALUES ($1, $2, $3) ON CONFLICT (org_id, pack) DO UPDATE SET enabled = $3, updated_at = now()")
        .bind(&ctx.org_id)
        .bind(&id)
        .bind(req.enabled)
        .execute(&st.postgres)
        .await?;
    Ok(axum::http::StatusCode::NO_CONTENT)
}

// ================================================================ overview

#[derive(Deserialize, ToSchema)]
pub struct SiemOverviewRequest {
    pub range: TimeRange,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SiemSetup {
    /// Content packs on.
    pub packs_enabled: usize,
    /// Packs receiving logs.
    pub receiving_logs: usize,
    /// Rules evaluated now.
    pub rules_active: usize,
    /// Rules with a case that notifies someone.
    pub rules_notifying: usize,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct CategoryCoverage {
    pub category: String,
    pub active: usize,
    pub total: usize,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SourceSeries {
    pub source: String,
    /// `[epoch ms, logs]`
    pub points: Vec<(i64, u64)>,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SiemOverview {
    pub setup: SiemSetup,
    pub coverage: Vec<CategoryCoverage>,
    /// Pack counts by status.
    pub statuses: BTreeMap<String, usize>,
    /// Logs of the packs' sources over the range.
    pub logs_analyzed: Vec<SourceSeries>,
    pub signals_by_severity: BTreeMap<String, usize>,
    /// Open high and critical signals, newest first.
    pub important_signals: Vec<SecuritySignal>,
    pub open_signals: usize,
}

#[utoipa::path(post, path = "/api/v1/security/siem/overview", operation_id = "siem_overview", request_body = SiemOverviewRequest,
    responses((status = 200, body = SiemOverview)), tag = "security")]
pub async fn overview(State(st): State<AppState>, ctx: AuthContext, access: LogAccess, Json(req): Json<SiemOverviewRequest>) -> ApiResult<SiemOverview> {
    let now = chrono::Utc::now().timestamp_millis();
    let packs = pack_statuses(&st, &ctx.org_id, &access, now).await?;
    let rules = load_rules(&st, &ctx.org_id).await?;
    let mut statuses: BTreeMap<String, usize> = ["active", "warning", "available", "disabled"].iter().map(|s| (s.to_string(), 0)).collect();
    let mut coverage: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    for p in &packs {
        *statuses.entry(p.status.clone()).or_default() += 1;
        let c = coverage.entry(p.category.clone()).or_default();
        c.1 += 1;
        c.0 += usize::from(p.status == "active");
    }
    let sources: Vec<&str> = PACKS.iter().map(|p| source_of(p.query)).collect();
    let span = (req.range.to - req.range.from).max(HOUR);
    let step = [5 * 60_000, 15 * 60_000, HOUR, 3 * HOUR, 6 * HOUR, DAY].into_iter().find(|s| span / s <= 96).unwrap_or(DAY);
    let groups = log_counts(&st, &ctx.org_id, &access, &format!("source:({})", sources.join(" OR ")), &["source".to_string()], req.range, step).await?;
    let logs_analyzed = groups
        .into_iter()
        .filter(|g| g.points.iter().any(|(_, v)| *v > 0.0))
        .map(|g| SourceSeries { source: g.tags.get("source").cloned().unwrap_or_default(), points: g.points.into_iter().map(|(t, v)| (t, v as u64)).collect() })
        .collect();
    let signals = signals_in(&st, &ctx.org_id, req.range).await?;
    let mut by_severity: BTreeMap<String, usize> = super::security_rules::SEVERITIES.iter().map(|s| (s.to_string(), 0)).collect();
    for s in signals.iter().filter(|s| s.created_at >= req.range.from) {
        *by_severity.entry(s.severity.clone()).or_default() += 1;
    }
    let open: Vec<&SecuritySignal> = signals.iter().filter(|s| s.state == "open").collect();
    Ok(Json(SiemOverview {
        setup: SiemSetup {
            packs_enabled: packs.iter().filter(|p| p.enabled).count(),
            receiving_logs: packs.iter().filter(|p| p.logs24h > 0).count(),
            rules_active: rules.iter().filter(|r| r.active && r.not_evaluated.is_none()).count(),
            rules_notifying: rules.iter().filter(|r| r.cases.iter().any(|c| !c.notifications.is_empty())).count(),
        },
        coverage: coverage.into_iter().map(|(category, (active, total))| CategoryCoverage { category, active, total }).collect(),
        statuses,
        logs_analyzed,
        signals_by_severity: by_severity,
        open_signals: open.len(),
        important_signals: open.into_iter().filter(|s| matches!(s.severity.as_str(), "high" | "critical")).take(10).cloned().collect(),
    }))
}
