//! Session Replay (Digital Experience → Session Replay): the recordings of browser sessions, playlists
//! of them, who watched which, and heatmaps of where users click.
//!
//! - **Recordings** are the segments the intake stores in `otel.otel_rum_replay` (DOM snapshots and
//!   incremental records of Datadog's browser SDK, played back by the UI). A session's replay is its
//!   segments in order; finding sessions is the RUM session search with `withReplay`
//!   ([`super::rum::search_events`]).
//! - **Playlists** (`rum_replay_playlists`) group sessions by id; **watches** (`rum_replay_watches`)
//!   record who opened a replay (My Watch History, the "watched" count).
//! - **Heatmaps**: for a view name, the latest full snapshot of that view, the click positions of its
//!   recordings, and its most clicked elements (RUM actions).

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;

use super::logs::TimeRange;
use super::rum::{search_events, RumEventRow, RumSearchRequest, SessionFilter};
use crate::auth::{AuthContext, Writer};
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;

const DAY: i64 = 86_400_000;
/// How far back a session's recording is looked up around its start.
const LOOKUP: i64 = DAY;
const MAX_SEGMENTS: u32 = 2000;
/// Recordings read for a heatmap.
const HEATMAP_SEGMENTS: u32 = 300;

// ================================================================ recordings

#[derive(Serialize, ToSchema, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ReplaySegment {
    pub view_id: String,
    /// Epoch ms.
    pub start: i64,
    pub end: i64,
    /// The SDK's records (full snapshots, incremental snapshots, meta, focus, view end, …).
    #[schema(value_type = Vec<Object>)]
    pub records: Vec<Value>,
}

#[derive(Serialize, ToSchema, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SessionReplay {
    /// The session as the RUM explorer shows it (absent when its events expired).
    pub session: Option<RumEventRow>,
    pub segments: Vec<ReplaySegment>,
    /// Its views, actions and errors, oldest first.
    pub events: Vec<RumEventRow>,
    /// People who watched it.
    pub watched: i64,
}

#[derive(Deserialize, ToSchema, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SessionReplayRequest {
    /// When the session started (epoch ms; around it the recording is looked up).
    pub started_at: i64,
}

#[derive(clickhouse::Row, Deserialize)]
struct SegmentRow {
    view_id: String,
    start: i64,
    end: i64,
    body: String,
}

fn records_of(body: &str) -> Vec<Value> {
    serde_json::from_str::<Value>(body).ok().and_then(|v| v.get("records").and_then(Value::as_array).cloned()).unwrap_or_default()
}

/// The recording of one session, its RUM summary and events.
#[utoipa::path(post, path = "/api/v1/rum/replay/sessions/{id}", operation_id = "get_session_replay", params(("id" = String, Path)), request_body = SessionReplayRequest,
    responses((status = 200, body = SessionReplay), (status = 404, body = crate::error::ErrorBody)), tag = "rum")]
pub async fn session_replay(State(st): State<AppState>, ctx: AuthContext, Path(id): Path<String>, Json(req): Json<SessionReplayRequest>) -> ApiResult<SessionReplay> {
    let range = TimeRange { from: req.started_at - LOOKUP, to: req.started_at + LOOKUP };
    let rows: Vec<SegmentRow> = st
        .ch(&ctx.org_id)
        .query(&format!(
            "SELECT ViewId AS view_id, toInt64OrZero(LogAttributes['replay.start']) AS start, toInt64OrZero(LogAttributes['replay.end']) AS end, Body AS body \
             FROM otel.otel_rum_replay WHERE OrgId = {{oc_org:String}} AND SessionId = ? AND Timestamp >= fromUnixTimestamp64Milli(?) AND Timestamp <= fromUnixTimestamp64Milli(?) \
             ORDER BY Timestamp, toInt64OrZero(LogAttributes['replay.index_in_view']) LIMIT {MAX_SEGMENTS}"
        ))
        .bind(&id)
        .bind(range.from)
        .bind(range.to)
        .fetch_all()
        .await?;
    if rows.is_empty() {
        return Err(ApiError::NotFound("No recording of this session".into()));
    }
    let search = |event_type: &str, limit: u32| RumSearchRequest {
        range,
        query: String::new(),
        event_type: event_type.into(),
        application_id: None,
        limit: Some(limit),
        offset: None,
        ascending: true,
        sessions: Some(SessionFilter { ids: vec![id.clone()], ..Default::default() }),
    };
    let session = search_events(State(st.clone()), ctx.clone(), Json(search("session", 1))).await?.0.rows.into_iter().next();
    let events = search_events(State(st.clone()), ctx.clone(), Json(RumSearchRequest { query: format!("@session.id:{id} -@type:resource -@type:long_task"), sessions: None, ..search("all", 1000) }))
        .await?
        .0
        .rows;
    let (watched,): (i64,) = sqlx::query_as("SELECT count(*) FROM rum_replay_watches WHERE org_id = $1 AND session_id = $2").bind(&ctx.org_id).bind(&id).fetch_one(&st.postgres).await?;
    Ok(Json(SessionReplay {
        session,
        segments: rows.into_iter().map(|r| ReplaySegment { view_id: r.view_id, start: r.start, end: r.end, records: records_of(&r.body) }).collect(),
        events,
        watched,
    }))
}

/// Records that the caller watched a replay.
#[utoipa::path(put, path = "/api/v1/rum/replay/sessions/{id}/watched", operation_id = "watch_session_replay", params(("id" = String, Path)), request_body = SessionReplayRequest,
    responses((status = 204)), tag = "rum")]
pub async fn watched(State(st): State<AppState>, ctx: AuthContext, Path(id): Path<String>, Json(req): Json<SessionReplayRequest>) -> Result<StatusCode, ApiError> {
    sqlx::query(
        "INSERT INTO rum_replay_watches (org_id, user_id, session_id, started_at) VALUES ($1, $2::uuid, $3, $4) \
         ON CONFLICT (org_id, user_id, session_id) DO UPDATE SET watched_at = now()",
    )
    .bind(&ctx.org_id)
    .bind(&ctx.user_id)
    .bind(&id)
    .bind(req.started_at)
    .execute(&st.postgres)
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Serialize, ToSchema, sqlx::FromRow, Debug)]
#[serde(rename_all = "camelCase")]
pub struct WatchedSession {
    pub session_id: String,
    pub started_at: i64,
    pub watched_at: i64,
}

/// The caller's watch history, newest first.
#[utoipa::path(get, path = "/api/v1/rum/replay/history", operation_id = "session_replay_history", responses((status = 200, body = Vec<WatchedSession>)), tag = "rum")]
pub async fn history(State(st): State<AppState>, ctx: AuthContext) -> ApiResult<Vec<WatchedSession>> {
    let rows: Vec<WatchedSession> = sqlx::query_as(
        "SELECT session_id, started_at, (extract(epoch FROM watched_at) * 1000)::bigint AS watched_at FROM rum_replay_watches \
         WHERE org_id = $1 AND user_id = $2::uuid ORDER BY watched_at DESC LIMIT 200",
    )
    .bind(&ctx.org_id)
    .bind(&ctx.user_id)
    .fetch_all(&st.postgres)
    .await?;
    Ok(Json(rows))
}

// ================================================================ playlists

#[derive(Serialize, ToSchema, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ReplayPlaylist {
    pub id: String,
    pub name: String,
    pub description: String,
    pub session_count: i64,
    pub created_by: Option<String>,
    pub created_by_me: bool,
    pub created_at: i64,
    pub updated_at: i64,
    /// The playlist's sessions (on a single playlist): id and start.
    pub sessions: Vec<PlaylistSession>,
}

#[derive(Serialize, Deserialize, ToSchema, sqlx::FromRow, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct PlaylistSession {
    pub session_id: String,
    pub started_at: i64,
}

#[derive(Deserialize, ToSchema, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ReplayPlaylistInput {
    pub name: String,
    #[serde(default)]
    pub description: String,
}

#[derive(sqlx::FromRow)]
struct PlaylistRow {
    id: String,
    name: String,
    description: String,
    session_count: i64,
    created_by: Option<String>,
    created_by_id: Option<String>,
    created_at: i64,
    updated_at: i64,
}

const PLAYLIST_SELECT: &str = "SELECT p.id::text AS id, p.name, p.description, (SELECT count(*) FROM rum_replay_playlist_sessions s WHERE s.playlist_id = p.id) AS session_count, \
     u.name AS created_by, p.created_by::text AS created_by_id, (extract(epoch FROM p.created_at) * 1000)::bigint AS created_at, (extract(epoch FROM p.updated_at) * 1000)::bigint AS updated_at \
     FROM rum_replay_playlists p LEFT JOIN users u ON u.id = p.created_by";

fn playlist(r: PlaylistRow, me: &str, sessions: Vec<PlaylistSession>) -> ReplayPlaylist {
    ReplayPlaylist {
        created_by_me: r.created_by_id.as_deref() == Some(me),
        id: r.id,
        name: r.name,
        description: r.description,
        session_count: r.session_count,
        created_by: r.created_by,
        created_at: r.created_at,
        updated_at: r.updated_at,
        sessions,
    }
}

fn is_uuid(s: &str) -> bool {
    s.len() == 36 && s.char_indices().all(|(i, c)| if [8, 13, 18, 23].contains(&i) { c == '-' } else { c.is_ascii_hexdigit() })
}

fn not_found() -> ApiError {
    ApiError::NotFound("Playlist not found".into())
}

fn check_name(input: &ReplayPlaylistInput) -> Result<String, ApiError> {
    let name = input.name.trim();
    if name.is_empty() || name.chars().count() > 200 {
        return Err(ApiError::BadRequest("A playlist needs a name (at most 200 characters)".into()));
    }
    Ok(name.to_string())
}

async fn load_playlist(st: &AppState, ctx: &AuthContext, id: &str) -> Result<ReplayPlaylist, ApiError> {
    if !is_uuid(id) {
        return Err(not_found());
    }
    let row: PlaylistRow = sqlx::query_as(&format!("{PLAYLIST_SELECT} WHERE p.org_id = $1 AND p.id = $2::uuid")).bind(&ctx.org_id).bind(id).fetch_optional(&st.postgres).await?.ok_or_else(not_found)?;
    let sessions: Vec<PlaylistSession> = sqlx::query_as("SELECT session_id, started_at FROM rum_replay_playlist_sessions WHERE org_id = $1 AND playlist_id = $2::uuid ORDER BY added_at DESC")
        .bind(&ctx.org_id)
        .bind(id)
        .fetch_all(&st.postgres)
        .await?;
    Ok(playlist(row, &ctx.user_id, sessions))
}

#[utoipa::path(get, path = "/api/v1/rum/replay/playlists", operation_id = "list_replay_playlists", responses((status = 200, body = Vec<ReplayPlaylist>)), tag = "rum")]
pub async fn list_playlists(State(st): State<AppState>, ctx: AuthContext) -> ApiResult<Vec<ReplayPlaylist>> {
    let rows: Vec<PlaylistRow> = sqlx::query_as(&format!("{PLAYLIST_SELECT} WHERE p.org_id = $1 ORDER BY p.updated_at DESC")).bind(&ctx.org_id).fetch_all(&st.postgres).await?;
    Ok(Json(rows.into_iter().map(|r| playlist(r, &ctx.user_id, vec![])).collect()))
}

#[utoipa::path(get, path = "/api/v1/rum/replay/playlists/{id}", operation_id = "get_replay_playlist", params(("id" = String, Path)),
    responses((status = 200, body = ReplayPlaylist), (status = 404, body = crate::error::ErrorBody)), tag = "rum")]
pub async fn get_playlist(State(st): State<AppState>, ctx: AuthContext, Path(id): Path<String>) -> ApiResult<ReplayPlaylist> {
    Ok(Json(load_playlist(&st, &ctx, &id).await?))
}

#[utoipa::path(post, path = "/api/v1/rum/replay/playlists", operation_id = "create_replay_playlist", request_body = ReplayPlaylistInput,
    responses((status = 200, body = ReplayPlaylist), (status = 400, body = crate::error::ErrorBody)), tag = "rum")]
pub async fn create_playlist(State(st): State<AppState>, Writer(ctx): Writer, Json(input): Json<ReplayPlaylistInput>) -> ApiResult<ReplayPlaylist> {
    let name = check_name(&input)?;
    let (id,): (String,) = sqlx::query_as("INSERT INTO rum_replay_playlists (org_id, name, description, created_by) VALUES ($1, $2, $3, $4::uuid) RETURNING id::text")
        .bind(&ctx.org_id)
        .bind(&name)
        .bind(input.description.trim())
        .bind(&ctx.user_id)
        .fetch_one(&st.postgres)
        .await?;
    ctx.audit(&st, "rum.replay_playlist_created", &id, &name).await;
    Ok(Json(load_playlist(&st, &ctx, &id).await?))
}

#[utoipa::path(put, path = "/api/v1/rum/replay/playlists/{id}", operation_id = "update_replay_playlist", params(("id" = String, Path)), request_body = ReplayPlaylistInput,
    responses((status = 200, body = ReplayPlaylist), (status = 404, body = crate::error::ErrorBody)), tag = "rum")]
pub async fn update_playlist(State(st): State<AppState>, Writer(ctx): Writer, Path(id): Path<String>, Json(input): Json<ReplayPlaylistInput>) -> ApiResult<ReplayPlaylist> {
    let name = check_name(&input)?;
    if !is_uuid(&id) {
        return Err(not_found());
    }
    let done = sqlx::query("UPDATE rum_replay_playlists SET name = $3, description = $4, updated_at = now() WHERE org_id = $1 AND id = $2::uuid")
        .bind(&ctx.org_id)
        .bind(&id)
        .bind(&name)
        .bind(input.description.trim())
        .execute(&st.postgres)
        .await?;
    if done.rows_affected() == 0 {
        return Err(not_found());
    }
    Ok(Json(load_playlist(&st, &ctx, &id).await?))
}

#[utoipa::path(delete, path = "/api/v1/rum/replay/playlists/{id}", operation_id = "delete_replay_playlist", params(("id" = String, Path)),
    responses((status = 204), (status = 404, body = crate::error::ErrorBody)), tag = "rum")]
pub async fn delete_playlist(State(st): State<AppState>, Writer(ctx): Writer, Path(id): Path<String>) -> Result<StatusCode, ApiError> {
    if !is_uuid(&id) {
        return Err(not_found());
    }
    let done = sqlx::query("DELETE FROM rum_replay_playlists WHERE org_id = $1 AND id = $2::uuid").bind(&ctx.org_id).bind(&id).execute(&st.postgres).await?;
    if done.rows_affected() == 0 {
        return Err(not_found());
    }
    ctx.audit(&st, "rum.replay_playlist_deleted", &id, "").await;
    Ok(StatusCode::NO_CONTENT)
}

/// Adds a session to a playlist (again: no change).
#[utoipa::path(post, path = "/api/v1/rum/replay/playlists/{id}/sessions", operation_id = "add_replay_playlist_session", params(("id" = String, Path)), request_body = PlaylistSession,
    responses((status = 200, body = ReplayPlaylist), (status = 404, body = crate::error::ErrorBody)), tag = "rum")]
pub async fn add_session(State(st): State<AppState>, Writer(ctx): Writer, Path(id): Path<String>, Json(s): Json<PlaylistSession>) -> ApiResult<ReplayPlaylist> {
    load_playlist(&st, &ctx, &id).await?;
    if s.session_id.trim().is_empty() || s.session_id.len() > 100 {
        return Err(ApiError::BadRequest("Invalid session id".into()));
    }
    sqlx::query(
        "INSERT INTO rum_replay_playlist_sessions (playlist_id, org_id, session_id, started_at, added_by) VALUES ($1::uuid, $2, $3, $4, $5::uuid) ON CONFLICT DO NOTHING",
    )
    .bind(&id)
    .bind(&ctx.org_id)
    .bind(s.session_id.trim())
    .bind(s.started_at)
    .bind(&ctx.user_id)
    .execute(&st.postgres)
    .await?;
    sqlx::query("UPDATE rum_replay_playlists SET updated_at = now() WHERE org_id = $1 AND id = $2::uuid").bind(&ctx.org_id).bind(&id).execute(&st.postgres).await?;
    Ok(Json(load_playlist(&st, &ctx, &id).await?))
}

#[utoipa::path(delete, path = "/api/v1/rum/replay/playlists/{id}/sessions/{session_id}", operation_id = "remove_replay_playlist_session", params(("id" = String, Path), ("session_id" = String, Path)),
    responses((status = 200, body = ReplayPlaylist), (status = 404, body = crate::error::ErrorBody)), tag = "rum")]
pub async fn remove_session(State(st): State<AppState>, Writer(ctx): Writer, Path((id, session_id)): Path<(String, String)>) -> ApiResult<ReplayPlaylist> {
    load_playlist(&st, &ctx, &id).await?;
    sqlx::query("DELETE FROM rum_replay_playlist_sessions WHERE org_id = $1 AND playlist_id = $2::uuid AND session_id = $3").bind(&ctx.org_id).bind(&id).bind(&session_id).execute(&st.postgres).await?;
    Ok(Json(load_playlist(&st, &ctx, &id).await?))
}

// ================================================================ heatmaps

#[derive(Deserialize, ToSchema, Debug)]
#[serde(rename_all = "camelCase")]
pub struct HeatmapRequest {
    pub range: TimeRange,
    #[serde(default)]
    pub application_id: Option<String>,
    /// The view; without one, the most visited views (to pick from).
    #[serde(default)]
    pub view_name: Option<String>,
    /// Only actions with this frustration type (`rage_click`, `dead_click`, `error_click`).
    #[serde(default)]
    pub frustration: Option<String>,
}

#[derive(Serialize, ToSchema, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct HeatmapPoint {
    pub x: f64,
    pub y: f64,
}

#[derive(Serialize, ToSchema, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct HeatmapCount {
    pub name: String,
    pub count: u64,
}

#[derive(Serialize, ToSchema, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct Heatmap {
    /// The latest full snapshot of the view: its Meta record and FullSnapshot record (absent without a recording).
    #[schema(value_type = Option<Vec<Object>>)]
    pub snapshot: Option<Vec<Value>>,
    /// Click positions in the snapshot's viewport, from the view's recordings.
    pub clicks: Vec<HeatmapPoint>,
    /// The view's most clicked elements (RUM actions by target name).
    pub top_elements: Vec<HeatmapCount>,
    /// The most visited views (sessions), to pick one.
    pub views: Vec<HeatmapCount>,
}

#[derive(clickhouse::Row, Deserialize)]
struct Counted {
    name: String,
    count: u64,
}

#[derive(clickhouse::Row, Deserialize)]
struct BodyRow {
    body: String,
}

/// Click positions of a segment's records (MouseInteraction records of type Click).
pub fn clicks_of(records: &[Value]) -> Vec<HeatmapPoint> {
    records
        .iter()
        .filter(|r| r.get("type").and_then(Value::as_i64) == Some(3))
        .filter_map(|r| r.get("data"))
        .filter(|d| d.get("source").and_then(Value::as_i64) == Some(2) && d.get("type").and_then(Value::as_i64) == Some(2))
        .filter_map(|d| Some(HeatmapPoint { x: d.get("x")?.as_f64()?, y: d.get("y")?.as_f64()? }))
        .collect()
}

/// The Meta and FullSnapshot records that open a segment, if it has a full snapshot.
pub fn snapshot_of(records: &[Value]) -> Option<Vec<Value>> {
    let full = records.iter().position(|r| r.get("type").and_then(Value::as_i64) == Some(2))?;
    let meta = records[..full].iter().rev().find(|r| r.get("type").and_then(Value::as_i64) == Some(4)).cloned();
    Some(meta.into_iter().chain([records[full].clone()]).collect())
}

#[utoipa::path(post, path = "/api/v1/rum/replay/heatmap", operation_id = "session_replay_heatmap", request_body = HeatmapRequest,
    responses((status = 200, body = Heatmap), (status = 400, body = crate::error::ErrorBody)), tag = "rum")]
pub async fn heatmap(State(st): State<AppState>, ctx: AuthContext, Json(req): Json<HeatmapRequest>) -> ApiResult<Heatmap> {
    req.range.validate()?;
    let ch = st.ch(&ctx.org_id);
    let app = req.application_id.clone().filter(|a| !a.is_empty());
    let app_sql = if app.is_some() { " AND ApplicationId = ?" } else { "" };
    let with_app = |q: clickhouse::query::Query| match &app {
        Some(a) => q.bind(a),
        None => q,
    };
    let views: Vec<Counted> = with_app(ch.query(&format!(
        "SELECT LogAttributes['view.name'] AS name, uniqExact(SessionId) AS count FROM otel.otel_rum WHERE OrgId = {{oc_org:String}} \
         AND Timestamp >= fromUnixTimestamp64Milli(?) AND Timestamp <= fromUnixTimestamp64Milli(?){app_sql} AND EventType = 'view' AND name != '' GROUP BY name ORDER BY count DESC LIMIT 10"
    ))
    .bind(req.range.from)
    .bind(req.range.to))
    .fetch_all()
    .await?;
    let mut out = Heatmap { views: views.into_iter().map(|c| HeatmapCount { name: c.name, count: c.count }).collect(), ..Default::default() };
    let Some(view) = req.view_name.clone().filter(|v| !v.is_empty()) else { return Ok(Json(out)) };
    let frustration_sql = if req.frustration.is_some() { " AND has(splitByChar(',', LogAttributes['action.frustration.type']), ?)" } else { "" };
    let q = with_app(
        ch.query(&format!(
            "SELECT LogAttributes['action.target.name'] AS name, count() AS count FROM otel.otel_rum WHERE OrgId = {{oc_org:String}} \
             AND Timestamp >= fromUnixTimestamp64Milli(?) AND Timestamp <= fromUnixTimestamp64Milli(?){app_sql} AND EventType = 'action' AND LogAttributes['view.name'] = ?{frustration_sql} \
             AND name != '' GROUP BY name ORDER BY count DESC LIMIT 20"
        ))
        .bind(req.range.from)
        .bind(req.range.to),
    )
    .bind(&view);
    let q = match &req.frustration {
        Some(f) => q.bind(f),
        None => q,
    };
    out.top_elements = q.fetch_all::<Counted>().await?.into_iter().map(|c| HeatmapCount { name: c.name, count: c.count }).collect();
    let bodies: Vec<BodyRow> = with_app(
        ch.query(&format!(
            "SELECT Body AS body FROM otel.otel_rum_replay WHERE OrgId = {{oc_org:String}} AND Timestamp >= fromUnixTimestamp64Milli(?) AND Timestamp <= fromUnixTimestamp64Milli(?){app_sql} \
             AND ViewId IN (SELECT ViewId FROM otel.otel_rum WHERE OrgId = {{oc_org:String}} AND Timestamp >= fromUnixTimestamp64Milli(?) AND Timestamp <= fromUnixTimestamp64Milli(?) \
             AND EventType = 'view' AND LogAttributes['view.name'] = ?) ORDER BY Timestamp DESC LIMIT {HEATMAP_SEGMENTS}"
        ))
        .bind(req.range.from)
        .bind(req.range.to),
    )
    .bind(req.range.from)
    .bind(req.range.to)
    .bind(&view)
    .fetch_all()
    .await?;
    for b in bodies {
        let records = records_of(&b.body);
        if out.snapshot.is_none() {
            out.snapshot = snapshot_of(&records);
        }
        out.clicks.extend(clicks_of(&records));
    }
    Ok(Json(out))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn clicks_and_snapshots_come_from_the_records() {
        let records = vec![
            json!({"type": 4, "timestamp": 1, "data": {"href": "https://shop.example.com/", "width": 1280, "height": 800}}),
            json!({"type": 2, "timestamp": 2, "data": {"node": {"type": 0, "id": 0, "childNodes": []}}}),
            json!({"type": 3, "timestamp": 3, "data": {"source": 1, "positions": [{"x": 5, "y": 5, "id": 1, "timeOffset": 0}]}}),
            json!({"type": 3, "timestamp": 4, "data": {"source": 2, "type": 2, "id": 7, "x": 640, "y": 120}}),
            json!({"type": 3, "timestamp": 5, "data": {"source": 2, "type": 1, "id": 7, "x": 640, "y": 120}}),
        ];
        let clicks = clicks_of(&records);
        assert_eq!(clicks.len(), 1);
        assert_eq!((clicks[0].x, clicks[0].y), (640.0, 120.0));
        let snap = snapshot_of(&records).unwrap();
        assert_eq!(snap.len(), 2);
        assert_eq!(snap[0]["type"], 4);
        assert!(snapshot_of(&records[2..]).is_none());
    }

    /// Against the live databases: a recorded session, the watch history and a playlist.
    #[tokio::test]
    async fn recordings_playlists_and_watches() {
        let Some((_, states)) = crate::test_support::replicas(1).await else { return };
        let st = states.into_iter().next().unwrap();
        let mut ctx = crate::test_support::admin_of_new_org(&st, "replay").await;
        let (user,): (String,) = sqlx::query_as("INSERT INTO users (id, org_id, email, name) VALUES (gen_random_uuid(), $1, $2, 'Replay Tester') RETURNING id::text")
            .bind(&ctx.org_id)
            .bind(format!("{}@example.com", ctx.org_id))
            .fetch_one(&st.postgres)
            .await
            .unwrap();
        ctx.user_id = user;
        let now = crate::routes::metrics::now_ms();
        let segment = json!({"application": {"id": "app"}, "session": {"id": "s-9"}, "view": {"id": "v-9"}, "start": now - 1000, "end": now,
            "records": [{"type": 4, "timestamp": now - 1000, "data": {"href": "https://shop.example.com/", "width": 1280, "height": 800}},
                        {"type": 2, "timestamp": now - 999, "data": {"node": {"type": 0, "id": 0, "childNodes": []}}},
                        {"type": 3, "timestamp": now - 500, "data": {"source": 2, "type": 2, "id": 0, "x": 10, "y": 20}}]});
        st.clickhouse
            .query("INSERT INTO otel.otel_rum_replay (Timestamp, Body, ResourceAttributes, LogAttributes) VALUES (fromUnixTimestamp64Milli(?), ?, map('oc.org_id', ?), map('session.id', 's-9', 'view.id', 'v-9', 'application.id', 'app', 'replay.start', ?, 'replay.end', ?))")
            .bind(now - 1000)
            .bind(segment.to_string())
            .bind(&ctx.org_id)
            .bind((now - 1000).to_string())
            .bind(now.to_string())
            .execute()
            .await
            .unwrap();
        st.clickhouse
            .query("INSERT INTO otel.otel_rum (Timestamp, Body, ResourceAttributes, LogAttributes) VALUES (fromUnixTimestamp64Milli(?), '{}', map('oc.org_id', ?), map('type', 'view', 'session.id', 's-9', 'view.id', 'v-9', 'view.name', '/cart', 'application.id', 'app'))")
            .bind(now - 1000)
            .bind(&ctx.org_id)
            .execute()
            .await
            .unwrap();
        let replay = session_replay(State(st.clone()), ctx.clone(), Path("s-9".into()), Json(SessionReplayRequest { started_at: now - 1000 })).await.unwrap().0;
        assert_eq!(replay.segments.len(), 1);
        assert_eq!(replay.segments[0].records.len(), 3);
        assert_eq!(replay.session.as_ref().unwrap().attributes["session.view.count"], "1");
        assert!(session_replay(State(st.clone()), ctx.clone(), Path("nope".into()), Json(SessionReplayRequest { started_at: now })).await.is_err());
        // Search Recordings finds it; a filter it doesn't pass hides it.
        let req = |f: SessionFilter| RumSearchRequest {
            range: TimeRange { from: now - 3_600_000, to: now },
            query: String::new(),
            event_type: "session".into(),
            application_id: None,
            limit: None,
            offset: None,
            ascending: false,
            sessions: Some(f),
        };
        assert_eq!(search_events(State(st.clone()), ctx.clone(), Json(req(SessionFilter { with_replay: true, ..Default::default() }))).await.unwrap().0.total, 1);
        assert_eq!(search_events(State(st.clone()), ctx.clone(), Json(req(SessionFilter { min_action_count: Some(4), ..Default::default() }))).await.unwrap().0.total, 0);
        watched(State(st.clone()), ctx.clone(), Path("s-9".into()), Json(SessionReplayRequest { started_at: now - 1000 })).await.unwrap();
        assert_eq!(history(State(st.clone()), ctx.clone()).await.unwrap().0[0].session_id, "s-9");
        let p = create_playlist(State(st.clone()), Writer(ctx.clone()), Json(ReplayPlaylistInput { name: "Checkout issues".into(), description: String::new() })).await.unwrap().0;
        let p = add_session(State(st.clone()), Writer(ctx.clone()), Path(p.id.clone()), Json(PlaylistSession { session_id: "s-9".into(), started_at: now - 1000 })).await.unwrap().0;
        assert_eq!((p.session_count, p.created_by_me), (1, true));
        let h = heatmap(State(st.clone()), ctx.clone(), Json(HeatmapRequest { range: TimeRange { from: now - 3_600_000, to: now }, application_id: None, view_name: Some("/cart".into()), frustration: None }))
            .await
            .unwrap()
            .0;
        assert_eq!((h.clicks.len(), h.snapshot.map(|s| s.len()), h.views[0].name.as_str()), (1, Some(2), "/cart"));
        delete_playlist(State(st.clone()), Writer(ctx.clone()), Path(p.id)).await.unwrap();
    }
}
