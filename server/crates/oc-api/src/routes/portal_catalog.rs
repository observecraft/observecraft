//! Developer Portal → Software Catalog: every entity the org runs, from telemetry and from what
//! users declare (see [`super::portal_model`] for the inference and merge rules).
//!
//! - **Telemetry** (ClickHouse, through `AppState::ch(org)`): APM services ([`super::apm::services`]),
//!   outbound spans for datastores, queues and external providers, entry spans for endpoints and
//!   error types, services named on logs (through the reader's log restrictions) and metrics, and
//!   CI repositories ([`super::ci::repositories`]).
//! - **Declared** (Postgres): entity definitions (`portal_entities`), service metadata shared with
//!   Scorecards, teams, SLOs, monitors and incidents.
//! - **Writes** (standard users and admins, audited): definitions from the UI, the API or imported
//!   documents. A service definition's owner, contacts, repository and docs go to the service
//!   metadata, which stays their single source.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;

use super::apm::{self, ApmRequest, ENTRY};
use super::logs::{bind, scope, TimeRange};
use super::portal_model::{
    assemble, definition_from_value, entities_per_team, infer_entities, metadata_of, parse_documents, ApmService, CatalogInputs, CiRepo, EndpointStat, EntityDefinition, IncidentRef, PeerCall,
    PortalEntity, ReliabilityRef, ServiceMeta, StoredDefinition,
};
use super::scorecards::{self, ServiceContact, ServiceMetadataInput};
use crate::auth::{AuthContext, Credential, Writer};
use crate::error::{ApiError, ApiResult};
use crate::log_access::LogAccess;
use crate::state::{AppState, OrgId};

const MAX_DEFINITIONS: i64 = 5000;
const MAX_ENDPOINTS: u32 = 1000;
const MAX_PEERS: u32 = 5000;

// ================================================================ API types

#[derive(Deserialize, ToSchema)]
pub struct PortalCatalogRequest {
    pub range: TimeRange,
    /// `deployment.environment`; empty or `*` for all.
    pub env: Option<String>,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct PortalTeam {
    pub handle: String,
    pub name: String,
    pub members: i64,
    /// Entities the team owns (endpoints excluded).
    pub entities: u32,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct PortalCatalog {
    pub entities: Vec<PortalEntity>,
    pub teams: Vec<PortalTeam>,
    /// Handles of the teams the signed-in user belongs to ("My Entities").
    pub my_teams: Vec<String>,
    pub envs: Vec<String>,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct PortalDefinitionItem {
    pub id: String,
    pub kind: String,
    pub name: String,
    /// ui | api | import
    pub source: String,
    pub definition: EntityDefinition,
    pub updated_at: i64,
}

#[derive(Deserialize, ToSchema)]
pub struct PortalImportRequest {
    /// One or more entity definition documents: YAML (separated by `---`) or JSON (an object or an array).
    pub content: String,
}

#[derive(Serialize, ToSchema)]
pub struct PortalImportResult {
    pub created: usize,
    pub updated: usize,
    pub entities: Vec<PortalDefinitionItem>,
}

// ================================================================ telemetry

#[derive(clickhouse::Row, Deserialize)]
struct PeerRow {
    service: String,
    db_system: String,
    db_name: String,
    server_address: String,
    peer_service: String,
    messaging_system: String,
    destination: String,
    hits: u64,
    errors: u64,
    p95: f64,
}

#[derive(clickhouse::Row, Deserialize)]
struct EndpointRow {
    service: String,
    resource: String,
    hits: u64,
    errors: u64,
    p95: f64,
}

fn time_and_env(range: TimeRange, env: &Option<String>) -> (String, Vec<oc_query::Param>) {
    use oc_query::Param;
    let mut clause = "Timestamp >= fromUnixTimestamp64Milli(?) AND Timestamp <= fromUnixTimestamp64Milli(?)".to_string();
    let mut params = vec![Param::Int(range.from), Param::Int(range.to)];
    if let Some(e) = env.as_deref().filter(|e| !e.is_empty() && *e != "*") {
        clause.push_str(" AND ResourceAttributes['deployment.environment'] = ?");
        params.push(Param::Str(e.to_string()));
    }
    (clause, params)
}

/// Outbound and messaging spans grouped by their peer attributes (OpenTelemetry and Datadog names).
async fn peer_calls(st: &AppState, org: &str, range: TimeRange, env: &Option<String>) -> Result<Vec<PeerCall>, ApiError> {
    let (clause, params) = time_and_env(range, env);
    let rows = bind(
        st.ch(org).query(&format!(
            "SELECT toString(ServiceName) AS service, \
             if(SpanAttributes['db.system'] != '', SpanAttributes['db.system'], SpanAttributes['db.type']) AS db_system, \
             multiIf(SpanAttributes['db.instance'] != '', SpanAttributes['db.instance'], SpanAttributes['db.namespace'] != '', SpanAttributes['db.namespace'], SpanAttributes['db.name']) AS db_name, \
             multiIf(SpanAttributes['server.address'] != '', SpanAttributes['server.address'], SpanAttributes['out.host'] != '', SpanAttributes['out.host'], SpanAttributes['peer.hostname']) AS server_address, \
             SpanAttributes['peer.service'] AS peer_service, SpanAttributes['messaging.system'] AS messaging_system, \
             multiIf(SpanAttributes['messaging.destination.name'] != '', SpanAttributes['messaging.destination.name'], SpanAttributes['messaging.destination'] != '', SpanAttributes['messaging.destination'], SpanAttributes['kafka.topic']) AS destination, \
             count() AS hits, countIf(StatusCode = 'Error') AS errors, quantile(0.95)(Duration) / 1e6 AS p95 \
             FROM otel.otel_traces WHERE OrgId = {{oc_org:String}} AND {clause} \
             AND (SpanKind IN ('Client', 'Producer') OR (SpanKind = 'Consumer' AND SpanAttributes['messaging.system'] != '')) \
             AND (SpanAttributes['peer.service'] != '' OR SpanAttributes['db.system'] != '' OR SpanAttributes['db.type'] != '' OR SpanAttributes['messaging.system'] != '') \
             GROUP BY service, db_system, db_name, server_address, peer_service, messaging_system, destination ORDER BY hits DESC LIMIT {MAX_PEERS}"
        )),
        &params,
    )
    .fetch_all::<PeerRow>()
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| PeerCall {
            service: r.service,
            db_system: r.db_system,
            db_name: r.db_name,
            server_address: r.server_address,
            peer_service: r.peer_service,
            messaging_system: r.messaging_system,
            destination: r.destination,
            hits: r.hits,
            errors: r.errors,
            p95_ms: if r.p95.is_finite() { r.p95 } else { 0.0 },
        })
        .collect())
}

async fn endpoints(st: &AppState, org: &str, range: TimeRange, env: &Option<String>) -> Result<Vec<EndpointStat>, ApiError> {
    let (clause, params) = time_and_env(range, env);
    let secs = ((range.to - range.from) as f64 / 1000.0).max(1.0);
    let rows = bind(
        st.ch(org).query(&format!(
            "SELECT toString(ServiceName) AS service, toString(SpanName) AS resource, count() AS hits, countIf(StatusCode = 'Error') AS errors, quantile(0.95)(Duration) / 1e6 AS p95 \
             FROM otel.otel_traces WHERE OrgId = {{oc_org:String}} AND {clause} AND SpanKind = 'Server' GROUP BY service, resource ORDER BY hits DESC LIMIT {MAX_ENDPOINTS}"
        )),
        &params,
    )
    .fetch_all::<EndpointRow>()
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| EndpointStat {
            req_per_sec: r.hits as f64 / secs,
            error_rate: if r.hits == 0 { 0.0 } else { r.errors as f64 / r.hits as f64 },
            p95_ms: if r.p95.is_finite() { r.p95 } else { 0.0 },
            requests: r.hits,
            service: r.service,
            resource: r.resource,
        })
        .collect())
}

/// Distinct error types on each service's entry spans.
async fn error_types(st: &AppState, org: &str, range: TimeRange, env: &Option<String>) -> Result<HashMap<String, u64>, ApiError> {
    let (clause, params) = time_and_env(range, env);
    let rows = bind(
        st.ch(org).query(&format!(
            "SELECT toString(ServiceName) AS service, uniqExact(if(SpanAttributes['exception.type'] != '', SpanAttributes['exception.type'], if(SpanAttributes['error.type'] != '', SpanAttributes['error.type'], toString(SpanName)))) AS n \
             FROM otel.otel_traces WHERE OrgId = {{oc_org:String}} AND {clause} AND {ENTRY} AND StatusCode = 'Error' GROUP BY service"
        )),
        &params,
    )
    .fetch_all::<(String, u64)>()
    .await?;
    Ok(rows.into_iter().collect())
}

/// Services named on the logs the reader may see.
async fn log_services(st: &AppState, org: &str, access: &LogAccess, range: TimeRange) -> Result<BTreeMap<String, u64>, ApiError> {
    let s = scope(org, access, range, &[], None);
    let rows = bind(st.ch(org).query(&format!("SELECT service, count() AS n FROM {} WHERE {} AND service != '' GROUP BY service ORDER BY n DESC LIMIT 2000", s.table, s.clause)), &s.params)
        .fetch_all::<(String, u64)>()
        .await?;
    Ok(rows.into_iter().collect())
}

/// Services named on metric series seen in the range (`service.name`, or a Datadog `service` tag).
async fn metric_services(st: &AppState, org: &str, range: TimeRange) -> Result<BTreeSet<String>, ApiError> {
    #[derive(clickhouse::Row, Deserialize)]
    struct Name {
        service: String,
    }
    let rows = st
        .ch(org)
        .query(
            "SELECT DISTINCT if(toString(ServiceName) != '', toString(ServiceName), Attributes['service']) AS service FROM otel.metrics_series \
             WHERE OrgId = {oc_org:String} AND LastSeen >= toStartOfHour(toDateTime(intDiv(?, 1000))) AND (ServiceName != '' OR Attributes['service'] != '') LIMIT 2000",
        )
        .bind(range.from)
        .fetch_all::<Name>()
        .await?;
    Ok(rows.into_iter().map(|r| r.service).filter(|s| !s.is_empty()).collect())
}

// ================================================================ storage

#[derive(sqlx::FromRow)]
struct DefinitionRow {
    id: String,
    kind: String,
    name: String,
    source: String,
    definition: Value,
    updated_at: i64,
}

const DEFINITION_COLUMNS: &str = "id::text AS id, kind, name, source, definition, (extract(epoch FROM updated_at) * 1000)::bigint AS updated_at";

fn item(row: DefinitionRow) -> Option<PortalDefinitionItem> {
    let definition = serde_json::from_value(row.definition).ok()?;
    Some(PortalDefinitionItem { id: row.id, kind: row.kind, name: row.name, source: row.source, definition, updated_at: row.updated_at })
}

async fn load_definitions(st: &AppState, org: &str) -> Result<Vec<PortalDefinitionItem>, ApiError> {
    let rows: Vec<DefinitionRow> = sqlx::query_as(&format!("SELECT {DEFINITION_COLUMNS} FROM portal_entities WHERE org_id = $1 ORDER BY kind, name")).bind(org).fetch_all(&st.postgres).await?;
    Ok(rows.into_iter().filter_map(item).collect())
}

async fn load_metadata(st: &AppState, org: &str) -> Result<BTreeMap<String, ServiceMeta>, ApiError> {
    let rows: Vec<(String, String, Value, String, String)> =
        sqlx::query_as("SELECT service, team, contacts, repo_url, docs_url FROM service_metadata WHERE org_id = $1").bind(org).fetch_all(&st.postgres).await?;
    Ok(rows
        .into_iter()
        .map(|(service, team, contacts, repo_url, docs_url)| (service, ServiceMeta { team, contacts: serde_json::from_value::<Vec<ServiceContact>>(contacts).unwrap_or_default(), repo_url, docs_url }))
        .collect())
}

async fn reliability(st: &AppState, org: &str, range: TimeRange) -> Result<(Vec<ReliabilityRef>, Vec<ReliabilityRef>, Vec<IncidentRef>), ApiError> {
    let monitors: Vec<(i64, String, String, Vec<String>, String)> =
        sqlx::query_as("SELECT id, name, query, tags, status FROM monitors WHERE org_id = $1 AND NOT draft").bind(org).fetch_all(&st.postgres).await?;
    let names: HashMap<i64, String> = monitors.iter().map(|(id, name, ..)| (*id, name.clone())).collect();
    #[derive(sqlx::FromRow)]
    struct SloRow {
        name: String,
        good_query: Option<String>,
        total_query: Option<String>,
        slice_query: Option<String>,
        tags: Vec<String>,
        monitor_ids: Vec<i64>,
    }
    let slos: Vec<SloRow> = sqlx::query_as("SELECT name, good_query, total_query, slice_query, tags, monitor_ids FROM slos WHERE org_id = $1").bind(org).fetch_all(&st.postgres).await?;
    let incidents: Vec<(Vec<String>, bool, bool)> = sqlx::query_as(
        "SELECT services, status <> 'resolved' AS open, declared_at >= to_timestamp($2 / 1000.0) AND declared_at <= to_timestamp($3 / 1000.0) AS in_range \
         FROM incidents WHERE org_id = $1 AND NOT test AND cardinality(services) > 0 AND (status <> 'resolved' OR declared_at >= to_timestamp($2 / 1000.0))",
    )
    .bind(org)
    .bind(range.from as f64)
    .bind(range.to as f64)
    .fetch_all(&st.postgres)
    .await?;
    Ok((
        slos.into_iter()
            .map(|s| ReliabilityRef {
                name: s.name,
                texts: [s.good_query, s.total_query, s.slice_query].into_iter().flatten().chain(s.tags).collect(),
                monitors: s.monitor_ids.iter().filter_map(|id| names.get(id).cloned()).collect(),
                alerting: false,
            })
            .collect(),
        monitors
            .into_iter()
            .map(|(_, name, query, tags, status)| ReliabilityRef { name, texts: std::iter::once(query).chain(tags).collect(), monitors: vec![], alerting: status == "alert" })
            .collect(),
        incidents.into_iter().map(|(services, open, in_range)| IncidentRef { services, open, in_range }).collect(),
    ))
}

/// Builds the catalog for a reader.
async fn build_catalog(st: &AppState, ctx: &AuthContext, access: &LogAccess, req: &PortalCatalogRequest) -> Result<PortalCatalog, ApiError> {
    let org = ctx.org_id.clone();
    let range = req.range;
    let apm_req = ApmRequest { range, env: req.env.clone(), query: String::new() };
    let services = apm::services(State(st.clone()), OrgId(org.clone()), Json(apm_req));
    let envs = apm::envs(State(st.clone()), OrgId(org.clone()), Json(apm::RangeRequest { range }));
    let repos = super::ci::repositories(State(st.clone()), OrgId(org.clone()));
    let (Json(services), Json(envs), Json(repos), peers, endpoints, error_types, log_services, metric_services) = tokio::try_join!(
        services,
        envs,
        repos,
        peer_calls(st, &org, range, &req.env),
        endpoints(st, &org, range, &req.env),
        error_types(st, &org, range, &req.env),
        log_services(st, &org, access, range),
        metric_services(st, &org, range),
    )?;
    let (slos, monitors, incidents) = reliability(st, &org, range).await?;
    let instrumented: HashSet<String> = services.iter().filter(|s| s.instrumented).map(|s| s.service.clone()).collect();
    let inputs = CatalogInputs {
        services: services
            .into_iter()
            .filter(|s| s.instrumented)
            .map(|s| ApmService {
                web: matches!(s.kind, apm::ServiceKind::Web),
                language: s.language,
                version: s.version,
                requests: s.requests,
                req_per_sec: s.req_per_sec,
                error_rate: s.error_rate,
                p95_ms: s.p95_ms,
                last_deploy: s.last_deploy,
                name: s.service,
            })
            .collect(),
        log_services,
        metric_services,
        peers: infer_entities(&peers, &instrumented),
        endpoints,
        error_types,
        definitions: load_definitions(st, &org).await?.into_iter().map(|d| StoredDefinition { id: d.id, definition: d.definition }).collect(),
        metadata: load_metadata(st, &org).await?,
        slos,
        monitors,
        incidents,
        repos: repos.into_iter().map(|r| CiRepo { name: r.repository, url: r.url, runs: r.runs, failures: r.failures, last_run: r.last_run }).collect(),
    };
    let entities = assemble(&inputs);
    let per_team = entities_per_team(&entities);
    let teams: Vec<(String, String, i64)> = sqlx::query_as(
        "SELECT t.handle, t.name, (SELECT count(*) FROM team_members m WHERE m.org_id = t.org_id AND m.handle = t.handle) FROM teams t WHERE t.org_id = $1 ORDER BY lower(t.name) LIMIT 1000",
    )
    .bind(&org)
    .fetch_all(&st.postgres)
    .await?;
    let my_teams: Vec<(String,)> =
        sqlx::query_as("SELECT handle FROM team_members WHERE org_id = $1 AND user_id::text = $2 ORDER BY handle").bind(&org).bind(&ctx.user_id).fetch_all(&st.postgres).await?;
    Ok(PortalCatalog {
        teams: teams.into_iter().map(|(handle, name, members)| PortalTeam { entities: per_team.get(&handle).copied().unwrap_or(0), handle, name, members }).collect(),
        my_teams: my_teams.into_iter().map(|(h,)| h).collect(),
        envs,
        entities,
    })
}

fn source_of(ctx: &AuthContext, import: bool) -> &'static str {
    if import {
        "import"
    } else if matches!(ctx.credential, Credential::Session) {
        "ui"
    } else {
        "api"
    }
}

/// A service definition's owner, contacts, repository and docs go to the shared service metadata;
/// other fields of the metadata stay when the definition leaves them out.
async fn sync_metadata(st: &AppState, ctx: &AuthContext, def: &EntityDefinition) -> Result<(), ApiError> {
    if def.kind != "service" {
        return Ok(());
    }
    let (team, contacts, repo_url, docs_url) = metadata_of(def);
    let current = load_metadata(st, &ctx.org_id).await?.remove(&def.metadata.name).unwrap_or_default();
    let input = ServiceMetadataInput {
        team: if team.is_empty() { current.team } else { team },
        contacts: if contacts.is_empty() { current.contacts } else { contacts },
        repo_url: if repo_url.is_empty() { current.repo_url } else { repo_url },
        docs_url: if docs_url.is_empty() { current.docs_url } else { docs_url },
    };
    scorecards::save_metadata(st, ctx, &def.metadata.name, &input).await?;
    Ok(())
}

/// Inserts or replaces the definition of (kind, name); answers it and whether it was created.
async fn upsert(st: &AppState, ctx: &AuthContext, def: &EntityDefinition, source: &str) -> Result<(PortalDefinitionItem, bool), ApiError> {
    let doc = serde_json::to_value(def).map_err(anyhow::Error::from)?;
    let row: (bool,) = sqlx::query_as(
        "INSERT INTO portal_entities (org_id, kind, name, definition, source, created_by, updated_by) VALUES ($1, $2, $3, $4, $5, $6::uuid, $6::uuid) \
         ON CONFLICT (org_id, kind, name) DO UPDATE SET definition = EXCLUDED.definition, source = EXCLUDED.source, updated_by = EXCLUDED.updated_by, updated_at = now() \
         RETURNING (xmax = 0)",
    )
    .bind(&ctx.org_id)
    .bind(&def.kind)
    .bind(&def.metadata.name)
    .bind(doc)
    .bind(source)
    .bind(&ctx.user_id)
    .fetch_one(&st.postgres)
    .await?;
    sync_metadata(st, ctx, def).await?;
    Ok((definition_by_name(st, &ctx.org_id, &def.kind, &def.metadata.name).await?, row.0))
}

async fn definition_by_name(st: &AppState, org: &str, kind: &str, name: &str) -> Result<PortalDefinitionItem, ApiError> {
    let row: Option<DefinitionRow> =
        sqlx::query_as(&format!("SELECT {DEFINITION_COLUMNS} FROM portal_entities WHERE org_id = $1 AND kind = $2 AND name = $3")).bind(org).bind(kind).bind(name).fetch_optional(&st.postgres).await?;
    row.and_then(item).ok_or_else(|| ApiError::NotFound("No such entity definition".into()))
}

fn valid_uuid(id: &str) -> bool {
    id.len() == 36 && id.chars().enumerate().all(|(i, c)| if [8, 13, 18, 23].contains(&i) { c == '-' } else { c.is_ascii_hexdigit() })
}

async fn definition_by_id(st: &AppState, org: &str, id: &str) -> Result<PortalDefinitionItem, ApiError> {
    let missing = || ApiError::NotFound("No such entity definition".into());
    if !valid_uuid(id) {
        return Err(missing());
    }
    let row: Option<DefinitionRow> =
        sqlx::query_as(&format!("SELECT {DEFINITION_COLUMNS} FROM portal_entities WHERE org_id = $1 AND id = $2::uuid")).bind(org).bind(id).fetch_optional(&st.postgres).await?;
    row.and_then(item).ok_or_else(missing)
}

async fn check_room(st: &AppState, org: &str, adding: usize) -> Result<(), ApiError> {
    let (count,): (i64,) = sqlx::query_as("SELECT count(*) FROM portal_entities WHERE org_id = $1").bind(org).fetch_one(&st.postgres).await?;
    if count + adding as i64 > MAX_DEFINITIONS {
        return Err(ApiError::BadRequest(format!("An organization can define at most {MAX_DEFINITIONS} entities")));
    }
    Ok(())
}

// ================================================================ handlers

/// Every entity of the catalog: services, datastores, queues, endpoints and external providers seen
/// in telemetry over the range, merged with the org's definitions, service metadata and repositories.
#[utoipa::path(post, path = "/api/v1/portal/catalog", operation_id = "portal_catalog", request_body = PortalCatalogRequest, responses((status = 200, body = PortalCatalog)), tag = "portal")]
pub async fn catalog(State(st): State<AppState>, ctx: AuthContext, access: LogAccess, Json(req): Json<PortalCatalogRequest>) -> ApiResult<PortalCatalog> {
    req.range.validate()?;
    Ok(Json(build_catalog(&st, &ctx, &access, &req).await?))
}

/// The org's entity definitions.
#[utoipa::path(get, path = "/api/v1/portal/entities", operation_id = "list_portal_entities", responses((status = 200, body = Vec<PortalDefinitionItem>)), tag = "portal")]
pub async fn list_entities(State(st): State<AppState>, OrgId(org): OrgId) -> ApiResult<Vec<PortalDefinitionItem>> {
    Ok(Json(load_definitions(&st, &org).await?))
}

#[utoipa::path(get, path = "/api/v1/portal/entities/{id}", operation_id = "get_portal_entity", params(("id" = String, Path)),
    responses((status = 200, body = PortalDefinitionItem), (status = 404, body = crate::error::ErrorBody)), tag = "portal")]
pub async fn get_entity(State(st): State<AppState>, OrgId(org): OrgId, Path(id): Path<String>) -> ApiResult<PortalDefinitionItem> {
    Ok(Json(definition_by_id(&st, &org, &id).await?))
}

/// Creates or replaces the definition of an entity (by kind and name), apiVersion v3.
#[utoipa::path(post, path = "/api/v1/portal/entities", operation_id = "upsert_portal_entity", request_body = EntityDefinition,
    responses((status = 200, body = PortalDefinitionItem), (status = 400, body = crate::error::ErrorBody)), tag = "portal")]
pub async fn upsert_entity(State(st): State<AppState>, Writer(ctx): Writer, Json(input): Json<Value>) -> ApiResult<PortalDefinitionItem> {
    let def = definition_from_value(input).map_err(ApiError::BadRequest)?;
    check_room(&st, &ctx.org_id, 1).await?;
    let (saved, created) = upsert(&st, &ctx, &def, source_of(&ctx, false)).await?;
    ctx.audit(&st, if created { "portal.entity_created" } else { "portal.entity_updated" }, &saved.id, &format!("{}:{}", saved.kind, saved.name)).await;
    Ok(Json(saved))
}

/// Replaces a definition; its kind and name may change when no other definition has them.
#[utoipa::path(put, path = "/api/v1/portal/entities/{id}", operation_id = "update_portal_entity", params(("id" = String, Path)), request_body = EntityDefinition,
    responses((status = 200, body = PortalDefinitionItem), (status = 400, body = crate::error::ErrorBody), (status = 404, body = crate::error::ErrorBody)), tag = "portal")]
pub async fn update_entity(State(st): State<AppState>, Writer(ctx): Writer, Path(id): Path<String>, Json(input): Json<Value>) -> ApiResult<PortalDefinitionItem> {
    let current = definition_by_id(&st, &ctx.org_id, &id).await?;
    let def = definition_from_value(input).map_err(ApiError::BadRequest)?;
    let doc = serde_json::to_value(&def).map_err(anyhow::Error::from)?;
    let taken: Option<(String,)> = sqlx::query_as("SELECT id::text FROM portal_entities WHERE org_id = $1 AND kind = $2 AND name = $3 AND id <> $4::uuid")
        .bind(&ctx.org_id)
        .bind(&def.kind)
        .bind(&def.metadata.name)
        .bind(&id)
        .fetch_optional(&st.postgres)
        .await?;
    if taken.is_some() {
        return Err(ApiError::BadRequest(format!("{}:{} is already defined", def.kind, def.metadata.name)));
    }
    sqlx::query("UPDATE portal_entities SET kind = $3, name = $4, definition = $5, source = $6, updated_by = $7::uuid, updated_at = now() WHERE org_id = $1 AND id = $2::uuid")
        .bind(&ctx.org_id)
        .bind(&id)
        .bind(&def.kind)
        .bind(&def.metadata.name)
        .bind(doc)
        .bind(source_of(&ctx, false))
        .bind(&ctx.user_id)
        .execute(&st.postgres)
        .await?;
    sync_metadata(&st, &ctx, &def).await?;
    let saved = definition_by_id(&st, &ctx.org_id, &id).await?;
    ctx.audit(&st, "portal.entity_updated", &id, &format!("{}:{} (was {}:{})", saved.kind, saved.name, current.kind, current.name)).await;
    Ok(Json(saved))
}

/// Deletes a definition; telemetry entities stay in the catalog, and a service keeps its metadata.
#[utoipa::path(delete, path = "/api/v1/portal/entities/{id}", operation_id = "delete_portal_entity", params(("id" = String, Path)),
    responses((status = 204), (status = 404, body = crate::error::ErrorBody)), tag = "portal")]
pub async fn delete_entity(State(st): State<AppState>, Writer(ctx): Writer, Path(id): Path<String>) -> Result<StatusCode, ApiError> {
    let current = definition_by_id(&st, &ctx.org_id, &id).await?;
    sqlx::query("DELETE FROM portal_entities WHERE org_id = $1 AND id = $2::uuid").bind(&ctx.org_id).bind(&id).execute(&st.postgres).await?;
    ctx.audit(&st, "portal.entity_deleted", &id, &format!("{}:{}", current.kind, current.name)).await;
    Ok(StatusCode::NO_CONTENT)
}

/// Imports entity definition documents (YAML or JSON, apiVersion v3): all are validated first, then
/// each creates or replaces the definition of its kind and name.
#[utoipa::path(post, path = "/api/v1/portal/entities/import", operation_id = "import_portal_entities", request_body = PortalImportRequest,
    responses((status = 200, body = PortalImportResult), (status = 400, body = crate::error::ErrorBody)), tag = "portal")]
pub async fn import_entities(State(st): State<AppState>, Writer(ctx): Writer, Json(req): Json<PortalImportRequest>) -> ApiResult<PortalImportResult> {
    let defs = parse_documents(&req.content).map_err(ApiError::BadRequest)?;
    check_room(&st, &ctx.org_id, defs.len()).await?;
    let mut out = PortalImportResult { created: 0, updated: 0, entities: Vec::new() };
    for def in &defs {
        let (saved, created) = upsert(&st, &ctx, def, source_of(&ctx, true)).await?;
        if created {
            out.created += 1;
        } else {
            out.updated += 1;
        }
        out.entities.push(saved);
    }
    ctx.audit(&st, "portal.entities_imported", "portal_entities", &format!("{} created, {} updated", out.created, out.updated)).await;
    Ok(Json(out))
}
