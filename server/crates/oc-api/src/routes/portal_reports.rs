//! Developer Portal → Reports: how teams respond to incidents and how far their services are
//! covered by SLOs, monitors and On-Call (Teams Performance), how scorecard levels move per team
//! (Scorecards Performance), and the org's own dashboards (Custom Reports). Reliability Overview and
//! Catalog Health are built in the UI from the SLO, monitor and catalog APIs.
//!
//! Everything here reads Postgres (incidents, teams, SLOs, monitors, On-Call schedules, service
//! metadata and the daily scorecard levels the evaluations write), so ranges may span months.

use std::collections::{BTreeMap, HashMap, HashSet};

use axum::extract::State;
use axum::Json;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use super::logs::TimeRange;
use crate::auth::{AuthContext, Writer};
use crate::error::{ApiError, ApiResult};
use crate::state::{AppState, OrgId};

const DAY_MS: i64 = 86_400_000;
const MAX_DASHBOARDS: usize = 50;

// ================================================================ API types

#[derive(Deserialize, ToSchema)]
pub struct PortalReportRequest {
    pub range: TimeRange,
    /// Team handles; empty for every team.
    #[serde(default)]
    pub teams: Vec<String>,
}

#[derive(Serialize, ToSchema, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PortalTeamReport {
    pub handle: String,
    pub name: String,
    pub members: i64,
    /// Incidents declared per bucket (see `buckets`).
    pub incidents: Vec<u32>,
    pub incident_count: u32,
    pub open_incidents: u32,
    /// Mean time from declaration to resolution of the incidents resolved in the range, in minutes.
    pub mttr_minutes: Option<f64>,
    pub services: u32,
    pub slos: u32,
    pub monitors: u32,
    pub on_call_schedules: u32,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct PortalTeamsReport {
    /// Bucket starts (epoch ms); a bucket is a day, or a week for ranges over 31 days.
    pub buckets: Vec<i64>,
    pub bucket_ms: i64,
    pub teams: Vec<PortalTeamReport>,
    /// Every team of the org, before the filter.
    pub total_teams: usize,
}

#[derive(Serialize, ToSchema, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PortalScorecardTeam {
    /// Team handle; empty for services without a team.
    pub team: String,
    /// Mean level of the team's services per day (null on days without an evaluation).
    pub levels: Vec<Option<f64>>,
    /// Services at level 0–3 on the last evaluated day.
    pub current: Vec<u32>,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct PortalScorecardsReport {
    /// Day starts (epoch ms, UTC).
    pub days: Vec<i64>,
    pub teams: Vec<PortalScorecardTeam>,
    /// Each rule's passes and failures per day, over the selected teams (all when none).
    pub rule_days: Vec<PortalScorecardRuleDay>,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct PortalScorecardRuleDay {
    /// Day start (epoch ms, UTC).
    pub day: i64,
    pub rule_id: String,
    pub pass: i64,
    pub fail: i64,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct PortalReportDashboard {
    pub id: String,
    pub title: String,
    pub description: Option<String>,
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct PortalReportDashboardsInput {
    /// Dashboard ids in display order.
    pub dashboard_ids: Vec<String>,
}

// ================================================================ aggregation (pure)

#[derive(Clone, Debug, Default)]
pub struct IncidentFact {
    pub declared_at: i64,
    pub resolved_at: Option<i64>,
    pub open: bool,
    pub teams: Vec<String>,
    pub services: Vec<String>,
}

#[derive(Clone, Debug, Default)]
pub struct TeamFacts {
    pub handle: String,
    pub name: String,
    pub members: i64,
}

/// What the org declares that the report counts per team.
#[derive(Default)]
pub struct CoverageFacts {
    /// service → owning team.
    pub owners: HashMap<String, String>,
    /// Teams of each SLO (its own teams, else the owners of the services it names).
    pub slo_teams: Vec<Vec<String>>,
    pub monitor_teams: Vec<Vec<String>>,
    /// Team handle of each On-Call schedule.
    pub schedule_teams: Vec<String>,
}

pub fn bucket_size(range: TimeRange) -> i64 {
    if range.to - range.from > 31 * DAY_MS {
        7 * DAY_MS
    } else {
        DAY_MS
    }
}

pub fn buckets(range: TimeRange, size: i64) -> Vec<i64> {
    let first = range.from.div_euclid(size) * size;
    (0..).map(|i| first + i * size).take_while(|b| *b <= range.to).collect()
}

/// The teams an incident counts for: the teams it lists, and the owners of the services it lists.
fn incident_teams(i: &IncidentFact, owners: &HashMap<String, String>) -> HashSet<String> {
    i.teams.iter().cloned().chain(i.services.iter().filter_map(|s| owners.get(s).cloned())).filter(|t| !t.is_empty()).collect()
}

pub fn teams_report(range: TimeRange, teams: &[TeamFacts], incidents: &[IncidentFact], coverage: &CoverageFacts) -> (Vec<i64>, i64, Vec<PortalTeamReport>) {
    let size = bucket_size(range);
    let starts = buckets(range, size);
    let first = starts.first().copied().unwrap_or(range.from);
    let mut out: Vec<PortalTeamReport> = teams
        .iter()
        .map(|t| PortalTeamReport {
            handle: t.handle.clone(),
            name: t.name.clone(),
            members: t.members,
            incidents: vec![0; starts.len()],
            incident_count: 0,
            open_incidents: 0,
            mttr_minutes: None,
            services: coverage.owners.values().filter(|o| **o == t.handle).count() as u32,
            slos: coverage.slo_teams.iter().filter(|ts| ts.contains(&t.handle)).count() as u32,
            monitors: coverage.monitor_teams.iter().filter(|ts| ts.contains(&t.handle)).count() as u32,
            on_call_schedules: coverage.schedule_teams.iter().filter(|h| **h == t.handle).count() as u32,
        })
        .collect();
    let index: HashMap<String, usize> = out.iter().enumerate().map(|(i, t)| (t.handle.clone(), i)).collect();
    let mut repair: Vec<(f64, u32)> = vec![(0.0, 0); out.len()];
    for inc in incidents {
        let declared_in_range = inc.declared_at >= range.from && inc.declared_at <= range.to;
        for team in incident_teams(inc, &coverage.owners) {
            let Some(&i) = index.get(&team) else { continue };
            if inc.open {
                out[i].open_incidents += 1;
            }
            if declared_in_range {
                out[i].incident_count += 1;
                if let Some(slot) = usize::try_from((inc.declared_at - first) / size).ok().and_then(|b| out[i].incidents.get_mut(b)) {
                    *slot += 1;
                }
            }
            if let Some(resolved) = inc.resolved_at.filter(|r| *r >= range.from && *r <= range.to && *r >= inc.declared_at) {
                repair[i].0 += (resolved - inc.declared_at) as f64 / 60_000.0;
                repair[i].1 += 1;
            }
        }
    }
    for (t, (sum, n)) in out.iter_mut().zip(repair) {
        t.mttr_minutes = (n > 0).then(|| (sum / n as f64 * 10.0).round() / 10.0);
    }
    (starts, size, out)
}

/// (day start in ms, service, team, level) rows → mean level per team and day, and the last day's distribution.
pub fn scorecard_trend(range: TimeRange, rows: &[(i64, String, String, i16)], teams: &[String]) -> (Vec<i64>, Vec<PortalScorecardTeam>) {
    let days = buckets(range, DAY_MS);
    let index: HashMap<i64, usize> = days.iter().enumerate().map(|(i, d)| (*d, i)).collect();
    let wanted = |t: &str| teams.is_empty() || teams.iter().any(|w| w == t);
    let mut sums: BTreeMap<String, Vec<(i64, u32)>> = BTreeMap::new();
    let mut last_day: BTreeMap<String, i64> = BTreeMap::new();
    for (day, _, team, level) in rows {
        let Some(&i) = index.get(day) else { continue };
        if !wanted(team) {
            continue;
        }
        let s = sums.entry(team.clone()).or_insert_with(|| vec![(0, 0); days.len()]);
        s[i].0 += i64::from(*level);
        s[i].1 += 1;
        let l = last_day.entry(team.clone()).or_insert(*day);
        *l = (*l).max(*day);
    }
    let out = sums
        .into_iter()
        .map(|(team, s)| {
            let last = last_day.get(&team).copied();
            let mut current = vec![0u32; 4];
            for (day, _, t, level) in rows {
                if *t == team && Some(*day) == last {
                    current[(*level).clamp(0, 3) as usize] += 1;
                }
            }
            PortalScorecardTeam { levels: s.iter().map(|(sum, n)| (*n > 0).then(|| (*sum as f64 / *n as f64 * 100.0).round() / 100.0)).collect(), current, team }
        })
        .collect();
    (days, out)
}

// ================================================================ storage

async fn coverage(st: &AppState, org: &str) -> Result<CoverageFacts, ApiError> {
    let owners: Vec<(String, String)> = sqlx::query_as("SELECT service, team FROM service_metadata WHERE org_id = $1 AND team <> ''").bind(org).fetch_all(&st.postgres).await?;
    let owners: HashMap<String, String> = owners.into_iter().collect();
    let slos: Vec<(Vec<String>, Vec<String>)> = sqlx::query_as("SELECT teams, tags FROM slos WHERE org_id = $1").bind(org).fetch_all(&st.postgres).await?;
    let monitors: Vec<(Vec<String>, Vec<String>)> = sqlx::query_as("SELECT teams, tags FROM monitors WHERE org_id = $1 AND NOT draft").bind(org).fetch_all(&st.postgres).await?;
    let schedules: Vec<(String,)> = sqlx::query_as("SELECT team_handle FROM on_call_schedules WHERE org_id = $1 AND team_handle IS NOT NULL").bind(org).fetch_all(&st.postgres).await?;
    // Without teams of their own, SLOs and monitors count for the owners of the services their tags name.
    let teams_of = |(teams, tags): (Vec<String>, Vec<String>)| -> Vec<String> {
        if !teams.is_empty() {
            return teams.into_iter().map(|t| t.trim_start_matches('@').to_string()).collect();
        }
        let mut out: Vec<String> =
            tags.iter().filter_map(|t| t.strip_prefix("service:")).filter_map(|s| owners.get(s).cloned()).chain(tags.iter().filter_map(|t| t.strip_prefix("team:").map(str::to_string))).collect();
        out.sort();
        out.dedup();
        out
    };
    Ok(CoverageFacts {
        slo_teams: slos.into_iter().map(teams_of).collect(),
        monitor_teams: monitors.into_iter().map(teams_of).collect(),
        schedule_teams: schedules.into_iter().map(|(h,)| h).collect(),
        owners,
    })
}

async fn report_dashboards(st: &AppState, org: &str) -> Result<Vec<PortalReportDashboard>, ApiError> {
    let rows: Vec<(String, String, Option<String>)> = sqlx::query_as(
        "SELECT d.id, d.title, d.description FROM portal_report_dashboards r JOIN dashboards d ON d.id = r.dashboard_id AND d.org_id = r.org_id \
         WHERE r.org_id = $1 AND d.deleted_at IS NULL ORDER BY r.position",
    )
    .bind(org)
    .fetch_all(&st.postgres)
    .await?;
    Ok(rows.into_iter().map(|(id, title, description)| PortalReportDashboard { id, title, description }).collect())
}

// ================================================================ handlers

/// Teams Performance: incidents per team over time, mean time to resolve, and each team's services,
/// SLOs, monitors and On-Call schedules.
#[utoipa::path(post, path = "/api/v1/portal/reports/teams", operation_id = "portal_teams_report", request_body = PortalReportRequest, responses((status = 200, body = PortalTeamsReport)), tag = "portal")]
pub async fn teams(State(st): State<AppState>, OrgId(org): OrgId, Json(req): Json<PortalReportRequest>) -> ApiResult<PortalTeamsReport> {
    req.range.validate_loose()?;
    let all: Vec<(String, String, i64)> = sqlx::query_as(
        "SELECT t.handle, t.name, (SELECT count(*) FROM team_members m WHERE m.org_id = t.org_id AND m.handle = t.handle) FROM teams t WHERE t.org_id = $1 ORDER BY lower(t.name) LIMIT 1000",
    )
    .bind(&org)
    .fetch_all(&st.postgres)
    .await?;
    let total_teams = all.len();
    let teams: Vec<TeamFacts> = all.into_iter().filter(|(h, ..)| req.teams.is_empty() || req.teams.contains(h)).map(|(handle, name, members)| TeamFacts { handle, name, members }).collect();
    #[derive(sqlx::FromRow)]
    struct Row {
        declared_at: i64,
        resolved_at: Option<i64>,
        open: bool,
        teams: Vec<String>,
        services: Vec<String>,
    }
    let rows: Vec<Row> = sqlx::query_as(
        "SELECT (extract(epoch FROM declared_at) * 1000)::bigint AS declared_at, (extract(epoch FROM resolved_at) * 1000)::bigint AS resolved_at, status <> 'resolved' AS open, teams, services \
         FROM incidents WHERE org_id = $1 AND NOT test AND (status <> 'resolved' OR declared_at >= to_timestamp($2 / 1000.0) OR resolved_at >= to_timestamp($2 / 1000.0))",
    )
    .bind(&org)
    .bind(req.range.from as f64)
    .fetch_all(&st.postgres)
    .await?;
    let incidents: Vec<IncidentFact> = rows.into_iter().map(|r| IncidentFact { declared_at: r.declared_at, resolved_at: r.resolved_at, open: r.open, teams: r.teams, services: r.services }).collect();
    let cov = coverage(&st, &org).await?;
    let (buckets, bucket_ms, teams) = teams_report(req.range, &teams, &incidents, &cov);
    Ok(Json(PortalTeamsReport { buckets, bucket_ms, teams, total_teams }))
}

/// Scorecards Performance: the mean scorecard level of each team's services per day.
#[utoipa::path(post, path = "/api/v1/portal/reports/scorecards", operation_id = "portal_scorecards_report", request_body = PortalReportRequest, responses((status = 200, body = PortalScorecardsReport)), tag = "portal")]
pub async fn scorecards(State(st): State<AppState>, OrgId(org): OrgId, Json(req): Json<PortalReportRequest>) -> ApiResult<PortalScorecardsReport> {
    req.range.validate_loose()?;
    let rows: Vec<(i64, String, String, i16)> = sqlx::query_as(
        "SELECT (extract(epoch FROM day::timestamp AT TIME ZONE 'UTC') * 1000)::bigint, service, team, level FROM portal_scorecard_levels \
         WHERE org_id = $1 AND day >= (to_timestamp($2 / 1000.0) AT TIME ZONE 'UTC')::date AND day <= (to_timestamp($3 / 1000.0) AT TIME ZONE 'UTC')::date",
    )
    .bind(&org)
    .bind(req.range.from as f64)
    .bind(req.range.to as f64)
    .fetch_all(&st.postgres)
    .await?;
    let (days, teams) = scorecard_trend(req.range, &rows, &req.teams);
    let rule_days: Vec<(i64, String, i64, i64)> = sqlx::query_as(
        "SELECT (extract(epoch FROM day::timestamp AT TIME ZONE 'UTC') * 1000)::bigint, rule_id::text, sum(pass)::bigint, sum(fail)::bigint FROM portal_scorecard_rule_days \
         WHERE org_id = $1 AND day >= (to_timestamp($2 / 1000.0) AT TIME ZONE 'UTC')::date AND day <= (to_timestamp($3 / 1000.0) AT TIME ZONE 'UTC')::date \
         AND (cardinality($4::text[]) = 0 OR team = ANY($4)) GROUP BY 1, 2 ORDER BY 1",
    )
    .bind(&org)
    .bind(req.range.from as f64)
    .bind(req.range.to as f64)
    .bind(&req.teams)
    .fetch_all(&st.postgres)
    .await?;
    let rule_days = rule_days.into_iter().map(|(day, rule_id, pass, fail)| PortalScorecardRuleDay { day, rule_id, pass, fail }).collect();
    Ok(Json(PortalScorecardsReport { days, teams, rule_days }))
}

/// Custom Reports: the dashboards added to the Reports tab, in order.
#[utoipa::path(get, path = "/api/v1/portal/reports/dashboards", operation_id = "list_portal_report_dashboards", responses((status = 200, body = Vec<PortalReportDashboard>)), tag = "portal")]
pub async fn list_dashboards(State(st): State<AppState>, ctx: AuthContext) -> ApiResult<Vec<PortalReportDashboard>> {
    Ok(Json(report_dashboards(&st, &ctx.org_id).await?))
}

/// Replaces the dashboards of Custom Reports (the org's own, not deleted).
#[utoipa::path(put, path = "/api/v1/portal/reports/dashboards", operation_id = "set_portal_report_dashboards", request_body = PortalReportDashboardsInput,
    responses((status = 200, body = Vec<PortalReportDashboard>), (status = 400, body = crate::error::ErrorBody)), tag = "portal")]
pub async fn set_dashboards(State(st): State<AppState>, Writer(ctx): Writer, Json(input): Json<PortalReportDashboardsInput>) -> ApiResult<Vec<PortalReportDashboard>> {
    let mut ids: Vec<String> = Vec::new();
    for id in input.dashboard_ids.iter().map(|i| i.trim().to_string()) {
        if !id.is_empty() && !ids.contains(&id) {
            ids.push(id);
        }
    }
    if ids.len() > MAX_DASHBOARDS {
        return Err(ApiError::BadRequest(format!("Add at most {MAX_DASHBOARDS} dashboards")));
    }
    let (found,): (i64,) = sqlx::query_as("SELECT count(*) FROM dashboards WHERE org_id = $1 AND id = ANY($2) AND deleted_at IS NULL").bind(&ctx.org_id).bind(&ids).fetch_one(&st.postgres).await?;
    if found != ids.len() as i64 {
        return Err(ApiError::BadRequest("Some dashboards don't exist".into()));
    }
    let mut tx = st.postgres.begin().await?;
    sqlx::query("DELETE FROM portal_report_dashboards WHERE org_id = $1").bind(&ctx.org_id).execute(&mut *tx).await?;
    sqlx::query("INSERT INTO portal_report_dashboards (org_id, dashboard_id, position, added_by) SELECT $1, id, (ord - 1)::int, $3::uuid FROM unnest($2::text[]) WITH ORDINALITY AS x(id, ord)")
        .bind(&ctx.org_id)
        .bind(&ids)
        .bind(&ctx.user_id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    ctx.audit(&st, "portal.report_dashboards_updated", "portal_report_dashboards", &format!("{} dashboards", ids.len())).await;
    Ok(Json(report_dashboards(&st, &ctx.org_id).await?))
}

#[cfg(test)]
mod tests {
    use super::*;

    const DAY: i64 = DAY_MS;
    const START: i64 = 1_800_000_000_000 / DAY * DAY;

    fn team(h: &str) -> TeamFacts {
        TeamFacts { handle: h.into(), name: h.to_uppercase(), members: 3 }
    }

    #[test]
    fn short_ranges_bucket_by_day_long_ones_by_week() {
        let week = TimeRange { from: START, to: START + 7 * DAY - 1 };
        assert_eq!(bucket_size(week), DAY);
        assert_eq!(buckets(week, DAY).len(), 7);
        let quarter = TimeRange { from: START, to: START + 90 * DAY };
        assert_eq!(bucket_size(quarter), 7 * DAY);
        assert_eq!(buckets(quarter, 7 * DAY).len(), 14);
    }

    #[test]
    fn incidents_count_for_listed_teams_and_service_owners() {
        let range = TimeRange { from: START, to: START + 3 * DAY - 1 };
        let cov = CoverageFacts {
            owners: HashMap::from([("order-service".into(), "checkout".into()), ("billing".into(), "payments".into())]),
            slo_teams: vec![vec!["checkout".into()], vec!["checkout".into(), "payments".into()]],
            monitor_teams: vec![vec!["payments".into()]],
            schedule_teams: vec!["checkout".into()],
        };
        let incidents = vec![
            // Listed team and its service's owner: counted once.
            IncidentFact { declared_at: START + 3_600_000, resolved_at: Some(START + 2 * 3_600_000), teams: vec!["checkout".into()], services: vec!["order-service".into()], ..Default::default() },
            IncidentFact { declared_at: START + DAY + 10, resolved_at: Some(START + DAY + 10 + 30 * 60_000), services: vec!["billing".into(), "order-service".into()], ..Default::default() },
            // Declared before the range, still open.
            IncidentFact { declared_at: START - 5 * DAY, open: true, teams: vec!["payments".into()], ..Default::default() },
        ];
        let (starts, size, out) = teams_report(range, &[team("checkout"), team("payments"), team("platform")], &incidents, &cov);
        assert_eq!((starts.len(), size), (3, DAY));
        let checkout = &out[0];
        assert_eq!((checkout.incidents.clone(), checkout.incident_count, checkout.open_incidents), (vec![1, 1, 0], 2, 0));
        assert_eq!(checkout.mttr_minutes, Some(45.0));
        assert_eq!((checkout.services, checkout.slos, checkout.monitors, checkout.on_call_schedules), (1, 2, 0, 1));
        let payments = &out[1];
        assert_eq!((payments.incident_count, payments.open_incidents, payments.mttr_minutes, payments.monitors), (1, 1, Some(30.0), 1));
        let platform = &out[2];
        assert_eq!((platform.incident_count, platform.mttr_minutes, platform.slos), (0, None, 0));
    }

    #[test]
    fn scorecard_levels_average_per_team_and_day() {
        let range = TimeRange { from: START, to: START + 3 * DAY - 1 };
        let row = |d: i64, s: &str, t: &str, l: i16| (START + d * DAY, s.to_string(), t.to_string(), l);
        let rows = vec![
            row(0, "order-service", "checkout", 1),
            row(0, "inventory-service", "checkout", 2),
            row(2, "order-service", "checkout", 3),
            row(2, "inventory-service", "checkout", 2),
            row(1, "billing", "", 0),
        ];
        let (days, teams) = scorecard_trend(range, &rows, &[]);
        assert_eq!(days.len(), 3);
        let checkout = teams.iter().find(|t| t.team == "checkout").unwrap();
        assert_eq!(checkout.levels, vec![Some(1.5), None, Some(2.5)]);
        assert_eq!(checkout.current, vec![0, 0, 1, 1]);
        let none = teams.iter().find(|t| t.team.is_empty()).unwrap();
        assert_eq!(none.current, vec![1, 0, 0, 0]);
        let (_, only) = scorecard_trend(range, &rows, &["checkout".into()]);
        assert_eq!(only.len(), 1);
    }
}
