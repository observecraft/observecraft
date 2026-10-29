//! Metrics → Settings → Rules: controls over what metrics cost.
//! - **Ingestion controls** run at the intake (`oc-intake/src/metric_rules.rs`) on the Datadog
//!   series, sketches and service checks it converts: drop the matching metrics, or strip tags from
//!   them before they are stored. OTLP metrics are passed through unchanged.
//! - **Indexing controls** are tag allowlists for querying: a metric query may filter and group a
//!   matching metric only by the listed tags ([`check_indexed`]); the series are still stored.
//!
//! Rules match metric names with `*` wildcards (`kubernetes.*`). Cardinality figures come from the
//! series table (as Metrics → Volume): "ingested" is the distinct series stored, "indexed" the
//! distinct combinations of the queryable tags.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use oc_query::{glob, metric_field, resource_alias, MetricQuery, Param};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use super::logs::{bind, TimeRange};
use crate::auth::{Admin, AuthContext};
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;

const MAX_RULES: i64 = 100;
const MAX_TAGS: usize = 50;
/// Metrics whose indexed cardinality is estimated per request (the biggest first).
const MAX_ESTIMATES: usize = 25;
const TTL: Duration = Duration::from_secs(30);

/// One indexing control: the tags queries may use on the metrics `pattern` matches.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct IndexRule {
    pub name: String,
    pub pattern: String,
    pub tags: Vec<String>,
}

/// A tag key as the series store it: `env` and `deployment.environment` are one tag.
fn canonical(tag: &str) -> &str {
    resource_alias(tag).unwrap_or(tag)
}

/// The tags a query may use on `metric`: None when no indexing control covers it, else the union
/// of the matching controls' tags and the first matching control's name.
fn allowed_tags<'a>(metric: &str, rules: &'a [IndexRule]) -> Option<(Vec<&'a str>, &'a str)> {
    let matching: Vec<&IndexRule> = rules.iter().filter(|r| glob(&r.pattern, metric)).collect();
    let first = matching.first()?;
    let mut tags: Vec<&str> = Vec::new();
    for t in matching.iter().flat_map(|r| r.tags.iter()) {
        if !tags.iter().any(|x| canonical(x) == canonical(t)) {
            tags.push(t);
        }
    }
    Some((tags, first.name.as_str()))
}

/// Refuses a query that filters or groups an indexed-controlled metric by a tag its controls don't list.
pub(crate) fn check_indexed(q: &MetricQuery, rules: &[IndexRule]) -> Result<(), String> {
    let Some((allowed, rule)) = allowed_tags(&q.metric, rules) else { return Ok(()) };
    let used = q.filters.iter().map(|f| f.key.as_str()).chain(q.by.iter().map(String::as_str));
    for tag in used {
        if !allowed.iter().any(|a| canonical(a) == canonical(tag)) {
            let list = if allowed.is_empty() { "none".to_string() } else { allowed.join(", ") };
            return Err(format!("'{tag}' isn't a queryable tag of {} (indexing rule '{rule}'); queryable tags: {list}", q.metric));
        }
    }
    Ok(())
}

/// Shared cache namespace of the indexing controls; every rule write invalidates it.
const INDEX_RULES: &str = "metric_index_rules";

/// The org's enabled indexing controls, through the shared cache for [`TTL`] (a write on any
/// replica applies to the next query everywhere).
pub(crate) async fn index_rules(st: &AppState, org: &str) -> Result<Arc<Vec<IndexRule>>, ApiError> {
    st.cache
        .get_or_load(INDEX_RULES, org, "", TTL, || async {
            let rows: Vec<(String, String, Vec<String>)> =
                sqlx::query_as("SELECT name, metric_pattern, tags FROM metric_rules WHERE org_id = $1 AND kind = 'indexing' AND enabled ORDER BY name").bind(org).fetch_all(&st.postgres).await?;
            Ok(rows.into_iter().map(|(name, pattern, tags)| IndexRule { name, pattern, tags }).collect())
        })
        .await
}

/// [`check_indexed`] with the org's controls; a refused query is a bad request.
pub(crate) async fn check_query(st: &AppState, org: &str, q: &MetricQuery) -> Result<(), ApiError> {
    let rules = index_rules(st, org).await?;
    check_indexed(q, &rules).map_err(ApiError::BadRequest)
}

/// After a write: the org's indexing controls are reloaded on every replica. (Ingestion controls
/// apply at the intake to new points; cached results only hold points stored before.)
async fn forget(st: &AppState, org: &str) {
    st.cache.invalidate(INDEX_RULES, org).await;
}

// ================================================================ API

#[derive(Serialize, Deserialize, ToSchema, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MetricRuleKind {
    /// Applied at the intake, before metrics are stored.
    Ingestion,
    /// Applied to metric queries.
    Indexing,
}

#[derive(Serialize, Deserialize, ToSchema, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MetricRuleAction {
    /// Ingestion: don't store the matching metrics.
    Drop,
    /// Ingestion: remove the listed tags from the matching metrics.
    StripTags,
    /// Indexing: queries may use only the listed tags.
    AllowTags,
}

impl MetricRuleKind {
    fn as_str(self) -> &'static str {
        match self {
            MetricRuleKind::Ingestion => "ingestion",
            MetricRuleKind::Indexing => "indexing",
        }
    }
}

impl MetricRuleAction {
    fn as_str(self) -> &'static str {
        match self {
            MetricRuleAction::Drop => "drop",
            MetricRuleAction::StripTags => "strip_tags",
            MetricRuleAction::AllowTags => "allow_tags",
        }
    }
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct MetricRule {
    pub id: String,
    pub name: String,
    pub kind: MetricRuleKind,
    /// Metric names it applies to; `*` matches any run of characters.
    pub metric_pattern: String,
    pub action: MetricRuleAction,
    pub tags: Vec<String>,
    pub enabled: bool,
    pub updated_by: String,
    /// Epoch milliseconds.
    pub updated_at: i64,
    /// Metrics reported in the last day that it matches.
    pub metrics_impacted: u64,
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct MetricRuleInput {
    pub name: String,
    pub kind: MetricRuleKind,
    pub metric_pattern: String,
    pub action: MetricRuleAction,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default = "enabled_default")]
    pub enabled: bool,
}

fn enabled_default() -> bool {
    true
}

fn valid_tag(t: &str) -> bool {
    !t.is_empty() && t.len() <= 200 && t.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-' | '/'))
}

fn valid_pattern(p: &str) -> bool {
    !p.is_empty() && p.len() <= 200 && p.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-' | '/' | '*'))
}

/// Trims and checks an input.
fn validate(input: &mut MetricRuleInput) -> Result<(), String> {
    input.name = input.name.trim().to_string();
    input.metric_pattern = input.metric_pattern.trim().to_string();
    let mut tags: Vec<String> = Vec::new();
    for t in input.tags.iter().map(|t| t.trim()).filter(|t| !t.is_empty()) {
        if !valid_tag(t) {
            return Err(format!("'{t}' isn't a valid tag key"));
        }
        if !tags.iter().any(|x| x == t) {
            tags.push(t.to_string());
        }
    }
    input.tags = tags;
    if input.name.is_empty() || input.name.chars().count() > 200 {
        return Err("The rule name is 1 to 200 characters".into());
    }
    if !valid_pattern(&input.metric_pattern) {
        return Err("The metric pattern is a metric name, with * for any characters (e.g. kubernetes.*)".into());
    }
    match (input.kind, input.action) {
        (MetricRuleKind::Ingestion, MetricRuleAction::Drop) => input.tags.clear(),
        (MetricRuleKind::Ingestion, MetricRuleAction::StripTags) | (MetricRuleKind::Indexing, MetricRuleAction::AllowTags) => {
            if input.tags.is_empty() {
                return Err("List at least one tag".into());
            }
        }
        _ => return Err("Ingestion controls drop metrics or strip tags; indexing controls allow tags".into()),
    }
    if input.tags.len() > MAX_TAGS {
        return Err(format!("At most {MAX_TAGS} tags"));
    }
    Ok(())
}

#[derive(sqlx::FromRow)]
struct Row {
    id: String,
    name: String,
    kind: String,
    metric_pattern: String,
    action: String,
    tags: Vec<String>,
    enabled: bool,
    updated_by: String,
    updated_at: i64,
}

impl Row {
    fn into_rule(self, names: &[String]) -> MetricRule {
        MetricRule {
            metrics_impacted: names.iter().filter(|n| glob(&self.metric_pattern, n)).count() as u64,
            id: self.id,
            name: self.name,
            kind: if self.kind == "indexing" { MetricRuleKind::Indexing } else { MetricRuleKind::Ingestion },
            action: match self.action.as_str() {
                "drop" => MetricRuleAction::Drop,
                "strip_tags" => MetricRuleAction::StripTags,
                _ => MetricRuleAction::AllowTags,
            },
            metric_pattern: self.metric_pattern,
            tags: self.tags,
            enabled: self.enabled,
            updated_by: self.updated_by,
            updated_at: self.updated_at,
        }
    }
}

const COLS: &str = "id::text AS id, name, kind, metric_pattern, action, tags, enabled, updated_by, (extract(epoch FROM updated_at) * 1000)::bigint AS updated_at";

/// Metric names reported in the last day.
async fn recent_names(st: &AppState, org: &str) -> Result<Vec<String>, ApiError> {
    Ok(st.ch(org).query("SELECT DISTINCT MetricName FROM otel.metrics_series WHERE OrgId = {oc_org:String} AND LastSeen > now() - INTERVAL 1 DAY").fetch_all::<String>().await?)
}

async fn rows(st: &AppState, org: &str) -> Result<Vec<Row>, ApiError> {
    Ok(sqlx::query_as(&format!("SELECT {COLS} FROM metric_rules WHERE org_id = $1 ORDER BY kind, lower(name)")).bind(org).fetch_all(&st.postgres).await?)
}

async fn get_rule(st: &AppState, org: &str, id: &str) -> Result<MetricRule, ApiError> {
    let row: Option<Row> = sqlx::query_as(&format!("SELECT {COLS} FROM metric_rules WHERE org_id = $1 AND id::text = $2")).bind(org).bind(id).fetch_optional(&st.postgres).await?;
    let row = row.ok_or_else(|| ApiError::NotFound("Rule not found".into()))?;
    Ok(row.into_rule(&recent_names(st, org).await?))
}

fn unique(e: sqlx::Error, name: &str) -> ApiError {
    match &e {
        sqlx::Error::Database(d) if d.is_unique_violation() => ApiError::BadRequest(format!("A rule named '{name}' already exists")),
        _ => e.into(),
    }
}

fn describe(input: &MetricRuleInput) -> String {
    format!("{} {} {}", input.action.as_str(), input.metric_pattern, input.tags.join(","))
}

#[utoipa::path(get, path = "/api/v1/metrics/rules", operation_id = "list_metric_rules", responses((status = 200, body = Vec<MetricRule>)), tag = "metrics")]
pub async fn list(State(st): State<AppState>, ctx: AuthContext) -> ApiResult<Vec<MetricRule>> {
    let (rows, names) = tokio::try_join!(rows(&st, &ctx.org_id), recent_names(&st, &ctx.org_id))?;
    Ok(Json(rows.into_iter().map(|r| r.into_rule(&names)).collect()))
}

#[utoipa::path(post, path = "/api/v1/metrics/rules", operation_id = "create_metric_rule", request_body = MetricRuleInput, responses((status = 200, body = MetricRule)), tag = "metrics")]
pub async fn create(State(st): State<AppState>, Admin(ctx): Admin, Json(mut input): Json<MetricRuleInput>) -> ApiResult<MetricRule> {
    validate(&mut input).map_err(ApiError::BadRequest)?;
    let (count,): (i64,) = sqlx::query_as("SELECT count(*) FROM metric_rules WHERE org_id = $1").bind(&ctx.org_id).fetch_one(&st.postgres).await?;
    if count >= MAX_RULES {
        return Err(ApiError::BadRequest(format!("An organization can have at most {MAX_RULES} metrics rules")));
    }
    let (id,): (String,) =
        sqlx::query_as("INSERT INTO metric_rules (org_id, name, kind, metric_pattern, action, tags, enabled, updated_by) VALUES ($1, $2, $3, $4, $5, $6, $7, $8) RETURNING id::text")
            .bind(&ctx.org_id)
            .bind(&input.name)
            .bind(input.kind.as_str())
            .bind(&input.metric_pattern)
            .bind(input.action.as_str())
            .bind(&input.tags)
            .bind(input.enabled)
            .bind(&ctx.name)
            .fetch_one(&st.postgres)
            .await
            .map_err(|e| unique(e, &input.name))?;
    forget(&st, &ctx.org_id).await;
    ctx.audit(&st, "metric_rule.created", &input.name, &describe(&input)).await;
    Ok(Json(get_rule(&st, &ctx.org_id, &id).await?))
}

#[utoipa::path(put, path = "/api/v1/metrics/rules/{id}", operation_id = "update_metric_rule", params(("id" = String, Path)), request_body = MetricRuleInput, responses((status = 200, body = MetricRule)), tag = "metrics")]
pub async fn update(State(st): State<AppState>, Admin(ctx): Admin, Path(id): Path<String>, Json(mut input): Json<MetricRuleInput>) -> ApiResult<MetricRule> {
    validate(&mut input).map_err(ApiError::BadRequest)?;
    let done =
        sqlx::query("UPDATE metric_rules SET name = $3, kind = $4, metric_pattern = $5, action = $6, tags = $7, enabled = $8, updated_by = $9, updated_at = now() WHERE org_id = $1 AND id::text = $2")
            .bind(&ctx.org_id)
            .bind(&id)
            .bind(&input.name)
            .bind(input.kind.as_str())
            .bind(&input.metric_pattern)
            .bind(input.action.as_str())
            .bind(&input.tags)
            .bind(input.enabled)
            .bind(&ctx.name)
            .execute(&st.postgres)
            .await
            .map_err(|e| unique(e, &input.name))?;
    if done.rows_affected() == 0 {
        return Err(ApiError::NotFound("Rule not found".into()));
    }
    forget(&st, &ctx.org_id).await;
    ctx.audit(&st, "metric_rule.updated", &input.name, &describe(&input)).await;
    Ok(Json(get_rule(&st, &ctx.org_id, &id).await?))
}

#[utoipa::path(delete, path = "/api/v1/metrics/rules/{id}", operation_id = "delete_metric_rule", params(("id" = String, Path)), responses((status = 204)), tag = "metrics")]
pub async fn delete(State(st): State<AppState>, Admin(ctx): Admin, Path(id): Path<String>) -> Result<StatusCode, ApiError> {
    let row: Option<(String,)> = sqlx::query_as("DELETE FROM metric_rules WHERE org_id = $1 AND id::text = $2 RETURNING name").bind(&ctx.org_id).bind(&id).fetch_optional(&st.postgres).await?;
    let Some((name,)) = row else { return Err(ApiError::NotFound("Rule not found".into())) };
    forget(&st, &ctx.org_id).await;
    ctx.audit(&st, "metric_rule.deleted", &name, "").await;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize, ToSchema)]
pub struct MetricRulesOverviewRequest {
    pub range: TimeRange,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct MetricCardinality {
    pub name: String,
    /// Distinct series stored in the range.
    pub ingested: u64,
    /// Distinct combinations of its queryable tags (= ingested without an indexing control).
    pub indexed: u64,
    /// Names of the enabled rules that match it.
    pub rules: Vec<String>,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct MetricRulesOverview {
    /// Metrics an enabled rule matches, the biggest first.
    pub filtered: Vec<MetricCardinality>,
    /// The metrics with the most series.
    pub top: Vec<MetricCardinality>,
}

#[derive(clickhouse::Row, Deserialize)]
struct SeriesCount {
    name: String,
    series: u64,
}

/// Rules Overview: cardinality of the metrics rules filter and of the biggest metrics.
#[utoipa::path(post, path = "/api/v1/metrics/rules/overview", operation_id = "metric_rules_overview", request_body = MetricRulesOverviewRequest, responses((status = 200, body = MetricRulesOverview)), tag = "metrics")]
pub async fn overview(State(st): State<AppState>, ctx: AuthContext, Json(req): Json<MetricRulesOverviewRequest>) -> ApiResult<MetricRulesOverview> {
    req.range.validate()?;
    let org = &ctx.org_id;
    let ch = st.ch(org);
    let counts = ch
        .query("SELECT MetricName AS name, uniqExact(SeriesId) AS series FROM otel.metrics_series WHERE OrgId = {oc_org:String} AND LastSeen >= toStartOfHour(toDateTime(intDiv(?, 1000))) GROUP BY MetricName ORDER BY series DESC, name")
        .bind(req.range.from)
        .fetch_all::<SeriesCount>();
    let (counts, rules) = tokio::try_join!(async { counts.await.map_err(ApiError::from) }, rows(&st, org))?;
    let rules: Vec<Row> = rules.into_iter().filter(|r| r.enabled).collect();
    let index: Vec<IndexRule> = rules.iter().filter(|r| r.kind == "indexing").map(|r| IndexRule { name: r.name.clone(), pattern: r.metric_pattern.clone(), tags: r.tags.clone() }).collect();
    let entry = |c: &SeriesCount| MetricCardinality {
        name: c.name.clone(),
        ingested: c.series,
        indexed: c.series,
        rules: rules.iter().filter(|r| glob(&r.metric_pattern, &c.name)).map(|r| r.name.clone()).collect(),
    };
    let mut filtered: Vec<MetricCardinality> = counts.iter().map(entry).filter(|m| !m.rules.is_empty()).collect();
    let mut top: Vec<MetricCardinality> = counts.iter().take(10).map(entry).collect();
    // Indexed cardinality of the biggest indexing-controlled metrics.
    let mut estimate: Vec<(String, Vec<String>)> = Vec::new();
    for m in filtered.iter().chain(top.iter()) {
        if estimate.len() >= MAX_ESTIMATES || estimate.iter().any(|(n, _)| *n == m.name) {
            continue;
        }
        if let Some((tags, _)) = allowed_tags(&m.name, &index) {
            estimate.push((m.name.clone(), tags.into_iter().map(str::to_string).collect()));
        }
    }
    let mut indexed: BTreeMap<String, u64> = BTreeMap::new();
    if !estimate.is_empty() {
        let (sql, params) = indexed_sql(&estimate, req.range.from);
        for c in bind(st.ch(org).query(&sql), &params).fetch_all::<SeriesCount>().await? {
            indexed.insert(c.name, c.series);
        }
    }
    for m in filtered.iter_mut().chain(top.iter_mut()) {
        if let Some(n) = indexed.get(&m.name) {
            m.indexed = *n;
        }
    }
    filtered.sort_by(|a, b| b.ingested.cmp(&a.ingested).then_with(|| a.name.cmp(&b.name)));
    Ok(Json(MetricRulesOverview { filtered, top }))
}

/// Distinct combinations of each metric's queryable tags since `from` (one SELECT per metric,
/// unioned; names and keys are bound).
fn indexed_sql(metrics: &[(String, Vec<String>)], from: i64) -> (String, Vec<Param>) {
    let mut parts = Vec::new();
    let mut params = Vec::new();
    for (name, tags) in metrics {
        params.push(Param::Str(name.clone()));
        let value = if tags.is_empty() {
            "toUInt64(count() > 0)".to_string()
        } else {
            let mut exprs = Vec::new();
            for t in tags {
                let (e, p) = metric_field(t);
                exprs.push(e);
                params.extend(p);
            }
            format!("uniqExact(tuple({}))", exprs.join(", "))
        };
        params.extend([Param::Str(name.clone()), Param::Int(from)]);
        parts.push(format!("SELECT ? AS name, {value} AS series FROM otel.metrics_series WHERE OrgId = {{oc_org:String}} AND MetricName = ? AND LastSeen >= toStartOfHour(toDateTime(intDiv(?, 1000)))"));
    }
    (parts.join(" UNION ALL "), params)
}

#[cfg(test)]
mod tests {
    use super::*;
    use oc_query::parse_metric_query;

    fn rule(name: &str, pattern: &str, tags: &[&str]) -> IndexRule {
        IndexRule { name: name.into(), pattern: pattern.into(), tags: tags.iter().map(|t| t.to_string()).collect() }
    }

    #[test]
    fn indexing_controls_limit_the_tags_of_matching_metrics() {
        let rules = vec![rule("k8s tags", "kubernetes.*", &["kube_namespace", "env"])];
        let ok = |q: &str| check_indexed(&parse_metric_query(q).unwrap(), &rules);
        assert!(ok("avg:kubernetes.cpu.usage.total{kube_namespace:shop} by {env}").is_ok());
        // `deployment.environment` is the stored name of `env`.
        assert!(ok("avg:kubernetes.cpu.usage.total{deployment.environment:prod}").is_ok());
        let err = ok("avg:kubernetes.cpu.usage.total{*} by {pod_name}").unwrap_err();
        assert!(err.contains("pod_name") && err.contains("k8s tags"), "{err}");
        assert!(ok("avg:kubernetes.cpu.usage.total{!host:i-0a1b2c3d4e5f60001}").is_err());
        // Other metrics aren't controlled.
        assert!(ok("avg:system.load.1{*} by {host}").is_ok());
    }

    #[test]
    fn several_matching_controls_allow_all_their_tags() {
        let rules = vec![rule("a", "kubernetes.*", &["kube_namespace"]), rule("b", "kubernetes.cpu.*", &["host"])];
        assert!(check_indexed(&parse_metric_query("avg:kubernetes.cpu.usage{*} by {host,kube_namespace}").unwrap(), &rules).is_ok());
        assert!(check_indexed(&parse_metric_query("avg:kubernetes.memory.usage{*} by {host}").unwrap(), &rules).is_err());
    }

    fn input(kind: MetricRuleKind, action: MetricRuleAction, tags: &[&str]) -> MetricRuleInput {
        MetricRuleInput { name: " Drop debug ".into(), kind, metric_pattern: " debug.* ".into(), action, tags: tags.iter().map(|t| t.to_string()).collect(), enabled: true }
    }

    #[test]
    fn rules_are_checked() {
        use MetricRuleAction::*;
        use MetricRuleKind::*;
        let mut drop = input(Ingestion, Drop, &["x"]);
        assert_eq!(validate(&mut drop), Ok(()));
        assert_eq!((drop.name.as_str(), drop.metric_pattern.as_str(), drop.tags.len()), ("Drop debug", "debug.*", 0));
        let mut strip = input(Ingestion, StripTags, &[" pod_name ", "pod_name", ""]);
        assert_eq!(validate(&mut strip), Ok(()));
        assert_eq!(strip.tags, vec!["pod_name"]);
        assert!(validate(&mut input(Ingestion, StripTags, &[])).is_err());
        assert!(validate(&mut input(Indexing, AllowTags, &[])).is_err());
        assert!(validate(&mut input(Indexing, Drop, &[])).is_err());
        assert!(validate(&mut input(Ingestion, AllowTags, &["a"])).is_err());
        assert!(validate(&mut input(Indexing, AllowTags, &["bad tag"])).is_err());
        let mut bad = input(Ingestion, Drop, &[]);
        bad.metric_pattern = "x'; --".into();
        assert!(validate(&mut bad).is_err());
    }

    #[test]
    fn indexed_cardinality_binds_names_and_keys() {
        let (sql, params) = indexed_sql(&[("kubernetes.cpu".into(), vec!["host".into()]), ("debug.x".into(), vec![])], 1_000);
        assert_eq!(sql.matches("UNION ALL").count(), 1);
        assert!(sql.contains("uniqExact(tuple(ResourceAttributes[?]))") && sql.contains("toUInt64(count() > 0)"));
        assert!(!sql.contains("kubernetes.cpu"), "names are bound");
        assert_eq!(params.len(), 7);
    }

    /// Indexing controls are read through the shared cache and a write drops them on every
    /// replica: a rule created or disabled on A applies to B's next query. Skipped unless
    /// OC_TEST_CLICKHOUSE_URL and OC_TEST_POSTGRES_URL are set.
    #[tokio::test]
    async fn indexing_rules_written_on_one_replica_apply_on_the_others_at_once() {
        let Some((_, states)) = crate::test_support::replicas(2).await else { return };
        let (a, b) = (states[0].clone(), states[1].clone());
        let ctx = crate::test_support::admin_of_new_org(&a, "rules-shared").await;
        let org = ctx.org_id.clone();
        assert!(index_rules(&b, &org).await.unwrap().is_empty());
        let input = |enabled: bool| MetricRuleInput {
            name: "kubernetes".into(),
            kind: MetricRuleKind::Indexing,
            metric_pattern: "kubernetes.*".into(),
            action: MetricRuleAction::AllowTags,
            tags: vec!["kube_namespace".into()],
            enabled,
        };
        let created = create(State(a.clone()), Admin(ctx.clone()), Json(input(true))).await.expect("create").0;
        let names = |rules: Arc<Vec<IndexRule>>| rules.iter().map(|r| r.name.clone()).collect::<Vec<_>>();
        assert_eq!(names(index_rules(&b, &org).await.unwrap()), ["kubernetes"]);

        // Served from the shared copy: a row changed behind the server's back isn't seen…
        sqlx::query("UPDATE metric_rules SET name = 'renamed' WHERE org_id = $1").bind(&org).execute(&a.postgres).await.unwrap();
        assert_eq!(names(index_rules(&a, &org).await.unwrap()), ["kubernetes"]);
        // …while a write through the API is, on the other replica too.
        let _ = update(State(a.clone()), Admin(ctx.clone()), Path(created.id.clone()), Json(input(false))).await.expect("update");
        assert!(index_rules(&b, &org).await.unwrap().is_empty());
    }
}
