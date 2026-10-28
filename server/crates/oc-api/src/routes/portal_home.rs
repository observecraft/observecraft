//! Developer Portal → Home and Settings.
//!
//! - **Connectors** (Settings → Configurations): one per provider (GitHub, GitLab, Jira, Linear)
//!   and org, configured by admins only. The credential is sealed with the server's secret key and
//!   never returned; clients see whether one is configured and the last connection check.
//! - **Identities**: each member's username (code hosts) or email (trackers) on a provider. Jira
//!   and Linear default to the member's ObserveCraft email.
//! - **Home**: the member's pull requests and tickets, fetched server-side with the org's
//!   connector ([`super::portal_sources`]) and cached for [`CACHE_SECONDS`] per member; Work Items
//!   are the incidents the member commands or responds to; Services & Entities combine APM
//!   services with monitors, Error Tracking issues, dashboards, scorecards and on-call schedules.
//! - **Apps**: org-wide shortcuts to tools (standard users and admins edit them).
//! - **Sources** (Settings → Entities Sources): which telemetry signals the org sent in the last day.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use super::apm::ENTRY;
use super::logs::{bind, scope, TimeRange};
use super::portal_sources::{self as sources, PortalProvider, PortalPullRequest, PortalTicket};
use crate::auth::{Admin, AuthContext, Writer};
use crate::error::{ApiError, ApiResult};
use crate::log_access::LogAccess;
use crate::state::AppState;

pub const CACHE_SECONDS: u64 = 60;
const CACHE_MAX_ENTRIES: usize = 2_000;
const MAX_APPS: i64 = 50;
const MAX_APP_NAME_CHARS: usize = 100;
const MAX_URL_CHARS: usize = 2048;
const MAX_TOKEN_CHARS: usize = 4096;
const MAX_ENTITY_CHARS: usize = 200;
const DAY_MS: i64 = 86_400_000;
/// Services seen on entry spans of this many days are listed.
const ENTITY_DAYS: i64 = 30;

// ================================================================ API types

/// A connector as clients see it: never the credential itself.
#[derive(Serialize, ToSchema, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PortalConnector {
    pub provider: PortalProvider,
    pub configured: bool,
    /// API base URL (the default when not configured).
    pub base_url: String,
    /// Jira: the email of the API token's account.
    pub account: String,
    pub last_checked_at: Option<i64>,
    pub last_check_ok: Option<bool>,
    pub last_check_error: String,
    pub updated_at: Option<i64>,
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct PortalConnectorInput {
    /// Empty for the provider's default (api.github.com, gitlab.com, api.linear.app).
    #[serde(default)]
    pub base_url: String,
    /// Jira: the email the API token belongs to.
    #[serde(default)]
    pub account: String,
    /// Token or API key; left out to keep the stored one.
    #[serde(default)]
    pub token: Option<String>,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct PortalConnectionTest {
    pub ok: bool,
    /// "Connected as …" or what went wrong.
    pub message: String,
    pub checked_at: i64,
}

#[derive(Serialize, ToSchema, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PortalIdentity {
    pub provider: PortalProvider,
    /// Username (GitHub, GitLab) or email (Jira, Linear); empty when unknown.
    pub handle: String,
    /// The handle is the default (the member's email), not one they set.
    pub is_default: bool,
}

#[derive(Deserialize, ToSchema)]
pub struct PortalIdentityInput {
    pub provider: PortalProvider,
    /// Empty to remove the mapping (back to the default).
    pub handle: String,
}

#[derive(Deserialize, ToSchema)]
pub struct PortalIdentitiesInput {
    pub identities: Vec<PortalIdentityInput>,
}

/// Whether a home page list could be read.
#[derive(Serialize, ToSchema, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PortalListStatus {
    Ok,
    /// No connector for the provider.
    NotConnected,
    /// The member's username on the provider is not set.
    IdentityRequired,
    Error,
}

#[derive(Serialize, ToSchema, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct PortalPullRequests {
    pub provider: PortalProvider,
    pub status: PortalListStatus,
    pub message: String,
    /// The member's username on the provider.
    pub user: String,
    pub pull_requests: Vec<PortalPullRequest>,
    pub fetched_at: i64,
}

#[derive(Serialize, ToSchema, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct PortalTickets {
    /// jira | linear | work_items
    pub provider: String,
    pub status: PortalListStatus,
    pub message: String,
    pub user: String,
    pub tickets: Vec<PortalTicket>,
    pub fetched_at: i64,
}

#[derive(Deserialize, IntoParams)]
pub struct PortalPrParams {
    /// github | gitlab
    pub provider: String,
    /// Skip the cache.
    #[serde(default)]
    pub refresh: bool,
}

#[derive(Deserialize, IntoParams)]
pub struct PortalTicketParams {
    /// jira | linear | work_items
    pub provider: String,
    #[serde(default)]
    pub refresh: bool,
}

/// A service on the home page, with what exists around it.
#[derive(Serialize, ToSchema, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PortalHomeEntity {
    pub name: String,
    /// service
    pub kind: String,
    /// Owning team handle; empty when none.
    pub team: String,
    /// Last span (epoch ms); null for services only declared in metadata.
    pub last_seen: Option<i64>,
    /// Last CI deployment or first span of the newest version.
    pub last_deploy: Option<i64>,
    pub monitors: u64,
    /// Open Error Tracking issues of the past 7 days.
    pub issues: u64,
    pub dashboards: u64,
    /// Scorecard level (0–3); null before scorecards are set up.
    pub scorecard_level: Option<u8>,
    /// Name of the owning team's on-call schedule.
    pub on_call: Option<String>,
    pub favorite: bool,
}

#[derive(Serialize, ToSchema, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PortalApp {
    pub id: String,
    pub name: String,
    pub url: String,
    /// Logo registry key (`github`, `tile:servicenow`); empty for a generic icon.
    pub logo: String,
}

#[derive(Deserialize, ToSchema)]
pub struct PortalAppInput {
    pub name: String,
    pub url: String,
    #[serde(default)]
    pub logo: String,
}

#[derive(Serialize, ToSchema, Clone, Debug, PartialEq)]
pub struct PortalSignal {
    /// apm | logs | network | containers
    pub key: String,
    /// The org sent this signal in the last 24 hours.
    pub active: bool,
}

#[derive(Serialize, ToSchema)]
pub struct PortalSources {
    pub signals: Vec<PortalSignal>,
    pub connectors: Vec<PortalConnector>,
}

// ================================================================ cache

/// Short-lived answers per (org, member, list), so the home page doesn't hit providers' rate limits.
struct TtlCache<T> {
    ttl: Duration,
    entries: HashMap<String, (Instant, T)>,
}

impl<T: Clone> TtlCache<T> {
    fn new(ttl: Duration) -> Self {
        TtlCache { ttl, entries: HashMap::new() }
    }

    fn get(&self, key: &str, now: Instant) -> Option<T> {
        self.entries.get(key).filter(|(at, _)| now.duration_since(*at) < self.ttl).map(|(_, v)| v.clone())
    }

    fn put(&mut self, key: String, value: T, now: Instant) {
        if self.entries.len() >= CACHE_MAX_ENTRIES {
            let ttl = self.ttl;
            self.entries.retain(|_, (at, _)| now.duration_since(*at) < ttl);
            if self.entries.len() >= CACHE_MAX_ENTRIES {
                self.entries.clear();
            }
        }
        self.entries.insert(key, (now, value));
    }

    /// Drops every answer of an org (its connectors changed), or of one member.
    fn forget(&mut self, prefix: &str) {
        self.entries.retain(|k, _| !k.starts_with(prefix));
    }
}

fn cache_key(org: &str, user: &str, list: &str) -> String {
    format!("{org}\u{1f}{user}\u{1f}{list}")
}

fn org_prefix(org: &str) -> String {
    format!("{org}\u{1f}")
}

fn pr_cache() -> &'static Mutex<TtlCache<PortalPullRequests>> {
    static CACHE: OnceLock<Mutex<TtlCache<PortalPullRequests>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(TtlCache::new(Duration::from_secs(CACHE_SECONDS))))
}

fn ticket_cache() -> &'static Mutex<TtlCache<PortalTickets>> {
    static CACHE: OnceLock<Mutex<TtlCache<PortalTickets>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(TtlCache::new(Duration::from_secs(CACHE_SECONDS))))
}

fn forget_cached(prefix: &str) {
    if let Ok(mut c) = pr_cache().lock() {
        c.forget(prefix);
    }
    if let Ok(mut c) = ticket_cache().lock() {
        c.forget(prefix);
    }
}

// ================================================================ pure helpers

fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

fn http_url(url: &str) -> Result<String, String> {
    let url = url.trim();
    if url.is_empty() || url.chars().count() > MAX_URL_CHARS {
        return Err("The URL must be 1–2048 characters".into());
    }
    let parsed = url::Url::parse(url).map_err(|_| "The URL must be an http(s) URL".to_string())?;
    if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none() {
        return Err("The URL must be an http(s) URL".into());
    }
    Ok(url.to_string())
}

fn check_app(input: &PortalAppInput) -> Result<PortalAppInput, String> {
    let name = input.name.trim();
    if name.is_empty() || name.chars().count() > MAX_APP_NAME_CHARS {
        return Err(format!("The app name must be 1–{MAX_APP_NAME_CHARS} characters"));
    }
    let logo = input.logo.trim();
    if logo.len() > 60 || !logo.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || ":._-".contains(c)) {
        return Err("Unknown logo".into());
    }
    Ok(PortalAppInput { name: name.into(), url: http_url(&input.url)?, logo: logo.into() })
}

fn check_entity(entity: &str) -> Result<String, String> {
    let e = entity.trim();
    if e.is_empty() || e.chars().count() > MAX_ENTITY_CHARS {
        return Err("Unknown entity".into());
    }
    Ok(e.into())
}

/// Priority dot of an incident severity.
fn severity_level(severity: &str) -> &'static str {
    match severity {
        "SEV-1" => "urgent",
        "SEV-2" => "high",
        "SEV-3" => "medium",
        "SEV-4" | "SEV-5" => "low",
        _ => "none",
    }
}

/// When the service was last deployed: the newest CI deployment, or the first span of its newest
/// version unless that version was already running when the window began (then it's unknown).
fn last_deploy(ci_deploy: Option<i64>, versions: &[(i64, i64)], window_start: i64) -> Option<i64> {
    let newest = versions.iter().filter(|(first, _)| *first > window_start + DAY_MS).map(|(first, _)| *first).max();
    match (ci_deploy, newest) {
        (Some(a), Some(b)) => Some(a.max(b)),
        (a, b) => a.or(b),
    }
}

/// Everything the Services & Entities table combines, read from storage.
#[derive(Default)]
struct EntityInputs {
    /// service → (last span, last deploy)
    seen: BTreeMap<String, (i64, Option<i64>)>,
    /// service → team (service metadata)
    teams: HashMap<String, String>,
    /// service → level, when scorecards are set up
    levels: Option<HashMap<String, u8>>,
    /// (query, tags) of the org's monitors
    monitors: Vec<(String, Vec<String>)>,
    /// definitions of the org's dashboards, as text
    dashboards: Vec<String>,
    issues: HashMap<String, u64>,
    /// team handle → schedule name
    on_call: HashMap<String, String>,
    favorites: HashSet<String>,
}

fn merge_entities(i: &EntityInputs) -> Vec<PortalHomeEntity> {
    let mut names: Vec<&String> = i.seen.keys().chain(i.teams.keys()).chain(i.favorites.iter()).collect();
    names.sort();
    names.dedup();
    let mut out: Vec<PortalHomeEntity> = names
        .into_iter()
        .map(|name| {
            let mentions = |text: &str| super::scorecard_rules::references_service(text, name);
            let team = i.teams.get(name).cloned().unwrap_or_default();
            let (last_seen, last_deploy) = i.seen.get(name).map(|(s, d)| (Some(*s), *d)).unwrap_or((None, None));
            PortalHomeEntity {
                kind: "service".into(),
                monitors: i.monitors.iter().filter(|(q, tags)| mentions(q) || tags.iter().any(|t| mentions(t))).count() as u64,
                dashboards: i.dashboards.iter().filter(|d| mentions(d)).count() as u64,
                issues: i.issues.get(name).copied().unwrap_or(0),
                scorecard_level: i.levels.as_ref().map(|l| l.get(name).copied().unwrap_or(0)),
                on_call: (!team.is_empty()).then(|| i.on_call.get(&team).cloned()).flatten(),
                favorite: i.favorites.contains(name),
                last_seen,
                last_deploy,
                team,
                name: name.clone(),
            }
        })
        .collect();
    // Most recently active first; services only declared in metadata last.
    out.sort_by(|a, b| b.last_seen.cmp(&a.last_seen).then(a.name.cmp(&b.name)));
    out
}

// ================================================================ storage

struct StoredConnector {
    base_url: String,
    account: String,
    secret: String,
}

type ConnectorRow = (String, String, String, Option<i64>, Option<bool>, String, i64);

fn connector_out(provider: PortalProvider, row: Option<&ConnectorRow>) -> PortalConnector {
    match row {
        Some((_, base_url, account, checked, ok, error, updated)) => PortalConnector {
            provider,
            configured: true,
            base_url: base_url.clone(),
            account: account.clone(),
            last_checked_at: *checked,
            last_check_ok: *ok,
            last_check_error: error.clone(),
            updated_at: Some(*updated),
        },
        None => PortalConnector {
            provider,
            configured: false,
            base_url: provider.default_base_url().into(),
            account: String::new(),
            last_checked_at: None,
            last_check_ok: None,
            last_check_error: String::new(),
            updated_at: None,
        },
    }
}

async fn list_connectors(st: &AppState, org: &str) -> Result<Vec<PortalConnector>, ApiError> {
    let rows: Vec<ConnectorRow> = sqlx::query_as(
        "SELECT provider, base_url, account, (extract(epoch FROM last_checked_at) * 1000)::bigint, last_check_ok, last_check_error, (extract(epoch FROM updated_at) * 1000)::bigint \
         FROM portal_connectors WHERE org_id = $1",
    )
    .bind(org)
    .fetch_all(&st.postgres)
    .await?;
    Ok(PortalProvider::ALL.into_iter().map(|p| connector_out(p, rows.iter().find(|r| r.0 == p.as_str()))).collect())
}

async fn stored_connector(st: &AppState, org: &str, provider: PortalProvider) -> Result<Option<StoredConnector>, ApiError> {
    let row: Option<(String, String, String)> =
        sqlx::query_as("SELECT base_url, account, secret_enc FROM portal_connectors WHERE org_id = $1 AND provider = $2").bind(org).bind(provider.as_str()).fetch_optional(&st.postgres).await?;
    match row {
        Some((base_url, account, sealed)) => Ok(Some(StoredConnector { base_url, account, secret: st.secrets.decrypt(&sealed)? })),
        None => Ok(None),
    }
}

async fn member_email(st: &AppState, org: &str, user: &str) -> Result<String, ApiError> {
    let row: Option<(String,)> = sqlx::query_as("SELECT email FROM users WHERE org_id = $1 AND id = $2::uuid").bind(org).bind(user).fetch_optional(&st.postgres).await?;
    Ok(row.map(|(e,)| e).unwrap_or_default())
}

async fn identities(st: &AppState, org: &str, user: &str) -> Result<Vec<PortalIdentity>, ApiError> {
    let rows: Vec<(String, String)> = sqlx::query_as("SELECT provider, handle FROM portal_identities WHERE org_id = $1 AND user_id = $2::uuid").bind(org).bind(user).fetch_all(&st.postgres).await?;
    let email = member_email(st, org, user).await?;
    Ok(PortalProvider::ALL
        .into_iter()
        .map(|p| match rows.iter().find(|(provider, _)| provider == p.as_str()) {
            Some((_, handle)) => PortalIdentity { provider: p, handle: handle.clone(), is_default: false },
            None if p.identity_is_email() => PortalIdentity { provider: p, handle: email.clone(), is_default: true },
            None => PortalIdentity { provider: p, handle: String::new(), is_default: true },
        })
        .collect())
}

async fn identity_of(st: &AppState, org: &str, user: &str, provider: PortalProvider) -> Result<String, ApiError> {
    Ok(identities(st, org, user).await?.into_iter().find(|i| i.provider == provider).map(|i| i.handle).unwrap_or_default())
}

/// Asks the provider who the stored credential belongs to.
async fn check_connection(provider: PortalProvider, c: &StoredConnector, allow_private: bool) -> Result<String, String> {
    let call = match provider {
        PortalProvider::Github => sources::github_user_call(&c.base_url, &c.secret),
        PortalProvider::Gitlab => sources::gitlab_user_call(&c.base_url, &c.secret),
        PortalProvider::Jira => sources::jira_myself_call(&c.base_url, &c.account, &c.secret),
        PortalProvider::Linear => sources::linear_viewer_call(&c.base_url, &c.secret),
    };
    let v = sources::send(&call, allow_private).await?;
    let who = match provider {
        PortalProvider::Github => v["login"].as_str(),
        PortalProvider::Gitlab => v["username"].as_str(),
        PortalProvider::Jira => v["displayName"].as_str().or(v["emailAddress"].as_str()),
        PortalProvider::Linear => {
            sources::graphql_errors(&v)?;
            v.pointer("/data/viewer/name").and_then(|n| n.as_str())
        }
    };
    Ok(match who {
        Some(w) if !w.is_empty() => format!("Connected as {}", w.chars().take(100).collect::<String>()),
        _ => "Connected".into(),
    })
}

async fn fetch_prs(provider: PortalProvider, c: &StoredConnector, login: &str, allow_private: bool) -> Result<Vec<PortalPullRequest>, String> {
    let since = sources::date_days_before(now_ms(), sources::MERGED_DAYS);
    match provider {
        PortalProvider::Github => sources::parse_github_prs(&sources::send(&sources::github_prs_call(&c.base_url, &c.secret, login, &since), allow_private).await?),
        PortalProvider::Gitlab => {
            let [review, authored, merged] = sources::gitlab_mr_calls(&c.base_url, &c.secret, login, &since);
            let (review, authored, merged) = tokio::try_join!(sources::send(&review, allow_private), sources::send(&authored, allow_private), sources::send(&merged, allow_private))?;
            sources::parse_gitlab_mrs(&review, &authored, &merged)
        }
        _ => Err("Not a code host".into()),
    }
}

async fn fetch_tickets(provider: PortalProvider, c: &StoredConnector, email: &str, allow_private: bool) -> Result<Vec<PortalTicket>, String> {
    match provider {
        PortalProvider::Jira => {
            let users = sources::send(&sources::jira_user_search_call(&c.base_url, &c.account, &c.secret, email), allow_private).await?;
            let account = sources::parse_jira_account(&users, email).ok_or_else(|| format!("No Jira user has the email {email}"))?;
            let jql = sources::jira_jql(&account)?;
            sources::parse_jira_issues(&sources::send(&sources::jira_search_call(&c.base_url, &c.account, &c.secret, &jql), allow_private).await?, &c.base_url)
        }
        PortalProvider::Linear => sources::parse_linear_issues(&sources::send(&sources::linear_issues_call(&c.base_url, &c.secret, email), allow_private).await?),
        _ => Err("Not a tracker".into()),
    }
}

/// Incidents the member commands or responds to that aren't resolved.
async fn work_items(st: &AppState, org: &str, user: &str) -> Result<Vec<PortalTicket>, ApiError> {
    type Row = (i32, String, String, String, i64, i64);
    let rows: Vec<Row> = sqlx::query_as(
        "SELECT i.public_id, i.title, i.status, i.severity, (extract(epoch FROM i.declared_at) * 1000)::bigint, (extract(epoch FROM i.modified_at) * 1000)::bigint \
         FROM incidents i WHERE i.org_id = $1 AND i.status <> 'resolved' AND NOT i.test \
         AND (i.commander_id = $2::uuid OR EXISTS (SELECT 1 FROM incident_responders r WHERE r.incident_id = i.id AND r.user_id = $2::uuid)) \
         ORDER BY i.modified_at DESC LIMIT $3",
    )
    .bind(org)
    .bind(user)
    .bind(sources::PAGE_SIZE as i64)
    .fetch_all(&st.postgres)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(id, title, status, severity, created, updated)| PortalTicket {
            key: format!("IR-{id}"),
            title,
            url: format!("/incidents/{id}"),
            status: match status.as_str() {
                "active" => "Active".into(),
                "stable" => "Stable".into(),
                other => other.to_string(),
            },
            status_category: "in_progress".into(),
            created_at: created,
            updated_at: updated,
            priority_level: severity_level(&severity).into(),
            priority: if severity == "UNKNOWN" { String::new() } else { severity },
        })
        .collect())
}

async fn entity_inputs(st: &AppState, ctx: &AuthContext, access: &LogAccess) -> Result<EntityInputs, ApiError> {
    #[derive(clickhouse::Row, Deserialize)]
    struct VersionRow {
        service: String,
        first_seen: i64,
        last_seen: i64,
        versioned: u8,
    }
    let org = ctx.org_id.as_str();
    let now = now_ms();
    let window_start = now - ENTITY_DAYS * DAY_MS;
    let versions = st
        .ch(org)
        .query(&format!(
            "SELECT toString(ServiceName) AS service, toInt64(toUnixTimestamp64Milli(min(Timestamp))) AS first_seen, toInt64(toUnixTimestamp64Milli(max(Timestamp))) AS last_seen, \
             toUInt8(ResourceAttributes['service.version'] != '') AS versioned \
             FROM otel.otel_traces WHERE OrgId = {{oc_org:String}} AND Timestamp >= now() - INTERVAL {ENTITY_DAYS} DAY AND {ENTRY} \
             GROUP BY service, ResourceAttributes['service.version'] LIMIT 20000"
        ))
        .fetch_all::<VersionRow>();
    let deploys = st
        .ch(org)
        .query(&format!(
            "SELECT SpanAttributes['deployment.service.name'] AS service, toInt64(toUnixTimestamp64Milli(max(Timestamp))) AS last \
             FROM otel.otel_ci_traces WHERE OrgId = {{oc_org:String}} AND SpanAttributes['deployment.service.name'] != '' AND SpanAttributes['cicd.pipeline.task.name'] != '' \
             AND Timestamp >= now() - INTERVAL {ENTITY_DAYS} DAY GROUP BY service"
        ))
        .fetch_all::<(String, i64)>();
    let (versions, deploys) = tokio::try_join!(versions, deploys)?;
    let deploys: HashMap<String, i64> = deploys.into_iter().collect();
    let mut per_service: BTreeMap<String, (i64, Vec<(i64, i64)>)> = BTreeMap::new();
    for v in versions.into_iter().filter(|v| !v.service.is_empty()) {
        let e = per_service.entry(v.service).or_default();
        e.0 = e.0.max(v.last_seen);
        if v.versioned == 1 {
            e.1.push((v.first_seen, v.last_seen));
        }
    }
    let seen = per_service.into_iter().map(|(s, (last, versions))| {
        let deploy = last_deploy(deploys.get(&s).copied(), &versions, window_start);
        (s, (last, deploy))
    });

    let teams: Vec<(String, String)> = sqlx::query_as("SELECT service, team FROM service_metadata WHERE org_id = $1").bind(org).fetch_all(&st.postgres).await?;
    let monitors: Vec<(String, Vec<String>)> = sqlx::query_as("SELECT query, tags FROM monitors WHERE org_id = $1 AND NOT draft").bind(org).fetch_all(&st.postgres).await?;
    let dashboards: Vec<(String,)> = sqlx::query_as("SELECT definition::text FROM dashboards WHERE org_id = $1 AND deleted_at IS NULL").bind(org).fetch_all(&st.postgres).await?;
    let on_call: Vec<(String, String)> =
        sqlx::query_as("SELECT team_handle, min(name) FROM on_call_schedules WHERE org_id = $1 AND team_handle IS NOT NULL GROUP BY team_handle").bind(org).fetch_all(&st.postgres).await?;
    let favorites: Vec<(String,)> = sqlx::query_as("SELECT entity FROM portal_favorites WHERE org_id = $1 AND user_id = $2::uuid").bind(org).bind(&ctx.user_id).fetch_all(&st.postgres).await?;
    let overview = super::scorecards::overview(st, org).await?;
    let levels = overview.set_up.then(|| overview.services.iter().map(|s| (s.service.clone(), s.level)).collect());
    let issues = super::error_tracking::open_issues_by_service(st, org, access, TimeRange { from: now - 7 * DAY_MS, to: now }).await?;
    Ok(EntityInputs {
        seen: seen.collect(),
        teams: teams.into_iter().filter(|(_, t)| !t.is_empty()).collect(),
        levels,
        monitors,
        dashboards: dashboards.into_iter().map(|(d,)| d).collect(),
        issues,
        on_call: on_call.into_iter().collect(),
        favorites: favorites.into_iter().map(|(f,)| f).collect(),
    })
}

/// Whether the org sent each signal in the last 24 hours.
async fn signals(st: &AppState, org: &str, access: &LogAccess) -> Result<Vec<PortalSignal>, ApiError> {
    let ch = st.ch(org);
    let apm = ch.query("SELECT count() FROM (SELECT 1 FROM otel.otel_traces WHERE OrgId = {oc_org:String} AND Timestamp >= now() - INTERVAL 1 DAY LIMIT 1)").fetch_one::<u64>();
    let network = ch.query("SELECT count() FROM (SELECT 1 FROM otel.otel_network WHERE OrgId = {oc_org:String} AND Timestamp >= now() - INTERVAL 1 DAY LIMIT 1)").fetch_one::<u64>();
    let containers = ch
        .query("SELECT count() FROM (SELECT 1 FROM otel.metrics_series WHERE OrgId = {oc_org:String} AND startsWith(MetricName, 'container.') AND LastSeen >= now() - INTERVAL 1 DAY LIMIT 1)")
        .fetch_one::<u64>();
    // Logs the reader may see.
    let now = now_ms();
    let logs_scope = scope(org, access, TimeRange { from: now - DAY_MS, to: now }, &[], None);
    let logs = bind(ch.query(&format!("SELECT count() FROM (SELECT 1 FROM {} WHERE {} LIMIT 1)", logs_scope.table, logs_scope.clause)), &logs_scope.params).fetch_one::<u64>();
    let (apm, network, containers, logs) = tokio::try_join!(apm, network, containers, logs)?;
    Ok([("apm", apm), ("logs", logs), ("network", network), ("containers", containers)].into_iter().map(|(key, n)| PortalSignal { key: key.into(), active: n > 0 }).collect())
}

fn parse_provider(s: &str) -> Result<PortalProvider, ApiError> {
    PortalProvider::parse(s).ok_or_else(|| ApiError::NotFound(format!("Unknown provider {s}")))
}

fn valid_uuid(s: &str) -> bool {
    s.len() == 36 && s.chars().all(|c| c.is_ascii_hexdigit() || c == '-')
}

// ================================================================ handlers: connectors and identities

/// Every provider's connector status; credentials are never returned.
#[utoipa::path(get, path = "/api/v1/portal/connectors", operation_id = "list_portal_connectors", responses((status = 200, body = Vec<PortalConnector>)), tag = "portal")]
pub async fn get_connectors(State(st): State<AppState>, ctx: AuthContext) -> ApiResult<Vec<PortalConnector>> {
    Ok(Json(list_connectors(&st, &ctx.org_id).await?))
}

/// Configures a provider (admins). The token is sealed at rest; leave it out to keep the stored one.
#[utoipa::path(put, path = "/api/v1/portal/connectors/{provider}", operation_id = "put_portal_connector", params(("provider" = String, Path, description = "github | gitlab | jira | linear")),
    request_body = PortalConnectorInput, responses((status = 200, body = PortalConnector), (status = 400, body = crate::error::ErrorBody)), tag = "portal")]
pub async fn put_connector(State(st): State<AppState>, Admin(ctx): Admin, Path(provider): Path<String>, Json(input): Json<PortalConnectorInput>) -> ApiResult<PortalConnector> {
    let provider = parse_provider(&provider)?;
    let allow_private = st.config.portal_allow_private;
    let base_url = sources::check_base_url(provider, &input.base_url, allow_private).map_err(ApiError::BadRequest)?;
    let account = input.account.trim().to_string();
    if provider == PortalProvider::Jira {
        sources::check_identity(PortalProvider::Jira, &account).map_err(|_| ApiError::BadRequest("Jira needs the email the API token belongs to".into()))?;
    }
    let token = input.token.as_deref().map(str::trim).filter(|t| !t.is_empty());
    if token.is_some_and(|t| t.len() > MAX_TOKEN_CHARS || t.chars().any(char::is_control)) {
        return Err(ApiError::BadRequest("The token is not valid".into()));
    }
    let existing = stored_connector(&st, &ctx.org_id, provider).await?;
    let sealed = match (token, &existing) {
        (Some(t), _) => st.secrets.encrypt(t),
        (None, Some(e)) => st.secrets.encrypt(&e.secret),
        (None, None) => return Err(ApiError::BadRequest(format!("{} needs a token", provider.label()))),
    };
    let account = if provider == PortalProvider::Jira { account } else { String::new() };
    sqlx::query(
        "INSERT INTO portal_connectors (org_id, provider, base_url, account, secret_enc, updated_by) VALUES ($1, $2, $3, $4, $5, $6::uuid) \
         ON CONFLICT (org_id, provider) DO UPDATE SET base_url = EXCLUDED.base_url, account = EXCLUDED.account, secret_enc = EXCLUDED.secret_enc, \
         updated_by = EXCLUDED.updated_by, updated_at = now(), last_checked_at = NULL, last_check_ok = NULL, last_check_error = ''",
    )
    .bind(&ctx.org_id)
    .bind(provider.as_str())
    .bind(&base_url)
    .bind(&account)
    .bind(sealed)
    .bind(&ctx.user_id)
    .execute(&st.postgres)
    .await?;
    forget_cached(&org_prefix(&ctx.org_id));
    let detail = format!("{base_url}{}", if token.is_some() { ", token replaced" } else { "" });
    ctx.audit(&st, "portal.connector_saved", provider.as_str(), &detail).await;
    let all = list_connectors(&st, &ctx.org_id).await?;
    Ok(Json(all.into_iter().find(|c| c.provider == provider).ok_or_else(|| ApiError::NotFound("No such connector".into()))?))
}

/// Removes a provider's connector and its stored credential (admins).
#[utoipa::path(delete, path = "/api/v1/portal/connectors/{provider}", operation_id = "delete_portal_connector", params(("provider" = String, Path, description = "github | gitlab | jira | linear")),
    responses((status = 204)), tag = "portal")]
pub async fn delete_connector(State(st): State<AppState>, Admin(ctx): Admin, Path(provider): Path<String>) -> Result<StatusCode, ApiError> {
    let provider = parse_provider(&provider)?;
    let done = sqlx::query("DELETE FROM portal_connectors WHERE org_id = $1 AND provider = $2").bind(&ctx.org_id).bind(provider.as_str()).execute(&st.postgres).await?;
    if done.rows_affected() == 0 {
        return Err(ApiError::NotFound(format!("{} is not configured", provider.label())));
    }
    forget_cached(&org_prefix(&ctx.org_id));
    ctx.audit(&st, "portal.connector_deleted", provider.as_str(), "").await;
    Ok(StatusCode::NO_CONTENT)
}

/// Checks the stored credential against the provider and records the outcome (admins).
#[utoipa::path(post, path = "/api/v1/portal/connectors/{provider}/test", operation_id = "test_portal_connector", params(("provider" = String, Path, description = "github | gitlab | jira | linear")),
    responses((status = 200, body = PortalConnectionTest)), tag = "portal")]
pub async fn test_connector(State(st): State<AppState>, Admin(ctx): Admin, Path(provider): Path<String>) -> ApiResult<PortalConnectionTest> {
    let provider = parse_provider(&provider)?;
    let c = stored_connector(&st, &ctx.org_id, provider).await?.ok_or_else(|| ApiError::NotFound(format!("{} is not configured", provider.label())))?;
    let outcome = check_connection(provider, &c, st.config.portal_allow_private).await;
    let (ok, message) = match &outcome {
        Ok(m) => (true, m.clone()),
        Err(e) => (false, e.clone()),
    };
    sqlx::query("UPDATE portal_connectors SET last_checked_at = now(), last_check_ok = $3, last_check_error = $4 WHERE org_id = $1 AND provider = $2")
        .bind(&ctx.org_id)
        .bind(provider.as_str())
        .bind(ok)
        .bind(if ok { "" } else { message.as_str() })
        .execute(&st.postgres)
        .await?;
    Ok(Json(PortalConnectionTest { ok, message, checked_at: now_ms() }))
}

/// The current member's identity on each provider.
#[utoipa::path(get, path = "/api/v1/portal/identities", operation_id = "list_portal_identities", responses((status = 200, body = Vec<PortalIdentity>)), tag = "portal")]
pub async fn get_identities(State(st): State<AppState>, ctx: AuthContext) -> ApiResult<Vec<PortalIdentity>> {
    Ok(Json(identities(&st, &ctx.org_id, &ctx.user_id).await?))
}

/// Sets the current member's identities; an empty handle goes back to the default.
#[utoipa::path(put, path = "/api/v1/portal/identities", operation_id = "put_portal_identities", request_body = PortalIdentitiesInput,
    responses((status = 200, body = Vec<PortalIdentity>), (status = 400, body = crate::error::ErrorBody)), tag = "portal")]
pub async fn put_identities(State(st): State<AppState>, ctx: AuthContext, Json(input): Json<PortalIdentitiesInput>) -> ApiResult<Vec<PortalIdentity>> {
    let mut checked = Vec::new();
    for i in input.identities.iter().take(PortalProvider::ALL.len()) {
        let handle = if i.handle.trim().is_empty() { None } else { Some(sources::check_identity(i.provider, &i.handle).map_err(ApiError::BadRequest)?) };
        checked.push((i.provider, handle));
    }
    let mut tx = st.postgres.begin().await?;
    for (provider, handle) in checked {
        match handle {
            Some(h) => {
                sqlx::query(
                    "INSERT INTO portal_identities (org_id, user_id, provider, handle) VALUES ($1, $2::uuid, $3, $4) \
                     ON CONFLICT (org_id, user_id, provider) DO UPDATE SET handle = EXCLUDED.handle, updated_at = now()",
                )
                .bind(&ctx.org_id)
                .bind(&ctx.user_id)
                .bind(provider.as_str())
                .bind(h)
                .execute(&mut *tx)
                .await?;
            }
            None => {
                sqlx::query("DELETE FROM portal_identities WHERE org_id = $1 AND user_id = $2::uuid AND provider = $3")
                    .bind(&ctx.org_id)
                    .bind(&ctx.user_id)
                    .bind(provider.as_str())
                    .execute(&mut *tx)
                    .await?;
            }
        }
    }
    tx.commit().await?;
    forget_cached(&cache_key(&ctx.org_id, &ctx.user_id, ""));
    Ok(Json(identities(&st, &ctx.org_id, &ctx.user_id).await?))
}

// ================================================================ handlers: home

/// The member's pull requests on GitHub or GitLab, grouped (cached for a minute).
#[utoipa::path(get, path = "/api/v1/portal/home/prs", operation_id = "portal_pull_requests", params(PortalPrParams), responses((status = 200, body = PortalPullRequests)), tag = "portal")]
pub async fn pull_requests(State(st): State<AppState>, ctx: AuthContext, Query(p): Query<PortalPrParams>) -> ApiResult<PortalPullRequests> {
    let provider = parse_provider(&p.provider)?;
    if !provider.is_code_host() {
        return Err(ApiError::BadRequest(format!("{} has no pull requests", provider.label())));
    }
    let key = cache_key(&ctx.org_id, &ctx.user_id, provider.as_str());
    if !p.refresh {
        if let Some(hit) = pr_cache().lock().ok().and_then(|c| c.get(&key, Instant::now())) {
            return Ok(Json(hit));
        }
    }
    let user = identity_of(&st, &ctx.org_id, &ctx.user_id, provider).await?;
    let out = |status, message: String, prs| PortalPullRequests { provider, status, message, user: user.clone(), pull_requests: prs, fetched_at: now_ms() };
    let Some(c) = stored_connector(&st, &ctx.org_id, provider).await? else {
        return Ok(Json(out(PortalListStatus::NotConnected, format!("Connect {} to see your pull requests", provider.label()), vec![])));
    };
    if user.is_empty() {
        return Ok(Json(out(PortalListStatus::IdentityRequired, format!("Set your {} username to see your pull requests", provider.label()), vec![])));
    }
    let answer = match fetch_prs(provider, &c, &user, st.config.portal_allow_private).await {
        Ok(prs) => out(PortalListStatus::Ok, String::new(), prs),
        Err(e) => out(PortalListStatus::Error, e, vec![]),
    };
    if let Ok(mut cache) = pr_cache().lock() {
        cache.put(key, answer.clone(), Instant::now());
    }
    Ok(Json(answer))
}

/// The member's open Jira or Linear tickets, or the incidents they work on (`work_items`).
#[utoipa::path(get, path = "/api/v1/portal/home/tickets", operation_id = "portal_tickets", params(PortalTicketParams), responses((status = 200, body = PortalTickets)), tag = "portal")]
pub async fn tickets(State(st): State<AppState>, ctx: AuthContext, Query(p): Query<PortalTicketParams>) -> ApiResult<PortalTickets> {
    if p.provider == "work_items" {
        let items = work_items(&st, &ctx.org_id, &ctx.user_id).await?;
        return Ok(Json(PortalTickets { provider: p.provider, status: PortalListStatus::Ok, message: String::new(), user: ctx.name.clone(), tickets: items, fetched_at: now_ms() }));
    }
    let provider = parse_provider(&p.provider)?;
    if provider.is_code_host() {
        return Err(ApiError::BadRequest(format!("{} has no tickets", provider.label())));
    }
    let key = cache_key(&ctx.org_id, &ctx.user_id, provider.as_str());
    if !p.refresh {
        if let Some(hit) = ticket_cache().lock().ok().and_then(|c| c.get(&key, Instant::now())) {
            return Ok(Json(hit));
        }
    }
    let user = identity_of(&st, &ctx.org_id, &ctx.user_id, provider).await?;
    let out = |status, message: String, tickets| PortalTickets { provider: provider.as_str().into(), status, message, user: user.clone(), tickets, fetched_at: now_ms() };
    let Some(c) = stored_connector(&st, &ctx.org_id, provider).await? else {
        return Ok(Json(out(PortalListStatus::NotConnected, format!("See your {} work items here", provider.label()), vec![])));
    };
    if user.is_empty() {
        return Ok(Json(out(PortalListStatus::IdentityRequired, format!("Set your {} email to see your tickets", provider.label()), vec![])));
    }
    let answer = match fetch_tickets(provider, &c, &user, st.config.portal_allow_private).await {
        Ok(t) => out(PortalListStatus::Ok, String::new(), t),
        Err(e) => out(PortalListStatus::Error, e, vec![]),
    };
    if let Ok(mut cache) = ticket_cache().lock() {
        cache.put(key, answer.clone(), Instant::now());
    }
    Ok(Json(answer))
}

/// Services with their monitors, open issues, dashboards, scorecard level and on-call schedule.
#[utoipa::path(get, path = "/api/v1/portal/home/entities", operation_id = "portal_entities", responses((status = 200, body = Vec<PortalHomeEntity>)), tag = "portal")]
pub async fn entities(State(st): State<AppState>, ctx: AuthContext, access: LogAccess) -> ApiResult<Vec<PortalHomeEntity>> {
    let inputs = entity_inputs(&st, &ctx, &access).await?;
    Ok(Json(merge_entities(&inputs)))
}

/// Stars an entity for the current member.
#[utoipa::path(put, path = "/api/v1/portal/favorites/{entity}", operation_id = "add_portal_favorite", params(("entity" = String, Path)), responses((status = 204)), tag = "portal")]
pub async fn add_favorite(State(st): State<AppState>, ctx: AuthContext, Path(entity): Path<String>) -> Result<StatusCode, ApiError> {
    let entity = check_entity(&entity).map_err(ApiError::BadRequest)?;
    let (count,): (i64,) = sqlx::query_as("SELECT count(*) FROM portal_favorites WHERE org_id = $1 AND user_id = $2::uuid").bind(&ctx.org_id).bind(&ctx.user_id).fetch_one(&st.postgres).await?;
    if count >= 500 {
        return Err(ApiError::BadRequest("At most 500 favorites".into()));
    }
    sqlx::query("INSERT INTO portal_favorites (org_id, user_id, entity) VALUES ($1, $2::uuid, $3) ON CONFLICT DO NOTHING")
        .bind(&ctx.org_id)
        .bind(&ctx.user_id)
        .bind(entity)
        .execute(&st.postgres)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Removes a star.
#[utoipa::path(delete, path = "/api/v1/portal/favorites/{entity}", operation_id = "remove_portal_favorite", params(("entity" = String, Path)), responses((status = 204)), tag = "portal")]
pub async fn remove_favorite(State(st): State<AppState>, ctx: AuthContext, Path(entity): Path<String>) -> Result<StatusCode, ApiError> {
    sqlx::query("DELETE FROM portal_favorites WHERE org_id = $1 AND user_id = $2::uuid AND entity = $3").bind(&ctx.org_id).bind(&ctx.user_id).bind(entity.trim()).execute(&st.postgres).await?;
    Ok(StatusCode::NO_CONTENT)
}

// ================================================================ handlers: apps and sources

async fn list_apps(st: &AppState, org: &str) -> Result<Vec<PortalApp>, ApiError> {
    let rows: Vec<(String, String, String, String)> =
        sqlx::query_as("SELECT id::text, name, url, logo FROM portal_apps WHERE org_id = $1 ORDER BY position, created_at").bind(org).fetch_all(&st.postgres).await?;
    Ok(rows.into_iter().map(|(id, name, url, logo)| PortalApp { id, name, url, logo }).collect())
}

/// The org's app shortcuts.
#[utoipa::path(get, path = "/api/v1/portal/apps", operation_id = "list_portal_apps", responses((status = 200, body = Vec<PortalApp>)), tag = "portal")]
pub async fn get_apps(State(st): State<AppState>, ctx: AuthContext) -> ApiResult<Vec<PortalApp>> {
    Ok(Json(list_apps(&st, &ctx.org_id).await?))
}

#[utoipa::path(post, path = "/api/v1/portal/apps", operation_id = "create_portal_app", request_body = PortalAppInput,
    responses((status = 200, body = Vec<PortalApp>), (status = 400, body = crate::error::ErrorBody)), tag = "portal")]
pub async fn create_app(State(st): State<AppState>, Writer(ctx): Writer, Json(input): Json<PortalAppInput>) -> ApiResult<Vec<PortalApp>> {
    let app = check_app(&input).map_err(ApiError::BadRequest)?;
    let (count,): (i64,) = sqlx::query_as("SELECT count(*) FROM portal_apps WHERE org_id = $1").bind(&ctx.org_id).fetch_one(&st.postgres).await?;
    if count >= MAX_APPS {
        return Err(ApiError::BadRequest(format!("An organization can have at most {MAX_APPS} apps")));
    }
    let (id,): (String,) = sqlx::query_as("INSERT INTO portal_apps (org_id, name, url, logo, position, created_by) VALUES ($1, $2, $3, $4, $5, $6::uuid) RETURNING id::text")
        .bind(&ctx.org_id)
        .bind(&app.name)
        .bind(&app.url)
        .bind(&app.logo)
        .bind(count as i32)
        .bind(&ctx.user_id)
        .fetch_one(&st.postgres)
        .await?;
    ctx.audit(&st, "portal.app_created", &id, &format!("{} ({})", app.name, app.url)).await;
    Ok(Json(list_apps(&st, &ctx.org_id).await?))
}

#[utoipa::path(put, path = "/api/v1/portal/apps/{id}", operation_id = "update_portal_app", params(("id" = String, Path)), request_body = PortalAppInput,
    responses((status = 200, body = Vec<PortalApp>), (status = 404, body = crate::error::ErrorBody)), tag = "portal")]
pub async fn update_app(State(st): State<AppState>, Writer(ctx): Writer, Path(id): Path<String>, Json(input): Json<PortalAppInput>) -> ApiResult<Vec<PortalApp>> {
    let app = check_app(&input).map_err(ApiError::BadRequest)?;
    let missing = || ApiError::NotFound("No such app".into());
    if !valid_uuid(&id) {
        return Err(missing());
    }
    let done = sqlx::query("UPDATE portal_apps SET name = $3, url = $4, logo = $5 WHERE org_id = $1 AND id = $2::uuid")
        .bind(&ctx.org_id)
        .bind(&id)
        .bind(&app.name)
        .bind(&app.url)
        .bind(&app.logo)
        .execute(&st.postgres)
        .await?;
    if done.rows_affected() == 0 {
        return Err(missing());
    }
    ctx.audit(&st, "portal.app_updated", &id, &format!("{} ({})", app.name, app.url)).await;
    Ok(Json(list_apps(&st, &ctx.org_id).await?))
}

#[utoipa::path(delete, path = "/api/v1/portal/apps/{id}", operation_id = "delete_portal_app", params(("id" = String, Path)), responses((status = 204)), tag = "portal")]
pub async fn delete_app(State(st): State<AppState>, Writer(ctx): Writer, Path(id): Path<String>) -> Result<StatusCode, ApiError> {
    let missing = || ApiError::NotFound("No such app".into());
    if !valid_uuid(&id) {
        return Err(missing());
    }
    let name: Option<(String,)> = sqlx::query_as("DELETE FROM portal_apps WHERE org_id = $1 AND id = $2::uuid RETURNING name").bind(&ctx.org_id).bind(&id).fetch_optional(&st.postgres).await?;
    let (name,) = name.ok_or_else(missing)?;
    ctx.audit(&st, "portal.app_deleted", &id, &name).await;
    Ok(StatusCode::NO_CONTENT)
}

/// Which telemetry signals feed the catalog (last 24 hours) and the external source connectors.
#[utoipa::path(get, path = "/api/v1/portal/sources", operation_id = "portal_sources", responses((status = 200, body = PortalSources)), tag = "portal")]
pub async fn get_sources(State(st): State<AppState>, ctx: AuthContext, access: LogAccess) -> ApiResult<PortalSources> {
    let (signals, connectors) = tokio::try_join!(signals(&st, &ctx.org_id, &access), list_connectors(&st, &ctx.org_id))?;
    Ok(Json(PortalSources { signals, connectors }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_expires_and_forgets_by_org() {
        let mut c: TtlCache<u32> = TtlCache::new(Duration::from_secs(60));
        let t0 = Instant::now();
        c.put(cache_key("org-a", "u1", "github"), 1, t0);
        c.put(cache_key("org-ab", "u1", "github"), 2, t0);
        c.put(cache_key("org-a", "u2", "jira"), 3, t0);
        assert_eq!(c.get(&cache_key("org-a", "u1", "github"), t0 + Duration::from_secs(59)), Some(1));
        assert_eq!(c.get(&cache_key("org-a", "u1", "github"), t0 + Duration::from_secs(60)), None);
        c.forget(&cache_key("org-a", "u2", ""));
        assert_eq!(c.get(&cache_key("org-a", "u2", "jira"), t0), None);
        assert_eq!(c.get(&cache_key("org-a", "u1", "github"), t0), Some(1));
        // "org-a" must not clear "org-ab".
        c.forget(&org_prefix("org-a"));
        assert_eq!(c.get(&cache_key("org-a", "u1", "github"), t0), None);
        assert_eq!(c.get(&cache_key("org-ab", "u1", "github"), t0), Some(2));
    }

    #[test]
    fn apps_are_validated() {
        let app = |name: &str, url: &str, logo: &str| check_app(&PortalAppInput { name: name.into(), url: url.into(), logo: logo.into() });
        assert_eq!(app(" Runbooks ", "https://wiki.example.com/runbooks", "confluence").unwrap().name, "Runbooks");
        assert!(app("", "https://x.example.com", "").is_err());
        assert!(app("x", "javascript:alert(1)", "").is_err());
        assert!(app("x", "ftp://files.example.com", "").is_err());
        assert!(app("x", "https://x.example.com", "<svg>").is_err());
        assert!(app("x", "https://x.example.com", "tile:servicenow").is_ok());
    }

    #[test]
    fn work_item_priorities() {
        assert_eq!(severity_level("SEV-1"), "urgent");
        assert_eq!(severity_level("SEV-3"), "medium");
        assert_eq!(severity_level("UNKNOWN"), "none");
    }

    #[test]
    fn last_deploy_prefers_known_changes() {
        let start = 1_000 * DAY_MS;
        // A version already running when the window began says nothing about deploys.
        assert_eq!(last_deploy(None, &[(start + 1, start + 5 * DAY_MS)], start), None);
        assert_eq!(last_deploy(None, &[(start + 1, start + 2 * DAY_MS), (start + 3 * DAY_MS, start + 9 * DAY_MS)], start), Some(start + 3 * DAY_MS));
        assert_eq!(last_deploy(Some(start + 5 * DAY_MS), &[(start + 3 * DAY_MS, start + 9 * DAY_MS)], start), Some(start + 5 * DAY_MS));
        assert_eq!(last_deploy(Some(start + 2 * DAY_MS), &[], start), Some(start + 2 * DAY_MS));
    }

    #[test]
    fn entities_combine_every_source() {
        let inputs = EntityInputs {
            seen: BTreeMap::from([("order-service".to_string(), (2_000, Some(1_500))), ("billing".to_string(), (3_000, None))]),
            teams: HashMap::from([("order-service".to_string(), "checkout".to_string()), ("catalog-service".to_string(), "catalog".to_string())]),
            levels: Some(HashMap::from([("order-service".to_string(), 2)])),
            monitors: vec![("avg:trace.error_rate{service:order-service} > 5".into(), vec![]), ("logs(\"status:error\")".into(), vec!["service:billing".into()]), ("avg:system.cpu{*}".into(), vec![])],
            dashboards: vec![r#"{"widgets":[{"q":"service:order-service"}]}"#.into(), r#"{"widgets":[{"q":"service:order-service-canary"}]}"#.into()],
            issues: HashMap::from([("billing".to_string(), 4)]),
            on_call: HashMap::from([("checkout".to_string(), "Checkout primary".to_string())]),
            favorites: HashSet::from(["catalog-service".to_string()]),
        };
        let e = merge_entities(&inputs);
        assert_eq!(e.iter().map(|x| x.name.as_str()).collect::<Vec<_>>(), ["billing", "order-service", "catalog-service"]);
        let order = &e[1];
        assert_eq!((order.team.as_str(), order.monitors, order.dashboards, order.issues), ("checkout", 1, 1, 0));
        assert_eq!((order.scorecard_level, order.on_call.as_deref(), order.last_deploy), (Some(2), Some("Checkout primary"), Some(1_500)));
        assert_eq!((e[0].monitors, e[0].issues, e[0].scorecard_level, e[0].on_call.as_deref()), (1, 4, Some(0), None));
        assert!(e[2].favorite && e[2].last_seen.is_none());
        let no_scorecards = merge_entities(&EntityInputs { levels: None, ..inputs });
        assert!(no_scorecards.iter().all(|x| x.scorecard_level.is_none()));
    }
}
