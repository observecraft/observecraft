//! Sensitive Data Scanner, Datadog-compatible (`/api/v2/sensitive-data-scanner/config/...`): the provider's
//! `datadog_sensitive_data_scanner_group`, `_rule` and `_group_order`, and the `_standard_pattern` data source, on the
//! configuration the UI edits ([`crate::sds::config`]).
//!
//! - The configuration (one per org) answers with its groups in order, and `included` holds every group and rule;
//!   `PATCH /config` sets the groups' order (it must list each group once). `meta.version` moves on every change.
//! - Rules name a pattern of their own (the `regex` crate's syntax: no lookaround or backreferences) or one of our
//!   standard patterns (`relationships.standard_pattern`, [`crate::sds::patterns`]), which then has no `pattern`.
//! - Text replacements: `none`, `hash`, `replacement_string`, `partial_replacement_from_beginning` and
//!   `_from_end`. `should_save_match` is kept and not acted on (matches aren't saved).
//! - Updates replace a group's or rule's attributes; a name or pattern left out is kept. A group's
//!   `relationships.rules` listing exactly its rules reorders them.
//! - Reading needs any role; changing needs an admin, as Datadog's `data_scanner_write`.

use axum::body::Bytes;
use axum::extract::{Path, State};
use serde_json::{json, Value};

use super::users::{bad_request, body, created, data, not_found, ok, require, DocResult};
use super::{DdAuth, DdError};
use crate::auth::{Access, AuthContext};
use crate::sds::config::{self, Config, Error, GroupRow, RuleRow, GROUP_LIMIT, RULE_LIMIT};
use crate::sds::patterns;
use crate::state::AppState;

const CONFIG: &str = "sensitive_data_scanner_configuration";
const GROUP: &str = "sensitive_data_scanner_group";
const RULE: &str = "sensitive_data_scanner_rule";
const STANDARD: &str = "sensitive_data_scanner_standard_pattern";

impl From<Error> for DdError {
    fn from(e: Error) -> Self {
        match e {
            Error::Invalid(m) => bad_request(m),
            Error::NotFound(m) => not_found(m),
            Error::Db(e) => e.into(),
        }
    }
}

/// The configuration's `meta`.
pub fn meta(version: i64) -> Value {
    json!({
        "count_limit": RULE_LIMIT,
        "group_count_limit": GROUP_LIMIT,
        "rule_count_limit": RULE_LIMIT,
        "has_cascading_enabled": false,
        "has_highlight_enabled": true,
        "has_multi_pass_enabled": true,
        "is_configuration_superseded": false,
        "is_float_sampling_rate_enabled": true,
        "is_pci_compliant": false,
        "min_sampling_rate": 0,
        "version": version,
    })
}

pub fn group_doc(org: &str, g: &GroupRow, rules: &[&RuleRow]) -> Value {
    json!({
        "type": GROUP,
        "id": g.id,
        "attributes": g.attributes,
        "relationships": {
            "configuration": { "data": { "type": CONFIG, "id": config::config_id(org) } },
            "rules": { "data": rules.iter().map(|r| json!({ "type": RULE, "id": r.id })).collect::<Vec<_>>() },
        },
    })
}

pub fn rule_doc(r: &RuleRow) -> Value {
    let mut relationships = json!({ "group": { "data": { "type": GROUP, "id": r.group_id } } });
    if let Some(p) = &r.standard_pattern_id {
        relationships["standard_pattern"] = json!({ "data": { "type": STANDARD, "id": p } });
    }
    json!({ "type": RULE, "id": r.id, "attributes": r.attributes, "relationships": relationships })
}

/// `GET /config`'s document.
pub fn config_doc(org: &str, cfg: &Config) -> Value {
    let mut included: Vec<Value> = cfg.groups.iter().map(|g| group_doc(org, g, &cfg.rules_of(&g.id).collect::<Vec<_>>())).collect();
    included.extend(cfg.rules.iter().map(rule_doc));
    json!({
        "data": {
            "type": CONFIG,
            "id": config::config_id(org),
            "attributes": {},
            "relationships": { "groups": { "data": cfg.groups.iter().map(|g| json!({ "type": GROUP, "id": g.id })).collect::<Vec<_>>() } },
        },
        "meta": meta(cfg.version),
        "included": included,
    })
}

pub fn standard_pattern_doc(p: &patterns::StandardPattern) -> Value {
    json!({
        "type": STANDARD,
        "id": p.id,
        "attributes": {
            "name": p.name,
            "pattern": p.pattern,
            "description": p.description,
            "priority": p.priority,
            "tags": p.tags(),
            "included_keywords": p.keywords,
            "suppressions": { "starts_with": [], "ends_with": [], "exact_match": [] },
        },
    })
}

/// The request's `data`, of JSON:API type `kind`.
fn typed_data(raw: &Bytes, kind: &str) -> Result<Value, DdError> {
    let v = body(raw)?;
    let d = data(&v)?.clone();
    let got = d.get("type").and_then(Value::as_str).unwrap_or("");
    if got != kind {
        return Err(bad_request(format!("got type {got:?} expected one of {kind:?}")));
    }
    Ok(d)
}

fn attributes(d: &Value) -> Value {
    d.get("attributes").cloned().unwrap_or_else(|| json!({}))
}

/// `relationships.<name>.data`: None when the relationship is left out, Some(None) when its data is null.
fn relationship(d: &Value, name: &str) -> Option<Option<String>> {
    let r = d.get("relationships")?.get(name)?;
    Some(r.get("data").and_then(|v| v.get("id")).and_then(Value::as_str).map(str::to_string))
}

async fn audit(st: &AppState, ctx: &AuthContext, action: &str, target: &str) {
    ctx.audit(st, action, target, "API").await;
}

fn name_of(attrs: &Value) -> String {
    attrs.get("name").and_then(Value::as_str).unwrap_or("").to_string()
}

pub async fn get_config(State(st): State<AppState>, DdAuth(ctx): DdAuth) -> DocResult {
    require(&ctx, Access::ReadOnly)?;
    let cfg = config::load(&st.postgres, &ctx.org_id).await?;
    ok(config_doc(&ctx.org_id, &cfg))
}

/// `PATCH /config`: the groups' order.
pub async fn reorder_groups(State(st): State<AppState>, DdAuth(ctx): DdAuth, raw: Bytes) -> DocResult {
    require(&ctx, Access::Admin)?;
    let d = typed_data(&raw, CONFIG)?;
    let ids: Vec<String> = d
        .get("relationships")
        .and_then(|r| r.get("groups"))
        .and_then(|g| g.get("data"))
        .and_then(Value::as_array)
        .ok_or_else(|| bad_request("The groups' order (`relationships.groups.data`) is required"))?
        .iter()
        .map(|g| g.get("id").and_then(Value::as_str).map(str::to_string).ok_or_else(|| bad_request("Each group needs an `id`")))
        .collect::<Result<_, _>>()?;
    let version = config::reorder_groups(&st.postgres, &ctx.org_id, &ids).await?;
    audit(&st, &ctx, "sds.groups_reordered", "Sensitive Data Scanner").await;
    ok(json!({ "meta": meta(version) }))
}

pub async fn create_group(State(st): State<AppState>, DdAuth(ctx): DdAuth, raw: Bytes) -> DocResult {
    require(&ctx, Access::Admin)?;
    let d = typed_data(&raw, GROUP)?;
    let attrs = config::group_attributes(&attributes(&d), None).map_err(bad_request)?;
    let (id, version) = config::create_group(&st.postgres, &ctx.org_id, attrs).await?;
    let g = config::find_group(&st.postgres, &ctx.org_id, &id).await?;
    audit(&st, &ctx, "sds.group_created", &name_of(&g.attributes)).await;
    created(json!({ "data": group_doc(&ctx.org_id, &g, &[]), "meta": meta(version) }))
}

pub async fn update_group(State(st): State<AppState>, DdAuth(ctx): DdAuth, Path(group_id): Path<String>, raw: Bytes) -> DocResult {
    require(&ctx, Access::Admin)?;
    let d = typed_data(&raw, GROUP)?;
    let current = config::find_group(&st.postgres, &ctx.org_id, &group_id).await?;
    let attrs = config::group_attributes(&attributes(&d), Some(&current.attributes)).map_err(bad_request)?;
    let order = d
        .get("relationships")
        .and_then(|r| r.get("rules"))
        .and_then(|r| r.get("data"))
        .and_then(Value::as_array)
        .map(|items| items.iter().filter_map(|i| i.get("id").and_then(Value::as_str).map(str::to_string)).collect::<Vec<_>>());
    let name = name_of(&attrs);
    let version = config::update_group(&st.postgres, &ctx.org_id, &current.id, attrs, order).await?;
    audit(&st, &ctx, "sds.group_updated", &name).await;
    ok(json!({ "meta": meta(version) }))
}

pub async fn delete_group(State(st): State<AppState>, DdAuth(ctx): DdAuth, Path(group_id): Path<String>) -> DocResult {
    require(&ctx, Access::Admin)?;
    let g = config::find_group(&st.postgres, &ctx.org_id, &group_id).await?;
    let version = config::delete_group(&st.postgres, &ctx.org_id, &g.id).await?;
    audit(&st, &ctx, "sds.group_deleted", &name_of(&g.attributes)).await;
    ok(json!({ "meta": { "version": version } }))
}

pub async fn create_rule(State(st): State<AppState>, DdAuth(ctx): DdAuth, raw: Bytes) -> DocResult {
    require(&ctx, Access::Admin)?;
    let d = typed_data(&raw, RULE)?;
    let standard = relationship(&d, "standard_pattern").flatten();
    let attrs = config::rule_attributes(&attributes(&d), None, standard.as_deref()).map_err(bad_request)?;
    let group = relationship(&d, "group").flatten().ok_or_else(|| bad_request("A rule needs its group (`relationships.group`)"))?;
    let (id, version) = config::create_rule(&st.postgres, &ctx.org_id, &group, standard.as_deref(), attrs).await?;
    let r = config::find_rule(&st.postgres, &ctx.org_id, &id).await?;
    audit(&st, &ctx, "sds.rule_created", &name_of(&r.attributes)).await;
    created(json!({ "data": rule_doc(&r), "meta": { "version": version } }))
}

pub async fn update_rule(State(st): State<AppState>, DdAuth(ctx): DdAuth, Path(rule_id): Path<String>, raw: Bytes) -> DocResult {
    require(&ctx, Access::Admin)?;
    let d = typed_data(&raw, RULE)?;
    let current = config::find_rule(&st.postgres, &ctx.org_id, &rule_id).await?;
    let standard = relationship(&d, "standard_pattern").unwrap_or(current.standard_pattern_id.clone());
    let attrs = config::rule_attributes(&attributes(&d), Some(&current.attributes), standard.as_deref()).map_err(bad_request)?;
    let name = name_of(&attrs);
    let version = config::update_rule(&st.postgres, &ctx.org_id, &current.id, standard.as_deref(), attrs).await?;
    audit(&st, &ctx, "sds.rule_updated", &name).await;
    ok(json!({ "meta": { "version": version } }))
}

pub async fn delete_rule(State(st): State<AppState>, DdAuth(ctx): DdAuth, Path(rule_id): Path<String>) -> DocResult {
    require(&ctx, Access::Admin)?;
    let r = config::find_rule(&st.postgres, &ctx.org_id, &rule_id).await?;
    let version = config::delete_rule(&st.postgres, &ctx.org_id, &r.id).await?;
    audit(&st, &ctx, "sds.rule_deleted", &name_of(&r.attributes)).await;
    ok(json!({ "meta": { "version": version } }))
}

pub async fn standard_patterns(DdAuth(ctx): DdAuth) -> DocResult {
    require(&ctx, Access::ReadOnly)?;
    ok(json!({ "data": patterns::LIBRARY.iter().map(standard_pattern_doc).collect::<Vec<_>>() }))
}

#[cfg(test)]
mod tests {
    use super::super::replay::{compare_shape, Feature, TestServer};
    use super::*;

    /// Our documents have the fields and JSON types of Datadog's recorded answers (offline: no server needed).
    #[test]
    fn documents_have_the_recorded_shapes() {
        let Some(f) = Feature::load("v2/sensitive_data_scanner") else {
            return;
        };
        let mut problems = vec![];
        let mut compared = 0;
        for s in f.scenarios.iter() {
            let Some(interactions) = f.cassette(s) else {
                continue;
            };
            for it in interactions.iter().filter(|it| it.response.code / 100 == 2) {
                let Ok(recorded) = serde_json::from_str::<Value>(&it.response.body) else {
                    continue;
                };
                let path = it.request.url.split("/api/v2/sensitive-data-scanner").nth(1).unwrap_or("");
                let request: Value = serde_json::from_str(&it.request.body).unwrap_or(Value::Null);
                let req_data = request.get("data").cloned().unwrap_or(Value::Null);
                let ours = match (it.request.method.as_str(), path) {
                    ("GET", "/config") => {
                        let g = GroupRow { id: "6e70d6cf-e0cc-4c78-b57c-542d26dec9ac".into(), attributes: config::group_attributes(&json!({ "name": "g" }), None).unwrap() };
                        let r = RuleRow {
                            id: "x".into(),
                            group_id: g.id.clone(),
                            standard_pattern_id: None,
                            attributes: config::rule_attributes(&json!({ "name": "r", "pattern": "p" }), None, None).unwrap(),
                        };
                        config_doc("acme", &Config { version: 3, groups: vec![g], rules: vec![r] })
                    }
                    ("POST", "/config/groups") => {
                        let g = GroupRow { id: "id".into(), attributes: config::group_attributes(&req_data["attributes"], None).unwrap() };
                        json!({ "data": group_doc("acme", &g, &[]), "meta": meta(1) })
                    }
                    ("POST", "/config/rules") => {
                        let r = RuleRow { id: "id".into(), group_id: "g".into(), standard_pattern_id: None, attributes: config::rule_attributes(&req_data["attributes"], None, None).unwrap() };
                        json!({ "data": rule_doc(&r), "meta": { "version": 1 } })
                    }
                    ("PATCH", _) => json!({ "meta": meta(1) }),
                    ("DELETE", _) => json!({ "meta": { "version": 1 } }),
                    other => panic!("unexpected request {other:?}"),
                };
                compare_shape(&recorded, &ours, "", &[], &mut problems, &format!("{} {} {path}", s.name, it.request.method));
                compared += 1;
            }
        }
        assert!(compared >= 20, "{compared} answers compared");
        assert!(problems.is_empty(), "{}", problems.join("\n"));
    }

    #[test]
    fn standard_patterns_are_listed_as_datadog_does() {
        let doc = standard_pattern_doc(patterns::find("credit_card").unwrap());
        assert_eq!(doc["type"], STANDARD);
        assert_eq!(doc["attributes"]["tags"], json!(["sensitive_data:credit_card", "sensitive_data_category:finance"]));
        assert!(doc["attributes"].get("suppressions").is_some());
    }

    #[tokio::test]
    async fn replays_datadogs_sensitive_data_scanner_scenarios() {
        let Some(f) = Feature::load("v2/sensitive_data_scanner") else {
            return;
        };
        let Some(s) = TestServer::start("dd-sds-replay").await else {
            return;
        };
        let mut problems = vec![];
        let mut replayed = 0;
        for scenario in &f.scenarios {
            if f.cassette(scenario).is_none() {
                continue;
            }
            let (_, p) = s.replayer().replay(&f, scenario).await;
            problems.extend(p);
            replayed += 1;
        }
        assert!(replayed >= 12, "{replayed} scenarios");
        assert!(problems.is_empty(), "{}", problems.join("\n"));
    }
}
