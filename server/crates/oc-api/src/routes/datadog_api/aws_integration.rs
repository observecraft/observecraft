//! The AWS integration, Datadog-compatible: accounts (`/api/v2/integration/aws/accounts[/:id]`, the
//! Terraform provider's `datadog_integration_aws_account`), on Postgres `aws_accounts`, and what the
//! add-account flows read: the namespaces and log sources ObserveCraft collects
//! (`seeds/aws_integration.json`), the IAM permissions its crawler needs and new external ids.
//!
//! - Configuration sections are stored normalized, with the Terraform provider's defaults filled in,
//!   and answered as stored; an update keeps the sections it doesn't send.
//! - Role authentication gets an external id when none is sent (32 hex characters, as Datadog's);
//!   key authentication keeps the secret access key sealed ([`crate::auth::crypto::SecretBox`]) and
//!   never answers it. An account can't switch between the two.
//! - Writes need an admin: they hold credentials of a cloud account.
//! - Errors are these endpoints' JSON:API shape: `{"errors": [{"status", "title", "detail"}]}`.

use std::sync::OnceLock;

use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Map, Value};

use super::users::require;
use super::DdAuth;
use crate::auth::{Access, AuthContext};
use crate::routes::scorecards::valid_uuid;
use crate::state::AppState;

pub type DdErr = (StatusCode, Json<Value>);

/// How ObserveCraft reaches the account: assuming a role (with an external id) or with access keys.
#[derive(Debug, Clone, PartialEq)]
pub enum Auth {
    Role { role_name: String, external_id: Option<String> },
    Keys { access_key_id: String, secret_access_key: Option<String> },
}

/// A stored account, as the API documents it (the secret access key stays sealed in the row).
#[derive(Debug, Clone)]
pub struct AccountRow {
    pub id: String,
    pub aws_account_id: String,
    pub aws_partition: String,
    pub account_tags: Value,
    pub auth: Auth,
    pub aws_regions: Value,
    pub logs_config: Value,
    pub metrics_config: Value,
    pub resources_config: Value,
    pub traces_config: Value,
    pub created_at: String,
    pub modified_at: String,
}

/// Checked, normalized attributes of a create or update.
#[derive(Debug, Clone, PartialEq)]
pub struct AccountInput {
    pub aws_account_id: String,
    pub aws_partition: String,
    pub account_tags: Value,
    pub auth: Auth,
    pub aws_regions: Value,
    pub logs_config: Value,
    pub metrics_config: Value,
    pub resources_config: Value,
    pub traces_config: Value,
}

/// Datadog's JSON:API error: `{"errors": [{"status", "title", "detail"}]}`.
pub fn error(status: StatusCode, title: &str, detail: &str) -> DdErr {
    (status, Json(json!({ "errors": [{ "status": status.as_u16().to_string(), "title": title, "detail": detail }] })))
}

fn invalid(detail: &str) -> DdErr {
    error(StatusCode::BAD_REQUEST, "Generic Error", detail)
}

pub const PARTITIONS: [&str; 3] = ["aws", "aws-cn", "aws-us-gov"];
/// Namespaces excluded when none are chosen (their GetMetricData calls cost the most), as the Terraform provider's default.
pub const DEFAULT_EXCLUDED_NAMESPACES: [&str; 3] = ["AWS/SQS", "AWS/ElasticMapReduce", "AWS/Usage"];

fn strings(v: Option<&Value>, field: &str) -> Result<Vec<String>, DdErr> {
    match v {
        None | Some(Value::Null) => Ok(vec![]),
        Some(Value::Array(items)) => items.iter().map(|i| i.as_str().map(str::to_string).ok_or_else(|| invalid(&format!("{field} must be a list of strings")))).collect(),
        Some(_) => Err(invalid(&format!("{field} must be a list of strings"))),
    }
}

fn flag(o: &Map<String, Value>, key: &str, default: bool) -> Result<bool, DdErr> {
    match o.get(key) {
        None | Some(Value::Null) => Ok(default),
        Some(Value::Bool(b)) => Ok(*b),
        Some(_) => Err(invalid(&format!("{key} must be a boolean"))),
    }
}

fn object<'a>(v: Option<&'a Value>, field: &str) -> Result<Option<&'a Map<String, Value>>, DdErr> {
    match v {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Object(o)) => Ok(Some(o)),
        Some(_) => Err(invalid(&format!("{field} must be an object"))),
    }
}

/// `{"include_all": true}` or `{"include_only": [...]}` (exactly one), `default` when empty.
fn all_or_only(o: Option<&Map<String, Value>>, field: &str, default: Value) -> Result<Value, DdErr> {
    let Some(o) = o.filter(|o| !o.is_empty()) else { return Ok(default) };
    match (o.get("include_all").and_then(Value::as_bool), o.get("include_only")) {
        (Some(true), None) => Ok(json!({ "include_all": true })),
        (Some(false) | None, Some(only)) => Ok(json!({ "include_only": strings(Some(only), &format!("{field}.include_only"))? })),
        _ => Err(invalid(&format!("{field} needs exactly one of include_all and include_only"))),
    }
}

/// `{"include_only": [...]}` or `{"exclude_only": [...]}` (exactly one).
fn include_or_exclude(o: &Map<String, Value>, field: &str) -> Result<Value, DdErr> {
    match (o.get("include_only"), o.get("exclude_only")) {
        (Some(only), None) => Ok(json!({ "include_only": strings(Some(only), &format!("{field}.include_only"))? })),
        (None, Some(except)) => Ok(json!({ "exclude_only": strings(Some(except), &format!("{field}.exclude_only"))? })),
        _ => Err(invalid(&format!("{field} needs exactly one of include_only and exclude_only"))),
    }
}

/// `[{"<key>": "...", "tags": [...]}]`.
fn tag_filters(v: Option<&Value>, key: &str, field: &str) -> Result<Value, DdErr> {
    let mut out = vec![];
    for f in v.and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default() {
        let name = f.get(key).and_then(Value::as_str).ok_or_else(|| invalid(&format!("{field}.{key} is required")))?;
        out.push(json!({ key: name, "tags": strings(f.get("tags"), &format!("{field}.tags"))? }));
    }
    Ok(Value::Array(out))
}

fn logs_config(o: Option<&Map<String, Value>>) -> Result<Value, DdErr> {
    let forwarder = object(o.and_then(|o| o.get("lambda_forwarder")), "logs_config.lambda_forwarder")?;
    let get = |k: &str| forwarder.and_then(|f| f.get(k));
    let source_config = object(get("log_source_config"), "logs_config.lambda_forwarder.log_source_config")?;
    Ok(json!({ "lambda_forwarder": {
        "lambdas": strings(get("lambdas"), "logs_config.lambda_forwarder.lambdas")?,
        "sources": strings(get("sources"), "logs_config.lambda_forwarder.sources")?,
        "log_source_config": { "tag_filters": tag_filters(source_config.and_then(|c| c.get("tag_filters")), "source", "log_source_config.tag_filters")? },
    } }))
}

fn metrics_config(o: Option<&Map<String, Value>>) -> Result<Value, DdErr> {
    let empty = Map::new();
    let o = o.unwrap_or(&empty);
    let namespaces = match object(o.get("namespace_filters"), "metrics_config.namespace_filters")?.filter(|n| !n.is_empty()) {
        Some(n) => include_or_exclude(n, "metrics_config.namespace_filters")?,
        None => json!({ "exclude_only": DEFAULT_EXCLUDED_NAMESPACES }),
    };
    let mut out = json!({
        "enabled": flag(o, "enabled", true)?,
        "automute_enabled": flag(o, "automute_enabled", true)?,
        "collect_custom_metrics": flag(o, "collect_custom_metrics", false)?,
        "collect_cloudwatch_alarms": flag(o, "collect_cloudwatch_alarms", false)?,
        "tag_filters": tag_filters(o.get("tag_filters"), "namespace", "metrics_config.tag_filters")?,
        "namespace_filters": namespaces,
    });
    let mut names = vec![];
    for f in o.get("metric_name_filters").and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default() {
        let f = f.as_object().ok_or_else(|| invalid("metric_name_filters items must be objects"))?;
        let namespace = f.get("namespace").and_then(Value::as_str).ok_or_else(|| invalid("metric_name_filters.namespace is required"))?;
        let mut filter = include_or_exclude(f, "metrics_config.metric_name_filters")?;
        filter["namespace"] = json!(namespace);
        names.push(filter);
    }
    if !names.is_empty() {
        out["metric_name_filters"] = Value::Array(names);
    }
    Ok(out)
}

fn resources_config(o: Option<&Map<String, Value>>) -> Result<Value, DdErr> {
    let empty = Map::new();
    let o = o.unwrap_or(&empty);
    Ok(json!({
        "cloud_security_posture_management_collection": flag(o, "cloud_security_posture_management_collection", false)?,
        "extended_collection": flag(o, "extended_collection", true)?,
    }))
}

fn traces_config(o: Option<&Map<String, Value>>) -> Result<Value, DdErr> {
    let xray = object(o.and_then(|o| o.get("xray_services")), "traces_config.xray_services")?;
    Ok(json!({ "xray_services": all_or_only(xray, "traces_config.xray_services", json!({ "include_only": [] }))? }))
}

fn auth(a: &Map<String, Value>, current: Option<&AccountRow>) -> Result<Auth, DdErr> {
    let Some(c) = object(a.get("auth_config"), "auth_config")? else {
        return current.map(|r| r.auth.clone()).ok_or_else(|| invalid("auth_config is required"));
    };
    let text = |k: &str| c.get(k).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()).map(str::to_string);
    let auth = match (text("role_name"), text("access_key_id")) {
        (Some(role_name), None) => {
            let kept = match current.map(|r| &r.auth) {
                Some(Auth::Role { external_id, .. }) => external_id.clone(),
                _ => None,
            };
            Auth::Role { role_name, external_id: text("external_id").or(kept) }
        }
        (None, Some(access_key_id)) => Auth::Keys { access_key_id, secret_access_key: text("secret_access_key") },
        _ => return Err(invalid("auth_config needs exactly one of role_name and access_key_id")),
    };
    match (current.map(|r| &r.auth), &auth) {
        (Some(Auth::Role { .. }), Auth::Keys { .. }) | (Some(Auth::Keys { .. }), Auth::Role { .. }) => Err(invalid("cannot switch between role and key based auth")),
        (None, Auth::Keys { secret_access_key: None, .. }) => Err(invalid("secret_access_key is required with access_key_id")),
        _ => Ok(auth),
    }
}

/// Normalizes one configuration section (None: not sent).
type Normalize = fn(Option<&Map<String, Value>>) -> Result<Value, DdErr>;

/// Checks and normalizes a create (`current` None) or an update, whose missing sections keep their current value.
pub fn account_input(a: &Map<String, Value>, current: Option<&AccountRow>) -> Result<AccountInput, DdErr> {
    let aws_account_id = match a.get("aws_account_id").and_then(Value::as_str).map(str::trim) {
        Some(id) if id.len() == 12 && id.bytes().all(|b| b.is_ascii_digit()) => id.to_string(),
        Some(id) => return Err(invalid(&format!("invalid aws_account_id: {id}"))),
        None => current.map(|r| r.aws_account_id.clone()).ok_or_else(|| invalid("aws_account_id is required"))?,
    };
    let aws_partition = match a.get("aws_partition").and_then(Value::as_str) {
        Some(p) if PARTITIONS.contains(&p) => p.to_string(),
        Some(p) => return Err(invalid(&format!("invalid partition: {p}"))),
        None => current.map_or_else(|| "aws".to_string(), |r| r.aws_partition.clone()),
    };
    // A section that isn't sent keeps its current value (update) or gets its defaults (create); in an
    // update, the fields a sent section leaves out keep theirs (each field is replaced whole).
    let section = |key: &str, normalize: Normalize, kept: Option<&Value>| -> Result<Value, DdErr> {
        match (object(a.get(key), key)?, kept) {
            (None, Some(kept)) => Ok(kept.clone()),
            (Some(sent), Some(Value::Object(kept))) => {
                let mut merged = kept.clone();
                merged.extend(sent.iter().map(|(k, v)| (k.clone(), v.clone())));
                normalize(Some(&merged))
            }
            (sent, _) => normalize(sent),
        }
    };
    Ok(AccountInput {
        account_tags: match a.get("account_tags") {
            None | Some(Value::Null) => current.map_or_else(|| json!([]), |r| r.account_tags.clone()),
            v => json!(strings(v, "account_tags")?),
        },
        auth: auth(a, current)?,
        // One of include_all / include_only: replaced whole, never merged.
        aws_regions: match object(a.get("aws_regions"), "aws_regions")? {
            None => current.map_or_else(|| json!({ "include_all": true }), |r| r.aws_regions.clone()),
            sent => all_or_only(sent, "aws_regions", json!({ "include_all": true }))?,
        },
        logs_config: section("logs_config", logs_config, current.map(|r| &r.logs_config))?,
        metrics_config: section("metrics_config", metrics_config, current.map(|r| &r.metrics_config))?,
        resources_config: section("resources_config", resources_config, current.map(|r| &r.resources_config))?,
        traces_config: section("traces_config", traces_config, current.map(|r| &r.traces_config))?,
        aws_account_id,
        aws_partition,
    })
}

/// The account's JSON:API document (`type: account`); a secret access key is never included.
pub fn account_doc(r: &AccountRow) -> Value {
    let auth_config = match &r.auth {
        Auth::Role { role_name, external_id } => {
            let mut c = json!({ "role_name": role_name });
            if let Some(e) = external_id {
                c["external_id"] = json!(e);
            }
            c
        }
        Auth::Keys { access_key_id, .. } => json!({ "access_key_id": access_key_id }),
    };
    json!({
        "id": r.id,
        "type": "account",
        "attributes": {
            "account_tags": r.account_tags,
            "auth_config": auth_config,
            "aws_account_id": r.aws_account_id,
            "aws_partition": r.aws_partition,
            "aws_regions": r.aws_regions,
            "logs_config": r.logs_config,
            "metrics_config": r.metrics_config,
            "resources_config": r.resources_config,
            "traces_config": r.traces_config,
            "created_at": r.created_at,
            "modified_at": r.modified_at,
        }
    })
}

// ================================================================ catalog

#[derive(Deserialize)]
pub struct Catalog {
    pub namespaces: Vec<CatalogNamespace>,
    pub log_services: Vec<CatalogLogService>,
}

/// The UI reads the names, tile ids and categories from the same file.
#[derive(Deserialize)]
pub struct CatalogNamespace {
    pub namespace: String,
}

#[derive(Deserialize)]
pub struct CatalogLogService {
    pub id: String,
}

/// ObserveCraft's AWS catalog (`seeds/aws_integration.json`).
pub fn catalog() -> &'static Catalog {
    static C: OnceLock<Catalog> = OnceLock::new();
    C.get_or_init(|| serde_json::from_str(include_str!("../../../../../seeds/aws_integration.json")).expect("seeds/aws_integration.json is valid"))
}

/// What the AWS crawler calls (`crate::aws::api`): CloudWatch's metric list and data, resource tags, and the lookups of
/// the resources whose ARNs their metrics don't carry (`crate::aws::naming::LOOKUPS`: REST APIs, RDS proxies, DMS
/// replication instances, WAF Classic web ACLs).
pub const STANDARD_PERMISSIONS: [&str; 10] = [
    "cloudwatch:GetMetricData",
    "cloudwatch:ListMetrics",
    "tag:GetResources",
    "apigateway:GET",
    "rds:DescribeDBProxies",
    "dms:DescribeReplicationInstances",
    "waf:ListWebACLs",
    "waf:GetWebACL",
    "waf-regional:ListWebACLs",
    "waf-regional:GetWebACL",
];

// ================================================================ storage

fn db_err(e: sqlx::Error) -> DdErr {
    tracing::error!(error = %e, "AWS integration request failed");
    error(StatusCode::INTERNAL_SERVER_ERROR, "Internal Server Error", "internal error")
}

fn admin(ctx: &AuthContext) -> Result<(), DdErr> {
    require(ctx, Access::Admin).map_err(|e| error(StatusCode::FORBIDDEN, "Forbidden", e.errors.first().map_or("Forbidden", String::as_str)))
}

fn not_found() -> DdErr {
    error(StatusCode::NOT_FOUND, "Account not found", "AWS account with provided id is not integrated")
}

fn conflict() -> DdErr {
    error(StatusCode::CONFLICT, "Account already exists", "AWS account with provided id already exists")
}

/// The account config id of a path: a UUID, without the quotes the Terraform provider reads it back with.
fn config_id(raw: &str) -> Result<String, DdErr> {
    let id = raw.trim_matches('"');
    if valid_uuid(id) {
        return Ok(id.to_string());
    }
    Err(error(StatusCode::BAD_REQUEST, "Invalid Parameter", "invalid parameter \"aws_account_config_id\" in \"path\"; expected type \"uuid\""))
}

/// 32 hexadecimal characters, as Datadog's external ids.
pub fn new_external_id() -> String {
    let bytes: [u8; 16] = rand::random();
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn rfc3339(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms).unwrap_or_default().to_rfc3339_opts(chrono::SecondsFormat::Micros, true)
}

#[derive(sqlx::FromRow)]
struct DbRow {
    id: String,
    aws_account_id: String,
    aws_partition: String,
    account_tags: Value,
    role_name: Option<String>,
    external_id: Option<String>,
    access_key_id: Option<String>,
    aws_regions: Value,
    logs_config: Value,
    metrics_config: Value,
    resources_config: Value,
    traces_config: Value,
    created_at: i64,
    modified_at: i64,
}

impl From<DbRow> for AccountRow {
    fn from(r: DbRow) -> Self {
        let auth = match (r.role_name, r.access_key_id) {
            (_, Some(access_key_id)) => Auth::Keys { access_key_id, secret_access_key: None },
            (role_name, None) => Auth::Role { role_name: role_name.unwrap_or_default(), external_id: r.external_id },
        };
        AccountRow {
            id: r.id,
            aws_account_id: r.aws_account_id,
            aws_partition: r.aws_partition,
            account_tags: r.account_tags,
            auth,
            aws_regions: r.aws_regions,
            logs_config: r.logs_config,
            metrics_config: r.metrics_config,
            resources_config: r.resources_config,
            traces_config: r.traces_config,
            created_at: rfc3339(r.created_at),
            modified_at: rfc3339(r.modified_at),
        }
    }
}

const SELECT: &str = "SELECT id::text AS id, aws_account_id, aws_partition, account_tags, role_name, external_id, access_key_id, aws_regions, logs_config, \
    metrics_config, resources_config, traces_config, (extract(epoch FROM created_at) * 1000)::bigint AS created_at, \
    (extract(epoch FROM modified_at) * 1000)::bigint AS modified_at FROM aws_accounts";

/// The org's accounts, optionally only the one with this AWS account id.
pub async fn list_rows(st: &AppState, org: &str, aws_account_id: Option<&str>) -> Result<Vec<AccountRow>, sqlx::Error> {
    let rows: Vec<DbRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!("{SELECT} WHERE org_id = $1 AND ($2::text IS NULL OR aws_account_id = $2) ORDER BY created_at, id")))
        .bind(org)
        .bind(aws_account_id)
        .fetch_all(&st.postgres)
        .await?;
    Ok(rows.into_iter().map(AccountRow::from).collect())
}

pub async fn get_row(st: &AppState, org: &str, id: &str) -> Result<Option<AccountRow>, sqlx::Error> {
    let row: Option<DbRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!("{SELECT} WHERE org_id = $1 AND id = $2::uuid"))).bind(org).bind(id).fetch_optional(&st.postgres).await?;
    Ok(row.map(AccountRow::from))
}

fn is_unique_violation(e: &sqlx::Error) -> bool {
    e.as_database_error().and_then(|d| d.code()).as_deref() == Some("23505")
}

/// The auth columns of an input: (role_name, external_id, access_key_id, sealed secret or None to keep it).
fn auth_columns(st: &AppState, auth: &Auth) -> (Option<String>, Option<String>, Option<String>, Option<String>) {
    match auth {
        Auth::Role { role_name, external_id } => (Some(role_name.clone()), Some(external_id.clone().unwrap_or_else(new_external_id)), None, None),
        Auth::Keys { access_key_id, secret_access_key } => (None, None, Some(access_key_id.clone()), secret_access_key.as_deref().map(|s| st.secrets.encrypt(s))),
    }
}

pub async fn create_row(st: &AppState, org: &str, a: &Map<String, Value>) -> Result<AccountRow, DdErr> {
    let _changed = crate::routes::dashboards::EnabledChange(org);
    let input = account_input(a, None)?;
    let (role_name, external_id, access_key_id, secret) = auth_columns(st, &input.auth);
    let id: Result<String, sqlx::Error> = sqlx::query_scalar(
        "INSERT INTO aws_accounts (org_id, aws_account_id, aws_partition, account_tags, role_name, external_id, access_key_id, secret_access_key_enc, \
         aws_regions, logs_config, metrics_config, resources_config, traces_config) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13) RETURNING id::text",
    )
    .bind(org)
    .bind(&input.aws_account_id)
    .bind(&input.aws_partition)
    .bind(&input.account_tags)
    .bind(role_name)
    .bind(external_id)
    .bind(access_key_id)
    .bind(secret)
    .bind(&input.aws_regions)
    .bind(&input.logs_config)
    .bind(&input.metrics_config)
    .bind(&input.resources_config)
    .bind(&input.traces_config)
    .fetch_one(&st.postgres)
    .await;
    let id = match id {
        Err(e) if is_unique_violation(&e) => return Err(conflict()),
        r => r.map_err(db_err)?,
    };
    get_row(st, org, &id).await.map_err(db_err)?.ok_or_else(not_found)
}

pub async fn update_row(st: &AppState, org: &str, id: &str, a: &Map<String, Value>) -> Result<AccountRow, DdErr> {
    let id = &config_id(id)?;
    let current = get_row(st, org, id).await.map_err(db_err)?.ok_or_else(not_found)?;
    let input = account_input(a, Some(&current))?;
    let (role_name, external_id, access_key_id, secret) = auth_columns(st, &input.auth);
    let r = sqlx::query(
        "UPDATE aws_accounts SET aws_account_id = $3, aws_partition = $4, account_tags = $5, role_name = $6, external_id = $7, access_key_id = $8, \
         secret_access_key_enc = COALESCE($9, secret_access_key_enc), aws_regions = $10, logs_config = $11, metrics_config = $12, resources_config = $13, \
         traces_config = $14, modified_at = now() WHERE org_id = $1 AND id = $2::uuid",
    )
    .bind(org)
    .bind(id)
    .bind(&input.aws_account_id)
    .bind(&input.aws_partition)
    .bind(&input.account_tags)
    .bind(role_name)
    .bind(external_id)
    .bind(access_key_id)
    .bind(secret)
    .bind(&input.aws_regions)
    .bind(&input.logs_config)
    .bind(&input.metrics_config)
    .bind(&input.resources_config)
    .bind(&input.traces_config)
    .execute(&st.postgres)
    .await;
    match r {
        Err(e) if is_unique_violation(&e) => return Err(conflict()),
        r => r.map_err(db_err)?,
    };
    get_row(st, org, id).await.map_err(db_err)?.ok_or_else(not_found)
}

pub async fn delete_row(st: &AppState, org: &str, id: &str) -> Result<(), DdErr> {
    let _changed = crate::routes::dashboards::EnabledChange(org);
    let id = &config_id(id)?;
    let n = sqlx::query("DELETE FROM aws_accounts WHERE org_id = $1 AND id = $2::uuid").bind(org).bind(id).execute(&st.postgres).await.map_err(db_err)?.rows_affected();
    if n == 0 {
        return Err(not_found());
    }
    Ok(())
}

// ================================================================ handlers

type DdResult = Result<(StatusCode, Json<Value>), DdErr>;

/// `data.attributes` of a JSON:API body of type `account`.
fn body_attrs(raw: &Bytes) -> Result<Map<String, Value>, DdErr> {
    let v: Value = serde_json::from_slice(raw).map_err(|_| invalid("invalid JSON body"))?;
    let d = v.get("data").and_then(Value::as_object).ok_or_else(|| invalid("data is required"))?;
    if d.get("type").and_then(Value::as_str).is_some_and(|t| t != "account") {
        return Err(invalid("data.type must be account"));
    }
    Ok(d.get("attributes").and_then(Value::as_object).cloned().unwrap_or_default())
}

fn one(r: &AccountRow) -> (StatusCode, Json<Value>) {
    (StatusCode::OK, Json(json!({ "data": account_doc(r) })))
}

#[derive(Deserialize)]
pub struct ListQuery {
    aws_account_id: Option<String>,
}

#[utoipa::path(get, path = "/api/v2/integration/aws/accounts", operation_id = "datadog_list_aws_accounts", params(("aws_account_id" = Option<String>, Query)), responses((status = 200, body = Object)), tag = "datadog-api")]
pub async fn list(State(st): State<AppState>, DdAuth(ctx): DdAuth, Query(q): Query<ListQuery>) -> DdResult {
    let rows = list_rows(&st, &ctx.org_id, q.aws_account_id.as_deref().filter(|s| !s.is_empty())).await.map_err(db_err)?;
    Ok((StatusCode::OK, Json(json!({ "data": rows.iter().map(account_doc).collect::<Vec<_>>() }))))
}

#[utoipa::path(post, path = "/api/v2/integration/aws/accounts", operation_id = "datadog_create_aws_account", request_body = Object, responses((status = 200, body = Object), (status = 400, body = Object), (status = 409, body = Object)), tag = "datadog-api")]
pub async fn create(State(st): State<AppState>, DdAuth(ctx): DdAuth, raw: Bytes) -> DdResult {
    admin(&ctx)?;
    Ok(one(&create_row(&st, &ctx.org_id, &body_attrs(&raw)?).await?))
}

#[utoipa::path(get, path = "/api/v2/integration/aws/accounts/{aws_account_config_id}", operation_id = "datadog_get_aws_account", params(("aws_account_config_id" = String, Path)), responses((status = 200, body = Object), (status = 400, body = Object), (status = 404, body = Object)), tag = "datadog-api")]
pub async fn get(State(st): State<AppState>, DdAuth(ctx): DdAuth, Path(id): Path<String>) -> DdResult {
    let id = config_id(&id)?;
    Ok(one(&get_row(&st, &ctx.org_id, &id).await.map_err(db_err)?.ok_or_else(not_found)?))
}

#[utoipa::path(patch, path = "/api/v2/integration/aws/accounts/{aws_account_config_id}", operation_id = "datadog_update_aws_account", params(("aws_account_config_id" = String, Path)), request_body = Object, responses((status = 200, body = Object), (status = 400, body = Object), (status = 404, body = Object)), tag = "datadog-api")]
pub async fn update(State(st): State<AppState>, DdAuth(ctx): DdAuth, Path(id): Path<String>, raw: Bytes) -> DdResult {
    admin(&ctx)?;
    Ok(one(&update_row(&st, &ctx.org_id, &id, &body_attrs(&raw)?).await?))
}

#[utoipa::path(delete, path = "/api/v2/integration/aws/accounts/{aws_account_config_id}", operation_id = "datadog_delete_aws_account", params(("aws_account_config_id" = String, Path)), responses((status = 204), (status = 400, body = Object), (status = 404, body = Object)), tag = "datadog-api")]
pub async fn delete(State(st): State<AppState>, DdAuth(ctx): DdAuth, Path(id): Path<String>) -> Result<StatusCode, DdErr> {
    admin(&ctx)?;
    delete_row(&st, &ctx.org_id, &id).await?;
    Ok(StatusCode::NO_CONTENT)
}

fn single(id: &str, kind: &str, attributes: Value) -> (StatusCode, Json<Value>) {
    (StatusCode::OK, Json(json!({ "data": { "id": id, "type": kind, "attributes": attributes } })))
}

#[utoipa::path(get, path = "/api/v2/integration/aws/available_namespaces", operation_id = "datadog_list_aws_namespaces", responses((status = 200, body = Object)), tag = "datadog-api")]
pub async fn namespaces(DdAuth(_ctx): DdAuth) -> (StatusCode, Json<Value>) {
    single("namespaces", "namespaces", json!({ "namespaces": catalog().namespaces.iter().map(|n| &n.namespace).collect::<Vec<_>>() }))
}

#[utoipa::path(get, path = "/api/v2/integration/aws/logs/services", operation_id = "datadog_list_aws_logs_services", responses((status = 200, body = Object)), tag = "datadog-api")]
pub async fn logs_services(DdAuth(_ctx): DdAuth) -> (StatusCode, Json<Value>) {
    single("logs_services", "logs_services", json!({ "logs_services": catalog().log_services.iter().map(|s| &s.id).collect::<Vec<_>>() }))
}

#[utoipa::path(post, path = "/api/v2/integration/aws/generate_new_external_id", operation_id = "datadog_create_aws_external_id", responses((status = 200, body = Object)), tag = "datadog-api")]
pub async fn generate_external_id(DdAuth(ctx): DdAuth) -> DdResult {
    admin(&ctx)?;
    let id = new_external_id();
    Ok(single(&id, "external_id", json!({ "external_id": id })))
}

fn permissions(list: &[&str]) -> (StatusCode, Json<Value>) {
    single("permissions", "permissions", json!({ "permissions": list }))
}

#[utoipa::path(get, path = "/api/v2/integration/aws/iam_permissions", operation_id = "datadog_get_aws_iam_permissions", responses((status = 200, body = Object)), tag = "datadog-api")]
pub async fn iam_permissions(DdAuth(_ctx): DdAuth) -> (StatusCode, Json<Value>) {
    permissions(&STANDARD_PERMISSIONS)
}

#[utoipa::path(get, path = "/api/v2/integration/aws/iam_permissions/standard", operation_id = "datadog_get_aws_iam_permissions_standard", responses((status = 200, body = Object)), tag = "datadog-api")]
pub async fn iam_permissions_standard(DdAuth(_ctx): DdAuth) -> (StatusCode, Json<Value>) {
    permissions(&STANDARD_PERMISSIONS)
}

/// What Cloud Security's account scans need on top of the AWS managed `SecurityAudit` policy (docs/adr/0006): the
/// read-only actions of the scan's session policy (`seeds/posture/aws-scan-session-policy.json`).
pub fn resource_collection_permissions() -> Vec<String> {
    let policy: Value = serde_json::from_str(crate::aws::api::SCAN_SESSION_POLICY).unwrap_or_default();
    let mut actions: Vec<String> = policy["Statement"].as_array().into_iter().flatten().flat_map(|s| s["Action"].as_array().cloned().unwrap_or_default()).filter_map(|a| a.as_str().map(str::to_string)).collect();
    actions.sort();
    actions.dedup();
    actions
}

#[utoipa::path(get, path = "/api/v2/integration/aws/iam_permissions/resource_collection", operation_id = "datadog_get_aws_iam_permissions_resource_collection", responses((status = 200, body = Object)), tag = "datadog-api")]
pub async fn iam_permissions_resource_collection(DdAuth(_ctx): DdAuth) -> (StatusCode, Json<Value>) {
    let actions = resource_collection_permissions();
    permissions(&actions.iter().map(String::as_str).collect::<Vec<_>>())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn attrs(v: Value) -> Map<String, Value> {
        v.as_object().cloned().unwrap()
    }

    fn detail(e: DdErr) -> (StatusCode, String) {
        (e.0, e.1 .0["errors"][0]["detail"].as_str().unwrap_or_default().to_string())
    }

    fn role_account() -> Map<String, Value> {
        attrs(json!({ "aws_account_id": "123456789012", "auth_config": { "role_name": "DatadogIntegrationRole" } }))
    }

    fn row(input: AccountInput) -> AccountRow {
        AccountRow {
            id: "9bb08fa5-18b0-49fd-a5d7-b74f930e52b0".into(),
            aws_account_id: input.aws_account_id,
            aws_partition: input.aws_partition,
            account_tags: input.account_tags,
            auth: input.auth,
            aws_regions: input.aws_regions,
            logs_config: input.logs_config,
            metrics_config: input.metrics_config,
            resources_config: input.resources_config,
            traces_config: input.traces_config,
            created_at: "2026-11-01T12:00:00Z".into(),
            modified_at: "2026-11-01T12:00:00Z".into(),
        }
    }

    #[test]
    fn a_minimal_account_gets_the_documented_defaults() {
        let input = account_input(&role_account(), None).unwrap();
        assert_eq!(input.aws_partition, "aws");
        assert_eq!(input.account_tags, json!([]));
        assert_eq!(input.auth, Auth::Role { role_name: "DatadogIntegrationRole".into(), external_id: None });
        assert_eq!(input.aws_regions, json!({ "include_all": true }));
        assert_eq!(input.logs_config, json!({ "lambda_forwarder": { "lambdas": [], "sources": [], "log_source_config": { "tag_filters": [] } } }));
        assert_eq!(
            input.metrics_config,
            json!({
                "enabled": true, "automute_enabled": true, "collect_custom_metrics": false, "collect_cloudwatch_alarms": false,
                "tag_filters": [], "namespace_filters": { "exclude_only": ["AWS/SQS", "AWS/ElasticMapReduce", "AWS/Usage"] }
            })
        );
        assert_eq!(input.resources_config, json!({ "cloud_security_posture_management_collection": false, "extended_collection": true }));
        assert_eq!(input.traces_config, json!({ "xray_services": { "include_only": [] } }));
    }

    #[test]
    fn sent_sections_are_kept_as_sent() {
        let mut a = role_account();
        a.insert("aws_regions".into(), json!({ "include_only": ["us-east-1"] }));
        a.insert("account_tags".into(), json!(["team:storefront"]));
        a.insert("metrics_config".into(), json!({ "enabled": false, "namespace_filters": { "include_only": ["AWS/EC2"] }, "tag_filters": [{ "namespace": "AWS/EC2", "tags": ["env:prod"] }], "metric_name_filters": [{ "namespace": "AWS/EC2", "include_only": ["aws.ec2.network_in"] }] }));
        a.insert("traces_config".into(), json!({ "xray_services": { "include_all": true } }));
        let input = account_input(&a, None).unwrap();
        assert_eq!(input.aws_regions, json!({ "include_only": ["us-east-1"] }));
        assert_eq!(input.account_tags, json!(["team:storefront"]));
        assert_eq!(input.metrics_config["enabled"], json!(false));
        assert_eq!(input.metrics_config["namespace_filters"], json!({ "include_only": ["AWS/EC2"] }));
        assert_eq!(input.metrics_config["tag_filters"], json!([{ "namespace": "AWS/EC2", "tags": ["env:prod"] }]));
        assert_eq!(input.metrics_config["metric_name_filters"], json!([{ "namespace": "AWS/EC2", "include_only": ["aws.ec2.network_in"] }]));
        assert_eq!(input.traces_config, json!({ "xray_services": { "include_all": true } }));
    }

    #[test]
    fn invalid_values_are_refused_as_datadog_does() {
        let mut a = role_account();
        a.insert("aws_partition".into(), json!("aws-invalid"));
        assert_eq!(detail(account_input(&a, None).unwrap_err()), (StatusCode::BAD_REQUEST, "invalid partition: aws-invalid".into()));

        let mut a = role_account();
        a.insert("aws_account_id".into(), json!("12345"));
        assert_eq!(account_input(&a, None).unwrap_err().0, StatusCode::BAD_REQUEST);

        let mut a = role_account();
        a.insert("auth_config".into(), json!({ "role_name": "R", "access_key_id": "AKIAIOSFODNN7EXAMPLE" }));
        assert_eq!(account_input(&a, None).unwrap_err().0, StatusCode::BAD_REQUEST);
        a.insert("auth_config".into(), json!({}));
        assert_eq!(account_input(&a, None).unwrap_err().0, StatusCode::BAD_REQUEST);

        let mut a = role_account();
        a.insert("metrics_config".into(), json!({ "namespace_filters": { "include_only": ["AWS/EC2"], "exclude_only": ["AWS/SQS"] } }));
        assert_eq!(account_input(&a, None).unwrap_err().0, StatusCode::BAD_REQUEST);
    }

    #[test]
    fn keys_need_a_secret_on_create_and_keep_it_on_update() {
        let a = attrs(json!({ "aws_account_id": "123456789012", "auth_config": { "access_key_id": "AKIAIOSFODNN7EXAMPLE" } }));
        assert_eq!(account_input(&a, None).unwrap_err().0, StatusCode::BAD_REQUEST);

        let created = account_input(&attrs(json!({ "aws_account_id": "123456789012", "auth_config": { "access_key_id": "AKIAIOSFODNN7EXAMPLE", "secret_access_key": "wJalrXUtnFEMI" } })), None).unwrap();
        let current = row(created);
        let updated = account_input(&a, Some(&current)).unwrap();
        assert_eq!(updated.auth, Auth::Keys { access_key_id: "AKIAIOSFODNN7EXAMPLE".into(), secret_access_key: None });
    }

    #[test]
    fn an_update_cannot_switch_the_authentication_method() {
        let current = row(account_input(&role_account(), None).unwrap());
        let a = attrs(json!({ "aws_account_id": "123456789012", "auth_config": { "access_key_id": "AKIAIOSFODNN7EXAMPLE", "secret_access_key": "s" } }));
        assert_eq!(detail(account_input(&a, Some(&current)).unwrap_err()), (StatusCode::BAD_REQUEST, "cannot switch between role and key based auth".into()));
    }

    #[test]
    fn an_update_keeps_the_sections_it_does_not_send() {
        let mut a = role_account();
        a.insert("aws_regions".into(), json!({ "include_only": ["eu-west-1"] }));
        a.insert("account_tags".into(), json!(["env:prod"]));
        let current = row(account_input(&a, None).unwrap());
        let updated = account_input(&attrs(json!({ "metrics_config": { "enabled": false } })), Some(&current)).unwrap();
        assert_eq!(updated.aws_account_id, "123456789012");
        assert_eq!(updated.aws_regions, json!({ "include_only": ["eu-west-1"] }));
        assert_eq!(updated.account_tags, json!(["env:prod"]));
        assert_eq!(updated.auth, current.auth);
        assert_eq!(updated.metrics_config["enabled"], json!(false));
    }

    #[test]
    fn an_update_keeps_the_fields_of_a_section_it_does_not_send() {
        let mut a = role_account();
        a.insert("metrics_config".into(), json!({ "namespace_filters": { "include_only": ["AWS/EC2"] }, "collect_custom_metrics": false }));
        let current = row(account_input(&a, None).unwrap());
        let updated = account_input(&attrs(json!({ "metrics_config": { "collect_custom_metrics": true } })), Some(&current)).unwrap();
        assert_eq!(updated.metrics_config["namespace_filters"], json!({ "include_only": ["AWS/EC2"] }));
        assert_eq!(updated.metrics_config["collect_custom_metrics"], json!(true));
        // A field is replaced whole: exclude_only replaces include_only.
        let updated = account_input(&attrs(json!({ "metrics_config": { "namespace_filters": { "exclude_only": ["AWS/SQS"] } } })), Some(&current)).unwrap();
        assert_eq!(updated.metrics_config["namespace_filters"], json!({ "exclude_only": ["AWS/SQS"] }));
    }

    #[test]
    fn an_update_replaces_the_regions_whole() {
        let mut a = role_account();
        a.insert("aws_regions".into(), json!({ "include_only": ["us-east-1"] }));
        let current = row(account_input(&a, None).unwrap());
        let updated = account_input(&attrs(json!({ "aws_regions": { "include_all": true } })), Some(&current)).unwrap();
        assert_eq!(updated.aws_regions, json!({ "include_all": true }));
    }

    #[test]
    fn the_document_has_the_external_id_and_never_the_secret() {
        let mut role = row(account_input(&role_account(), None).unwrap());
        role.auth = Auth::Role { role_name: "DatadogIntegrationRole".into(), external_id: Some("2803c423184c499dbd123d346e5bd16f".into()) };
        let doc = account_doc(&role);
        assert_eq!(doc["type"], "account");
        assert_eq!(doc["id"], "9bb08fa5-18b0-49fd-a5d7-b74f930e52b0");
        assert_eq!(doc["attributes"]["auth_config"], json!({ "role_name": "DatadogIntegrationRole", "external_id": "2803c423184c499dbd123d346e5bd16f" }));
        assert_eq!(doc["attributes"]["aws_account_id"], "123456789012");
        assert_eq!(doc["attributes"]["created_at"], "2026-11-01T12:00:00Z");

        let keys = row(account_input(&attrs(json!({ "aws_account_id": "123456789012", "auth_config": { "access_key_id": "AKIAIOSFODNN7EXAMPLE", "secret_access_key": "wJalrXUtnFEMI" } })), None).unwrap());
        let doc = account_doc(&keys);
        assert_eq!(doc["attributes"]["auth_config"], json!({ "access_key_id": "AKIAIOSFODNN7EXAMPLE" }));
        assert!(!doc.to_string().contains("wJalrXUtnFEMI"));
    }

    #[test]
    fn the_catalog_lists_unique_namespaces_and_the_forwarder_log_sources() {
        let c = catalog();
        let namespaces: std::collections::HashSet<_> = c.namespaces.iter().map(|n| n.namespace.as_str()).collect();
        assert_eq!(namespaces.len(), c.namespaces.len());
        assert!(DEFAULT_EXCLUDED_NAMESPACES.iter().all(|n| namespaces.contains(n)));
        assert!(c.log_services.iter().any(|s| s.id == "s3"));
    }

    #[test]
    fn config_ids_are_uuids_and_the_providers_quotes_are_dropped() {
        assert_eq!(config_id("448c72e5-e081-4cd2-ae97-9f7c26d88133").unwrap(), "448c72e5-e081-4cd2-ae97-9f7c26d88133");
        // Terraform provider 4.27.0 reads the account back with the id quoted.
        assert_eq!(config_id("\"448c72e5-e081-4cd2-ae97-9f7c26d88133\"").unwrap(), "448c72e5-e081-4cd2-ae97-9f7c26d88133");
        assert_eq!(config_id("not-a-uuid").unwrap_err().0, StatusCode::BAD_REQUEST);
    }

    #[test]
    fn external_ids_are_32_hex_characters() {
        let id = new_external_id();
        assert_eq!(id.len(), 32);
        assert!(id.bytes().all(|b| b.is_ascii_hexdigit()));
        assert_ne!(id, new_external_id());
    }

    #[tokio::test]
    async fn accounts_belong_to_their_org_and_secrets_are_sealed() {
        let Some((_, states)) = crate::test_support::replicas(1).await else { return };
        let st = &states[0];
        let (org, other) = (crate::test_support::new_org(st, "aws-acct").await, crate::test_support::new_org(st, "aws-acct-other").await);
        let keys = attrs(json!({ "aws_account_id": "123456789012", "auth_config": { "access_key_id": "AKIAIOSFODNN7EXAMPLE", "secret_access_key": "wJalrXUtnFEMI" } }));
        let created = create_row(st, &org, &keys).await.unwrap();
        // The same AWS account can be integrated by another org, once per org.
        create_row(st, &other, &keys).await.unwrap();
        assert_eq!(create_row(st, &org, &keys).await.unwrap_err().0, StatusCode::CONFLICT);

        assert!(get_row(st, &other, &created.id).await.unwrap().is_none());
        assert_eq!(update_row(st, &other, &created.id, &keys).await.unwrap_err().0, StatusCode::NOT_FOUND);
        assert_eq!(delete_row(st, &other, &created.id).await.unwrap_err().0, StatusCode::NOT_FOUND);
        assert_eq!(list_rows(st, &org, None).await.unwrap().len(), 1);

        let sealed: String = sqlx::query_scalar("SELECT secret_access_key_enc FROM aws_accounts WHERE org_id = $1").bind(&org).fetch_one(&st.postgres).await.unwrap();
        assert!(!sealed.contains("wJalrXUtnFEMI"));
        assert_eq!(st.secrets.decrypt(&sealed).unwrap(), "wJalrXUtnFEMI");
        // An update without the secret keeps it.
        update_row(st, &org, &created.id, &attrs(json!({ "auth_config": { "access_key_id": "AKIAIOSFODNN7EXAMPLE" } }))).await.unwrap();
        let kept: String = sqlx::query_scalar("SELECT secret_access_key_enc FROM aws_accounts WHERE org_id = $1").bind(&org).fetch_one(&st.postgres).await.unwrap();
        assert_eq!(st.secrets.decrypt(&kept).unwrap(), "wJalrXUtnFEMI");
        delete_row(st, &org, &created.id).await.unwrap();
    }

    use super::super::replay::{Feature, TestServer};

    async fn replay(feature: &str, prefix: &str, min: usize) {
        let Some(f) = Feature::load(feature) else { return };
        let Some(s) = TestServer::start(prefix).await else { return };
        let mut problems = vec![];
        let mut replayed = 0;
        for scenario in &f.scenarios {
            if f.cassette(scenario).is_none() || !f.replayed(scenario) {
                continue;
            }
            let (_, p) = s.replayer().replay(&f, scenario).await;
            problems.extend(p);
            replayed += 1;
        }
        assert!(replayed >= min, "{replayed} scenarios");
        assert!(problems.is_empty(), "{}", problems.join("\n"));
    }

    #[tokio::test]
    async fn replays_datadogs_aws_integration_scenarios() {
        replay("v2/aws_integration", "dd-aws-replay", 15).await;
    }

    #[tokio::test]
    async fn replays_datadogs_aws_logs_integration_scenarios() {
        replay("v2/aws_logs_integration", "dd-aws-logs-replay", 1).await;
    }
}
