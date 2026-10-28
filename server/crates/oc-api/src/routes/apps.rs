//! Automation → App Builder: pages of components that run workflows and show their results.
//!
//! An app is a list of components laid out on a 12-column grid: headings and text, inputs (text,
//! number, select), buttons, tables and metric values. A **button** runs a published workflow that
//! has an app trigger, filling the workflow's inputs from templates over the app's inputs
//! (`{{ components.<id>.value }}`); the run acts as the member who published the workflow. A
//! **table** shows a list from the latest run of a button (`Steps.<step>.output.<list>`) and a
//! **metric** the value of a metric query. Standard users and admins build, publish and use apps;
//! every member can open them.

use std::collections::{BTreeMap, HashSet};

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use utoipa::ToSchema;

use super::portal_action_steps::{self as steps, RenderContext, TemplateScope};
use crate::auth::{AuthContext, Writer};
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;

const MAX_APPS: i64 = 500;
const MAX_COMPONENTS: usize = 60;
const MAX_TAGS: usize = 20;
const ROOTS: &[&str] = &["components"];

#[derive(Serialize, Deserialize, ToSchema, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum AppComponentKind {
    Heading,
    Text,
    TextInput,
    NumberInput,
    Select,
    Button,
    Table,
    Metric,
}

impl AppComponentKind {
    fn is_input(self) -> bool {
        matches!(self, AppComponentKind::TextInput | AppComponentKind::NumberInput | AppComponentKind::Select)
    }
}

/// A component of an app; the fields used depend on its kind.
#[derive(Serialize, Deserialize, ToSchema, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AppComponent {
    /// `[a-z0-9_]`, unique in the app (`components.<id>.value`).
    pub id: String,
    pub kind: AppComponentKind,
    #[serde(default)]
    pub label: String,
    /// Heading and text: the content.
    #[serde(default)]
    pub text: String,
    /// Inputs: pre-filled value.
    #[serde(default)]
    pub default_value: String,
    #[serde(default)]
    pub placeholder: String,
    /// Select: its options.
    #[serde(default)]
    pub options: Vec<String>,
    /// Button: the workflow it runs.
    #[serde(default)]
    pub workflow_id: String,
    /// Button: a template per workflow input key (`{{ components.env.value }}`).
    #[serde(default)]
    pub inputs: BTreeMap<String, String>,
    /// Table: the button whose latest run it shows.
    #[serde(default)]
    pub source: String,
    /// Table: path of the list in the run's data (`Steps.top.output.groups`).
    #[serde(default)]
    pub path: String,
    /// Table: columns (keys of the list's items); all when empty.
    #[serde(default)]
    pub columns: Vec<String>,
    /// Metric: the metric query.
    #[serde(default)]
    pub query: String,
    /// Metric: avg, sum, min, max or last.
    #[serde(default)]
    pub aggregation: String,
    #[serde(default)]
    pub unit: String,
    /// Columns of the 12-column grid it spans (3, 4, 6, 8 or 12).
    #[serde(default = "full_width")]
    pub width: u8,
}

fn full_width() -> u8 {
    12
}

#[derive(Serialize, Deserialize, ToSchema, Clone, Debug, PartialEq, Default)]
pub struct AutomationAppSpec {
    #[serde(default)]
    pub components: Vec<AppComponent>,
}

#[derive(Deserialize, ToSchema, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct AutomationAppInput {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub blueprint: String,
    pub spec: AutomationAppSpec,
    #[serde(default)]
    pub published: bool,
}

#[derive(Serialize, ToSchema, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct AutomationApp {
    pub id: String,
    pub name: String,
    pub description: String,
    pub tags: Vec<String>,
    pub blueprint: String,
    pub spec: AutomationAppSpec,
    pub published: bool,
    pub created_by: String,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Deserialize, ToSchema, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct AutomationAppRunInput {
    /// The button pressed.
    pub component: String,
    /// The app's input values, by component id.
    #[serde(default)]
    #[schema(value_type = std::collections::HashMap<String, serde_json::Value>)]
    pub values: Map<String, Value>,
}

#[derive(Serialize, ToSchema, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct AutomationAppRun {
    pub workflow_id: String,
    pub run_id: i64,
}

// ================================================================ checks

fn text(value: &str, what: &str, max: usize, required: bool) -> Result<String, String> {
    let v = value.trim();
    if required && v.is_empty() {
        return Err(format!("{what} is required"));
    }
    if v.chars().count() > max {
        return Err(format!("{what} is longer than {max} characters"));
    }
    Ok(v.to_string())
}

fn id_ok(id: &str) -> bool {
    !id.is_empty() && id.len() <= 40 && id.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

/// Checks an app's components: ids, what each kind needs, button templates and table sources.
pub fn check_spec(spec: &AutomationAppSpec) -> Result<AutomationAppSpec, String> {
    if spec.components.len() > MAX_COMPONENTS {
        return Err(format!("An app has at most {MAX_COMPONENTS} components"));
    }
    let mut ids = HashSet::new();
    for c in &spec.components {
        if !id_ok(c.id.trim()) {
            return Err(format!("{} isn't a component id (a–z, 0–9 and _, up to 40)", c.id.trim()));
        }
        if !ids.insert(c.id.trim().to_string()) {
            return Err(format!("The component id {} is used twice", c.id.trim()));
        }
    }
    let buttons: HashSet<&str> = spec.components.iter().filter(|c| c.kind == AppComponentKind::Button).map(|c| c.id.trim()).collect();
    let names = HashSet::new();
    let scope = TemplateScope { inputs: &[], secrets: &names, allow_secrets: false, prior_steps: 0, roots: ROOTS };
    let mut out = Vec::with_capacity(spec.components.len());
    for c in &spec.components {
        let id = c.id.trim().to_string();
        let at = |e: String| format!("Component {id}: {e}");
        if ![3, 4, 6, 8, 12].contains(&c.width) {
            return Err(at("the width is 3, 4, 6, 8 or 12 columns".into()));
        }
        let mut k = AppComponent {
            id: id.clone(),
            kind: c.kind,
            label: text(&c.label, "The label", 200, false).map_err(at)?,
            text: String::new(),
            default_value: String::new(),
            placeholder: String::new(),
            options: vec![],
            workflow_id: String::new(),
            inputs: BTreeMap::new(),
            source: String::new(),
            path: String::new(),
            columns: vec![],
            query: String::new(),
            aggregation: String::new(),
            unit: String::new(),
            width: c.width,
        };
        match c.kind {
            AppComponentKind::Heading | AppComponentKind::Text => k.text = text(&c.text, "The text", 5000, true).map_err(at)?,
            kind if kind.is_input() => {
                k.default_value = text(&c.default_value, "The default value", 1000, false).map_err(at)?;
                k.placeholder = text(&c.placeholder, "The placeholder", 200, false).map_err(at)?;
                if kind == AppComponentKind::Select {
                    k.options = c.options.iter().map(|o| o.trim().to_string()).filter(|o| !o.is_empty()).collect();
                    let unique: HashSet<&String> = k.options.iter().collect();
                    if k.options.is_empty() || k.options.len() > 50 || unique.len() != k.options.len() {
                        return Err(at("a select needs 1–50 distinct options".into()));
                    }
                }
                if kind == AppComponentKind::NumberInput && !k.default_value.is_empty() && k.default_value.parse::<f64>().is_err() {
                    return Err(at("the default value must be a number".into()));
                }
            }
            AppComponentKind::Button => {
                k.workflow_id = c.workflow_id.trim().to_string();
                if k.workflow_id.is_empty() {
                    return Err(at("choose the workflow the button runs".into()));
                }
                for (key, tpl) in &c.inputs {
                    steps::check_template(tpl, &scope).map_err(at)?;
                    for piece in referenced(tpl) {
                        if !ids.contains(&piece) {
                            return Err(at(format!("there's no component {piece}")));
                        }
                    }
                    k.inputs.insert(key.trim().to_string(), tpl.trim().to_string());
                }
            }
            AppComponentKind::Table => {
                k.source = c.source.trim().to_string();
                if !buttons.contains(k.source.as_str()) {
                    return Err(at("a table shows the latest run of one of the app's buttons".into()));
                }
                k.path = text(&c.path, "The path", 500, true).map_err(at)?;
                k.columns = c.columns.iter().map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).take(20).collect();
            }
            AppComponentKind::Metric => {
                k.query = text(&c.query, "The query", 2000, true).map_err(at)?;
                k.aggregation = if c.aggregation.trim().is_empty() { "avg".into() } else { c.aggregation.trim().to_string() };
                if !["avg", "sum", "min", "max", "last"].contains(&k.aggregation.as_str()) {
                    return Err(at("the aggregation is avg, sum, min, max or last".into()));
                }
                k.unit = text(&c.unit, "The unit", 40, false).map_err(at)?;
            }
            _ => {}
        }
        out.push(k);
    }
    Ok(AutomationAppSpec { components: out })
}

/// Component ids a template reads (`{{ components.env.value }}` → `env`).
fn referenced(template: &str) -> Vec<String> {
    template.split("{{").skip(1).filter_map(|p| p.split("}}").next()).filter_map(|p| p.trim().strip_prefix("components.").map(|r| r.split('.').next().unwrap_or("").to_string())).collect()
}

/// The workflow inputs a button sends, its templates filled with the app's values.
pub fn button_inputs(button: &AppComponent, values: &Map<String, Value>) -> Result<Map<String, Value>, String> {
    let components: Map<String, Value> = values.iter().map(|(k, v)| (k.clone(), json!({"value": v}))).collect();
    let vars = json!({"components": components});
    let empty = Map::new();
    let secrets = BTreeMap::new();
    let rc = RenderContext { inputs: &empty, user_email: "", user_name: "", secrets: &secrets, steps: vec![], vars: Some(&vars) };
    let mut out = Map::new();
    for (key, tpl) in &button.inputs {
        // A template that is one placeholder keeps the value's type (numbers, booleans).
        let trimmed = tpl.trim();
        let single = trimmed.starts_with("{{") && trimmed.ends_with("}}") && trimmed.matches("{{").count() == 1;
        let value = if single {
            let path = trimmed.trim_start_matches("{{").trim_end_matches("}}").trim();
            super::workflow_expr::lookup(&vars, &super::workflow_expr::segments(path)).cloned().unwrap_or(Value::Null)
        } else {
            Value::String(steps::render_text(tpl, &rc, false)?.trim().to_string())
        };
        if !value.is_null() && value.as_str() != Some("") {
            out.insert(key.clone(), value);
        }
    }
    Ok(out)
}

// ================================================================ storage

fn valid_uuid(id: &str) -> bool {
    id.len() == 36 && id.chars().enumerate().all(|(i, c)| if [8, 13, 18, 23].contains(&i) { c == '-' } else { c.is_ascii_hexdigit() })
}

fn not_found() -> ApiError {
    ApiError::NotFound("App not found".into())
}

#[derive(sqlx::FromRow)]
struct AppRow {
    id: String,
    name: String,
    description: String,
    tags: Vec<String>,
    blueprint: String,
    spec: Value,
    published: bool,
    created_by: String,
    created_at: i64,
    updated_at: i64,
}

impl AppRow {
    fn view(self) -> Result<AutomationApp, ApiError> {
        Ok(AutomationApp {
            spec: serde_json::from_value(self.spec).map_err(|e| ApiError::Internal(anyhow::anyhow!("stored app unreadable: {e}")))?,
            id: self.id,
            name: self.name,
            description: self.description,
            tags: self.tags,
            blueprint: self.blueprint,
            published: self.published,
            created_by: self.created_by,
            created_at: self.created_at,
            updated_at: self.updated_at,
        })
    }
}

async fn app_rows(st: &AppState, org: &str, id: Option<&str>) -> Result<Vec<AppRow>, ApiError> {
    Ok(sqlx::query_as(
        "SELECT a.id::text, a.name, a.description, a.tags, a.blueprint, a.spec, a.published, COALESCE(u.name, '') AS created_by, \
         (extract(epoch FROM a.created_at) * 1000)::bigint AS created_at, (extract(epoch FROM a.updated_at) * 1000)::bigint AS updated_at \
         FROM automation_apps a LEFT JOIN users u ON u.id = a.created_by WHERE a.org_id = $1 AND ($2::text IS NULL OR a.id::text = $2) ORDER BY a.name",
    )
    .bind(org)
    .bind(id)
    .fetch_all(&st.postgres)
    .await?)
}

async fn app_row(st: &AppState, org: &str, id: &str) -> Result<AppRow, ApiError> {
    if !valid_uuid(id) {
        return Err(not_found());
    }
    app_rows(st, org, Some(id)).await?.pop().ok_or_else(not_found)
}

struct Checked {
    name: String,
    description: String,
    tags: Vec<String>,
    blueprint: String,
    spec: AutomationAppSpec,
}

fn check_app(input: &AutomationAppInput) -> Result<Checked, String> {
    let name = text(&input.name, "The name", 200, true)?;
    let description = text(&input.description, "The description", 2000, false)?;
    if input.tags.len() > MAX_TAGS {
        return Err(format!("An app has at most {MAX_TAGS} tags"));
    }
    let mut tags: Vec<String> = Vec::new();
    for t in &input.tags {
        let t = text(t, "A tag", 100, true)?;
        if !tags.contains(&t) {
            tags.push(t);
        }
    }
    let blueprint = text(&input.blueprint, "The blueprint", 100, false)?;
    Ok(Checked { name, description, tags, blueprint, spec: check_spec(&input.spec)? })
}

fn name_taken(e: sqlx::Error, name: &str) -> ApiError {
    match &e {
        sqlx::Error::Database(d) if d.is_unique_violation() => ApiError::BadRequest(format!("An app named {name} exists already")),
        _ => e.into(),
    }
}

// ================================================================ handlers

#[utoipa::path(get, path = "/api/v1/apps", operation_id = "list_automation_apps", responses((status = 200, body = Vec<AutomationApp>)), tag = "automation")]
pub async fn list_apps(State(st): State<AppState>, ctx: AuthContext) -> ApiResult<Vec<AutomationApp>> {
    Ok(Json(app_rows(&st, &ctx.org_id, None).await?.into_iter().map(AppRow::view).collect::<Result<_, _>>()?))
}

#[utoipa::path(get, path = "/api/v1/apps/{id}", operation_id = "get_automation_app", params(("id" = String, Path)),
    responses((status = 200, body = AutomationApp), (status = 404, body = crate::error::ErrorBody)), tag = "automation")]
pub async fn get_app(State(st): State<AppState>, ctx: AuthContext, Path(id): Path<String>) -> ApiResult<AutomationApp> {
    Ok(Json(app_row(&st, &ctx.org_id, &id).await?.view()?))
}

#[utoipa::path(post, path = "/api/v1/apps", operation_id = "create_automation_app", request_body = AutomationAppInput,
    responses((status = 200, body = AutomationApp), (status = 400, body = crate::error::ErrorBody)), tag = "automation")]
pub async fn create_app(State(st): State<AppState>, Writer(ctx): Writer, Json(input): Json<AutomationAppInput>) -> ApiResult<AutomationApp> {
    let a = check_app(&input).map_err(ApiError::BadRequest)?;
    let (count,): (i64,) = sqlx::query_as("SELECT count(*) FROM automation_apps WHERE org_id = $1").bind(&ctx.org_id).fetch_one(&st.postgres).await?;
    if count >= MAX_APPS {
        return Err(ApiError::BadRequest(format!("An organization has at most {MAX_APPS} apps")));
    }
    let (id,): (String,) = sqlx::query_as(
        "INSERT INTO automation_apps (org_id, name, description, tags, blueprint, spec, published, created_by) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, (SELECT id FROM users WHERE org_id = $1 AND id::text = $8)) RETURNING id::text",
    )
    .bind(&ctx.org_id)
    .bind(&a.name)
    .bind(&a.description)
    .bind(&a.tags)
    .bind(&a.blueprint)
    .bind(serde_json::to_value(&a.spec).map_err(anyhow::Error::from)?)
    .bind(input.published)
    .bind(&ctx.user_id)
    .fetch_one(&st.postgres)
    .await
    .map_err(|e| name_taken(e, &a.name))?;
    ctx.audit(&st, "app.created", &id, &a.name).await;
    get_app(State(st), ctx, Path(id)).await
}

#[utoipa::path(put, path = "/api/v1/apps/{id}", operation_id = "update_automation_app", params(("id" = String, Path)), request_body = AutomationAppInput,
    responses((status = 200, body = AutomationApp), (status = 400, body = crate::error::ErrorBody), (status = 404, body = crate::error::ErrorBody)), tag = "automation")]
pub async fn update_app(State(st): State<AppState>, Writer(ctx): Writer, Path(id): Path<String>, Json(input): Json<AutomationAppInput>) -> ApiResult<AutomationApp> {
    let row = app_row(&st, &ctx.org_id, &id).await?;
    let a = check_app(&input).map_err(ApiError::BadRequest)?;
    sqlx::query("UPDATE automation_apps SET name = $3, description = $4, tags = $5, blueprint = $6, spec = $7, published = $8, updated_at = now() WHERE org_id = $1 AND id::text = $2")
        .bind(&ctx.org_id)
        .bind(&id)
        .bind(&a.name)
        .bind(&a.description)
        .bind(&a.tags)
        .bind(&a.blueprint)
        .bind(serde_json::to_value(&a.spec).map_err(anyhow::Error::from)?)
        .bind(input.published)
        .execute(&st.postgres)
        .await
        .map_err(|e| name_taken(e, &a.name))?;
    let what = if input.published != row.published {
        if input.published {
            "app.published"
        } else {
            "app.unpublished"
        }
    } else {
        "app.updated"
    };
    ctx.audit(&st, what, &id, &a.name).await;
    get_app(State(st), ctx, Path(id)).await
}

#[utoipa::path(delete, path = "/api/v1/apps/{id}", operation_id = "delete_automation_app", params(("id" = String, Path)),
    responses((status = 204), (status = 404, body = crate::error::ErrorBody)), tag = "automation")]
pub async fn delete_app(State(st): State<AppState>, Writer(ctx): Writer, Path(id): Path<String>) -> Result<StatusCode, ApiError> {
    let row = app_row(&st, &ctx.org_id, &id).await?;
    sqlx::query("DELETE FROM automation_apps WHERE org_id = $1 AND id::text = $2").bind(&ctx.org_id).bind(&id).execute(&st.postgres).await?;
    ctx.audit(&st, "app.deleted", &id, &row.name).await;
    Ok(StatusCode::NO_CONTENT)
}

/// Presses a button: runs its workflow with the app's values. Read the run with `get_workflow_run`.
#[utoipa::path(post, path = "/api/v1/apps/{id}/run", operation_id = "run_automation_app", params(("id" = String, Path)), request_body = AutomationAppRunInput,
    responses((status = 200, body = AutomationAppRun), (status = 400, body = crate::error::ErrorBody), (status = 404, body = crate::error::ErrorBody)), tag = "automation")]
pub async fn run_app(State(st): State<AppState>, Writer(ctx): Writer, Path(id): Path<String>, Json(input): Json<AutomationAppRunInput>) -> ApiResult<AutomationAppRun> {
    let row = app_row(&st, &ctx.org_id, &id).await?;
    let app = row.view()?;
    let button =
        app.spec.components.iter().find(|c| c.id == input.component && c.kind == AppComponentKind::Button).ok_or_else(|| ApiError::BadRequest(format!("The app has no button {}", input.component)))?;
    let inputs = button_inputs(button, &input.values).map_err(ApiError::BadRequest)?;
    let run_id = super::workflows::start_from_app(&st, &ctx, &button.workflow_id, &app.name, &inputs).await?;
    ctx.audit(&st, "app.run", &id, &format!("{}: {} → run #{run_id}", app.name, button.id)).await;
    Ok(Json(AutomationAppRun { workflow_id: button.workflow_id.clone(), run_id }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn component(id: &str, kind: AppComponentKind) -> AppComponent {
        AppComponent {
            id: id.into(),
            kind,
            label: String::new(),
            text: String::new(),
            default_value: String::new(),
            placeholder: String::new(),
            options: vec![],
            workflow_id: String::new(),
            inputs: BTreeMap::new(),
            source: String::new(),
            path: String::new(),
            columns: vec![],
            query: String::new(),
            aggregation: String::new(),
            unit: String::new(),
            width: 12,
        }
    }

    fn restart_app() -> AutomationAppSpec {
        let mut env = component("env", AppComponentKind::Select);
        env.options = vec!["staging".into(), "production".into()];
        let replicas = component("replicas", AppComponentKind::NumberInput);
        let mut button = component("restart", AppComponentKind::Button);
        button.workflow_id = "6c1f2a10-3b4d-4e5f-8a9b-0c1d2e3f4b01".into();
        button.inputs = BTreeMap::from([
            ("environment".to_string(), "{{ components.env.value }}".to_string()),
            ("replicas".to_string(), "{{components.replicas.value}}".to_string()),
            ("note".to_string(), "Restart in {{ components.env.value }}".to_string()),
        ]);
        let mut table = component("results", AppComponentKind::Table);
        table.source = "restart".into();
        table.path = "Steps.restart.output.body".into();
        AutomationAppSpec { components: vec![env, replicas, button, table] }
    }

    #[test]
    fn apps_are_checked() {
        assert!(check_spec(&restart_app()).is_ok());
        let mut s = restart_app();
        s.components[2].inputs.insert("x".into(), "{{ components.missing.value }}".into());
        assert!(check_spec(&s).unwrap_err().contains("missing"));
        let mut s = restart_app();
        s.components[2].inputs.insert("x".into(), "{{ secrets.token }}".into());
        assert!(check_spec(&s).is_err(), "only components can be read");
        let mut s = restart_app();
        s.components[3].source = "env".into();
        assert!(check_spec(&s).unwrap_err().contains("buttons"));
        let mut s = restart_app();
        s.components[0].options.clear();
        assert!(check_spec(&s).unwrap_err().contains("options"));
        let mut s = restart_app();
        s.components[1].id = "env".into();
        assert!(check_spec(&s).unwrap_err().contains("twice"));
        let mut s = restart_app();
        s.components[1].width = 5;
        assert!(check_spec(&s).unwrap_err().contains("width"));
        let mut s = restart_app();
        s.components[2].workflow_id.clear();
        assert!(check_spec(&s).unwrap_err().contains("workflow"));
    }

    #[test]
    fn buttons_fill_workflow_inputs() {
        let spec = restart_app();
        let values = Map::from_iter([("env".to_string(), json!("production")), ("replicas".to_string(), json!(3))]);
        let out = button_inputs(&spec.components[2], &values).unwrap();
        assert_eq!(Value::Object(out), json!({"environment": "production", "replicas": 3, "note": "Restart in production"}));
        let out = button_inputs(&spec.components[2], &Map::new()).unwrap();
        assert_eq!(Value::Object(out), json!({"note": "Restart in"}), "empty values are left out");
    }
}
