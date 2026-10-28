//! Log data access (Logs → Configuration → Data Access): restriction queries limit which of the
//! org's logs a user reads.
//!
//! A restriction query is a log search (`service:billing`, `env:prod kube_namespace:payments`)
//! assigned to roles and/or teams. The rules, deny by default once an org opts in:
//! - an org without restriction queries: everyone with log access reads all of its logs;
//! - once it has at least one: org admins still read everything, and every other user reads only
//!   the logs matching at least one of the queries assigned to one of their roles or (direct) teams.
//!   A user with no query assigned reads no logs.
//!
//! Why not "unrestricted unless a query applies": with that rule a new user, a new role, a removed
//! assignment or a deleted role or team silently widens access to everything. Here each of those
//! can only narrow it, and widening always takes an explicit assignment.
//!
//! Enforcement: [`LogAccess`] is resolved per request from the session or API key (it is an axum
//! extractor), and `routes::logs::scope`, which every log read builds its WHERE clause with, requires
//! one and ANDs its filter in. Live tail and stored archive search hits use [`LogAccess::allows`];
//! archive searches run in DuckDB use [`LogAccess::duckdb_filter`]. Only the org's own background
//! jobs (monitor evaluation, the scorecards' per-service log counts) use [`LogAccess::org_background`].

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::async_trait;
use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use oc_query::{build_any, build_any_duckdb, matches_any, parse_query, Fields, LogsSchema, SqlFilter, Term};

use crate::auth::{Access, AuthContext};
use crate::error::ApiError;
use crate::state::AppState;

pub const MAX_QUERY_CHARS: usize = 2_000;
/// How long a server reuses an org's restriction queries; changes made on this server apply at once.
const CACHE_TTL: Duration = Duration::from_secs(10);

/// One restriction query and who it is assigned to.
#[derive(Debug, Clone)]
pub struct Restriction {
    pub terms: Vec<Term>,
    /// Role ids.
    pub roles: HashSet<String>,
    /// Team handles.
    pub teams: HashSet<String>,
}

/// Which of the org's logs a request may read.
#[derive(Debug, Clone)]
pub struct LogAccess {
    /// `None`: every log of the org. `Some`: only logs matching one of these searches (none: no logs).
    only: Option<Arc<Vec<Vec<Term>>>>,
}

impl LogAccess {
    /// Applies the rules in the module docs to a user (`admin`: holds the org admin role).
    pub fn resolve(restrictions: &[Restriction], admin: bool, roles: &[String], teams: &[String]) -> LogAccess {
        if restrictions.is_empty() || admin {
            return LogAccess { only: None };
        }
        let searches = restrictions.iter().filter(|r| roles.iter().any(|id| r.roles.contains(id)) || teams.iter().any(|t| r.teams.contains(t))).map(|r| r.terms.clone()).collect();
        LogAccess { only: Some(Arc::new(searches)) }
    }

    /// The org's own background work (monitor evaluation): every log of the org. Never use it for
    /// data served to a user; a guard test lists where it may appear.
    pub fn org_background() -> LogAccess {
        LogAccess { only: None }
    }

    /// Whether the reader sees only part of the org's logs.
    pub fn restricted(&self) -> bool {
        self.only.is_some()
    }

    /// The searches a restricted reader's logs match (`None`: unrestricted).
    pub fn searches(&self) -> Option<&[Vec<Term>]> {
        self.only.as_deref().map(Vec::as_slice)
    }

    /// The extra WHERE condition for ClickHouse log tables, with bound parameters (`None`: unrestricted).
    pub fn clickhouse_filter(&self) -> Option<SqlFilter> {
        self.searches().map(|s| build_any(&LogsSchema, s))
    }

    /// The same for DuckDB over archived logs.
    pub fn duckdb_filter(&self) -> Option<SqlFilter> {
        self.searches().map(build_any_duckdb)
    }

    /// Whether the reader may see this event (live tail, stored results).
    pub fn allows(&self, f: &Fields<'_>) -> bool {
        self.searches().is_none_or(|s| matches_any(s, f))
    }
}

/// Checks a restriction query and returns its terms: a non-empty search with balanced quotes and
/// parentheses, and no `index:` term (the live tail can't tell indexes apart, and it would pick the table).
pub fn validate_query(query: &str) -> Result<Vec<Term>, String> {
    let q = query.trim();
    if q.chars().count() > MAX_QUERY_CHARS {
        return Err(format!("The query is at most {MAX_QUERY_CHARS} characters"));
    }
    if !balanced(q) {
        return Err("Unbalanced quotes or parentheses".into());
    }
    let terms = parse_query(q);
    if terms.is_empty() {
        return Err("Enter a search, e.g. service:billing".into());
    }
    if terms.iter().any(|t| matches!(t, Term::Tag { key, .. } if key == "index")) {
        return Err("Restriction queries can't filter on index".into());
    }
    Ok(terms)
}

/// Quotes close, and parentheses outside quotes close in order.
fn balanced(q: &str) -> bool {
    let (mut depth, mut quoted) = (0i32, false);
    for c in q.chars() {
        match c {
            '"' => quoted = !quoted,
            '(' if !quoted => depth += 1,
            ')' if !quoted => {
                depth -= 1;
                if depth < 0 {
                    return false;
                }
            }
            _ => {}
        }
    }
    depth == 0 && !quoted
}

// ================================================================ loading

/// Each org's restriction queries, reloaded after [`CACHE_TTL`] or when they change on this server.
#[derive(Default)]
pub struct RestrictionCache {
    orgs: Mutex<HashMap<String, Cached>>,
}

/// When an org's restriction queries were loaded, and them.
type Cached = (Instant, Arc<Vec<Restriction>>);

impl RestrictionCache {
    pub fn invalidate(&self, org: &str) {
        self.orgs.lock().unwrap_or_else(|e| e.into_inner()).remove(org);
    }

    fn fresh(&self, org: &str) -> Option<Arc<Vec<Restriction>>> {
        let orgs = self.orgs.lock().unwrap_or_else(|e| e.into_inner());
        orgs.get(org).filter(|(at, _)| at.elapsed() < CACHE_TTL).map(|(_, r)| r.clone())
    }

    fn store(&self, org: &str, rules: Arc<Vec<Restriction>>) {
        self.orgs.lock().unwrap_or_else(|e| e.into_inner()).insert(org.to_string(), (Instant::now(), rules));
    }
}

#[derive(sqlx::FromRow)]
struct RestrictionRow {
    query: String,
    roles: Vec<String>,
    teams: Vec<String>,
}

/// The org's restriction queries with their assignments (cached).
pub async fn org_restrictions(st: &AppState, org: &str) -> Result<Arc<Vec<Restriction>>, ApiError> {
    if let Some(r) = st.log_restrictions.fresh(org) {
        return Ok(r);
    }
    let rows: Vec<RestrictionRow> = sqlx::query_as(
        "SELECT q.query, \
         COALESCE((SELECT array_agg(r.role_id) FROM log_restriction_roles r WHERE r.org_id = q.org_id AND r.restriction_id = q.id), '{}') AS roles, \
         COALESCE((SELECT array_agg(t.team_handle) FROM log_restriction_teams t WHERE t.org_id = q.org_id AND t.restriction_id = q.id), '{}') AS teams \
         FROM log_restriction_queries q WHERE q.org_id = $1",
    )
    .bind(org)
    .fetch_all(&st.postgres)
    .await?;
    // Stored queries were validated; parsing again can't fail, it only splits terms.
    let rules = Arc::new(rows.into_iter().map(|r| Restriction { terms: parse_query(&r.query), roles: r.roles.into_iter().collect(), teams: r.teams.into_iter().collect() }).collect());
    st.log_restrictions.store(org, Arc::clone(&rules));
    Ok(rules)
}

/// The log access of an authenticated user of `ctx.org_id`.
pub async fn for_user(st: &AppState, ctx: &AuthContext) -> Result<LogAccess, ApiError> {
    let rules = org_restrictions(st, &ctx.org_id).await?;
    let admin = ctx.access == Access::Admin;
    if rules.is_empty() || admin {
        return Ok(LogAccess::resolve(&rules, admin, &[], &[]));
    }
    let (roles, teams): (Vec<String>, Vec<String>) = sqlx::query_as(
        "SELECT COALESCE((SELECT array_agg(ur.role_id) FROM user_roles ur JOIN roles r ON r.id = ur.role_id WHERE ur.user_id = $2::uuid AND r.org_id = $1), '{}'), \
         COALESCE((SELECT array_agg(m.handle) FROM team_members m WHERE m.org_id = $1 AND m.user_id = $2::uuid), '{}')",
    )
    .bind(&ctx.org_id)
    .bind(&ctx.user_id)
    .fetch_one(&st.postgres)
    .await?;
    Ok(LogAccess::resolve(&rules, false, &roles, &teams))
}

/// Handlers that read logs for a user take `LogAccess` (resolved from the session or API key).
#[async_trait]
impl FromRequestParts<AppState> for LogAccess {
    type Rejection = ApiError;
    async fn from_request_parts(parts: &mut Parts, st: &AppState) -> Result<Self, ApiError> {
        let ctx = AuthContext::from_request_parts(parts, st).await?;
        for_user(st, &ctx).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oc_query::Param;

    fn rule(query: &str, roles: &[&str], teams: &[&str]) -> Restriction {
        Restriction { terms: parse_query(query), roles: roles.iter().map(|r| r.to_string()).collect(), teams: teams.iter().map(|t| t.to_string()).collect() }
    }

    fn strings(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    fn rules() -> Vec<Restriction> {
        vec![rule("service:billing", &["r-finance"], &[]), rule("env:prod kube_namespace:payments", &[], &["payments"]), rule("service:catalog-service", &["r-unused"], &[])]
    }

    #[test]
    fn without_restriction_queries_everyone_reads_everything() {
        assert!(!LogAccess::resolve(&[], false, &[], &[]).restricted());
        assert!(!LogAccess::resolve(&[], false, &strings(&["r-finance"]), &strings(&["payments"])).restricted());
    }

    #[test]
    fn admins_read_everything() {
        let a = LogAccess::resolve(&rules(), true, &[], &[]);
        assert!(!a.restricted());
        assert!(a.clickhouse_filter().is_none());
    }

    #[test]
    fn users_read_the_union_of_their_roles_and_teams_queries() {
        let a = LogAccess::resolve(&rules(), false, &strings(&["r-finance", "org-standard"]), &strings(&["payments"]));
        assert!(a.restricted());
        assert_eq!(a.searches().map(<[_]>::len), Some(2));
        let f = a.clickhouse_filter().unwrap();
        assert_eq!(f.clause, "((service = ?)) OR ((tags[?] = ?) AND (tags[?] = ?))");
        assert_eq!(f.params[0], Param::Str("billing".into()));
        // Only through a role, or only through a team.
        assert_eq!(LogAccess::resolve(&rules(), false, &strings(&["r-finance"]), &[]).searches().map(<[_]>::len), Some(1));
        assert_eq!(LogAccess::resolve(&rules(), false, &[], &strings(&["payments"])).searches().map(<[_]>::len), Some(1));
    }

    #[test]
    fn users_without_an_assigned_query_read_nothing_once_restrictions_exist() {
        let a = LogAccess::resolve(&rules(), false, &strings(&["org-standard"]), &strings(&["web"]));
        assert!(a.restricted());
        assert_eq!(a.searches().map(<[_]>::len), Some(0));
        assert_eq!(a.clickhouse_filter().unwrap().clause, "0");
        assert_eq!(a.duckdb_filter().unwrap().clause, "false");
        // A query assigned to nobody still turns restrictions on.
        let unassigned = vec![rule("service:billing", &[], &[])];
        assert_eq!(LogAccess::resolve(&unassigned, false, &[], &[]).clickhouse_filter().unwrap().clause, "0");
    }

    #[test]
    fn a_query_reached_twice_counts_once() {
        let r = vec![rule("service:billing", &["r-finance"], &["payments"])];
        assert_eq!(LogAccess::resolve(&r, false, &strings(&["r-finance"]), &strings(&["payments"])).searches().map(<[_]>::len), Some(1));
    }

    #[test]
    fn background_jobs_read_everything() {
        assert!(!LogAccess::org_background().restricted());
        assert!(LogAccess::org_background().duckdb_filter().is_none());
    }

    #[test]
    fn in_memory_matching_follows_the_same_rules() {
        let none = std::collections::HashMap::new();
        let event = |service: &'static str| oc_query::Fields { service, status: "info", host: "h", source: "s", message: "m", tags: &none, attr: &none, index: None };
        let a = LogAccess::resolve(&rules(), false, &strings(&["r-finance"]), &[]);
        assert!(a.allows(&event("billing")));
        assert!(!a.allows(&event("order-service")));
        assert!(!LogAccess::resolve(&rules(), false, &[], &[]).allows(&event("billing")));
        assert!(LogAccess::org_background().allows(&event("order-service")));
    }

    #[test]
    fn restriction_queries_are_validated() {
        assert_eq!(validate_query("  service:billing env:prod ").unwrap().len(), 2);
        assert!(validate_query("@http.status_code:5* -status:debug \"card declined\"").is_ok());
        for bad in ["", "   ", "AND", "index:main", "service:billing -index:rehydrated", "service:(billing", "\"unterminated", "service:billing)"] {
            assert!(validate_query(bad).is_err(), "{bad:?}");
        }
        assert!(validate_query(&format!("service:{}", "a".repeat(MAX_QUERY_CHARS))).is_err());
    }
}
