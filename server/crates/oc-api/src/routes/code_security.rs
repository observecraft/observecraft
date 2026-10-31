//! Code Security: vulnerable libraries (SCA), code and secret findings of static scans (SAST,
//! Secrets, IaC) and the code vulnerabilities tracers detect at runtime (IAST).
//!
//! Data comes from Datadog's own senders, unchanged ([`super::code_security_model`] parses them):
//! - Libraries a service loads: the tracers' instrumentation telemetry (`app-dependencies-loaded`),
//!   which the Agent forwards to the intake's `/api/v2/apmtelemetry`; the intake passes it to
//!   [`ingest_telemetry`] (Postgres `cs_libraries`, source `runtime`).
//! - Libraries a repository declares: `datadog-ci sbom upload` ([`upload_sbom`], Datadog's
//!   `/api/v2/static-analysis-sca/dependencies`, application key), source `static`.
//! - Static findings: `datadog-ci sarif upload` (any SARIF 2.1.0 report: Datadog's static analyzer,
//!   Semgrep, Checkov, Gitleaks, …) to the intake's `/api/v2/cicodescan`, passed to [`ingest_sarif`]
//!   (`cs_findings`, `cs_rules`). A scan of a pull request records the findings it would introduce.
//! - IAST: the vulnerabilities tracers attach to spans (`_dd.iast.json`), read from `otel.otel_traces`.
//!
//! Libraries are matched against public advisories (`cs_advisories`): the OSV.dev records of every
//! package some org uses, looked up by [`run_osv_sync`] (`OC_OSV_URL`, off without internet), and
//! in demo mode the demo's own fictional advisories ([`seed_demo`]).
//!
//! Tenancy: org data is filtered on `org_id` (Postgres) and `OrgId = {oc_org:String}` (ClickHouse,
//! through `AppState::ch(org)`). Advisories are public reference data with no org.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::io::Read;
use std::time::Duration;

use axum::body::Bytes;
use axum::extract::{Multipart, Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use utoipa::ToSchema;

use super::code_security_model::{self as model, AdvisoryRef, AffectedRange, IastVulnerability, Library, ScanContext};
use super::datadog_api::{DdAuth, DdError};
use super::logs::TimeRange;
use crate::auth::Writer;
use crate::error::{ApiError, ApiResult};
use crate::state::{AppState, OrgId};

const DAY_MS: i64 = 86_400_000;
/// How long a package's OSV lookup stays fresh.
const OSV_REFRESH_HOURS: i32 = 12;
const OSV_BATCH: usize = 500;
const MAX_PAGE: usize = 200;

fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64)
}

/// The org of a request from the intake's internal listener (checked like the other `/ingest/*`).
pub(crate) fn internal_org(st: &AppState, headers: &HeaderMap) -> Result<String, StatusCode> {
    if let Some(token) = &st.config.internal_token {
        let ok = headers.get("authorization").and_then(|v| v.to_str().ok()).and_then(|v| v.strip_prefix("Bearer ")) == Some(token.as_str());
        if !ok {
            return Err(StatusCode::UNAUTHORIZED);
        }
    }
    headers.get("x-oc-org").and_then(|v| v.to_str().ok()).filter(|o| !o.is_empty()).map(str::to_string).ok_or(StatusCode::BAD_REQUEST)
}

pub(crate) fn gunzip_if(headers: &HeaderMap, body: &[u8]) -> Option<Vec<u8>> {
    let gz = headers.get("content-encoding").and_then(|v| v.to_str().ok()).is_some_and(|v| v.contains("gzip")) || body.starts_with(&[0x1f, 0x8b]);
    if !gz {
        return Some(body.to_vec());
    }
    let mut out = Vec::new();
    flate2::read::GzDecoder::new(body).take(64 * 1024 * 1024).read_to_end(&mut out).ok()?;
    Some(out)
}

// ================================================================ storage

/// Upserts libraries; `replace` first drops the source's previous libraries of the same service
/// (runtime) or repository (static), so an upload is the current inventory.
async fn store_libraries(st: &AppState, org: &str, source: &str, ctx: &ScanContext, language: &str, libs: &[Library], replace: bool) -> Result<(), ApiError> {
    let mut tx = st.postgres.begin().await?;
    if replace {
        if source == "static" {
            sqlx::query("DELETE FROM cs_libraries WHERE org_id = $1 AND source = 'static' AND repository = $2").bind(org).bind(&ctx.repository).execute(&mut *tx).await?;
        } else {
            sqlx::query("DELETE FROM cs_libraries WHERE org_id = $1 AND source = 'runtime' AND service = $2 AND env = $3 AND last_seen < now() - interval '1 day'")
                .bind(org)
                .bind(&ctx.service)
                .bind(&ctx.env)
                .execute(&mut *tx)
                .await?;
        }
    }
    for l in libs {
        let lang = if language.is_empty() { model::language_of_ecosystem(&l.ecosystem) } else { language };
        sqlx::query(
            "INSERT INTO cs_libraries (org_id, source, ecosystem, name, version, service, env, repository, language, branch, commit_sha, file, direct, dev, licenses) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15) \
             ON CONFLICT (org_id, source, ecosystem, name, version, service, env, repository) DO UPDATE SET last_seen = now(), branch = EXCLUDED.branch, \
             commit_sha = EXCLUDED.commit_sha, file = CASE WHEN EXCLUDED.file = '' THEN cs_libraries.file ELSE EXCLUDED.file END, \
             direct = COALESCE(EXCLUDED.direct, cs_libraries.direct), dev = EXCLUDED.dev, licenses = EXCLUDED.licenses",
        )
        .bind(org)
        .bind(source)
        .bind(&l.ecosystem)
        .bind(&l.name)
        .bind(&l.version)
        .bind(&ctx.service)
        .bind(&ctx.env)
        .bind(&ctx.repository)
        .bind(lang)
        .bind(&ctx.branch)
        .bind(&ctx.commit_sha)
        .bind(&l.file)
        .bind(l.direct)
        .bind(l.dev)
        .bind(&l.licenses)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(())
}

async fn store_scan(st: &AppState, org: &str, id: &str, kind: &str, tool: &str, ctx: &ScanContext, results: usize) -> Result<(), ApiError> {
    sqlx::query(
        "INSERT INTO cs_scans (org_id, id, kind, tool, repository, branch, commit_sha, pr_number, base_branch, service, env, results) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12) ON CONFLICT (org_id, id) DO NOTHING",
    )
    .bind(org)
    .bind(id)
    .bind(kind)
    .bind(tool)
    .bind(&ctx.repository)
    .bind(&ctx.branch)
    .bind(&ctx.commit_sha)
    .bind(&ctx.pr_number)
    .bind(&ctx.base_branch)
    .bind(&ctx.service)
    .bind(&ctx.env)
    .bind(results as i32)
    .execute(&st.postgres)
    .await?;
    Ok(())
}

/// Stores a SARIF report's rules and findings. A branch scan updates the repository's findings and
/// resolves those of its tools it no longer reports; a pull request scan only records the findings
/// the change would introduce.
pub async fn store_sarif(st: &AppState, org: &str, ctx: &ScanContext, report: &Value) -> Result<usize, ApiError> {
    let (rules, findings) = model::parse_sarif(report, &ctx.repository);
    let scan_id = crate::auth::crypto::random_token("scan_");
    let tools: BTreeSet<String> = rules.iter().map(|r| r.tool.clone()).chain(findings.iter().map(|f| f.tool.clone())).collect();
    let mut tx = st.postgres.begin().await?;
    for r in &rules {
        sqlx::query(
            "INSERT INTO cs_rules (org_id, kind, tool, rule_id, name, description, severity, cwes, help_uri, language) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10) \
             ON CONFLICT (org_id, tool, rule_id) DO UPDATE SET kind = EXCLUDED.kind, name = EXCLUDED.name, description = EXCLUDED.description, severity = EXCLUDED.severity, \
             cwes = EXCLUDED.cwes, help_uri = EXCLUDED.help_uri, language = EXCLUDED.language",
        )
        .bind(org)
        .bind(r.kind)
        .bind(&r.tool)
        .bind(&r.rule_id)
        .bind(&r.name)
        .bind(&r.description)
        .bind(&r.severity)
        .bind(&r.cwes)
        .bind(&r.help_uri)
        .bind(&r.language)
        .execute(&mut *tx)
        .await?;
    }
    let pr = !ctx.pr_number.is_empty();
    for f in &findings {
        // A pull request's finding is its own row (id + PR), so it never touches the branch's.
        let id = if pr { format!("{}-pr{}", f.id, ctx.pr_number) } else { f.id.clone() };
        if pr {
            let known: Option<(String,)> = sqlx::query_as("SELECT id FROM cs_findings WHERE org_id = $1 AND id = $2 AND resolved_at IS NULL").bind(org).bind(&f.id).fetch_optional(&mut *tx).await?;
            if known.is_some() {
                continue;
            }
        }
        sqlx::query(
            "INSERT INTO cs_findings (org_id, id, kind, tool, rule_id, severity, message, repository, branch, commit_sha, file, line, snippet, pr_number, scan_id) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15) \
             ON CONFLICT (org_id, id) DO UPDATE SET severity = EXCLUDED.severity, message = EXCLUDED.message, branch = EXCLUDED.branch, commit_sha = EXCLUDED.commit_sha, \
             line = EXCLUDED.line, snippet = EXCLUDED.snippet, scan_id = EXCLUDED.scan_id, last_seen = now(), resolved_at = NULL",
        )
        .bind(org)
        .bind(&id)
        .bind(f.kind)
        .bind(&f.tool)
        .bind(&f.rule_id)
        .bind(&f.severity)
        .bind(&f.message)
        .bind(&ctx.repository)
        .bind(&ctx.branch)
        .bind(&ctx.commit_sha)
        .bind(&f.file)
        .bind(f.line)
        .bind(&f.snippet)
        .bind(&ctx.pr_number)
        .bind(&scan_id)
        .execute(&mut *tx)
        .await?;
    }
    if !pr {
        let tools: Vec<String> = tools.iter().cloned().collect();
        sqlx::query("UPDATE cs_findings SET resolved_at = now() WHERE org_id = $1 AND repository = $2 AND pr_number = '' AND tool = ANY($3) AND scan_id <> $4 AND resolved_at IS NULL")
            .bind(org)
            .bind(&ctx.repository)
            .bind(&tools)
            .bind(&scan_id)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    store_scan(st, org, &scan_id, "sarif", &tools.into_iter().collect::<Vec<_>>().join(", "), ctx, findings.len()).await?;
    Ok(findings.len())
}

// ================================================================ ingestion

/// Intake → server: a tracer's instrumentation telemetry message (JSON, maybe gzip).
pub async fn ingest_telemetry(State(st): State<AppState>, headers: HeaderMap, body: Bytes) -> StatusCode {
    let org = match internal_org(&st, &headers) {
        Ok(o) => o,
        Err(s) => return s,
    };
    let Some(raw) = gunzip_if(&headers, &body) else { return StatusCode::BAD_REQUEST };
    let Ok(v) = serde_json::from_slice::<Value>(&raw) else { return StatusCode::BAD_REQUEST };
    let Some((app, libs)) = model::parse_telemetry(&v) else { return StatusCode::ACCEPTED };
    if app.service.is_empty() {
        return StatusCode::ACCEPTED;
    }
    let ctx = ScanContext { service: app.service, env: app.env, ..Default::default() };
    match store_libraries(&st, &org, "runtime", &ctx, &app.language, &libs, true).await {
        Ok(()) => StatusCode::ACCEPTED,
        Err(e) => {
            tracing::warn!(error = %e, "storing runtime libraries failed");
            StatusCode::SERVICE_UNAVAILABLE
        }
    }
}

/// Intake → server: a `datadog-ci sarif upload` (multipart: `event` JSON, `sarif_report_file` gzip).
pub async fn ingest_sarif(State(st): State<AppState>, headers: HeaderMap, mut form: Multipart) -> StatusCode {
    let org = match internal_org(&st, &headers) {
        Ok(o) => o,
        Err(s) => return s,
    };
    let mut event = Value::Null;
    let mut report = None;
    while let Ok(Some(field)) = form.next_field().await {
        let name = field.name().unwrap_or_default().to_string();
        let Ok(data) = field.bytes().await else { return StatusCode::BAD_REQUEST };
        if name == "event" {
            event = serde_json::from_slice(&data).unwrap_or(Value::Null);
        } else if name.starts_with("sarif") {
            let Some(raw) = gunzip_if(&HeaderMap::new(), &data) else { return StatusCode::BAD_REQUEST };
            report = serde_json::from_slice::<Value>(&raw).ok();
        }
    }
    let Some(report) = report else { return StatusCode::BAD_REQUEST };
    let ctx = model::sarif_context(&event);
    if ctx.repository.is_empty() {
        return StatusCode::BAD_REQUEST;
    }
    match store_sarif(&st, &org, &ctx, &report).await {
        Ok(_) => StatusCode::ACCEPTED,
        Err(e) => {
            tracing::warn!(error = %e, "storing a SARIF report failed");
            StatusCode::SERVICE_UNAVAILABLE
        }
    }
}

/// `POST /api/v2/static-analysis-sca/dependencies`: `datadog-ci sbom upload`, a repository's libraries.
pub async fn upload_sbom(State(st): State<AppState>, DdAuth(ctx): DdAuth, raw: Bytes) -> Result<(StatusCode, Json<Value>), DdError> {
    let bad = |m: String| DdError { status: StatusCode::BAD_REQUEST, errors: vec![m] };
    let v: Value = serde_json::from_slice(&raw).map_err(|e| bad(format!("invalid JSON: {e}")))?;
    let (scan, libs) = model::parse_sbom_request(&v).map_err(bad)?;
    if scan.repository.is_empty() {
        return Err(bad("repository.url is required".into()));
    }
    let id = v.pointer("/data/attributes/id").and_then(Value::as_str).map(str::to_string).unwrap_or_else(|| crate::auth::crypto::random_token("scan_"));
    let fail = |e: ApiError| DdError { status: StatusCode::INTERNAL_SERVER_ERROR, errors: vec![e.to_string()] };
    // A pull request's SBOM doesn't change the repository's inventory.
    if scan.pr_number.is_empty() {
        store_libraries(&st, &ctx.org_id, "static", &scan, "", &libs, true).await.map_err(fail)?;
    }
    store_scan(&st, &ctx.org_id, &id, "sbom", "datadog-ci", &scan, libs.len()).await.map_err(fail)?;
    Ok((StatusCode::ACCEPTED, Json(json!({ "data": { "id": id, "type": "scarequests" } }))))
}

// ================================================================ advisories (OSV)

pub(crate) struct AdvisoryWrite<'a>(pub &'a model::Advisory, pub &'a str);

pub(crate) async fn store_advisory(pg: &sqlx::PgPool, AdvisoryWrite(a, source): AdvisoryWrite<'_>) -> Result<(), sqlx::Error> {
    let mut tx = pg.begin().await?;
    sqlx::query(
        "INSERT INTO cs_advisories (id, aliases, summary, details, severity, score, vector, cwes, refs, published, modified, withdrawn, source, fetched_at) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9::jsonb, to_timestamp($10 / 1000.0), to_timestamp($11 / 1000.0), to_timestamp($12 / 1000.0), $13, now()) \
         ON CONFLICT (id) DO UPDATE SET aliases = EXCLUDED.aliases, summary = EXCLUDED.summary, details = EXCLUDED.details, severity = EXCLUDED.severity, \
         score = EXCLUDED.score, vector = EXCLUDED.vector, cwes = EXCLUDED.cwes, refs = EXCLUDED.refs, published = EXCLUDED.published, modified = EXCLUDED.modified, \
         withdrawn = EXCLUDED.withdrawn, source = EXCLUDED.source, fetched_at = now()",
    )
    .bind(&a.id)
    .bind(&a.aliases)
    .bind(&a.summary)
    .bind(&a.details)
    .bind(&a.severity)
    .bind(a.score.map(|s| s as f32))
    .bind(&a.vector)
    .bind(&a.cwes)
    .bind(serde_json::to_string(&a.refs).unwrap_or_else(|_| "[]".into()))
    .bind(a.published.map(|t| t as f64))
    .bind(a.modified.map(|t| t as f64))
    .bind(a.withdrawn.map(|t| t as f64))
    .bind(source)
    .execute(&mut *tx)
    .await?;
    sqlx::query("DELETE FROM cs_advisory_packages WHERE advisory_id = $1").bind(&a.id).execute(&mut *tx).await?;
    for p in &a.packages {
        sqlx::query("INSERT INTO cs_advisory_packages (advisory_id, ecosystem, package, ranges, versions, fixed) VALUES ($1, $2, $3, $4::jsonb, $5, $6) ON CONFLICT DO NOTHING")
            .bind(&a.id)
            .bind(&p.ecosystem)
            .bind(&p.package)
            .bind(serde_json::to_string(&p.ranges).unwrap_or_else(|_| "[]".into()))
            .bind(&p.versions)
            .bind(&p.fixed)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await
}

/// One pass of the OSV lookup: the packages some org uses that weren't looked up recently.
async fn sync_osv(st: &AppState, base: &str, http: &reqwest::Client) -> anyhow::Result<usize> {
    // Package names only (no org data leaves the server, and none is read per org here): the
    // libraries of Code Security and the packages of Cloud Security's SBOMs.
    let due: Vec<(String, String)> = sqlx::query_as(
        "SELECT DISTINCT l.ecosystem, l.name FROM (SELECT ecosystem, name FROM cs_libraries UNION SELECT ecosystem, name FROM csm_packages) l \
         LEFT JOIN cs_osv_packages o ON o.ecosystem = l.ecosystem AND o.package = l.name WHERE o.checked_at IS NULL OR o.checked_at < now() - make_interval(hours => $1) LIMIT $2",
    )
    .bind(OSV_REFRESH_HOURS)
    .bind(OSV_BATCH as i64)
    .fetch_all(&st.postgres)
    .await?;
    if due.is_empty() {
        return Ok(0);
    }
    let queries: Vec<Value> = due.iter().map(|(e, n)| json!({ "package": { "ecosystem": e, "name": n } })).collect();
    let res: Value = http.post(format!("{base}/v1/querybatch")).json(&json!({ "queries": queries })).send().await?.error_for_status()?.json().await?;
    let mut ids: BTreeMap<String, String> = BTreeMap::new();
    for r in res.get("results").and_then(Value::as_array).into_iter().flatten() {
        for v in r.get("vulns").and_then(Value::as_array).into_iter().flatten() {
            if let Some(id) = v.get("id").and_then(Value::as_str) {
                ids.insert(id.to_string(), v.get("modified").and_then(Value::as_str).unwrap_or("").to_string());
            }
        }
    }
    let known: Vec<(String, Option<i64>)> = sqlx::query_as("SELECT id, (EXTRACT(EPOCH FROM modified) * 1000)::bigint FROM cs_advisories WHERE id = ANY($1)")
        .bind(ids.keys().cloned().collect::<Vec<_>>())
        .fetch_all(&st.postgres)
        .await?;
    let known: HashMap<String, Option<i64>> = known.into_iter().collect();
    let mut fetched = 0;
    for (id, modified) in &ids {
        let modified = chrono::DateTime::parse_from_rfc3339(modified).ok().map(|d| d.timestamp_millis());
        if let Some(prev) = known.get(id) {
            if prev.is_some() && *prev >= modified {
                continue;
            }
        }
        let v: Value = match http.get(format!("{base}/v1/vulns/{id}")).send().await.and_then(|r| r.error_for_status()) {
            Ok(r) => r.json().await?,
            Err(e) => {
                tracing::warn!(id, error = %e, "fetching an OSV advisory failed");
                continue;
            }
        };
        if let Some(a) = model::parse_osv(&v) {
            store_advisory(&st.postgres, AdvisoryWrite(&a, "osv")).await?;
            fetched += 1;
        }
    }
    let (ecos, names): (Vec<String>, Vec<String>) = due.into_iter().unzip();
    sqlx::query(
        "INSERT INTO cs_osv_packages (ecosystem, package, checked_at) SELECT * , now() FROM unnest($1::text[], $2::text[]) \
         ON CONFLICT (ecosystem, package) DO UPDATE SET checked_at = now()",
    )
    .bind(&ecos)
    .bind(&names)
    .execute(&st.postgres)
    .await?;
    Ok(fetched)
}

/// Keeps the advisories of the packages in use current (every few minutes, one server at a time).
pub async fn run_osv_sync(st: AppState) {
    let Some(base) = st.config.osv_url.clone() else {
        tracing::info!("OSV lookups are off (OC_OSV_URL): Code Security matches only advisories already stored");
        return;
    };
    let http = reqwest::Client::builder().timeout(Duration::from_secs(60)).user_agent("ObserveCraft-CodeSecurity").build().unwrap_or_default();
    let mut tick = tokio::time::interval(Duration::from_secs(300));
    loop {
        tick.tick().await;
        let Ok(mut conn) = st.postgres.acquire().await else { continue };
        let locked: (bool,) = match sqlx::query_as("SELECT pg_try_advisory_lock(7310)").fetch_one(&mut *conn).await {
            Ok(r) => r,
            Err(_) => continue,
        };
        if !locked.0 {
            continue;
        }
        // Several passes while packages are due, so a new inventory is covered quickly.
        for _ in 0..10 {
            match sync_osv(&st, &base, &http).await {
                Ok(0) => break,
                Ok(n) => tracing::info!(advisories = n, "fetched OSV advisories"),
                Err(e) => {
                    tracing::warn!(error = %e, "OSV lookup failed");
                    break;
                }
            }
        }
        let _ = sqlx::query("SELECT pg_advisory_unlock(7310)").execute(&mut *conn).await;
    }
}

// ================================================================ vulnerabilities (SCA)

#[derive(Debug, Clone, Default, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", default)]
pub struct CsFilter {
    /// Free text: advisory id, CVE, package, rule, file, message.
    pub query: String,
    pub severity: Vec<String>,
    pub repository: String,
    pub service: String,
    /// open (default) | muted | resolved | all
    pub status: String,
    /// production (default for libraries: not dev-only) | all
    pub scope: String,
    pub language: String,
    pub offset: usize,
    pub limit: usize,
}

impl CsFilter {
    fn page<T>(&self, rows: Vec<T>) -> (usize, Vec<T>) {
        let total = rows.len();
        let limit = if self.limit == 0 { 25 } else { self.limit.min(MAX_PAGE) };
        (total, rows.into_iter().skip(self.offset).take(limit).collect())
    }
    fn severity_ok(&self, s: &str) -> bool {
        self.severity.is_empty() || self.severity.iter().any(|x| x.eq_ignore_ascii_case(s))
    }
    fn text_ok(&self, hay: &[&str]) -> bool {
        let q = self.query.trim().to_lowercase();
        q.is_empty() || q.split_whitespace().all(|w| hay.iter().any(|h| h.to_lowercase().contains(w)))
    }
}

#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct Occurrence {
    /// runtime (a service loads it) | static (a repository declares it)
    pub source: String,
    pub service: String,
    pub env: String,
    pub repository: String,
    pub file: String,
    /// A development dependency of the repository (tests, build tools).
    pub dev: bool,
    pub first_seen: i64,
    pub last_seen: i64,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct LibraryVulnerability {
    /// `advisory|ecosystem|package|version`.
    pub key: String,
    pub advisory_id: String,
    pub cve: Option<String>,
    pub aliases: Vec<String>,
    pub title: String,
    pub severity: String,
    pub score: Option<f64>,
    pub cwes: Vec<String>,
    pub ecosystem: String,
    pub language: String,
    pub package: String,
    pub version: String,
    pub fixed: Option<String>,
    pub services: Vec<String>,
    pub repositories: Vec<String>,
    pub envs: Vec<String>,
    /// Loaded by a service running in a production environment.
    pub in_production: bool,
    pub direct: Option<bool>,
    pub first_seen: i64,
    pub last_seen: i64,
    pub status: String,
    pub occurrences: Vec<Occurrence>,
}

#[derive(sqlx::FromRow)]
struct MatchRow {
    source: String,
    ecosystem: String,
    name: String,
    version: String,
    service: String,
    env: String,
    repository: String,
    language: String,
    file: String,
    direct: Option<bool>,
    dev: bool,
    first_seen: i64,
    last_seen: i64,
    advisory_id: String,
    ranges: String,
    versions: Vec<String>,
    aliases: Vec<String>,
    summary: String,
    severity: String,
    score: Option<f32>,
    cwes: Vec<String>,
}

fn is_production(env: &str) -> bool {
    matches!(env.to_lowercase().as_str(), "prod" | "production" | "prd" | "live")
}

/// Every library vulnerability of the org: each (advisory, package version) its libraries match.
async fn library_vulnerabilities(st: &AppState, org: &str) -> Result<Vec<LibraryVulnerability>, ApiError> {
    let rows: Vec<MatchRow> = sqlx::query_as(
        "SELECT l.source, l.ecosystem, l.name, l.version, l.service, l.env, l.repository, l.language, l.file, l.direct, l.dev, \
         (EXTRACT(EPOCH FROM l.first_seen) * 1000)::bigint AS first_seen, (EXTRACT(EPOCH FROM l.last_seen) * 1000)::bigint AS last_seen, \
         p.advisory_id, p.ranges::text AS ranges, p.versions, a.aliases, a.summary, a.severity, a.score, a.cwes \
         FROM cs_libraries l JOIN cs_advisory_packages p ON p.ecosystem = l.ecosystem AND p.package = l.name \
         JOIN cs_advisories a ON a.id = p.advisory_id WHERE l.org_id = $1 AND a.withdrawn IS NULL",
    )
    .bind(org)
    .fetch_all(&st.postgres)
    .await?;
    let triage = triage_map(st, org).await?;
    let mut out: BTreeMap<String, LibraryVulnerability> = BTreeMap::new();
    for r in rows {
        let ranges: Vec<AffectedRange> = serde_json::from_str(&r.ranges).unwrap_or_default();
        if !model::is_affected(&r.version, &ranges, &r.versions) {
            continue;
        }
        let key = format!("{}|{}|{}|{}", r.advisory_id, r.ecosystem, r.name, r.version);
        let v = out.entry(key.clone()).or_insert_with(|| LibraryVulnerability {
            status: triage.get(&format!("vuln:{}:{}:{}", r.advisory_id, r.ecosystem, r.name)).cloned().unwrap_or_else(|| "open".into()),
            key,
            cve: r.aliases.iter().find(|a| a.starts_with("CVE-")).cloned(),
            aliases: r.aliases.clone(),
            title: if r.summary.is_empty() { r.advisory_id.clone() } else { r.summary.clone() },
            severity: r.severity.clone(),
            score: r.score.map(|s| (s as f64 * 10.0).round() / 10.0),
            cwes: r.cwes.clone(),
            language: if r.language.is_empty() { model::language_of_ecosystem(&r.ecosystem).into() } else { r.language.clone() },
            fixed: model::fix_for(&r.version, &ranges),
            ecosystem: r.ecosystem.clone(),
            package: r.name.clone(),
            version: r.version.clone(),
            advisory_id: r.advisory_id.clone(),
            services: vec![],
            repositories: vec![],
            envs: vec![],
            in_production: false,
            direct: None,
            first_seen: r.first_seen,
            last_seen: r.last_seen,
            occurrences: vec![],
        });
        // Dev-only libraries of a repository never reach production.
        if r.source == "runtime" {
            if !v.services.contains(&r.service) {
                v.services.push(r.service.clone());
            }
            if !r.env.is_empty() && !v.envs.contains(&r.env) {
                v.envs.push(r.env.clone());
            }
            v.in_production |= is_production(&r.env);
        } else if !v.repositories.contains(&r.repository) {
            v.repositories.push(r.repository.clone());
        }
        v.direct = v.direct.or(r.direct);
        v.first_seen = v.first_seen.min(r.first_seen);
        v.last_seen = v.last_seen.max(r.last_seen);
        v.occurrences.push(Occurrence { source: r.source, service: r.service, env: r.env, repository: r.repository, file: r.file, dev: r.dev, first_seen: r.first_seen, last_seen: r.last_seen });
    }
    let mut list: Vec<LibraryVulnerability> = out.into_values().collect();
    list.sort_by(|a, b| {
        model::severity_rank(&b.severity)
            .cmp(&model::severity_rank(&a.severity))
            .then(b.score.unwrap_or(0.0).partial_cmp(&a.score.unwrap_or(0.0)).unwrap_or(std::cmp::Ordering::Equal))
            .then(a.title.cmp(&b.title))
    });
    Ok(list)
}

pub(crate) async fn triage_map(st: &AppState, org: &str) -> Result<HashMap<String, String>, ApiError> {
    let rows: Vec<(String, String)> = sqlx::query_as("SELECT key, status FROM cs_triage WHERE org_id = $1").bind(org).fetch_all(&st.postgres).await?;
    Ok(rows.into_iter().collect())
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct LibraryVulnerabilityPage {
    pub total: usize,
    /// Occurrences (library × service or repository) of the matching vulnerabilities.
    pub findings: usize,
    pub vulnerabilities: Vec<LibraryVulnerability>,
}

fn status_ok(f: &CsFilter, status: &str) -> bool {
    match f.status.as_str() {
        "" | "open" => status == "open",
        "all" => true,
        s => s == status,
    }
}

#[utoipa::path(post, path = "/api/v1/code-security/library-vulnerabilities", request_body = CsFilter, responses((status = 200, body = LibraryVulnerabilityPage)), tag = "code-security")]
pub async fn list_library_vulnerabilities(State(st): State<AppState>, OrgId(org): OrgId, Json(f): Json<CsFilter>) -> ApiResult<LibraryVulnerabilityPage> {
    let rows: Vec<LibraryVulnerability> = library_vulnerabilities(&st, &org)
        .await?
        .into_iter()
        .filter(|v| {
            status_ok(&f, &v.status)
                && f.severity_ok(&v.severity)
                && (f.repository.is_empty() || v.repositories.contains(&f.repository))
                && (f.service.is_empty() || v.services.contains(&f.service))
                && (f.language.is_empty() || v.language == f.language)
                && (f.scope != "production" || v.occurrences.iter().any(|o| !o.dev))
                && f.text_ok(&[&v.advisory_id, v.cve.as_deref().unwrap_or(""), &v.title, &v.package])
        })
        .collect();
    let findings = rows.iter().map(|v| v.occurrences.len()).sum();
    let (total, vulnerabilities) = f.page(rows);
    Ok(Json(LibraryVulnerabilityPage { total, findings, vulnerabilities }))
}

// ================================================================ advisories (CVE explorer)

/// id, aliases, summary, severity, score, published, packages, ecosystems.
type AdvisoryListRow = (String, Vec<String>, String, String, Option<f32>, Option<i64>, Vec<String>, Vec<String>);

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AdvisorySummary {
    pub id: String,
    pub cve: Option<String>,
    pub aliases: Vec<String>,
    pub summary: String,
    pub severity: String,
    pub score: Option<f64>,
    pub packages: Vec<String>,
    pub ecosystems: Vec<String>,
    pub published: Option<i64>,
    /// Some library of the org is in an affected range.
    pub impacted: bool,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AdvisoryPage {
    pub total: usize,
    pub advisories: Vec<AdvisorySummary>,
}

#[derive(Debug, Clone, Default, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", default)]
pub struct AdvisoryFilter {
    pub query: String,
    pub severity: Vec<String>,
    /// impacted | not_impacted
    pub impact: String,
    pub ecosystem: String,
    pub offset: usize,
    pub limit: usize,
}

#[utoipa::path(post, path = "/api/v1/code-security/advisories", request_body = AdvisoryFilter, responses((status = 200, body = AdvisoryPage)), tag = "code-security")]
pub async fn list_advisories(State(st): State<AppState>, OrgId(org): OrgId, Json(f): Json<AdvisoryFilter>) -> ApiResult<AdvisoryPage> {
    let rows: Vec<AdvisoryListRow> = sqlx::query_as(
        "SELECT a.id, a.aliases, a.summary, a.severity, a.score, (EXTRACT(EPOCH FROM a.published) * 1000)::bigint, \
         array_agg(DISTINCT p.package), array_agg(DISTINCT p.ecosystem) FROM cs_advisories a JOIN cs_advisory_packages p ON p.advisory_id = a.id \
         WHERE a.withdrawn IS NULL GROUP BY a.id ORDER BY a.published DESC NULLS LAST",
    )
    .fetch_all(&st.postgres)
    .await?;
    let impacted: HashSet<String> = library_vulnerabilities(&st, &org).await?.into_iter().map(|v| v.advisory_id).collect();
    let cf = CsFilter { query: f.query.clone(), severity: f.severity.clone(), offset: f.offset, limit: f.limit, ..Default::default() };
    let list: Vec<AdvisorySummary> = rows
        .into_iter()
        .map(|(id, aliases, summary, severity, score, published, packages, ecosystems)| AdvisorySummary {
            impacted: impacted.contains(&id),
            cve: aliases.iter().find(|a| a.starts_with("CVE-")).cloned(),
            id,
            aliases,
            summary,
            severity,
            score: score.map(|s| (s as f64 * 10.0).round() / 10.0),
            packages,
            ecosystems,
            published,
        })
        .filter(|a| {
            cf.severity_ok(&a.severity)
                && (f.ecosystem.is_empty() || a.ecosystems.contains(&f.ecosystem))
                && match f.impact.as_str() {
                    "impacted" => a.impacted,
                    "not_impacted" => !a.impacted,
                    _ => true,
                }
                && cf.text_ok(&[&a.id, &a.aliases.join(" "), &a.summary, &a.packages.join(" ")])
        })
        .collect();
    let (total, advisories) = cf.page(list);
    Ok(Json(AdvisoryPage { total, advisories }))
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AdvisoryPackageInfo {
    pub ecosystem: String,
    pub package: String,
    pub ranges: Vec<AffectedRange>,
    pub fixed: String,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AdvisoryDetail {
    pub id: String,
    pub aliases: Vec<String>,
    pub summary: String,
    pub details: String,
    pub severity: String,
    pub score: Option<f64>,
    pub vector: String,
    pub cwes: Vec<String>,
    pub refs: Vec<AdvisoryRef>,
    pub published: Option<i64>,
    pub modified: Option<i64>,
    pub source: String,
    pub packages: Vec<AdvisoryPackageInfo>,
    /// The org's vulnerabilities from this advisory.
    pub vulnerabilities: Vec<LibraryVulnerability>,
    pub services_analyzed: usize,
    pub repositories_analyzed: usize,
}

#[derive(sqlx::FromRow)]
struct AdvisoryRow {
    id: String,
    aliases: Vec<String>,
    summary: String,
    details: String,
    severity: String,
    score: Option<f32>,
    vector: String,
    cwes: Vec<String>,
    refs: String,
    published: Option<i64>,
    modified: Option<i64>,
    source: String,
}

#[utoipa::path(get, path = "/api/v1/code-security/advisories/{id}", params(("id" = String, Path)), responses((status = 200, body = AdvisoryDetail)), tag = "code-security")]
pub async fn get_advisory(State(st): State<AppState>, OrgId(org): OrgId, Path(id): Path<String>) -> ApiResult<AdvisoryDetail> {
    let a: AdvisoryRow = sqlx::query_as(
        "SELECT id, aliases, summary, details, severity, score, vector, cwes, refs::text AS refs, (EXTRACT(EPOCH FROM published) * 1000)::bigint AS published, \
         (EXTRACT(EPOCH FROM modified) * 1000)::bigint AS modified, source FROM cs_advisories WHERE id = $1",
    )
    .bind(&id)
    .fetch_optional(&st.postgres)
    .await?
    .ok_or_else(|| ApiError::NotFound("advisory not found".into()))?;
    let packages: Vec<(String, String, String, String)> =
        sqlx::query_as("SELECT ecosystem, package, ranges::text, fixed FROM cs_advisory_packages WHERE advisory_id = $1 ORDER BY ecosystem, package").bind(&id).fetch_all(&st.postgres).await?;
    let (services, repos): (i64, i64) = sqlx::query_as(
        "SELECT count(DISTINCT service) FILTER (WHERE source = 'runtime'), count(DISTINCT repository) FILTER (WHERE source = 'static') FROM cs_libraries WHERE org_id = $1",
    )
    .bind(&org)
    .fetch_one(&st.postgres)
    .await?;
    Ok(Json(AdvisoryDetail {
        vulnerabilities: library_vulnerabilities(&st, &org).await?.into_iter().filter(|v| v.advisory_id == id).collect(),
        packages: packages
            .into_iter()
            .map(|(ecosystem, package, ranges, fixed)| AdvisoryPackageInfo { ecosystem, package, ranges: serde_json::from_str(&ranges).unwrap_or_default(), fixed })
            .collect(),
        id: a.id,
        aliases: a.aliases,
        summary: a.summary,
        details: a.details,
        severity: a.severity,
        score: a.score.map(|s| (s as f64 * 10.0).round() / 10.0),
        vector: a.vector,
        cwes: a.cwes,
        refs: serde_json::from_str(&a.refs).unwrap_or_default(),
        published: a.published,
        modified: a.modified,
        source: a.source,
        services_analyzed: services as usize,
        repositories_analyzed: repos as usize,
    }))
}

// ================================================================ static findings (SAST, Secrets, IaC)

#[derive(Serialize, ToSchema, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct StaticFinding {
    pub id: String,
    pub kind: String,
    pub tool: String,
    pub rule_id: String,
    pub rule_name: String,
    pub description: String,
    pub cwes: Vec<String>,
    pub help_uri: String,
    pub severity: String,
    pub message: String,
    pub repository: String,
    pub branch: String,
    pub commit_sha: String,
    pub file: String,
    pub line: i32,
    pub snippet: String,
    pub pr_number: String,
    pub first_seen: i64,
    pub last_seen: i64,
    pub resolved_at: Option<i64>,
    #[sqlx(skip)]
    pub status: String,
}

#[derive(Debug, Clone, Default, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", default)]
pub struct FindingFilter {
    /// sast | secret | iac
    pub kind: String,
    #[serde(flatten)]
    pub filter: CsFilter,
}

async fn static_findings(st: &AppState, org: &str, kind: Option<&str>, prs: bool) -> Result<Vec<StaticFinding>, ApiError> {
    let mut rows: Vec<StaticFinding> = sqlx::query_as(
        "SELECT f.id, f.kind, f.tool, f.rule_id, COALESCE(NULLIF(r.name, ''), f.rule_id) AS rule_name, COALESCE(r.description, '') AS description, \
         COALESCE(r.cwes, '{}') AS cwes, COALESCE(r.help_uri, '') AS help_uri, f.severity, f.message, f.repository, f.branch, f.commit_sha, f.file, f.line, f.snippet, f.pr_number, \
         (EXTRACT(EPOCH FROM f.first_seen) * 1000)::bigint AS first_seen, (EXTRACT(EPOCH FROM f.last_seen) * 1000)::bigint AS last_seen, \
         (EXTRACT(EPOCH FROM f.resolved_at) * 1000)::bigint AS resolved_at \
         FROM cs_findings f LEFT JOIN cs_rules r ON r.org_id = f.org_id AND r.tool = f.tool AND r.rule_id = f.rule_id \
         WHERE f.org_id = $1 AND ($2 = '' OR f.kind = $2) AND ((f.pr_number <> '') = $3) ORDER BY f.last_seen DESC",
    )
    .bind(org)
    .bind(kind.unwrap_or(""))
    .bind(prs)
    .fetch_all(&st.postgres)
    .await?;
    let triage = triage_map(st, org).await?;
    for r in &mut rows {
        r.status = if r.resolved_at.is_some() { "resolved".into() } else { triage.get(&format!("finding:{}", r.id)).cloned().unwrap_or_else(|| "open".into()) };
    }
    rows.sort_by(|a, b| model::severity_rank(&b.severity).cmp(&model::severity_rank(&a.severity)).then(b.last_seen.cmp(&a.last_seen)));
    Ok(rows)
}

#[derive(Serialize, ToSchema)]
pub struct StaticFindingPage {
    pub total: usize,
    pub findings: Vec<StaticFinding>,
}

#[utoipa::path(post, path = "/api/v1/code-security/findings", request_body = FindingFilter, responses((status = 200, body = StaticFindingPage)), tag = "code-security")]
pub async fn list_findings(State(st): State<AppState>, OrgId(org): OrgId, Json(f): Json<FindingFilter>) -> ApiResult<StaticFindingPage> {
    let c = &f.filter;
    let rows: Vec<StaticFinding> = static_findings(&st, &org, Some(&f.kind), false)
        .await?
        .into_iter()
        .filter(|x| status_ok(c, &x.status) && c.severity_ok(&x.severity) && (c.repository.is_empty() || x.repository == c.repository) && c.text_ok(&[&x.rule_id, &x.rule_name, &x.file, &x.message]))
        .collect();
    let (total, findings) = c.page(rows);
    Ok(Json(StaticFindingPage { total, findings }))
}

// ================================================================ IAST

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeCodeVulnerability {
    #[serde(flatten)]
    pub vulnerability: IastVulnerability,
    pub services: Vec<String>,
    pub envs: Vec<String>,
    pub resources: Vec<String>,
    pub count: u64,
    pub first_seen: i64,
    pub last_seen: i64,
    pub trace_id: String,
    pub status: String,
}

#[derive(Debug, Clone, Default, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", default)]
pub struct IastRequest {
    pub range: Option<TimeRange>,
    #[serde(flatten)]
    pub filter: CsFilter,
}

#[derive(clickhouse::Row, Deserialize)]
struct IastRow {
    service: String,
    env: String,
    resource: String,
    json: String,
    ts: i64,
    trace_id: String,
}

async fn iast_vulnerabilities(st: &AppState, org: &str, range: TimeRange) -> Result<Vec<RuntimeCodeVulnerability>, ApiError> {
    let rows: Vec<IastRow> = st
        .ch(org)
        .query(
            "SELECT toString(ServiceName) AS service, ResourceAttributes['deployment.environment'] AS env, toString(SpanName) AS resource, \
             SpanAttributes['_dd.iast.json'] AS json, toUnixTimestamp64Milli(Timestamp) AS ts, TraceId AS trace_id FROM otel.otel_traces \
             WHERE OrgId = {oc_org:String} AND Timestamp >= fromUnixTimestamp64Milli(?) AND Timestamp <= fromUnixTimestamp64Milli(?) \
             AND SpanAttributes['_dd.iast.json'] != '' ORDER BY Timestamp DESC LIMIT 5000",
        )
        .bind(range.from)
        .bind(range.to)
        .fetch_all()
        .await?;
    let triage = triage_map(st, org).await?;
    let mut by: BTreeMap<String, RuntimeCodeVulnerability> = BTreeMap::new();
    for r in rows {
        for v in model::parse_iast(&r.json) {
            let key = if v.hash.is_empty() { format!("{}:{}:{}", v.kind, v.path, v.line) } else { v.hash.clone() };
            let e = by.entry(key.clone()).or_insert_with(|| RuntimeCodeVulnerability {
                status: triage.get(&format!("iast:{key}")).cloned().unwrap_or_else(|| "open".into()),
                vulnerability: v.clone(),
                services: vec![],
                envs: vec![],
                resources: vec![],
                count: 0,
                first_seen: r.ts,
                last_seen: r.ts,
                trace_id: r.trace_id.clone(),
            });
            e.count += 1;
            e.first_seen = e.first_seen.min(r.ts);
            e.last_seen = e.last_seen.max(r.ts);
            for (list, val) in [(&mut e.services, &r.service), (&mut e.envs, &r.env), (&mut e.resources, &r.resource)] {
                if !val.is_empty() && !list.contains(val) {
                    list.push(val.clone());
                }
            }
        }
    }
    let mut out: Vec<RuntimeCodeVulnerability> = by.into_values().collect();
    out.sort_by(|a, b| model::severity_rank(&b.vulnerability.severity).cmp(&model::severity_rank(&a.vulnerability.severity)).then(b.last_seen.cmp(&a.last_seen)));
    Ok(out)
}

fn default_range(r: Option<TimeRange>) -> TimeRange {
    r.unwrap_or_else(|| {
        let to = now_ms();
        TimeRange { from: to - 7 * DAY_MS, to }
    })
}

#[derive(Serialize, ToSchema)]
pub struct RuntimeCodeVulnerabilityPage {
    pub total: usize,
    pub vulnerabilities: Vec<RuntimeCodeVulnerability>,
}

#[utoipa::path(post, path = "/api/v1/code-security/runtime-code-vulnerabilities", request_body = IastRequest, responses((status = 200, body = RuntimeCodeVulnerabilityPage)), tag = "code-security")]
pub async fn list_iast(State(st): State<AppState>, OrgId(org): OrgId, Json(req): Json<IastRequest>) -> ApiResult<RuntimeCodeVulnerabilityPage> {
    let range = default_range(req.range);
    range.validate()?;
    let c = &req.filter;
    let rows: Vec<RuntimeCodeVulnerability> = iast_vulnerabilities(&st, &org, range)
        .await?
        .into_iter()
        .filter(|v| {
            status_ok(c, &v.status)
                && c.severity_ok(&v.vulnerability.severity)
                && (c.service.is_empty() || v.services.contains(&c.service))
                && c.text_ok(&[&v.vulnerability.title, &v.vulnerability.path, &v.vulnerability.method, &v.services.join(" ")])
        })
        .collect();
    let (total, vulnerabilities) = c.page(rows);
    Ok(Json(RuntimeCodeVulnerabilityPage { total, vulnerabilities }))
}

// ================================================================ summary

#[derive(Serialize, ToSchema, Default)]
#[serde(rename_all = "camelCase")]
pub struct SeverityCounts {
    pub critical: usize,
    pub high: usize,
    pub medium: usize,
    pub low: usize,
    pub unknown: usize,
}

impl SeverityCounts {
    fn add(&mut self, s: &str) {
        match s {
            "critical" => self.critical += 1,
            "high" => self.high += 1,
            "medium" => self.medium += 1,
            "low" => self.low += 1,
            _ => self.unknown += 1,
        }
    }
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct GroupCount {
    pub name: String,
    pub count: usize,
    pub critical: usize,
    pub high: usize,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct DayCount {
    pub day: i64,
    pub libraries: usize,
    pub code: usize,
    pub secrets: usize,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct CodeSecuritySummary {
    pub libraries: SeverityCounts,
    pub code: SeverityCounts,
    pub runtime_code: SeverityCounts,
    pub secrets: SeverityCounts,
    pub iac: SeverityCounts,
    /// Library vulnerabilities by stage: on the default branch, loaded in production, with a fix.
    pub in_repositories: usize,
    pub in_production: usize,
    pub fixable: usize,
    pub repositories_analyzed: usize,
    pub services_analyzed: usize,
    /// Advisories published in the last 30 days for packages in use, and how many affect the org.
    pub emerging: Vec<AdvisorySummary>,
    pub by_service: Vec<GroupCount>,
    pub by_repository: Vec<GroupCount>,
    pub history: Vec<DayCount>,
}

fn group(list: impl Iterator<Item = (String, String)>) -> Vec<GroupCount> {
    let mut m: BTreeMap<String, GroupCount> = BTreeMap::new();
    for (name, sev) in list {
        if name.is_empty() {
            continue;
        }
        let g = m.entry(name.clone()).or_insert(GroupCount { name, count: 0, critical: 0, high: 0 });
        g.count += 1;
        g.critical += usize::from(sev == "critical");
        g.high += usize::from(sev == "high");
    }
    let mut v: Vec<GroupCount> = m.into_values().collect();
    v.sort_by_key(|g| std::cmp::Reverse((g.critical, g.high, g.count)));
    v.truncate(10);
    v
}

#[utoipa::path(get, path = "/api/v1/code-security/summary", responses((status = 200, body = CodeSecuritySummary)), tag = "code-security")]
pub async fn summary(State(st): State<AppState>, OrgId(org): OrgId) -> ApiResult<CodeSecuritySummary> {
    let libs: Vec<LibraryVulnerability> = library_vulnerabilities(&st, &org).await?.into_iter().filter(|v| v.status == "open").collect();
    let findings: Vec<StaticFinding> = static_findings(&st, &org, None, false).await?;
    let iast = iast_vulnerabilities(&st, &org, default_range(None)).await.unwrap_or_default();
    let mut s = CodeSecuritySummary {
        libraries: SeverityCounts::default(),
        code: SeverityCounts::default(),
        runtime_code: SeverityCounts::default(),
        secrets: SeverityCounts::default(),
        iac: SeverityCounts::default(),
        in_repositories: libs.iter().filter(|v| !v.repositories.is_empty()).count(),
        in_production: libs.iter().filter(|v| v.in_production).count(),
        fixable: libs.iter().filter(|v| v.fixed.is_some()).count(),
        repositories_analyzed: 0,
        services_analyzed: 0,
        emerging: vec![],
        by_service: group(libs.iter().flat_map(|v| v.services.iter().map(|x| (x.clone(), v.severity.clone())))),
        by_repository: group(libs.iter().flat_map(|v| v.repositories.iter().map(|x| (x.clone(), v.severity.clone()))).chain(
            findings.iter().filter(|f| f.status == "open").map(|f| (f.repository.clone(), f.severity.clone())),
        )),
        history: vec![],
    };
    for v in &libs {
        s.libraries.add(&v.severity);
    }
    for v in iast.iter().filter(|v| v.status == "open") {
        s.runtime_code.add(&v.vulnerability.severity);
    }
    for f in findings.iter().filter(|f| f.status == "open") {
        match f.kind.as_str() {
            "secret" => s.secrets.add(&f.severity),
            "iac" => s.iac.add(&f.severity),
            _ => s.code.add(&f.severity),
        }
    }
    let (services, repos): (i64, i64) = sqlx::query_as(
        "SELECT (SELECT count(DISTINCT service) FROM cs_libraries WHERE org_id = $1 AND source = 'runtime'), \
         (SELECT count(DISTINCT repository) FROM cs_scans WHERE org_id = $1 AND repository <> '')",
    )
    .bind(&org)
    .fetch_one(&st.postgres)
    .await?;
    s.services_analyzed = services as usize;
    s.repositories_analyzed = repos as usize;
    // Open items per day of the last 30: those seen by then and not resolved before.
    let today = now_ms() / DAY_MS * DAY_MS;
    for d in (0..30).rev() {
        let day = today - d * DAY_MS;
        let end = day + DAY_MS;
        s.history.push(DayCount {
            day,
            libraries: libs.iter().filter(|v| v.first_seen < end).count(),
            code: findings.iter().filter(|f| f.kind != "secret" && f.first_seen < end && f.resolved_at.is_none_or(|r| r >= end)).count(),
            secrets: findings.iter().filter(|f| f.kind == "secret" && f.first_seen < end && f.resolved_at.is_none_or(|r| r >= end)).count(),
        });
    }
    // Emerging: recent advisories on packages the org uses.
    let impacted: HashSet<&str> = libs.iter().map(|v| v.advisory_id.as_str()).collect();
    let recent: Vec<AdvisoryListRow> = sqlx::query_as(
        "SELECT a.id, a.aliases, a.summary, a.severity, a.score, (EXTRACT(EPOCH FROM a.published) * 1000)::bigint, array_agg(DISTINCT p.package), array_agg(DISTINCT p.ecosystem) \
         FROM cs_advisories a JOIN cs_advisory_packages p ON p.advisory_id = a.id \
         WHERE a.withdrawn IS NULL AND a.published > now() - interval '30 days' AND a.severity IN ('critical', 'high') \
         AND EXISTS (SELECT 1 FROM cs_libraries l WHERE l.org_id = $1 AND l.ecosystem = p.ecosystem AND l.name = p.package) \
         GROUP BY a.id ORDER BY a.published DESC LIMIT 10",
    )
    .bind(&org)
    .fetch_all(&st.postgres)
    .await?;
    s.emerging = recent
        .into_iter()
        .map(|(id, aliases, summary, severity, score, published, packages, ecosystems)| AdvisorySummary {
            impacted: impacted.contains(id.as_str()),
            cve: aliases.iter().find(|a| a.starts_with("CVE-")).cloned(),
            id,
            aliases,
            summary,
            severity,
            score: score.map(|x| (x as f64 * 10.0).round() / 10.0),
            packages,
            ecosystems,
            published,
        })
        .collect();
    Ok(Json(s))
}

// ================================================================ repositories, inventory, rules, PRs

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct RepositorySummary {
    pub repository: String,
    pub code_vulnerabilities: usize,
    pub code_violations: usize,
    pub library_vulnerabilities: usize,
    pub libraries: usize,
    pub secrets: usize,
    pub iac_violations: usize,
    pub last_scan: Option<i64>,
    pub last_branch: String,
    pub last_commit: String,
}

#[utoipa::path(get, path = "/api/v1/code-security/repositories", responses((status = 200, body = Vec<RepositorySummary>)), tag = "code-security")]
pub async fn repositories(State(st): State<AppState>, OrgId(org): OrgId) -> ApiResult<Vec<RepositorySummary>> {
    let scans: Vec<(String, i64, String, String)> = sqlx::query_as(
        "SELECT DISTINCT ON (repository) repository, (EXTRACT(EPOCH FROM received_at) * 1000)::bigint, branch, commit_sha FROM cs_scans \
         WHERE org_id = $1 AND repository <> '' AND pr_number = '' ORDER BY repository, received_at DESC",
    )
    .bind(&org)
    .fetch_all(&st.postgres)
    .await?;
    let libs: Vec<(String, i64)> = sqlx::query_as("SELECT repository, count(*) FROM cs_libraries WHERE org_id = $1 AND source = 'static' GROUP BY repository").bind(&org).fetch_all(&st.postgres).await?;
    let libs: HashMap<String, i64> = libs.into_iter().collect();
    let vulns = library_vulnerabilities(&st, &org).await?;
    let findings = static_findings(&st, &org, None, false).await?;
    let out = scans
        .into_iter()
        .map(|(repository, last, branch, commit)| {
            let count = |kind: &str, keep: &dyn Fn(&StaticFinding) -> bool| findings.iter().filter(|f| f.repository == repository && f.kind == kind && f.status == "open" && keep(f)).count();
            let all = |_: &StaticFinding| true;
            RepositorySummary {
                code_vulnerabilities: count("sast", &|f| !f.cwes.is_empty()),
                code_violations: count("sast", &|f| f.cwes.is_empty()),
                library_vulnerabilities: vulns.iter().filter(|v| v.status == "open" && v.repositories.contains(&repository)).count(),
                libraries: libs.get(&repository).copied().unwrap_or(0) as usize,
                secrets: count("secret", &all),
                iac_violations: count("iac", &all),
                last_scan: Some(last),
                last_branch: branch,
                last_commit: commit,
                repository,
            }
        })
        .collect();
    Ok(Json(out))
}

#[derive(Debug, Clone, Default, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", default)]
pub struct InventoryRequest {
    /// static (repositories) | runtime (services)
    pub view: String,
    #[serde(flatten)]
    pub filter: CsFilter,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct InventoryLibrary {
    pub ecosystem: String,
    pub language: String,
    pub name: String,
    pub version: String,
    pub licenses: Vec<String>,
    pub vulnerabilities: usize,
    pub worst_severity: String,
    pub locations: Vec<String>,
    pub files: Vec<String>,
    pub first_seen: i64,
    pub direct: Option<bool>,
}

/// ecosystem, language, name, version, licenses, service, repository, file, first seen, direct.
type InventoryRow = (String, String, String, String, Vec<String>, String, String, String, i64, Option<bool>);

#[derive(Serialize, ToSchema)]
pub struct InventoryPage {
    pub total: usize,
    pub libraries: Vec<InventoryLibrary>,
}

#[utoipa::path(post, path = "/api/v1/code-security/inventory/libraries", request_body = InventoryRequest, responses((status = 200, body = InventoryPage)), tag = "code-security")]
pub async fn inventory_libraries(State(st): State<AppState>, OrgId(org): OrgId, Json(req): Json<InventoryRequest>) -> ApiResult<InventoryPage> {
    let source = if req.view == "runtime" { "runtime" } else { "static" };
    let rows: Vec<InventoryRow> = sqlx::query_as(
        "SELECT ecosystem, language, name, version, licenses, service, repository, file, (EXTRACT(EPOCH FROM first_seen) * 1000)::bigint, direct \
         FROM cs_libraries WHERE org_id = $1 AND source = $2",
    )
    .bind(&org)
    .bind(source)
    .fetch_all(&st.postgres)
    .await?;
    let vulns = library_vulnerabilities(&st, &org).await?;
    let mut by: BTreeMap<(String, String, String), InventoryLibrary> = BTreeMap::new();
    for (ecosystem, language, name, version, licenses, service, repository, file, first, direct) in rows {
        let e = by.entry((ecosystem.clone(), name.clone(), version.clone())).or_insert_with(|| {
            let vs: Vec<&LibraryVulnerability> = vulns.iter().filter(|v| v.ecosystem == ecosystem && v.package == name && v.version == version && v.status == "open").collect();
            InventoryLibrary {
                worst_severity: vs.iter().map(|v| v.severity.as_str()).max_by_key(|s| model::severity_rank(s)).unwrap_or("").to_string(),
                vulnerabilities: vs.len(),
                language: if language.is_empty() { model::language_of_ecosystem(&ecosystem).into() } else { language },
                ecosystem,
                name,
                version,
                licenses,
                locations: vec![],
                files: vec![],
                first_seen: first,
                direct,
            }
        });
        let loc = if source == "runtime" { service } else { repository };
        if !loc.is_empty() && !e.locations.contains(&loc) {
            e.locations.push(loc);
        }
        if !file.is_empty() && !e.files.contains(&file) {
            e.files.push(file);
        }
        e.first_seen = e.first_seen.min(first);
    }
    let c = &req.filter;
    let mut list: Vec<InventoryLibrary> = by
        .into_values()
        .filter(|l| (c.language.is_empty() || l.language == c.language) && (c.severity.is_empty() || c.severity_ok(&l.worst_severity)) && c.text_ok(&[&l.name, &l.version, &l.licenses.join(" ")]))
        .collect();
    list.sort_by(|a, b| b.vulnerabilities.cmp(&a.vulnerabilities).then(a.name.cmp(&b.name)));
    let (total, libraries) = c.page(list);
    Ok(Json(InventoryPage { total, libraries }))
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct InventoryService {
    pub service: String,
    pub language: String,
    pub envs: Vec<String>,
    pub libraries: usize,
    pub vulnerabilities: SeverityCounts,
    pub runtime_code_vulnerabilities: usize,
    pub last_seen: i64,
}

#[utoipa::path(get, path = "/api/v1/code-security/inventory/services", responses((status = 200, body = Vec<InventoryService>)), tag = "code-security")]
pub async fn inventory_services(State(st): State<AppState>, OrgId(org): OrgId) -> ApiResult<Vec<InventoryService>> {
    let rows: Vec<(String, String, Vec<String>, i64, i64)> = sqlx::query_as(
        "SELECT service, max(language), array_agg(DISTINCT env), count(DISTINCT (ecosystem, name, version)), (EXTRACT(EPOCH FROM max(last_seen)) * 1000)::bigint \
         FROM cs_libraries WHERE org_id = $1 AND source = 'runtime' GROUP BY service ORDER BY service",
    )
    .bind(&org)
    .fetch_all(&st.postgres)
    .await?;
    let vulns = library_vulnerabilities(&st, &org).await?;
    let iast = iast_vulnerabilities(&st, &org, default_range(None)).await.unwrap_or_default();
    Ok(Json(
        rows.into_iter()
            .map(|(service, language, envs, libraries, last_seen)| {
                let mut counts = SeverityCounts::default();
                for v in vulns.iter().filter(|v| v.status == "open" && v.services.contains(&service)) {
                    counts.add(&v.severity);
                }
                InventoryService {
                    runtime_code_vulnerabilities: iast.iter().filter(|v| v.services.contains(&service)).count(),
                    envs: envs.into_iter().filter(|e| !e.is_empty()).collect(),
                    libraries: libraries as usize,
                    vulnerabilities: counts,
                    service,
                    language,
                    last_seen,
                }
            })
            .collect(),
    ))
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct RuleSummary {
    pub kind: String,
    pub tool: String,
    pub rule_id: String,
    pub name: String,
    pub description: String,
    pub severity: String,
    pub cwes: Vec<String>,
    pub help_uri: String,
    pub language: String,
    pub open_findings: usize,
    pub repositories: Vec<String>,
}

/// kind, tool, rule id, name, description, severity, CWEs, help URI, language.
type RuleRow = (String, String, String, String, String, String, Vec<String>, String, String);

#[derive(Debug, Clone, Default, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", default)]
pub struct RulesRequest {
    /// sast | iac | secret
    pub kind: String,
    #[serde(flatten)]
    pub filter: CsFilter,
}

#[utoipa::path(post, path = "/api/v1/code-security/rules", request_body = RulesRequest, responses((status = 200, body = Vec<RuleSummary>)), tag = "code-security")]
pub async fn rules(State(st): State<AppState>, OrgId(org): OrgId, Json(req): Json<RulesRequest>) -> ApiResult<Vec<RuleSummary>> {
    let rows: Vec<RuleRow> =
        sqlx::query_as("SELECT kind, tool, rule_id, name, description, severity, cwes, help_uri, language FROM cs_rules WHERE org_id = $1 AND ($2 = '' OR kind = $2) ORDER BY rule_id")
            .bind(&org)
            .bind(&req.kind)
            .fetch_all(&st.postgres)
            .await?;
    let findings = static_findings(&st, &org, if req.kind.is_empty() { None } else { Some(&req.kind) }, false).await?;
    let c = &req.filter;
    let mut out: Vec<RuleSummary> = rows
        .into_iter()
        .map(|(kind, tool, rule_id, name, description, severity, cwes, help_uri, language)| {
            let mine: Vec<&StaticFinding> = findings.iter().filter(|f| f.tool == tool && f.rule_id == rule_id && f.status == "open").collect();
            let mut repositories: Vec<String> = mine.iter().map(|f| f.repository.clone()).collect::<BTreeSet<_>>().into_iter().collect();
            repositories.retain(|r| !r.is_empty());
            RuleSummary { open_findings: mine.len(), repositories, kind, tool, rule_id, name, description, severity, cwes, help_uri, language }
        })
        .filter(|r| c.severity_ok(&r.severity) && (c.language.is_empty() || r.language == c.language) && c.text_ok(&[&r.rule_id, &r.name, &r.cwes.join(" ")]))
        .collect();
    out.sort_by(|a, b| model::severity_rank(&b.severity).cmp(&model::severity_rank(&a.severity)).then(b.open_findings.cmp(&a.open_findings)).then(a.rule_id.cmp(&b.rule_id)));
    Ok(Json(out))
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct PullRequestScan {
    pub repository: String,
    pub pr_number: String,
    pub branch: String,
    pub base_branch: String,
    pub commit_sha: String,
    pub received_at: i64,
    pub tool: String,
    /// Findings the pull request would introduce (not on its base branch).
    pub findings: Vec<StaticFinding>,
}

#[utoipa::path(get, path = "/api/v1/code-security/pull-requests", responses((status = 200, body = Vec<PullRequestScan>)), tag = "code-security")]
pub async fn pull_requests(State(st): State<AppState>, OrgId(org): OrgId) -> ApiResult<Vec<PullRequestScan>> {
    let scans: Vec<(String, String, String, String, String, i64, String)> = sqlx::query_as(
        "SELECT DISTINCT ON (repository, pr_number) repository, pr_number, branch, base_branch, commit_sha, (EXTRACT(EPOCH FROM received_at) * 1000)::bigint, tool \
         FROM cs_scans WHERE org_id = $1 AND pr_number <> '' ORDER BY repository, pr_number, received_at DESC",
    )
    .bind(&org)
    .fetch_all(&st.postgres)
    .await?;
    let mut findings = static_findings(&st, &org, None, true).await?;
    let mut out: Vec<PullRequestScan> = scans
        .into_iter()
        .map(|(repository, pr_number, branch, base_branch, commit_sha, received_at, tool)| {
            let (mine, rest): (Vec<StaticFinding>, Vec<StaticFinding>) = std::mem::take(&mut findings).into_iter().partition(|f| f.repository == repository && f.pr_number == pr_number);
            findings = rest;
            PullRequestScan { repository, pr_number, branch, base_branch, commit_sha, received_at, tool, findings: mine }
        })
        .collect();
    out.sort_by_key(|p| std::cmp::Reverse(p.received_at));
    Ok(Json(out))
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct TriageRequest {
    /// `vuln:<advisory>:<ecosystem>:<package>`, `finding:<id>` or `iast:<hash>`.
    pub key: String,
    /// open | muted
    pub status: String,
    #[serde(default)]
    pub reason: String,
}

#[utoipa::path(post, path = "/api/v1/code-security/triage", request_body = TriageRequest, responses((status = 204)), tag = "code-security")]
pub async fn triage(State(st): State<AppState>, Writer(ctx): Writer, Json(req): Json<TriageRequest>) -> Result<StatusCode, ApiError> {
    if !["open", "muted"].contains(&req.status.as_str()) {
        return Err(ApiError::BadRequest("status must be open or muted".into()));
    }
    if !(req.key.starts_with("vuln:") || req.key.starts_with("finding:") || req.key.starts_with("iast:")) || req.key.len() > 512 {
        return Err(ApiError::BadRequest("unknown key".into()));
    }
    sqlx::query(
        "INSERT INTO cs_triage (org_id, key, status, reason, updated_by) VALUES ($1, $2, $3, $4, $5) \
         ON CONFLICT (org_id, key) DO UPDATE SET status = EXCLUDED.status, reason = EXCLUDED.reason, updated_by = EXCLUDED.updated_by, updated_at = now()",
    )
    .bind(&ctx.org_id)
    .bind(&req.key)
    .bind(&req.status)
    .bind(&req.reason)
    .bind(&ctx.user_id)
    .execute(&st.postgres)
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct CodeSecuritySetup {
    pub runtime_services: usize,
    pub repositories_with_libraries: usize,
    pub repositories_with_code_scans: usize,
    pub iast_services: usize,
    pub advisories: usize,
    pub osv_enabled: bool,
    pub last_osv_check: Option<i64>,
}

#[utoipa::path(get, path = "/api/v1/code-security/setup", responses((status = 200, body = CodeSecuritySetup)), tag = "code-security")]
pub async fn setup(State(st): State<AppState>, OrgId(org): OrgId) -> ApiResult<CodeSecuritySetup> {
    let (runtime, static_repos, code_repos): (i64, i64, i64) = sqlx::query_as(
        "SELECT (SELECT count(DISTINCT service) FROM cs_libraries WHERE org_id = $1 AND source = 'runtime'), \
         (SELECT count(DISTINCT repository) FROM cs_libraries WHERE org_id = $1 AND source = 'static'), \
         (SELECT count(DISTINCT repository) FROM cs_scans WHERE org_id = $1 AND kind = 'sarif')",
    )
    .bind(&org)
    .fetch_one(&st.postgres)
    .await?;
    let (advisories, last): (i64, Option<i64>) =
        sqlx::query_as("SELECT (SELECT count(*) FROM cs_advisories), (SELECT (EXTRACT(EPOCH FROM max(checked_at)) * 1000)::bigint FROM cs_osv_packages)").fetch_one(&st.postgres).await?;
    let iast = iast_vulnerabilities(&st, &org, default_range(None)).await.unwrap_or_default();
    let iast_services: HashSet<&String> = iast.iter().flat_map(|v| v.services.iter()).collect();
    Ok(Json(CodeSecuritySetup {
        runtime_services: runtime as usize,
        repositories_with_libraries: static_repos as usize,
        repositories_with_code_scans: code_repos as usize,
        iast_services: iast_services.len(),
        advisories: advisories as usize,
        osv_enabled: st.config.osv_url.is_some(),
        last_osv_check: last,
    }))
}

// ================================================================ demo

/// A fictional advisory against a fictional package, for the demo: id, CVE, summary, CVSS vector,
/// CWE, ecosystem, package, introduced, fixed, days since published.
pub(crate) type DemoAdvisory<'a> = (&'a str, &'a str, &'a str, &'a str, &'a str, &'a str, &'a str, &'a str, &'a str, i64);

pub(crate) fn demo_advisory((id, cve, summary, vector, cwe, ecosystem, package, introduced, fixed, days_ago): DemoAdvisory) -> model::Advisory {
    let score = model::cvss3_score(vector);
    model::Advisory {
        id: id.into(),
        aliases: vec![cve.into()],
        summary: summary.into(),
        details: format!("{summary}. Versions from {introduced} up to (not including) {fixed} are affected; upgrade to {fixed} or later. This is a fictional advisory of the ObserveCraft demo."),
        severity: score.map(model::severity_of_score).unwrap_or("unknown").into(),
        score,
        vector: vector.into(),
        cwes: vec![cwe.into()],
        refs: vec![AdvisoryRef { kind: "ADVISORY".into(), url: format!("https://advisories.example.com/{id}") }],
        published: Some(now_ms() - days_ago * DAY_MS),
        modified: Some(now_ms() - days_ago * DAY_MS),
        withdrawn: None,
        packages: vec![model::AdvisoryPackage {
            ecosystem: ecosystem.into(),
            package: package.into(),
            ranges: vec![AffectedRange { introduced: introduced.into(), fixed: fixed.into(), ..Default::default() }],
            versions: vec![],
            fixed: fixed.into(),
        }],
    }
}

const DEMO_SARIF_REPO: &str = "github.com/example-org/storefront";

/// The demo org's Code Security data: libraries of the demo services and repositories, fictional
/// advisories on fictional packages, and SARIF scans of the demo repositories (one of them a pull
/// request). Idempotent.
pub async fn seed_demo(st: &AppState, org: &str) -> Result<(), ApiError> {
    let (n,): (i64,) = sqlx::query_as("SELECT count(*) FROM cs_scans WHERE org_id = $1").bind(org).fetch_one(&st.postgres).await?;
    if n > 0 {
        return Ok(());
    }
    let n3 = "CVSS:3.1/AV:N/AC:L/PR:N/UI:N/S:U/C:H/I:H/A:H";
    let advisories = [
        demo_advisory(("OCSA-DEMO-2026-0001", "CVE-2026-90001", "example-http-server: Digest authenticator accepts unknown users", "CVSS:3.1/AV:N/AC:L/PR:N/UI:N/S:U/C:H/I:H/A:N", "CWE-287", "Maven", "com.example.web:http-server-core", "11.0.0", "11.0.22", 160)),
        demo_advisory(("OCSA-DEMO-2026-0002", "CVE-2026-90002", "example-http-server: HTTP/2 request headers not validated", "CVSS:3.1/AV:N/AC:L/PR:N/UI:N/S:U/C:H/I:H/A:N", "CWE-20", "Maven", "com.example.web:http-server-core", "11.0.0", "11.0.15", 90)),
        demo_advisory(("OCSA-DEMO-2026-0003", "CVE-2026-90003", "example-json: polymorphic type validator bypass allows arbitrary class instantiation", "CVSS:3.1/AV:N/AC:H/PR:N/UI:N/S:U/C:H/I:H/A:H", "CWE-502", "Maven", "com.example.json:json-databind", "2.0.0", "2.19.4", 21)),
        demo_advisory(("OCSA-DEMO-2026-0004", "CVE-2026-90004", "example-json: unbounded token growth in the data input parser", "CVSS:3.1/AV:N/AC:L/PR:N/UI:N/S:U/C:N/I:N/A:H", "CWE-400", "Maven", "com.example.json:json-core", "2.0.0", "2.19.3", 12)),
        demo_advisory(("OCSA-DEMO-2026-0005", "CVE-2026-90005", "@example/session: session fixation through crafted cookies", "CVSS:3.1/AV:N/AC:L/PR:N/UI:R/S:U/C:H/I:L/A:N", "CWE-384", "npm", "@example/session", "1.0.0", "1.4.2", 45)),
        demo_advisory(("OCSA-DEMO-2026-0006", "CVE-2026-90006", "example-yaml: arbitrary code execution when loading untrusted documents", n3, "CWE-94", "PyPI", "example-yaml", "5.0", "6.0.1", 6)),
        demo_advisory(("OCSA-DEMO-2026-0007", "CVE-2026-90007", "Example.Http.Client: response splitting via redirect headers", "CVSS:3.1/AV:N/AC:H/PR:N/UI:R/S:U/C:L/I:L/A:N", "CWE-113", "NuGet", "example.http.client", "3.0.0", "3.2.5", 70)),
        demo_advisory(("OCSA-DEMO-2026-0008", "CVE-2026-90008", "example.com/grpc-gateway: path traversal in static file handler", "CVSS:3.1/AV:N/AC:L/PR:N/UI:N/S:U/C:H/I:N/A:N", "CWE-22", "Go", "example.com/grpc-gateway", "1.0.0", "1.8.3", 30)),
    ];
    for a in &advisories {
        store_advisory(&st.postgres, AdvisoryWrite(a, "demo")).await?;
    }
    let lib = |eco: &str, name: &str, version: &str, file: &str| Library { ecosystem: eco.into(), name: name.into(), version: version.into(), direct: Some(true), file: file.into(), ..Default::default() };
    let java = [lib("Maven", "com.example.web:http-server-core", "11.0.9", "pom.xml"), lib("Maven", "com.example.json:json-databind", "2.19.2", "pom.xml"), lib("Maven", "com.example.json:json-core", "2.19.2", "pom.xml"), lib("Maven", "com.example.logging:log-core", "3.4.1", "pom.xml")];
    let node = [lib("npm", "@example/session", "1.3.0", "package-lock.json"), lib("npm", "@example/router", "4.2.0", "package-lock.json")];
    let python = [lib("PyPI", "example-yaml", "5.4.1", "requirements.txt"), lib("PyPI", "example-http", "2.31.0", "requirements.txt")];
    let dotnet = [lib("NuGet", "example.http.client", "3.1.0", "Catalog.csproj")];
    let go = [lib("Go", "example.com/grpc-gateway", "1.7.0", "go.mod")];
    let runtime: [(&str, &str, &str, &[Library]); 5] =
        [("storefront-api", "prod", "java", &java), ("api-gateway", "prod", "nodejs", &node), ("billing", "prod", "python", &python), ("catalog-service", "prod", "dotnet", &dotnet), ("order-service", "staging", "go", &go)];
    for (service, env, lang, libs) in runtime {
        let ctx = ScanContext { service: service.into(), env: env.into(), ..Default::default() };
        store_libraries(st, org, "runtime", &ctx, lang, libs, false).await?;
    }
    let repos: [(&str, &[Library]); 4] = [
        (DEMO_SARIF_REPO, &java),
        ("github.com/example-org/api-gateway", &node),
        ("github.com/example-org/billing", &python),
        ("github.com/example-org/order-service", &go),
    ];
    for (repo, libs) in repos {
        let ctx = ScanContext { repository: repo.into(), branch: "main".into(), commit_sha: "4f2a9c1".into(), ..Default::default() };
        store_libraries(st, org, "static", &ctx, "", libs, false).await?;
        store_scan(st, org, &crate::auth::crypto::random_token("scan_"), "sbom", "datadog-ci", &ctx, libs.len()).await?;
    }
    let sarif = |results: Value, rules: Value, tool: &str| json!({ "version": "2.1.0", "runs": [{ "tool": { "driver": { "name": tool, "rules": rules } }, "results": results }] });
    let result = |rule: &str, file: &str, line: i64, msg: &str, snippet: &str| {
        json!({ "ruleId": rule, "message": { "text": msg }, "locations": [{ "physicalLocation": { "artifactLocation": { "uri": file }, "region": { "startLine": line, "snippet": { "text": snippet } } } }] })
    };
    let rule = |id: &str, name: &str, tags: &[&str], desc: &str| json!({ "id": id, "shortDescription": { "text": name }, "fullDescription": { "text": desc }, "properties": { "tags": tags } });
    let code = sarif(
        json!([
            result("python-security/variable-sql-statement-injection", "billing/coupons/validators.py", 138, "SQL statement built from user input", "admin_query = f\"SELECT * FROM coupons WHERE {custom_filter}\""),
            result("java-security/path-traversal", "src/main/java/com/example/storefront/AssetController.java", 61, "File path built from a request parameter", "Files.readAllBytes(Paths.get(baseDir, name))"),
            result("java-best-practices/avoid-printstacktrace", "src/main/java/com/example/storefront/CartService.java", 112, "Use a logger instead of printStackTrace", "e.printStackTrace();"),
        ]),
        json!([
            rule("python-security/variable-sql-statement-injection", "Potential SQL injection", &["DATADOG_RULE_TYPE:STATIC_ANALYSIS", "DATADOG_SEVERITY:CRITICAL", "CWE:89", "DATADOG_LANGUAGE:PYTHON"], "SQL built by string formatting from variables can be injected; use query parameters."),
            rule("java-security/path-traversal", "Path traversal", &["DATADOG_RULE_TYPE:STATIC_ANALYSIS", "DATADOG_SEVERITY:ERROR", "CWE:22", "DATADOG_LANGUAGE:JAVA"], "File paths built from request input can escape the intended directory."),
            rule("java-best-practices/avoid-printstacktrace", "Avoid printStackTrace", &["DATADOG_RULE_TYPE:STATIC_ANALYSIS", "DATADOG_SEVERITY:NOTICE", "DATADOG_LANGUAGE:JAVA"], "Stack traces printed to stderr bypass logging."),
        ]),
        "datadog-static-analyzer",
    );
    let secrets = sarif(
        json!([result("secrets/example-payments-api-key", "docs/README.md", 54, "Payments API key committed in the repository", "PAYMENTS_KEY=pk_live_EXAMPLE…")]),
        json!([rule("secrets/example-payments-api-key", "Payments API key", &["DATADOG_RULE_TYPE:SECRET", "DATADOG_SEVERITY:CRITICAL"], "Matches an API key of the example payments provider.")]),
        "datadog-static-analyzer",
    );
    let iac = sarif(
        json!([
            result("CKV_AWS_28", "infra/dynamodb.tf", 139, "aws_dynamodb_table.orders has no point-in-time recovery", "resource \"aws_dynamodb_table\" \"orders\" {"),
            result("CKV_AWS_20", "infra/s3.tf", 12, "aws_s3_bucket.assets allows public read", "acl = \"public-read\""),
        ]),
        json!([
            {"id": "CKV_AWS_28", "shortDescription": {"text": "DynamoDB point-in-time recovery disabled"}, "defaultConfiguration": {"level": "warning"}, "properties": {"tags": ["CWE:693"]}},
            {"id": "CKV_AWS_20", "shortDescription": {"text": "S3 bucket is publicly readable"}, "defaultConfiguration": {"level": "error"}, "properties": {"security-severity": "9.1", "tags": ["CWE:284"]}}
        ]),
        "Checkov",
    );
    let main = ScanContext { repository: DEMO_SARIF_REPO.into(), branch: "main".into(), commit_sha: "4f2a9c1".into(), ..Default::default() };
    for report in [&code, &secrets, &iac] {
        store_sarif(st, org, &main, report).await?;
    }
    let pr = ScanContext { pr_number: "418".into(), branch: "feature/gift-cards".into(), base_branch: "main".into(), commit_sha: "b71e0d3".into(), ..main.clone() };
    let pr_code = sarif(
        json!([result("java-security/sql-injection", "src/main/java/com/example/storefront/GiftCardDao.java", 37, "SQL statement built from user input", "\"SELECT * FROM gift_cards WHERE code = '\" + code + \"'\"")]),
        json!([rule("java-security/sql-injection", "SQL injection", &["DATADOG_RULE_TYPE:STATIC_ANALYSIS", "DATADOG_SEVERITY:CRITICAL", "CWE:89", "DATADOG_LANGUAGE:JAVA"], "Concatenated SQL can be injected.")]),
        "datadog-static-analyzer",
    );
    store_sarif(st, org, &pr, &pr_code).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filters_page_and_match_text() {
        let f = CsFilter { query: "json core".into(), severity: vec!["HIGH".into()], limit: 2, offset: 1, ..Default::default() };
        assert!(f.text_ok(&["com.example.json:json-core", "x"]));
        assert!(!f.text_ok(&["com.example.json:json-databind"]));
        assert!(f.severity_ok("high") && !f.severity_ok("low"));
        assert_eq!(f.page(vec![1, 2, 3, 4]), (4, vec![2, 3]));
        assert!(status_ok(&CsFilter::default(), "open") && !status_ok(&CsFilter::default(), "muted"));
    }

    #[test]
    fn production_envs() {
        assert!(is_production("prod") && is_production("Production") && !is_production("staging"));
    }

    #[test]
    fn groups_rank_by_critical_then_high() {
        let g = group([("a", "high"), ("b", "critical"), ("a", "high"), ("", "critical")].into_iter().map(|(a, b)| (a.to_string(), b.to_string())));
        assert_eq!(g.iter().map(|x| x.name.as_str()).collect::<Vec<_>>(), vec!["b", "a"]);
    }

    /// The datadog-ci SBOM upload, an advisory and two SARIF scans, end to end against Postgres and
    /// the Datadog-compatible route (skipped without a live database).
    #[tokio::test]
    async fn sbom_and_sarif_uploads_become_vulnerabilities_and_findings() {
        use crate::routes::datadog_api::replay::TestServer;
        let Some(s) = TestServer::start("code-security").await else { return };
        let http = reqwest::Client::new();
        let sbom = json!({ "data": { "type": "scarequests", "attributes": {
            "commit": { "sha": "abc1234", "branch": "main" }, "repository": { "url": "git@github.com:example-org/shop.git" }, "tags": {},
            "dependencies": [{ "name": "core", "group": "org.example", "version": "1.1.0", "language": "jvm", "purl": "pkg:maven/org.example/core@1.1.0" }]
        } } });
        let res = http.post(format!("{}/api/v2/static-analysis-sca/dependencies", s.base)).header("dd-api-key", &s.api_key).header("dd-application-key", &s.app_key).json(&sbom).send().await.unwrap();
        assert_eq!(res.status(), 202, "{}", res.text().await.unwrap_or_default());
        let adv = demo_advisory(("OCSA-TEST-0001", "CVE-2026-99999", "core: flaw", "CVSS:3.1/AV:N/AC:L/PR:N/UI:N/S:U/C:H/I:H/A:H", "CWE-20", "Maven", "org.example:core", "1.0.0", "1.2.0", 3));
        store_advisory(&s.st.postgres, AdvisoryWrite(&adv, "demo")).await.unwrap();
        let vulns: Vec<LibraryVulnerability> = library_vulnerabilities(&s.st, &s.org).await.unwrap().into_iter().filter(|v| v.advisory_id == "OCSA-TEST-0001").collect();
        assert_eq!(vulns.len(), 1);
        assert_eq!((vulns[0].repositories[0].as_str(), vulns[0].fixed.as_deref(), vulns[0].severity.as_str()), ("github.com/example-org/shop", Some("1.2.0"), "critical"));

        let ctx = ScanContext { repository: "github.com/example-org/shop".into(), branch: "main".into(), ..Default::default() };
        let report = |rule: &str| json!({ "runs": [{ "tool": { "driver": { "name": "semgrep" } }, "results": [{ "ruleId": rule, "level": "error", "message": { "text": "x" },
            "locations": [{ "physicalLocation": { "artifactLocation": { "uri": "a.py" }, "region": { "startLine": 3 } } }] }] }] });
        store_sarif(&s.st, &s.org, &ctx, &report("rule-a")).await.unwrap();
        store_sarif(&s.st, &s.org, &ctx, &report("rule-b")).await.unwrap();
        let open: Vec<StaticFinding> = static_findings(&s.st, &s.org, Some("sast"), false).await.unwrap();
        let state = |rule: &str| open.iter().find(|f| f.rule_id == rule).map(|f| f.status.clone());
        // The second scan of the branch no longer reports rule-a: resolved.
        assert_eq!((state("rule-a").as_deref(), state("rule-b").as_deref()), (Some("resolved"), Some("open")));
        // A pull request scan records only what it would add.
        let pr = ScanContext { pr_number: "7".into(), branch: "feature".into(), ..ctx.clone() };
        store_sarif(&s.st, &s.org, &pr, &report("rule-b")).await.unwrap();
        store_sarif(&s.st, &s.org, &pr, &report("rule-c")).await.unwrap();
        let prs = static_findings(&s.st, &s.org, None, true).await.unwrap();
        assert_eq!(prs.iter().map(|f| f.rule_id.as_str()).collect::<Vec<_>>(), vec!["rule-c"]);
        // Advisories are shared by every org of the database: leave none behind.
        sqlx::query("DELETE FROM cs_advisories WHERE id = 'OCSA-TEST-0001'").execute(&s.st.postgres).await.unwrap();
    }

    #[test]
    fn demo_advisories_are_scored() {
        let a = demo_advisory(("OCSA-DEMO-1", "CVE-1", "x", "CVSS:3.1/AV:N/AC:L/PR:N/UI:N/S:U/C:H/I:H/A:H", "CWE-1", "npm", "p", "1.0.0", "1.2.0", 1));
        assert_eq!((a.severity.as_str(), a.score), ("critical", Some(9.8)));
    }
}
