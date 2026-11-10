//! Integrations → connectors (docs/adr/0007-connectors.md): the catalog of connectors (bundled and operator packages,
//! native connectors) and an org's connections of packages, which the server polls (`crate::connectors::runner`).
//! Admins save, test, sync and remove connections; every member reads them and their streams' status. Native
//! connectors are set up on their own pages; here they are listed, and their jobs' status read and synced.
//!
//! - Secrets are sealed field by field, never answered, scrubbed from errors. A saved secret is only sent to the
//!   URLs it was saved with: changing a `url` field asks for every secret again.
//! - URL fields are checked with the Developer Portal connectors' rules (`check_site_url`) and every request goes
//!   through the SSRF guard (`OC_PORTAL_ALLOW_PRIVATE` for internal hosts).
//! - Saving installs the package's log pipeline and dashboards for the org, and its dashboards are listed once the
//!   org has a connection (`routes::dashboards`); it creates the package's monitors, Cloud SIEM rules and log-based
//!   metrics once (`connectors::assets`). Removing the last connection removes those and the pipeline.

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use oc_connectors::engine::{self, Runtime};
use oc_connectors::manifest::{FieldKind, Manifest};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use utoipa::ToSchema;

use super::dashboards::EnabledChange;
use super::portal_sources::check_site_url;
use crate::auth::Admin;
use crate::cloud::jobs::{self, CrawlIssue};
use crate::connectors::{self, registry, store, ConnectorEntry, PackageConnector};
use crate::error::{ApiError, ApiResult};
use crate::state::{AppState, OrgId};

const MAX_VALUE_CHARS: usize = 65_536;
const MAX_NAME_CHARS: usize = 100;

/// A package that couldn't be loaded, for instance admins.
#[derive(Serialize, ToSchema, Debug)]
pub struct InvalidPackage {
    pub dir: String,
    pub problems: Vec<String>,
}

/// Every connector of this server.
#[derive(Serialize, ToSchema, Debug)]
pub struct ConnectorCatalog {
    pub connectors: Vec<ConnectorEntry>,
    /// The operator's packages that couldn't be loaded (admins only).
    pub invalid: Vec<InvalidPackage>,
}

/// A stream's collection, from its job.
#[derive(Serialize, ToSchema, Debug)]
#[serde(rename_all = "camelCase")]
pub struct StreamStatus {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    /// Epoch ms of the last successful run.
    pub last_success_at: Option<i64>,
    /// Epoch ms: records up to here were read (time windows), or the last run's minute.
    pub collected_until: Option<i64>,
    /// Records read by the last run.
    pub records: i32,
    /// Logs, points, events and costs it wrote.
    pub written: i32,
    /// Why the last run failed, or its warning.
    pub error: Option<CrawlIssue>,
}

/// A connection as answered: its configuration and which secrets are saved, never their values.
#[derive(Serialize, ToSchema, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionView {
    pub id: String,
    pub name: String,
    #[schema(value_type = std::collections::HashMap<String, Value>)]
    pub config: Map<String, Value>,
    /// The secret fields saved.
    pub secrets: Vec<String>,
    pub enabled: bool,
    pub streams: Vec<StreamStatus>,
    /// Epoch ms; None for native connectors.
    pub updated_at: Option<i64>,
}

/// A connector and the org's connections of it.
#[derive(Serialize, ToSchema, Debug)]
pub struct ConnectorDetail {
    pub connector: ConnectorEntry,
    pub connections: Vec<ConnectionView>,
}

/// What an admin enters.
#[derive(Deserialize, ToSchema, Clone, Debug, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct ConnectionInput {
    /// Defaults to the connected account (`connection.instance`).
    pub name: String,
    /// The package's non-secret fields.
    #[schema(value_type = std::collections::HashMap<String, Value>)]
    pub config: Map<String, Value>,
    /// Secret fields entered; left out or empty keeps the saved one.
    #[schema(value_type = std::collections::HashMap<String, String>)]
    pub secrets: Map<String, Value>,
    /// The streams to collect (default: those enabled by default).
    pub streams: Option<Vec<String>>,
    pub enabled: Option<bool>,
    /// For a test: the saved connection whose secrets fill the ones left out.
    pub connection_id: Option<String>,
    /// For a push-only package's `push.create`: the URL the vendor is to send to (as the page shows it).
    pub push_url: Option<String>,
}

/// A connection test's result.
#[derive(Serialize, ToSchema, Debug, PartialEq)]
pub struct ConnectorTest {
    pub ok: bool,
    pub message: String,
    /// Records the test read.
    pub records: Option<usize>,
}

/// A checked input: the configuration, the secrets (unsealed, entered or kept), the streams, and whether a URL
/// changed.
pub struct Checked {
    pub name: String,
    pub config: Map<String, Value>,
    pub secrets: Map<String, Value>,
    /// Sealed secrets kept as they are.
    pub kept: Map<String, Value>,
    pub streams: Vec<String>,
    pub url_changed: bool,
}

fn text_of(v: &Value) -> Option<String> {
    match v {
        Value::Null => None,
        Value::String(s) => Some(s.trim().to_string()).filter(|s| !s.is_empty()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

/// Checks an input against the package's form: types, options, patterns, URLs (SSRF rules), required fields, the
/// secrets to keep. `previous`: the saved connection being changed (its secrets are kept unless a URL changed).
pub fn check(m: &Manifest, input: &ConnectionInput, previous: Option<&store::Saved>, allow_private: bool) -> Result<Checked, String> {
    for k in input.config.keys() {
        if !m.config.iter().any(|f| !f.secret && &f.key == k) {
            return Err(format!("{k} is not a field of {}", m.name));
        }
    }
    for k in input.secrets.keys() {
        if !m.config.iter().any(|f| f.secret && &f.key == k) {
            return Err(format!("{k} is not a secret field of {}", m.name));
        }
    }
    let mut config = Map::new();
    let mut url_changed = false;
    for f in m.config.iter().filter(|f| !f.secret) {
        let raw = input.config.get(&f.key).cloned().unwrap_or(Value::Null);
        if raw.to_string().len() > MAX_VALUE_CHARS {
            return Err(format!("{} is too long", f.label));
        }
        let pattern = f.pattern.as_deref().map(|p| regex::Regex::new(&format!("^(?:{p})$"))).transpose().map_err(|e| e.to_string())?;
        let matches = |s: &str| pattern.as_ref().is_none_or(|re| re.is_match(s));
        let value = match f.kind {
            FieldKind::Url => match text_of(&raw) {
                None => Value::Null,
                Some(s) => {
                    let mut s = s.trim_end_matches('/').to_string();
                    for suffix in &f.strip_suffix {
                        if let Some(stripped) = s.strip_suffix(suffix.as_str()) {
                            s = stripped.trim_end_matches('/').to_string();
                        }
                    }
                    Value::String(check_site_url(&s, allow_private).map_err(|e| format!("{}: {e}", f.label))?)
                }
            },
            FieldKind::String | FieldKind::Text => match text_of(&raw) {
                None => Value::Null,
                Some(s) if !matches(&s) => return Err(format!("{} is not valid", f.label)),
                Some(s) => Value::String(s),
            },
            FieldKind::Number => match &raw {
                Value::Null => Value::Null,
                Value::String(s) if s.trim().is_empty() => Value::Null,
                v => v.as_f64().or_else(|| v.as_str().and_then(|s| s.trim().parse().ok())).map(oc_connectors::expr::number).ok_or_else(|| format!("{} must be a number", f.label))?,
            },
            FieldKind::Boolean => match &raw {
                Value::Null => Value::Null,
                Value::Bool(b) => Value::Bool(*b),
                Value::String(s) => Value::Bool(matches!(s.as_str(), "true" | "1" | "yes" | "on")),
                _ => return Err(format!("{} must be true or false", f.label)),
            },
            FieldKind::Select => match text_of(&raw) {
                None => Value::Null,
                Some(s) if f.options.iter().any(|o| o.value == s) => Value::String(s),
                Some(s) => return Err(format!("{s} is not a choice of {}", f.label)),
            },
            FieldKind::Multiselect | FieldKind::List => {
                let items: Vec<String> = match &raw {
                    Value::Array(a) => a.iter().filter_map(text_of).collect(),
                    Value::String(s) => s.split(['\n', ',']).map(str::trim).filter(|s| !s.is_empty()).map(str::to_string).collect(),
                    Value::Null => vec![],
                    _ => return Err(format!("{} must be a list", f.label)),
                };
                for item in &items {
                    if f.kind == FieldKind::Multiselect && !f.options.iter().any(|o| &o.value == item) {
                        return Err(format!("{item} is not a choice of {}", f.label));
                    }
                    if !matches(item) {
                        return Err(format!("{item} is not valid for {}", f.label));
                    }
                }
                if items.is_empty() {
                    Value::Null
                } else {
                    Value::Array(items.into_iter().map(Value::String).collect())
                }
            }
        };
        if value.is_null() {
            if f.required && f.default.is_none() {
                return Err(format!("{} needs {}", m.name, f.label));
            }
            continue;
        }
        if f.kind == FieldKind::Url && previous.is_some_and(|p| p.config.get(&f.key) != Some(&value)) {
            url_changed = true;
        }
        config.insert(f.key.clone(), value);
    }
    let mut secrets = Map::new();
    let mut kept = Map::new();
    for f in m.config.iter().filter(|f| f.secret) {
        let entered = input.secrets.get(&f.key).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty());
        match entered {
            Some(s) if s.len() > MAX_VALUE_CHARS => return Err(format!("{} is too long", f.label)),
            Some(s) => {
                secrets.insert(f.key.clone(), Value::String(s.to_string()));
            }
            None => match previous.and_then(|p| p.secrets.get(&f.key)) {
                Some(_) if url_changed => return Err(format!("Enter {} again: the URL changed", f.label)),
                Some(sealed) => {
                    kept.insert(f.key.clone(), sealed.clone());
                }
                None if f.required => return Err(format!("{} needs {}", m.name, f.label)),
                None => {}
            },
        }
    }
    let streams = match &input.streams {
        Some(list) => {
            for s in list {
                if !m.streams.iter().any(|x| &x.id == s) {
                    return Err(format!("{s} is not a stream of {}", m.name));
                }
            }
            let mut list = list.clone();
            list.sort();
            list.dedup();
            list
        }
        None => previous.map(|p| p.streams.clone()).unwrap_or_else(|| m.streams.iter().filter(|s| s.default_enabled).map(|s| s.id.clone()).collect()),
    };
    let name = match input.name.trim() {
        "" => previous.map(|p| p.name.clone()).unwrap_or_else(|| engine::instance(m, &engine::effective_config(m, &config))),
        n if n.chars().count() > MAX_NAME_CHARS || n.chars().any(char::is_control) => return Err("The connection's name is not valid".into()),
        n => n.to_string(),
    };
    Ok(Checked { name, config, secrets, kept, streams, url_changed })
}

/// The engine's connection for a checked input: entered secrets, and the saved ones kept (unsealed).
fn engine_connection(st: &AppState, m: &Manifest, id: String, checked: &Checked, previous: Option<&store::Saved>) -> Result<engine::Connection, ApiError> {
    let mut conn = engine::Connection { id, name: checked.name.clone(), config: engine::effective_config(m, &checked.config), secrets: checked.secrets.clone() };
    if let Some(prev) = previous {
        let saved = store::unseal(st, &store::Saved { secrets: checked.kept.clone(), ..prev.clone() }).map_err(ApiError::BadRequest)?;
        conn.secrets.extend(saved.secrets);
    }
    super::connector_oauth::with_app(m, &mut conn);
    Ok(conn)
}

/// A fresh runtime through the SSRF guard: a test or a setup call never reuses a token cached for other credentials.
fn fresh_runtime(st: &AppState) -> Runtime {
    Runtime::new(Arc::new(connectors::net::Net { allow_private: st.config.portal_allow_private }), true)
}

fn package(id: &str) -> Result<&'static Arc<PackageConnector>, ApiError> {
    match registry().package(id) {
        Some(p) if p.package.manifest.push.is_none() || !p.package.manifest.streams.is_empty() => Ok(p),
        // A signed push: its connection holds the signing secret the intake checks requests with.
        Some(p) if p.package.manifest.push.as_ref().is_some_and(|x| x.signature.is_some()) => Ok(p),
        Some(_) => Err(ApiError::BadRequest(format!("{id} is installed from its tile: it has nothing to connect"))),
        None if registry().get(id).is_some() => Err(ApiError::BadRequest(format!("{id} is set up on its own page"))),
        None => Err(ApiError::NotFound(format!("No connector {id}"))),
    }
}

async fn view(st: &AppState, provider: &str, org: &str, id: &str, name: &str, streams: &[(String, String, bool)], saved: Option<&store::Saved>) -> Result<ConnectionView, sqlx::Error> {
    let jobs = jobs::status(st, provider, org, id).await?;
    let streams = streams
        .iter()
        .map(|(sid, sname, enabled)| {
            let j = jobs.iter().find(|j| &j.scope == sid);
            StreamStatus {
                id: sid.clone(),
                name: sname.clone(),
                enabled: *enabled,
                last_success_at: j.and_then(|j| j.last_success_at),
                collected_until: j.and_then(|j| j.collected_until),
                records: j.map_or(0, |j| j.metrics),
                written: j.map_or(0, |j| j.points),
                error: j.and_then(|j| j.error.as_ref()).map(|e| CrawlIssue { kind: e.kind.clone(), message: e.message.clone(), missing_action: e.missing_action.clone(), at: e.at }),
            }
        })
        .collect();
    Ok(ConnectionView {
        id: id.to_string(),
        name: name.to_string(),
        config: saved.map(|s| s.config.clone()).unwrap_or_default(),
        secrets: saved.map(|s| s.secrets.keys().cloned().collect()).unwrap_or_default(),
        enabled: saved.is_none_or(|s| s.enabled),
        streams,
        updated_at: saved.map(|s| s.updated_at),
    })
}

/// The org's connections of a connector, with their streams' status.
pub async fn connections_of(st: &AppState, org: &str, id: &str) -> Result<Vec<ConnectionView>, ApiError> {
    let c = registry().get(id).ok_or_else(|| ApiError::NotFound(format!("No connector {id}")))?;
    let mut out = Vec::new();
    match registry().package(id) {
        Some(p) => {
            for s in store::list(st, org, id).await? {
                let streams: Vec<(String, String, bool)> = p.package.manifest.streams.iter().map(|x| (x.id.clone(), x.name.clone(), s.streams.contains(&x.id))).collect();
                out.push(view(st, id, org, &s.id, &s.name, &streams, Some(&s)).await?);
            }
        }
        None => {
            for r in c.connections(st, org).await? {
                let scopes: Vec<(String, String, bool)> = jobs::status(st, c.provider(), org, &r.id).await?.into_iter().map(|j| (j.scope.clone(), j.scope, true)).collect();
                out.push(view(st, c.provider(), org, &r.id, &r.name, &scopes, None).await?);
            }
        }
    }
    Ok(out)
}

/// Asks the poller to run a connection's jobs now.
async fn poll_now(st: &AppState, provider: &str, org: &str, connection: &str) -> Result<u64, sqlx::Error> {
    Ok(sqlx::query("UPDATE cloud_crawl_jobs SET next_run_at = now() WHERE provider = $1 AND org_id = $2 AND account_id::text = $3")
        .bind(provider)
        .bind(org)
        .bind(connection)
        .execute(&st.postgres)
        .await?
        .rows_affected())
}

/// Installs the package's log pipeline (kept when the org has it) and its dashboards for the org.
pub async fn install_assets(st: &AppState, org: &str, user: Option<&str>, p: &PackageConnector) -> Result<(), ApiError> {
    if let Some(pipeline) = p.package.pipeline() {
        let input = super::log_pipelines::LogPipelineInput {
            name: pipeline["name"].as_str().unwrap_or(&p.package.manifest.name).to_string(),
            filter: super::datadog_api::logs::our_query(pipeline.pointer("/filter/query").and_then(Value::as_str).unwrap_or_default()),
            enabled: pipeline["is_enabled"].as_bool().unwrap_or(true),
            processors: super::datadog_api::logs::pipelines::to_ours(pipeline["processors"].as_array().map(Vec::as_slice).unwrap_or_default()),
        };
        super::log_pipelines::insert(st, org, &input, &format!("connector:{}", p.package.manifest.id)).await?;
    }
    for doc in p.package.dashboards() {
        if let Some(id) = doc["id"].as_str() {
            super::dashboards::upsert_seed(&st.postgres, org, user, id, &doc).await.map_err(ApiError::Internal)?;
        }
    }
    Ok(())
}

/// Whether the operator registered this install's OAuth app for the connector.
fn with_oauth_app(mut e: ConnectorEntry) -> ConnectorEntry {
    if let Some(o) = e.oauth.as_mut() {
        o.app_configured = super::connector_oauth::operator_app(&e.id).is_some();
    }
    e
}

/// Every connector of this server: bundled and operator packages, native connectors.
#[utoipa::path(get, path = "/api/v1/connectors", operation_id = "list_connectors", responses((status = 200, body = ConnectorCatalog)), tag = "integrations")]
pub async fn list_connectors(ctx: crate::auth::AuthContext) -> ApiResult<ConnectorCatalog> {
    let r = registry();
    let admin = ctx.access >= crate::auth::Access::Admin;
    Ok(Json(ConnectorCatalog {
        connectors: r.connectors.iter().map(|c| with_oauth_app(c.entry().clone())).collect(),
        invalid: if admin { r.invalid.iter().map(|(dir, problems)| InvalidPackage { dir: dir.clone(), problems: problems.clone() }).collect() } else { vec![] },
    }))
}

/// A connector and the org's connections of it, with their streams' status.
#[utoipa::path(get, path = "/api/v1/connectors/{id}", operation_id = "get_connector", params(("id" = String, Path)),
    responses((status = 200, body = ConnectorDetail), (status = 404, body = crate::error::ErrorBody)), tag = "integrations")]
pub async fn get_connector(State(st): State<AppState>, OrgId(org): OrgId, Path(id): Path<String>) -> ApiResult<ConnectorDetail> {
    let c = registry().get(&id).ok_or_else(|| ApiError::NotFound(format!("No connector {id}")))?;
    Ok(Json(ConnectorDetail { connector: with_oauth_app(c.entry().clone()), connections: connections_of(&st, &org, &id).await? }))
}

/// The org's connections of a connector.
#[utoipa::path(get, path = "/api/v1/connectors/{id}/connections", operation_id = "list_connector_connections", params(("id" = String, Path)),
    responses((status = 200, body = Vec<ConnectionView>), (status = 404, body = crate::error::ErrorBody)), tag = "integrations")]
pub async fn list_connections(State(st): State<AppState>, OrgId(org): OrgId, Path(id): Path<String>) -> ApiResult<Vec<ConnectionView>> {
    Ok(Json(connections_of(&st, &org, &id).await?))
}

async fn save(st: &AppState, ctx: &crate::auth::AuthContext, p: &PackageConnector, previous: Option<&store::Saved>, input: &ConnectionInput) -> Result<ConnectionView, ApiError> {
    save_with_tokens(st, ctx, p, previous, input, &Map::new()).await
}

/// Saves a connection; `tokens`: an OAuth sign-in's (`connector_oauth`), else the saved ones are kept while its URLs
/// stay the same.
pub async fn save_with_tokens(st: &AppState, ctx: &crate::auth::AuthContext, p: &PackageConnector, previous: Option<&store::Saved>, input: &ConnectionInput, tokens: &Map<String, Value>) -> Result<ConnectionView, ApiError> {
    let m = &p.package.manifest;
    let mut checked = check(m, input, previous, st.config.portal_allow_private).map_err(ApiError::BadRequest)?;
    if tokens.is_empty() {
        if let (Some(prev), false) = (previous, checked.url_changed) {
            for k in [oc_connectors::manifest::OAUTH_REFRESH_TOKEN, oc_connectors::manifest::OAUTH_ACCESS_TOKEN] {
                if let Some(v) = prev.secrets.get(k) {
                    checked.kept.insert(k.into(), v.clone());
                }
            }
        }
    } else {
        checked.secrets.extend(tokens.iter().map(|(k, v)| (k.clone(), v.clone())));
    }
    // The package's setup calls (starting a subscription) before anything is saved: one failing saves nothing.
    if !m.on_connect.is_empty() {
        let conn = engine_connection(st, m, previous.map_or_else(|| format!("new-{}", ctx.org_id), |p| p.id.clone()), &checked, previous)?;
        let rt = fresh_runtime(st);
        for call in &m.on_connect {
            engine::call(m, call, &conn, Map::new(), chrono::Utc::now().timestamp_millis(), &rt).await.map_err(|i| ApiError::BadRequest(i.message))?;
        }
    }
    let mut secrets = checked.kept.clone();
    secrets.extend(store::seal(st, &checked.secrets));
    let enabled = input.enabled.or(previous.map(|p| p.enabled)).unwrap_or(true);
    let _changed = EnabledChange(&ctx.org_id);
    let mut tx = st.postgres.begin().await?;
    let result = match previous {
        Some(prev) => {
            sqlx::query_scalar(
                "UPDATE connector_connections SET name = $4, config = $5, secrets = $6, streams = $7, enabled = $8, updated_by = $9::uuid, updated_at = now() \
                 WHERE org_id = $1 AND connector_id = $2 AND id = $3::uuid RETURNING id::text",
            )
            .bind(&ctx.org_id)
            .bind(&m.id)
            .bind(&prev.id)
            .bind(&checked.name)
            .bind(Value::Object(checked.config.clone()))
            .bind(Value::Object(secrets))
            .bind(&checked.streams)
            .bind(enabled)
            .bind(&ctx.user_id)
            .fetch_one(&mut *tx)
            .await
        }
        None => {
            sqlx::query_scalar(
                "INSERT INTO connector_connections (org_id, connector_id, name, config, secrets, streams, enabled, updated_by) VALUES ($1, $2, $3, $4, $5, $6, $7, $8::uuid) RETURNING id::text",
            )
            .bind(&ctx.org_id)
            .bind(&m.id)
            .bind(&checked.name)
            .bind(Value::Object(checked.config.clone()))
            .bind(Value::Object(secrets))
            .bind(&checked.streams)
            .bind(enabled)
            .bind(&ctx.user_id)
            .fetch_one(&mut *tx)
            .await
        }
    };
    let id: String = result.map_err(|e| match &e {
        sqlx::Error::Database(d) if d.is_unique_violation() => ApiError::Conflict(format!("A connection named {} exists", checked.name)),
        _ => ApiError::Postgres(e),
    })?;
    // Another account or instance: its streams start again from their lookback.
    if checked.url_changed {
        sqlx::query("DELETE FROM connector_stream_state WHERE org_id = $1 AND connection_id = $2::uuid").bind(&ctx.org_id).bind(&id).execute(&mut *tx).await?;
        sqlx::query("DELETE FROM cloud_crawl_jobs WHERE provider = $1 AND org_id = $2 AND account_id = $3::uuid").bind(&m.id).bind(&ctx.org_id).bind(&id).execute(&mut *tx).await?;
    }
    // The jobs now, rather than at the poller's next planning.
    let wanted: Vec<String> = if enabled { checked.streams.clone() } else { vec![] };
    sqlx::query("DELETE FROM cloud_crawl_jobs WHERE provider = $1 AND org_id = $2 AND account_id = $3::uuid AND NOT (scope = ANY($4))").bind(&m.id).bind(&ctx.org_id).bind(&id).bind(&wanted).execute(&mut *tx).await?;
    sqlx::query("INSERT INTO cloud_crawl_jobs (provider, org_id, account_id, scope) SELECT $1, $2, $3::uuid, s FROM unnest($4::text[]) AS s ON CONFLICT DO NOTHING")
        .bind(&m.id)
        .bind(&ctx.org_id)
        .bind(&id)
        .bind(&wanted)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    poll_now(st, &m.id, &ctx.org_id, &id).await?;
    install_assets(st, &ctx.org_id, Some(&ctx.user_id), p).await?;
    let (created, problems) = connectors::assets::install(st, ctx, p).await?;
    if !created.is_empty() {
        ctx.audit(st, "connectors.assets_created", &m.id, &created.join(", ")).await;
    }
    for problem in problems {
        tracing::warn!(connector = %m.id, %problem, "a package asset was not created");
    }
    let replaced: Vec<&str> = checked.secrets.keys().map(String::as_str).collect();
    let detail = format!("{}{}", checked.name, if replaced.is_empty() { String::new() } else { format!(", {} replaced", replaced.join(", ")) });
    ctx.audit(st, if previous.is_some() { "connectors.connection_updated" } else { "connectors.connection_created" }, &m.id, &detail).await;
    let saved = store::get(st, &ctx.org_id, &m.id, &id).await?.ok_or_else(|| ApiError::NotFound("The connection is gone".into()))?;
    let streams: Vec<(String, String, bool)> = m.streams.iter().map(|x| (x.id.clone(), x.name.clone(), saved.streams.contains(&x.id))).collect();
    Ok(view(st, &m.id, &ctx.org_id, &saved.id, &saved.name, &streams, Some(&saved)).await?)
}

/// Creates a connection (admins): its secrets are sealed, its streams start within a minute.
#[utoipa::path(post, path = "/api/v1/connectors/{id}/connections", operation_id = "create_connector_connection", params(("id" = String, Path)), request_body = ConnectionInput,
    responses((status = 200, body = ConnectionView), (status = 400, body = crate::error::ErrorBody), (status = 409, body = crate::error::ErrorBody)), tag = "integrations")]
pub async fn create_connection(State(st): State<AppState>, Admin(ctx): Admin, Path(id): Path<String>, Json(input): Json<ConnectionInput>) -> ApiResult<ConnectionView> {
    let p = package(&id)?;
    Ok(Json(save(&st, &ctx, p, None, &input).await?))
}

/// Changes a connection (admins); secrets left out are kept while its URLs stay the same.
#[utoipa::path(put, path = "/api/v1/connectors/{id}/connections/{connection}", operation_id = "update_connector_connection",
    params(("id" = String, Path), ("connection" = String, Path)), request_body = ConnectionInput,
    responses((status = 200, body = ConnectionView), (status = 400, body = crate::error::ErrorBody), (status = 404, body = crate::error::ErrorBody)), tag = "integrations")]
pub async fn update_connection(State(st): State<AppState>, Admin(ctx): Admin, Path((id, connection)): Path<(String, String)>, Json(input): Json<ConnectionInput>) -> ApiResult<ConnectionView> {
    let p = package(&id)?;
    let previous = store::get(&st, &ctx.org_id, &id, &connection).await?.ok_or_else(|| ApiError::NotFound("No such connection".into()))?;
    Ok(Json(save(&st, &ctx, p, Some(&previous), &input).await?))
}

/// Removes a connection, its secrets, state and polling (admins). Stored logs and metrics stay.
#[utoipa::path(delete, path = "/api/v1/connectors/{id}/connections/{connection}", operation_id = "delete_connector_connection",
    params(("id" = String, Path), ("connection" = String, Path)), responses((status = 204), (status = 404, body = crate::error::ErrorBody)), tag = "integrations")]
pub async fn delete_connection(State(st): State<AppState>, Admin(ctx): Admin, Path((id, connection)): Path<(String, String)>) -> Result<StatusCode, ApiError> {
    let p = package(&id)?;
    let saved = store::get(&st, &ctx.org_id, &id, &connection).await?.ok_or_else(|| ApiError::NotFound("No such connection".into()))?;
    let _changed = EnabledChange(&ctx.org_id);
    let mut tx = st.postgres.begin().await?;
    sqlx::query("DELETE FROM connector_connections WHERE org_id = $1 AND connector_id = $2 AND id = $3::uuid").bind(&ctx.org_id).bind(&id).bind(&saved.id).execute(&mut *tx).await?;
    sqlx::query("DELETE FROM cloud_crawl_jobs WHERE provider = $1 AND org_id = $2 AND account_id = $3::uuid").bind(&id).bind(&ctx.org_id).bind(&saved.id).execute(&mut *tx).await?;
    tx.commit().await?;
    // The last one: the package's monitors, rules, log-based metrics and pipeline go (its dashboards stay).
    if store::list(&st, &ctx.org_id, &id).await?.is_empty() {
        connectors::assets::remove(&st, &ctx, &id).await?;
    }
    ctx.audit(&st, "connectors.connection_deleted", &p.package.manifest.id, &saved.name).await;
    Ok(StatusCode::NO_CONTENT)
}

/// Tests a connection before or after saving it (admins): the package's test request, with the secrets entered or
/// (`connectionId`) the saved ones.
#[utoipa::path(post, path = "/api/v1/connectors/{id}/test", operation_id = "test_connector_connection", params(("id" = String, Path)), request_body = ConnectionInput,
    responses((status = 200, body = ConnectorTest), (status = 400, body = crate::error::ErrorBody)), tag = "integrations")]
pub async fn test_connection(State(st): State<AppState>, Admin(ctx): Admin, Path(id): Path<String>, Json(input): Json<ConnectionInput>) -> ApiResult<ConnectorTest> {
    let p = package(&id)?;
    let m = &p.package.manifest;
    let previous = match input.connection_id.as_deref() {
        Some(cid) => Some(store::get(&st, &ctx.org_id, &id, cid).await?.ok_or_else(|| ApiError::NotFound("No such connection".into()))?),
        None => None,
    };
    let mut checked = check(m, &input, previous.as_ref(), st.config.portal_allow_private).map_err(ApiError::BadRequest)?;
    if let (Some(prev), false) = (&previous, checked.url_changed) {
        for k in [oc_connectors::manifest::OAUTH_REFRESH_TOKEN, oc_connectors::manifest::OAUTH_ACCESS_TOKEN] {
            if let Some(v) = prev.secrets.get(k) {
                checked.kept.insert(k.into(), v.clone());
            }
        }
    }
    let conn = engine_connection(&st, m, previous.as_ref().map_or_else(|| format!("test-{}", ctx.org_id), |p| p.id.clone()), &checked, previous.as_ref())?;
    let rt = fresh_runtime(&st);
    let t = engine::test(m, &conn, chrono::Utc::now().timestamp_millis(), &rt).await;
    Ok(Json(ConnectorTest { ok: t.ok, message: t.message, records: t.records }))
}

/// For a push-only package: creates the push job or webhook on the vendor's side with the config entered
/// (`push.create`, admins). Nothing is stored.
#[utoipa::path(post, path = "/api/v1/connectors/{id}/push", operation_id = "create_connector_push", params(("id" = String, Path)), request_body = ConnectionInput,
    responses((status = 200, body = ConnectorTest), (status = 400, body = crate::error::ErrorBody)), tag = "integrations")]
pub async fn create_push(State(st): State<AppState>, Admin(ctx): Admin, Path(id): Path<String>, Json(input): Json<ConnectionInput>) -> ApiResult<ConnectorTest> {
    let p = registry().package(&id).ok_or_else(|| ApiError::NotFound(format!("No connector {id}")))?;
    let m = &p.package.manifest;
    let create = m.push.as_ref().and_then(|p| p.create.as_ref()).ok_or_else(|| ApiError::BadRequest(format!("{} sets up its push in its console", m.name)))?;
    let checked = check(m, &input, None, st.config.portal_allow_private).map_err(ApiError::BadRequest)?;
    let conn = engine_connection(&st, m, format!("push-{}", ctx.org_id), &checked, None)?;
    let url = input.push_url.as_deref().map(str::trim).filter(|u| u.starts_with("https://") || u.starts_with("http://")).ok_or_else(|| ApiError::BadRequest("The intake URL is missing".into()))?;
    let extra: Map<String, Value> = [("push_url".to_string(), Value::String(url.to_string()))].into_iter().collect();
    let result = engine::call(m, create, &conn, extra, chrono::Utc::now().timestamp_millis(), &fresh_runtime(&st)).await;
    ctx.audit(&st, "connectors.push_created", &m.id, if result.is_ok() { "ok" } else { "failed" }).await;
    Ok(Json(match result {
        Ok(message) => ConnectorTest { ok: true, message, records: None },
        Err(issue) => ConnectorTest { ok: false, message: issue.message, records: None },
    }))
}

/// Runs a connection's streams now instead of at their next interval (admins); native connectors' too.
#[utoipa::path(post, path = "/api/v1/connectors/{id}/connections/{connection}/sync", operation_id = "sync_connector_connection",
    params(("id" = String, Path), ("connection" = String, Path)), responses((status = 204), (status = 404, body = crate::error::ErrorBody)), tag = "integrations")]
pub async fn sync_connection(State(st): State<AppState>, Admin(ctx): Admin, Path((id, connection)): Path<(String, String)>) -> Result<StatusCode, ApiError> {
    let c = registry().get(&id).ok_or_else(|| ApiError::NotFound(format!("No connector {id}")))?;
    let mine = c.connections(&st, &ctx.org_id).await?;
    if !mine.iter().any(|r| r.id == connection) {
        return Err(ApiError::NotFound("No such connection".into()));
    }
    poll_now(&st, c.provider(), &ctx.org_id, &connection).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn hibp() -> &'static Manifest {
        &registry().package("have-i-been-pwned").expect("bundled").package.manifest
    }

    fn input(v: Value) -> ConnectionInput {
        serde_json::from_value(v).unwrap()
    }

    #[test]
    fn inputs_are_checked_against_the_package_form() {
        let nb = &registry().package("netbird").expect("bundled").package.manifest;
        let c = check(nb, &input(json!({ "config": { "base_url": " https://api.netbird.io/api/ " }, "secrets": { "token": "nbp_x" } })), None, false).unwrap();
        assert_eq!(c.config["base_url"], "https://api.netbird.io", "the paths add /api");
        assert_eq!((c.name.as_str(), c.streams.clone()), ("api.netbird.io", vec!["events".to_string(), "peers".to_string()]));
        for (body, needle) in [
            (json!({ "config": { "base_url": "http://netbird.example.com" }, "secrets": { "token": "t" } }), "https"),
            (json!({ "config": { "base_url": "https://10.0.0.5" }, "secrets": { "token": "t" } }), ""),
            (json!({ "config": { "base_url": "https://netbird.example.com" } }), "token"),
            (json!({ "config": { "base_url": "https://netbird.example.com", "nope": 1 }, "secrets": { "token": "t" } }), "not a field"),
            (json!({ "config": { "base_url": "https://netbird.example.com" }, "secrets": { "token": "t" }, "streams": ["nope"] }), "not a stream"),
        ] {
            let err = check(nb, &input(body.clone()), None, false).err().unwrap_or_else(|| panic!("{body} passed"));
            assert!(err.to_lowercase().contains(&needle.to_lowercase()), "{body}: {err}");
        }
        assert!(check(nb, &input(json!({ "config": { "base_url": "http://10.0.0.5:33073" }, "secrets": { "token": "t" } })), None, true).is_ok());
        assert_eq!(check(nb, &input(json!({ "secrets": { "token": "t" } })), None, false).unwrap().name, "api.netbird.io", "the URL's default: NetBird Cloud");
        let saved = store::Saved {
            id: "00000000-0000-4000-8000-000000000001".into(),
            name: "netbird.example.com".into(),
            config: json!({ "base_url": "https://netbird.example.com" }).as_object().unwrap().clone(),
            secrets: json!({ "token": "v1:sealed" }).as_object().unwrap().clone(),
            streams: vec!["events".into()],
            enabled: true,
            updated_at: 0,
        };
        let kept = check(nb, &input(json!({ "config": { "base_url": "https://netbird.example.com/" } })), Some(&saved), false).unwrap();
        assert_eq!((kept.kept["token"].as_str(), kept.streams.clone(), kept.url_changed), (Some("v1:sealed"), vec!["events".to_string()], false));
        let moved = check(nb, &input(json!({ "config": { "base_url": "https://vpn.example.com" } })), Some(&saved), false).err().unwrap();
        assert!(moved.contains("again"), "{moved}");
        assert!(hibp().config.iter().any(|f| f.secret));
    }

    #[tokio::test]
    async fn connections_belong_to_their_org_and_never_answer_secrets() {
        let Some((_, states)) = crate::test_support::replicas(1).await else { return };
        let st = &states[0];
        let a = crate::test_support::admin_of_new_org(st, "connectors-a").await;
        let b = crate::test_support::admin_of_new_org(st, "connectors-b").await;
        let key = "hibp-SuperSecret-0123456789";
        let body = || input(json!({ "config": { "domains": ["example.com"] }, "secrets": { "api_key": key } }));
        let saved = create_connection(State(st.clone()), Admin(a.clone()), Path("have-i-been-pwned".into()), Json(body())).await.unwrap().0;
        assert!(!serde_json::to_string(&saved).unwrap().contains(key));
        assert_eq!(saved.secrets, ["api_key"]);
        let sealed: Value = sqlx::query_scalar("SELECT secrets FROM connector_connections WHERE org_id = $1").bind(&a.org_id).fetch_one(&st.postgres).await.unwrap();
        assert!(!sealed.to_string().contains(key));
        // The jobs exist at once, one per enabled stream.
        let scopes: Vec<String> = sqlx::query_scalar("SELECT scope FROM cloud_crawl_jobs WHERE provider = 'have-i-been-pwned' AND org_id = $1 AND next_run_at <= now() ORDER BY scope").bind(&a.org_id).fetch_all(&st.postgres).await.unwrap();
        assert_eq!(scopes, saved.streams.iter().filter(|s| s.enabled).map(|s| s.id.clone()).collect::<Vec<_>>());
        // Installed and its dashboard listed for A only.
        assert!(connectors::connected(st, &a.org_id).await.unwrap().contains("have-i-been-pwned"));
        assert!(!connectors::connected(st, &b.org_id).await.unwrap().contains("have-i-been-pwned"));
        assert!(super::super::integration_installs::installed(st, &a.org_id).await.unwrap().contains(&"have-i-been-pwned".to_string()));
        assert!(super::super::dashboards::enabled_integrations(st, &a.org_id).await.unwrap().contains("have-i-been-pwned"));
        // B sees, changes, syncs and deletes nothing of A's.
        assert!(list_connections(State(st.clone()), OrgId(b.org_id.clone()), Path("have-i-been-pwned".into())).await.unwrap().0.is_empty());
        let other = Path(("have-i-been-pwned".to_string(), saved.id.clone()));
        assert!(matches!(update_connection(State(st.clone()), Admin(b.clone()), other, Json(body())).await, Err(ApiError::NotFound(_))));
        assert!(matches!(delete_connection(State(st.clone()), Admin(b.clone()), Path(("have-i-been-pwned".into(), saved.id.clone()))).await, Err(ApiError::NotFound(_))));
        assert!(matches!(sync_connection(State(st.clone()), Admin(b.clone()), Path(("have-i-been-pwned".into(), saved.id.clone()))).await, Err(ApiError::NotFound(_))));
        let probe = input(json!({ "config": { "domains": ["example.com"] }, "connectionId": saved.id }));
        assert!(matches!(test_connection(State(st.clone()), Admin(b.clone()), Path("have-i-been-pwned".into()), Json(probe)).await, Err(ApiError::NotFound(_))), "B can't test with A's key");
        // A keeps its key when it leaves it out; the same name twice is a conflict.
        let kept = update_connection(State(st.clone()), Admin(a.clone()), Path(("have-i-been-pwned".into(), saved.id.clone())), Json(input(json!({ "config": { "domains": ["example.com", "shop.example.com"] } })))).await.unwrap().0;
        assert_eq!(kept.config["domains"], json!(["example.com", "shop.example.com"]));
        let s = store::get(st, &a.org_id, "have-i-been-pwned", &saved.id).await.unwrap().unwrap();
        assert_eq!(store::unseal(st, &s).unwrap().secrets["api_key"], key);
        let dup = create_connection(State(st.clone()), Admin(a.clone()), Path("have-i-been-pwned".into()), Json(input(json!({ "name": saved.name, "config": { "domains": ["example.com"] }, "secrets": { "api_key": key } })))).await;
        assert!(matches!(dup, Err(ApiError::Conflict(_))));
        // A native connector is listed but set up on its page.
        assert!(matches!(create_connection(State(st.clone()), Admin(a.clone()), Path("authentik".into()), Json(body())).await, Err(ApiError::BadRequest(m)) if m.contains("own page")));
        // Deleting removes the jobs and the installed state.
        assert_eq!(delete_connection(State(st.clone()), Admin(a.clone()), Path(("have-i-been-pwned".into(), saved.id.clone()))).await.unwrap(), StatusCode::NO_CONTENT);
        let left: i64 = sqlx::query_scalar("SELECT count(*) FROM cloud_crawl_jobs WHERE provider = 'have-i-been-pwned' AND org_id = $1").bind(&a.org_id).fetch_one(&st.postgres).await.unwrap();
        assert_eq!(left, 0);
        assert!(!connectors::connected(st, &a.org_id).await.unwrap().contains("have-i-been-pwned"));
    }
}
