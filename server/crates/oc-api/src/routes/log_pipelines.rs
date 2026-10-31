//! Log pipelines (Log Configuration → Pipelines): CRUD, ordering, the integration pipeline library
//! and a test endpoint that runs processors on a sample log (the editors' live preview).
//!
//! The indexer (`logproc`) reloads enabled pipelines every 10 s and runs them, in order, on every
//! log before index routing. Processors are stored as JSON in Datadog's pipeline format; the
//! engine is in `crate::pipelines`.

use std::collections::HashMap;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use utoipa::ToSchema;

use crate::auth::{AuthContext, Writer};
use crate::error::{ApiError, ApiResult};
use crate::pipelines::dates::to_rfc3339;
use crate::pipelines::grok::Grok;
use crate::pipelines::{library, Log, Pipeline};
use crate::state::AppState;

const MAX_PIPELINES: i64 = 100;
const MAX_PROCESSORS: usize = 50;
const MAX_PROCESSORS_BYTES: usize = 256 * 1024;
const MAX_SAMPLES: usize = 20;
const MAX_SAMPLE_CHARS: usize = 10_000;

#[derive(Serialize, ToSchema, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct LogPipeline {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    /// Logs matching this search go through the pipeline; '' = every log.
    pub filter: String,
    /// Processors in Datadog's format (`{"type": "grok-parser", "name", "enabled", …}`), run in order.
    #[sqlx(json)]
    #[schema(value_type = Vec<Object>)]
    pub processors: Vec<Value>,
    pub position: i32,
    /// `custom`, or `integration:<name>` for a pipeline installed from an integration.
    pub source: String,
    /// Integration pipelines can only be enabled, disabled, moved or deleted; clone one to edit it.
    pub read_only: bool,
    pub description: String,
    /// `key:value` tags (set through Datadog's API).
    #[sqlx(json)]
    pub tags: Vec<String>,
    /// Epoch ms of the last change, and who made it.
    pub updated_at: i64,
    pub updated_by: Option<String>,
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct LogPipelineInput {
    pub name: String,
    #[serde(default)]
    pub filter: String,
    #[serde(default = "yes")]
    pub enabled: bool,
    #[serde(default)]
    #[schema(value_type = Vec<Object>)]
    pub processors: Vec<Value>,
}

fn yes() -> bool {
    true
}

#[derive(Deserialize, ToSchema)]
pub struct LogPipelinePatch {
    pub enabled: bool,
}

#[derive(Deserialize, ToSchema)]
pub struct ReorderPipelines {
    /// Every pipeline id, in processing order.
    pub ids: Vec<String>,
}

/// An integration's pipeline that can be installed.
#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct LibraryPipeline {
    /// The integration (e.g. `nginx`).
    pub id: String,
    pub name: String,
    pub filter: String,
    #[schema(value_type = Vec<Object>)]
    pub processors: Vec<Value>,
    pub upstream: PipelineUpstream,
    /// Already installed in this organization.
    pub installed: bool,
}

#[derive(Serialize, ToSchema)]
pub struct PipelineUpstream {
    pub repository: String,
    pub revision: String,
    pub path: String,
    pub license: String,
}

/// A log for the test endpoint, before or after processing.
#[derive(Serialize, Deserialize, ToSchema, Clone, Debug, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct SampleLog {
    #[serde(default)]
    pub message: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub service: String,
    #[serde(default)]
    pub host: String,
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub tags: HashMap<String, String>,
    /// Nested attributes.
    #[serde(default)]
    #[schema(value_type = Object)]
    pub attributes: Map<String, Value>,
    /// RFC 3339; a date remapper sets it.
    #[serde(default)]
    pub date: Option<String>,
    #[serde(default)]
    pub trace_id: String,
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct PipelineTestRequest {
    /// The pipeline's filter; omitted = every log.
    #[serde(default)]
    pub filter: Option<String>,
    #[serde(default)]
    #[schema(value_type = Vec<Object>)]
    pub processors: Vec<Value>,
    pub log: SampleLog,
}

#[derive(Serialize, ToSchema, Debug)]
#[serde(rename_all = "camelCase")]
pub struct PipelineTestResult {
    /// The filter matched (otherwise `log` is unchanged).
    pub matched: bool,
    pub log: SampleLog,
    /// Processors that don't compile (they were skipped).
    pub errors: Vec<String>,
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct GrokTestRequest {
    #[serde(default)]
    pub support_rules: String,
    #[serde(default)]
    pub match_rules: String,
    #[serde(default)]
    pub samples: Vec<String>,
}

#[derive(Serialize, ToSchema, Debug)]
#[serde(rename_all = "camelCase")]
pub struct GrokTestResult {
    /// Why the rules don't compile.
    pub error: Option<String>,
    pub samples: Vec<GrokSampleResult>,
}

#[derive(Serialize, ToSchema, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct GrokSampleResult {
    pub matched: bool,
    /// The match rule that matched.
    pub rule: Option<String>,
    /// What it extracted, as nested attributes.
    #[schema(value_type = Object)]
    pub attributes: Map<String, Value>,
}

fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis() as i64
}

/// Trims and checks a pipeline; every processor of a supported type must compile.
pub(crate) fn validate(input: &mut LogPipelineInput) -> Result<(), ApiError> {
    input.name = input.name.trim().to_string();
    input.filter = input.filter.trim().to_string();
    if input.name.is_empty() || input.name.chars().count() > 100 {
        return Err(ApiError::BadRequest("Pipeline names are 1–100 characters".into()));
    }
    if input.processors.len() > MAX_PROCESSORS {
        return Err(ApiError::BadRequest(format!("At most {MAX_PROCESSORS} processors per pipeline")));
    }
    if serde_json::to_vec(&input.processors).map_or(0, |b| b.len()) > MAX_PROCESSORS_BYTES {
        return Err(ApiError::BadRequest("The processors are too large (256 KiB at most)".into()));
    }
    if let Some(i) = input.processors.iter().position(|p| p.get("type").and_then(Value::as_str).is_none_or(str::is_empty)) {
        return Err(ApiError::BadRequest(format!("Processor {} needs a type", i + 1)));
    }
    let (_, errors) = Pipeline::compile(&input.name, &input.filter, &input.processors);
    if !errors.is_empty() {
        return Err(ApiError::BadRequest(errors.join("\n")));
    }
    Ok(())
}

/// Runs processors on a sample log.
fn run_test(req: PipelineTestRequest, now_ms: i64) -> PipelineTestResult {
    let s = req.log;
    let row = serde_json::json!({
        "message": s.message, "status": s.status, "service": s.service, "host": s.host, "source": s.source,
        "trace_id": s.trace_id, "tags": s.tags, "attributes": Value::Object(s.attributes.clone()).to_string(),
    });
    let (pipeline, errors) = Pipeline::compile("test", req.filter.as_deref().unwrap_or(""), &req.processors);
    let mut log = Log::from_row(&row);
    let matched = pipeline.apply(&mut log, now_ms);
    let date = log.timestamp_ms.map(to_rfc3339).or(s.date);
    let log = SampleLog { message: log.message, status: log.status, service: log.service, host: log.host, source: log.source, tags: log.tags, attributes: log.attrs, date, trace_id: log.trace_id };
    PipelineTestResult { matched, log, errors }
}

/// Parses samples with grok rules.
fn grok_test(req: &GrokTestRequest, now_ms: i64) -> GrokTestResult {
    let unmatched = || GrokSampleResult { matched: false, rule: None, attributes: Map::new() };
    let grok = match Grok::compile(&req.support_rules, &req.match_rules) {
        Ok(g) => g,
        Err(e) => return GrokTestResult { error: Some(e), samples: req.samples.iter().map(|_| unmatched()).collect() },
    };
    let samples = req
        .samples
        .iter()
        .map(|sample| match grok.parse(sample, now_ms) {
            Some(parsed) => {
                // Placed as the processor would place them: dotted paths nest, `message` included.
                let mut log = Log::default();
                for (path, v) in parsed.values {
                    match (path.is_empty(), v) {
                        (true, Value::Object(o)) => o.into_iter().for_each(|(k, v)| log.set(&k, v)),
                        (true, _) => {}
                        (false, v) => log.set(&path, v),
                    }
                }
                let mut attributes = log.attrs;
                if !log.message.is_empty() {
                    attributes.insert("message".into(), Value::from(log.message));
                }
                GrokSampleResult { matched: true, rule: Some(parsed.rule), attributes }
            }
            None => unmatched(),
        })
        .collect();
    GrokTestResult { error: None, samples }
}

const COLS: &str = "p.id::text AS id, p.name, p.enabled, p.filter, p.processors, p.position, p.source, p.source <> 'custom' AS read_only, p.description, p.tags, \
    (extract(epoch FROM p.updated_at) * 1000)::bigint AS updated_at, u.name AS updated_by";
const FROM: &str = "FROM log_pipelines p LEFT JOIN users u ON u.id = p.updated_by";

async fn get_pipeline(st: &AppState, org: &str, id: &str) -> Result<LogPipeline, ApiError> {
    sqlx::query_as(&format!("SELECT {COLS} {FROM} WHERE p.org_id = $1 AND p.id::text = $2"))
        .bind(org)
        .bind(id)
        .fetch_optional(&st.postgres)
        .await?
        .ok_or_else(|| ApiError::NotFound("Pipeline not found".into()))
}

async fn count(st: &AppState, org: &str) -> Result<i64, ApiError> {
    let (n,): (i64,) = sqlx::query_as("SELECT count(*) FROM log_pipelines WHERE org_id = $1").bind(org).fetch_one(&st.postgres).await?;
    Ok(n)
}

/// Inserts a pipeline at the end of the org's list.
pub(crate) async fn insert(st: &AppState, org: &str, input: &LogPipelineInput, source: &str) -> Result<Option<String>, ApiError> {
    if count(st, org).await? >= MAX_PIPELINES {
        return Err(ApiError::BadRequest(format!("An organization can have at most {MAX_PIPELINES} pipelines")));
    }
    let row: Option<(String,)> = sqlx::query_as(
        "INSERT INTO log_pipelines (org_id, name, enabled, filter, processors, source, position) \
         VALUES ($1, $2, $3, $4, $5, $6, (SELECT COALESCE(max(position), -1) + 1 FROM log_pipelines WHERE org_id = $1)) \
         ON CONFLICT DO NOTHING RETURNING id::text",
    )
    .bind(org)
    .bind(&input.name)
    .bind(input.enabled)
    .bind(&input.filter)
    .bind(Value::Array(input.processors.clone()))
    .bind(source)
    .fetch_optional(&st.postgres)
    .await?;
    Ok(row.map(|(id,)| id))
}

/// Records who last changed a pipeline.
async fn edited_by(st: &AppState, ctx: &AuthContext, id: &str) -> Result<(), ApiError> {
    sqlx::query("UPDATE log_pipelines SET updated_by = $3::uuid WHERE org_id = $1 AND id::text = $2").bind(&ctx.org_id).bind(id).bind(&ctx.user_id).execute(&st.postgres).await?;
    Ok(())
}

#[utoipa::path(get, path = "/api/v1/logs/pipelines", operation_id = "list_log_pipelines", responses((status = 200, body = Vec<LogPipeline>)), tag = "logs")]
pub async fn list(State(st): State<AppState>, ctx: AuthContext) -> ApiResult<Vec<LogPipeline>> {
    let rows = sqlx::query_as(&format!("SELECT {COLS} {FROM} WHERE p.org_id = $1 ORDER BY p.position, p.created_at")).bind(&ctx.org_id).fetch_all(&st.postgres).await?;
    Ok(Json(rows))
}

#[utoipa::path(post, path = "/api/v1/logs/pipelines", operation_id = "create_log_pipeline", request_body = LogPipelineInput, responses((status = 200, body = LogPipeline)), tag = "logs")]
pub async fn create(State(st): State<AppState>, Writer(ctx): Writer, Json(mut input): Json<LogPipelineInput>) -> ApiResult<LogPipeline> {
    validate(&mut input)?;
    let id = insert(&st, &ctx.org_id, &input, "custom").await?.ok_or_else(|| ApiError::BadRequest("The pipeline could not be created".into()))?;
    edited_by(&st, &ctx, &id).await?;
    ctx.audit(&st, "log_pipeline.created", &input.name, &input.filter).await;
    Ok(Json(get_pipeline(&st, &ctx.org_id, &id).await?))
}

#[utoipa::path(put, path = "/api/v1/logs/pipelines/{id}", operation_id = "update_log_pipeline", params(("id" = String, Path)), request_body = LogPipelineInput, responses((status = 200, body = LogPipeline)), tag = "logs")]
pub async fn update(State(st): State<AppState>, Writer(ctx): Writer, Path(id): Path<String>, Json(mut input): Json<LogPipelineInput>) -> ApiResult<LogPipeline> {
    validate(&mut input)?;
    if get_pipeline(&st, &ctx.org_id, &id).await?.read_only {
        return Err(ApiError::BadRequest("Integration pipelines are read-only: clone it to make changes".into()));
    }
    sqlx::query("UPDATE log_pipelines SET name = $3, enabled = $4, filter = $5, processors = $6, updated_at = now() WHERE org_id = $1 AND id::text = $2 AND source = 'custom'")
        .bind(&ctx.org_id)
        .bind(&id)
        .bind(&input.name)
        .bind(input.enabled)
        .bind(&input.filter)
        .bind(Value::Array(input.processors.clone()))
        .execute(&st.postgres)
        .await?;
    edited_by(&st, &ctx, &id).await?;
    ctx.audit(&st, "log_pipeline.updated", &input.name, &format!("{} processors", input.processors.len())).await;
    Ok(Json(get_pipeline(&st, &ctx.org_id, &id).await?))
}

/// Enables or disables a pipeline (integration pipelines too).
#[utoipa::path(patch, path = "/api/v1/logs/pipelines/{id}", operation_id = "set_log_pipeline_enabled", params(("id" = String, Path)), request_body = LogPipelinePatch, responses((status = 200, body = LogPipeline)), tag = "logs")]
pub async fn set_enabled(State(st): State<AppState>, Writer(ctx): Writer, Path(id): Path<String>, Json(p): Json<LogPipelinePatch>) -> ApiResult<LogPipeline> {
    let n = sqlx::query("UPDATE log_pipelines SET enabled = $3, updated_at = now() WHERE org_id = $1 AND id::text = $2")
        .bind(&ctx.org_id)
        .bind(&id)
        .bind(p.enabled)
        .execute(&st.postgres)
        .await?
        .rows_affected();
    if n == 0 {
        return Err(ApiError::NotFound("Pipeline not found".into()));
    }
    edited_by(&st, &ctx, &id).await?;
    let pipeline = get_pipeline(&st, &ctx.org_id, &id).await?;
    ctx.audit(&st, if p.enabled { "log_pipeline.enabled" } else { "log_pipeline.disabled" }, &pipeline.name, "").await;
    Ok(Json(pipeline))
}

#[utoipa::path(delete, path = "/api/v1/logs/pipelines/{id}", operation_id = "delete_log_pipeline", params(("id" = String, Path)), responses((status = 204)), tag = "logs")]
pub async fn delete(State(st): State<AppState>, Writer(ctx): Writer, Path(id): Path<String>) -> Result<StatusCode, ApiError> {
    let row: Option<(String,)> = sqlx::query_as("DELETE FROM log_pipelines WHERE org_id = $1 AND id::text = $2 RETURNING name").bind(&ctx.org_id).bind(&id).fetch_optional(&st.postgres).await?;
    let Some((name,)) = row else { return Err(ApiError::NotFound("Pipeline not found".into())) };
    ctx.audit(&st, "log_pipeline.deleted", &name, "").await;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(post, path = "/api/v1/logs/pipelines/reorder", operation_id = "reorder_log_pipelines", request_body = ReorderPipelines, responses((status = 204)), tag = "logs")]
pub async fn reorder(State(st): State<AppState>, Writer(ctx): Writer, Json(req): Json<ReorderPipelines>) -> Result<StatusCode, ApiError> {
    let mut tx = st.postgres.begin().await?;
    for (pos, id) in req.ids.iter().enumerate() {
        sqlx::query("UPDATE log_pipelines SET position = $3 WHERE org_id = $1 AND id::text = $2").bind(&ctx.org_id).bind(id).bind(pos as i32).execute(&mut *tx).await?;
    }
    tx.commit().await?;
    ctx.audit(&st, "log_pipeline.reordered", "", "").await;
    Ok(StatusCode::NO_CONTENT)
}

/// Runs processors (and optionally a filter) on a sample log and returns the processed log.
#[utoipa::path(post, path = "/api/v1/logs/pipelines/test", operation_id = "test_log_pipeline", request_body = PipelineTestRequest, responses((status = 200, body = PipelineTestResult)), tag = "logs")]
pub async fn test(_ctx: AuthContext, Json(req): Json<PipelineTestRequest>) -> ApiResult<PipelineTestResult> {
    if req.processors.len() > MAX_PROCESSORS || req.log.message.chars().count() > MAX_SAMPLE_CHARS {
        return Err(ApiError::BadRequest(format!("At most {MAX_PROCESSORS} processors and a {MAX_SAMPLE_CHARS}-character message")));
    }
    Ok(Json(run_test(req, now_ms())))
}

/// Parses sample values with grok rules (the grok parser editor's live preview).
#[utoipa::path(post, path = "/api/v1/logs/pipelines/grok", operation_id = "parse_grok_samples", request_body = GrokTestRequest, responses((status = 200, body = GrokTestResult)), tag = "logs")]
pub async fn grok(_ctx: AuthContext, Json(req): Json<GrokTestRequest>) -> ApiResult<GrokTestResult> {
    if req.samples.len() > MAX_SAMPLES || req.samples.iter().any(|s| s.chars().count() > MAX_SAMPLE_CHARS) {
        return Err(ApiError::BadRequest(format!("At most {MAX_SAMPLES} samples of {MAX_SAMPLE_CHARS} characters")));
    }
    Ok(Json(grok_test(&req, now_ms())))
}

/// Integration pipelines (from integrations-core) and whether each is installed.
#[utoipa::path(get, path = "/api/v1/logs/pipeline-library", operation_id = "list_pipeline_library", responses((status = 200, body = Vec<LibraryPipeline>)), tag = "logs")]
pub async fn library_list(State(st): State<AppState>, ctx: AuthContext) -> ApiResult<Vec<LibraryPipeline>> {
    let installed: Vec<(String,)> = sqlx::query_as("SELECT source FROM log_pipelines WHERE org_id = $1 AND source <> 'custom'").bind(&ctx.org_id).fetch_all(&st.postgres).await?;
    let installed: std::collections::HashSet<String> = installed.into_iter().map(|(s,)| s).collect();
    Ok(Json(
        library::all()
            .iter()
            .map(|p| LibraryPipeline {
                id: p.id.clone(),
                name: p.name.clone(),
                filter: p.filter.clone(),
                processors: p.processors.clone(),
                upstream: PipelineUpstream { repository: p.source.repository.clone(), revision: p.source.revision.clone(), path: p.source.path.clone(), license: p.source.license.clone() },
                installed: installed.contains(&format!("integration:{}", p.id)),
            })
            .collect(),
    ))
}

/// Installs an integration's pipeline (read-only, at the end of the list).
#[utoipa::path(post, path = "/api/v1/logs/pipeline-library/{id}/install", operation_id = "install_integration_pipeline", params(("id" = String, Path)), responses((status = 200, body = LogPipeline)), tag = "logs")]
pub async fn install(State(st): State<AppState>, Writer(ctx): Writer, Path(id): Path<String>) -> ApiResult<LogPipeline> {
    let p = library::get(&id).ok_or_else(|| ApiError::NotFound("No integration pipeline with that name".into()))?;
    let input = LogPipelineInput { name: p.name.clone(), filter: p.filter.clone(), enabled: true, processors: p.processors.clone() };
    let new_id = insert(&st, &ctx.org_id, &input, &format!("integration:{}", p.id)).await?.ok_or_else(|| ApiError::BadRequest(format!("The {} pipeline is already installed", p.name)))?;
    ctx.audit(&st, "log_pipeline.installed", &p.name, &p.source.path).await;
    Ok(Json(get_pipeline(&st, &ctx.org_id, &new_id).await?))
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const NOW: i64 = 1_792_933_200_000;

    fn input(name: &str, processors: Value) -> LogPipelineInput {
        LogPipelineInput { name: name.into(), filter: " source:nginx ".into(), enabled: true, processors: processors.as_array().unwrap().clone() }
    }

    #[test]
    fn validation() {
        let mut ok = input("  Nginx  ", json!([{"type": "grok-parser", "grok": {"matchRules": "r %{data:x}"}}, {"type": "user-agent-parser"}]));
        assert!(validate(&mut ok).is_ok());
        assert_eq!((ok.name.as_str(), ok.filter.as_str()), ("Nginx", "source:nginx"));
        assert!(validate(&mut input(" ", json!([]))).is_err());
        assert!(validate(&mut input(&"x".repeat(101), json!([]))).is_err());
        let err = validate(&mut input("p", json!([{"type": "grok-parser", "name": "bad", "grok": {"matchRules": "r %{nope}"}}]))).unwrap_err();
        assert!(err.to_string().contains("bad"), "{err}");
        assert!(validate(&mut input("p", json!(["not an object"]))).is_err());
        assert!(validate(&mut input("p", json!([{"name": "no type"}]))).is_err());
        let many: Vec<Value> = (0..=MAX_PROCESSORS).map(|_| json!({"type": "status-remapper", "sources": ["level"]})).collect();
        assert!(validate(&mut input("p", Value::Array(many))).is_err());
        // Nested pipelines go one level deep.
        let nested = json!([{"type": "pipeline", "filter": {"query": "*"}, "processors": [{"type": "pipeline", "processors": []}]}]);
        assert!(validate(&mut input("p", nested)).is_err());
    }

    #[test]
    fn test_endpoint_runs_processors_on_the_sample() {
        let req = PipelineTestRequest {
            filter: Some("source:nginx".into()),
            processors: json!([
                {"type": "grok-parser", "grok": {"matchRules": "r %{date(\"yyyy-MM-dd HH:mm:ss\"):ts} %{word:level} %{data:msg}"}},
                {"type": "date-remapper", "sources": ["ts"]},
                {"type": "status-remapper", "sources": ["level"]},
                {"type": "message-remapper", "sources": ["msg"]},
                {"type": "grok-parser", "name": "broken", "grok": {"matchRules": "r %{nope}"}}
            ])
            .as_array()
            .unwrap()
            .clone(),
            log: SampleLog { message: "2026-10-25 13:00:00 ERROR payment declined".into(), source: "nginx".into(), status: "info".into(), ..Default::default() },
        };
        let out = run_test(req, NOW);
        assert!(out.matched);
        assert_eq!((out.log.status.as_str(), out.log.message.as_str()), ("error", "payment declined"));
        assert_eq!(out.log.date.as_deref(), Some("2026-10-25T13:00:00.000Z"));
        assert_eq!(out.log.attributes["level"], json!("ERROR"));
        assert_eq!(out.errors.len(), 1);
        let miss = run_test(PipelineTestRequest { filter: Some("source:redis".into()), processors: vec![], log: SampleLog { message: "m".into(), ..Default::default() } }, NOW);
        assert!(!miss.matched);
        assert_eq!(miss.log.message, "m");
    }

    #[test]
    fn grok_preview_parses_each_sample() {
        let req = GrokTestRequest { support_rules: "_m %{word:http.method}".into(), match_rules: "access %{_m} %{integer:http.status_code}".into(), samples: vec!["GET 200".into(), "nope".into()] };
        let out = grok_test(&req, NOW);
        assert_eq!(out.error, None);
        assert_eq!(out.samples[0], GrokSampleResult { matched: true, rule: Some("access".into()), attributes: json!({"http": {"method": "GET", "status_code": 200}}).as_object().unwrap().clone() });
        assert!(!out.samples[1].matched);
        let bad = grok_test(&GrokTestRequest { support_rules: String::new(), match_rules: "r %{nope}".into(), samples: vec!["x".into()] }, NOW);
        assert!(bad.error.unwrap().contains("nope"));
        assert_eq!(bad.samples.len(), 1);
        assert!(!bad.samples[0].matched);
    }
}
