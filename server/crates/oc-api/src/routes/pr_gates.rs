//! Software Delivery → PR Gates: rules a pull request's commit must pass before it merges, checked
//! by a CI step (`POST /api/v1/pr-gates/evaluate` with an application key; the org is the key's).
//!
//! Rules count findings for the repository and commit from CI Visibility (pipeline runs and test
//! results in `otel.otel_ci_traces`, see `ci.rs`): failed pipelines, tests that failed (and never
//! passed) on the commit, and tests that both passed and failed on it (flaky). A rule fails when
//! the count is above its threshold; blocking rules fail the check, non-blocking ones only report.
//! The code vulnerability, code quality, library vulnerability and license rules are kept for when
//! a Code Security product exists: they are reported as not evaluated, never as passed.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use super::ci::STATUS;
use crate::auth::{AuthContext, Writer};
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;

const MAX_RULES: i64 = 200;
const MAX_REPOSITORIES: usize = 100;
const MAX_NAME_CHARS: usize = 200;
/// How far back CI runs of a commit are looked for.
const LOOKBACK_MS: i64 = 30 * 86_400_000;

#[derive(Serialize, Deserialize, ToSchema, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PrGateSource {
    /// Pipeline runs of the commit that failed.
    PipelineFailures,
    /// Tests that failed on the commit and never passed on it.
    TestFailures,
    /// Tests that both passed and failed on the commit.
    FlakyTests,
    CodeVulnerabilities,
    CodeQuality,
    LibraryVulnerabilities,
    LicenseViolations,
}

impl PrGateSource {
    const ALL: [PrGateSource; 7] = [
        PrGateSource::PipelineFailures,
        PrGateSource::TestFailures,
        PrGateSource::FlakyTests,
        PrGateSource::CodeVulnerabilities,
        PrGateSource::CodeQuality,
        PrGateSource::LibraryVulnerabilities,
        PrGateSource::LicenseViolations,
    ];

    fn as_str(self) -> &'static str {
        match self {
            PrGateSource::PipelineFailures => "pipeline_failures",
            PrGateSource::TestFailures => "test_failures",
            PrGateSource::FlakyTests => "flaky_tests",
            PrGateSource::CodeVulnerabilities => "code_vulnerabilities",
            PrGateSource::CodeQuality => "code_quality",
            PrGateSource::LibraryVulnerabilities => "library_vulnerabilities",
            PrGateSource::LicenseViolations => "license_violations",
        }
    }

    fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|k| k.as_str() == s)
    }

    /// Whether this install has the data to evaluate the source (CI Visibility); the others need Code Security.
    pub(crate) fn evaluable(self) -> bool {
        matches!(self, PrGateSource::PipelineFailures | PrGateSource::TestFailures | PrGateSource::FlakyTests)
    }
}

#[derive(Serialize, Deserialize, ToSchema, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PrGateSeverity {
    Critical,
    High,
    Medium,
    Low,
}

impl PrGateSeverity {
    fn as_str(self) -> &'static str {
        match self {
            PrGateSeverity::Critical => "critical",
            PrGateSeverity::High => "high",
            PrGateSeverity::Medium => "medium",
            PrGateSeverity::Low => "low",
        }
    }
    fn parse(s: &str) -> Self {
        match s {
            "critical" => PrGateSeverity::Critical,
            "medium" => PrGateSeverity::Medium,
            "low" => PrGateSeverity::Low,
            _ => PrGateSeverity::High,
        }
    }
}

#[derive(Serialize, ToSchema, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct PrGateRule {
    pub id: String,
    pub name: String,
    pub source: PrGateSource,
    /// Empty = every repository.
    pub repositories: Vec<String>,
    /// The rule fails when the count of findings is above this.
    pub threshold: i32,
    /// Code Security sources: the lowest severity counted.
    pub min_severity: PrGateSeverity,
    pub blocking: bool,
    pub enabled: bool,
    /// False for sources that need Code Security: such rules are stored but not evaluated.
    pub evaluable: bool,
    pub created_by: String,
    pub updated_by: String,
    /// Epoch milliseconds.
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Deserialize, ToSchema, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct PrGateRuleInput {
    pub name: String,
    pub source: PrGateSource,
    #[serde(default)]
    pub repositories: Vec<String>,
    #[serde(default)]
    pub threshold: i32,
    #[serde(default = "default_severity")]
    pub min_severity: PrGateSeverity,
    #[serde(default = "yes")]
    pub blocking: bool,
    #[serde(default = "yes")]
    pub enabled: bool,
}

fn default_severity() -> PrGateSeverity {
    PrGateSeverity::High
}
fn yes() -> bool {
    true
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct PrGateEvaluationRequest {
    /// `example-org/billing` (a repository URL works too).
    pub repository: String,
    /// The pull request's head commit.
    pub sha: String,
}

#[derive(Serialize, Deserialize, ToSchema, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PrGateCheckStatus {
    Passed,
    Failed,
    /// No CI run of the commit is known yet.
    NoData,
    /// The source needs Code Security, which this install doesn't have.
    NotEvaluated,
}

#[derive(Serialize, ToSchema, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PrGateRuleResult {
    pub rule_id: String,
    pub name: String,
    pub source: PrGateSource,
    pub blocking: bool,
    pub status: PrGateCheckStatus,
    /// Findings counted (None when not evaluated or without data).
    pub count: Option<u64>,
    pub threshold: i32,
    pub reason: String,
}

#[derive(Serialize, Deserialize, ToSchema, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PrGateVerdict {
    Passed,
    Failed,
}

/// What CI Visibility knows about the commit.
#[derive(Serialize, ToSchema, Clone, Copy, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PrGateCiSummary {
    pub pipelines: u64,
    pub failed_pipelines: u64,
    pub tests: u64,
    pub failed_tests: u64,
    pub flaky_tests: u64,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct PrGateEvaluation {
    pub repository: String,
    pub sha: String,
    /// Failed when a blocking rule failed: what the PR check reports.
    pub status: PrGateVerdict,
    pub pass: bool,
    /// The enabled rules that apply to the repository.
    pub rules: Vec<PrGateRuleResult>,
    pub ci: PrGateCiSummary,
}

// ================================================================ evaluation logic

/// `https://git.example.com/Example-Org/Billing.git` → `example-org/billing`.
fn normalize_repo(repo: &str) -> String {
    let r = repo.trim().trim_end_matches('/').to_lowercase();
    let r = r.strip_suffix(".git").unwrap_or(&r);
    let r = match r.split_once("://") {
        Some((_, rest)) => rest.split_once('/').map_or("", |(_, path)| path),
        None => r.split_once(':').filter(|(host, _)| host.contains('@')).map_or(r, |(_, path)| path),
    };
    r.to_string()
}

/// Whether a rule scoped to `repositories` (empty = all) applies to `repo`.
pub(crate) fn applies_to(repositories: &[String], repo: &str) -> bool {
    let repo = normalize_repo(repo);
    repositories.is_empty() || repositories.iter().any(|r| normalize_repo(r) == repo)
}

fn source_label(source: PrGateSource) -> &'static str {
    match source {
        PrGateSource::PipelineFailures => "failed pipelines",
        PrGateSource::TestFailures => "failed tests",
        PrGateSource::FlakyTests => "flaky tests",
        PrGateSource::CodeVulnerabilities => "code vulnerabilities",
        PrGateSource::CodeQuality => "code quality violations",
        PrGateSource::LibraryVulnerabilities => "library vulnerabilities",
        PrGateSource::LicenseViolations => "license violations",
    }
}

/// One rule's outcome given what CI Visibility knows about the commit.
pub(crate) fn rule_result(rule: &PrGateRule, ci: &PrGateCiSummary) -> PrGateRuleResult {
    let result = |status, count, reason: String| PrGateRuleResult {
        rule_id: rule.id.clone(),
        name: rule.name.clone(),
        source: rule.source,
        blocking: rule.blocking,
        status,
        count,
        threshold: rule.threshold,
        reason,
    };
    if !rule.source.evaluable() {
        return result(PrGateCheckStatus::NotEvaluated, None, format!("Needs Code Security to count {}: not evaluated", source_label(rule.source)));
    }
    if ci.pipelines == 0 && ci.tests == 0 {
        return result(PrGateCheckStatus::NoData, None, "No CI run of this commit is known yet".into());
    }
    let count = match rule.source {
        PrGateSource::PipelineFailures => ci.failed_pipelines,
        PrGateSource::TestFailures => ci.failed_tests,
        _ => ci.flaky_tests,
    };
    let threshold = u64::try_from(rule.threshold).unwrap_or(0);
    let failed = count > threshold;
    let reason = format!("{count} {} on this commit ({} {threshold} allowed)", source_label(rule.source), if failed { "more than the" } else { "within the" });
    result(if failed { PrGateCheckStatus::Failed } else { PrGateCheckStatus::Passed }, Some(count), reason)
}

/// The check's verdict: failed when a blocking rule failed.
pub(crate) fn pr_verdict(results: &[PrGateRuleResult]) -> PrGateVerdict {
    if results.iter().any(|r| r.blocking && r.status == PrGateCheckStatus::Failed) {
        PrGateVerdict::Failed
    } else {
        PrGateVerdict::Passed
    }
}

/// Trims and checks a rule.
pub(crate) fn validate(input: &mut PrGateRuleInput) -> Result<(), String> {
    input.name = input.name.trim().to_string();
    if input.name.is_empty() || input.name.chars().count() > MAX_NAME_CHARS {
        return Err(format!("The rule name is required (at most {MAX_NAME_CHARS} characters)"));
    }
    input.repositories = input.repositories.iter().map(|r| r.trim().to_string()).filter(|r| !r.is_empty()).collect();
    input.repositories.dedup();
    if input.repositories.len() > MAX_REPOSITORIES || input.repositories.iter().any(|r| r.chars().count() > MAX_NAME_CHARS) {
        return Err(format!("A rule lists up to {MAX_REPOSITORIES} repositories of at most {MAX_NAME_CHARS} characters"));
    }
    if !(0..=100_000).contains(&input.threshold) {
        return Err("The threshold is 0 to 100,000".into());
    }
    Ok(())
}

// ================================================================ storage

#[derive(sqlx::FromRow)]
struct RuleRow {
    id: String,
    name: String,
    source: String,
    repositories: Vec<String>,
    threshold: i32,
    min_severity: String,
    blocking: bool,
    enabled: bool,
    created_by: String,
    updated_by: String,
    created_at: i64,
    updated_at: i64,
}

impl RuleRow {
    fn into_rule(self) -> Option<PrGateRule> {
        let source = PrGateSource::parse(&self.source)?;
        Some(PrGateRule {
            id: self.id,
            name: self.name,
            source,
            repositories: self.repositories,
            threshold: self.threshold,
            min_severity: PrGateSeverity::parse(&self.min_severity),
            blocking: self.blocking,
            enabled: self.enabled,
            evaluable: source.evaluable(),
            created_by: self.created_by,
            updated_by: self.updated_by,
            created_at: self.created_at,
            updated_at: self.updated_at,
        })
    }
}

const COLS: &str = "id::text AS id, name, source, repositories, threshold, min_severity, blocking, enabled, created_by, updated_by, \
     (extract(epoch FROM created_at) * 1000)::bigint AS created_at, (extract(epoch FROM updated_at) * 1000)::bigint AS updated_at";

async fn rules(st: &AppState, org: &str) -> Result<Vec<PrGateRule>, ApiError> {
    let rows: Vec<RuleRow> = sqlx::query_as(&format!("SELECT {COLS} FROM pr_gate_rules WHERE org_id = $1 ORDER BY lower(name)")).bind(org).fetch_all(&st.postgres).await?;
    Ok(rows.into_iter().filter_map(RuleRow::into_rule).collect())
}

async fn get_rule(st: &AppState, org: &str, id: &str) -> Result<PrGateRule, ApiError> {
    let row: Option<RuleRow> = sqlx::query_as(&format!("SELECT {COLS} FROM pr_gate_rules WHERE org_id = $1 AND id::text = $2")).bind(org).bind(id).fetch_optional(&st.postgres).await?;
    row.and_then(RuleRow::into_rule).ok_or_else(|| ApiError::NotFound("PR gate rule not found".into()))
}

fn unique(e: sqlx::Error, name: &str) -> ApiError {
    match &e {
        sqlx::Error::Database(d) if d.is_unique_violation() => ApiError::BadRequest(format!("A rule named '{name}' already exists")),
        _ => e.into(),
    }
}

#[derive(clickhouse::Row, Deserialize)]
struct PipelineCounts {
    pipelines: u64,
    failed_pipelines: u64,
}

#[derive(clickhouse::Row, Deserialize)]
struct TestCounts {
    tests: u64,
    failed_tests: u64,
    flaky_tests: u64,
}

/// Pipelines and tests of the repository's commit, from CI Visibility.
async fn ci_summary(st: &AppState, org: &str, repo: &str, sha: &str) -> Result<PrGateCiSummary, ApiError> {
    let from = super::metrics::now_ms() - LOOKBACK_MS;
    let scope = "Timestamp >= fromUnixTimestamp64Milli(?) AND lower(ResourceAttributes['vcs.repository.name']) = ? AND startsWith(lower(ResourceAttributes['vcs.ref.head.revision']), ?)";
    let pipelines = st
        .ch(org)
        .query(&format!("SELECT countIf(ParentSpanId = '') AS pipelines, countIf(ParentSpanId = '' AND {STATUS} = 'error') AS failed_pipelines FROM otel.otel_ci_traces WHERE OrgId = {{oc_org:String}} AND {scope}"))
        .bind(from)
        .bind(repo)
        .bind(sha)
        .fetch_one::<PipelineCounts>()
        .await?;
    let tests = st
        .ch(org)
        .query(&format!(
            "SELECT count() AS tests, countIf(failed AND NOT passed) AS failed_tests, countIf(failed AND passed) AS flaky_tests FROM ( \
             SELECT SpanAttributes['test.suite.name'] AS suite, SpanAttributes['test.case.name'] AS name, \
             max(SpanAttributes['test.case.result.status'] = 'fail') AS failed, max(SpanAttributes['test.case.result.status'] != 'fail') AS passed \
             FROM otel.otel_ci_traces WHERE OrgId = {{oc_org:String}} AND {scope} AND SpanAttributes['test.case.name'] != '' GROUP BY suite, name)"
        ))
        .bind(from)
        .bind(repo)
        .bind(sha)
        .fetch_one::<TestCounts>()
        .await?;
    Ok(PrGateCiSummary { pipelines: pipelines.pipelines, failed_pipelines: pipelines.failed_pipelines, tests: tests.tests, failed_tests: tests.failed_tests, flaky_tests: tests.flaky_tests })
}

// ================================================================ API

#[utoipa::path(get, path = "/api/v1/pr-gates/rules", operation_id = "list_pr_gate_rules", responses((status = 200, body = Vec<PrGateRule>)), tag = "delivery")]
pub async fn list(State(st): State<AppState>, ctx: AuthContext) -> ApiResult<Vec<PrGateRule>> {
    Ok(Json(rules(&st, &ctx.org_id).await?))
}

#[utoipa::path(post, path = "/api/v1/pr-gates/rules", operation_id = "create_pr_gate_rule", request_body = PrGateRuleInput, responses((status = 200, body = PrGateRule)), tag = "delivery")]
pub async fn create(State(st): State<AppState>, Writer(ctx): Writer, Json(mut input): Json<PrGateRuleInput>) -> ApiResult<PrGateRule> {
    validate(&mut input).map_err(ApiError::BadRequest)?;
    let (count,): (i64,) = sqlx::query_as("SELECT count(*) FROM pr_gate_rules WHERE org_id = $1").bind(&ctx.org_id).fetch_one(&st.postgres).await?;
    if count >= MAX_RULES {
        return Err(ApiError::BadRequest(format!("An organization can have at most {MAX_RULES} PR gate rules")));
    }
    let (id,): (String,) = sqlx::query_as(
        "INSERT INTO pr_gate_rules (org_id, name, source, repositories, threshold, min_severity, blocking, enabled, created_by, updated_by) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $9) RETURNING id::text",
    )
    .bind(&ctx.org_id)
    .bind(&input.name)
    .bind(input.source.as_str())
    .bind(&input.repositories)
    .bind(input.threshold)
    .bind(input.min_severity.as_str())
    .bind(input.blocking)
    .bind(input.enabled)
    .bind(&ctx.name)
    .fetch_one(&st.postgres)
    .await
    .map_err(|e| unique(e, &input.name))?;
    ctx.audit(&st, "pr_gate_rule.created", &input.name, input.source.as_str()).await;
    Ok(Json(get_rule(&st, &ctx.org_id, &id).await?))
}

#[utoipa::path(put, path = "/api/v1/pr-gates/rules/{id}", operation_id = "update_pr_gate_rule", params(("id" = String, Path)), request_body = PrGateRuleInput, responses((status = 200, body = PrGateRule)), tag = "delivery")]
pub async fn update(State(st): State<AppState>, Writer(ctx): Writer, Path(id): Path<String>, Json(mut input): Json<PrGateRuleInput>) -> ApiResult<PrGateRule> {
    validate(&mut input).map_err(ApiError::BadRequest)?;
    let done = sqlx::query(
        "UPDATE pr_gate_rules SET name = $3, source = $4, repositories = $5, threshold = $6, min_severity = $7, blocking = $8, enabled = $9, updated_by = $10, updated_at = now() \
         WHERE org_id = $1 AND id::text = $2",
    )
    .bind(&ctx.org_id)
    .bind(&id)
    .bind(&input.name)
    .bind(input.source.as_str())
    .bind(&input.repositories)
    .bind(input.threshold)
    .bind(input.min_severity.as_str())
    .bind(input.blocking)
    .bind(input.enabled)
    .bind(&ctx.name)
    .execute(&st.postgres)
    .await
    .map_err(|e| unique(e, &input.name))?;
    if done.rows_affected() == 0 {
        return Err(ApiError::NotFound("PR gate rule not found".into()));
    }
    ctx.audit(&st, "pr_gate_rule.updated", &input.name, if input.enabled { "enabled" } else { "disabled" }).await;
    Ok(Json(get_rule(&st, &ctx.org_id, &id).await?))
}

#[utoipa::path(delete, path = "/api/v1/pr-gates/rules/{id}", operation_id = "delete_pr_gate_rule", params(("id" = String, Path)), responses((status = 204)), tag = "delivery")]
pub async fn delete(State(st): State<AppState>, Writer(ctx): Writer, Path(id): Path<String>) -> Result<StatusCode, ApiError> {
    let row: Option<(String,)> = sqlx::query_as("DELETE FROM pr_gate_rules WHERE org_id = $1 AND id::text = $2 RETURNING name").bind(&ctx.org_id).bind(&id).fetch_optional(&st.postgres).await?;
    let Some((name,)) = row else { return Err(ApiError::NotFound("PR gate rule not found".into())) };
    ctx.audit(&st, "pr_gate_rule.deleted", &name, "").await;
    Ok(StatusCode::NO_CONTENT)
}

/// Checks a pull request's commit against the enabled rules that apply to its repository. Called
/// from CI with an application key (`Authorization: Bearer ocak_…`); `pass` is the check's result.
#[utoipa::path(post, path = "/api/v1/pr-gates/evaluate", operation_id = "evaluate_pr_gates", request_body = PrGateEvaluationRequest, responses((status = 200, body = PrGateEvaluation)), tag = "delivery")]
pub async fn evaluate(State(st): State<AppState>, ctx: AuthContext, Json(req): Json<PrGateEvaluationRequest>) -> ApiResult<PrGateEvaluation> {
    let repository = normalize_repo(&req.repository);
    let sha = req.sha.trim().to_lowercase();
    if repository.is_empty() || repository.chars().count() > MAX_NAME_CHARS {
        return Err(ApiError::BadRequest("repository is required, e.g. example-org/billing".into()));
    }
    if sha.len() < 7 || sha.len() > 64 || !sha.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(ApiError::BadRequest("sha is the commit's hexadecimal hash".into()));
    }
    let applicable: Vec<PrGateRule> = rules(&st, &ctx.org_id).await?.into_iter().filter(|r| r.enabled && applies_to(&r.repositories, &repository)).collect();
    let ci = if applicable.iter().any(|r| r.source.evaluable()) { ci_summary(&st, &ctx.org_id, &repository, &sha).await? } else { PrGateCiSummary::default() };
    let results: Vec<PrGateRuleResult> = applicable.iter().map(|r| rule_result(r, &ci)).collect();
    let status = pr_verdict(&results);
    Ok(Json(PrGateEvaluation { repository, sha, pass: status == PrGateVerdict::Passed, status, rules: results, ci }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(source: PrGateSource, threshold: i32, blocking: bool) -> PrGateRule {
        PrGateRule {
            id: "r1".into(),
            name: "No failing tests".into(),
            source,
            repositories: vec![],
            threshold,
            min_severity: PrGateSeverity::High,
            blocking,
            enabled: true,
            evaluable: source.evaluable(),
            created_by: String::new(),
            updated_by: String::new(),
            created_at: 0,
            updated_at: 0,
        }
    }

    fn ci(pipelines: u64, failed_pipelines: u64, tests: u64, failed_tests: u64, flaky_tests: u64) -> PrGateCiSummary {
        PrGateCiSummary { pipelines, failed_pipelines, tests, failed_tests, flaky_tests }
    }

    #[test]
    fn rules_apply_to_their_repositories() {
        let scoped = vec!["example-org/billing".to_string(), "Example-Org/Catalog-Service".to_string()];
        assert!(applies_to(&[], "example-org/anything"), "no list = every repository");
        assert!(applies_to(&scoped, "example-org/billing"));
        assert!(applies_to(&scoped, "example-org/catalog-service"), "case-insensitive");
        assert!(applies_to(&scoped, "https://git.example.com/example-org/billing.git"), "URLs name the repository too");
        assert!(applies_to(&scoped, "git@git.example.com:example-org/billing.git"));
        assert!(!applies_to(&scoped, "example-org/order-service"));
        assert!(!applies_to(&scoped, "example-org/billing-v2"));
    }

    #[test]
    fn ci_rules_count_findings_against_the_threshold() {
        let summary = ci(3, 1, 120, 2, 1);
        let r = rule_result(&rule(PrGateSource::TestFailures, 0, true), &summary);
        assert_eq!((r.status, r.count), (PrGateCheckStatus::Failed, Some(2)));
        let r = rule_result(&rule(PrGateSource::TestFailures, 2, true), &summary);
        assert_eq!((r.status, r.count), (PrGateCheckStatus::Passed, Some(2)), "at the threshold passes");
        let r = rule_result(&rule(PrGateSource::PipelineFailures, 0, true), &summary);
        assert_eq!((r.status, r.count), (PrGateCheckStatus::Failed, Some(1)));
        let r = rule_result(&rule(PrGateSource::FlakyTests, 0, false), &summary);
        assert_eq!((r.status, r.count), (PrGateCheckStatus::Failed, Some(1)));
        assert!(!r.reason.is_empty());
    }

    #[test]
    fn a_commit_without_ci_runs_has_no_data() {
        let r = rule_result(&rule(PrGateSource::TestFailures, 0, true), &PrGateCiSummary::default());
        assert_eq!((r.status, r.count), (PrGateCheckStatus::NoData, None));
        // Pipelines without tests: test rules pass with zero.
        let r = rule_result(&rule(PrGateSource::TestFailures, 0, true), &ci(1, 0, 0, 0, 0));
        assert_eq!((r.status, r.count), (PrGateCheckStatus::Passed, Some(0)));
    }

    #[test]
    fn code_security_rules_are_never_faked() {
        for source in [PrGateSource::CodeVulnerabilities, PrGateSource::CodeQuality, PrGateSource::LibraryVulnerabilities, PrGateSource::LicenseViolations] {
            assert!(!source.evaluable());
            let r = rule_result(&rule(source, 0, true), &ci(3, 0, 10, 0, 0));
            assert_eq!(r.status, PrGateCheckStatus::NotEvaluated, "{source:?}");
            assert_eq!(r.count, None);
            assert!(r.reason.contains("Code Security"), "{}", r.reason);
        }
    }

    #[test]
    fn only_blocking_failures_fail_the_check() {
        let summary = ci(1, 1, 10, 3, 0);
        let blocking = rule_result(&rule(PrGateSource::TestFailures, 0, true), &summary);
        let advisory = rule_result(&rule(PrGateSource::PipelineFailures, 0, false), &summary);
        let not_evaluated = rule_result(&rule(PrGateSource::CodeQuality, 0, true), &summary);
        let no_data = rule_result(&rule(PrGateSource::TestFailures, 0, true), &PrGateCiSummary::default());
        assert_eq!(pr_verdict(&[]), PrGateVerdict::Passed);
        assert_eq!(pr_verdict(&[advisory.clone(), not_evaluated.clone(), no_data]), PrGateVerdict::Passed);
        assert_eq!(pr_verdict(&[advisory, not_evaluated, blocking]), PrGateVerdict::Failed);
    }

    #[test]
    fn rules_are_checked() {
        let base = || PrGateRuleInput {
            name: " No failing tests ".into(),
            source: PrGateSource::TestFailures,
            repositories: vec![" example-org/billing ".into(), "".into()],
            threshold: 0,
            min_severity: PrGateSeverity::High,
            blocking: true,
            enabled: true,
        };
        let mut ok = base();
        validate(&mut ok).unwrap();
        assert_eq!(ok.name, "No failing tests");
        assert_eq!(ok.repositories, vec!["example-org/billing"]);
        let mut no_name = PrGateRuleInput { name: "  ".into(), ..base() };
        assert!(validate(&mut no_name).is_err());
        let mut negative = PrGateRuleInput { threshold: -1, ..base() };
        assert!(validate(&mut negative).is_err());
        let mut many = PrGateRuleInput { repositories: (0..=MAX_REPOSITORIES).map(|i| format!("example-org/r{i}")).collect(), ..base() };
        assert!(validate(&mut many).is_err());
    }
}
