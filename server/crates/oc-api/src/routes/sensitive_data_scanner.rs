//! Sensitive Data Scanner (Organization Settings → Sensitive Data Scanner): the org's scanning groups and rules,
//! the standard pattern library and a pattern tester. The same configuration as Datadog's API
//! (`datadog_api::sensitive_data_scanner`), stored with Datadog's field names ([`crate::sds::config`]); this API
//! speaks the UI's camelCase. Anyone reads it; admins change it. Every change is audited and reaches the log
//! indexers within 10 s. Matches are counted in the `sds.matches` metric, which the Findings page charts.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use utoipa::ToSchema;

use crate::auth::{Admin, AuthContext};
use crate::error::{ApiError, ApiResult};
use crate::sds::config::{self, Config, GroupRow, RuleRow};
use crate::sds::{patterns, Scanner, Target};
use crate::state::AppState;

const MAX_SAMPLE_BYTES: usize = 64 * 1024;

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SdsConfiguration {
    /// Moves on every change (Datadog's `meta.version`).
    pub version: i64,
    /// In scanning order.
    pub groups: Vec<SdsGroup>,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SdsGroup {
    pub id: String,
    pub name: String,
    pub description: String,
    /// Log search syntax; `*` or empty scans every log.
    pub query: String,
    pub enabled: bool,
    /// `logs`, `rum`, `events`, `apm`: only logs are scanned.
    pub products: Vec<String>,
    /// Percent of the matching logs scanned.
    pub sampling_rate: f64,
    /// In scanning order.
    pub rules: Vec<SdsRule>,
}

#[derive(Serialize, Deserialize, ToSchema, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct SdsReplacement {
    /// `none`, `hash`, `replacement_string`, `partial_replacement_from_beginning`, `partial_replacement_from_end`.
    pub kind: String,
    #[serde(default)]
    pub replacement_string: Option<String>,
    #[serde(default)]
    pub number_of_chars: Option<i64>,
}

#[derive(Serialize, Deserialize, ToSchema, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct SdsSuppressions {
    #[serde(default)]
    pub starts_with: Vec<String>,
    #[serde(default)]
    pub ends_with: Vec<String>,
    #[serde(default)]
    pub exact_match: Vec<String>,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SdsRule {
    pub id: String,
    pub group_id: String,
    pub name: String,
    pub description: String,
    /// The rule's own regular expression; None with a standard pattern.
    pub pattern: Option<String>,
    pub standard_pattern_id: Option<String>,
    /// Attribute paths scanned (`message` for the message); empty: the whole log.
    pub namespaces: Vec<String>,
    pub excluded_namespaces: Vec<String>,
    /// `key:value` tags added to matching logs.
    pub tags: Vec<String>,
    pub enabled: bool,
    /// 1 (highest) to 5.
    pub priority: Option<i64>,
    pub replacement: SdsReplacement,
    /// A match counts only with one of these within `keywordCharacters` before it.
    pub keywords: Vec<String>,
    pub keyword_characters: Option<i64>,
    pub use_recommended_keywords: bool,
    pub suppressions: SdsSuppressions,
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SdsGroupInput {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub query: String,
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub products: Vec<String>,
    /// Percent of the matching logs scanned (default 100).
    #[serde(default)]
    pub sampling_rate: Option<f64>,
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SdsRuleInput {
    pub group_id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub pattern: Option<String>,
    #[serde(default)]
    pub standard_pattern_id: Option<String>,
    #[serde(default)]
    pub namespaces: Vec<String>,
    #[serde(default)]
    pub excluded_namespaces: Vec<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub priority: Option<i64>,
    #[serde(default)]
    pub replacement: SdsReplacement,
    #[serde(default)]
    pub keywords: Vec<String>,
    #[serde(default)]
    pub keyword_characters: Option<i64>,
    #[serde(default)]
    pub use_recommended_keywords: bool,
    #[serde(default)]
    pub suppressions: SdsSuppressions,
}

#[derive(Deserialize, ToSchema)]
pub struct SdsOrderInput {
    /// Every group (or every rule of the group) once, in the new order.
    pub ids: Vec<String>,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SdsStandardPattern {
    pub id: String,
    pub name: String,
    pub description: String,
    pub pattern: String,
    /// `credentials`, `finance`, `pii` or `network`.
    pub category: String,
    pub tags: Vec<String>,
    /// Recommended keywords.
    pub keywords: Vec<String>,
    pub priority: i64,
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SdsTestRequest {
    /// The rule to try (its group, name and `enabled` don't matter).
    pub rule: SdsRuleInput,
    /// Sample text, scanned as a log message.
    pub sample: String,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SdsTestResult {
    pub matches: u64,
    /// The sample as the rule leaves it.
    pub redacted: String,
    /// Tags a matching log gets.
    pub tags: Vec<String>,
}

// ================================================================ mapping

fn str_of(v: &Value, key: &str) -> String {
    v.get(key).and_then(Value::as_str).unwrap_or("").to_string()
}

fn strings(v: Option<&Value>) -> Vec<String> {
    v.and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect()).unwrap_or_default()
}

fn rule_out(r: &RuleRow) -> SdsRule {
    let a = &r.attributes;
    let t = a.get("text_replacement");
    let k = a.get("included_keyword_configuration");
    let s = a.get("suppressions");
    SdsRule {
        id: r.id.clone(),
        group_id: r.group_id.clone(),
        name: str_of(a, "name"),
        description: str_of(a, "description"),
        pattern: a.get("pattern").and_then(Value::as_str).map(str::to_string),
        standard_pattern_id: r.standard_pattern_id.clone(),
        namespaces: strings(a.get("namespaces")),
        excluded_namespaces: strings(a.get("excluded_namespaces")),
        tags: strings(a.get("tags")),
        enabled: a.get("is_enabled").and_then(Value::as_bool).unwrap_or(false),
        priority: a.get("priority").and_then(Value::as_i64),
        replacement: SdsReplacement {
            kind: t.and_then(|t| t.get("type")).and_then(Value::as_str).unwrap_or("none").to_string(),
            replacement_string: t.and_then(|t| t.get("replacement_string")).and_then(Value::as_str).map(str::to_string),
            number_of_chars: t.and_then(|t| t.get("number_of_chars")).and_then(Value::as_i64),
        },
        keywords: strings(k.and_then(|k| k.get("keywords"))),
        keyword_characters: k.and_then(|k| k.get("character_count")).and_then(Value::as_i64),
        use_recommended_keywords: k.and_then(|k| k.get("use_recommended_keywords")).and_then(Value::as_bool).unwrap_or(false),
        suppressions: SdsSuppressions {
            starts_with: strings(s.and_then(|s| s.get("starts_with"))),
            ends_with: strings(s.and_then(|s| s.get("ends_with"))),
            exact_match: strings(s.and_then(|s| s.get("exact_match"))),
        },
    }
}

fn group_out(g: &GroupRow, cfg: &Config) -> SdsGroup {
    let a = &g.attributes;
    let rate = a
        .get("samplings")
        .and_then(Value::as_array)
        .and_then(|s| s.iter().find(|s| s.get("product").and_then(Value::as_str) == Some("logs")))
        .and_then(|s| s.get("rate").and_then(Value::as_f64))
        .unwrap_or(100.0);
    SdsGroup {
        id: g.id.clone(),
        name: str_of(a, "name"),
        description: str_of(a, "description"),
        query: a.get("filter").map(|f| str_of(f, "query")).unwrap_or_default(),
        enabled: a.get("is_enabled").and_then(Value::as_bool).unwrap_or(false),
        products: strings(a.get("product_list")),
        sampling_rate: rate,
        rules: cfg.rules_of(&g.id).map(rule_out).collect(),
    }
}

fn group_in(i: &SdsGroupInput) -> Result<Value, ApiError> {
    let samplings = match i.sampling_rate {
        Some(rate) if rate < 100.0 => json!([{ "product": "logs", "rate": rate }]),
        _ => json!([]),
    };
    let attrs = json!({ "name": i.name, "description": i.description, "filter": { "query": i.query.trim() }, "is_enabled": i.enabled, "product_list": i.products, "samplings": samplings });
    config::group_attributes(&attrs, None).map_err(ApiError::BadRequest)
}

fn standard_of(i: &SdsRuleInput) -> Option<&str> {
    i.standard_pattern_id.as_deref().map(str::trim).filter(|s| !s.is_empty())
}

fn rule_in(i: &SdsRuleInput) -> Result<Value, ApiError> {
    let mut t = json!({ "type": if i.replacement.kind.is_empty() { "none" } else { i.replacement.kind.as_str() } });
    if let Some(s) = &i.replacement.replacement_string {
        t["replacement_string"] = json!(s);
    }
    if let Some(n) = i.replacement.number_of_chars {
        t["number_of_chars"] = json!(n);
    }
    let mut attrs = json!({
        "name": i.name,
        "description": i.description,
        "namespaces": i.namespaces,
        "excluded_namespaces": i.excluded_namespaces,
        "tags": i.tags,
        "is_enabled": i.enabled,
        "text_replacement": t,
    });
    if let Some(p) = &i.pattern {
        attrs["pattern"] = json!(p);
    }
    if let Some(p) = i.priority {
        attrs["priority"] = json!(p);
    }
    if !i.keywords.is_empty() || i.use_recommended_keywords {
        let mut k = json!({ "keywords": i.keywords, "use_recommended_keywords": i.use_recommended_keywords });
        if let Some(n) = i.keyword_characters {
            k["character_count"] = json!(n);
        }
        attrs["included_keyword_configuration"] = k;
    }
    let s = &i.suppressions;
    if !s.starts_with.is_empty() || !s.ends_with.is_empty() || !s.exact_match.is_empty() {
        attrs["suppressions"] = json!({ "starts_with": s.starts_with, "ends_with": s.ends_with, "exact_match": s.exact_match });
    }
    config::rule_attributes(&attrs, None, standard_of(i)).map_err(ApiError::BadRequest)
}

fn standard_out(p: &patterns::StandardPattern) -> SdsStandardPattern {
    SdsStandardPattern {
        id: p.id.into(),
        name: p.name.into(),
        description: p.description.into(),
        pattern: p.pattern.into(),
        category: p.category.into(),
        tags: p.tags(),
        keywords: p.keywords.iter().map(|k| k.to_string()).collect(),
        priority: p.priority,
    }
}

async fn group_reply(st: &AppState, org: &str, id: &str) -> ApiResult<SdsGroup> {
    let cfg = config::load(&st.postgres, org).await?;
    let g = cfg.groups.iter().find(|g| g.id == id).ok_or_else(|| ApiError::NotFound("Scanning group not found".into()))?;
    Ok(Json(group_out(g, &cfg)))
}

// ================================================================ routes

#[utoipa::path(get, path = "/api/v1/sensitive-data-scanner", operation_id = "get_sds_configuration", responses((status = 200, body = SdsConfiguration)), tag = "sensitive-data-scanner")]
pub async fn get_configuration(State(st): State<AppState>, ctx: AuthContext) -> ApiResult<SdsConfiguration> {
    let cfg = config::load(&st.postgres, &ctx.org_id).await?;
    Ok(Json(SdsConfiguration { version: cfg.version, groups: cfg.groups.iter().map(|g| group_out(g, &cfg)).collect() }))
}

#[utoipa::path(get, path = "/api/v1/sensitive-data-scanner/standard-patterns", operation_id = "list_sds_standard_patterns", responses((status = 200, body = Vec<SdsStandardPattern>)), tag = "sensitive-data-scanner")]
pub async fn standard_patterns(_ctx: AuthContext) -> ApiResult<Vec<SdsStandardPattern>> {
    Ok(Json(patterns::LIBRARY.iter().map(standard_out).collect()))
}

#[utoipa::path(post, path = "/api/v1/sensitive-data-scanner/groups", operation_id = "create_sds_group", request_body = SdsGroupInput,
    responses((status = 200, body = SdsGroup), (status = 400, body = crate::error::ErrorBody)), tag = "sensitive-data-scanner")]
pub async fn create_group(State(st): State<AppState>, Admin(ctx): Admin, Json(input): Json<SdsGroupInput>) -> ApiResult<SdsGroup> {
    let attrs = group_in(&input)?;
    let (id, _) = config::create_group(&st.postgres, &ctx.org_id, attrs).await?;
    ctx.audit(&st, "sds.group_created", input.name.trim(), "").await;
    group_reply(&st, &ctx.org_id, &id).await
}

#[utoipa::path(put, path = "/api/v1/sensitive-data-scanner/groups/{id}", operation_id = "update_sds_group", params(("id" = String, Path)), request_body = SdsGroupInput,
    responses((status = 200, body = SdsGroup), (status = 400, body = crate::error::ErrorBody), (status = 404, body = crate::error::ErrorBody)), tag = "sensitive-data-scanner")]
pub async fn update_group(State(st): State<AppState>, Admin(ctx): Admin, Path(id): Path<String>, Json(input): Json<SdsGroupInput>) -> ApiResult<SdsGroup> {
    let attrs = group_in(&input)?;
    let g = config::find_group(&st.postgres, &ctx.org_id, &id).await?;
    config::update_group(&st.postgres, &ctx.org_id, &g.id, attrs, None).await?;
    ctx.audit(&st, "sds.group_updated", input.name.trim(), "").await;
    group_reply(&st, &ctx.org_id, &g.id).await
}

#[utoipa::path(delete, path = "/api/v1/sensitive-data-scanner/groups/{id}", operation_id = "delete_sds_group", params(("id" = String, Path)),
    responses((status = 204), (status = 404, body = crate::error::ErrorBody)), tag = "sensitive-data-scanner")]
pub async fn delete_group(State(st): State<AppState>, Admin(ctx): Admin, Path(id): Path<String>) -> Result<StatusCode, ApiError> {
    let g = config::find_group(&st.postgres, &ctx.org_id, &id).await?;
    config::delete_group(&st.postgres, &ctx.org_id, &g.id).await?;
    ctx.audit(&st, "sds.group_deleted", &str_of(&g.attributes, "name"), "").await;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(put, path = "/api/v1/sensitive-data-scanner/group-order", operation_id = "reorder_sds_groups", request_body = SdsOrderInput,
    responses((status = 204), (status = 400, body = crate::error::ErrorBody)), tag = "sensitive-data-scanner")]
pub async fn reorder_groups(State(st): State<AppState>, Admin(ctx): Admin, Json(input): Json<SdsOrderInput>) -> Result<StatusCode, ApiError> {
    config::reorder_groups(&st.postgres, &ctx.org_id, &input.ids).await?;
    ctx.audit(&st, "sds.groups_reordered", "Sensitive Data Scanner", "").await;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(put, path = "/api/v1/sensitive-data-scanner/groups/{id}/rule-order", operation_id = "reorder_sds_rules", params(("id" = String, Path)), request_body = SdsOrderInput,
    responses((status = 204), (status = 400, body = crate::error::ErrorBody), (status = 404, body = crate::error::ErrorBody)), tag = "sensitive-data-scanner")]
pub async fn reorder_rules(State(st): State<AppState>, Admin(ctx): Admin, Path(id): Path<String>, Json(input): Json<SdsOrderInput>) -> Result<StatusCode, ApiError> {
    let g = config::find_group(&st.postgres, &ctx.org_id, &id).await?;
    config::reorder_rules(&st.postgres, &ctx.org_id, &g.id, &input.ids).await?;
    ctx.audit(&st, "sds.rules_reordered", &str_of(&g.attributes, "name"), "").await;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(post, path = "/api/v1/sensitive-data-scanner/rules", operation_id = "create_sds_rule", request_body = SdsRuleInput,
    responses((status = 200, body = SdsRule), (status = 400, body = crate::error::ErrorBody), (status = 404, body = crate::error::ErrorBody)), tag = "sensitive-data-scanner")]
pub async fn create_rule(State(st): State<AppState>, Admin(ctx): Admin, Json(input): Json<SdsRuleInput>) -> ApiResult<SdsRule> {
    let attrs = rule_in(&input)?;
    let (id, _) = config::create_rule(&st.postgres, &ctx.org_id, &input.group_id, standard_of(&input), attrs).await?;
    ctx.audit(&st, "sds.rule_created", input.name.trim(), "").await;
    Ok(Json(rule_out(&config::find_rule(&st.postgres, &ctx.org_id, &id).await?)))
}

#[utoipa::path(put, path = "/api/v1/sensitive-data-scanner/rules/{id}", operation_id = "update_sds_rule", params(("id" = String, Path)), request_body = SdsRuleInput,
    responses((status = 200, body = SdsRule), (status = 400, body = crate::error::ErrorBody), (status = 404, body = crate::error::ErrorBody)), tag = "sensitive-data-scanner")]
pub async fn update_rule(State(st): State<AppState>, Admin(ctx): Admin, Path(id): Path<String>, Json(input): Json<SdsRuleInput>) -> ApiResult<SdsRule> {
    let attrs = rule_in(&input)?;
    let r = config::find_rule(&st.postgres, &ctx.org_id, &id).await?;
    config::update_rule(&st.postgres, &ctx.org_id, &r.id, standard_of(&input), attrs).await?;
    ctx.audit(&st, "sds.rule_updated", input.name.trim(), "").await;
    Ok(Json(rule_out(&config::find_rule(&st.postgres, &ctx.org_id, &r.id).await?)))
}

#[utoipa::path(delete, path = "/api/v1/sensitive-data-scanner/rules/{id}", operation_id = "delete_sds_rule", params(("id" = String, Path)),
    responses((status = 204), (status = 404, body = crate::error::ErrorBody)), tag = "sensitive-data-scanner")]
pub async fn delete_rule(State(st): State<AppState>, Admin(ctx): Admin, Path(id): Path<String>) -> Result<StatusCode, ApiError> {
    let r = config::find_rule(&st.postgres, &ctx.org_id, &id).await?;
    config::delete_rule(&st.postgres, &ctx.org_id, &r.id).await?;
    ctx.audit(&st, "sds.rule_deleted", &str_of(&r.attributes, "name"), "").await;
    Ok(StatusCode::NO_CONTENT)
}

/// Tries a rule on sample text without saving it.
pub fn test_rule(input: &SdsTestRequest) -> Result<SdsTestResult, String> {
    if input.sample.len() > MAX_SAMPLE_BYTES {
        return Err(format!("The sample is at most {} KiB", MAX_SAMPLE_BYTES / 1024));
    }
    let mut attrs = rule_in(&input.rule).map_err(|e| e.to_string())?;
    attrs["is_enabled"] = json!(true);
    // The sample is a message: namespaces naming attributes would skip it.
    attrs["namespaces"] = json!([]);
    attrs["excluded_namespaces"] = json!([]);
    let cfg = Config {
        version: 0,
        groups: vec![GroupRow { id: "test".into(), attributes: json!({ "name": "test", "is_enabled": true, "filter": { "query": "*" } }) }],
        rules: vec![RuleRow { id: "test".into(), group_id: "test".into(), standard_pattern_id: standard_of(&input.rule).map(str::to_string), attributes: attrs }],
    };
    let (mut message, mut tags, mut attr) = (input.sample.clone(), Default::default(), Default::default());
    let hits = Scanner::compile(&cfg).scan(&mut Target { id: "test", service: "", status: "", host: "", source: "", message: &mut message, tags: &mut tags, attr: &mut attr }, b"sds-test");
    let mut tags: Vec<String> = tags.into_iter().map(|(k, v): (String, String)| if v.is_empty() { k } else { format!("{k}:{v}") }).collect();
    tags.sort();
    Ok(SdsTestResult { matches: hits.iter().map(|h| h.count).sum(), redacted: message, tags })
}

#[utoipa::path(post, path = "/api/v1/sensitive-data-scanner/test", operation_id = "test_sds_rule", request_body = SdsTestRequest,
    responses((status = 200, body = SdsTestResult), (status = 400, body = crate::error::ErrorBody)), tag = "sensitive-data-scanner")]
pub async fn test(_ctx: AuthContext, Json(input): Json<SdsTestRequest>) -> ApiResult<SdsTestResult> {
    test_rule(&input).map(Json).map_err(ApiError::BadRequest)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(json: Value) -> SdsRuleInput {
        serde_json::from_value(json).unwrap()
    }

    #[test]
    fn rules_round_trip_through_datadogs_attributes() {
        let input = rule(json!({
            "groupId": "g", "name": " Cards ", "standardPatternId": "credit_card", "namespaces": ["payment"], "tags": ["team:billing"], "enabled": true, "priority": 2,
            "replacement": { "kind": "partial_replacement_from_end", "numberOfChars": 12 }, "keywords": ["card"], "keywordCharacters": 20,
            "suppressions": { "exactMatch": ["4111 1111 1111 1111"] }
        }));
        let attrs = rule_in(&input).unwrap();
        assert_eq!(attrs["text_replacement"], json!({ "type": "partial_replacement_from_end", "number_of_chars": 12 }));
        let out = rule_out(&RuleRow { id: "r".into(), group_id: "g".into(), standard_pattern_id: Some("credit_card".into()), attributes: attrs });
        assert_eq!((out.name.as_str(), out.pattern, out.priority, out.keyword_characters), ("Cards", None, Some(2), Some(20)));
        assert_eq!(out.suppressions.exact_match, ["4111 1111 1111 1111"]);
        assert!(rule_in(&rule(json!({ "groupId": "g", "name": "n" }))).is_err(), "no pattern");
    }

    #[test]
    fn groups_keep_their_sampling_rate() {
        let attrs = group_in(&SdsGroupInput { name: "PCI".into(), description: String::new(), query: "service:billing".into(), enabled: true, products: vec![], sampling_rate: Some(25.0) }).unwrap();
        let cfg = Config::default();
        let g = group_out(&GroupRow { id: "g".into(), attributes: attrs }, &cfg);
        assert_eq!((g.sampling_rate, g.query.as_str(), g.products.clone()), (25.0, "service:billing", vec!["logs".to_string()]));
    }

    #[test]
    fn rules_are_tried_on_samples() {
        let r = test_rule(&SdsTestRequest {
            rule: rule(json!({ "groupId": "", "name": "t", "pattern": r"order-\d+", "namespaces": ["user"], "tags": ["sensitive_data:order"], "replacement": { "kind": "replacement_string", "replacementString": "[order]" } })),
            sample: "refund order-42 and order-43".into(),
        })
        .unwrap();
        assert_eq!((r.matches, r.redacted.as_str(), r.tags.clone()), (2, "refund [order] and [order]", vec!["sensitive_data:order".to_string()]));
        let r = test_rule(&SdsTestRequest { rule: rule(json!({ "groupId": "", "name": "t", "standardPatternId": "email_address" })), sample: "to alice@example.com".into() }).unwrap();
        assert_eq!((r.matches, r.redacted.as_str()), (1, "to alice@example.com"));
        assert!(r.tags.contains(&"sensitive_data:email_address".to_string()));
    }
}
