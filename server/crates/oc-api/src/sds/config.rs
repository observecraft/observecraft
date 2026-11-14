//! An org's Sensitive Data Scanner configuration in Postgres (`sds_configs`, `sds_groups`, `sds_rules`): what
//! Datadog's API (`routes::datadog_api::sensitive_data_scanner`) and the UI (`routes::sensitive_data_scanner`) both
//! edit. Attributes are stored with Datadog's field names, checked and normalized here; every change moves the
//! configuration's version (Datadog's `meta.version`), which makes the indexers recompile the org's scanner.

use serde_json::{json, Map, Value};
use sqlx::PgPool;

use super::{compile_pattern, patterns};

/// Groups an org may have.
pub const GROUP_LIMIT: i64 = 50;
/// Rules an org may have, over all its groups.
pub const RULE_LIMIT: i64 = 1000;
const MAX_NAME_CHARS: usize = 200;
const MAX_DESCRIPTION_CHARS: usize = 1000;
const MAX_PATTERN_CHARS: usize = 4000;
const MAX_KEYWORDS: usize = 30;
const MAX_LIST: usize = 100;
pub const PRODUCTS: [&str; 4] = ["logs", "rum", "events", "apm"];
pub const REPLACEMENTS: [&str; 5] = ["none", "hash", "replacement_string", "partial_replacement_from_beginning", "partial_replacement_from_end"];

#[derive(sqlx::FromRow, Clone, Debug)]
pub struct GroupRow {
    pub id: String,
    pub attributes: Value,
}

#[derive(sqlx::FromRow, Clone, Debug)]
pub struct RuleRow {
    pub id: String,
    pub group_id: String,
    pub standard_pattern_id: Option<String>,
    pub attributes: Value,
}

/// An org's configuration: groups in order, rules in their group's order.
#[derive(Clone, Debug, Default)]
pub struct Config {
    pub version: i64,
    pub groups: Vec<GroupRow>,
    pub rules: Vec<RuleRow>,
}

impl Config {
    pub fn rules_of<'a>(&'a self, group: &'a str) -> impl Iterator<Item = &'a RuleRow> + 'a {
        self.rules.iter().filter(move |r| r.group_id == group)
    }
}

#[derive(Debug)]
pub enum Error {
    Invalid(String),
    NotFound(String),
    Db(sqlx::Error),
}

impl From<sqlx::Error> for Error {
    fn from(e: sqlx::Error) -> Self {
        Error::Db(e)
    }
}

impl From<Error> for crate::error::ApiError {
    fn from(e: Error) -> Self {
        match e {
            Error::Invalid(m) => crate::error::ApiError::BadRequest(m),
            Error::NotFound(m) => crate::error::ApiError::NotFound(m),
            Error::Db(e) => e.into(),
        }
    }
}

/// The configuration's id, as Datadog's API shows it (stable per org).
pub fn config_id(org: &str) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(format!("sds-config:{org}").as_bytes()).iter().map(|b| format!("{b:02x}")).collect()
}

fn uuid(s: &str) -> Option<String> {
    let s = s.trim().to_ascii_lowercase();
    let ok = s.len() == 36 && s.char_indices().all(|(i, c)| if [8, 13, 18, 23].contains(&i) { c == '-' } else { c.is_ascii_hexdigit() });
    ok.then_some(s)
}

pub async fn load(pg: &PgPool, org: &str) -> Result<Config, sqlx::Error> {
    let version = version(pg, org).await?;
    let groups: Vec<GroupRow> = sqlx::query_as("SELECT id::text AS id, attributes FROM sds_groups WHERE org_id = $1 ORDER BY position, created_at").bind(org).fetch_all(pg).await?;
    let rules: Vec<RuleRow> = sqlx::query_as(
        "SELECT r.id::text AS id, r.group_id::text AS group_id, r.standard_pattern_id, r.attributes FROM sds_rules r \
         JOIN sds_groups g ON g.org_id = r.org_id AND g.id = r.group_id WHERE r.org_id = $1 ORDER BY g.position, r.position, r.created_at",
    )
    .bind(org)
    .fetch_all(pg)
    .await?;
    Ok(Config { version, groups, rules })
}

pub async fn version(pg: &PgPool, org: &str) -> Result<i64, sqlx::Error> {
    let v: Option<(i64,)> = sqlx::query_as("SELECT version FROM sds_configs WHERE org_id = $1").bind(org).fetch_optional(pg).await?;
    Ok(v.map_or(0, |(v,)| v))
}

/// Serializes the org's changes and returns the next version (the transaction commits it).
async fn begin(pg: &PgPool, org: &str) -> Result<(sqlx::Transaction<'static, sqlx::Postgres>, i64), sqlx::Error> {
    let mut tx = pg.begin().await?;
    let (version,): (i64,) =
        sqlx::query_as("INSERT INTO sds_configs (org_id, version) VALUES ($1, 1) ON CONFLICT (org_id) DO UPDATE SET version = sds_configs.version + 1, updated_at = now() RETURNING version")
            .bind(org)
            .fetch_one(&mut *tx)
            .await?;
    Ok((tx, version))
}

// ================================================================ attributes

fn text(a: &Map<String, Value>, key: &str, max: usize, what: &str) -> Result<Option<String>, String> {
    match a.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) if s.chars().count() <= max => Ok(Some(s.clone())),
        Some(Value::String(_)) => Err(format!("{what} is at most {max} characters")),
        Some(_) => Err(format!("{what} must be a string")),
    }
}

fn string_list(a: &Map<String, Value>, key: &str, max: usize) -> Result<Vec<Value>, String> {
    match a.get(key) {
        None | Some(Value::Null) => Ok(vec![]),
        Some(Value::Array(items)) if items.len() <= max => items.iter().map(|i| i.as_str().map(|s| Value::from(s.trim())).ok_or_else(|| format!("`{key}` must be a list of strings"))).collect(),
        Some(Value::Array(_)) => Err(format!("`{key}` has at most {max} items")),
        Some(_) => Err(format!("`{key}` must be a list of strings")),
    }
}

fn boolean(a: &Map<String, Value>, key: &str) -> Result<bool, String> {
    match a.get(key) {
        None | Some(Value::Null) => Ok(false),
        Some(Value::Bool(b)) => Ok(*b),
        Some(_) => Err(format!("`{key}` must be a boolean")),
    }
}

fn object<'a>(v: &'a Value, what: &str) -> Result<&'a Map<String, Value>, String> {
    v.as_object().ok_or_else(|| format!("{what} must be an object"))
}

/// A group's attributes, normalized; `current` fills the name an update leaves out.
pub fn group_attributes(input: &Value, current: Option<&Value>) -> Result<Value, String> {
    let a = object(input, "`attributes`")?;
    let name = text(a, "name", MAX_NAME_CHARS, "The group name")?.or_else(|| current.and_then(|c| c.get("name")).and_then(Value::as_str).map(str::to_string)).unwrap_or_default();
    if name.trim().is_empty() {
        return Err("group name should not be blank".into());
    }
    let description = text(a, "description", MAX_DESCRIPTION_CHARS, "The description")?.unwrap_or_default();
    let query = match a.get("filter") {
        None | Some(Value::Null) => String::new(),
        Some(f) => text(object(f, "`filter`")?, "query", 2000, "The filter query")?.unwrap_or_default(),
    };
    let q = query.trim();
    if !q.is_empty() && q != "*" {
        crate::log_access::validate_query(q).map_err(|e| format!("Invalid filter query: {e}"))?;
    }
    let products = string_list(a, "product_list", PRODUCTS.len())?;
    if let Some(p) = products.iter().filter_map(Value::as_str).find(|p| !PRODUCTS.contains(p)) {
        return Err(format!("Unknown product {p:?} (one of {})", PRODUCTS.join(", ")));
    }
    let products = if products.is_empty() { vec![Value::from("logs")] } else { products };
    let mut samplings = vec![];
    if let Some(s) = a.get("samplings").filter(|s| !s.is_null()) {
        for item in s.as_array().ok_or("`samplings` must be a list")? {
            let product = item.get("product").and_then(Value::as_str).ok_or("A sampling needs a `product`")?;
            if !PRODUCTS.contains(&product) {
                return Err(format!("Unknown product {product:?} in `samplings`"));
            }
            let rate = item.get("rate").and_then(Value::as_f64).ok_or("A sampling needs a `rate`")?;
            if !(0.0..=100.0).contains(&rate) {
                return Err("A sampling rate is between 0 and 100".into());
            }
            samplings.push(json!({ "product": product, "rate": item["rate"] }));
        }
    }
    Ok(json!({
        "name": name.trim(),
        "description": description,
        "filter": { "query": query },
        "is_enabled": boolean(a, "is_enabled")?,
        "product_list": products,
        "samplings": samplings,
    }))
}

/// A rule's attributes, normalized; `current` fills the name and pattern an update leaves out. A rule using a
/// standard pattern (`standard`) has no pattern of its own.
pub fn rule_attributes(input: &Value, current: Option<&Value>, standard: Option<&str>) -> Result<Value, String> {
    let a = object(input, "`attributes`")?;
    let kept = |key: &str| current.and_then(|c| c.get(key)).and_then(Value::as_str).map(str::to_string);
    let name = text(a, "name", MAX_NAME_CHARS, "The rule name")?.or_else(|| kept("name")).unwrap_or_default();
    if name.trim().is_empty() {
        return Err("rule name should not be blank".into());
    }
    let mut out = Map::new();
    out.insert("name".into(), Value::from(name.trim()));
    out.insert("description".into(), Value::from(text(a, "description", MAX_DESCRIPTION_CHARS, "The description")?.unwrap_or_default()));
    match standard {
        Some(id) => {
            if patterns::find(id).is_none() {
                return Err(format!("Unknown standard pattern {id:?}"));
            }
        }
        None => {
            let pattern = text(a, "pattern", MAX_PATTERN_CHARS, "The pattern")?.or_else(|| kept("pattern")).unwrap_or_default();
            if pattern.is_empty() {
                return Err("A rule needs a pattern or a standard pattern".into());
            }
            compile_pattern(&pattern)?;
            out.insert("pattern".into(), Value::from(pattern));
        }
    }
    for key in ["namespaces", "excluded_namespaces", "tags"] {
        out.insert(key.into(), Value::Array(string_list(a, key, MAX_LIST)?.into_iter().filter(|v| v.as_str().is_some_and(|s| !s.is_empty())).collect()));
    }
    out.insert("is_enabled".into(), Value::from(boolean(a, "is_enabled")?));
    match a.get("priority") {
        None | Some(Value::Null) => {}
        Some(p) => {
            let p = p.as_i64().filter(|p| (1..=5).contains(p)).ok_or("`priority` is a number from 1 (highest) to 5")?;
            out.insert("priority".into(), Value::from(p));
        }
    }
    if let Some(t) = a.get("text_replacement").filter(|t| !t.is_null()) {
        let t = object(t, "`text_replacement`")?;
        let kind = t.get("type").and_then(Value::as_str).unwrap_or("none");
        if !REPLACEMENTS.contains(&kind) {
            return Err(format!("got type {kind:?} expected one of {}", REPLACEMENTS.map(|r| format!("{r:?}")).join(", ")));
        }
        let mut r = Map::new();
        r.insert("type".into(), Value::from(kind));
        if kind == "replacement_string" {
            let s = t.get("replacement_string").and_then(Value::as_str).ok_or("A `replacement_string` replacement needs its `replacement_string`")?;
            r.insert("replacement_string".into(), Value::from(s));
        } else if let Some(s) = t.get("replacement_string").and_then(Value::as_str) {
            r.insert("replacement_string".into(), Value::from(s));
        }
        if kind.starts_with("partial_") {
            let n = t.get("number_of_chars").and_then(Value::as_i64).filter(|n| *n >= 0).ok_or("A partial replacement needs `number_of_chars` (0 or more)")?;
            r.insert("number_of_chars".into(), Value::from(n));
        } else if let Some(n) = t.get("number_of_chars").and_then(Value::as_i64) {
            r.insert("number_of_chars".into(), Value::from(n));
        }
        if let Some(b) = t.get("should_save_match").and_then(Value::as_bool) {
            r.insert("should_save_match".into(), Value::from(b));
        }
        out.insert("text_replacement".into(), Value::Object(r));
    }
    if let Some(k) = a.get("included_keyword_configuration").filter(|k| !k.is_null()) {
        let k = object(k, "`included_keyword_configuration`")?;
        let keywords = string_list(k, "keywords", MAX_KEYWORDS)?;
        let mut c = Map::new();
        c.insert("keywords".into(), Value::Array(keywords));
        if let Some(n) = k.get("character_count").filter(|n| !n.is_null()) {
            let n = n.as_i64().filter(|n| (1..=50).contains(n)).ok_or("`character_count` is a number from 1 to 50")?;
            c.insert("character_count".into(), Value::from(n));
        }
        if let Some(b) = k.get("use_recommended_keywords").and_then(Value::as_bool) {
            c.insert("use_recommended_keywords".into(), Value::from(b));
        }
        out.insert("included_keyword_configuration".into(), Value::Object(c));
    }
    if let Some(s) = a.get("suppressions").filter(|s| !s.is_null()) {
        let s = object(s, "`suppressions`")?;
        let mut c = Map::new();
        for key in ["starts_with", "ends_with", "exact_match"] {
            c.insert(key.into(), Value::Array(string_list(s, key, MAX_LIST)?));
        }
        out.insert("suppressions".into(), Value::Object(c));
    }
    out.insert("labels".into(), Value::Array(string_list(a, "labels", MAX_LIST)?));
    Ok(Value::Object(out))
}

// ================================================================ changes

/// Creates a group (last in the order); returns its id and the new version.
pub async fn create_group(pg: &PgPool, org: &str, attributes: Value) -> Result<(String, i64), Error> {
    let (mut tx, version) = begin(pg, org).await?;
    let (count,): (i64,) = sqlx::query_as("SELECT count(*) FROM sds_groups WHERE org_id = $1").bind(org).fetch_one(&mut *tx).await?;
    if count >= GROUP_LIMIT {
        return Err(Error::Invalid(format!("An organization has at most {GROUP_LIMIT} scanning groups")));
    }
    let (id,): (String,) =
        sqlx::query_as("INSERT INTO sds_groups (org_id, position, attributes) VALUES ($1, (SELECT COALESCE(max(position), -1) + 1 FROM sds_groups WHERE org_id = $1), $2) RETURNING id::text")
            .bind(org)
            .bind(&attributes)
            .fetch_one(&mut *tx)
            .await?;
    tx.commit().await?;
    Ok((id, version))
}

pub async fn find_group(pg: &PgPool, org: &str, id: &str) -> Result<GroupRow, Error> {
    let not_found = || Error::NotFound(format!("Scanning group {id} not found"));
    let id = uuid(id).ok_or_else(not_found)?;
    sqlx::query_as("SELECT id::text AS id, attributes FROM sds_groups WHERE org_id = $1 AND id = $2::uuid").bind(org).bind(&id).fetch_optional(pg).await?.ok_or_else(not_found)
}

pub async fn find_rule(pg: &PgPool, org: &str, id: &str) -> Result<RuleRow, Error> {
    let not_found = || Error::NotFound(format!("Scanning rule {id} not found"));
    let id = uuid(id).ok_or_else(not_found)?;
    sqlx::query_as("SELECT id::text AS id, group_id::text AS group_id, standard_pattern_id, attributes FROM sds_rules WHERE org_id = $1 AND id = $2::uuid")
        .bind(org)
        .bind(&id)
        .fetch_optional(pg)
        .await?
        .ok_or_else(not_found)
}

/// Replaces a group's attributes; `rule_order`, when it lists exactly the group's rules, reorders them.
pub async fn update_group(pg: &PgPool, org: &str, id: &str, attributes: Value, rule_order: Option<Vec<String>>) -> Result<i64, Error> {
    let g = find_group(pg, org, id).await?;
    let (mut tx, version) = begin(pg, org).await?;
    sqlx::query("UPDATE sds_groups SET attributes = $3, updated_at = now() WHERE org_id = $1 AND id = $2::uuid").bind(org).bind(&g.id).bind(&attributes).execute(&mut *tx).await?;
    if let Some(order) = rule_order.filter(|o| !o.is_empty()) {
        let current: Vec<(String,)> = sqlx::query_as("SELECT id::text FROM sds_rules WHERE org_id = $1 AND group_id = $2::uuid").bind(org).bind(&g.id).fetch_all(&mut *tx).await?;
        let mut have: Vec<String> = current.into_iter().map(|(i,)| i).collect();
        let mut want: Vec<String> = order.iter().map(|i| i.to_ascii_lowercase()).collect();
        let listed = want.clone();
        have.sort();
        want.sort();
        if have == want {
            for (pos, rule) in listed.iter().enumerate() {
                sqlx::query("UPDATE sds_rules SET position = $3 WHERE org_id = $1 AND id = $2::uuid").bind(org).bind(rule).bind(pos as i32).execute(&mut *tx).await?;
            }
        }
    }
    tx.commit().await?;
    Ok(version)
}

/// Deletes a group and its rules.
pub async fn delete_group(pg: &PgPool, org: &str, id: &str) -> Result<i64, Error> {
    let g = find_group(pg, org, id).await?;
    let (mut tx, version) = begin(pg, org).await?;
    sqlx::query("DELETE FROM sds_groups WHERE org_id = $1 AND id = $2::uuid").bind(org).bind(&g.id).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(version)
}

/// Sets the groups' order: `ids` must list every group once.
pub async fn reorder_groups(pg: &PgPool, org: &str, ids: &[String]) -> Result<i64, Error> {
    let (mut tx, version) = begin(pg, org).await?;
    let current: Vec<(String,)> = sqlx::query_as("SELECT id::text FROM sds_groups WHERE org_id = $1").bind(org).fetch_all(&mut *tx).await?;
    let mut have: Vec<String> = current.into_iter().map(|(i,)| i).collect();
    let listed: Vec<String> = ids.iter().map(|i| i.trim().to_ascii_lowercase()).collect();
    let mut want = listed.clone();
    have.sort();
    want.sort();
    if have != want {
        return Err(Error::Invalid(format!("The number of groups in the request ({}) must match the configuration's groups ({}), each listed once", listed.len(), have.len())));
    }
    for (pos, id) in listed.iter().enumerate() {
        sqlx::query("UPDATE sds_groups SET position = $3 WHERE org_id = $1 AND id = $2::uuid").bind(org).bind(id).bind(pos as i32).execute(&mut *tx).await?;
    }
    tx.commit().await?;
    Ok(version)
}

/// Sets the order of a group's rules: `ids` must list each of them once.
pub async fn reorder_rules(pg: &PgPool, org: &str, group: &str, ids: &[String]) -> Result<i64, Error> {
    let g = find_group(pg, org, group).await?;
    let (mut tx, version) = begin(pg, org).await?;
    let current: Vec<(String,)> = sqlx::query_as("SELECT id::text FROM sds_rules WHERE org_id = $1 AND group_id = $2::uuid").bind(org).bind(&g.id).fetch_all(&mut *tx).await?;
    let mut have: Vec<String> = current.into_iter().map(|(i,)| i).collect();
    let listed: Vec<String> = ids.iter().map(|i| i.trim().to_ascii_lowercase()).collect();
    let mut want = listed.clone();
    have.sort();
    want.sort();
    if have != want {
        return Err(Error::Invalid("List each of the group's rules once".into()));
    }
    for (pos, id) in listed.iter().enumerate() {
        sqlx::query("UPDATE sds_rules SET position = $3 WHERE org_id = $1 AND id = $2::uuid").bind(org).bind(id).bind(pos as i32).execute(&mut *tx).await?;
    }
    tx.commit().await?;
    Ok(version)
}

/// Creates a rule (last in its group); returns its id and the new version.
pub async fn create_rule(pg: &PgPool, org: &str, group: &str, standard: Option<&str>, attributes: Value) -> Result<(String, i64), Error> {
    let g = find_group(pg, org, group).await?;
    let (mut tx, version) = begin(pg, org).await?;
    let (count,): (i64,) = sqlx::query_as("SELECT count(*) FROM sds_rules WHERE org_id = $1").bind(org).fetch_one(&mut *tx).await?;
    if count >= RULE_LIMIT {
        return Err(Error::Invalid(format!("An organization has at most {RULE_LIMIT} scanning rules")));
    }
    let (id,): (String,) = sqlx::query_as(
        "INSERT INTO sds_rules (org_id, group_id, position, standard_pattern_id, attributes) \
         VALUES ($1, $2::uuid, (SELECT COALESCE(max(position), -1) + 1 FROM sds_rules WHERE org_id = $1 AND group_id = $2::uuid), $3, $4) RETURNING id::text",
    )
    .bind(org)
    .bind(&g.id)
    .bind(standard)
    .bind(&attributes)
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok((id, version))
}

/// Replaces a rule's attributes and standard pattern.
pub async fn update_rule(pg: &PgPool, org: &str, id: &str, standard: Option<&str>, attributes: Value) -> Result<i64, Error> {
    let r = find_rule(pg, org, id).await?;
    let (mut tx, version) = begin(pg, org).await?;
    sqlx::query("UPDATE sds_rules SET attributes = $3, standard_pattern_id = $4, updated_at = now() WHERE org_id = $1 AND id = $2::uuid")
        .bind(org)
        .bind(&r.id)
        .bind(&attributes)
        .bind(standard)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(version)
}

pub async fn delete_rule(pg: &PgPool, org: &str, id: &str) -> Result<i64, Error> {
    let r = find_rule(pg, org, id).await?;
    let (mut tx, version) = begin(pg, org).await?;
    sqlx::query("DELETE FROM sds_rules WHERE org_id = $1 AND id = $2::uuid").bind(org).bind(&r.id).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(version)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn groups_are_normalized() {
        let g = group_attributes(&json!({ "name": " PCI ", "is_enabled": true, "filter": { "query": "*" } }), None).unwrap();
        assert_eq!(g, json!({ "name": "PCI", "description": "", "filter": { "query": "*" }, "is_enabled": true, "product_list": ["logs"], "samplings": [] }));
        let g = group_attributes(&json!({ "filter": {}, "samplings": [{ "product": "logs", "rate": 50 }] }), Some(&json!({ "name": "kept" }))).unwrap();
        assert_eq!(g["name"], "kept");
        assert_eq!(g["samplings"], json!([{ "product": "logs", "rate": 50 }]));
        assert!(group_attributes(&json!({ "filter": {} }), None).unwrap_err().contains("blank"));
        assert!(group_attributes(&json!({ "name": "x", "samplings": [{ "product": "logs", "rate": 101 }] }), None).is_err());
        assert!(group_attributes(&json!({ "name": "x", "product_list": ["traces"] }), None).is_err());
        assert!(group_attributes(&json!({ "name": "x", "filter": { "query": "service:(a" } }), None).is_err());
    }

    #[test]
    fn rules_are_normalized() {
        let r = rule_attributes(
            &json!({ "name": "n", "pattern": "pattern", "namespaces": ["admin"], "excluded_namespaces": ["admin.name"], "text_replacement": { "type": "none" }, "tags": ["sensitive_data:true"], "is_enabled": true, "priority": 1, "included_keyword_configuration": { "keywords": ["credit card"], "character_count": 35 } }),
            None,
            None,
        )
        .unwrap();
        assert_eq!(r["included_keyword_configuration"], json!({ "keywords": ["credit card"], "character_count": 35 }));
        assert_eq!(r["labels"], json!([]));
        assert_eq!(r["priority"], 1);
        assert!(rule_attributes(&json!({ "pattern": "pattern" }), None, None).unwrap_err().contains("rule name should not be blank"));
        assert_eq!(rule_attributes(&json!({ "priority": 5 }), Some(&json!({ "name": "n", "pattern": "p" })), None).unwrap()["pattern"], "p");
        assert!(rule_attributes(&json!({ "name": "n", "pattern": "(" }), None, None).is_err());
        assert!(rule_attributes(&json!({ "name": "n" }), None, None).is_err(), "no pattern");
        let std = rule_attributes(&json!({ "name": "n", "pattern": "ignored" }), None, Some("credit_card")).unwrap();
        assert!(std.get("pattern").is_none());
        assert!(rule_attributes(&json!({ "name": "n" }), None, Some("nope")).is_err());
        assert!(rule_attributes(&json!({ "name": "n", "pattern": "p", "text_replacement": { "type": "redacted_hash" } }), None, None).is_err());
        assert!(rule_attributes(&json!({ "name": "n", "pattern": "p", "text_replacement": { "type": "partial_replacement_from_end" } }), None, None).is_err());
        assert!(rule_attributes(&json!({ "name": "n", "pattern": "p", "priority": 9 }), None, None).is_err());
        assert!(rule_attributes(&json!({ "name": "n", "pattern": "p", "included_keyword_configuration": { "keywords": [], "character_count": 51 } }), None, None).is_err());
    }

    #[test]
    fn config_ids_are_stable_per_org() {
        assert_eq!(config_id("acme"), config_id("acme"));
        assert_ne!(config_id("acme"), config_id("other"));
        assert_eq!(config_id("acme").len(), 64);
    }
}
