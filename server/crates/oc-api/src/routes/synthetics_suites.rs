//! Synthetic Monitoring & Testing → Test Suites: named groups of tests with tags, an environment
//! and a team. A suite's state is the worst state of its running tests (Alert, then Warn, then OK;
//! No Data only when none of them has a state), its uptime the share of passing scheduled runs of
//! its tests over the requested range.

use std::collections::HashMap;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use super::logs::TimeRange;
use super::monitor_engine::MonitorStatus;
use super::synthetics_exec::SyntheticTestType;
use crate::auth::{AuthContext, Writer};
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;

const MAX_SUITES: i64 = 500;
const MAX_SUITE_TESTS: usize = 200;
const MAX_TAGS: usize = 50;

// ================================================================ API types

#[derive(Deserialize, ToSchema, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SyntheticSuiteInput {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub env: String,
    #[serde(default)]
    pub team: String,
    #[serde(default)]
    pub tags: Vec<String>,
    /// The suite's tests, in order.
    #[serde(default)]
    pub test_ids: Vec<String>,
}

/// A test of a suite with its current state.
#[derive(Serialize, ToSchema, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SyntheticSuiteTest {
    pub id: String,
    pub name: String,
    pub r#type: SyntheticTestType,
    pub status: MonitorStatus,
    pub paused: bool,
}

#[derive(Serialize, ToSchema, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SyntheticSuite {
    pub id: String,
    pub name: String,
    pub description: String,
    pub env: String,
    pub team: String,
    pub tags: Vec<String>,
    pub tests: Vec<SyntheticSuiteTest>,
    /// The worst state of its running tests.
    pub status: MonitorStatus,
    /// Percent of passing scheduled runs of its tests in the range (absent without runs).
    pub uptime: Option<f64>,
    pub created_by: Option<String>,
    pub updated_by: Option<String>,
    /// Epoch milliseconds.
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Deserialize, ToSchema)]
pub struct SyntheticSuiteSearchRequest {
    /// Words match name, description and tags; `env:`, `team:` and `status:` (ok, alert, nodata) filter; other `key:value` words match tags.
    #[serde(default)]
    pub query: String,
    /// The uptime is computed over this range.
    pub range: TimeRange,
}

#[derive(Serialize, ToSchema)]
pub struct SyntheticSuitesPage {
    pub suites: Vec<SyntheticSuite>,
}

// ================================================================ rules

/// The suite's state from its tests' (state, paused).
pub fn suite_status(tests: &[(MonitorStatus, bool)]) -> MonitorStatus {
    let running = || tests.iter().filter(|(_, paused)| !paused).map(|(s, _)| *s);
    for s in [MonitorStatus::Alert, MonitorStatus::Warn, MonitorStatus::Ok] {
        if running().any(|x| x == s) {
            return s;
        }
    }
    MonitorStatus::NoData
}

/// Passing share (percent) of the summed (runs, passed) of the suite's tests.
pub fn suite_uptime(counts: &[(i64, i64)]) -> Option<f64> {
    let (runs, passed) = counts.iter().fold((0i64, 0i64), |(r, p), (runs, passed)| (r + runs, p + passed));
    (runs > 0).then(|| passed as f64 * 100.0 / runs as f64)
}

/// The search box of the suites list.
pub fn matches_suite_query(s: &SyntheticSuite, query: &str) -> bool {
    query.split_whitespace().all(|w| {
        let w = w.to_lowercase();
        let tags = || s.tags.iter().map(|x| x.to_lowercase());
        match w.split_once(':') {
            Some(("env", v)) => s.env.to_lowercase() == v,
            Some(("team", v)) => s.team.to_lowercase() == v,
            Some(("status", v)) => s.status.as_str() == v || (v == "no_data" && s.status == MonitorStatus::NoData),
            Some((k, _)) if !k.is_empty() => tags().any(|x| x == w),
            _ => s.name.to_lowercase().contains(&w) || s.description.to_lowercase().contains(&w) || tags().any(|x| x.contains(&w)),
        }
    })
}

fn is_uuid(s: &str) -> bool {
    s.len() == 36 && s.bytes().enumerate().all(|(i, b)| if matches!(i, 8 | 13 | 18 | 23) { b == b'-' } else { b.is_ascii_hexdigit() })
}

/// Checks and normalizes a suite definition.
pub fn check_suite_input(input: &mut SyntheticSuiteInput) -> Result<(), String> {
    input.name = input.name.trim().to_string();
    if input.name.is_empty() {
        return Err("The name is required".into());
    }
    if input.name.chars().count() > 200 {
        return Err("The name is at most 200 characters".into());
    }
    input.description = input.description.trim().to_string();
    if input.description.chars().count() > 2000 {
        return Err("The description is at most 2,000 characters".into());
    }
    input.env = input.env.trim().to_string();
    input.team = input.team.trim().to_string();
    if input.env.contains([',', ' ']) || input.team.contains(',') || input.env.len() > 100 || input.team.len() > 100 {
        return Err("Environments have no spaces or commas, teams no commas (at most 100 characters)".into());
    }
    let mut tags: Vec<String> = Vec::new();
    for t in &input.tags {
        let t = t.trim().to_string();
        if !t.is_empty() && !tags.contains(&t) {
            tags.push(t);
        }
    }
    if tags.len() > MAX_TAGS || tags.iter().any(|t| t.len() > 200 || t.contains(',')) {
        return Err(format!("Up to {MAX_TAGS} tags of at most 200 characters, without commas"));
    }
    input.tags = tags;
    let mut ids: Vec<String> = Vec::new();
    for id in &input.test_ids {
        let id = id.trim().to_ascii_lowercase();
        if !is_uuid(&id) {
            return Err(format!("Unknown test '{id}'"));
        }
        if !ids.contains(&id) {
            ids.push(id);
        }
    }
    if ids.len() > MAX_SUITE_TESTS {
        return Err(format!("A suite has at most {MAX_SUITE_TESTS} tests"));
    }
    input.test_ids = ids;
    Ok(())
}

// ================================================================ storage

#[derive(sqlx::FromRow)]
struct SuiteRow {
    id: String,
    name: String,
    description: String,
    env: String,
    team: String,
    tags: Vec<String>,
    created_by: Option<String>,
    updated_by: Option<String>,
    created_at: i64,
    updated_at: i64,
}

const SUITE_SELECT: &str = "SELECT s.id::text AS id, s.name, s.description, s.env, s.team, s.tags, c.name AS created_by, u.name AS updated_by, \
     (extract(epoch FROM s.created_at) * 1000)::bigint AS created_at, (extract(epoch FROM s.updated_at) * 1000)::bigint AS updated_at \
     FROM synthetic_suites s LEFT JOIN users c ON c.id = s.created_by LEFT JOIN users u ON u.id = s.updated_by";

fn not_found() -> ApiError {
    ApiError::NotFound("Test suite not found".into())
}

/// Builds the suites of `rows`, with their tests' states and the uptime over `range`.
async fn assemble(st: &AppState, org: &str, rows: Vec<SuiteRow>, range: Option<&TimeRange>) -> Result<Vec<SyntheticSuite>, ApiError> {
    let members: Vec<(String, String, String, String, bool, Option<String>)> = sqlx::query_as(
        "SELECT st.suite_id::text, t.id::text, t.name, t.type, t.paused, m.status FROM synthetic_suite_tests st \
         JOIN synthetic_tests t ON t.id = st.test_id AND t.org_id = st.org_id LEFT JOIN monitors m ON m.id = t.monitor_id AND m.org_id = t.org_id \
         WHERE st.org_id = $1 ORDER BY st.position, lower(t.name)",
    )
    .bind(org)
    .fetch_all(&st.postgres)
    .await?;
    let mut counts: HashMap<String, (i64, i64)> = HashMap::new();
    if let Some(range) = range {
        let rows: Vec<(String, i64, i64)> = sqlx::query_as(
            "SELECT test_id::text, count(*), count(*) FILTER (WHERE passed) FROM synthetic_results \
             WHERE org_id = $1 AND trigger = 'scheduled' AND started_at >= to_timestamp($2) AND started_at <= to_timestamp($3) GROUP BY test_id",
        )
        .bind(org)
        .bind(range.from as f64 / 1000.0)
        .bind(range.to as f64 / 1000.0)
        .fetch_all(&st.postgres)
        .await?;
        counts = rows.into_iter().map(|(id, runs, passed)| (id, (runs, passed))).collect();
    }
    let mut tests: HashMap<String, Vec<SyntheticSuiteTest>> = HashMap::new();
    for (suite, id, name, kind, paused, status) in members {
        let status = status.as_deref().map(MonitorStatus::parse).unwrap_or(MonitorStatus::NoData);
        tests.entry(suite).or_default().push(SyntheticSuiteTest { id, name, r#type: SyntheticTestType::parse(&kind), status, paused });
    }
    Ok(rows
        .into_iter()
        .map(|r| {
            let tests = tests.remove(&r.id).unwrap_or_default();
            let status = suite_status(&tests.iter().map(|t| (t.status, t.paused)).collect::<Vec<_>>());
            let uptime = range.and_then(|_| suite_uptime(&tests.iter().filter_map(|t| counts.get(&t.id).copied()).collect::<Vec<_>>()));
            SyntheticSuite {
                id: r.id,
                name: r.name,
                description: r.description,
                env: r.env,
                team: r.team,
                tags: r.tags,
                tests,
                status,
                uptime,
                created_by: r.created_by,
                updated_by: r.updated_by,
                created_at: r.created_at,
                updated_at: r.updated_at,
            }
        })
        .collect())
}

async fn load(st: &AppState, org: &str, id: &str) -> Result<SyntheticSuite, ApiError> {
    if !is_uuid(id) {
        return Err(not_found());
    }
    let row: Option<SuiteRow> = sqlx::query_as(&format!("{SUITE_SELECT} WHERE s.org_id = $1 AND s.id = $2::uuid")).bind(org).bind(id).fetch_optional(&st.postgres).await?;
    let row = row.ok_or_else(not_found)?;
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0);
    let range = TimeRange { from: now - 24 * 3_600_000, to: now };
    Ok(assemble(st, org, vec![row], Some(&range)).await?.remove(0))
}

/// Replaces the suite's tests; every id must name a test of the org.
async fn set_tests(tx: &mut sqlx::Transaction<'_, sqlx::Postgres>, org: &str, suite: &str, ids: &[String]) -> Result<(), ApiError> {
    if !ids.is_empty() {
        let (known,): (i64,) = sqlx::query_as("SELECT count(*) FROM synthetic_tests WHERE org_id = $1 AND id::text = ANY($2)").bind(org).bind(ids).fetch_one(&mut **tx).await?;
        if known != ids.len() as i64 {
            return Err(ApiError::BadRequest("Unknown test: every test of a suite must exist in the organization".into()));
        }
    }
    sqlx::query("DELETE FROM synthetic_suite_tests WHERE org_id = $1 AND suite_id = $2::uuid").bind(org).bind(suite).execute(&mut **tx).await?;
    for (position, id) in ids.iter().enumerate() {
        sqlx::query("INSERT INTO synthetic_suite_tests (suite_id, test_id, org_id, position) VALUES ($1::uuid, $2::uuid, $3, $4)")
            .bind(suite)
            .bind(id)
            .bind(org)
            .bind(position as i32)
            .execute(&mut **tx)
            .await?;
    }
    Ok(())
}

// ================================================================ handlers

/// The org's suites with their state and uptime over the range.
#[utoipa::path(post, path = "/api/v1/synthetics/suites/search", operation_id = "search_synthetic_suites", request_body = SyntheticSuiteSearchRequest,
    responses((status = 200, body = SyntheticSuitesPage), (status = 400, body = crate::error::ErrorBody)), tag = "synthetics")]
pub async fn search_suites(State(st): State<AppState>, ctx: AuthContext, Json(req): Json<SyntheticSuiteSearchRequest>) -> ApiResult<SyntheticSuitesPage> {
    req.range.validate()?;
    let rows: Vec<SuiteRow> = sqlx::query_as(&format!("{SUITE_SELECT} WHERE s.org_id = $1 ORDER BY lower(s.name), s.id")).bind(&ctx.org_id).fetch_all(&st.postgres).await?;
    let mut suites: Vec<SyntheticSuite> = assemble(&st, &ctx.org_id, rows, Some(&req.range)).await?.into_iter().filter(|s| matches_suite_query(s, &req.query)).collect();
    suites.sort_by_key(|s| std::cmp::Reverse(s.status.severity()));
    Ok(Json(SyntheticSuitesPage { suites }))
}

/// A suite with its tests; the uptime is over the last day.
#[utoipa::path(get, path = "/api/v1/synthetics/suites/{id}", operation_id = "get_synthetic_suite", params(("id" = String, Path)),
    responses((status = 200, body = SyntheticSuite), (status = 404, body = crate::error::ErrorBody)), tag = "synthetics")]
pub async fn get_suite(State(st): State<AppState>, ctx: AuthContext, Path(id): Path<String>) -> ApiResult<SyntheticSuite> {
    Ok(Json(load(&st, &ctx.org_id, &id).await?))
}

#[utoipa::path(post, path = "/api/v1/synthetics/suites", operation_id = "create_synthetic_suite", request_body = SyntheticSuiteInput,
    responses((status = 200, body = SyntheticSuite), (status = 400, body = crate::error::ErrorBody)), tag = "synthetics")]
pub async fn create_suite(State(st): State<AppState>, Writer(ctx): Writer, Json(mut input): Json<SyntheticSuiteInput>) -> ApiResult<SyntheticSuite> {
    check_suite_input(&mut input).map_err(ApiError::BadRequest)?;
    let mut tx = st.postgres.begin().await?;
    let (count,): (i64,) = sqlx::query_as("SELECT count(*) FROM synthetic_suites WHERE org_id = $1").bind(&ctx.org_id).fetch_one(&mut *tx).await?;
    if count >= MAX_SUITES {
        return Err(ApiError::BadRequest(format!("An organization can have at most {MAX_SUITES} test suites")));
    }
    let (id,): (String,) =
        sqlx::query_as("INSERT INTO synthetic_suites (org_id, name, description, env, team, tags, created_by, updated_by) VALUES ($1, $2, $3, $4, $5, $6, $7::uuid, $7::uuid) RETURNING id::text")
            .bind(&ctx.org_id)
            .bind(&input.name)
            .bind(&input.description)
            .bind(&input.env)
            .bind(&input.team)
            .bind(&input.tags)
            .bind(&ctx.user_id)
            .fetch_one(&mut *tx)
            .await?;
    set_tests(&mut tx, &ctx.org_id, &id, &input.test_ids).await?;
    tx.commit().await?;
    ctx.audit(&st, "synthetics.suite_created", &id, &format!("{} ({} tests)", input.name, input.test_ids.len())).await;
    Ok(Json(load(&st, &ctx.org_id, &id).await?))
}

#[utoipa::path(put, path = "/api/v1/synthetics/suites/{id}", operation_id = "update_synthetic_suite", params(("id" = String, Path)), request_body = SyntheticSuiteInput,
    responses((status = 200, body = SyntheticSuite), (status = 400, body = crate::error::ErrorBody), (status = 404, body = crate::error::ErrorBody)), tag = "synthetics")]
pub async fn update_suite(State(st): State<AppState>, Writer(ctx): Writer, Path(id): Path<String>, Json(mut input): Json<SyntheticSuiteInput>) -> ApiResult<SyntheticSuite> {
    check_suite_input(&mut input).map_err(ApiError::BadRequest)?;
    if !is_uuid(&id) {
        return Err(not_found());
    }
    let mut tx = st.postgres.begin().await?;
    let updated = sqlx::query("UPDATE synthetic_suites SET name = $3, description = $4, env = $5, team = $6, tags = $7, updated_by = $8::uuid, updated_at = now() WHERE org_id = $1 AND id = $2::uuid")
        .bind(&ctx.org_id)
        .bind(&id)
        .bind(&input.name)
        .bind(&input.description)
        .bind(&input.env)
        .bind(&input.team)
        .bind(&input.tags)
        .bind(&ctx.user_id)
        .execute(&mut *tx)
        .await?;
    if updated.rows_affected() == 0 {
        return Err(not_found());
    }
    set_tests(&mut tx, &ctx.org_id, &id, &input.test_ids).await?;
    tx.commit().await?;
    ctx.audit(&st, "synthetics.suite_updated", &id, &format!("{} ({} tests)", input.name, input.test_ids.len())).await;
    Ok(Json(load(&st, &ctx.org_id, &id).await?))
}

/// Deletes a suite; its tests stay.
#[utoipa::path(delete, path = "/api/v1/synthetics/suites/{id}", operation_id = "delete_synthetic_suite", params(("id" = String, Path)),
    responses((status = 204), (status = 404, body = crate::error::ErrorBody)), tag = "synthetics")]
pub async fn delete_suite(State(st): State<AppState>, Writer(ctx): Writer, Path(id): Path<String>) -> Result<StatusCode, ApiError> {
    if !is_uuid(&id) {
        return Err(not_found());
    }
    let row: Option<(String,)> = sqlx::query_as("DELETE FROM synthetic_suites WHERE org_id = $1 AND id = $2::uuid RETURNING name").bind(&ctx.org_id).bind(&id).fetch_optional(&st.postgres).await?;
    let Some((name,)) = row else { return Err(not_found()) };
    ctx.audit(&st, "synthetics.suite_deleted", &id, &name).await;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;
    use MonitorStatus::*;

    #[test]
    fn a_suite_is_as_bad_as_its_worst_running_test() {
        assert_eq!(suite_status(&[]), NoData);
        assert_eq!(suite_status(&[(Ok, false), (NoData, false)]), Ok, "a new test doesn't hide the others' state");
        assert_eq!(suite_status(&[(Ok, false), (Alert, false), (Warn, false)]), Alert);
        assert_eq!(suite_status(&[(Ok, false), (Warn, false)]), Warn);
        assert_eq!(suite_status(&[(Ok, false), (Alert, true)]), Ok, "paused tests don't count");
        assert_eq!(suite_status(&[(Alert, true)]), NoData);
        assert_eq!(suite_status(&[(NoData, false)]), NoData);
    }

    #[test]
    fn the_uptime_is_the_share_of_passing_runs() {
        assert_eq!(suite_uptime(&[]), None);
        assert_eq!(suite_uptime(&[(0, 0)]), None);
        assert_eq!(suite_uptime(&[(10, 10), (30, 20)]), Some(75.0));
    }

    fn suite() -> SyntheticSuite {
        SyntheticSuite {
            id: "a".into(),
            name: "Checkout journey".into(),
            description: "Cart, payment and order confirmation".into(),
            env: "prod".into(),
            team: "checkout".into(),
            tags: vec!["service:order-service".into(), "tier:critical".into()],
            tests: vec![],
            status: Alert,
            uptime: None,
            created_by: None,
            updated_by: None,
            created_at: 0,
            updated_at: 0,
        }
    }

    #[test]
    fn the_search_box_filters_suites() {
        let s = suite();
        for q in ["", "checkout", "PAYMENT", "env:prod", "team:checkout", "status:alert", "service:order-service", "critical", "journey env:prod"] {
            assert!(matches_suite_query(&s, q), "{q}");
        }
        for q in ["env:staging", "team:billing", "status:ok", "service:billing", "inventory"] {
            assert!(!matches_suite_query(&s, q), "{q}");
        }
    }

    #[test]
    fn suite_inputs_are_normalized_and_checked() {
        let id = "f1d2c3b4-0001-4a00-8000-00000000a001".to_string();
        let mut ok = SyntheticSuiteInput {
            name: " Checkout journey ".into(),
            description: " x ".into(),
            env: " prod ".into(),
            team: "checkout".into(),
            tags: vec![" tier:critical ".into(), "tier:critical".into(), "".into()],
            test_ids: vec![id.clone(), id.clone()],
        };
        assert_eq!(check_suite_input(&mut ok), Result::Ok(()));
        assert_eq!((ok.name.as_str(), ok.description.as_str(), ok.env.as_str()), ("Checkout journey", "x", "prod"));
        assert_eq!(ok.tags, vec!["tier:critical".to_string()]);
        assert_eq!(ok.test_ids, vec![id.clone()]);
        let bad = |f: fn(&mut SyntheticSuiteInput)| {
            let mut i = SyntheticSuiteInput { name: "S".into(), description: String::new(), env: String::new(), team: String::new(), tags: vec![], test_ids: vec![] };
            f(&mut i);
            check_suite_input(&mut i).unwrap_err()
        };
        assert_eq!(bad(|i| i.name = "  ".into()), "The name is required");
        assert!(bad(|i| i.env = "pr od".into()).contains("Environments"));
        assert!(bad(|i| i.test_ids = vec!["nope".into()]).contains("Unknown test"));
        assert!(bad(|i| i.test_ids = (0..201).map(|n| format!("f1d2c3b4-0001-4a00-8000-{n:012}")).collect()).contains("200 tests"));
        assert!(bad(|i| i.tags = vec!["a,b".into()]).contains("tags"));
    }
}
