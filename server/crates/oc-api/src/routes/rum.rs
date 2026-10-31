//! Real User Monitoring (Digital Experience → RUM): applications and the events their SDKs send.
//!
//! - **Applications** (`rum_applications`): a name, a type (browser, ios, android, …) and the public
//!   client token the SDK sends with its events. The same records back Datadog's
//!   `/api/v2/rum/applications` ([`super::datadog_api::rum`], the provider's `datadog_rum_application`).
//! - **Events** (`otel.otel_rum`, written by the intake's `POST /api/v2/rum`): views, actions,
//!   errors, resources, long tasks and vitals with their fields as `LogAttributes` paths. A view is
//!   sent again whenever it changes; its latest `_dd.document_version` is the view. Sessions are
//!   aggregated from their events: first and last event, views, errors, actions, frustrations.
//! - The explorer's query is the RUM search syntax (`@type:error @view.name:/checkout*`,
//!   [`oc_query::RumSchema`]).

use std::collections::BTreeMap;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use oc_query::{build_where, parse_query, Param, RumSchema};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use super::logs::{bind, TimeRange};
use crate::auth::{AuthContext, Writer};
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;

/// Application types, as Datadog names them.
pub const APP_TYPES: [&str; 9] = ["browser", "ios", "android", "react-native", "flutter", "roku", "electron", "unity", "kotlin-multiplatform"];
pub const PROCESSING_STATES: [&str; 3] = ["ALL", "ERROR_FOCUSED_MODE", "NONE"];
pub const RETENTION_STATES: [&str; 2] = ["MAX", "NONE"];
const MAX_APPS: i64 = 500;
/// RUM event types the explorer lists (`session` is aggregated from the others).
pub const EVENT_TYPES: [&str; 7] = ["session", "view", "action", "error", "resource", "long_task", "vital"];

// ================================================================ applications

#[derive(Serialize, ToSchema, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct RumApplication {
    /// The application ID the SDK is initialized with.
    pub id: String,
    pub name: String,
    #[serde(rename = "type")]
    pub kind: String,
    /// Public token the SDK sends events with.
    pub client_token: String,
    pub tags: Vec<String>,
    /// ALL, ERROR_FOCUSED_MODE (only sessions with errors) or NONE (events are dropped).
    pub rum_event_processing_state: String,
    pub rum_event_processing_modified_at: i64,
    /// MAX or NONE.
    pub product_analytics_retention_state: String,
    pub product_analytics_modified_at: i64,
    pub created_by: Option<String>,
    pub created_by_handle: Option<String>,
    pub updated_by: Option<String>,
    pub updated_by_handle: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(sqlx::FromRow)]
struct AppRow {
    id: String,
    name: String,
    kind: String,
    client_token: String,
    tags: Vec<String>,
    processing: String,
    processing_at: i64,
    analytics: String,
    analytics_at: i64,
    created_by: Option<String>,
    created_by_handle: Option<String>,
    updated_by: Option<String>,
    updated_by_handle: Option<String>,
    created_at: i64,
    updated_at: i64,
}

const APP_SELECT: &str = "SELECT a.id::text AS id, a.name, a.type AS kind, a.client_token, a.tags, a.rum_event_processing_state AS processing, \
     (extract(epoch FROM a.rum_event_processing_modified_at) * 1000)::bigint AS processing_at, a.product_analytics_retention_state AS analytics, \
     (extract(epoch FROM a.product_analytics_modified_at) * 1000)::bigint AS analytics_at, c.name AS created_by, c.email AS created_by_handle, \
     u.name AS updated_by, u.email AS updated_by_handle, (extract(epoch FROM a.created_at) * 1000)::bigint AS created_at, \
     (extract(epoch FROM a.updated_at) * 1000)::bigint AS updated_at \
     FROM rum_applications a LEFT JOIN users c ON c.id = a.created_by LEFT JOIN users u ON u.id = a.updated_by";

impl From<AppRow> for RumApplication {
    fn from(r: AppRow) -> Self {
        RumApplication {
            id: r.id,
            name: r.name,
            kind: r.kind,
            client_token: r.client_token,
            tags: r.tags,
            rum_event_processing_state: r.processing,
            rum_event_processing_modified_at: r.processing_at,
            product_analytics_retention_state: r.analytics,
            product_analytics_modified_at: r.analytics_at,
            created_by: r.created_by,
            created_by_handle: r.created_by_handle,
            updated_by: r.updated_by,
            updated_by_handle: r.updated_by_handle,
            created_at: r.created_at,
            updated_at: r.updated_at,
        }
    }
}

/// What creating or changing an application sets; absent fields keep their value (or default).
#[derive(Deserialize, ToSchema, Default, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct RumApplicationInput {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default, rename = "type")]
    pub kind: Option<String>,
    #[serde(default)]
    pub tags: Option<Vec<String>>,
    #[serde(default)]
    pub rum_event_processing_state: Option<String>,
    #[serde(default)]
    pub product_analytics_retention_state: Option<String>,
}

/// A client token as Datadog's look: `pub` and 32 hex digits.
pub fn new_client_token() -> String {
    let mut b = [0u8; 16];
    rand::rngs::OsRng.fill_bytes(&mut b);
    format!("pub{}", b.iter().map(|x| format!("{x:02x}")).collect::<String>())
}

fn is_uuid(s: &str) -> bool {
    s.len() == 36 && s.char_indices().all(|(i, c)| if [8, 13, 18, 23].contains(&i) { c == '-' } else { c.is_ascii_hexdigit() })
}

/// Checks an input's values; `creating` requires a name and a type.
pub fn check_input(input: &mut RumApplicationInput, creating: bool) -> Result<(), String> {
    if let Some(n) = &input.name {
        let n = n.trim();
        if n.is_empty() || n.chars().count() > 200 {
            return Err("The application name is required (at most 200 characters)".into());
        }
        input.name = Some(n.to_string());
    } else if creating {
        return Err("The application name is required".into());
    }
    match &input.kind {
        Some(k) if !APP_TYPES.contains(&k.as_str()) => return Err(format!("Unknown application type '{k}': use one of {}", APP_TYPES.join(", "))),
        None if creating => return Err("The application type is required".into()),
        _ => {}
    }
    if let Some(s) = &input.rum_event_processing_state {
        if !PROCESSING_STATES.contains(&s.as_str()) {
            return Err(format!("rum_event_processing_state is one of {}", PROCESSING_STATES.join(", ")));
        }
    }
    if let Some(s) = &input.product_analytics_retention_state {
        if !RETENTION_STATES.contains(&s.as_str()) {
            return Err(format!("product_analytics_retention_state is one of {}", RETENTION_STATES.join(", ")));
        }
    }
    if let Some(tags) = &mut input.tags {
        tags.retain(|t| !t.trim().is_empty());
        for t in tags.iter_mut() {
            *t = t.trim().to_string();
        }
        tags.dedup();
    }
    Ok(())
}

pub async fn list_apps(st: &AppState, org: &str) -> Result<Vec<RumApplication>, ApiError> {
    let rows: Vec<AppRow> = sqlx::query_as(&format!("{APP_SELECT} WHERE a.org_id = $1 ORDER BY lower(a.name), a.id")).bind(org).fetch_all(&st.postgres).await?;
    Ok(rows.into_iter().map(Into::into).collect())
}

pub async fn find_app(st: &AppState, org: &str, id: &str) -> Result<Option<RumApplication>, ApiError> {
    if !is_uuid(id) {
        return Ok(None);
    }
    let row: Option<AppRow> = sqlx::query_as(&format!("{APP_SELECT} WHERE a.org_id = $1 AND a.id = $2::uuid")).bind(org).bind(id).fetch_optional(&st.postgres).await?;
    Ok(row.map(Into::into))
}

fn not_found() -> ApiError {
    ApiError::NotFound("RUM application not found".into())
}

/// Creates an application with a new client token.
pub async fn insert_app(st: &AppState, ctx: &AuthContext, input: RumApplicationInput) -> Result<RumApplication, ApiError> {
    let (count,): (i64,) = sqlx::query_as("SELECT count(*) FROM rum_applications WHERE org_id = $1").bind(&ctx.org_id).fetch_one(&st.postgres).await?;
    if count >= MAX_APPS {
        return Err(ApiError::BadRequest(format!("An organization can have at most {MAX_APPS} RUM applications")));
    }
    let (id,): (String,) = sqlx::query_as(
        "INSERT INTO rum_applications (org_id, name, type, client_token, tags, rum_event_processing_state, product_analytics_retention_state, created_by, updated_by) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8::uuid, $8::uuid) RETURNING id::text",
    )
    .bind(&ctx.org_id)
    .bind(input.name.as_deref().unwrap_or_default())
    .bind(input.kind.as_deref().unwrap_or("browser"))
    .bind(new_client_token())
    .bind(input.tags.clone().unwrap_or_default())
    .bind(input.rum_event_processing_state.as_deref().unwrap_or("ALL"))
    .bind(input.product_analytics_retention_state.as_deref().unwrap_or("NONE"))
    .bind(&ctx.user_id)
    .fetch_one(&st.postgres)
    .await?;
    ctx.audit(st, "rum.application_created", &id, input.name.as_deref().unwrap_or_default()).await;
    find_app(st, &ctx.org_id, &id).await?.ok_or_else(not_found)
}

/// Changes the given fields; a changed state moves its modification time.
pub async fn change_app(st: &AppState, ctx: &AuthContext, id: &str, input: RumApplicationInput) -> Result<RumApplication, ApiError> {
    if !is_uuid(id) {
        return Err(not_found());
    }
    let done = sqlx::query(
        "UPDATE rum_applications SET name = COALESCE($3, name), type = COALESCE($4, type), tags = COALESCE($5, tags), \
         rum_event_processing_modified_at = CASE WHEN $6::text IS NOT NULL AND $6 <> rum_event_processing_state THEN now() ELSE rum_event_processing_modified_at END, \
         rum_event_processing_state = COALESCE($6, rum_event_processing_state), \
         product_analytics_modified_at = CASE WHEN $7::text IS NOT NULL AND $7 <> product_analytics_retention_state THEN now() ELSE product_analytics_modified_at END, \
         product_analytics_retention_state = COALESCE($7, product_analytics_retention_state), updated_by = $8::uuid, updated_at = now() \
         WHERE org_id = $1 AND id = $2::uuid",
    )
    .bind(&ctx.org_id)
    .bind(id)
    .bind(&input.name)
    .bind(&input.kind)
    .bind(&input.tags)
    .bind(&input.rum_event_processing_state)
    .bind(&input.product_analytics_retention_state)
    .bind(&ctx.user_id)
    .execute(&st.postgres)
    .await?;
    if done.rows_affected() == 0 {
        return Err(not_found());
    }
    ctx.audit(st, "rum.application_updated", id, input.name.as_deref().unwrap_or_default()).await;
    find_app(st, &ctx.org_id, id).await?.ok_or_else(not_found)
}

/// Deletes an application; its events stay until they expire. Its client token stops working.
pub async fn remove_app(st: &AppState, ctx: &AuthContext, id: &str) -> Result<(), ApiError> {
    if !is_uuid(id) {
        return Err(not_found());
    }
    let done = sqlx::query("DELETE FROM rum_applications WHERE org_id = $1 AND id = $2::uuid").bind(&ctx.org_id).bind(id).execute(&st.postgres).await?;
    if done.rows_affected() == 0 {
        return Err(not_found());
    }
    ctx.audit(st, "rum.application_deleted", id, "").await;
    Ok(())
}

#[utoipa::path(get, path = "/api/v1/rum/applications", operation_id = "list_rum_applications", responses((status = 200, body = Vec<RumApplication>)), tag = "rum")]
pub async fn list_applications(State(st): State<AppState>, ctx: AuthContext) -> ApiResult<Vec<RumApplication>> {
    Ok(Json(list_apps(&st, &ctx.org_id).await?))
}

#[utoipa::path(get, path = "/api/v1/rum/applications/{id}", operation_id = "get_rum_application", params(("id" = String, Path)),
    responses((status = 200, body = RumApplication), (status = 404, body = crate::error::ErrorBody)), tag = "rum")]
pub async fn get_application(State(st): State<AppState>, ctx: AuthContext, Path(id): Path<String>) -> ApiResult<RumApplication> {
    Ok(Json(find_app(&st, &ctx.org_id, &id).await?.ok_or_else(not_found)?))
}

#[utoipa::path(post, path = "/api/v1/rum/applications", operation_id = "create_rum_application", request_body = RumApplicationInput,
    responses((status = 200, body = RumApplication), (status = 400, body = crate::error::ErrorBody)), tag = "rum")]
pub async fn create_application(State(st): State<AppState>, Writer(ctx): Writer, Json(mut input): Json<RumApplicationInput>) -> ApiResult<RumApplication> {
    check_input(&mut input, true).map_err(ApiError::BadRequest)?;
    Ok(Json(insert_app(&st, &ctx, input).await?))
}

#[utoipa::path(put, path = "/api/v1/rum/applications/{id}", operation_id = "update_rum_application", params(("id" = String, Path)), request_body = RumApplicationInput,
    responses((status = 200, body = RumApplication), (status = 400, body = crate::error::ErrorBody), (status = 404, body = crate::error::ErrorBody)), tag = "rum")]
pub async fn update_application(State(st): State<AppState>, Writer(ctx): Writer, Path(id): Path<String>, Json(mut input): Json<RumApplicationInput>) -> ApiResult<RumApplication> {
    check_input(&mut input, false).map_err(ApiError::BadRequest)?;
    Ok(Json(change_app(&st, &ctx, &id, input).await?))
}

#[utoipa::path(delete, path = "/api/v1/rum/applications/{id}", operation_id = "delete_rum_application", params(("id" = String, Path)),
    responses((status = 204), (status = 404, body = crate::error::ErrorBody)), tag = "rum")]
pub async fn delete_application(State(st): State<AppState>, Writer(ctx): Writer, Path(id): Path<String>) -> Result<StatusCode, ApiError> {
    remove_app(&st, &ctx, &id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// The demo application of the local generators (`oc-tracegen` sends its browser sessions with this
/// application ID and client token).
pub const DEMO_APPLICATION_ID: &str = "5e1f0000-0000-4000-8000-0000000000a1";
pub const DEMO_CLIENT_TOKEN: &str = "publocaldemoonlydonotuseinproduction";

/// Development only (`OC_SEED_DEMO`, with the demo ingest key): the install's org gets the demo
/// `storefront-web` application, so the generators' RUM events have somewhere to go.
pub async fn ensure_demo_application(st: &AppState) -> anyhow::Result<()> {
    if st.config.demo_ingest_key.is_none() {
        return Ok(());
    }
    let done = sqlx::query(
        "INSERT INTO rum_applications (id, org_id, name, type, client_token, tags) VALUES ($1::uuid, $2, 'storefront-web', 'browser', $3, ARRAY['team:storefront']) \
         ON CONFLICT DO NOTHING",
    )
    .bind(DEMO_APPLICATION_ID)
    .bind(&st.config.dev_org)
    .bind(DEMO_CLIENT_TOKEN)
    .execute(&st.postgres)
    .await?;
    if done.rows_affected() > 0 {
        tracing::info!("created the demo RUM application storefront-web for the install's org");
    }
    Ok(())
}

// ================================================================ events

#[derive(Deserialize, ToSchema, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct RumSearchRequest {
    pub range: TimeRange,
    /// RUM search syntax (`@view.name:/checkout* -@session.type:synthetics`).
    #[serde(default)]
    pub query: String,
    /// session, view, action, error, resource, long_task, vital, or `all` (every event type, not sessions).
    pub event_type: String,
    /// Only this application's events.
    #[serde(default)]
    pub application_id: Option<String>,
    #[serde(default)]
    pub limit: Option<u32>,
    #[serde(default)]
    pub offset: Option<u32>,
    /// Oldest first (default newest first).
    #[serde(default)]
    pub ascending: bool,
    /// Sessions only: conditions on the sessions themselves.
    #[serde(default)]
    pub sessions: Option<SessionFilter>,
}

/// What a session must be (Session Replay's filters); the query picks sessions with a matching event.
#[derive(Deserialize, ToSchema, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct SessionFilter {
    /// Only sessions with a Session Replay recording.
    #[serde(default)]
    pub with_replay: bool,
    #[serde(default)]
    pub min_time_spent_ms: Option<i64>,
    #[serde(default)]
    pub min_action_count: Option<u64>,
    /// In progress (an event in the last 15 minutes) or ended.
    #[serde(default)]
    pub active: Option<bool>,
    /// Only these sessions (a playlist's).
    #[serde(default)]
    pub ids: Vec<String>,
}

/// One event (or session): its time, type and fields.
#[derive(Serialize, ToSchema, Debug)]
#[serde(rename_all = "camelCase")]
pub struct RumEventRow {
    /// view.id, session.id or a hash of the event.
    pub id: String,
    pub timestamp: i64,
    #[serde(rename = "type")]
    pub kind: String,
    pub service: String,
    /// Fields as dotted paths. Sessions carry `session.time_spent` (ms), `session.view.count`,
    /// `session.error.count`, `session.action.count`, `session.frustration.count`,
    /// `session.initial_view.name`, `session.last_view.name` and `session.last_event_at` (ms).
    pub attributes: BTreeMap<String, String>,
}

#[derive(Serialize, ToSchema, Debug)]
#[serde(rename_all = "camelCase")]
pub struct RumBucket {
    pub t: i64,
    pub count: u64,
}

#[derive(Serialize, ToSchema, Debug)]
#[serde(rename_all = "camelCase")]
pub struct RumSearchPage {
    pub rows: Vec<RumEventRow>,
    /// Events (or sessions) matching in the range.
    pub total: u64,
    /// Counts per bucket over the range.
    pub buckets: Vec<RumBucket>,
    pub bucket_ms: i64,
}

/// The events of `req` as a WHERE clause (without the org, which [`AppState::ch`] binds).
struct Scope {
    clause: String,
    params: Vec<Param>,
}

fn scope(req: &RumSearchRequest, kinds: &[&str]) -> Result<Scope, ApiError> {
    req.range.validate()?;
    // Every query adds `OrgId = {oc_org:String}` in front ([`AppState::ch`] binds it).
    let mut clause = "Timestamp >= fromUnixTimestamp64Milli(?) AND Timestamp <= fromUnixTimestamp64Milli(?)".to_string();
    let mut params = vec![Param::Int(req.range.from), Param::Int(req.range.to)];
    if let Some(app) = req.application_id.as_deref().filter(|a| !a.is_empty()) {
        clause.push_str(" AND ApplicationId = ?");
        params.push(Param::Str(app.to_string()));
    }
    if !kinds.is_empty() {
        clause.push_str(&format!(" AND EventType IN ({})", vec!["?"; kinds.len()].join(", ")));
        params.extend(kinds.iter().map(|k| Param::Str(k.to_string())));
    }
    let terms = parse_query(&req.query);
    if !terms.is_empty() {
        let f = build_where(&RumSchema, &terms, None);
        clause.push_str(&format!(" AND ({})", f.clause));
        params.extend(f.params);
    }
    Ok(Scope { clause, params })
}

fn event_type(req: &RumSearchRequest) -> Result<&str, ApiError> {
    let t = req.event_type.as_str();
    if t == "all" || EVENT_TYPES.contains(&t) {
        Ok(t)
    } else {
        Err(ApiError::BadRequest(format!("eventType is one of {} or all", EVENT_TYPES.join(", "))))
    }
}

#[derive(clickhouse::Row, Deserialize)]
struct EventRow {
    id: String,
    ts: i64,
    kind: String,
    service: String,
    attrs: Vec<(String, String)>,
}

#[derive(clickhouse::Row, Deserialize)]
struct SessionAgg {
    id: String,
    ts: i64,
    last: i64,
    views: u64,
    errors: u64,
    actions: u64,
    frustrations: u64,
    initial_view: String,
    last_view: String,
    service: String,
    attrs: Vec<(String, String)>,
}

#[derive(clickhouse::Row, Deserialize)]
struct CountRow {
    n: u64,
}

#[derive(clickhouse::Row, Deserialize)]
struct BucketRow {
    t: i64,
    n: u64,
}

/// The latest version of each view: views are sent again as they change.
const VIEW_VERSION: &str = "toUInt64OrZero(LogAttributes['_dd.document_version'])";
/// Session-level fields taken from a session's first view.
const SESSION_FIELDS: [&str; 18] = [
    "application.id", "session.type", "session.has_replay", "usr.id", "usr.name", "usr.email", "device.type", "device.name", "device.model", "browser.name",
    "browser.version", "os.name", "os.version", "geo.country", "geo.city", "env", "version", "network.client.ip",
];

#[utoipa::path(post, path = "/api/v1/rum/events/search", operation_id = "search_rum_events", request_body = RumSearchRequest,
    responses((status = 200, body = RumSearchPage), (status = 400, body = crate::error::ErrorBody)), tag = "rum")]
pub async fn search_events(State(st): State<AppState>, ctx: AuthContext, Json(req): Json<RumSearchRequest>) -> ApiResult<RumSearchPage> {
    let kind = event_type(&req)?;
    let limit = req.limit.unwrap_or(100).clamp(1, 1000);
    let offset = req.offset.unwrap_or(0).min(10_000);
    let order = if req.ascending { "ASC" } else { "DESC" };
    let size = super::apm::bucket_ms(req.range, 60);
    let ch = st.ch(&ctx.org_id);
    let (rows, total, buckets) = match kind {
        "session" => {
            let (where_sql, having, params) = session_scope(&req)?;
            let fields: Vec<String> = SESSION_FIELDS.iter().map(|f| format!("('{f}', argMin(LogAttributes['{f}'], Timestamp))")).collect();
            let sql = format!(
                "SELECT SessionId AS id, toInt64(toUnixTimestamp64Milli(min(Timestamp))) AS ts, toInt64(toUnixTimestamp64Milli(max(Timestamp))) AS last, \
                 uniqExactIf(ViewId, EventType = 'view') AS views, countIf(EventType = 'error') AS errors, countIf(EventType = 'action') AS actions, \
                 countIf(EventType = 'action' AND LogAttributes['action.frustration.type'] != '') AS frustrations, \
                 argMinIf(LogAttributes['view.name'], Timestamp, EventType = 'view') AS initial_view, argMaxIf(LogAttributes['view.name'], Timestamp, EventType = 'view') AS last_view, \
                 toString(argMin(ServiceName, Timestamp)) AS service, [{}] AS attrs \
                 FROM otel.otel_rum WHERE OrgId = {{oc_org:String}} AND {where_sql} GROUP BY SessionId HAVING {having} ORDER BY ts {order} LIMIT {limit} OFFSET {offset}",
                fields.join(", "),
            );
            let rows: Vec<SessionAgg> = bind(ch.query(&sql), &params).fetch_all().await?;
            let grouped = format!("SELECT SessionId, min(Timestamp) AS start FROM otel.otel_rum WHERE OrgId = {{oc_org:String}} AND {where_sql} GROUP BY SessionId HAVING {having}");
            let n: CountRow = bind(ch.query(&format!("SELECT count() AS n FROM ({grouped})")), &params).fetch_one().await?;
            let b: Vec<BucketRow> =
                bind(ch.query(&format!("SELECT intDiv(toInt64(toUnixTimestamp64Milli(start)), {size}) * {size} AS t, count() AS n FROM ({grouped}) GROUP BY t ORDER BY t")), &params).fetch_all().await?;
            (rows.into_iter().map(session_row).collect(), n.n, b)
        }
        "view" => {
            let s = scope(&req, &["view"])?;
            let sql = format!(
                "SELECT ViewId AS id, toInt64(toUnixTimestamp64Milli(min(Timestamp))) AS ts, 'view' AS kind, toString(any(ServiceName)) AS service, \
                 arrayZip(mapKeys(argMax(LogAttributes, {VIEW_VERSION})), mapValues(argMax(LogAttributes, {VIEW_VERSION}))) AS attrs \
                 FROM otel.otel_rum WHERE OrgId = {{oc_org:String}} AND {} AND ViewId != '' GROUP BY ViewId ORDER BY ts {order} LIMIT {limit} OFFSET {offset}",
                s.clause
            );
            let rows: Vec<EventRow> = bind(ch.query(&sql), &s.params).fetch_all().await?;
            let n: CountRow = bind(ch.query(&format!("SELECT uniqExact(ViewId) AS n FROM otel.otel_rum WHERE OrgId = {{oc_org:String}} AND {}", s.clause)), &s.params).fetch_one().await?;
            let b: Vec<BucketRow> = bind(
                ch.query(&format!(
                    "SELECT intDiv(toInt64(toUnixTimestamp64Milli(Timestamp)), {size}) * {size} AS t, uniqExact(ViewId) AS n FROM otel.otel_rum WHERE OrgId = {{oc_org:String}} AND {} GROUP BY t ORDER BY t",
                    s.clause
                )),
                &s.params,
            )
            .fetch_all()
            .await?;
            (rows.into_iter().map(event_row).collect(), n.n, b)
        }
        other => {
            let kinds: Vec<&str> = if other == "all" { vec![] } else { vec![other] };
            let s = scope(&req, &kinds)?;
            let sql = format!(
                "SELECT toString(cityHash64(Timestamp, Body)) AS id, toInt64(toUnixTimestamp64Milli(Timestamp)) AS ts, toString(EventType) AS kind, toString(ServiceName) AS service, \
                 arrayZip(mapKeys(LogAttributes), mapValues(LogAttributes)) AS attrs FROM otel.otel_rum WHERE OrgId = {{oc_org:String}} AND {} ORDER BY Timestamp {order} LIMIT {limit} OFFSET {offset}",
                s.clause
            );
            let rows: Vec<EventRow> = bind(ch.query(&sql), &s.params).fetch_all().await?;
            let n: CountRow = bind(ch.query(&format!("SELECT count() AS n FROM otel.otel_rum WHERE OrgId = {{oc_org:String}} AND {}", s.clause)), &s.params).fetch_one().await?;
            let b: Vec<BucketRow> = bind(
                ch.query(&format!(
                    "SELECT intDiv(toInt64(toUnixTimestamp64Milli(Timestamp)), {size}) * {size} AS t, count() AS n FROM otel.otel_rum WHERE OrgId = {{oc_org:String}} AND {} GROUP BY t ORDER BY t",
                    s.clause
                )),
                &s.params,
            )
            .fetch_all()
            .await?;
            (rows.into_iter().map(event_row).collect(), n.n, b)
        }
    };
    Ok(Json(RumSearchPage { rows, total, buckets: buckets.into_iter().map(|b| RumBucket { t: b.t, count: b.n }).collect(), bucket_ms: size }))
}

/// Sessions: the WHERE clause (their events in the range and application; with a query, the sessions
/// having a matching event), the HAVING clause of the session filters, and their parameters in order.
fn session_scope(req: &RumSearchRequest) -> Result<(String, String, Vec<Param>), ApiError> {
    let base = scope(&RumSearchRequest { query: String::new(), ..req.clone() }, &[])?;
    let mut where_sql = format!("{} AND SessionId != ''", base.clause);
    let mut params = base.params;
    if !req.query.trim().is_empty() {
        let full = scope(req, &[])?;
        where_sql.push_str(&format!(" AND SessionId IN (SELECT SessionId FROM otel.otel_rum WHERE OrgId = {{oc_org:String}} AND {})", full.clause));
        params.extend(full.params);
    }
    let f = req.sessions.clone().unwrap_or_default();
    if f.with_replay {
        where_sql.push_str(" AND SessionId IN (SELECT SessionId FROM otel.otel_rum_replay WHERE OrgId = {oc_org:String} AND Timestamp >= fromUnixTimestamp64Milli(?) AND Timestamp <= fromUnixTimestamp64Milli(?))");
        params.extend([Param::Int(req.range.from), Param::Int(req.range.to)]);
    }
    if !f.ids.is_empty() {
        where_sql.push_str(&format!(" AND SessionId IN ({})", vec!["?"; f.ids.len().min(1000)].join(", ")));
        params.extend(f.ids.iter().take(1000).map(|id| Param::Str(id.clone())));
    }
    let mut having = vec!["1".to_string()];
    if let Some(ms) = f.min_time_spent_ms {
        having.push("dateDiff('millisecond', min(Timestamp), max(Timestamp)) >= ?".into());
        params.push(Param::Int(ms));
    }
    if let Some(n) = f.min_action_count {
        having.push("countIf(EventType = 'action') >= ?".into());
        params.push(Param::Int(n as i64));
    }
    match f.active {
        Some(true) => having.push("max(Timestamp) >= now64(3) - INTERVAL 15 MINUTE".into()),
        Some(false) => having.push("max(Timestamp) < now64(3) - INTERVAL 15 MINUTE".into()),
        None => {}
    }
    Ok((where_sql, having.join(" AND "), params))
}

fn event_row(r: EventRow) -> RumEventRow {
    RumEventRow { id: r.id, timestamp: r.ts, kind: r.kind, service: r.service, attributes: r.attrs.into_iter().filter(|(_, v)| !v.is_empty()).collect() }
}

fn session_row(r: SessionAgg) -> RumEventRow {
    let mut attributes: BTreeMap<String, String> = r.attrs.into_iter().filter(|(_, v)| !v.is_empty()).collect();
    attributes.insert("session.id".into(), r.id.clone());
    for (k, v) in [
        ("session.time_spent", (r.last - r.ts).max(0).to_string()),
        ("session.view.count", r.views.to_string()),
        ("session.error.count", r.errors.to_string()),
        ("session.action.count", r.actions.to_string()),
        ("session.frustration.count", r.frustrations.to_string()),
        ("session.initial_view.name", r.initial_view),
        ("session.last_view.name", r.last_view),
        ("session.last_event_at", r.last.to_string()),
    ] {
        attributes.insert(k.into(), v);
    }
    RumEventRow { id: r.id, timestamp: r.ts, kind: "session".into(), service: r.service, attributes }
}

// ================================================================ facets

/// The facets of an event type: common ones, then the type's own.
pub fn facet_paths(kind: &str) -> Vec<&'static str> {
    let mut out = vec![
        "application.id", "service", "env", "version", "session.type", "usr.name", "usr.id", "usr.email", "device.type", "device.name", "device.model", "browser.name", "os.name",
        "geo.country", "geo.city", "view.name", "view.url_path",
    ];
    out.extend(match kind {
        "error" => vec!["error.source", "error.type", "error.handling", "error.is_crash"],
        "action" => vec!["action.type", "action.target.name", "action.frustration.type"],
        "resource" => vec!["resource.type", "resource.method", "resource.status_code", "resource.url_host"],
        "long_task" => vec!["long_task.entry_type"],
        "vital" => vec!["vital.name", "vital.type"],
        _ => vec![],
    });
    out
}

#[derive(Serialize, ToSchema, Debug)]
#[serde(rename_all = "camelCase")]
pub struct RumFacetValue {
    pub value: String,
    pub count: u64,
}

#[derive(Serialize, ToSchema, Debug)]
#[serde(rename_all = "camelCase")]
pub struct RumFacet {
    /// The field's path (`browser.name`).
    pub path: String,
    pub values: Vec<RumFacetValue>,
}

#[derive(clickhouse::Row, Deserialize)]
struct FacetRow {
    k: String,
    v: String,
    n: u64,
}

/// The top values of the event type's facets among the matching events (sessions: among their events).
#[utoipa::path(post, path = "/api/v1/rum/facets", operation_id = "rum_facets", request_body = RumSearchRequest,
    responses((status = 200, body = Vec<RumFacet>), (status = 400, body = crate::error::ErrorBody)), tag = "rum")]
pub async fn facets(State(st): State<AppState>, ctx: AuthContext, Json(req): Json<RumSearchRequest>) -> ApiResult<Vec<RumFacet>> {
    let kind = event_type(&req)?;
    let kinds: Vec<&str> = match kind {
        "all" | "session" => vec![],
        k => vec![k],
    };
    let s = scope(&req, &kinds)?;
    let paths = facet_paths(kind);
    let pairs = vec!["(?, LogAttributes[?])"; paths.len()].join(", ");
    let mut params: Vec<Param> = paths.iter().flat_map(|p| [Param::Str(p.to_string()), Param::Str(p.to_string())]).collect();
    params.extend(s.params);
    // `service`, `env` and `version` live in the resource too; the intake copies them into the event.
    // Sessions count each session once; other types count their events.
    let (session, n) = if kind == "session" { (", SessionId AS sid", "uniqExact(sid)") } else { ("", "count()") };
    let sql = format!(
        "SELECT kv.1 AS k, kv.2 AS v, {n} AS n FROM (SELECT arrayJoin(arrayFilter(x -> x.2 != '', [{pairs}])) AS kv{session} FROM otel.otel_rum WHERE OrgId = {{oc_org:String}} AND {}) \
         GROUP BY k, v ORDER BY n DESC LIMIT 20 BY k",
        s.clause
    );
    let rows: Vec<FacetRow> = bind(st.ch(&ctx.org_id).query(&sql), &params).fetch_all().await?;
    let mut by: BTreeMap<String, Vec<RumFacetValue>> = BTreeMap::new();
    for r in rows {
        by.entry(r.k).or_default().push(RumFacetValue { value: r.v, count: r.n });
    }
    Ok(Json(paths.into_iter().map(|p| RumFacet { path: p.to_string(), values: by.remove(p).unwrap_or_default() }).collect()))
}

// ================================================================ summary

#[derive(Deserialize, ToSchema, Debug)]
#[serde(rename_all = "camelCase")]
pub struct RumSummaryRequest {
    pub range: TimeRange,
    pub application_id: String,
}

#[derive(Serialize, ToSchema, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct RumNamedCount {
    pub name: String,
    pub count: u64,
    /// Errors per 100 views (top views by error rate), or sessions (deployments).
    pub value: f64,
}

/// An application's health over a range: Core Web Vitals, errors, deployments and usage.
#[derive(Serialize, ToSchema, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct RumSummary {
    pub sessions: u64,
    pub views: u64,
    pub errors: u64,
    /// Errors per minute of the range.
    pub error_rate: f64,
    /// Sessions with at least one error, percent.
    pub crash_free_sessions: Option<f64>,
    /// 75th percentiles, milliseconds (CLS has no unit).
    pub loading_time_p75: Option<f64>,
    pub largest_contentful_paint_p75: Option<f64>,
    pub interaction_to_next_paint_p75: Option<f64>,
    pub cumulative_layout_shift_p75: Option<f64>,
    pub top_error_views: Vec<RumNamedCount>,
    pub versions: Vec<RumNamedCount>,
    pub top_views: Vec<RumNamedCount>,
    pub sessions_over_time: Vec<RumBucket>,
    pub bucket_ms: i64,
}

#[derive(clickhouse::Row, Deserialize)]
struct Totals {
    sessions: u64,
    views: u64,
    errors: u64,
    sessions_with_errors: u64,
}

#[derive(clickhouse::Row, Deserialize)]
struct Vitals {
    n: u64,
    loading: f64,
    lcp: f64,
    inp: f64,
    cls: f64,
}

#[derive(clickhouse::Row, Deserialize)]
struct Named {
    name: String,
    n: u64,
    v: f64,
}

#[utoipa::path(post, path = "/api/v1/rum/summary", operation_id = "rum_summary", request_body = RumSummaryRequest,
    responses((status = 200, body = RumSummary), (status = 400, body = crate::error::ErrorBody)), tag = "rum")]
pub async fn summary(State(st): State<AppState>, ctx: AuthContext, Json(req): Json<RumSummaryRequest>) -> ApiResult<RumSummary> {
    let base = RumSearchRequest { range: req.range, query: String::new(), event_type: "all".into(), application_id: Some(req.application_id.clone()), limit: None, offset: None, ascending: false, sessions: None };
    let s = scope(&base, &[])?;
    let ch = st.ch(&ctx.org_id);
    let t: Totals = bind(
        ch.query(&format!(
            "SELECT uniqExact(SessionId) AS sessions, uniqExactIf(ViewId, EventType = 'view') AS views, countIf(EventType = 'error') AS errors, \
             uniqExactIf(SessionId, EventType = 'error') AS sessions_with_errors FROM otel.otel_rum WHERE OrgId = {{oc_org:String}} AND {}",
            s.clause
        )),
        &s.params,
    )
    .fetch_one()
    .await?;
    // Timings are nanoseconds in the events; the latest version of each view counts.
    let ns = |f: &str| format!("quantileIf(0.75)(toFloat64OrZero(a['{f}']) / 1e6, a['{f}'] != '')");
    let v: Vitals = bind(
        ch.query(&format!(
            "SELECT count() AS n, {} AS loading, {} AS lcp, {} AS inp, quantileIf(0.75)(toFloat64OrZero(a['view.cumulative_layout_shift']), a['view.cumulative_layout_shift'] != '') AS cls \
             FROM (SELECT argMax(LogAttributes, {VIEW_VERSION}) AS a FROM otel.otel_rum WHERE OrgId = {{oc_org:String}} AND {} AND EventType = 'view' GROUP BY ViewId)",
            ns("view.loading_time"),
            ns("view.largest_contentful_paint"),
            ns("view.interaction_to_next_paint"),
            s.clause
        )),
        &s.params,
    )
    .fetch_one()
    .await?;
    let named = |sql: String| {
        let q = bind(ch.query(&sql), &s.params);
        async move { q.fetch_all::<Named>().await }
    };
    let error_views = named(format!(
        "SELECT LogAttributes['view.name'] AS name, countIf(EventType = 'error') AS n, if(uniqExactIf(ViewId, EventType = 'view') = 0, 0, 100 * n / uniqExactIf(ViewId, EventType = 'view')) AS v \
         FROM otel.otel_rum WHERE OrgId = {{oc_org:String}} AND {} AND LogAttributes['view.name'] != '' GROUP BY name HAVING n > 0 ORDER BY v DESC, n DESC LIMIT 10",
        s.clause
    ))
    .await?;
    let versions = named(format!(
        "SELECT ResourceAttributes['service.version'] AS name, uniqExact(SessionId) AS n, toFloat64(n) AS v FROM otel.otel_rum WHERE OrgId = {{oc_org:String}} AND {} AND name != '' GROUP BY name ORDER BY n DESC LIMIT 10",
        s.clause
    ))
    .await?;
    let top_views = named(format!(
        "SELECT LogAttributes['view.name'] AS name, uniqExact(ViewId) AS n, toFloat64(n) AS v FROM otel.otel_rum WHERE OrgId = {{oc_org:String}} AND {} AND EventType = 'view' AND name != '' GROUP BY name ORDER BY n DESC LIMIT 10",
        s.clause
    ))
    .await?;
    let size = super::apm::bucket_ms(req.range, 60);
    let series: Vec<BucketRow> = bind(
        ch.query(&format!(
            "SELECT intDiv(toInt64(toUnixTimestamp64Milli(Timestamp)), {size}) * {size} AS t, uniqExact(SessionId) AS n FROM otel.otel_rum WHERE OrgId = {{oc_org:String}} AND {} GROUP BY t ORDER BY t",
            s.clause
        )),
        &s.params,
    )
    .fetch_all()
    .await?;
    let minutes = ((req.range.to - req.range.from) as f64 / 60_000.0).max(1.0);
    let present = |x: f64| (v.n > 0 && x.is_finite() && x > 0.0).then_some(x);
    let to_named = |rows: Vec<Named>| rows.into_iter().map(|r| RumNamedCount { name: r.name, count: r.n, value: r.v }).collect();
    Ok(Json(RumSummary {
        sessions: t.sessions,
        views: t.views,
        errors: t.errors,
        error_rate: t.errors as f64 / minutes,
        crash_free_sessions: (t.sessions > 0).then(|| 100.0 * (t.sessions - t.sessions_with_errors.min(t.sessions)) as f64 / t.sessions as f64),
        loading_time_p75: present(v.loading),
        largest_contentful_paint_p75: present(v.lcp),
        interaction_to_next_paint_p75: present(v.inp),
        cumulative_layout_shift_p75: (v.n > 0 && v.cls.is_finite()).then_some(v.cls),
        top_error_views: to_named(error_views),
        versions: to_named(versions),
        top_views: to_named(top_views),
        sessions_over_time: series.into_iter().map(|b| RumBucket { t: b.t, count: b.n }).collect(),
        bucket_ms: size,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_tokens_look_like_datadogs() {
        let t = new_client_token();
        assert!(t.starts_with("pub") && t.len() == 35 && t[3..].chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(t, new_client_token());
    }

    #[test]
    fn inputs_are_checked() {
        let mut i = RumApplicationInput { name: Some("  storefront-web ".into()), kind: Some("browser".into()), tags: Some(vec![" team:web ".into(), "".into()]), ..Default::default() };
        check_input(&mut i, true).unwrap();
        assert_eq!(i.name.as_deref(), Some("storefront-web"));
        assert_eq!(i.tags, Some(vec!["team:web".to_string()]));
        assert!(check_input(&mut RumApplicationInput { name: Some("x".into()), ..Default::default() }, true).unwrap_err().contains("type"));
        assert!(check_input(&mut RumApplicationInput { kind: Some("wrong_android".into()), ..Default::default() }, false).unwrap_err().contains("Unknown application type"));
        assert!(check_input(&mut RumApplicationInput { rum_event_processing_state: Some("SOME".into()), ..Default::default() }, false).is_err());
        check_input(&mut RumApplicationInput::default(), false).unwrap();
    }

    #[test]
    fn the_scope_binds_the_range_application_types_and_query() {
        let req = RumSearchRequest {
            range: TimeRange { from: 1, to: 2 },
            query: "@view.name:/checkout*".into(),
            event_type: "error".into(),
            application_id: Some("app-1".into()),
            limit: None,
            offset: None,
            ascending: false,
            sessions: None,
        };
        let s = scope(&req, &["error"]).unwrap();
        assert!(s.clause.starts_with("Timestamp >= "), "{}", s.clause);
        assert!(s.clause.contains("ApplicationId = ?") && s.clause.contains("EventType IN (?)") && s.clause.contains("LogAttributes[?] ILIKE ?"));
        assert_eq!(s.params.len(), 2 + 1 + 1 + 2);
        assert!(event_type(&RumSearchRequest { event_type: "sessions".into(), ..req.clone() }).is_err());
    }

    /// Against the live databases: sessions, views (latest version), events, facets and the summary.
    #[tokio::test]
    async fn events_are_searched_aggregated_and_summarized() {
        let Some((_, states)) = crate::test_support::replicas(1).await else { return };
        let st = states.into_iter().next().unwrap();
        let ctx = crate::test_support::admin_of_new_org(&st, "rum").await;
        let now = crate::routes::metrics::now_ms();
        let app = "11111111-2222-4333-8444-555555555555";
        // (offset ms, type, extra attributes): one session, a view sent twice, an action, an error.
        type Event<'a> = (i64, &'a str, &'a [(&'a str, &'a str)]);
        let events: [Event; 5] = [
            (-60_000, "view", &[("view.id", "v1"), ("view.name", "/cart"), ("_dd.document_version", "1"), ("view.loading_time", "900000000")]),
            (-50_000, "view", &[("view.id", "v1"), ("view.name", "/cart"), ("_dd.document_version", "2"), ("view.loading_time", "1200000000"), ("view.largest_contentful_paint", "800000000")]),
            (-40_000, "action", &[("view.id", "v1"), ("view.name", "/cart"), ("action.type", "click"), ("action.frustration.type", "rage_click")]),
            (-30_000, "error", &[("view.id", "v1"), ("view.name", "/cart"), ("error.message", "Cannot read properties of undefined"), ("error.source", "source")]),
            (-20_000, "view", &[("view.id", "v2"), ("view.name", "/checkout"), ("_dd.document_version", "1")]),
        ];
        for (offset, kind, extra) in events {
            let mut attrs: Vec<(String, String)> = vec![("type".into(), kind.into()), ("application.id".into(), app.into()), ("session.id".into(), "s1".into()), ("session.type".into(), "user".into()), ("browser.name".into(), "Firefox".into())];
            attrs.extend(extra.iter().map(|(k, v)| (k.to_string(), v.to_string())));
            let mut sql = "INSERT INTO otel.otel_rum (Timestamp, ServiceName, Body, ResourceAttributes, LogAttributes) VALUES (fromUnixTimestamp64Milli(?), 'storefront-web', '{}', map('oc.org_id', ?, 'service.version', '1.4.2'), map(".to_string();
            sql.push_str(&vec!["?, ?"; attrs.len()].join(", "));
            sql.push_str("))");
            let mut q = st.clickhouse.query(&sql).bind(now + offset).bind(&ctx.org_id);
            for (k, v) in &attrs {
                q = q.bind(k).bind(v);
            }
            q.execute().await.unwrap();
        }
        let req = |kind: &str, query: &str| RumSearchRequest {
            range: TimeRange { from: now - 3_600_000, to: now },
            query: query.into(),
            event_type: kind.into(),
            application_id: Some(app.into()),
            limit: None,
            offset: None,
            ascending: false,
            sessions: None,
        };
        let sessions = search_events(State(st.clone()), ctx.clone(), Json(req("session", ""))).await.unwrap().0;
        assert_eq!(sessions.total, 1);
        let a = &sessions.rows[0].attributes;
        assert_eq!((a["session.view.count"].as_str(), a["session.error.count"].as_str(), a["session.action.count"].as_str(), a["session.frustration.count"].as_str()), ("2", "1", "1", "1"));
        assert_eq!((a["session.initial_view.name"].as_str(), a["session.last_view.name"].as_str(), a["session.time_spent"].as_str()), ("/cart", "/checkout", "40000"));
        assert_eq!(a["browser.name"], "Firefox");
        // A session found by one of its events is summed up from all of them.
        let found = search_events(State(st.clone()), ctx.clone(), Json(req("session", "@view.name:/checkout"))).await.unwrap().0;
        assert_eq!((found.total, found.rows[0].attributes["session.view.count"].as_str()), (1, "2"));
        assert_eq!(search_events(State(st.clone()), ctx.clone(), Json(req("session", "@view.name:/nowhere"))).await.unwrap().0.total, 0);
        let views = search_events(State(st.clone()), ctx.clone(), Json(req("view", ""))).await.unwrap().0;
        assert_eq!(views.total, 2);
        let cart = views.rows.iter().find(|r| r.id == "v1").unwrap();
        assert_eq!(cart.attributes["_dd.document_version"], "2", "the latest version of a view");
        let errors = search_events(State(st.clone()), ctx.clone(), Json(req("error", "@error.source:source"))).await.unwrap().0;
        assert_eq!((errors.total, errors.rows[0].service.as_str()), (1, "storefront-web"));
        assert_eq!(search_events(State(st.clone()), ctx.clone(), Json(req("all", "-@type:view"))).await.unwrap().0.total, 2);
        assert_eq!(search_events(State(st.clone()), ctx.clone(), Json(req("error", "@view.name:/checkout"))).await.unwrap().0.total, 0);
        let f = facets(State(st.clone()), ctx.clone(), Json(req("view", ""))).await.unwrap().0;
        let names = f.iter().find(|x| x.path == "view.name").unwrap();
        assert_eq!(names.values.len(), 2);
        let per_session = facets(State(st.clone()), ctx.clone(), Json(req("session", ""))).await.unwrap().0;
        let browsers = per_session.iter().find(|x| x.path == "browser.name").unwrap();
        assert_eq!((browsers.values[0].value.as_str(), browsers.values[0].count), ("Firefox", 1), "one session, not five events");
        let s = summary(State(st.clone()), ctx.clone(), Json(RumSummaryRequest { range: TimeRange { from: now - 3_600_000, to: now }, application_id: app.into() })).await.unwrap().0;
        assert_eq!((s.sessions, s.views, s.errors), (1, 2, 1));
        assert_eq!(s.largest_contentful_paint_p75, Some(800.0));
        assert_eq!(s.versions[0].name, "1.4.2");
        assert_eq!(s.top_error_views[0].name, "/cart");
        // Another org sees nothing.
        let other = crate::test_support::admin_of_new_org(&st, "rum-other").await;
        assert_eq!(search_events(State(st.clone()), other, Json(req("all", ""))).await.unwrap().0.total, 0);
    }

    #[test]
    fn facets_follow_the_event_type() {
        assert!(facet_paths("error").contains(&"error.source"));
        assert!(!facet_paths("view").contains(&"error.source"));
        assert!(facet_paths("resource").contains(&"resource.status_code"));
    }
}
