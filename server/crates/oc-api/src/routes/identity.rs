//! Identity: the current user's profile, users, roles, teams and SAML group mappings.
//! Everything is scoped by org; "me" is the [`CurrentUser`] (the dev user until auth lands).

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use sqlx::{Postgres, QueryBuilder};
use utoipa::{IntoParams, ToSchema};

use crate::auth::{Admin, Writer};
use crate::error::{ApiError, ApiResult};
use crate::state::{AppState, CurrentUser, OrgId};

#[derive(Serialize, ToSchema, sqlx::FromRow, Clone)]
pub struct RoleRef {
    pub id: String,
    pub name: String,
}

#[derive(Serialize, ToSchema, sqlx::FromRow, Clone)]
pub struct TeamRef {
    pub handle: String,
    pub name: String,
}

#[derive(Serialize, ToSchema, Clone, Copy, PartialEq, Eq, Deserialize, Debug)]
#[serde(rename_all = "snake_case")]
pub enum UserStatus {
    Active,
    Pending,
    Deactivated,
}

impl UserStatus {
    fn parse(s: &str) -> Self {
        match s {
            "pending" => UserStatus::Pending,
            "deactivated" => UserStatus::Deactivated,
            _ => UserStatus::Active,
        }
    }
    fn as_str(self) -> &'static str {
        match self {
            UserStatus::Active => "active",
            UserStatus::Pending => "pending",
            UserStatus::Deactivated => "deactivated",
        }
    }
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct UserSummary {
    pub id: String,
    pub name: String,
    pub email: String,
    pub title: Option<String>,
    pub status: UserStatus,
    pub roles: Vec<RoleRef>,
    pub login_methods: Vec<String>,
    pub mfa: bool,
    pub last_login_at: Option<i64>,
    pub created_at: i64,
}

#[derive(sqlx::FromRow)]
struct UserRow {
    id: String,
    name: String,
    email: String,
    title: Option<String>,
    status: String,
    login_methods: Vec<String>,
    mfa_enabled: bool,
    timezone: Option<String>,
    last_login_at: Option<i64>,
    created_at: i64,
    role_ids: Vec<String>,
    role_names: Vec<String>,
    total: Option<i64>,
}

impl UserRow {
    fn summary(self) -> UserSummary {
        UserSummary {
            roles: self.role_ids.into_iter().zip(self.role_names).map(|(id, name)| RoleRef { id, name }).collect(),
            status: UserStatus::parse(&self.status),
            id: self.id,
            name: self.name,
            email: self.email,
            title: self.title,
            login_methods: self.login_methods,
            mfa: self.mfa_enabled,
            last_login_at: self.last_login_at,
            created_at: self.created_at,
        }
    }
}

const USER_SELECT: &str = "SELECT u.id::text AS id, u.name, u.email, u.title, u.status, u.login_methods, u.mfa_enabled, u.timezone, \
    (extract(epoch FROM u.last_login_at) * 1000)::bigint AS last_login_at, (extract(epoch FROM u.created_at) * 1000)::bigint AS created_at, \
    COALESCE(array_agg(r.id ORDER BY r.managed DESC, r.name) FILTER (WHERE r.id IS NOT NULL), '{}') AS role_ids, \
    COALESCE(array_agg(r.name ORDER BY r.managed DESC, r.name) FILTER (WHERE r.id IS NOT NULL), '{}') AS role_names";
const USER_FROM: &str = " FROM users u LEFT JOIN user_roles ur ON ur.user_id = u.id LEFT JOIN roles r ON r.id = ur.role_id";

fn ilike(s: &str) -> String {
    format!("%{}%", s.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_"))
}

// ================================================================ me

#[derive(Serialize, ToSchema)]
pub struct OrgRef {
    pub id: String,
    pub name: String,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct Profile {
    #[serde(flatten)]
    pub user: UserSummary,
    pub timezone: Option<String>,
    pub org: OrgRef,
    pub teams: Vec<TeamRef>,
    /// May create and manage organizations (Instance Admin).
    pub is_instance_admin: bool,
}

async fn profile(st: &AppState, org: &str, user: &str) -> Result<Profile, ApiError> {
    let mut qb: QueryBuilder<Postgres> = QueryBuilder::new(USER_SELECT);
    qb.push(", NULL::bigint AS total").push(USER_FROM).push(" WHERE u.org_id = ").push_bind(org).push(" AND u.id = ").push_bind(user).push("::uuid GROUP BY u.id");
    let row: UserRow = qb.build_query_as().fetch_optional(&st.postgres).await?.ok_or_else(|| ApiError::NotFound("Current user not found".into()))?;
    let org_ref: (String, String) = sqlx::query_as("SELECT id, name FROM orgs WHERE id = $1").bind(org).fetch_one(&st.postgres).await?;
    let teams: Vec<TeamRef> = sqlx::query_as("SELECT t.handle, t.name FROM team_members m JOIN teams t ON t.org_id = m.org_id AND t.handle = m.handle WHERE m.org_id = $1 AND m.user_id = $2::uuid ORDER BY t.name")
        .bind(org)
        .bind(user)
        .fetch_all(&st.postgres)
        .await?;
    let (is_instance_admin,): (bool,) = sqlx::query_as("SELECT a.is_instance_admin FROM users u JOIN accounts a ON a.id = u.account_id WHERE u.id = $1::uuid").bind(user).fetch_one(&st.postgres).await?;
    let timezone = row.timezone.clone();
    Ok(Profile { user: row.summary(), timezone, org: OrgRef { id: org_ref.0, name: org_ref.1 }, teams, is_instance_admin })
}

#[utoipa::path(get, path = "/api/v1/me", responses((status = 200, body = Profile)), tag = "identity")]
pub async fn me(State(st): State<AppState>, OrgId(org): OrgId, CurrentUser(user): CurrentUser) -> ApiResult<Profile> {
    Ok(Json(profile(&st, &org, &user).await?))
}

#[derive(Deserialize, ToSchema)]
pub struct ProfilePatch {
    pub name: Option<String>,
    pub title: Option<String>,
    pub timezone: Option<String>,
}

fn check_name(s: &str, what: &str, max: usize) -> Result<String, ApiError> {
    let t = s.trim();
    if t.is_empty() || t.chars().count() > max {
        return Err(ApiError::BadRequest(format!("{what} must be 1–{max} characters")));
    }
    Ok(t.to_string())
}

#[utoipa::path(patch, path = "/api/v1/me", request_body = ProfilePatch, responses((status = 200, body = Profile)), tag = "identity")]
pub async fn update_me(State(st): State<AppState>, OrgId(org): OrgId, CurrentUser(user): CurrentUser, Json(p): Json<ProfilePatch>) -> ApiResult<Profile> {
    let name = p.name.as_deref().map(|n| check_name(n, "name", 120)).transpose()?;
    let title = p.title.map(|t| t.trim().chars().take(120).collect::<String>());
    sqlx::query("UPDATE users SET name = COALESCE($3, name), title = COALESCE($4, title), timezone = COALESCE($5, timezone) WHERE org_id = $1 AND id = $2::uuid")
        .bind(&org)
        .bind(&user)
        .bind(name)
        .bind(title)
        .bind(p.timezone)
        .execute(&st.postgres)
        .await?;
    Ok(Json(profile(&st, &org, &user).await?))
}

// ================================================================ users

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct UserSearchRequest {
    /// Matches name or email.
    #[serde(default)]
    pub query: String,
    #[serde(default)]
    pub statuses: Vec<UserStatus>,
    /// password | google | saml
    #[serde(default)]
    pub login_methods: Vec<String>,
    /// configured | unconfigured
    #[serde(default)]
    pub mfa: Vec<String>,
    #[serde(default)]
    pub role_ids: Vec<String>,
    pub offset: i64,
    pub limit: i64,
}

#[derive(Serialize, ToSchema)]
pub struct UserSearchPage {
    pub users: Vec<UserSummary>,
    pub total: i64,
}

#[utoipa::path(post, path = "/api/v1/users/search", operation_id = "search_users", request_body = UserSearchRequest, responses((status = 200, body = UserSearchPage)), tag = "identity")]
pub async fn search_users(State(st): State<AppState>, OrgId(org): OrgId, Json(req): Json<UserSearchRequest>) -> ApiResult<UserSearchPage> {
    let mut qb: QueryBuilder<Postgres> = QueryBuilder::new(USER_SELECT);
    qb.push(", count(*) OVER () AS total").push(USER_FROM).push(" WHERE u.org_id = ").push_bind(&org);
    let text = req.query.trim();
    if !text.is_empty() {
        let p = ilike(text);
        qb.push(" AND (u.name ILIKE ").push_bind(p.clone()).push(" OR u.email ILIKE ").push_bind(p).push(")");
    }
    if !req.statuses.is_empty() {
        qb.push(" AND u.status = ANY(").push_bind(req.statuses.iter().map(|s| s.as_str().to_string()).collect::<Vec<_>>()).push(")");
    }
    if !req.login_methods.is_empty() {
        qb.push(" AND u.login_methods && ").push_bind(req.login_methods.clone());
    }
    let (cfg, uncfg) = (req.mfa.iter().any(|m| m == "configured"), req.mfa.iter().any(|m| m == "unconfigured"));
    if cfg != uncfg {
        qb.push(" AND u.mfa_enabled = ").push_bind(cfg);
    }
    if !req.role_ids.is_empty() {
        qb.push(" AND u.id IN (SELECT user_id FROM user_roles WHERE role_id = ANY(").push_bind(req.role_ids.clone()).push("))");
    }
    qb.push(" GROUP BY u.id ORDER BY lower(u.name), u.id LIMIT ").push_bind(req.limit.clamp(1, 200)).push(" OFFSET ").push_bind(req.offset.max(0));
    let rows: Vec<UserRow> = qb.build_query_as().fetch_all(&st.postgres).await?;
    let total = rows.first().and_then(|r| r.total).unwrap_or(0);
    Ok(Json(UserSearchPage { total, users: rows.into_iter().map(UserRow::summary).collect() }))
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct InviteRequest {
    pub emails: Vec<String>,
    #[serde(default)]
    pub role_ids: Vec<String>,
}

#[derive(Serialize, ToSchema)]
pub struct Invitation {
    pub id: String,
    pub email: String,
    /// One-time link (72h) the user opens to set a password. Share it with them until mail delivery is configured.
    pub url: String,
}

/// Creates pending users and returns their invitation links.
#[utoipa::path(post, path = "/api/v1/users/invite", request_body = InviteRequest, responses((status = 200, body = Vec<Invitation>)), tag = "identity")]
pub async fn invite(State(st): State<AppState>, Admin(auth): Admin, OrgId(org): OrgId, Json(req): Json<InviteRequest>) -> ApiResult<Vec<Invitation>> {
    if req.emails.is_empty() || req.emails.len() > 50 {
        return Err(ApiError::BadRequest("invite 1–50 emails at a time".into()));
    }
    let mut ids: Vec<(String, String)> = Vec::new();
    let mut tx = st.postgres.begin().await?;
    for raw in &req.emails {
        let email = raw.trim().to_lowercase();
        let valid = email.len() <= 254 && email.split_once('@').is_some_and(|(l, d)| !l.is_empty() && d.contains('.') && !d.starts_with('.'));
        if !valid {
            return Err(ApiError::BadRequest(format!("'{raw}' is not a valid email address")));
        }
        let name = email.split('@').next().unwrap_or(&email).replace(['.', '_'], " ");
        let row: Option<(String,)> = sqlx::query_as(
            "INSERT INTO users (id, org_id, email, name, status, login_methods) \
             VALUES (gen_random_uuid(), $1, $2, COALESCE((SELECT name FROM accounts WHERE email = $2), $3), 'pending', '{}') \
             ON CONFLICT (org_id, email) DO NOTHING RETURNING id::text",
        )
        .bind(&org)
        .bind(&email)
        .bind(&name)
        .fetch_optional(&mut *tx)
        .await?;
        let Some((id,)) = row else { continue };
        for role in &req.role_ids {
            sqlx::query("INSERT INTO user_roles (user_id, role_id) SELECT $1::uuid, id FROM roles WHERE org_id = $2 AND id = $3 ON CONFLICT DO NOTHING").bind(&id).bind(&org).bind(role).execute(&mut *tx).await?;
        }
        ids.push((id, email));
    }
    tx.commit().await?;
    let mut out = Vec::new();
    for (id, email) in ids {
        let url = super::auth::issue_link(&st, &org, &id, "invite", Some(&auth.user_id), 72).await?;
        auth.audit(&st, "user.invited", &email, "").await;
        out.push(Invitation { id, email, url });
    }
    Ok(Json(out))
}

#[derive(Serialize, ToSchema)]
pub struct LoginLink {
    pub url: String,
}

/// One-time link (24h) that lets the user set a new password — for invites that expired or forgotten passwords.
#[utoipa::path(post, path = "/api/v1/users/{id}/login-link", params(("id" = String, Path)), responses((status = 200, body = LoginLink), (status = 404, body = crate::error::ErrorBody)), tag = "identity")]
pub async fn login_link(State(st): State<AppState>, Admin(auth): Admin, Path(id): Path<String>) -> ApiResult<LoginLink> {
    let user: Option<(String, String)> = sqlx::query_as("SELECT email, status FROM users WHERE org_id = $1 AND id::text = $2").bind(&auth.org_id).bind(&id).fetch_optional(&st.postgres).await?;
    let Some((email, status)) = user else { return Err(ApiError::NotFound(format!("User {id} not found"))) };
    if status == "deactivated" {
        return Err(ApiError::BadRequest("Reactivate the user first".into()));
    }
    let purpose = if status == "pending" { "invite" } else { "reset" };
    if purpose == "reset" {
        // A reset link sets the account's password, which would also open the person's other organizations.
        let (others,): (i64,) = sqlx::query_as(
            "SELECT count(*) FROM users x WHERE x.account_id = (SELECT account_id FROM users WHERE id::text = $1) AND x.org_id <> $2 AND x.status = 'active'",
        )
        .bind(&id)
        .bind(&auth.org_id)
        .fetch_one(&st.postgres)
        .await?;
        if others > 0 {
            return Err(ApiError::BadRequest(format!("{email} also belongs to other organizations, so only they can change their password (Personal Settings → Security).")));
        }
    }
    let url = super::auth::issue_link(&st, &auth.org_id, &id, purpose, Some(&auth.user_id), if purpose == "invite" { 72 } else { 24 }).await?;
    auth.audit(&st, "user.login_link_issued", &email, purpose).await;
    Ok(Json(LoginLink { url }))
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct UserPatch {
    pub status: Option<UserStatus>,
    /// Replaces the user's roles.
    pub role_ids: Option<Vec<String>>,
}

#[utoipa::path(patch, path = "/api/v1/users/{id}", params(("id" = String, Path)), request_body = UserPatch, responses((status = 204), (status = 404, body = crate::error::ErrorBody)), tag = "identity")]
pub async fn update_user(State(st): State<AppState>, Admin(auth): Admin, OrgId(org): OrgId, CurrentUser(me): CurrentUser, Path(id): Path<String>, Json(p): Json<UserPatch>) -> Result<StatusCode, ApiError> {
    if id == me && p.status == Some(UserStatus::Deactivated) {
        return Err(ApiError::BadRequest("you can't deactivate yourself".into()));
    }
    let mut tx = st.postgres.begin().await?;
    let found: Option<(String,)> = sqlx::query_as("SELECT email FROM users WHERE org_id = $1 AND id::text = $2 FOR UPDATE").bind(&org).bind(&id).fetch_optional(&mut *tx).await?;
    let Some((email,)) = found else { return Err(ApiError::NotFound(format!("User {id} not found"))) };
    if id == me && p.role_ids.as_ref().is_some_and(|r| !r.iter().any(|r| *r == format!("{org}-admin"))) {
        return Err(ApiError::BadRequest("you can't remove your own admin role".into()));
    }
    if let Some(s) = p.status {
        sqlx::query("UPDATE users SET status = $2 WHERE id = $1::uuid").bind(&id).bind(s.as_str()).execute(&mut *tx).await?;
        if s == UserStatus::Deactivated {
            // Signed out everywhere; application keys stop working because the user is no longer active.
            sqlx::query("DELETE FROM sessions WHERE user_id = $1::uuid").bind(&id).execute(&mut *tx).await?;
        }
    }
    if let Some(roles) = &p.role_ids {
        sqlx::query("DELETE FROM user_roles WHERE user_id = $1::uuid").bind(&id).execute(&mut *tx).await?;
        sqlx::query("INSERT INTO user_roles (user_id, role_id) SELECT $1::uuid, id FROM roles WHERE org_id = $2 AND id = ANY($3)").bind(&id).bind(&org).bind(roles).execute(&mut *tx).await?;
    }
    tx.commit().await?;
    if let Some(s) = p.status {
        auth.audit(&st, &format!("user.{}", s.as_str()), &email, "").await;
    }
    if let Some(roles) = p.role_ids {
        auth.audit(&st, "user.roles_changed", &email, &roles.join(", ")).await;
    }
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Serialize, ToSchema, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct Role {
    pub id: String,
    pub name: String,
    pub description: String,
    pub managed: bool,
    pub user_count: i64,
}

#[utoipa::path(get, path = "/api/v1/roles", responses((status = 200, body = Vec<Role>)), tag = "identity")]
pub async fn roles(State(st): State<AppState>, OrgId(org): OrgId) -> ApiResult<Vec<Role>> {
    let rows = sqlx::query_as(
        "SELECT r.id, r.name, r.description, r.managed, count(ur.user_id) AS user_count FROM roles r LEFT JOIN user_roles ur ON ur.role_id = r.id \
         WHERE r.org_id = $1 GROUP BY r.id ORDER BY r.managed DESC, r.name",
    )
    .bind(&org)
    .fetch_all(&st.postgres)
    .await?;
    Ok(Json(rows))
}

// ================================================================ teams

#[derive(Deserialize, ToSchema, Clone, Copy)]
#[serde(rename_all = "snake_case")]
pub enum TeamSort {
    Name,
    Members,
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct TeamSearchRequest {
    /// Matches name, handle or description.
    #[serde(default)]
    pub query: String,
    /// Only teams this user (id, or "me") belongs to.
    pub member: Option<String>,
    pub restricted: Option<bool>,
    pub created_via: Option<String>,
    pub has_notifications: Option<bool>,
    pub sort: TeamSort,
    pub offset: i64,
    pub limit: i64,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct Team {
    pub handle: String,
    pub name: String,
    pub description: String,
    pub restricted: bool,
    pub created_via: String,
    pub notifications: Vec<String>,
    pub member_count: i64,
    /// First few members, for avatars.
    pub members: Vec<MemberRef>,
    pub subteams: Vec<String>,
}

#[derive(Serialize, ToSchema, Clone)]
pub struct MemberRef {
    pub id: String,
    pub name: String,
}

#[derive(sqlx::FromRow)]
struct TeamRow {
    handle: String,
    name: String,
    description: String,
    restricted: bool,
    created_via: String,
    notifications: Vec<String>,
    member_count: i64,
    member_ids: Vec<String>,
    member_names: Vec<String>,
    subteams: Vec<String>,
    total: Option<i64>,
}

#[derive(Serialize, ToSchema)]
pub struct TeamSearchPage {
    pub teams: Vec<Team>,
    pub total: i64,
}

#[utoipa::path(post, path = "/api/v1/teams/search", operation_id = "search_teams", request_body = TeamSearchRequest, responses((status = 200, body = TeamSearchPage)), tag = "identity")]
pub async fn search_teams(State(st): State<AppState>, OrgId(org): OrgId, CurrentUser(me): CurrentUser, Json(req): Json<TeamSearchRequest>) -> ApiResult<TeamSearchPage> {
    let mut qb: QueryBuilder<Postgres> = QueryBuilder::new(
        "SELECT t.handle, t.name, t.description, t.restricted, t.created_via, t.notifications, count(m.user_id) AS member_count, \
         COALESCE((array_agg(u.id::text ORDER BY m.role, u.name) FILTER (WHERE u.id IS NOT NULL))[1:3], '{}') AS member_ids, \
         COALESCE((array_agg(u.name ORDER BY m.role, u.name) FILTER (WHERE u.id IS NOT NULL))[1:3], '{}') AS member_names, \
         COALESCE((SELECT array_agg(c.handle ORDER BY c.handle) FROM teams c WHERE c.org_id = t.org_id AND c.parent_handle = t.handle), '{}') AS subteams, \
         count(*) OVER () AS total \
         FROM teams t LEFT JOIN team_members m ON m.org_id = t.org_id AND m.handle = t.handle LEFT JOIN users u ON u.id = m.user_id WHERE t.org_id = ",
    );
    qb.push_bind(&org);
    let text = req.query.trim();
    if !text.is_empty() {
        let p = ilike(text);
        qb.push(" AND (t.name ILIKE ").push_bind(p.clone()).push(" OR t.handle ILIKE ").push_bind(p.clone()).push(" OR t.description ILIKE ").push_bind(p).push(")");
    }
    if let Some(member) = &req.member {
        let id = if member == "me" { me.clone() } else { member.clone() };
        qb.push(" AND t.handle IN (SELECT handle FROM team_members WHERE org_id = t.org_id AND user_id::text = ").push_bind(id).push(")");
    }
    if let Some(r) = req.restricted {
        qb.push(" AND t.restricted = ").push_bind(r);
    }
    if let Some(v) = &req.created_via {
        qb.push(" AND t.created_via = ").push_bind(v.clone());
    }
    if let Some(n) = req.has_notifications {
        qb.push(if n { " AND cardinality(t.notifications) > 0" } else { " AND cardinality(t.notifications) = 0" });
    }
    qb.push(" GROUP BY t.org_id, t.handle");
    qb.push(match req.sort {
        TeamSort::Name => " ORDER BY lower(t.name)",
        TeamSort::Members => " ORDER BY member_count DESC, lower(t.name)",
    });
    qb.push(" LIMIT ").push_bind(req.limit.clamp(1, 200)).push(" OFFSET ").push_bind(req.offset.max(0));
    let rows: Vec<TeamRow> = qb.build_query_as().fetch_all(&st.postgres).await?;
    let total = rows.first().and_then(|r| r.total).unwrap_or(0);
    let teams = rows
        .into_iter()
        .map(|r| Team {
            members: r.member_ids.into_iter().zip(r.member_names).map(|(id, name)| MemberRef { id, name }).collect(),
            handle: r.handle,
            name: r.name,
            description: r.description,
            restricted: r.restricted,
            created_via: r.created_via,
            notifications: r.notifications,
            member_count: r.member_count,
            subteams: r.subteams,
        })
        .collect();
    Ok(Json(TeamSearchPage { teams, total }))
}

#[derive(Deserialize, ToSchema)]
pub struct CreateTeamRequest {
    pub handle: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
}

/// Creates a team with the current user as its admin.
#[utoipa::path(post, path = "/api/v1/teams", request_body = CreateTeamRequest, responses((status = 204), (status = 400, body = crate::error::ErrorBody)), tag = "identity")]
pub async fn create_team(State(st): State<AppState>, _: Writer, OrgId(org): OrgId, CurrentUser(me): CurrentUser, Json(req): Json<CreateTeamRequest>) -> Result<StatusCode, ApiError> {
    let handle = req.handle.trim().to_lowercase();
    if handle.is_empty() || handle.len() > 64 || !handle.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
        return Err(ApiError::BadRequest("handle must be 1–64 characters: letters, digits, '-' or '_'".into()));
    }
    let name = check_name(&req.name, "name", 120)?;
    let mut tx = st.postgres.begin().await?;
    let done = sqlx::query("INSERT INTO teams (org_id, handle, name, description, created_via) VALUES ($1, $2, $3, $4, 'ui') ON CONFLICT DO NOTHING")
        .bind(&org)
        .bind(&handle)
        .bind(&name)
        .bind(req.description.trim())
        .execute(&mut *tx)
        .await?;
    if done.rows_affected() == 0 {
        return Err(ApiError::BadRequest(format!("a team with handle '{handle}' already exists")));
    }
    sqlx::query("INSERT INTO team_members (org_id, handle, user_id, role) VALUES ($1, $2, $3::uuid, 'admin')").bind(&org).bind(&handle).bind(&me).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

// ================================================================ SAML group mappings

#[derive(Serialize, Deserialize, ToSchema, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MappingKind {
    Team,
    Role,
}

impl MappingKind {
    fn as_str(self) -> &'static str {
        match self {
            MappingKind::Team => "team",
            MappingKind::Role => "role",
        }
    }
}

#[derive(Serialize, ToSchema, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct SamlMapping {
    pub id: i64,
    /// Team handle or role id.
    pub target: String,
    pub target_name: String,
    pub attribute_key: String,
    pub attribute_value: String,
    pub created_at: i64,
}

#[derive(Deserialize, IntoParams)]
pub struct MappingParams {
    pub kind: MappingKind,
}

#[utoipa::path(get, path = "/api/v1/saml/mappings", params(MappingParams), responses((status = 200, body = Vec<SamlMapping>)), tag = "identity")]
pub async fn mappings(State(st): State<AppState>, OrgId(org): OrgId, Query(p): Query<MappingParams>) -> ApiResult<Vec<SamlMapping>> {
    let rows = sqlx::query_as(
        "SELECT m.id, m.target, COALESCE(t.handle, r.name, m.target) AS target_name, m.attribute_key, m.attribute_value, \
         (extract(epoch FROM m.created_at) * 1000)::bigint AS created_at \
         FROM saml_mappings m LEFT JOIN teams t ON m.kind = 'team' AND t.org_id = m.org_id AND t.handle = m.target \
         LEFT JOIN roles r ON m.kind = 'role' AND r.id = m.target WHERE m.org_id = $1 AND m.kind = $2 ORDER BY m.created_at",
    )
    .bind(&org)
    .bind(p.kind.as_str())
    .fetch_all(&st.postgres)
    .await?;
    Ok(Json(rows))
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct CreateMappingRequest {
    pub kind: MappingKind,
    pub target: String,
    pub attribute_key: String,
    pub attribute_value: String,
}

#[utoipa::path(post, path = "/api/v1/saml/mappings", request_body = CreateMappingRequest, responses((status = 204), (status = 400, body = crate::error::ErrorBody)), tag = "identity")]
pub async fn create_mapping(State(st): State<AppState>, Admin(auth): Admin, OrgId(org): OrgId, Json(req): Json<CreateMappingRequest>) -> Result<StatusCode, ApiError> {
    let key = check_name(&req.attribute_key, "attribute key", 500)?;
    let value = check_name(&req.attribute_value, "attribute value", 500)?;
    let exists: Option<(i32,)> = match req.kind {
        MappingKind::Team => sqlx::query_as("SELECT 1 FROM teams WHERE org_id = $1 AND handle = $2").bind(&org).bind(&req.target).fetch_optional(&st.postgres).await?,
        MappingKind::Role => sqlx::query_as("SELECT 1 FROM roles WHERE org_id = $1 AND id = $2").bind(&org).bind(&req.target).fetch_optional(&st.postgres).await?,
    };
    if exists.is_none() {
        return Err(ApiError::BadRequest(format!("unknown {} '{}'", req.kind.as_str(), req.target)));
    }
    sqlx::query("INSERT INTO saml_mappings (org_id, kind, target, attribute_key, attribute_value) VALUES ($1, $2, $3, $4, $5)")
        .bind(&org)
        .bind(req.kind.as_str())
        .bind(&req.target)
        .bind(&key)
        .bind(&value)
        .execute(&st.postgres)
        .await?;
    auth.audit(&st, "org.group_mapping_created", &req.target, &format!("{key} = {value}")).await;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(delete, path = "/api/v1/saml/mappings/{id}", params(("id" = i64, Path)), responses((status = 204), (status = 404, body = crate::error::ErrorBody)), tag = "identity")]
pub async fn delete_mapping(State(st): State<AppState>, Admin(auth): Admin, OrgId(org): OrgId, Path(id): Path<i64>) -> Result<StatusCode, ApiError> {
    let done = sqlx::query("DELETE FROM saml_mappings WHERE org_id = $1 AND id = $2").bind(&org).bind(id).execute(&st.postgres).await?;
    if done.rows_affected() == 0 {
        return Err(ApiError::NotFound(format!("Mapping {id} not found")));
    }
    auth.audit(&st, "org.group_mapping_deleted", &id.to_string(), "").await;
    Ok(StatusCode::NO_CONTENT)
}

// ================================================================ audit trail

#[derive(Deserialize, IntoParams)]
pub struct AuditParams {
    /// Matches actor, action, target or detail.
    pub query: Option<String>,
    /// Actors, comma-separated.
    pub actor: Option<String>,
    /// Action categories (`ingest_key` for `ingest_key.revoked`), comma-separated.
    pub category: Option<String>,
    pub offset: Option<i64>,
    pub limit: Option<i64>,
}

/// The non-empty, trimmed values of a comma-separated filter.
fn csv_list(v: Option<String>) -> Vec<String> {
    v.unwrap_or_default().split(',').map(str::trim).filter(|s| !s.is_empty()).map(str::to_string).collect()
}

#[derive(Serialize, ToSchema, sqlx::FromRow)]
pub struct AuditFacetValue {
    pub value: String,
    pub count: i64,
}

#[derive(Serialize, ToSchema, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct AuditEvent {
    pub id: i64,
    pub actor: String,
    pub action: String,
    pub target: String,
    pub detail: String,
    pub ip: Option<String>,
    pub created_at: i64,
    #[serde(skip)]
    pub total: i64,
}

#[derive(Serialize, ToSchema)]
pub struct AuditPage {
    pub events: Vec<AuditEvent>,
    pub total: i64,
    /// Events per actor, ignoring the actor filter.
    pub actors: Vec<AuditFacetValue>,
    /// Events per action category, ignoring the category filter.
    pub categories: Vec<AuditFacetValue>,
}

/// The audit filter after the org: $2 text, $3 its ILIKE pattern, $4 actors, $5 categories (empty = all).
const AUDIT_WHERE: &str = "org_id = $1 AND ($2 = '' OR actor ILIKE $3 OR action ILIKE $3 OR target ILIKE $3 OR detail ILIKE $3) \
    AND (cardinality($4::text[]) = 0 OR actor = ANY($4)) AND (cardinality($5::text[]) = 0 OR split_part(action, '.', 1) = ANY($5))";

/// Audit events per value of `expr` (a column expression constant) under the filter.
async fn audit_facet(st: &AppState, expr: &str, org: &str, text: &str, actors: &[String], categories: &[String]) -> Result<Vec<AuditFacetValue>, ApiError> {
    let sql = format!("SELECT {expr} AS value, count(*) AS count FROM audit_events WHERE {AUDIT_WHERE} GROUP BY 1 ORDER BY 2 DESC, 1 LIMIT 25");
    Ok(sqlx::query_as(&sql).bind(org).bind(text).bind(ilike(text)).bind(actors).bind(categories).fetch_all(&st.postgres).await?)
}

#[utoipa::path(get, path = "/api/v1/audit", params(AuditParams), responses((status = 200, body = AuditPage)), tag = "identity")]
pub async fn audit_trail(State(st): State<AppState>, Admin(auth): Admin, Query(p): Query<AuditParams>) -> ApiResult<AuditPage> {
    let q = p.query.unwrap_or_default();
    let (actors, categories) = (csv_list(p.actor), csv_list(p.category));
    let events: Vec<AuditEvent> = sqlx::query_as(&format!(
        "SELECT id, actor, action, target, detail, ip, (extract(epoch FROM created_at) * 1000)::bigint AS created_at, count(*) OVER () AS total \
         FROM audit_events WHERE {AUDIT_WHERE} ORDER BY created_at DESC, id DESC LIMIT $6 OFFSET $7"
    ))
    .bind(&auth.org_id)
    .bind(q.trim())
    .bind(ilike(q.trim()))
    .bind(&actors)
    .bind(&categories)
    .bind(p.limit.unwrap_or(50).clamp(1, 200))
    .bind(p.offset.unwrap_or(0).max(0))
    .fetch_all(&st.postgres)
    .await?;
    let total = events.first().map_or(0, |e| e.total);
    // Each facet's counts honour every filter but its own.
    let (actor_facet, category_facet) = tokio::try_join!(
        audit_facet(&st, "actor", &auth.org_id, q.trim(), &[], &categories),
        audit_facet(&st, "split_part(action, '.', 1)", &auth.org_id, q.trim(), &actors, &[]),
    )?;
    Ok(Json(AuditPage { events, total, actors: actor_facet, categories: category_facet }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audit_filters_are_comma_separated_values() {
        assert_eq!(csv_list(Some("Alex Morgan, Priya Shah,,".into())), vec!["Alex Morgan".to_string(), "Priya Shah".to_string()]);
        assert!(csv_list(None).is_empty());
        assert!(csv_list(Some(" ".into())).is_empty());
    }
}
