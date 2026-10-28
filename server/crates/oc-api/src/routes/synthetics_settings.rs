//! Synthetic Monitoring & Testing → Settings: the org's default settings (enforced tag rules and the
//! test editor's defaults) and global variables.
//!
//! - **Defaults.** Locations, tags, timeout, frequency, retries and the minimum failure duration the
//!   editor starts a new test with. Enforced rules are checked when a test is saved: with
//!   `enforceTagKeys` every test needs a tag for each key of the default tags (`team:` in the
//!   defaults: some `team:<value>` tag), with `enforceEnv` an environment.
//! - **Global variables.** `{{ NAME }}` in a test's URL, host, headers, body, credentials, DNS
//!   server and assertion targets is replaced with the variable's value when the test runs (also
//!   from the editor's Send). Secure values are write-only: the API never returns them and they're
//!   masked in stored results.

use std::collections::HashMap;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;

use super::synthetics::{SyntheticRetry, FREQUENCIES, LOCATION_ID};
use super::synthetics_exec::{SyntheticAssertion, SyntheticRequest, SyntheticRunDetail, MAX_TIMEOUT_SECONDS};
use crate::auth::{AuthContext, Writer};
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;

const MAX_VARIABLES: i64 = 500;
const MAX_VARIABLE_NAME: usize = 80;
const MAX_VARIABLE_VALUE: usize = 10_000;
const MAX_DEFAULT_TAGS: usize = 20;
/// What secure values become in stored results.
pub const MASK: &str = "••••••";

// ================================================================ API types

/// What the test editor starts a new test with.
#[derive(Serialize, Deserialize, ToSchema, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct SyntheticDefaults {
    pub locations: Vec<String>,
    pub tags: Vec<String>,
    /// 1–60 seconds.
    pub timeout_seconds: u32,
    pub frequency_seconds: i32,
    pub retry: SyntheticRetry,
    pub min_failure_duration_minutes: i32,
}

impl Default for SyntheticDefaults {
    fn default() -> Self {
        SyntheticDefaults { locations: vec![LOCATION_ID.to_string()], tags: Vec::new(), timeout_seconds: 30, frequency_seconds: 300, retry: SyntheticRetry::default(), min_failure_duration_minutes: 0 }
    }
}

#[derive(Serialize, Deserialize, ToSchema, Clone, Debug, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct SyntheticSettings {
    /// Every test needs a tag for each key of the default tags.
    #[serde(default)]
    pub enforce_tag_keys: bool,
    /// Every test needs an environment.
    #[serde(default)]
    pub enforce_env: bool,
    #[serde(default)]
    pub defaults: SyntheticDefaults,
}

#[derive(Serialize, ToSchema, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SyntheticVariableUse {
    pub id: String,
    pub name: String,
}

#[derive(Serialize, ToSchema, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SyntheticVariable {
    pub id: String,
    pub name: String,
    /// Absent for secure variables.
    pub value: Option<String>,
    pub secure: bool,
    pub description: String,
    pub tags: Vec<String>,
    /// Tests that use the variable.
    pub used_by: Vec<SyntheticVariableUse>,
    pub created_by: Option<String>,
    pub updated_by: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Deserialize, ToSchema, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SyntheticVariableInput {
    /// Upper-case letters, digits and underscores, starting with a letter.
    pub name: String,
    /// Required on create; on update, absent keeps the stored value (secure variables are never sent back).
    pub value: Option<String>,
    #[serde(default)]
    pub secure: bool,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub tags: Vec<String>,
}

// ================================================================ rules

/// A variable name: `A`–`Z`, digits and `_`, starting with a letter.
pub fn check_variable_name(raw: &str) -> Result<String, String> {
    let name = raw.trim();
    let valid = name.len() <= MAX_VARIABLE_NAME && name.starts_with(|c: char| c.is_ascii_uppercase()) && name.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_');
    if valid {
        Ok(name.to_string())
    } else {
        Err(format!("Variable names are upper-case letters, digits and underscores, start with a letter and have at most {MAX_VARIABLE_NAME} characters"))
    }
}

/// The `{{ NAME }}` occurrences of a text: (start, end, name).
fn occurrences(text: &str) -> Vec<(usize, usize, String)> {
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(open) = text[from..].find("{{").map(|i| from + i) {
        let Some(close) = text[open + 2..].find("}}").map(|i| open + 2 + i) else { break };
        let name = text[open + 2..close].trim();
        if check_variable_name(name).is_ok() {
            out.push((open, close + 2, name.to_string()));
            from = close + 2;
        } else {
            from = open + 2;
        }
    }
    out
}

/// The variable names a text uses (`{{ NAME }}`, spaces optional), in order, without repeats.
pub fn placeholders(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for (_, _, name) in occurrences(text) {
        if !out.contains(&name) {
            out.push(name);
        }
    }
    out
}

/// The text with every `{{ NAME }}` replaced by a plain word, to check its shape (a URL, a host).
pub fn neutralized(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut last = 0;
    for (start, end, _) in occurrences(text) {
        out.push_str(&text[last..start]);
        out.push_str("var");
        last = end;
    }
    out.push_str(&text[last..]);
    out
}

/// The text with every known `{{ NAME }}` replaced; unknown names are collected in `missing`.
fn fill(text: &str, vars: &HashMap<String, String>, missing: &mut Vec<String>) -> String {
    let mut out = String::with_capacity(text.len());
    let mut last = 0;
    for (start, end, name) in occurrences(text) {
        out.push_str(&text[last..start]);
        match vars.get(&name) {
            Some(v) => out.push_str(v),
            None => {
                out.push_str(&text[start..end]);
                if !missing.contains(&name) {
                    missing.push(name);
                }
            }
        }
        last = end;
    }
    out.push_str(&text[last..]);
    out
}

/// Every text of a request that may use variables, mutably.
fn request_texts(r: &mut SyntheticRequest) -> Vec<&mut String> {
    let mut out = vec![&mut r.url, &mut r.host, &mut r.body, &mut r.dns_server];
    for h in &mut r.headers {
        out.push(&mut h.value);
    }
    if let Some(a) = &mut r.basic_auth {
        out.push(&mut a.username);
        out.push(&mut a.password);
    }
    out
}

/// The request and assertions with variables replaced; fails naming unknown variables.
pub fn substitute(request: &SyntheticRequest, assertions: &[SyntheticAssertion], vars: &HashMap<String, String>) -> Result<(SyntheticRequest, Vec<SyntheticAssertion>), String> {
    let mut missing = Vec::new();
    let mut r = request.clone();
    for text in request_texts(&mut r) {
        *text = fill(text, vars, &mut missing);
    }
    let mut a = assertions.to_vec();
    for x in &mut a {
        x.target = fill(&x.target, vars, &mut missing);
    }
    if missing.is_empty() {
        Ok((r, a))
    } else {
        Err(format!("Unknown global variables: {}", missing.join(", ")))
    }
}

/// Every variable a test definition uses.
pub fn used_variables(request: &SyntheticRequest, assertions: &[SyntheticAssertion]) -> Vec<String> {
    let mut r = request.clone();
    let mut texts: Vec<String> = request_texts(&mut r).into_iter().map(|t| t.clone()).collect();
    texts.extend(assertions.iter().map(|a| a.target.clone()));
    let mut out: Vec<String> = Vec::new();
    for t in texts {
        for name in placeholders(&t) {
            if !out.contains(&name) {
                out.push(name);
            }
        }
    }
    out
}

/// Hides secure values wherever a run could show them (URL, redirects, error, headers, body, assertions).
pub fn mask(run: &mut SyntheticRunDetail, secrets: &[String]) {
    let secrets: Vec<&String> = secrets.iter().filter(|s| !s.is_empty()).collect();
    if secrets.is_empty() {
        return;
    }
    let hide = |s: &mut String| {
        for secret in &secrets {
            if s.contains(secret.as_str()) {
                *s = s.replace(secret.as_str(), MASK);
            }
        }
    };
    for s in [run.url.as_mut(), run.error.as_mut(), run.body.as_mut()].into_iter().flatten() {
        hide(s);
    }
    run.redirects.iter_mut().for_each(hide);
    for h in &mut run.response_headers {
        hide(&mut h.value);
    }
    for a in &mut run.assertions {
        hide(&mut a.assertion.target);
        for s in [a.actual.as_mut(), a.error.as_mut()].into_iter().flatten() {
            hide(s);
        }
    }
}

/// Why a test can't be saved under the enforced rules.
pub fn enforcement_problem(settings: &SyntheticSettings, tags: &[String], env: &str) -> Option<String> {
    if settings.enforce_tag_keys {
        let mut keys: Vec<&str> = Vec::new();
        for t in &settings.defaults.tags {
            let key = t.split_once(':').map_or(t.as_str(), |(k, _)| k).trim();
            if !key.is_empty() && !keys.contains(&key) {
                keys.push(key);
            }
        }
        let has = |key: &str| tags.iter().any(|t| t.split_once(':').is_some_and(|(k, v)| k.trim() == key && !v.trim().is_empty()));
        let missing: Vec<&str> = keys.into_iter().filter(|k| !has(k)).collect();
        if !missing.is_empty() {
            return Some(format!("Tests need a tag for each of: {}", missing.join(", ")));
        }
    }
    if settings.enforce_env && env.trim().is_empty() {
        return Some("Tests need an environment (env)".into());
    }
    None
}

/// Checks and normalizes the settings.
pub fn check_settings(s: &mut SyntheticSettings) -> Result<(), String> {
    let d = &mut s.defaults;
    if d.locations.is_empty() {
        d.locations = vec![LOCATION_ID.to_string()];
    }
    d.locations.dedup();
    if let Some(unknown) = d.locations.iter().find(|l| l.as_str() != LOCATION_ID) {
        return Err(format!("Unknown location '{unknown}': tests run from the ObserveCraft server ({LOCATION_ID})"));
    }
    let mut tags: Vec<String> = Vec::new();
    for t in &d.tags {
        let t = t.trim().to_string();
        if !t.is_empty() && !tags.contains(&t) {
            tags.push(t);
        }
    }
    if tags.len() > MAX_DEFAULT_TAGS || tags.iter().any(|t| t.len() > 200 || t.contains([',', ' '])) {
        return Err(format!("Up to {MAX_DEFAULT_TAGS} default tags of at most 200 characters, without commas or spaces"));
    }
    d.tags = tags;
    if !(1..=MAX_TIMEOUT_SECONDS).contains(&d.timeout_seconds) {
        return Err(format!("The default timeout is 1–{MAX_TIMEOUT_SECONDS} seconds"));
    }
    if !FREQUENCIES.contains(&d.frequency_seconds) {
        return Err("Run every 1, 5, 15 or 30 minutes, 1, 6 or 12 hours, daily or weekly".into());
    }
    if !(0..=3).contains(&d.retry.count) || !(0..=60_000).contains(&d.retry.interval_ms) {
        return Err("Retry 0–3 times, 0–60000 ms apart".into());
    }
    if !(0..=120).contains(&d.min_failure_duration_minutes) {
        return Err("Alert after 0–120 minutes of failures".into());
    }
    Ok(())
}

// ================================================================ storage

#[derive(sqlx::FromRow)]
struct SettingsRow {
    enforce_tag_keys: bool,
    enforce_env: bool,
    defaults: Value,
}

/// The org's settings (the defaults until first saved).
pub async fn load_settings(st: &AppState, org: &str) -> Result<SyntheticSettings, ApiError> {
    let row: Option<SettingsRow> = sqlx::query_as("SELECT enforce_tag_keys, enforce_env, defaults FROM synthetic_settings WHERE org_id = $1").bind(org).fetch_optional(&st.postgres).await?;
    Ok(match row {
        Some(r) => SyntheticSettings { enforce_tag_keys: r.enforce_tag_keys, enforce_env: r.enforce_env, defaults: serde_json::from_value(r.defaults).unwrap_or_default() },
        None => SyntheticSettings::default(),
    })
}

/// The org's variables by name, and the secure values to mask.
pub async fn load_variables(st: &AppState, org: &str) -> Result<(HashMap<String, String>, Vec<String>), ApiError> {
    let rows: Vec<(String, String, bool)> = sqlx::query_as("SELECT name, value, secure FROM synthetic_variables WHERE org_id = $1").bind(org).fetch_all(&st.postgres).await?;
    let secrets = rows.iter().filter(|r| r.2).map(|r| r.1.clone()).collect();
    Ok((rows.into_iter().map(|(n, v, _)| (n, v)).collect(), secrets))
}

#[derive(sqlx::FromRow)]
struct VariableRow {
    id: String,
    name: String,
    value: String,
    secure: bool,
    description: String,
    tags: Vec<String>,
    created_by: Option<String>,
    updated_by: Option<String>,
    created_at: i64,
    updated_at: i64,
}

const VARIABLE_SELECT: &str = "SELECT v.id::text AS id, v.name, v.value, v.secure, v.description, v.tags, c.name AS created_by, u.name AS updated_by, \
     (extract(epoch FROM v.created_at) * 1000)::bigint AS created_at, (extract(epoch FROM v.updated_at) * 1000)::bigint AS updated_at \
     FROM synthetic_variables v LEFT JOIN users c ON c.id = v.created_by LEFT JOIN users u ON u.id = v.updated_by";

fn is_uuid(s: &str) -> bool {
    s.len() == 36 && s.bytes().enumerate().all(|(i, b)| if matches!(i, 8 | 13 | 18 | 23) { b == b'-' } else { b.is_ascii_hexdigit() })
}

fn variable_not_found() -> ApiError {
    ApiError::NotFound("Global variable not found".into())
}

/// Which tests use each variable.
async fn variable_uses(st: &AppState, org: &str) -> Result<HashMap<String, Vec<SyntheticVariableUse>>, ApiError> {
    let tests: Vec<(String, String, Value, Value)> =
        sqlx::query_as("SELECT id::text, name, request, assertions FROM synthetic_tests WHERE org_id = $1 ORDER BY lower(name)").bind(org).fetch_all(&st.postgres).await?;
    let mut out: HashMap<String, Vec<SyntheticVariableUse>> = HashMap::new();
    for (id, name, request, assertions) in tests {
        let request: SyntheticRequest = serde_json::from_value(request).unwrap_or_default();
        let assertions: Vec<SyntheticAssertion> = serde_json::from_value(assertions).unwrap_or_default();
        for v in used_variables(&request, &assertions) {
            out.entry(v).or_default().push(SyntheticVariableUse { id: id.clone(), name: name.clone() });
        }
    }
    Ok(out)
}

fn variable_to_api(r: VariableRow, uses: &HashMap<String, Vec<SyntheticVariableUse>>) -> SyntheticVariable {
    SyntheticVariable {
        used_by: uses.get(&r.name).cloned().unwrap_or_default(),
        value: (!r.secure).then_some(r.value),
        id: r.id,
        name: r.name,
        secure: r.secure,
        description: r.description,
        tags: r.tags,
        created_by: r.created_by,
        updated_by: r.updated_by,
        created_at: r.created_at,
        updated_at: r.updated_at,
    }
}

async fn load_variable(st: &AppState, org: &str, id: &str) -> Result<SyntheticVariable, ApiError> {
    let row: Option<VariableRow> = sqlx::query_as(&format!("{VARIABLE_SELECT} WHERE v.org_id = $1 AND v.id = $2::uuid")).bind(org).bind(id).fetch_optional(&st.postgres).await?;
    let row = row.ok_or_else(variable_not_found)?;
    Ok(variable_to_api(row, &variable_uses(st, org).await?))
}

fn check_variable(input: &mut SyntheticVariableInput, creating: bool) -> Result<(), String> {
    input.name = check_variable_name(&input.name)?;
    match &input.value {
        None if creating => return Err("The value is required".into()),
        Some(v) if v.chars().count() > MAX_VARIABLE_VALUE => return Err(format!("Values are at most {MAX_VARIABLE_VALUE} characters")),
        _ => {}
    }
    input.description = input.description.trim().to_string();
    if input.description.chars().count() > 1000 {
        return Err("The description is at most 1,000 characters".into());
    }
    let mut tags: Vec<String> = Vec::new();
    for t in &input.tags {
        let t = t.trim().to_string();
        if !t.is_empty() && !tags.contains(&t) {
            tags.push(t);
        }
    }
    if tags.len() > 20 || tags.iter().any(|t| t.len() > 200 || t.contains(',')) {
        return Err("Up to 20 tags of at most 200 characters, without commas".into());
    }
    input.tags = tags;
    Ok(())
}

fn duplicate(e: sqlx::Error, name: &str) -> ApiError {
    match &e {
        sqlx::Error::Database(d) if d.is_unique_violation() => ApiError::BadRequest(format!("A global variable named {name} exists")),
        _ => e.into(),
    }
}

// ================================================================ handlers

#[utoipa::path(get, path = "/api/v1/synthetics/settings", operation_id = "get_synthetic_settings", responses((status = 200, body = SyntheticSettings)), tag = "synthetics")]
pub async fn get_settings(State(st): State<AppState>, ctx: AuthContext) -> ApiResult<SyntheticSettings> {
    Ok(Json(load_settings(&st, &ctx.org_id).await?))
}

/// Saves the enforced tag rules and the test editor's defaults.
#[utoipa::path(put, path = "/api/v1/synthetics/settings", operation_id = "update_synthetic_settings", request_body = SyntheticSettings,
    responses((status = 200, body = SyntheticSettings), (status = 400, body = crate::error::ErrorBody)), tag = "synthetics")]
pub async fn update_settings(State(st): State<AppState>, Writer(ctx): Writer, Json(mut input): Json<SyntheticSettings>) -> ApiResult<SyntheticSettings> {
    check_settings(&mut input).map_err(ApiError::BadRequest)?;
    let defaults = serde_json::to_value(&input.defaults).unwrap_or_default();
    sqlx::query(
        "INSERT INTO synthetic_settings (org_id, enforce_tag_keys, enforce_env, defaults, updated_by, updated_at) VALUES ($1, $2, $3, $4, $5::uuid, now()) \
         ON CONFLICT (org_id) DO UPDATE SET enforce_tag_keys = EXCLUDED.enforce_tag_keys, enforce_env = EXCLUDED.enforce_env, defaults = EXCLUDED.defaults, \
         updated_by = EXCLUDED.updated_by, updated_at = now()",
    )
    .bind(&ctx.org_id)
    .bind(input.enforce_tag_keys)
    .bind(input.enforce_env)
    .bind(&defaults)
    .bind(&ctx.user_id)
    .execute(&st.postgres)
    .await?;
    ctx.audit(&st, "synthetics.settings_updated", "synthetics", "default settings").await;
    Ok(Json(load_settings(&st, &ctx.org_id).await?))
}

#[utoipa::path(get, path = "/api/v1/synthetics/variables", operation_id = "list_synthetic_variables", responses((status = 200, body = Vec<SyntheticVariable>)), tag = "synthetics")]
pub async fn list_variables(State(st): State<AppState>, ctx: AuthContext) -> ApiResult<Vec<SyntheticVariable>> {
    let rows: Vec<VariableRow> = sqlx::query_as(&format!("{VARIABLE_SELECT} WHERE v.org_id = $1 ORDER BY v.name")).bind(&ctx.org_id).fetch_all(&st.postgres).await?;
    let uses = variable_uses(&st, &ctx.org_id).await?;
    Ok(Json(rows.into_iter().map(|r| variable_to_api(r, &uses)).collect()))
}

#[utoipa::path(post, path = "/api/v1/synthetics/variables", operation_id = "create_synthetic_variable", request_body = SyntheticVariableInput,
    responses((status = 200, body = SyntheticVariable), (status = 400, body = crate::error::ErrorBody)), tag = "synthetics")]
pub async fn create_variable(State(st): State<AppState>, Writer(ctx): Writer, Json(mut input): Json<SyntheticVariableInput>) -> ApiResult<SyntheticVariable> {
    check_variable(&mut input, true).map_err(ApiError::BadRequest)?;
    let (count,): (i64,) = sqlx::query_as("SELECT count(*) FROM synthetic_variables WHERE org_id = $1").bind(&ctx.org_id).fetch_one(&st.postgres).await?;
    if count >= MAX_VARIABLES {
        return Err(ApiError::BadRequest(format!("An organization can have at most {MAX_VARIABLES} global variables")));
    }
    let (id,): (String,) = sqlx::query_as(
        "INSERT INTO synthetic_variables (org_id, name, value, secure, description, tags, created_by, updated_by) VALUES ($1, $2, $3, $4, $5, $6, $7::uuid, $7::uuid) RETURNING id::text",
    )
    .bind(&ctx.org_id)
    .bind(&input.name)
    .bind(input.value.as_deref().unwrap_or_default())
    .bind(input.secure)
    .bind(&input.description)
    .bind(&input.tags)
    .bind(&ctx.user_id)
    .fetch_one(&st.postgres)
    .await
    .map_err(|e| duplicate(e, &input.name))?;
    ctx.audit(&st, "synthetics.variable_created", &id, &input.name).await;
    Ok(Json(load_variable(&st, &ctx.org_id, &id).await?))
}

/// Updates a variable; without a value the stored one is kept.
#[utoipa::path(put, path = "/api/v1/synthetics/variables/{id}", operation_id = "update_synthetic_variable", params(("id" = String, Path)), request_body = SyntheticVariableInput,
    responses((status = 200, body = SyntheticVariable), (status = 400, body = crate::error::ErrorBody), (status = 404, body = crate::error::ErrorBody)), tag = "synthetics")]
pub async fn update_variable(State(st): State<AppState>, Writer(ctx): Writer, Path(id): Path<String>, Json(mut input): Json<SyntheticVariableInput>) -> ApiResult<SyntheticVariable> {
    check_variable(&mut input, false).map_err(ApiError::BadRequest)?;
    if !is_uuid(&id) {
        return Err(variable_not_found());
    }
    let updated = sqlx::query(
        "UPDATE synthetic_variables SET name = $3, value = COALESCE($4, value), secure = $5, description = $6, tags = $7, updated_by = $8::uuid, updated_at = now() \
         WHERE org_id = $1 AND id = $2::uuid",
    )
    .bind(&ctx.org_id)
    .bind(&id)
    .bind(&input.name)
    .bind(&input.value)
    .bind(input.secure)
    .bind(&input.description)
    .bind(&input.tags)
    .bind(&ctx.user_id)
    .execute(&st.postgres)
    .await
    .map_err(|e| duplicate(e, &input.name))?;
    if updated.rows_affected() == 0 {
        return Err(variable_not_found());
    }
    ctx.audit(&st, "synthetics.variable_updated", &id, &input.name).await;
    Ok(Json(load_variable(&st, &ctx.org_id, &id).await?))
}

#[utoipa::path(delete, path = "/api/v1/synthetics/variables/{id}", operation_id = "delete_synthetic_variable", params(("id" = String, Path)),
    responses((status = 204), (status = 404, body = crate::error::ErrorBody)), tag = "synthetics")]
pub async fn delete_variable(State(st): State<AppState>, Writer(ctx): Writer, Path(id): Path<String>) -> Result<StatusCode, ApiError> {
    if !is_uuid(&id) {
        return Err(variable_not_found());
    }
    let row: Option<(String,)> = sqlx::query_as("DELETE FROM synthetic_variables WHERE org_id = $1 AND id = $2::uuid RETURNING name").bind(&ctx.org_id).bind(&id).fetch_optional(&st.postgres).await?;
    let Some((name,)) = row else { return Err(variable_not_found()) };
    ctx.audit(&st, "synthetics.variable_deleted", &id, &name).await;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::routes::synthetics_exec::{SyntheticAssertionResult, SyntheticAssertionType, SyntheticBasicAuth, SyntheticHeader, SyntheticOperator};

    fn vars(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    #[test]
    fn variable_names_are_upper_snake_case() {
        assert_eq!(check_variable_name(" API_TOKEN "), Ok("API_TOKEN".to_string()));
        assert_eq!(check_variable_name("BASE_URL2"), Ok("BASE_URL2".to_string()));
        for bad in ["", "api_token", "2FA", "API-TOKEN", "API TOKEN", "_X"] {
            assert!(check_variable_name(bad).is_err(), "{bad}");
        }
        assert!(check_variable_name(&"A".repeat(81)).is_err());
    }

    #[test]
    fn placeholders_are_found_with_or_without_spaces() {
        assert_eq!(placeholders("https://{{ HOST }}/a?k={{KEY}}&again={{ HOST }}"), vec!["HOST".to_string(), "KEY".to_string()]);
        assert_eq!(placeholders("no vars {{ lower }} {{}} {{ A B }}"), Vec::<String>::new());
    }

    #[test]
    fn requests_and_assertions_get_the_values() {
        let request = SyntheticRequest {
            url: "https://{{ HOST }}/checkout?token={{TOKEN}}".into(),
            headers: vec![SyntheticHeader { name: "Authorization".into(), value: "Bearer {{ TOKEN }}".into() }],
            body: "{\"user\":\"{{ USER }}\"}".into(),
            basic_auth: Some(SyntheticBasicAuth { username: "{{ USER }}".into(), password: "{{ PASSWORD }}".into() }),
            ..Default::default()
        };
        let assertions = vec![SyntheticAssertion { kind: SyntheticAssertionType::Body, operator: SyntheticOperator::Contains, property: String::new(), target: "{{ USER }}".into() }];
        let v = vars(&[("HOST", "shop.example.com"), ("TOKEN", "s3cr3t"), ("USER", "alex"), ("PASSWORD", "pw")]);
        let (r, a) = substitute(&request, &assertions, &v).unwrap();
        assert_eq!(r.url, "https://shop.example.com/checkout?token=s3cr3t");
        assert_eq!(r.headers[0].value, "Bearer s3cr3t");
        assert_eq!(r.body, "{\"user\":\"alex\"}");
        assert_eq!(r.basic_auth.unwrap().password, "pw");
        assert_eq!(a[0].target, "alex");
        let err = substitute(&request, &assertions, &vars(&[("HOST", "x")])).unwrap_err();
        assert_eq!(err, "Unknown global variables: TOKEN, USER, PASSWORD");
        assert_eq!(used_variables(&request, &assertions), vec!["HOST", "TOKEN", "USER", "PASSWORD"]);
    }

    #[test]
    fn secure_values_are_masked_in_results() {
        let mut run = SyntheticRunDetail {
            url: Some("https://shop.example.com/?token=s3cr3t".into()),
            error: Some("Timed out calling https://shop.example.com/?token=s3cr3t".into()),
            redirects: vec!["https://shop.example.com/login?token=s3cr3t".into()],
            response_headers: vec![SyntheticHeader { name: "x-echo".into(), value: "s3cr3t".into() }],
            body: Some("hello s3cr3t".into()),
            assertions: vec![SyntheticAssertionResult {
                assertion: SyntheticAssertion { kind: SyntheticAssertionType::Body, operator: SyntheticOperator::Contains, property: String::new(), target: "s3cr3t".into() },
                passed: true,
                actual: Some("hello s3cr3t".into()),
                error: None,
            }],
            ..Default::default()
        };
        mask(&mut run, &["s3cr3t".into(), String::new()]);
        let text = serde_json::to_string(&run).unwrap();
        assert!(!text.contains("s3cr3t"), "{text}");
        assert_eq!(run.url.as_deref(), Some("https://shop.example.com/?token=••••••"));
    }

    #[test]
    fn enforced_rules_need_the_default_tag_keys_and_an_env() {
        let mut s = SyntheticSettings { defaults: SyntheticDefaults { tags: vec!["team:".into(), "service:storefront-api".into()], ..Default::default() }, ..Default::default() };
        let tags = |t: &[&str]| t.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        assert_eq!(enforcement_problem(&s, &[], ""), None, "nothing enforced");
        s.enforce_tag_keys = true;
        assert_eq!(enforcement_problem(&s, &tags(&["team:checkout"]), ""), Some("Tests need a tag for each of: service".to_string()));
        assert_eq!(enforcement_problem(&s, &tags(&["team:checkout", "service:billing"]), ""), None);
        assert_eq!(enforcement_problem(&s, &tags(&["team:"]), "").as_deref(), Some("Tests need a tag for each of: team, service"), "an empty value doesn't count");
        s.enforce_env = true;
        assert_eq!(enforcement_problem(&s, &tags(&["team:a", "service:b"]), " "), Some("Tests need an environment (env)".to_string()));
        assert_eq!(enforcement_problem(&s, &tags(&["team:a", "service:b"]), "prod"), None);
    }

    #[test]
    fn settings_are_checked() {
        let mut ok = SyntheticSettings {
            defaults: SyntheticDefaults { tags: vec![" team:checkout ".into(), "".into(), "team:checkout".into()], locations: vec![], ..Default::default() },
            ..Default::default()
        };
        assert_eq!(check_settings(&mut ok), Ok(()));
        assert_eq!(ok.defaults.tags, vec!["team:checkout".to_string()]);
        assert_eq!(ok.defaults.locations, vec![LOCATION_ID.to_string()]);
        let bad = |f: fn(&mut SyntheticDefaults)| {
            let mut s = SyntheticSettings::default();
            f(&mut s.defaults);
            check_settings(&mut s).unwrap_err()
        };
        assert!(bad(|d| d.timeout_seconds = 0).contains("timeout"));
        assert!(bad(|d| d.frequency_seconds = 30).contains("Run every"));
        assert!(bad(|d| d.retry.count = 4).contains("Retry"));
        assert!(bad(|d| d.locations = vec!["aws:eu-west-1".into()]).contains("Unknown location"));
        assert!(bad(|d| d.min_failure_duration_minutes = -1).contains("minutes"));
    }
}
