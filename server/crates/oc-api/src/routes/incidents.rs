//! Incident management: declare, list, update status/severity/commander, timeline notes,
//! impacts and follow-ups. Incidents are addressed by their per-org number (`IR-<n>`).
//! Status, severity and commander changes are recorded on the timeline automatically.

use std::collections::BTreeMap;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use sqlx::{Postgres, QueryBuilder};
use utoipa::ToSchema;

use super::identity::MemberRef;
use super::logs::TimeRange;
use crate::auth::Writer;
use crate::error::{ApiError, ApiResult};
use crate::state::{AppState, CurrentUser, OrgId};

#[derive(Serialize, Deserialize, ToSchema, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum IncidentStatus {
    Active,
    Stable,
    Resolved,
}

impl IncidentStatus {
    fn as_str(self) -> &'static str {
        match self {
            IncidentStatus::Active => "active",
            IncidentStatus::Stable => "stable",
            IncidentStatus::Resolved => "resolved",
        }
    }
    fn parse(s: &str) -> Self {
        match s {
            "stable" => IncidentStatus::Stable,
            "resolved" => IncidentStatus::Resolved,
            _ => IncidentStatus::Active,
        }
    }
    fn label(self) -> &'static str {
        match self {
            IncidentStatus::Active => "Active",
            IncidentStatus::Stable => "Stable",
            IncidentStatus::Resolved => "Resolved",
        }
    }
}

const SEVERITIES: &[&str] = &["SEV-1", "SEV-2", "SEV-3", "SEV-4", "SEV-5", "UNKNOWN"];
const DETECTION: &[&str] = &["monitor", "customer", "employee", "automation", "unknown"];

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct IncidentSummary {
    /// The IR-<n> number.
    pub id: i32,
    pub title: String,
    pub status: IncidentStatus,
    /// SEV-1 … SEV-5 or UNKNOWN.
    pub severity: String,
    pub incident_type: String,
    pub detection_method: String,
    pub commander: Option<MemberRef>,
    pub customer_impact: bool,
    pub test: bool,
    pub services: Vec<String>,
    pub teams: Vec<String>,
    pub detected_at: i64,
    pub declared_at: i64,
    pub resolved_at: Option<i64>,
}

#[derive(sqlx::FromRow)]
pub(crate) struct Row {
    pub(crate) pk: i64,
    pub(crate) public_id: i32,
    pub(crate) title: String,
    pub(crate) status: String,
    pub(crate) severity: String,
    pub(crate) incident_type: String,
    pub(crate) detection_method: String,
    pub(crate) commander_id: Option<String>,
    pub(crate) commander_name: Option<String>,
    pub(crate) customer_impact: bool,
    pub(crate) test: bool,
    pub(crate) services: Vec<String>,
    pub(crate) teams: Vec<String>,
    pub(crate) detected_at: i64,
    pub(crate) declared_at: i64,
    pub(crate) acknowledged_at: Option<i64>,
    pub(crate) stable_at: Option<i64>,
    pub(crate) resolved_at: Option<i64>,
    pub(crate) declared_by: String,
    pub(crate) summary: String,
    pub(crate) root_cause: String,
    pub(crate) total: Option<i64>,
}

const SELECT: &str = "SELECT i.id AS pk, i.public_id, i.title, i.status, i.severity, i.incident_type, i.detection_method, i.commander_id::text AS commander_id, u.name AS commander_name, \
    i.customer_impact, i.test, i.services, i.teams, \
    (extract(epoch FROM i.detected_at) * 1000)::bigint AS detected_at, (extract(epoch FROM i.declared_at) * 1000)::bigint AS declared_at, \
    (extract(epoch FROM i.acknowledged_at) * 1000)::bigint AS acknowledged_at, (extract(epoch FROM i.stable_at) * 1000)::bigint AS stable_at, \
    (extract(epoch FROM i.resolved_at) * 1000)::bigint AS resolved_at, i.declared_by, i.summary, i.root_cause";
const FROM: &str = " FROM incidents i LEFT JOIN users u ON u.id = i.commander_id";

impl Row {
    fn summary(&self) -> IncidentSummary {
        IncidentSummary {
            id: self.public_id,
            title: self.title.clone(),
            status: IncidentStatus::parse(&self.status),
            severity: self.severity.clone(),
            incident_type: self.incident_type.clone(),
            detection_method: self.detection_method.clone(),
            commander: self.commander_id.clone().map(|id| MemberRef { id, name: self.commander_name.clone().unwrap_or_default() }),
            customer_impact: self.customer_impact,
            test: self.test,
            services: self.services.clone(),
            teams: self.teams.clone(),
            detected_at: self.detected_at,
            declared_at: self.declared_at,
            resolved_at: self.resolved_at,
        }
    }
}

// ================================================================ search

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct IncidentSearchRequest {
    /// Matches title, IR number, services and teams.
    #[serde(default)]
    pub query: String,
    #[serde(default)]
    pub statuses: Vec<IncidentStatus>,
    #[serde(default)]
    pub severities: Vec<String>,
    /// Only incidents the current user commands or responds to.
    #[serde(default)]
    pub mine: bool,
    #[serde(default)]
    pub teams: Vec<String>,
    #[serde(default)]
    pub services: Vec<String>,
    #[serde(default)]
    pub detection_methods: Vec<String>,
    #[serde(default)]
    pub incident_types: Vec<String>,
    /// Only incidents with (true) or without (false) customer impact.
    #[serde(default)]
    pub customer_impact: Option<bool>,
    /// Declared within this range.
    pub range: TimeRange,
    pub offset: i64,
    pub limit: i64,
}

#[derive(Serialize, ToSchema, Default)]
pub struct IncidentCounts {
    pub active: i64,
    pub stable: i64,
    pub resolved: i64,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct IncidentSearchPage {
    pub incidents: Vec<IncidentSummary>,
    pub total: i64,
    /// Per status for the same filters, ignoring the status filter (facet counts).
    pub counts: IncidentCounts,
    /// Per severity, ignoring the severity filter.
    pub severity_counts: BTreeMap<String, i64>,
    /// Per team, ignoring the team filter.
    pub team_counts: BTreeMap<String, i64>,
    /// Per service, ignoring the service filter.
    pub service_counts: BTreeMap<String, i64>,
    /// Per detection method, ignoring the detection method filter.
    pub detection_counts: BTreeMap<String, i64>,
    /// Per incident type, ignoring the incident type filter.
    pub type_counts: BTreeMap<String, i64>,
    /// `yes` / `no`, ignoring the customer impact filter.
    pub impact_counts: BTreeMap<String, i64>,
}

/// A filter left out of a facet's own counts.
#[derive(Clone, Copy, PartialEq)]
enum Facet {
    Severity,
    Team,
    Service,
    Detection,
    IncidentType,
    Impact,
}

impl Facet {
    /// The value counted and the rows it's counted over.
    fn select(self) -> &'static str {
        match self {
            Facet::Severity => "SELECT i.severity, count(*) FROM incidents i",
            Facet::Team => "SELECT t.v, count(*) FROM incidents i CROSS JOIN LATERAL unnest(i.teams) AS t(v)",
            Facet::Service => "SELECT t.v, count(*) FROM incidents i CROSS JOIN LATERAL unnest(i.services) AS t(v)",
            Facet::Detection => "SELECT i.detection_method, count(*) FROM incidents i",
            Facet::IncidentType => "SELECT i.incident_type, count(*) FROM incidents i",
            Facet::Impact => "SELECT CASE WHEN i.customer_impact THEN 'yes' ELSE 'no' END, count(*) FROM incidents i",
        }
    }
}

fn push_filters(qb: &mut QueryBuilder<Postgres>, org: &str, me: &str, req: &IncidentSearchRequest, skip: Option<Facet>) {
    qb.push(" WHERE i.org_id = ").push_bind(org.to_string());
    qb.push(" AND i.declared_at >= to_timestamp(").push_bind(req.range.from as f64 / 1000.0).push(") AND i.declared_at <= to_timestamp(").push_bind(req.range.to as f64 / 1000.0).push(")");
    let text = req.query.trim();
    if !text.is_empty() {
        let p = format!("%{}%", text.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_"));
        let num = text.trim_start_matches("IR-").trim_start_matches("ir-").parse::<i32>().ok();
        qb.push(" AND (i.title ILIKE ").push_bind(p.clone()).push(" OR array_to_string(i.services, ' ') ILIKE ").push_bind(p.clone()).push(" OR array_to_string(i.teams, ' ') ILIKE ").push_bind(p);
        if let Some(n) = num {
            qb.push(" OR i.public_id = ").push_bind(n);
        }
        qb.push(")");
    }
    if !req.severities.is_empty() && skip != Some(Facet::Severity) {
        qb.push(" AND i.severity = ANY(").push_bind(req.severities.clone()).push(")");
    }
    if !req.teams.is_empty() && skip != Some(Facet::Team) {
        qb.push(" AND i.teams && ").push_bind(req.teams.clone());
    }
    if !req.services.is_empty() && skip != Some(Facet::Service) {
        qb.push(" AND i.services && ").push_bind(req.services.clone());
    }
    if !req.detection_methods.is_empty() && skip != Some(Facet::Detection) {
        qb.push(" AND i.detection_method = ANY(").push_bind(req.detection_methods.clone()).push(")");
    }
    if !req.incident_types.is_empty() && skip != Some(Facet::IncidentType) {
        qb.push(" AND i.incident_type = ANY(").push_bind(req.incident_types.clone()).push(")");
    }
    if let Some(impact) = req.customer_impact.filter(|_| skip != Some(Facet::Impact)) {
        qb.push(" AND i.customer_impact = ").push_bind(impact);
    }
    if req.mine {
        qb.push(" AND (i.commander_id::text = ").push_bind(me.to_string()).push(" OR i.id IN (SELECT incident_id FROM incident_responders WHERE user_id::text = ").push_bind(me.to_string()).push("))");
    }
}

/// Incidents per value of `facet` under every filter but the facet's own.
async fn facet_counts(st: &AppState, org: &str, me: &str, req: &IncidentSearchRequest, facet: Facet) -> Result<BTreeMap<String, i64>, ApiError> {
    let mut q: QueryBuilder<Postgres> = QueryBuilder::new(facet.select());
    push_filters(&mut q, org, me, req, Some(facet));
    push_statuses(&mut q, req);
    q.push(" GROUP BY 1");
    Ok(q.build_query_as::<(String, i64)>().fetch_all(&st.postgres).await?.into_iter().collect())
}

fn push_statuses(qb: &mut QueryBuilder<Postgres>, req: &IncidentSearchRequest) {
    if !req.statuses.is_empty() {
        qb.push(" AND i.status = ANY(").push_bind(req.statuses.iter().map(|s| s.as_str().to_string()).collect::<Vec<_>>()).push(")");
    }
}

#[utoipa::path(post, path = "/api/v1/incidents/search", operation_id = "search_incidents", request_body = IncidentSearchRequest, responses((status = 200, body = IncidentSearchPage)), tag = "incidents")]
pub async fn search(State(st): State<AppState>, OrgId(org): OrgId, CurrentUser(me): CurrentUser, Json(req): Json<IncidentSearchRequest>) -> ApiResult<IncidentSearchPage> {
    req.range.validate_loose()?;
    let mut qb: QueryBuilder<Postgres> = QueryBuilder::new(SELECT);
    qb.push(", count(*) OVER () AS total").push(FROM);
    push_filters(&mut qb, &org, &me, &req, None);
    push_statuses(&mut qb, &req);
    // Open incidents first, then newest.
    qb.push(" ORDER BY (i.status = 'resolved'), i.declared_at DESC LIMIT ").push_bind(req.limit.clamp(1, 200)).push(" OFFSET ").push_bind(req.offset.max(0));
    let rows: Vec<Row> = qb.build_query_as().fetch_all(&st.postgres).await?;

    let mut cq: QueryBuilder<Postgres> = QueryBuilder::new("SELECT i.status, count(*) FROM incidents i");
    push_filters(&mut cq, &org, &me, &req, None);
    cq.push(" GROUP BY i.status");
    let per_status: Vec<(String, i64)> = cq.build_query_as().fetch_all(&st.postgres).await?;
    let mut counts = IncidentCounts::default();
    for (s, n) in per_status {
        match s.as_str() {
            "active" => counts.active = n,
            "stable" => counts.stable = n,
            _ => counts.resolved = n,
        }
    }
    let (severity_counts, team_counts, service_counts, detection_counts, type_counts, impact_counts) = tokio::try_join!(
        facet_counts(&st, &org, &me, &req, Facet::Severity),
        facet_counts(&st, &org, &me, &req, Facet::Team),
        facet_counts(&st, &org, &me, &req, Facet::Service),
        facet_counts(&st, &org, &me, &req, Facet::Detection),
        facet_counts(&st, &org, &me, &req, Facet::IncidentType),
        facet_counts(&st, &org, &me, &req, Facet::Impact),
    )?;
    let total = rows.first().and_then(|r| r.total).unwrap_or(0);
    Ok(Json(IncidentSearchPage { incidents: rows.iter().map(Row::summary).collect(), total, counts, severity_counts, team_counts, service_counts, detection_counts, type_counts, impact_counts }))
}

// ================================================================ detail

#[derive(Serialize, ToSchema, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct TimelineEvent {
    pub id: i64,
    /// declared | note | status | severity | commander | notification | impact | followup
    pub kind: String,
    pub author: String,
    pub author_id: Option<String>,
    pub body: String,
    pub important: bool,
    pub created_at: i64,
}

#[derive(Serialize, ToSchema, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct Impact {
    pub id: i64,
    pub description: String,
    pub start_at: i64,
    pub end_at: Option<i64>,
}

#[derive(Serialize, ToSchema, sqlx::FromRow)]
pub struct Followup {
    pub id: i64,
    pub title: String,
    pub done: bool,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct IncidentDetail {
    #[serde(flatten)]
    pub incident: IncidentSummary,
    pub summary: String,
    pub root_cause: String,
    pub declared_by: String,
    pub acknowledged_at: Option<i64>,
    pub stable_at: Option<i64>,
    pub responders: Vec<MemberRef>,
    pub impacts: Vec<Impact>,
    pub followups: Vec<Followup>,
    /// Latest notification events, newest first.
    pub notifications: Vec<TimelineEvent>,
}

pub(crate) async fn find(st: &AppState, org: &str, id: i32) -> Result<Row, ApiError> {
    let mut qb: QueryBuilder<Postgres> = QueryBuilder::new(SELECT);
    qb.push(", NULL::bigint AS total").push(FROM).push(" WHERE i.org_id = ").push_bind(org).push(" AND i.public_id = ").push_bind(id);
    qb.build_query_as().fetch_optional(&st.postgres).await?.ok_or_else(|| ApiError::NotFound(format!("Incident IR-{id} not found")))
}

async fn detail_of(st: &AppState, org: &str, id: i32) -> Result<IncidentDetail, ApiError> {
    let row = find(st, org, id).await?;
    let responders = sqlx::query_as::<_, (String, String)>("SELECT u.id::text, u.name FROM incident_responders r JOIN users u ON u.id = r.user_id WHERE r.incident_id = $1 ORDER BY u.name")
        .bind(row.pk)
        .fetch_all(&st.postgres);
    let impacts = sqlx::query_as::<_, Impact>(
        "SELECT id, description, (extract(epoch FROM start_at) * 1000)::bigint AS start_at, (extract(epoch FROM end_at) * 1000)::bigint AS end_at FROM incident_impacts WHERE incident_id = $1 ORDER BY start_at",
    )
    .bind(row.pk)
    .fetch_all(&st.postgres);
    let followups = sqlx::query_as::<_, Followup>("SELECT id, title, done FROM incident_followups WHERE incident_id = $1 ORDER BY id").bind(row.pk).fetch_all(&st.postgres);
    let notifications = sqlx::query_as::<_, TimelineEvent>(
        "SELECT id, kind, author, author_id::text, body, important, (extract(epoch FROM created_at) * 1000)::bigint AS created_at FROM incident_events WHERE incident_id = $1 AND kind = 'notification' ORDER BY created_at DESC LIMIT 5",
    )
    .bind(row.pk)
    .fetch_all(&st.postgres);
    let (responders, impacts, followups, notifications) = tokio::try_join!(responders, impacts, followups, notifications)?;
    Ok(IncidentDetail {
        incident: row.summary(),
        summary: row.summary,
        root_cause: row.root_cause,
        declared_by: row.declared_by,
        acknowledged_at: row.acknowledged_at,
        stable_at: row.stable_at,
        responders: responders.into_iter().map(|(id, name)| MemberRef { id, name }).collect(),
        impacts,
        followups,
        notifications,
    })
}

#[utoipa::path(get, path = "/api/v1/incidents/{id}", operation_id = "get_incident", params(("id" = i32, Path, description = "IR number")), responses((status = 200, body = IncidentDetail), (status = 404, body = crate::error::ErrorBody)), tag = "incidents")]
pub async fn get_one(State(st): State<AppState>, OrgId(org): OrgId, Path(id): Path<i32>) -> ApiResult<IncidentDetail> {
    Ok(Json(detail_of(&st, &org, id).await?))
}

pub(crate) async fn user_name(st: &AppState, user: &str) -> Result<String, ApiError> {
    let row: Option<(String,)> = sqlx::query_as("SELECT name FROM users WHERE id::text = $1").bind(user).fetch_optional(&st.postgres).await?;
    Ok(row.map(|r| r.0).unwrap_or_else(|| "Unknown user".into()))
}

fn check_severity(s: &str) -> Result<(), ApiError> {
    if SEVERITIES.contains(&s) {
        Ok(())
    } else {
        Err(ApiError::BadRequest(format!("severity must be one of {}", SEVERITIES.join(", "))))
    }
}

fn check_title(t: &str) -> Result<String, ApiError> {
    let t = t.trim();
    if t.is_empty() || t.chars().count() > 300 {
        return Err(ApiError::BadRequest("title must be 1–300 characters".into()));
    }
    Ok(t.to_string())
}

// ================================================================ declare / update

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct DeclareRequest {
    pub title: String,
    pub severity: String,
    #[serde(default = "default_type")]
    pub incident_type: String,
    #[serde(default)]
    pub detection_method: Option<String>,
    #[serde(default)]
    pub summary: String,
    /// Defaults to the declaring user.
    pub commander_id: Option<String>,
    #[serde(default)]
    pub services: Vec<String>,
    #[serde(default)]
    pub teams: Vec<String>,
    #[serde(default)]
    pub customer_impact: bool,
    #[serde(default)]
    pub test: bool,
}

fn default_type() -> String {
    "Production Incident".into()
}

#[utoipa::path(post, path = "/api/v1/incidents", request_body = DeclareRequest, responses((status = 200, body = IncidentDetail)), tag = "incidents")]
pub async fn declare(State(st): State<AppState>, _: Writer, OrgId(org): OrgId, CurrentUser(me): CurrentUser, Json(req): Json<DeclareRequest>) -> ApiResult<IncidentDetail> {
    let title = check_title(&req.title)?;
    check_severity(&req.severity)?;
    let detection = req.detection_method.unwrap_or_else(|| "employee".into());
    if !DETECTION.contains(&detection.as_str()) {
        return Err(ApiError::BadRequest(format!("detectionMethod must be one of {}", DETECTION.join(", "))));
    }
    let author = user_name(&st, &me).await?;
    let commander = req.commander_id.unwrap_or_else(|| me.clone());
    let mut tx = st.postgres.begin().await?;
    // Serialise numbering per org.
    sqlx::query("SELECT pg_advisory_xact_lock(hashtext($1))").bind(format!("incidents:{org}")).execute(&mut *tx).await?;
    let (id, pk): (i32, i64) = sqlx::query_as(
        "INSERT INTO incidents (org_id, public_id, title, severity, incident_type, detection_method, commander_id, declared_by, summary, services, teams, customer_impact, test) \
         SELECT $1, COALESCE(max(public_id), 0) + 1, $2, $3, $4, $5, (SELECT id FROM users WHERE org_id = $1 AND id::text = $6), $7, $8, $9, $10, $11, $12 FROM incidents WHERE org_id = $1 \
         RETURNING public_id, id",
    )
    .bind(&org)
    .bind(&title)
    .bind(&req.severity)
    .bind(req.incident_type.trim())
    .bind(&detection)
    .bind(&commander)
    .bind(&author)
    .bind(req.summary.trim())
    .bind(&req.services)
    .bind(&req.teams)
    .bind(req.customer_impact)
    .bind(req.test)
    .fetch_one(&mut *tx)
    .await?;
    sqlx::query("INSERT INTO incident_events (incident_id, kind, author, author_id, body) VALUES ($1, 'declared', $2, $3::uuid, $4)")
        .bind(pk)
        .bind(&author)
        .bind(&me)
        .bind(format!("Incident declared with severity {}", req.severity))
        .execute(&mut *tx)
        .await?;
    // Teams with notification channels hear about their incidents automatically.
    let channels: Vec<(Vec<String>,)> = sqlx::query_as("SELECT notifications FROM teams WHERE org_id = $1 AND handle = ANY($2) AND cardinality(notifications) > 0").bind(&org).bind(&req.teams).fetch_all(&mut *tx).await?;
    let recipients: Vec<String> = channels.into_iter().flat_map(|c| c.0).collect();
    if !recipients.is_empty() {
        let subject = format!("Incident #{id}: {title}");
        let body = format!("Status: Active | Severity: {} | Commander: {author}", req.severity);
        sqlx::query("INSERT INTO incident_notifications (incident_id, recipients, subject, body, automated, author) VALUES ($1, $2, $3, $4, true, 'ObserveCraft Incident Management')")
            .bind(pk)
            .bind(&recipients)
            .bind(&subject)
            .bind(&body)
            .execute(&mut *tx)
            .await?;
        sqlx::query("INSERT INTO incident_events (incident_id, kind, author, body) VALUES ($1, 'notification', 'ObserveCraft Incident Management', $2)")
            .bind(pk)
            .bind(format!("{subject} — sent to {}", recipients.join(", ")))
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok(Json(detail_of(&st, &org, id).await?))
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct IncidentPatch {
    pub title: Option<String>,
    pub status: Option<IncidentStatus>,
    pub severity: Option<String>,
    pub commander_id: Option<String>,
    pub summary: Option<String>,
    pub root_cause: Option<String>,
    pub services: Option<Vec<String>>,
    pub teams: Option<Vec<String>>,
    pub customer_impact: Option<bool>,
}

#[utoipa::path(patch, path = "/api/v1/incidents/{id}", operation_id = "update_incident", params(("id" = i32, Path)), request_body = IncidentPatch, responses((status = 200, body = IncidentDetail), (status = 404, body = crate::error::ErrorBody)), tag = "incidents")]
pub async fn update(State(st): State<AppState>, _: Writer, OrgId(org): OrgId, CurrentUser(me): CurrentUser, Path(id): Path<i32>, Json(p): Json<IncidentPatch>) -> ApiResult<IncidentDetail> {
    let row = find(&st, &org, id).await?;
    let author = user_name(&st, &me).await?;
    let title = p.title.as_deref().map(check_title).transpose()?;
    if let Some(s) = &p.severity {
        check_severity(s)?;
    }
    let mut events: Vec<(&str, String)> = Vec::new();
    let mut tx = st.postgres.begin().await?;
    if let Some(s) = p.status.filter(|s| s.as_str() != row.status) {
        events.push(("status", format!("Status changed from {} to {}", IncidentStatus::parse(&row.status).label(), s.label())));
        sqlx::query(
            "UPDATE incidents SET status = $2, \
             stable_at = CASE WHEN $2 IN ('stable', 'resolved') THEN COALESCE(stable_at, now()) ELSE NULL END, \
             resolved_at = CASE WHEN $2 = 'resolved' THEN now() ELSE NULL END WHERE id = $1",
        )
        .bind(row.pk)
        .bind(s.as_str())
        .execute(&mut *tx)
        .await?;
    }
    if let Some(s) = p.severity.as_ref().filter(|s| **s != row.severity) {
        events.push(("severity", format!("Severity changed from {} to {s}", row.severity)));
    }
    if let Some(c) = p.commander_id.as_ref().filter(|c| Some(*c) != row.commander_id.as_ref()) {
        let name = user_name(&st, c).await?;
        events.push(("commander", format!("{name} is now the incident commander")));
    }
    sqlx::query(
        "UPDATE incidents SET title = COALESCE($2, title), severity = COALESCE($3, severity), \
         commander_id = COALESCE((SELECT id FROM users WHERE org_id = $4 AND id::text = $5), commander_id), \
         summary = COALESCE($6, summary), root_cause = COALESCE($7, root_cause), services = COALESCE($8, services), teams = COALESCE($9, teams), \
         customer_impact = COALESCE($10, customer_impact), acknowledged_at = COALESCE(acknowledged_at, now()), modified_at = now() WHERE id = $1",
    )
    .bind(row.pk)
    .bind(title)
    .bind(&p.severity)
    .bind(&org)
    .bind(&p.commander_id)
    .bind(p.summary.map(|s| s.trim().to_string()))
    .bind(p.root_cause.map(|s| s.trim().to_string()))
    .bind(&p.services)
    .bind(&p.teams)
    .bind(p.customer_impact)
    .execute(&mut *tx)
    .await?;
    for (kind, body) in events {
        sqlx::query("INSERT INTO incident_events (incident_id, kind, author, author_id, body) VALUES ($1, $2, $3, $4::uuid, $5)").bind(row.pk).bind(kind).bind(&author).bind(&me).bind(body).execute(&mut *tx).await?;
    }
    tx.commit().await?;
    Ok(Json(detail_of(&st, &org, id).await?))
}

// ================================================================ timeline, impacts, follow-ups

#[utoipa::path(get, path = "/api/v1/incidents/{id}/timeline", params(("id" = i32, Path)), responses((status = 200, body = Vec<TimelineEvent>)), tag = "incidents")]
pub async fn timeline(State(st): State<AppState>, OrgId(org): OrgId, Path(id): Path<i32>) -> ApiResult<Vec<TimelineEvent>> {
    let row = find(&st, &org, id).await?;
    let events = sqlx::query_as("SELECT id, kind, author, author_id::text, body, important, (extract(epoch FROM created_at) * 1000)::bigint AS created_at FROM incident_events WHERE incident_id = $1 ORDER BY created_at, id")
        .bind(row.pk)
        .fetch_all(&st.postgres)
        .await?;
    Ok(Json(events))
}

#[derive(Deserialize, ToSchema)]
pub struct NoteRequest {
    pub body: String,
}

#[utoipa::path(post, path = "/api/v1/incidents/{id}/timeline", params(("id" = i32, Path)), request_body = NoteRequest, responses((status = 204)), tag = "incidents")]
pub async fn add_note(State(st): State<AppState>, _: Writer, OrgId(org): OrgId, CurrentUser(me): CurrentUser, Path(id): Path<i32>, Json(req): Json<NoteRequest>) -> Result<StatusCode, ApiError> {
    let body = req.body.trim();
    if body.is_empty() || body.chars().count() > 10_000 {
        return Err(ApiError::BadRequest("note must be 1–10000 characters".into()));
    }
    let row = find(&st, &org, id).await?;
    let author = user_name(&st, &me).await?;
    sqlx::query("INSERT INTO incident_events (incident_id, kind, author, author_id, body) VALUES ($1, 'note', $2, $3::uuid, $4)").bind(row.pk).bind(&author).bind(&me).bind(body).execute(&st.postgres).await?;
    sqlx::query("UPDATE incidents SET acknowledged_at = COALESCE(acknowledged_at, now()), modified_at = now() WHERE id = $1").bind(row.pk).execute(&st.postgres).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ImpactRequest {
    pub description: String,
    pub start_at: i64,
    pub end_at: Option<i64>,
}

#[utoipa::path(post, path = "/api/v1/incidents/{id}/impacts", params(("id" = i32, Path)), request_body = ImpactRequest, responses((status = 204)), tag = "incidents")]
pub async fn add_impact(State(st): State<AppState>, _: Writer, OrgId(org): OrgId, CurrentUser(me): CurrentUser, Path(id): Path<i32>, Json(req): Json<ImpactRequest>) -> Result<StatusCode, ApiError> {
    let desc = check_title(&req.description)?;
    if req.end_at.is_some_and(|e| e < req.start_at) {
        return Err(ApiError::BadRequest("impact end must be after its start".into()));
    }
    let row = find(&st, &org, id).await?;
    let author = user_name(&st, &me).await?;
    sqlx::query("INSERT INTO incident_impacts (incident_id, description, start_at, end_at) VALUES ($1, $2, to_timestamp($3), to_timestamp($4))")
        .bind(row.pk)
        .bind(&desc)
        .bind(req.start_at as f64 / 1000.0)
        .bind(req.end_at.map(|e| e as f64 / 1000.0))
        .execute(&st.postgres)
        .await?;
    sqlx::query("UPDATE incidents SET customer_impact = true, modified_at = now() WHERE id = $1").bind(row.pk).execute(&st.postgres).await?;
    sqlx::query("INSERT INTO incident_events (incident_id, kind, author, body) VALUES ($1, 'impact', $2, $3)").bind(row.pk).bind(&author).bind(format!("Impact added: {desc}")).execute(&st.postgres).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize, ToSchema)]
pub struct FollowupRequest {
    pub title: String,
}

#[utoipa::path(post, path = "/api/v1/incidents/{id}/followups", params(("id" = i32, Path)), request_body = FollowupRequest, responses((status = 204)), tag = "incidents")]
pub async fn add_followup(State(st): State<AppState>, _: Writer, OrgId(org): OrgId, Path(id): Path<i32>, Json(req): Json<FollowupRequest>) -> Result<StatusCode, ApiError> {
    let title = check_title(&req.title)?;
    let row = find(&st, &org, id).await?;
    sqlx::query("INSERT INTO incident_followups (incident_id, title) VALUES ($1, $2)").bind(row.pk).bind(&title).execute(&st.postgres).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize, ToSchema)]
pub struct FollowupPatch {
    pub done: bool,
}

#[utoipa::path(patch, path = "/api/v1/incidents/{id}/followups/{followup_id}", params(("id" = i32, Path), ("followup_id" = i64, Path)), request_body = FollowupPatch, responses((status = 204), (status = 404, body = crate::error::ErrorBody)), tag = "incidents")]
pub async fn update_followup(State(st): State<AppState>, _: Writer, OrgId(org): OrgId, Path((id, followup_id)): Path<(i32, i64)>, Json(req): Json<FollowupPatch>) -> Result<StatusCode, ApiError> {
    let row = find(&st, &org, id).await?;
    let done = sqlx::query("UPDATE incident_followups SET done = $3 WHERE incident_id = $1 AND id = $2").bind(row.pk).bind(followup_id).bind(req.done).execute(&st.postgres).await?;
    if done.rows_affected() == 0 {
        return Err(ApiError::NotFound(format!("Follow-up {followup_id} not found")));
    }
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req() -> IncidentSearchRequest {
        serde_json::from_value(serde_json::json!({
            "severities": ["SEV-1"], "teams": ["payments"], "services": ["billing"], "detectionMethods": ["monitor"], "incidentTypes": ["Security Incident"], "customerImpact": true,
            "range": {"from": 0, "to": 1}, "offset": 0, "limit": 10
        }))
        .unwrap()
    }

    fn sql(skip: Option<Facet>) -> String {
        let mut qb: QueryBuilder<Postgres> = QueryBuilder::new("SELECT 1 FROM incidents i");
        push_filters(&mut qb, "org", "me", &req(), skip);
        qb.sql().to_string()
    }

    #[test]
    fn a_facets_counts_ignore_only_its_own_filter() {
        assert!(sql(None).contains("i.severity = ANY") && sql(None).contains("i.teams &&"));
        assert!(!sql(Some(Facet::Severity)).contains("i.severity") && sql(Some(Facet::Severity)).contains("i.teams &&"));
        assert!(!sql(Some(Facet::Team)).contains("i.teams &&") && sql(Some(Facet::Team)).contains("i.severity = ANY"));
        // The organization is always filtered.
        assert!(sql(Some(Facet::Team)).contains("i.org_id = $1"));
        for (facet, clause) in [(Facet::Service, "i.services &&"), (Facet::Detection, "i.detection_method = ANY"), (Facet::IncidentType, "i.incident_type = ANY"), (Facet::Impact, "i.customer_impact =")] {
            assert!(sql(None).contains(clause), "{clause}");
            assert!(!sql(Some(facet)).contains(clause), "{clause}");
        }
    }
}
