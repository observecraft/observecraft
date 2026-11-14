//! Remediation of security findings (ADR-0008, levels Explain, Fix in code and Fix in infrastructure): which level a
//! finding gets, Assist runs that explain a finding (kind `explain`), fix it in its file (kind `fix`) or where its
//! provenance says the resource is defined (kind `fix_infra`), deterministic manifest bumps for library vulnerabilities
//! and S3 public access blocks, tags added to a repository from Tag Coverage (kind `add_tags`, or the AWS provider's
//! `default_tags` and OCI labels without an LLM), and the merge request opened from a fix through the org's source code
//! connections (Developer Portal → Settings), linked to the finding (`remediation_fixes`).

use std::future::Future;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use axum::extract::{Path, State};
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use utoipa::ToSchema;

use super::provenance::{self, Provenance, ProvenanceKind, TagTarget, TaggedResource};
use super::remediation_infra::{self as infra, InfraFinding, InfraLevel, InfraTools, Located, MissingTag, ProvenanceSnippet, Subject};
use super::remediation_manifests::{self as manifests, BumpError};
use super::remediation_model::{self as model, Declared, Facts, FixTarget, Kind, Level};
use crate::assist::agent::{self, Loop, Model, Outcome, Tools};
use crate::assist::code::{self, Forge, ForgeKind};
use crate::assist::llm::{Msg, ToolSpec, Usage};
use crate::assist::runs::{ChangedFile, CodeChange, Conclusion, Kind as RunKind, RunTools};
use crate::auth::{AuthContext, Writer};
use crate::error::{ApiError, ApiResult};
use crate::routes::assist_runs::{self, insert_run, within_limits, Done, RunStep, RunSummary};
use crate::routes::portal_sources::{send, HttpCall, PortalProvider};
use crate::state::AppState;

/// Where Tagging Policies are set (Organization Settings).
pub const TAGGING_POLICIES: &str = "/infrastructure/tagging/policies";

/// A finding: `library` (a library vulnerability's key), `code` (a static finding's id: SAST, secret, IaC),
/// `cloud_misconfiguration` or `cloud_vulnerability` (their triage keys), `resource` (a resource of Tag Coverage).
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct FindingRef {
    pub kind: String,
    pub id: String,
    /// Fix: the repository to change, when the finding is in several (default: the first).
    #[serde(default)]
    pub repository: Option<String>,
    /// Fix in infrastructure: the person confirmed an inferred source.
    #[serde(default)]
    pub confirmed: bool,
}

/// What a cloud finding says about its resource.
#[derive(Clone, Debug)]
struct InfraFacts {
    subject: Subject,
    resource_id: String,
    resource_name: String,
    resource_type: String,
    env: Option<String>,
    rule_id: String,
}

/// A finding as remediation sees it.
struct Finding {
    facts: Facts,
    title: String,
    severity: String,
    /// Rule id, or the advisory and its aliases.
    ids: Vec<String>,
    /// `file:line` or the resource.
    location: String,
    line: i32,
    snippet: String,
    package: String,
    version: String,
    /// The finding as JSON, for Assist.
    detail: Value,
    /// Its page in the app.
    link: String,
    /// Cloud findings: their resource.
    infra: Option<InfraFacts>,
}

async fn load(st: &AppState, org: &str, r: &FindingRef) -> Result<Finding, ApiError> {
    let missing = || ApiError::NotFound("No such finding".into());
    let page = |p: &str| format!("{}{p}", st.config.public_url);
    match r.kind.as_str() {
        "library" => {
            let v = super::code_security::library_vulnerabilities(st, org).await?.into_iter().find(|v| v.key == r.id).ok_or_else(missing)?;
            let declared = v.occurrences.iter().map(|o| Declared { repository: if o.source == "runtime" { String::new() } else { o.repository.clone() }, file: o.file.clone() }).collect();
            let mut ids = vec![v.advisory_id.clone()];
            ids.extend(v.aliases.iter().filter(|a| a.starts_with("CVE-")).cloned());
            Ok(Finding {
                facts: Facts { kind: Kind::Library, repository: String::new(), file: String::new(), fixed: v.fixed.clone(), declared },
                title: v.title.clone(),
                severity: v.severity.clone(),
                ids,
                location: format!("{} {}", v.package, v.version),
                line: 0,
                snippet: String::new(),
                package: v.package.clone(),
                version: v.version.clone(),
                infra: None,
                detail: json!({"advisory": v.advisory_id, "aliases": v.aliases, "title": v.title, "severity": v.severity, "score": v.score, "cwes": v.cwes, "ecosystem": v.ecosystem,
                    "package": v.package, "version": v.version, "fixed": v.fixed, "services": v.services, "repositories": v.repositories, "inProduction": v.in_production}),
                link: page("/code-security/vulnerabilities/libraries"),
            })
        }
        "code" => {
            let f = super::code_security::static_findings(st, org, None, false).await?.into_iter().find(|f| f.id == r.id).ok_or_else(missing)?;
            let kind = Kind::of_static(&f.kind);
            let tab = match kind {
                Kind::Secret => "secrets",
                Kind::Iac => "iac",
                _ => "static-code",
            };
            Ok(Finding {
                facts: Facts { kind, repository: f.repository.clone(), file: f.file.clone(), fixed: None, declared: vec![] },
                title: f.rule_name.clone(),
                severity: f.severity.clone(),
                ids: std::iter::once(f.rule_id.clone()).chain(f.cwes.iter().cloned()).collect(),
                location: format!("{}:{}", f.file, f.line),
                line: f.line,
                snippet: f.snippet.clone(),
                package: String::new(),
                version: String::new(),
                infra: None,
                detail: json!({"kind": f.kind, "tool": f.tool, "rule": f.rule_id, "ruleName": f.rule_name, "description": f.description, "cwes": f.cwes, "severity": f.severity,
                    "message": f.message, "repository": f.repository, "branch": f.branch, "file": f.file, "line": f.line, "helpUri": f.help_uri}),
                link: page(&format!("/code-security/vulnerabilities/{tab}")),
            })
        }
        "cloud_misconfiguration" => {
            let f = super::cloud_security::misconfigurations_of(st, org).await?.0.into_iter().find(|f| f.key == r.id).ok_or_else(missing)?;
            let facts = InfraFacts { subject: Subject::of_type(&f.resource_type), resource_id: f.resource_id.clone(), resource_name: f.resource_name.clone(), resource_type: f.resource_type.clone(), env: f.env.clone(), rule_id: f.rule_id.clone() };
            let mut c = cloud(Kind::CloudMisconfiguration, f.rule_name.clone(), f.severity.clone(), vec![f.rule_id.clone()], f.resource_name.clone(), serde_json::to_value(&f).unwrap_or_default(), page("/security/cloud/findings"));
            c.infra = Some(facts);
            Ok(c)
        }
        "cloud_vulnerability" => {
            let v = super::cloud_security::infra_vulnerabilities(st, org).await?.into_iter().find(|v| v.key == r.id).ok_or_else(missing)?;
            let ids = std::iter::once(v.advisory_id.clone()).chain(v.cve.clone()).collect();
            let facts = InfraFacts { subject: Subject::of_type(&v.resource_kind), resource_id: v.resource_id.clone(), resource_name: v.resource_name.clone(), resource_type: v.resource_kind.clone(), env: Some(v.env.clone()).filter(|e| !e.is_empty()), rule_id: v.advisory_id.clone() };
            let mut c = cloud(Kind::CloudVulnerability, v.title.clone(), v.severity.clone(), ids, v.resource_name.clone(), serde_json::to_value(&v).unwrap_or_default(), page("/security/cloud/vulnerabilities"));
            c.infra = Some(facts);
            Ok(c)
        }
        "resource" => {
            let res = provenance::inventory(st, org, &TagTarget::ALL).await?.into_iter().find(|x| x.id == r.id).ok_or_else(missing)?;
            let mut c = cloud(
                Kind::MissingTags,
                format!("{} lacks the tags that name its source", res.name),
                "low".into(),
                vec![super::tagging_policies::MISCONFIG_RULE.into()],
                res.name.clone(),
                serde_json::to_value(&res).unwrap_or_default(),
                page("/infrastructure/tagging"),
            );
            c.facts.kind = Kind::MissingTags;
            Ok(c)
        }
        _ => Err(ApiError::BadRequest("kind is library, code, cloud_misconfiguration, cloud_vulnerability or resource".into())),
    }
}

fn cloud(kind: Kind, title: String, severity: String, ids: Vec<String>, location: String, detail: Value, link: String) -> Finding {
    Finding { facts: Facts { kind, repository: String::new(), file: String::new(), fixed: None, declared: vec![] }, title, severity, ids, location, line: 0, snippet: String::new(), package: String::new(), version: String::new(), detail, link, infra: None }
}

/// A cloud finding's resource, its provenance and level, and what tags it lacks when it has none.
struct InfraResolution {
    level: InfraLevel,
    missing: Vec<MissingTag>,
    snippet: Option<ProvenanceSnippet>,
}

async fn resolve_infra(st: &AppState, org: &str, i: &InfraFacts) -> Result<InfraResolution, ApiError> {
    let resource: Option<TaggedResource> = match i.subject.target() {
        Some(t) => provenance::inventory(st, org, &[t]).await?.into_iter().find(|r| r.id == i.resource_id),
        None => None,
    };
    let (gh, gl) = connection_bases(st, org).await?;
    let level = infra::infra_level(i.subject, resource.as_ref().and_then(|r| r.provenance.as_ref()), |repo| model::connection_for(repo, gh.as_deref(), gl.as_deref()).map(|_| ()));
    let (missing, snippet) = match i.subject.target() {
        Some(t) if level.level == Level::NeedsProvenance => {
            let policies = super::tagging_policies::enabled_rules(st, org).await?;
            let missing = infra::missing_tags(t, resource.as_ref(), &policies);
            let snippet = infra::provenance_snippet(t, resource.as_ref(), &missing);
            (missing, Some(snippet))
        }
        _ => (vec![], None),
    };
    Ok(InfraResolution { level, missing, snippet })
}

/// What the run needs to find the resource in its source (Helm: the Argo CD Application's values files).
async fn infra_finding(st: &AppState, org: &str, i: &InfraFacts, p: &Provenance) -> InfraFinding {
    let mut value_files = Vec::new();
    if let (ProvenanceKind::Helm, Some(app)) = (p.kind, p.application.as_deref()) {
        if let Ok(apps) = provenance::argo_apps(st, org).await {
            if let Some(a) = apps.iter().find(|a| a.name == app) {
                let sources = a.doc.pointer("/spec/source").into_iter().chain(a.doc.pointer("/spec/sources").and_then(Value::as_array).into_iter().flatten());
                value_files = sources.filter_map(|s| s.pointer("/helm/valueFiles").and_then(Value::as_array)).flatten().filter_map(Value::as_str).map(str::to_string).collect();
            }
        }
    }
    InfraFinding { resource_name: i.resource_name.clone(), resource_type: i.resource_type.clone(), env: i.env.clone(), value_files }
}

/// How the merge request names the resource: the Kubernetes object, the cloud id, the image.
fn resource_label(i: &InfraFacts, p: &Provenance) -> String {
    match i.subject {
        Subject::Kubernetes => p.address.clone().unwrap_or_else(|| i.resource_name.clone()),
        Subject::Image => i.resource_name.clone(),
        _ => i.resource_id.clone(),
    }
}

// ---------------------------------------------------------------- connections

/// The org's GitHub and GitLab API base URLs.
async fn connection_bases(st: &AppState, org: &str) -> Result<(Option<String>, Option<String>), ApiError> {
    let gh = crate::routes::portal_home::stored_connector(st, org, PortalProvider::Github).await?.map(|c| c.base_url);
    let gl = crate::routes::portal_home::stored_connector(st, org, PortalProvider::Gitlab).await?.map(|c| c.base_url);
    Ok((gh, gl))
}

/// The connection reaching a repository and its path there.
async fn forge_for(st: &AppState, org: &str, repository: &str) -> Result<(Forge, String), ApiError> {
    let (gh, gl) = connection_bases(st, org).await?;
    let (kind, path) = model::connection_for(repository, gh.as_deref(), gl.as_deref()).map_err(ApiError::BadRequest)?;
    let provider = if kind == ForgeKind::Gitlab { PortalProvider::Gitlab } else { PortalProvider::Github };
    let c = crate::routes::portal_home::stored_connector(st, org, provider).await?.ok_or_else(|| ApiError::BadRequest("The connection was removed".into()))?;
    Ok((Forge { kind, base: c.base_url.trim_end_matches('/').to_string(), token: c.secret }, path))
}

/// Why the connection can't open a merge request (none: it can, or the forge didn't say).
async fn write_blocked(st: &AppState, f: &Forge, path: &str) -> Result<Option<String>, ApiError> {
    let allow = st.config.portal_allow_private;
    let repo = send(&f.repo_call(path), allow).await.map_err(|e| ApiError::BadRequest(format!("Can't read {path}: {e}")))?;
    let scopes = match f.kind {
        ForgeKind::Gitlab => send(&HttpCall::get(format!("{}/api/v4/personal_access_tokens/self", f.base), f.auth()), allow)
            .await
            .ok()
            .and_then(|t| t["scopes"].as_array().map(|a| a.iter().filter_map(|s| s.as_str().map(str::to_string)).collect::<Vec<_>>())),
        ForgeKind::Github => None,
    };
    Ok(model::missing_write(f.kind, &repo, scopes.as_deref()))
}

// ---------------------------------------------------------------- the remediation of a finding

#[derive(Serialize, Deserialize, ToSchema, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct MergeRequest {
    pub url: String,
    pub number: i64,
    pub branch: String,
    pub draft: bool,
    /// open | merged | closed
    pub state: String,
}

/// A fix of a finding: the change proposed (a manifest bump, or an Assist run's change) and its merge request.
#[derive(Serialize, ToSchema, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct FixView {
    pub id: String,
    pub finding_kind: String,
    pub finding_id: String,
    pub repository: String,
    /// manifest (a deterministic bump) | assist (an Assist run)
    pub method: String,
    /// ready | queued | running | concluded | failed | cancelled
    pub status: String,
    pub run_id: Option<String>,
    pub change: Option<CodeChange>,
    /// What must still happen (the lockfile, rotating a credential); also in the merge request.
    pub notes: Vec<String>,
    /// Why no change was proposed, and what to do instead.
    pub explanation: Option<String>,
    /// Why a merge request can't be opened with the connection (the scope it lacks): copy the diff instead.
    pub blocked: Option<String>,
    pub merge_request: Option<MergeRequest>,
    /// Fix in infrastructure and added tags: the resource and the source the change was made in.
    pub source: Option<FixSource>,
    pub created_by: String,
    pub created_at: i64,
}

/// The resource of an infrastructure fix and where it was changed.
#[derive(Serialize, Deserialize, ToSchema, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FixSource {
    /// The cloud id, Kubernetes object or image.
    pub resource: String,
    pub provenance: Option<Provenance>,
    /// The Terraform resource found (deterministic fixes).
    #[serde(default)]
    pub located: Option<Located>,
    /// An inferred source the person confirmed.
    #[serde(default)]
    pub confirmed: bool,
    /// Added tags: the directory they were added under.
    #[serde(default)]
    pub path: Option<String>,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct Remediation {
    pub level: Level,
    /// Why this level applies.
    pub reason: String,
    /// Fix in code: the repositories and files a fix changes.
    pub targets: Vec<FixTarget>,
    /// Fix in code: why no connection reaches the first target's repository (none: one does).
    pub connection: Option<String>,
    /// Needs provenance: where to set it up.
    pub tagging_policies: Option<String>,
    /// The latest explanation run of the finding.
    pub explain_run: Option<RunSummary>,
    /// The latest fix of the finding, with its merge request.
    pub fix: Option<FixView>,
    /// Cloud findings: the resource's source (also when no connection reaches it).
    pub source: Option<Provenance>,
    /// Fix in infrastructure from an inferred source: confirm it before fixing (`confirmed` on the fix request).
    pub confirm: bool,
    /// Needs provenance: the tags the resource lacks, per the tagging policy requiring them.
    pub missing_tags: Vec<MissingTag>,
    /// Needs provenance: how to add them where the resource is defined.
    pub snippet: Option<ProvenanceSnippet>,
}

#[derive(sqlx::FromRow)]
struct FixRow {
    id: String,
    finding_kind: String,
    finding_id: String,
    repository: String,
    method: String,
    run_id: Option<String>,
    change: Option<sqlx::types::Json<CodeChange>>,
    notes: Vec<String>,
    explanation: Option<String>,
    blocked: Option<String>,
    merge_request: Option<sqlx::types::Json<MergeRequest>>,
    source: Option<sqlx::types::Json<FixSource>>,
    created_by: String,
    created_at: i64,
}

/// A fix run's status, change, conclusion and error.
type RunOutcome = (String, Option<sqlx::types::Json<CodeChange>>, Option<sqlx::types::Json<Conclusion>>, Option<String>);

const FIX_ROW: &str = "SELECT id::text, finding_kind, finding_id, repository, method, run_id::text, change, notes, explanation, blocked, merge_request, source, created_by, \
    (extract(epoch FROM created_at) * 1000)::bigint AS created_at FROM remediation_fixes";

async fn fix_view(st: &AppState, org: &str, row: FixRow) -> Result<FixView, ApiError> {
    let (status, change) = match &row.run_id {
        Some(run) => {
            let r: Option<RunOutcome> =
                sqlx::query_as("SELECT status, change, conclusion, error FROM assist_runs WHERE org_id = $1 AND id = $2::uuid").bind(org).bind(run).fetch_optional(&st.postgres).await?;
            match r {
                Some((status, change, conclusion, error)) => {
                    let explanation = if change.is_none() { conclusion.map(|c| c.0.summary).or(error) } else { None };
                    return Ok(FixView { status, change: change.map(|c| c.0), explanation: explanation.or(row.explanation.clone()), ..base_view(row) });
                }
                None => ("failed".to_string(), None),
            }
        }
        None => ("ready".to_string(), row.change.clone().map(|c| c.0)),
    };
    Ok(FixView { status, change, ..base_view(row) })
}

fn base_view(row: FixRow) -> FixView {
    FixView {
        id: row.id,
        finding_kind: row.finding_kind,
        finding_id: row.finding_id,
        repository: row.repository,
        method: row.method,
        status: String::new(),
        run_id: row.run_id,
        change: None,
        notes: row.notes,
        explanation: row.explanation,
        blocked: row.blocked,
        merge_request: row.merge_request.map(|m| m.0),
        source: row.source.map(|x| x.0),
        created_by: row.created_by,
        created_at: row.created_at,
    }
}

async fn load_fix(st: &AppState, org: &str, id: &str) -> Result<FixRow, ApiError> {
    let uuid = id.len() == 36 && id.chars().enumerate().all(|(i, c)| if [8, 13, 18, 23].contains(&i) { c == '-' } else { c.is_ascii_hexdigit() });
    if !uuid {
        return Err(ApiError::NotFound("No such fix".into()));
    }
    sqlx::query_as::<_, FixRow>(sqlx::AssertSqlSafe(format!("{FIX_ROW} WHERE org_id = $1 AND id = $2::uuid")))
        .bind(org)
        .bind(id)
        .fetch_optional(&st.postgres)
        .await?
        .ok_or_else(|| ApiError::NotFound("No such fix".into()))
}

/// Refreshes an open merge request's state from the forge (errors leave it as it was).
async fn refresh_state(st: &AppState, org: &str, row: &mut FixRow) {
    let Some(mr) = row.merge_request.as_mut() else { return };
    if mr.0.state != "open" {
        return;
    }
    let Ok((f, path)) = forge_for(st, org, &row.repository).await else { return };
    if let Ok(state) = code::pull_request_state(st, &f, &path, mr.0.number).await {
        if state != mr.0.state {
            mr.0.state = state.clone();
            let _ = sqlx::query("UPDATE remediation_fixes SET merge_request = jsonb_set(merge_request, '{state}', to_jsonb($3::text)) WHERE org_id = $1 AND id = $2::uuid")
                .bind(org)
                .bind(&row.id)
                .bind(state)
                .execute(&st.postgres)
                .await;
        }
    }
}

/// What can be done about a finding: its level and why, its latest explanation and fix (with the merge request's state).
#[utoipa::path(post, path = "/api/v1/remediation", operation_id = "remediation_of_finding", request_body = FindingRef, responses((status = 200, body = Remediation)), tag = "remediation")]
pub async fn remediation(State(st): State<AppState>, ctx: AuthContext, Json(r): Json<FindingRef>) -> ApiResult<Remediation> {
    let f = load(&st, &ctx.org_id, &r).await?;
    let (level, reason, targets, connection, source, confirm, missing_tags, snippet) = match &f.infra {
        Some(i) => {
            let x = resolve_infra(&st, &ctx.org_id, i).await?;
            (x.level.level, x.level.reason, vec![], x.level.connection, x.level.source, x.level.confirm, x.missing, x.snippet)
        }
        None => {
            let (level, reason, targets) = model::level(&f.facts);
            let connection = match targets.first() {
                Some(t) => {
                    let (gh, gl) = connection_bases(&st, &ctx.org_id).await?;
                    model::connection_for(&t.repository, gh.as_deref(), gl.as_deref()).err()
                }
                None => None,
            };
            (level, reason, targets, connection, None, false, vec![], None)
        }
    };
    let explain: Option<String> = sqlx::query_scalar(
        "SELECT id::text FROM assist_runs WHERE org_id = $1 AND kind = 'explain' AND subject->'finding'->>'kind' = $2 AND subject->'finding'->>'id' = $3 ORDER BY created_at DESC LIMIT 1",
    )
    .bind(&ctx.org_id)
    .bind(&r.kind)
    .bind(&r.id)
    .fetch_optional(&st.postgres)
    .await?;
    let explain_run = match explain {
        Some(id) => Some(assist_runs::get_run(State(st.clone()), ctx.clone(), Path(id)).await?.0.summary),
        None => None,
    };
    let row = sqlx::query_as::<_, FixRow>(sqlx::AssertSqlSafe(format!("{FIX_ROW} WHERE org_id = $1 AND finding_kind = $2 AND finding_id = $3 ORDER BY created_at DESC LIMIT 1")))
        .bind(&ctx.org_id)
        .bind(&r.kind)
        .bind(&r.id)
        .fetch_optional(&st.postgres)
        .await?;
    let fix = match row {
        Some(mut row) => {
            refresh_state(&st, &ctx.org_id, &mut row).await;
            Some(fix_view(&st, &ctx.org_id, row).await?)
        }
        None => None,
    };
    let tagging_policies = (level == Level::NeedsProvenance).then(|| TAGGING_POLICIES.to_string());
    Ok(Json(Remediation { level, reason, targets, connection, tagging_policies, explain_run, fix, source, confirm, missing_tags, snippet }))
}

/// Starts an Assist run explaining the finding, with remediation steps (polled as other runs).
#[utoipa::path(post, path = "/api/v1/remediation/explain", operation_id = "remediation_explain", request_body = FindingRef, responses((status = 200, body = RunSummary)), tag = "remediation")]
pub async fn explain(State(st): State<AppState>, ctx: AuthContext, Json(r): Json<FindingRef>) -> ApiResult<RunSummary> {
    let f = load(&st, &ctx.org_id, &r).await?;
    within_limits(&st, &ctx.org_id).await?;
    let title: String = format!("Explain: {}", f.title).chars().take(140).collect();
    Ok(Json(insert_run(&st, &ctx.org_id, RunKind::Explain, &title, "manual", &json!({"finding": {"kind": r.kind, "id": r.id}}), &ctx.user_id, &ctx.name).await?))
}

/// A fix as stored, before its run (if any) is attached.
#[allow(clippy::too_many_arguments)]
async fn insert_fix(st: &AppState, ctx: &AuthContext, kind: &str, finding: &str, repository: &str, method: &str, change: Option<CodeChange>, notes: Vec<String>, explanation: Option<String>, blocked: Option<String>, source: Option<FixSource>) -> Result<String, ApiError> {
    Ok(sqlx::query_scalar(
        "INSERT INTO remediation_fixes (org_id, finding_kind, finding_id, repository, method, change, notes, explanation, blocked, source, created_by) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11) RETURNING id::text",
    )
    .bind(&ctx.org_id)
    .bind(kind)
    .bind(finding)
    .bind(repository)
    .bind(method)
    .bind(change.as_ref().map(sqlx::types::Json))
    .bind(&notes)
    .bind(&explanation)
    .bind(&blocked)
    .bind(source.as_ref().map(sqlx::types::Json))
    .bind(&ctx.name)
    .fetch_one(&st.postgres)
    .await?)
}

/// Starts the Assist run of a fix and links it.
async fn start_fix_run(st: &AppState, ctx: &AuthContext, fix: &str, kind: RunKind, title: &str, subject: &Value) -> Result<(), ApiError> {
    within_limits(st, &ctx.org_id).await?;
    let title: String = title.chars().take(140).collect();
    let run = insert_run(st, &ctx.org_id, kind, &title, "manual", subject, &ctx.user_id, &ctx.name).await?;
    sqlx::query("UPDATE remediation_fixes SET run_id = $3::uuid WHERE org_id = $1 AND id = $2::uuid").bind(&ctx.org_id).bind(fix).bind(&run.id).execute(&st.postgres).await?;
    Ok(())
}

/// Fixes the finding in code: a library's manifest is bumped right away; a static finding gets an Assist run
/// limited to its file; a cloud finding is fixed where its provenance says the resource is defined. Either way the
/// result is a proposal to review before opening the merge request.
#[utoipa::path(post, path = "/api/v1/remediation/fix", operation_id = "remediation_fix", request_body = FindingRef, responses((status = 200, body = FixView)), tag = "remediation")]
pub async fn fix(State(st): State<AppState>, Writer(ctx): Writer, Json(r): Json<FindingRef>) -> ApiResult<FixView> {
    let f = load(&st, &ctx.org_id, &r).await?;
    if let Some(i) = f.infra.clone() {
        return fix_infra(st, ctx, r, f, i).await;
    }
    let (level, reason, targets) = model::level(&f.facts);
    if level != Level::FixInCode {
        return Err(ApiError::BadRequest(reason));
    }
    let target = match &r.repository {
        Some(repo) => targets.into_iter().find(|t| &t.repository == repo).ok_or_else(|| ApiError::BadRequest(format!("The finding isn't in {repo}")))?,
        None => targets.into_iter().next().ok_or_else(|| ApiError::BadRequest(reason.clone()))?,
    };
    let (forge, path) = forge_for(&st, &ctx.org_id, &target.repository).await?;
    let blocked = write_blocked(&st, &forge, &path).await?;
    let insert = |method: &'static str, change: Option<CodeChange>, notes: Vec<String>, explanation: Option<String>| insert_fix(&st, &ctx, &r.kind, &r.id, &target.repository, method, change, notes, explanation, blocked.clone(), None);
    let id = if f.facts.kind == Kind::Library {
        let fixed = f.facts.fixed.clone().unwrap_or_default();
        let found = bump_manifest(&target.files, &f.package, &fixed, |p| {
            let p = p.to_string();
            let (st, forge, path) = (&st, &forge, &path);
            async move { read_or_list(st, forge, path, &p).await }
        })
        .await;
        match found {
            Ok((file, before, after)) => {
                let mut notes: Vec<String> = manifests::lockfile_note(&file).map(|n| n.replace("<package>", &f.package).replace("<gem>", &f.package).replace("<crate>", &f.package)).into_iter().collect();
                notes.extend(model::major_note(&f.package, &f.version, &fixed));
                let change = CodeChange {
                    title: format!("Bump {} to {fixed}", f.package),
                    description: format!("Bumps `{}` from {} to {fixed} in `{file}`, the first version that fixes {}.", f.package, f.version, f.ids.join(" / ")),
                    files: vec![ChangedFile { path: file, before: Some(before), after }],
                };
                insert("manifest", Some(change), notes, None).await?
            }
            Err(why) => insert("manifest", None, vec![], Some(why)).await?,
        }
    } else {
        within_limits(&st, &ctx.org_id).await?;
        let file = target.files.first().cloned().unwrap_or_default();
        let id = insert("assist", None, if f.facts.kind == Kind::Secret { vec![model::SECRET_NOTE.into()] } else { vec![] }, None).await?;
        let subject = json!({"finding": {"kind": r.kind, "id": r.id}, "fixId": id, "repository": target.repository, "file": file, "line": f.line});
        start_fix_run(&st, &ctx, &id, RunKind::Fix, &format!("Fix {} in {file}", f.title), &subject).await?;
        id
    };
    ctx.audit(&st, "remediation.fix", &format!("{}:{}", r.kind, r.id), &target.repository).await;
    Ok(Json(fix_view(&st, &ctx.org_id, load_fix(&st, &ctx.org_id, &id).await?).await?))
}

/// Fixes a cloud finding where its provenance says the resource is defined: an S3 bucket's public access is blocked
/// without an LLM when its Terraform resource is found; otherwise an Assist run (`fix_infra`) limited to the source.
async fn fix_infra(st: AppState, ctx: AuthContext, r: FindingRef, f: Finding, i: InfraFacts) -> ApiResult<FixView> {
    let res = resolve_infra(&st, &ctx.org_id, &i).await?;
    if res.level.level != Level::FixInInfra {
        return Err(ApiError::BadRequest(res.level.reason));
    }
    let p = res.level.source.clone().ok_or_else(|| ApiError::BadRequest("The resource has no source".into()))?;
    if res.level.confirm && !r.confirmed {
        return Err(ApiError::BadRequest("The source was inferred: confirm it before Assist edits it.".into()));
    }
    let repository = p.repo.clone().unwrap_or_default();
    let (forge, path) = forge_for(&st, &ctx.org_id, &repository).await?;
    let blocked = write_blocked(&st, &forge, &path).await?;
    let reference = code::base_branch(&st, &forge, &path).await.map_err(ApiError::BadRequest)?;
    let files = ForgeFiles { st: st.clone(), forge, repo: path, reference };
    let finding = infra_finding(&st, &ctx.org_id, &i, &p).await;
    let mut source = FixSource { resource: resource_label(&i, &p), provenance: Some(p.clone()), located: None, confirmed: r.confirmed && res.level.confirm, path: None };
    let mut notes = vec![infra::ci_note(p.kind)];
    if source.confirmed {
        notes.push(format!("The source was inferred (via {}) and confirmed by {} before the fix.", p.via.as_str(), ctx.name));
    }
    // Deterministic: block public access on an S3 bucket found in its Terraform.
    let mut change = None;
    if p.kind == ProvenanceKind::Terraform && infra::S3_PUBLIC_ACCESS_CHECKS.contains(&i.rule_id.as_str()) {
        let prep = infra::prepare(&files, &p, &finding).await;
        if let Some(l) = prep.located.as_ref().filter(|l| l.address.starts_with("aws_s3_bucket.")) {
            if let Ok((file, before, after)) = infra::block_public_access(&prep.files, l) {
                change = Some(CodeChange {
                    title: format!("Block public access to the {} bucket", i.resource_name),
                    description: format!("Sets `block_public_acls`, `block_public_policy`, `ignore_public_acls` and `restrict_public_buckets` to true for `{}`, defined at `{}` in `{}`.", i.resource_name, l.address, l.file),
                    files: vec![ChangedFile { path: file, before: Some(before), after }],
                });
                notes.push("If something relies on public access to this bucket (a static website, public downloads), serve it through CloudFront with origin access control before merging.".into());
                source.located = Some(l.clone());
            }
        }
    }
    let id = match change {
        Some(c) => insert_fix(&st, &ctx, &r.kind, &r.id, &repository, "editor", Some(c), notes, None, blocked, Some(source)).await?,
        None => {
            let id = insert_fix(&st, &ctx, &r.kind, &r.id, &repository, "assist", None, notes, None, blocked, Some(source)).await?;
            let subject = json!({"finding": {"kind": r.kind, "id": r.id}, "fixId": id, "repository": repository, "source": p, "confirmed": r.confirmed});
            start_fix_run(&st, &ctx, &id, RunKind::FixInfra, &format!("Fix {} on {}", f.title, i.resource_name), &subject).await?;
            id
        }
    };
    ctx.audit(&st, "remediation.fix", &format!("{}:{}", r.kind, r.id), &repository).await;
    Ok(Json(fix_view(&st, &ctx.org_id, load_fix(&st, &ctx.org_id, &id).await?).await?))
}

/// Tag Coverage → Add tags with Assist: a resource, and the repository (and directory) that defines it.
#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AddTagsInput {
    /// The resource's id in Tag Coverage.
    pub resource_id: String,
    /// `github.com/example-org/infra-terraform`
    pub repository: String,
    /// The Terraform root module, chart or image build directory; the repository root when empty.
    #[serde(default)]
    pub path: String,
}

/// The tags `add_tags` writes for a resource defined in `repository` under `path`.
pub fn tags_to_add(target: TagTarget, repository: &str, path: &str) -> Vec<(String, String)> {
    match target {
        TagTarget::Cloud => vec![("managed-by".into(), "terraform".into()), ("tf_repo".into(), repository.into()), ("tf_path".into(), path.into())],
        _ => vec![],
    }
}

/// Adds the tags that name a resource's source to the repository that defines it: the AWS provider's `default_tags`
/// or the Dockerfile's OCI labels without an LLM when they can be written, else an Assist run (`add_tags`) limited to
/// the directory. Reviewed and opened as a merge request like other fixes.
#[utoipa::path(post, path = "/api/v1/remediation/add-tags", operation_id = "remediation_add_tags", request_body = AddTagsInput, responses((status = 200, body = FixView)), tag = "remediation")]
pub async fn add_tags(State(st): State<AppState>, Writer(ctx): Writer, Json(input): Json<AddTagsInput>) -> ApiResult<FixView> {
    let dir = input.path.trim().trim_start_matches("./").trim_matches('/').to_string();
    if !dir.is_empty() {
        code::check_path(&dir).map_err(ApiError::BadRequest)?;
    }
    let repository = super::code_security_model::normalize_repo(&input.repository);
    let resource = provenance::inventory(&st, &ctx.org_id, &TagTarget::ALL).await?.into_iter().find(|x| x.id == input.resource_id).ok_or_else(|| ApiError::NotFound("No such resource".into()))?;
    if resource.target == TagTarget::Kubernetes {
        return Err(ApiError::BadRequest(
            "Argo CD writes its tracking annotation itself: set application.resourceTrackingMethod: annotation in argocd-cm and collect the Applications with the Cluster Agent (Tagging → Policies has the snippet).".into(),
        ));
    }
    let policies = super::tagging_policies::enabled_rules(&st, &ctx.org_id).await?;
    let missing = infra::missing_tags(resource.target, Some(&resource), &policies);
    let (forge, path) = forge_for(&st, &ctx.org_id, &repository).await?;
    let blocked = write_blocked(&st, &forge, &path).await?;
    let reference = code::base_branch(&st, &forge, &path).await.map_err(ApiError::BadRequest)?;
    let files = ForgeFiles { st: st.clone(), forge, repo: path, reference };
    let listing = files.list(&dir).await.unwrap_or_default();
    let (change, notes) = deterministic_tags(&files, &listing, &resource, &repository, &dir).await;
    let source = FixSource { resource: resource.name.clone(), provenance: resource.provenance.clone(), located: None, confirmed: false, path: Some(dir.clone()) };
    let r = FindingRef { kind: "resource".into(), id: resource.id.clone(), repository: None, confirmed: false };
    let id = match change {
        Some(c) => insert_fix(&st, &ctx, &r.kind, &r.id, &repository, "editor", Some(c), notes, None, blocked, Some(source)).await?,
        None => {
            let id = insert_fix(&st, &ctx, &r.kind, &r.id, &repository, "assist", None, notes, None, blocked, Some(source)).await?;
            let subject = json!({"finding": {"kind": r.kind, "id": r.id}, "fixId": id, "repository": repository, "path": dir, "resource": resource, "missing": missing, "tags": tags_to_add(resource.target, &repository, &dir)});
            start_fix_run(&st, &ctx, &id, RunKind::AddTags, &format!("Add provenance tags for {} to {repository}", resource.name), &subject).await?;
            id
        }
    };
    ctx.audit(&st, "remediation.add_tags", &resource.id, &repository).await;
    Ok(Json(fix_view(&st, &ctx.org_id, load_fix(&st, &ctx.org_id, &id).await?).await?))
}

/// The tags written without an LLM: the AWS provider's `default_tags` in the directory's `.tf` files, or the OCI labels
/// in the image's Dockerfile. None: an Assist run writes them.
async fn deterministic_tags<R: RepoFiles>(files: &R, listing: &[String], resource: &TaggedResource, repository: &str, dir: &str) -> (Option<CodeChange>, Vec<String>) {
    let mut changed = Vec::new();
    match resource.target {
        TagTarget::Cloud if resource.provider.as_deref().is_none_or(|p| p == "aws") => {
            let tags = tags_to_add(TagTarget::Cloud, repository, dir);
            for path in listing.iter().filter(|p| p.ends_with(".tf") && p.rsplit_once('/').map_or("", |(d, _)| d) == dir).take(infra::MAX_INFRA_READS / 2) {
                let Ok(before) = files.read(path).await else { continue };
                if let Ok(after) = infra::add_default_tags(&before, &tags) {
                    changed.push(ChangedFile { path: path.clone(), before: Some(before), after });
                }
            }
            if changed.is_empty() {
                return (None, vec![]);
            }
            let change = CodeChange {
                title: format!("Tag {dir} with its source", dir = if dir.is_empty() { "the root module" } else { dir }),
                description: format!("Adds `managed-by`, `tf_repo` and `tf_path` to the AWS provider's `default_tags`, so every resource this module creates names `{repository}` and `{dir}` (ADR-0008 tagging policies)."),
                files: changed,
            };
            (Some(change), vec!["`terraform plan` shows tag-only changes on every resource of this root module; apply it so the resources carry their source.".into(), infra::ci_note(ProvenanceKind::Terraform)])
        }
        TagTarget::Image => {
            let Some(path) = infra::dockerfiles(listing, dir, &resource.name).into_iter().next() else { return (None, vec![]) };
            let Ok(before) = files.read(&path).await else { return (None, vec![]) };
            let Ok(after) = infra::add_oci_labels(&before, &format!("https://{repository}")) else { return (None, vec![]) };
            let change = CodeChange {
                title: format!("Label {} with its source", resource.name.rsplit('/').next().unwrap_or(&resource.name)),
                description: format!("Adds the OCI `source` and `revision` labels to `{path}`, so the image names `{repository}` and the commit it was built from."),
                files: vec![ChangedFile { path, before: Some(before), after }],
            };
            (Some(change), vec!["Build with `--build-arg GIT_COMMIT=$(git rev-parse HEAD)` so the revision label carries the commit.".into(), infra::ci_note(ProvenanceKind::Dockerfile)])
        }
        _ => (None, vec![]),
    }
}

/// A file of the repository; `dir/*.csproj` reads the directory's first project file (as `path\ncontent`).
async fn read_or_list(st: &AppState, f: &Forge, repo: &str, path: &str) -> Result<(String, String), String> {
    if let Some(dir) = path.strip_suffix("*.csproj") {
        let files = code::list_files(st, f, repo, dir.trim_end_matches('/'), None).await?;
        let p = files.into_iter().find(|p| p.rsplit_once('/').map_or(p.as_str(), |(d, _)| d) == dir.trim_end_matches('/') && manifests::format_of(p).is_some()).ok_or("no project file")?;
        let content = code::read_file(st, f, repo, &p, None).await?;
        return Ok((p, content));
    }
    Ok((path.to_string(), code::read_file(st, f, repo, path, None).await?))
}

/// The first candidate manifest that declares the package, bumped: `(path, before, after)`; else what to do instead.
pub(crate) async fn bump_manifest<F, Fut>(candidates: &[String], package: &str, to: &str, read: F) -> Result<(String, String, String), String>
where
    F: Fn(&str) -> Fut,
    Fut: Future<Output = Result<(String, String), String>>,
{
    let mut why: Vec<String> = Vec::new();
    for c in candidates {
        let Ok((path, before)) = read(c).await else {
            why.push(format!("{c} wasn't found on the default branch."));
            continue;
        };
        match manifests::bump(&path, &before, package, to) {
            Ok(after) => return Ok((path, before, after)),
            Err(BumpError::NotDeclared) => why.push(format!(
                "{path} doesn't declare {package}: it comes in through another dependency. Upgrade the dependency that requires it, or force {to} with the package manager's override mechanism (npm `overrides`, Yarn `resolutions`, Maven `dependencyManagement`, a Go `require`)."
            )),
            Err(BumpError::AlreadyAllowed) => why.push(format!("{path} already allows {package} {to}: only the lockfile pins the vulnerable version. Update it with the package manager (for example `npm update {package}`, `poetry update {package}`, `bundle update {package}`, `cargo update -p {package}`).")),
            Err(BumpError::Unsupported(w)) => why.push(format!("{path} can't be edited automatically: {w}. Upgrade {package} to {to} there by hand.")),
        }
    }
    Err(why.join(" "))
}

/// A fix with its merge request's state refreshed.
#[utoipa::path(get, path = "/api/v1/remediation/fixes/{id}", operation_id = "remediation_get_fix", params(("id" = String, Path)), responses((status = 200, body = FixView)), tag = "remediation")]
pub async fn get_fix(State(st): State<AppState>, ctx: AuthContext, Path(id): Path<String>) -> ApiResult<FixView> {
    let mut row = load_fix(&st, &ctx.org_id, &id).await?;
    refresh_state(&st, &ctx.org_id, &mut row).await;
    Ok(Json(fix_view(&st, &ctx.org_id, row).await?))
}

/// Opens the merge request (pull request) of a fix on a new branch, with a description naming the finding,
/// its rule or CVE, the change and that CI must pass; the fix keeps it, so the finding shows its state.
#[utoipa::path(post, path = "/api/v1/remediation/fixes/{id}/merge-request", operation_id = "remediation_open_merge_request", params(("id" = String, Path)), responses((status = 200, body = FixView)), tag = "remediation")]
pub async fn open_merge_request(State(st): State<AppState>, Writer(ctx): Writer, Path(id): Path<String>) -> ApiResult<FixView> {
    let row = load_fix(&st, &ctx.org_id, &id).await?;
    if row.merge_request.is_some() {
        return Err(ApiError::Conflict("A merge request was already opened for this fix".into()));
    }
    let r = FindingRef { kind: row.finding_kind.clone(), id: row.finding_id.clone(), repository: None, confirmed: false };
    let view = fix_view(&st, &ctx.org_id, load_fix(&st, &ctx.org_id, &id).await?).await?;
    let change = view.change.clone().ok_or_else(|| ApiError::BadRequest("This fix proposed no change".into()))?;
    let f = load(&st, &ctx.org_id, &r).await?;
    let (forge, path) = forge_for(&st, &ctx.org_id, &row.repository).await?;
    if let Some(why) = write_blocked(&st, &forge, &path).await? {
        return Err(ApiError::BadRequest(why));
    }
    let settings = crate::routes::assist_code::settings(&st, &ctx.org_id).await?;
    let branch = code::branch_name(&settings.branch_prefix, &change.title, &id);
    let location = match f.facts.kind {
        Kind::Library => change.files.first().map(|c| c.path.clone()).unwrap_or_default(),
        _ if row.source.is_some() => String::new(),
        _ => f.location.clone(),
    };
    let (source, ci) = match row.source.as_ref().map(|x| &x.0) {
        Some(FixSource { provenance: Some(p), located, resource, path: None, .. }) => (infra::source_note(resource, p, located.as_ref()), infra::ci_note(p.kind)),
        Some(x) => {
            let kind = if change.files.iter().any(|c| c.path.ends_with(".tf")) { ProvenanceKind::Terraform } else if change.files.iter().any(|c| c.path.to_ascii_lowercase().contains("dockerfile")) { ProvenanceKind::Dockerfile } else { ProvenanceKind::Code };
            (format!("- Resource: `{}`\n", x.resource), infra::ci_note(kind))
        }
        None => (String::new(), String::new()),
    };
    let body = model::merge_request_description(&model::Description {
        kind: f.facts.kind,
        title: &f.title,
        severity: &f.severity,
        ids: &f.ids,
        location: &location,
        change: if change.description.trim().is_empty() { &change.title } else { &change.description },
        notes: &row.notes.iter().filter(|n| **n != ci).cloned().collect::<Vec<_>>(),
        link: &f.link,
        source: &source,
        ci: &ci,
    });
    let files: Vec<code::FileChange> = change.files.iter().map(|x| code::FileChange { path: x.path.clone(), content: x.after.clone(), existed: x.before.is_some() }).collect();
    let pr = code::open_pull_request(&st, &forge, &path, &branch, &change.title, &body, &files, settings.draft).await.map_err(ApiError::BadRequest)?;
    let mr = MergeRequest { url: pr.url.clone(), number: pr.number, branch: pr.branch, draft: pr.draft, state: "open".into() };
    sqlx::query("UPDATE remediation_fixes SET merge_request = $3 WHERE org_id = $1 AND id = $2::uuid").bind(&ctx.org_id).bind(&id).bind(sqlx::types::Json(&mr)).execute(&st.postgres).await?;
    ctx.audit(&st, "remediation.merge_request", &format!("{}:{}", row.finding_kind, row.finding_id), &pr.url).await;
    Ok(Json(FixView { merge_request: Some(mr), ..view }))
}

// ---------------------------------------------------------------- the runs

/// What a fix run may read: files of the repository's default branch.
pub trait RepoFiles {
    fn read(&self, path: &str) -> impl Future<Output = Result<String, String>> + Send;
    fn list(&self, dir: &str) -> impl Future<Output = Result<Vec<String>, String>> + Send;
}

struct ForgeFiles {
    st: AppState,
    forge: Forge,
    repo: String,
    reference: String,
}

impl RepoFiles for ForgeFiles {
    async fn read(&self, path: &str) -> Result<String, String> {
        code::read_file(&self.st, &self.forge, &self.repo, path, Some(&self.reference)).await
    }
    async fn list(&self, dir: &str) -> Result<Vec<String>, String> {
        code::list_files(&self.st, &self.forge, &self.repo, dir, Some(&self.reference)).await
    }
}

/// Reads allowed to a fix run, at most.
const MAX_READS: usize = 8;

/// A fix run's tools: the finding's directory, listed and read (its file's neighbours and imports).
pub struct FixTools<R: RepoFiles> {
    pub files: R,
    /// The finding's file.
    pub file: String,
    reads: Mutex<usize>,
}

impl<R: RepoFiles> FixTools<R> {
    pub fn new(files: R, file: &str) -> Self {
        FixTools { files, file: file.to_string(), reads: Mutex::new(0) }
    }

    fn dir(&self) -> &str {
        self.file.rsplit_once('/').map_or("", |(d, _)| d)
    }

    /// A path the run may read: in the finding's directory or below it.
    fn nearby(&self, path: &str) -> bool {
        let dir = self.dir();
        code::check_path(path).is_ok() && (dir.is_empty() || path.starts_with(&format!("{dir}/")))
    }
}

impl<R: RepoFiles + Sync> Tools for FixTools<R> {
    fn specs(&self) -> Vec<ToolSpec> {
        let obj = |props: Value, req: &[&str]| json!({"type": "object", "properties": props, "required": req});
        vec![
            ToolSpec { name: "list_nearby_files".into(), description: format!("Files of the finding's directory ({}) and below.", if self.dir().is_empty() { "the repository root" } else { self.dir() }), schema: obj(json!({}), &[]) },
            ToolSpec {
                name: "read_repo_file".into(),
                description: "A file of the finding's directory or below (its neighbours and local imports), on the default branch.".into(),
                schema: obj(json!({"path": {"type": "string"}}), &["path"]),
            },
        ]
    }

    fn writes(&self, _: &str) -> bool {
        false
    }

    async fn run(&self, name: &str, input: &Value) -> Result<String, String> {
        match name {
            "list_nearby_files" => Ok(json!({"files": self.files.list(self.dir()).await?.into_iter().take(200).collect::<Vec<_>>()}).to_string()),
            "read_repo_file" => {
                let path = input["path"].as_str().unwrap_or_default().trim_start_matches("./");
                if !self.nearby(path) {
                    return Err(format!("Only files under {} can be read in this run", if self.dir().is_empty() { "the repository root" } else { self.dir() }));
                }
                {
                    let mut n = self.reads.lock().unwrap();
                    if *n >= MAX_READS {
                        return Err(format!("At most {MAX_READS} files can be read: propose the change now"));
                    }
                    *n += 1;
                }
                Ok(json!({"path": path, "content": self.files.read(path).await?}).to_string())
            }
            _ => Err(format!("Unknown tool {name}")),
        }
    }
}

/// No tools: an explanation needs only the finding.
pub struct NoTools;

impl Tools for NoTools {
    fn specs(&self) -> Vec<ToolSpec> {
        vec![]
    }
    fn writes(&self, _: &str) -> bool {
        false
    }
    async fn run(&self, name: &str, _: &Value) -> Result<String, String> {
        Err(format!("Unknown tool {name}"))
    }
}

/// Lines `line - around ..= line + around` of a file, numbered.
pub fn excerpt(content: &str, line: i32, around: i32) -> String {
    let from = (line - around).max(1);
    content.lines().enumerate().map(|(i, l)| (i as i32 + 1, l)).filter(|(n, _)| *n >= from && *n <= line + around).map(|(n, l)| format!("{n:>5}  {l}")).collect::<Vec<_>>().join("\n")
}

/// The opening message of an explanation run.
pub fn explain_opening(kind: Kind, detail: &Value, code: &str) -> String {
    let mut s = format!("Explain this {} and how to remediate it.\n\n{}", kind.label().to_lowercase(), agent::truncate(&serde_json::to_string_pretty(detail).unwrap_or_default(), 8000));
    if !code.trim().is_empty() {
        s.push_str(&format!("\n\nThe code at the finding:\n```\n{}\n```", agent::truncate(code, 6000)));
    }
    s
}

/// The opening message of a fix run: the finding and its whole file.
pub fn fix_opening(kind: Kind, detail: &Value, file: &str, line: i32, content: &str) -> String {
    let secret = if kind == Kind::Secret { "\nThis is a secret: remove it from the code (read it from configuration instead) and say in the description that it must be rotated." } else { "" };
    format!(
        "Fix this {} in {file} (line {line}). Change only {file}: propose its whole new content with propose_change, then conclude.{secret}\n\n{}\n\n{file}:\n```\n{}\n```",
        kind.label().to_lowercase(),
        agent::truncate(&serde_json::to_string_pretty(detail).unwrap_or_default(), 6000),
        agent::truncate(content, 60_000)
    )
}

/// Runs an explanation or a fix (assist runs of kind `explain` and `fix`).
#[allow(clippy::too_many_arguments)]
pub(crate) async fn run_job<M: Model + Sync>(st: &AppState, org: &str, id: &str, kind: RunKind, subject: &Value, model: &M, system: &str, steps: &Arc<Mutex<Vec<RunStep>>>, msgs: &mut Vec<Msg>, usage: &mut Usage) -> Result<Done, String> {
    let e = |e: ApiError| e.to_string();
    let r: FindingRef = serde_json::from_value(subject["finding"].clone()).map_err(|x| format!("Invalid finding: {x}"))?;
    let f = load(st, org, &r).await.map_err(e)?;
    if kind == RunKind::Explain {
        // The code around the finding when the repository is readable; else the scanner's snippet.
        let mut code = f.snippet.clone();
        if !f.facts.repository.is_empty() && !f.facts.file.is_empty() {
            if let Ok((forge, path)) = forge_for(st, org, &f.facts.repository).await {
                if let Ok(content) = code::read_file(st, &forge, &path, &f.facts.file, None).await {
                    code = excerpt(&content, f.line, 20);
                }
            }
        }
        msgs.push(Msg::User(explain_opening(f.facts.kind, &f.detail, &code)));
        let tools = RunTools::new(NoTools, RunKind::Explain, false);
        return drive(st, org, id, model, &tools, system, steps, msgs, usage, None).await;
    }
    let repository = subject["repository"].as_str().unwrap_or_default();
    if matches!(kind, RunKind::FixInfra | RunKind::AddTags) {
        let (forge, path) = forge_for(st, org, repository).await.map_err(e)?;
        let reference = code::base_branch(st, &forge, &path).await?;
        let files = ForgeFiles { st: st.clone(), forge, repo: path, reference };
        let (opening, prep) = if kind == RunKind::FixInfra {
            let p: Provenance = serde_json::from_value(subject["source"].clone()).map_err(|x| format!("Invalid source: {x}"))?;
            let i = f.infra.clone().ok_or("Not a cloud finding")?;
            let finding = infra_finding(st, org, &i, &p).await;
            let prep = infra::prepare(&files, &p, &finding).await;
            (infra::infra_opening(&f.facts.kind.label().to_lowercase(), &f.detail, &p, &prep, &resource_label(&i, &p)), prep)
        } else {
            let dir = subject["path"].as_str().unwrap_or_default().to_string();
            let prep = infra::Prepared { scopes: vec![dir.clone()], write_dirs: vec![dir.clone()], ..Default::default() };
            (add_tags_opening(subject, repository, &dir), prep)
        };
        msgs.push(Msg::User(opening));
        let mut tools = RunTools::new(InfraTools::new(files, prep.scopes.clone(), prep.reads), kind, false).only_under(prep.write_dirs.clone());
        if let Some(only) = &prep.write_files {
            tools = tools.only_paths(only.clone());
        }
        let mut done = drive(st, org, id, model, &tools, system, steps, msgs, usage, None).await?;
        fill_before(&tools.base().files, &prep.files, &mut done).await;
        return Ok(done);
    }
    let file = subject["file"].as_str().unwrap_or_default().to_string();
    let (forge, path) = forge_for(st, org, repository).await.map_err(e)?;
    let reference = code::base_branch(st, &forge, &path).await?;
    let content = code::read_file(st, &forge, &path, &file, Some(&reference)).await?;
    msgs.push(Msg::User(fix_opening(f.facts.kind, &f.detail, &file, f.line, &content)));
    let files = ForgeFiles { st: st.clone(), forge, repo: path, reference };
    let tools = RunTools::new(FixTools::new(files, &file), RunKind::Fix, false).only_paths(vec![file.clone()]);
    drive(st, org, id, model, &tools, system, steps, msgs, usage, Some((file, content))).await
}

/// The opening of an `add_tags` run: the resource, the tags it lacks and where to add them.
fn add_tags_opening(subject: &Value, repository: &str, dir: &str) -> String {
    let r = &subject["resource"];
    let missing: Vec<String> = subject["missing"].as_array().into_iter().flatten().filter_map(|m| m["key"].as_str().map(str::to_string)).collect();
    let values: Vec<String> = subject["tags"].as_array().into_iter().flatten().filter_map(|t| Some(format!("{} = \"{}\"", t[0].as_str()?, t[1].as_str()?))).collect();
    let where_ = if dir.is_empty() { "the repository root".to_string() } else { dir.to_string() };
    format!(
        "Add the tags that name where this resource is defined to {repository}, under {where_}.\n\nThe resource ({}, {}): {}\nIts tags: {}\nMissing (alternatives separated by |): {}\n{}\nList and read the files under {where_}, then propose the smallest change that adds them where every resource of the kind gets them (Terraform: the provider's default_tags or a merged locals map; images: OCI labels in the Dockerfile or the build; services: DD_GIT_* in the deployment). Only files under {where_} may change.",
        r["target"].as_str().unwrap_or_default(),
        r["resourceType"].as_str().unwrap_or_default(),
        r["name"].as_str().unwrap_or_default(),
        r["tags"].as_array().map(|t| t.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(", ")).unwrap_or_default(),
        if missing.is_empty() { "none per the policies".to_string() } else { missing.join(", ") },
        if values.is_empty() { String::new() } else { format!("Values to use: {}\n", values.join(", ")) },
    )
}

/// The changed files' content before the change: from the files read for the opening, else the default branch (none:
/// a new file).
async fn fill_before<R: RepoFiles>(files: &R, read: &[(String, String)], done: &mut Done) {
    let Some(change) = done.change.as_mut() else { return };
    for c in &mut change.files {
        c.before = match read.iter().find(|(p, _)| *p == c.path) {
            Some((_, content)) => Some(content.clone()),
            None => files.read(&c.path).await.ok(),
        };
    }
}

/// The tool loop of a run, cancelled with it; `before` is the content of the file a fix changes.
#[allow(clippy::too_many_arguments)]
async fn drive<M: Model + Sync, T: Tools + Sync>(
    st: &AppState,
    org: &str,
    id: &str,
    model: &M,
    tools: &RunTools<T>,
    system: &str,
    steps: &Arc<Mutex<Vec<RunStep>>>,
    msgs: &mut Vec<Msg>,
    usage: &mut Usage,
    before: Option<(String, String)>,
) -> Result<Done, String> {
    let l = Loop { model, tools, system, auto_approve: false, max_steps: agent::MAX_STEPS };
    let cancelled = AtomicBool::new(false);
    let outcome = {
        let sink = steps.clone();
        let mut emit = move |ev: agent::Event| assist_runs::apply(&mut sink.lock().unwrap(), &ev, chrono::Utc::now().timestamp_millis());
        let work = l.run(msgs, usage, &mut emit);
        let watch = assist_runs::watch(st, org, id, steps, &cancelled);
        tokio::select! {
            o = work => o,
            _ = watch => Outcome::Failed("cancelled".into()),
        }
    };
    if cancelled.load(Ordering::Relaxed) {
        return Ok(Done { status: "cancelled", conclusion: None, error: None, change: None });
    }
    Ok(finish(tools, outcome, msgs, before))
}

/// How a run ended: its conclusion (or its last words) and the change, with the file's old content.
pub(crate) fn finish<T: Tools>(tools: &RunTools<T>, outcome: Outcome, msgs: &[Msg], before: Option<(String, String)>) -> Done {
    let change = tools.change().map(|mut c| {
        for f in &mut c.files {
            f.before = before.as_ref().filter(|(p, _)| *p == f.path).map(|(_, b)| b.clone());
        }
        c
    });
    match (tools.conclusion(), outcome) {
        (Some(c), _) => Done { status: "concluded", conclusion: Some(c), error: None, change },
        (None, Outcome::Failed(why)) => Done { status: "failed", conclusion: None, error: Some(why), change },
        (None, _) => {
            let text = msgs.iter().rev().find_map(|m| if let Msg::Assistant { text, .. } = m { (!text.trim().is_empty()).then(|| text.clone()) } else { None }).unwrap_or_default();
            let verdict = if change.is_some() { "change_proposed" } else if tools.kind() == RunKind::Explain { "explained" } else { "no_change" };
            Done { status: "concluded", conclusion: Some(Conclusion { summary: text, verdict: verdict.into(), confidence: "low".into(), ..Default::default() }), error: None, change }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assist::http::{HttpModel, Reach};
    use crate::assist::llm::Provider;
    use crate::assist::testing::{fake_anthropic, sse_text, sse_tool};
    use std::collections::HashMap;

    struct MapFiles(HashMap<String, String>);

    impl RepoFiles for MapFiles {
        async fn read(&self, path: &str) -> Result<String, String> {
            self.0.get(path).cloned().ok_or_else(|| format!("{path} not found"))
        }
        async fn list(&self, dir: &str) -> Result<Vec<String>, String> {
            Ok(self.0.keys().filter(|k| dir.is_empty() || k.starts_with(&format!("{dir}/"))).cloned().collect())
        }
    }

    fn files() -> MapFiles {
        MapFiles(HashMap::from([
            ("app/billing/charges.py".to_string(), "import os\nfrom .db import query\n\nSTRIPE_KEY = \"sk_live_example\"\n\ndef charge(id):\n    return query(\"SELECT * FROM charges WHERE id = \" + id)\n".to_string()),
            ("app/billing/db.py".to_string(), "def query(sql, params=()):\n    ...\n".to_string()),
            ("app/settings.py".to_string(), "DEBUG = False\n".to_string()),
        ]))
    }

    #[tokio::test]
    async fn bumping_tries_each_candidate_and_says_what_to_do_otherwise() {
        let repo = HashMap::from([("web/package.json".to_string(), "{\"dependencies\": {\"lodash\": \"^4.17.20\"}}".to_string()), ("api/package.json".to_string(), "{\"dependencies\": {}}".to_string())]);
        let read = |p: &str| {
            let r = repo.get(p).cloned().map(|c| (p.to_string(), c)).ok_or_else(|| "404".to_string());
            async move { r }
        };
        let (path, before, after) = bump_manifest(&["web/package.json".into()], "lodash", "4.17.21", read).await.unwrap();
        assert_eq!((path.as_str(), before.contains("4.17.20"), after.contains("^4.17.21")), ("web/package.json", true, true));
        let why = bump_manifest(&["missing/package.json".into(), "api/package.json".into()], "lodash", "4.17.21", read).await.unwrap_err();
        assert!(why.contains("missing/package.json wasn't found") && why.contains("comes in through another dependency") && why.contains("overrides"), "{why}");
        let pinned = HashMap::from([("package.json".to_string(), "{\"dependencies\": {\"lodash\": \"^4.17.21\"}}".to_string())]);
        let read = |p: &str| {
            let r = pinned.get(p).cloned().map(|c| (p.to_string(), c)).ok_or_else(|| "404".to_string());
            async move { r }
        };
        assert!(bump_manifest(&["package.json".into()], "lodash", "4.17.21", read).await.unwrap_err().contains("only the lockfile pins"));
    }

    #[test]
    fn openings_carry_the_finding_and_its_code() {
        let detail = json!({"rule": "python-security/sql-injection", "file": "app/billing/charges.py", "line": 7});
        let e = explain_opening(Kind::Sast, &detail, &excerpt("a\nb\nc\nd\ne\n", 3, 1));
        assert!(e.contains("Explain this code vulnerability") && e.contains("python-security/sql-injection") && e.contains("    2  b\n    3  c\n    4  d"), "{e}");
        assert!(!explain_opening(Kind::CloudMisconfiguration, &detail, "").contains("```"));
        let f = fix_opening(Kind::Secret, &detail, "app/billing/charges.py", 4, "STRIPE_KEY = \"x\"\n");
        // No message: the opening carries the secret's line, which a failing assert would print.
        assert!(f.contains("Change only app/billing/charges.py") && f.contains("must be rotated") && f.contains("STRIPE_KEY"));
    }

    #[tokio::test]
    async fn fix_tools_read_only_near_the_finding_and_only_so_often() {
        let t = FixTools::new(files(), "app/billing/charges.py");
        assert!(t.run("read_repo_file", &json!({"path": "app/billing/db.py"})).await.unwrap().contains("def query"));
        assert!(t.run("read_repo_file", &json!({"path": "app/settings.py"})).await.unwrap_err().contains("Only files under app/billing"));
        assert!(t.run("read_repo_file", &json!({"path": "app/billing/../settings.py"})).await.is_err());
        assert!(t.run("list_nearby_files", &json!({})).await.unwrap().contains("app/billing/db.py"));
        for _ in 1..MAX_READS {
            t.run("read_repo_file", &json!({"path": "app/billing/db.py"})).await.unwrap();
        }
        assert!(t.run("read_repo_file", &json!({"path": "app/billing/db.py"})).await.unwrap_err().contains("At most"));
    }

    /// A fix run through the fake model: it reads a neighbour, may not change another file, proposes the whole file
    /// (shown with its old content) and concludes.
    #[tokio::test]
    async fn a_fix_run_proposes_its_file_through_the_fake_model() {
        let fixed = "import os\nfrom .db import query\n\nSTRIPE_KEY = os.environ[\"STRIPE_KEY\"]\n\ndef charge(id):\n    return query(\"SELECT * FROM charges WHERE id = %s\", (id,))\n";
        let (base_url, seen) = fake_anthropic(vec![
            sse_tool("t1", "read_repo_file", json!({"path": "app/billing/db.py"})),
            sse_tool("t2", "propose_change", json!({"title": "Bind the charge id", "files": [{"path": "app/billing/db.py", "content": "x"}]})),
            sse_tool("t3", "propose_change", json!({"title": "Bind the charge id", "description": "Uses a bound parameter.", "files": [{"path": "app/billing/charges.py", "content": fixed}]})),
            sse_tool("t4", "conclude", json!({"summary": "The id was concatenated into SQL", "verdict": "change_proposed"})),
            sse_text("Done."),
        ])
        .await;
        let model = HttpModel { provider: Provider::Anthropic { base_url, api_key: "sk-test".into(), model: "claude-opus-5".into(), fallbacks: true }, max_tokens: 1000, reach: Reach::Trusted };
        let tools = RunTools::new(FixTools::new(files(), "app/billing/charges.py"), RunKind::Fix, false).only_paths(vec!["app/billing/charges.py".into()]);
        let before = files().0["app/billing/charges.py"].clone();
        let mut msgs = vec![Msg::User(fix_opening(Kind::Sast, &json!({"rule": "sql-injection"}), "app/billing/charges.py", 7, &before))];
        let mut usage = Usage::default();
        let outcome = Loop { model: &model, tools: &tools, system: "s", auto_approve: false, max_steps: agent::MAX_STEPS }.run(&mut msgs, &mut usage, &mut |_| {}).await;
        let done = finish(&tools, outcome, &msgs, Some(("app/billing/charges.py".into(), before.clone())));
        assert_eq!((done.status, done.conclusion.as_ref().map(|c| c.verdict.as_str())), ("concluded", Some("change_proposed")));
        let change = done.change.unwrap();
        assert_eq!((change.files.len(), change.files[0].after.as_str(), change.files[0].before.as_deref()), (1, fixed, Some(before.as_str())));
        // The model saw the wrong file refused.
        let bodies = seen.lock().unwrap().clone();
        assert!(bodies[2].to_string().contains("Only app/billing/charges.py may change"), "the refusal reached the model");
        // The tools offered: the neighbours, the proposal and the conclusion, nothing else.
        let names: Vec<String> = bodies[0]["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap().to_string()).collect();
        assert_eq!(names, vec!["list_nearby_files", "read_repo_file", "propose_change", "conclude"]);
    }

    /// An explanation through the fake model concludes with remediation steps and no tools but conclude.
    #[tokio::test]
    async fn an_explain_run_concludes_with_steps() {
        let (base_url, seen) = fake_anthropic(vec![
            sse_tool("t1", "conclude", json!({"summary": "Public S3 buckets expose their objects.", "verdict": "explained", "next_steps": ["Turn on Block Public Access", "Review the bucket policy"]})),
            sse_text("Done."),
        ])
        .await;
        let model = HttpModel { provider: Provider::Anthropic { base_url, api_key: "sk-test".into(), model: "claude-opus-5".into(), fallbacks: true }, max_tokens: 1000, reach: Reach::Trusted };
        let tools = RunTools::new(NoTools, RunKind::Explain, false);
        let mut msgs = vec![Msg::User(explain_opening(Kind::CloudMisconfiguration, &json!({"ruleId": "aws-s3-public"}), ""))];
        let outcome = Loop { model: &model, tools: &tools, system: "s", auto_approve: false, max_steps: agent::MAX_STEPS }.run(&mut msgs, &mut Usage::default(), &mut |_| {}).await;
        let done = finish(&tools, outcome, &msgs, None);
        let c = done.conclusion.unwrap();
        assert_eq!((c.verdict.as_str(), c.next_steps.len()), ("explained", 2));
        assert!(done.change.is_none());
        let names: Vec<String> = seen.lock().unwrap()[0]["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap().to_string()).collect();
        assert_eq!(names, vec!["conclude"]);
    }

    /// Fixes are stored, linked to the finding and show their merge request. Skipped unless OC_TEST_POSTGRES_URL and
    /// OC_TEST_CLICKHOUSE_URL are set.
    #[tokio::test]
    async fn a_fix_is_linked_to_its_finding_with_its_merge_request() {
        let Some((_, states)) = crate::test_support::replicas_with(1, |_| {}).await else { return };
        let st = states[0].clone();
        let ctx = crate::test_support::admin_of_new_org(&st, "remediation").await;
        let change = CodeChange { title: "Bump lodash to 4.17.21".into(), description: String::new(), files: vec![ChangedFile { path: "package.json".into(), before: Some("a".into()), after: "b".into() }] };
        let mr = MergeRequest { url: "https://gitlab.example.com/example-org/storefront/-/merge_requests/3".into(), number: 3, branch: "assist/bump-lodash".into(), draft: true, state: "open".into() };
        sqlx::query("INSERT INTO remediation_fixes (org_id, finding_kind, finding_id, repository, method, change, notes, merge_request, created_by) VALUES ($1, 'library', 'GHSA-x|npm|lodash|4.17.20', 'gitlab.example.com/example-org/storefront', 'manifest', $2, $3, $4, 'Test')")
            .bind(&ctx.org_id)
            .bind(sqlx::types::Json(&change))
            .bind(vec!["Regenerate the lockfile.".to_string()])
            .bind(sqlx::types::Json(&mr))
            .execute(&st.postgres)
            .await
            .unwrap();
        let row = sqlx::query_as::<_, FixRow>(sqlx::AssertSqlSafe(format!("{FIX_ROW} WHERE org_id = $1 AND finding_kind = 'library' AND finding_id = $2")))
            .bind(&ctx.org_id)
            .bind("GHSA-x|npm|lodash|4.17.20")
            .fetch_one(&st.postgres)
            .await
            .unwrap();
        let v = fix_view(&st, &ctx.org_id, row).await.unwrap();
        assert_eq!((v.status.as_str(), v.merge_request.unwrap().number, v.change.unwrap().files[0].after.as_str()), ("ready", 3, "b"));
        // Another org doesn't see it.
        let other = crate::test_support::admin_of_new_org(&st, "remediation-other").await;
        assert!(matches!(load_fix(&st, &other.org_id, &v.id).await, Err(ApiError::NotFound(_))));
    }

    fn tagged(target: TagTarget, provider: Option<&str>, name: &str) -> TaggedResource {
        TaggedResource {
            id: format!("id-{name}"),
            target,
            name: name.into(),
            resource_type: "AwsS3Bucket".into(),
            provider: provider.map(str::to_string),
            account: None,
            region: None,
            cluster: None,
            namespace: None,
            env: None,
            team: None,
            tags: vec![],
            provenance: None,
        }
    }

    #[tokio::test]
    async fn tags_are_added_to_the_aws_provider_or_the_dockerfile_without_an_llm() {
        let repo = MapFiles(HashMap::from([
            ("envs/prod/storage/providers.tf".to_string(), "provider \"aws\" {\n  region = \"eu-west-1\"\n}\n".to_string()),
            ("envs/prod/storage/main.tf".to_string(), "resource \"aws_s3_bucket\" \"logs\" {\n  bucket = \"example-shop-logs\"\n}\n".to_string()),
            ("services/catalog-service/Dockerfile".to_string(), "FROM debian:12-slim\nCOPY app /app\n".to_string()),
        ]));
        let listing = repo.list("envs/prod/storage").await.unwrap();
        let (change, notes) = deterministic_tags(&repo, &listing, &tagged(TagTarget::Cloud, Some("aws"), "example-shop-logs"), "github.com/example-org/infra-terraform", "envs/prod/storage").await;
        let change = change.unwrap();
        assert_eq!(change.files.len(), 1, "only the file with the provider");
        assert!(change.files[0].after.contains("tf_path    = \"envs/prod/storage\"") && change.files[0].after.contains("tf_repo    = \"github.com/example-org/infra-terraform\""));
        assert!(notes[0].contains("tag-only changes"));
        // Azure has no default tags: Assist writes them.
        assert!(deterministic_tags(&repo, &listing, &tagged(TagTarget::Cloud, Some("azure"), "shopexports"), "github.com/example-org/infra-terraform", "envs/prod/storage").await.0.is_none());
        let all = repo.list("").await.unwrap();
        let (change, notes) = deterministic_tags(&repo, &all, &tagged(TagTarget::Image, None, "registry.example.com/catalog-service:2.4.0"), "github.com/example-org/catalog-service", "").await;
        let change = change.unwrap();
        assert_eq!(change.files[0].path, "services/catalog-service/Dockerfile");
        assert!(change.files[0].after.contains("org.opencontainers.image.source=\"https://github.com/example-org/catalog-service\"") && notes[0].contains("GIT_COMMIT"));
        assert_eq!(tags_to_add(TagTarget::Image, "x", "y"), vec![]);
    }

    /// An infrastructure fix through the fake model: it reads the source, may not change a file outside it, proposes
    /// the change in the module defining the resource and concludes; the change's old content is filled in.
    #[tokio::test]
    async fn a_fix_infra_run_changes_only_its_source() {
        let module = "resource \"aws_db_instance\" \"orders\" {\n  identifier          = \"orders-prod\"\n  publicly_accessible = true\n}\n";
        let fixed = module.replace("true", "false");
        let repo = MapFiles(HashMap::from([
            ("envs/prod/main.tf".to_string(), "module \"orders_db\" {\n  source = \"../../modules/orders-db\"\n}\n".to_string()),
            ("modules/orders-db/main.tf".to_string(), module.to_string()),
            ("envs/staging/main.tf".to_string(), "x".to_string()),
        ]));
        let (base_url, seen) = fake_anthropic(vec![
            sse_tool("t1", "read_source_file", json!({"path": "envs/staging/main.tf"})),
            sse_tool("t2", "propose_change", json!({"title": "x", "files": [{"path": "envs/staging/main.tf", "content": "y"}]})),
            sse_tool("t3", "propose_change", json!({"title": "Make orders-prod private", "files": [{"path": "modules/orders-db/main.tf", "content": fixed}]})),
            sse_tool("t4", "conclude", json!({"summary": "publicly_accessible was true", "verdict": "change_proposed"})),
            sse_text("Done."),
        ])
        .await;
        let model = HttpModel { provider: Provider::Anthropic { base_url, api_key: "sk-test".into(), model: "claude-opus-5".into(), fallbacks: true }, max_tokens: 1000, reach: Reach::Trusted };
        let p = Provenance { repo: Some("github.com/example-org/infra-terraform".into()), path: Some("envs/prod".into()), git_ref: None, address: Some("module.orders_db.aws_db_instance.orders".into()), application: None, kind: ProvenanceKind::Terraform, confidence: provenance::Confidence::Exact, via: provenance::ProvenanceVia::State };
        let prep = infra::prepare(&repo, &p, &InfraFinding { resource_name: "orders-prod".into(), resource_type: "AwsRdsDbInstance".into(), ..Default::default() }).await;
        assert_eq!(prep.write_dirs, vec!["envs/prod", "modules/orders-db"]);
        let tools = RunTools::new(InfraTools::new(repo, prep.scopes.clone(), prep.reads), RunKind::FixInfra, false).only_under(prep.write_dirs.clone());
        let mut msgs = vec![Msg::User(infra::infra_opening("cloud misconfiguration", &json!({"checkId": "rds_instance_no_public_access"}), &p, &prep, "arn:aws:rds:eu-west-1:123456789012:db:orders-prod"))];
        let outcome = Loop { model: &model, tools: &tools, system: "s", auto_approve: false, max_steps: agent::MAX_STEPS }.run(&mut msgs, &mut Usage::default(), &mut |_| {}).await;
        let mut done = finish(&tools, outcome, &msgs, None);
        fill_before(&tools.base().files, &prep.files, &mut done).await;
        let change = done.change.unwrap();
        assert_eq!((change.files[0].path.as_str(), change.files[0].before.as_deref(), change.files[0].after.as_str()), ("modules/orders-db/main.tf", Some(module), fixed.as_str()));
        let bodies = seen.lock().unwrap().clone();
        assert!(bodies[1].to_string().contains("Only files under envs/prod, modules/orders-db can be read"), "the read refusal reached the model");
        assert!(bodies[2].to_string().contains("Only files under envs/prod, modules/orders-db may change"), "the write refusal reached the model");
        let names: Vec<String> = bodies[0]["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap().to_string()).collect();
        assert_eq!(names, vec!["list_source_files", "read_source_file", "propose_change", "conclude"]);
    }

    /// An infrastructure fix keeps its source and stays in its org. Skipped unless OC_TEST_POSTGRES_URL and
    /// OC_TEST_CLICKHOUSE_URL are set.
    #[tokio::test]
    async fn an_infra_fix_keeps_its_source_in_its_org() {
        let Some((_, states)) = crate::test_support::replicas_with(1, |_| {}).await else { return };
        let st = states[0].clone();
        let ctx = crate::test_support::admin_of_new_org(&st, "remediation-infra").await;
        let p = Provenance { repo: Some("github.com/example-org/gitops".into()), path: Some("apps/catalog".into()), git_ref: Some("main".into()), address: Some("StatefulSet/catalog/catalog-service".into()), application: Some("catalog".into()), kind: ProvenanceKind::Helm, confidence: provenance::Confidence::Inferred, via: provenance::ProvenanceVia::Argocd };
        let source = FixSource { resource: "StatefulSet/catalog/catalog-service".into(), provenance: Some(p), located: None, confirmed: true, path: None };
        let id = insert_fix(&st, &ctx, "cloud_misconfiguration", "misconfig:ocs-k8s-009:uid-1", "github.com/example-org/gitops", "assist", None, vec![], None, None, Some(source.clone())).await.unwrap();
        let v = fix_view(&st, &ctx.org_id, load_fix(&st, &ctx.org_id, &id).await.unwrap()).await.unwrap();
        assert_eq!(v.source, Some(source));
        let other = crate::test_support::admin_of_new_org(&st, "remediation-infra-other").await;
        assert!(matches!(load_fix(&st, &other.org_id, &id).await, Err(ApiError::NotFound(_))));
    }
}
