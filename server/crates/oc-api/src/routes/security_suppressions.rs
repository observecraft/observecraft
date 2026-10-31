//! Cloud SIEM suppressions (Security → Cloud SIEM → Settings → Suppressions; Terraform
//! `datadog_security_monitoring_suppression`): silence the signals of some rules for some groups, or
//! leave some logs out of the rules' evaluation, optionally between a start and an expiration date.
//!
//! - `rule_query` picks the rules, matched against a rule's attributes: `type:`, `ruleId:`, `source:`,
//!   its tags (`tactic:…`), and free text in its name. Empty: every rule.
//! - `suppression_query` matches a would-be signal's group values (`@usr.email:…`, `service:…`),
//!   its severity (`status:low`) and the rule's tags; a match drops the signal.
//! - `data_exclusion_query` (one term) is excluded from the counts of the rules it applies to.

use std::collections::BTreeMap;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use oc_query::{parse_query, Term};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use super::security_rules::{new_id, SecurityRule};
use crate::auth::{AuthContext, Writer};
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;

#[derive(Serialize, ToSchema, Clone, Debug, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct Suppression {
    pub id: String,
    pub version: i32,
    pub name: String,
    pub description: String,
    pub enabled: bool,
    pub rule_query: String,
    pub suppression_query: String,
    pub data_exclusion_query: String,
    /// Epoch ms.
    pub start_date: Option<i64>,
    pub expiration_date: Option<i64>,
    pub tags: Vec<String>,
    pub creator: String,
    pub updater: String,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Deserialize, ToSchema, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct SuppressionInput {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default = "yes")]
    pub enabled: bool,
    #[serde(default)]
    pub rule_query: String,
    #[serde(default)]
    pub suppression_query: String,
    #[serde(default)]
    pub data_exclusion_query: String,
    pub start_date: Option<i64>,
    pub expiration_date: Option<i64>,
    #[serde(default)]
    pub tags: Vec<String>,
}

fn yes() -> bool {
    true
}

impl SuppressionInput {
    pub fn validate(&self) -> Result<(), String> {
        if self.name.trim().is_empty() {
            return Err("the suppression needs a name".into());
        }
        if self.suppression_query.trim().is_empty() && self.data_exclusion_query.trim().is_empty() {
            return Err("set a suppression query, a data exclusion query, or both".into());
        }
        if let (Some(s), Some(e)) = (self.start_date, self.expiration_date) {
            if e <= s {
                return Err("the expiration date must be after the start date".into());
            }
        }
        Ok(())
    }
}

/// `*` wildcards, case-insensitive.
fn glob(pattern: &str, value: &str) -> bool {
    let (p, v) = (pattern.to_lowercase(), value.to_lowercase());
    let parts: Vec<&str> = p.split('*').collect();
    if parts.len() == 1 {
        return p == v;
    }
    let mut rest = v.as_str();
    for (i, part) in parts.iter().enumerate() {
        if part.is_empty() {
            continue;
        }
        match rest.find(part) {
            Some(pos) if i > 0 || pos == 0 => rest = &rest[pos + part.len()..],
            _ => return false,
        }
    }
    parts.last().is_some_and(|l| l.is_empty()) || rest.is_empty()
}

/// Whether every term matches `attrs` (keys with or without `@`) or, for free text, `text`.
fn terms_match(terms: &[Term], attrs: &BTreeMap<String, Vec<String>>, text: &str) -> bool {
    terms.iter().all(|t| {
        let (hit, negate) = match t {
            Term::Tag { key, values, negate } => {
                let k = key.trim_start_matches('@');
                let got = attrs.get(key).or_else(|| attrs.get(k)).or_else(|| attrs.get(&format!("@{k}")));
                (got.is_some_and(|got| values.iter().any(|p| got.iter().any(|v| glob(p, v)))), *negate)
            }
            Term::Text { text: t, negate } => (text.to_lowercase().contains(&t.to_lowercase()), *negate),
        };
        hit != negate
    })
}

fn rule_attrs(rule: &SecurityRule) -> BTreeMap<String, Vec<String>> {
    let mut m: BTreeMap<String, Vec<String>> = BTreeMap::new();
    m.entry("type".into()).or_default().push(rule.kind.clone());
    for k in ["ruleId", "rule_id", "id"] {
        m.entry(k.into()).or_default().push(rule.id.clone());
    }
    for t in &rule.tags {
        if let Some((k, v)) = t.split_once(':') {
            m.entry(k.to_string()).or_default().push(v.to_string());
        }
    }
    m
}

impl Suppression {
    pub fn applies_to(&self, rule: &SecurityRule) -> bool {
        terms_match(&parse_query(&self.rule_query), &rule_attrs(rule), &rule.name)
    }

    /// Whether a signal of `rule` for `group` at `severity` is dropped.
    pub fn suppresses(&self, rule: &SecurityRule, group: &BTreeMap<String, String>, severity: &str) -> bool {
        if self.suppression_query.trim().is_empty() {
            return false;
        }
        let mut attrs = rule_attrs(rule);
        for (k, v) in group {
            attrs.entry(k.clone()).or_default().push(v.clone());
        }
        attrs.entry("status".into()).or_default().push(severity.to_string());
        attrs.entry("severity".into()).or_default().push(severity.to_string());
        terms_match(&parse_query(&self.suppression_query), &attrs, "")
    }
}

const SELECT: &str = "SELECT id, version, name, description, enabled, rule_query, suppression_query, data_exclusion_query, start_date, expiration_date, tags, creator, updater, \
    (extract(epoch FROM created_at) * 1000)::bigint AS created_at, (extract(epoch FROM updated_at) * 1000)::bigint AS updated_at FROM security_suppressions";

pub async fn list_rows(st: &AppState, org: &str) -> Result<Vec<Suppression>, ApiError> {
    Ok(sqlx::query_as(&format!("{SELECT} WHERE org_id = $1 ORDER BY name")).bind(org).fetch_all(&st.postgres).await?)
}

pub async fn load(st: &AppState, org: &str, id: &str) -> Result<Suppression, ApiError> {
    sqlx::query_as(&format!("{SELECT} WHERE org_id = $1 AND id = $2"))
        .bind(org)
        .bind(id)
        .fetch_optional(&st.postgres)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("no suppression {id}")))
}

/// The enabled suppressions in effect at `now`.
pub async fn active_suppressions(st: &AppState, org: &str, now: i64) -> Result<Vec<Suppression>, ApiError> {
    Ok(sqlx::query_as(&format!("{SELECT} WHERE org_id = $1 AND enabled AND (start_date IS NULL OR start_date <= $2) AND (expiration_date IS NULL OR expiration_date > $2)"))
        .bind(org)
        .bind(now)
        .fetch_all(&st.postgres)
        .await?)
}

pub async fn insert(st: &AppState, ctx: &AuthContext, input: &SuppressionInput) -> Result<String, ApiError> {
    input.validate().map_err(ApiError::BadRequest)?;
    let exists: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM security_suppressions WHERE org_id = $1 AND name = $2)").bind(&ctx.org_id).bind(input.name.trim()).fetch_one(&st.postgres).await?;
    if exists {
        return Err(ApiError::Conflict(format!("a suppression named {} already exists", input.name.trim())));
    }
    let id = new_id();
    sqlx::query(
        "INSERT INTO security_suppressions (org_id, id, name, description, enabled, rule_query, suppression_query, data_exclusion_query, start_date, expiration_date, tags, creator, updater) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $12)",
    )
    .bind(&ctx.org_id)
    .bind(&id)
    .bind(input.name.trim())
    .bind(&input.description)
    .bind(input.enabled)
    .bind(&input.rule_query)
    .bind(&input.suppression_query)
    .bind(&input.data_exclusion_query)
    .bind(input.start_date)
    .bind(input.expiration_date)
    .bind(&input.tags)
    .bind(&ctx.name)
    .execute(&st.postgres)
    .await?;
    Ok(id)
}

pub async fn update_row(st: &AppState, ctx: &AuthContext, id: &str, input: &SuppressionInput) -> Result<(), ApiError> {
    load(st, &ctx.org_id, id).await?;
    input.validate().map_err(ApiError::BadRequest)?;
    sqlx::query(
        "UPDATE security_suppressions SET name = $3, description = $4, enabled = $5, rule_query = $6, suppression_query = $7, data_exclusion_query = $8, start_date = $9, \
         expiration_date = $10, tags = $11, updater = $12, updated_at = now(), version = version + 1 WHERE org_id = $1 AND id = $2",
    )
    .bind(&ctx.org_id)
    .bind(id)
    .bind(input.name.trim())
    .bind(&input.description)
    .bind(input.enabled)
    .bind(&input.rule_query)
    .bind(&input.suppression_query)
    .bind(&input.data_exclusion_query)
    .bind(input.start_date)
    .bind(input.expiration_date)
    .bind(&input.tags)
    .bind(&ctx.name)
    .execute(&st.postgres)
    .await?;
    Ok(())
}

pub async fn delete_row(st: &AppState, org: &str, id: &str) -> Result<(), ApiError> {
    let n = sqlx::query("DELETE FROM security_suppressions WHERE org_id = $1 AND id = $2").bind(org).bind(id).execute(&st.postgres).await?.rows_affected();
    if n == 0 {
        return Err(ApiError::NotFound(format!("no suppression {id}")));
    }
    Ok(())
}

#[utoipa::path(get, path = "/api/v1/security/suppressions", operation_id = "list_security_suppressions", responses((status = 200, body = Vec<Suppression>)), tag = "security")]
pub async fn list(State(st): State<AppState>, ctx: AuthContext) -> ApiResult<Vec<Suppression>> {
    Ok(Json(list_rows(&st, &ctx.org_id).await?))
}

#[utoipa::path(post, path = "/api/v1/security/suppressions", operation_id = "create_security_suppression", request_body = SuppressionInput,
    responses((status = 200, body = Suppression), (status = 400, body = crate::error::ErrorBody), (status = 409, body = crate::error::ErrorBody)), tag = "security")]
pub async fn create(State(st): State<AppState>, Writer(ctx): Writer, Json(input): Json<SuppressionInput>) -> ApiResult<Suppression> {
    let id = insert(&st, &ctx, &input).await?;
    Ok(Json(load(&st, &ctx.org_id, &id).await?))
}

#[utoipa::path(put, path = "/api/v1/security/suppressions/{id}", operation_id = "update_security_suppression", params(("id" = String, Path)), request_body = SuppressionInput,
    responses((status = 200, body = Suppression), (status = 400, body = crate::error::ErrorBody), (status = 404, body = crate::error::ErrorBody)), tag = "security")]
pub async fn update(State(st): State<AppState>, Writer(ctx): Writer, Path(id): Path<String>, Json(input): Json<SuppressionInput>) -> ApiResult<Suppression> {
    update_row(&st, &ctx, &id, &input).await?;
    Ok(Json(load(&st, &ctx.org_id, &id).await?))
}

#[utoipa::path(delete, path = "/api/v1/security/suppressions/{id}", operation_id = "delete_security_suppression", params(("id" = String, Path)),
    responses((status = 204), (status = 404, body = crate::error::ErrorBody)), tag = "security")]
pub async fn delete(State(st): State<AppState>, Writer(ctx): Writer, Path(id): Path<String>) -> Result<StatusCode, ApiError> {
    delete_row(&st, &ctx.org_id, &id).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn suppression(rule_query: &str, suppression_query: &str) -> Suppression {
        Suppression {
            id: "s".into(),
            version: 1,
            name: "s".into(),
            description: String::new(),
            enabled: true,
            rule_query: rule_query.into(),
            suppression_query: suppression_query.into(),
            data_exclusion_query: String::new(),
            start_date: None,
            expiration_date: None,
            tags: vec![],
            creator: String::new(),
            updater: String::new(),
            created_at: 0,
            updated_at: 0,
        }
    }

    fn rule() -> SecurityRule {
        let mut r = crate::routes::security_rules::test_rule();
        r.tags = vec!["source:cloudtrail".into(), "tactic:TA0006-credential-access".into()];
        r.name = "Brute force on AWS console sign-ins".into();
        r
    }

    #[test]
    fn globs_match_wildcards() {
        assert!(glob("10.0.*", "10.0.4.8"));
        assert!(glob("*admin*", "role/Admin-Ops"));
        assert!(!glob("10.0.*", "192.0.2.1"));
        assert!(glob("prod", "PROD"));
    }

    #[test]
    fn rule_and_suppression_queries_match() {
        let r = rule();
        assert!(suppression("source:cloudtrail", "env:staging").applies_to(&r));
        assert!(suppression("", "env:staging").applies_to(&r));
        assert!(suppression("brute", "env:staging").applies_to(&r));
        assert!(!suppression("source:okta", "env:staging").applies_to(&r));
        let g: BTreeMap<String, String> = [("@userIdentity.arn".to_string(), "arn:aws:iam::123456789012:user/ci-deployer".to_string())].into();
        assert!(suppression("", "@userIdentity.arn:*ci-deployer").suppresses(&r, &g, "medium"));
        assert!(suppression("", "status:medium").suppresses(&r, &g, "medium"));
        assert!(!suppression("", "status:high").suppresses(&r, &g, "medium"));
        assert!(!suppression("", "").suppresses(&r, &g, "medium"));
    }
}
