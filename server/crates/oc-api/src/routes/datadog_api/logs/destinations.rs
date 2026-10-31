//! Custom destinations, Datadog-compatible (`/api/v2/logs/config/custom-destinations`): the
//! provider's `datadog_logs_custom_destination`, on `log_custom_destinations` (forwarding:
//! [`crate::log_forwarding`]).
//!
//! - `forwarder_destination` is kept without its secrets (passwords, header values, access tokens),
//!   which are encrypted apart and never answered, as Datadog does.
//! - Changing the destination without a Splunk `sourcetype` keeps the previous one; secrets left out
//!   of a change are kept too.

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use serde_json::{json, Map, Value};

use super::super::users::{bad_request, body, data, not_found, ok, require, uuid, DocResult};
use super::super::{DdAuth, DdError};
use super::{dd_query, our_query};
use crate::auth::Access;
use crate::state::AppState;

const MAX_DESTINATIONS: i64 = 50;

#[derive(sqlx::FromRow)]
pub(crate) struct Row {
    pub id: String,
    pub name: String,
    pub query: String,
    pub enabled: bool,
    pub forward_tags: bool,
    pub tags_list: Value,
    pub tags_list_type: String,
    pub destination: Value,
    pub secrets_enc: String,
}

pub(crate) const SELECT: &str = "SELECT id::text AS id, name, query, enabled, forward_tags, tags_list, tags_list_type, destination, secrets_enc FROM log_custom_destinations";

fn doc(r: &Row) -> Value {
    json!({
        "type": "custom_destination",
        "id": r.id,
        "attributes": {
            "name": r.name,
            "query": dd_query(&r.query),
            "enabled": r.enabled,
            "forwarder_destination": r.destination,
            "forward_tags": r.forward_tags,
            "forward_tags_restriction_list": r.tags_list,
            "forward_tags_restriction_list_type": r.tags_list_type,
        }
    })
}

pub(crate) async fn find(st: &AppState, org: &str, id: &str) -> Result<Row, DdError> {
    let Some(id) = uuid(id) else { return Err(bad_request(format!("Invalid custom destination id {id}"))) };
    sqlx::query_as(&format!("{SELECT} WHERE org_id = $1 AND id = $2::uuid")).bind(org).bind(&id).fetch_optional(&st.postgres).await?.ok_or_else(|| not_found("Custom destination not found"))
}

/// Checks a `forwarder_destination` and splits it into (stored document, secrets).
pub(crate) fn split(d: &Value) -> Result<(Value, Map<String, Value>), DdError> {
    let invalid = |m: &str| DdError { status: StatusCode::BAD_REQUEST, errors: vec!["Invalid custom destination configuration".into(), m.to_string()] };
    let Some(o) = d.as_object() else { return Err(invalid("forwarder_destination must be an object")) };
    let mut doc = o.clone();
    let mut secrets = Map::new();
    let s = |k: &str| o.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    let https = |endpoint: &str| endpoint.starts_with("https://") || endpoint.starts_with("http://localhost") || endpoint.starts_with("http://127.0.0.1");
    match s("type").as_str() {
        "http" => {
            if !https(&s("endpoint")) {
                return Err(invalid("The endpoint must be an https URL"));
            }
            let auth = o.get("auth").and_then(Value::as_object).cloned().unwrap_or_default();
            let mut kept = Map::new();
            match auth.get("type").and_then(Value::as_str) {
                Some("basic") => {
                    for k in ["username", "password"] {
                        if let Some(v) = auth.get(k) {
                            secrets.insert(k.into(), v.clone());
                        }
                    }
                    kept.insert("type".into(), json!("basic"));
                }
                Some("custom_header") => {
                    let name = auth.get("header_name").and_then(Value::as_str).filter(|n| !n.is_empty()).ok_or_else(|| invalid("A custom header needs a header_name"))?;
                    if let Some(v) = auth.get("header_value") {
                        secrets.insert("header_value".into(), v.clone());
                    }
                    kept.insert("type".into(), json!("custom_header"));
                    kept.insert("header_name".into(), json!(name));
                }
                _ => return Err(invalid("auth.type must be basic or custom_header")),
            }
            doc.insert("auth".into(), Value::Object(kept));
        }
        "splunk_hec" => {
            if !https(&s("endpoint")) {
                return Err(invalid("The endpoint must be an https URL"));
            }
            if let Some(t) = doc.remove("access_token") {
                secrets.insert("access_token".into(), t);
            }
            if o.get("sourcetype").is_some_and(|v| !v.is_string() && !v.is_null()) {
                return Err(invalid("Sourcetype must be a string or null"));
            }
        }
        "elasticsearch" => {
            if !https(&s("endpoint")) {
                return Err(invalid("The endpoint must be an https URL"));
            }
            if s("index_name").is_empty() {
                return Err(invalid("An Elasticsearch destination needs an index_name"));
            }
            if let Some(auth) = o.get("auth").and_then(Value::as_object) {
                for k in ["username", "password"] {
                    if let Some(v) = auth.get(k) {
                        secrets.insert(k.into(), v.clone());
                    }
                }
            }
            doc.insert("auth".into(), json!({}));
        }
        "microsoft_sentinel" => {
            for k in ["tenant_id", "client_id", "data_collection_endpoint", "data_collection_rule_id", "stream_name"] {
                if s(k).is_empty() {
                    return Err(invalid(&format!("A Microsoft Sentinel destination needs {k}")));
                }
            }
        }
        "" => return Err(invalid("forwarder_destination.type is required")),
        t => return Err(invalid(&format!("Unknown destination type '{t}'"))),
    }
    Ok((Value::Object(doc), secrets))
}

/// Writable attributes over `current` (when updating).
pub(crate) struct Input {
    pub name: String,
    pub query: String,
    pub enabled: bool,
    pub forward_tags: bool,
    pub tags_list: Vec<String>,
    pub tags_list_type: String,
    pub destination: Value,
    pub secrets: Map<String, Value>,
}

pub(crate) fn input(a: &Value, current: Option<(&Row, Map<String, Value>)>) -> Result<Input, DdError> {
    let name = match a.get("name").and_then(Value::as_str) {
        Some(n) if !n.trim().is_empty() && n.chars().count() <= 200 => n.trim().to_string(),
        Some(_) => return Err(bad_request("The name must be 1–200 characters")),
        None => current.as_ref().map(|(c, _)| c.name.clone()).ok_or_else(|| bad_request("The parameter 'name' is required"))?,
    };
    let (destination, secrets) = match a.get("forwarder_destination") {
        Some(d) => {
            let (mut doc, mut secrets) = split(d)?;
            if let Some((c, old)) = &current {
                // Same type: keep what the change leaves out (Splunk's source type, secrets).
                if c.destination.get("type") == doc.get("type") {
                    if doc.get("type").and_then(Value::as_str) == Some("splunk_hec") && d.get("sourcetype").is_none() {
                        if let Some(st) = c.destination.get("sourcetype") {
                            doc["sourcetype"] = st.clone();
                        }
                    }
                    for (k, v) in old {
                        secrets.entry(k.clone()).or_insert_with(|| v.clone());
                    }
                }
            }
            (doc, secrets)
        }
        None => match &current {
            Some((c, old)) => (c.destination.clone(), old.clone()),
            None => return Err(bad_request("The parameter 'forwarder_destination' is required")),
        },
    };
    let tags_list_type = match a.get("forward_tags_restriction_list_type").and_then(Value::as_str) {
        Some(t @ ("ALLOW_LIST" | "BLOCK_LIST")) => t.to_string(),
        Some(_) => return Err(bad_request("forward_tags_restriction_list_type must be ALLOW_LIST or BLOCK_LIST")),
        None => current.as_ref().map_or("ALLOW_LIST".into(), |(c, _)| c.tags_list_type.clone()),
    };
    let tags_list = match a.get("forward_tags_restriction_list") {
        Some(Value::Array(t)) => t.iter().filter_map(Value::as_str).map(str::to_string).collect(),
        _ => current.as_ref().map(|(c, _)| serde_json::from_value(c.tags_list.clone()).unwrap_or_default()).unwrap_or_default(),
    };
    let query = match a.get("query").and_then(Value::as_str) {
        Some(q) => our_query(q),
        None => current.as_ref().map(|(c, _)| c.query.clone()).unwrap_or_default(),
    };
    oc_query::parse_query(&query);
    Ok(Input {
        name,
        query,
        enabled: a.get("enabled").and_then(Value::as_bool).unwrap_or_else(|| current.as_ref().is_none_or(|(c, _)| c.enabled)),
        forward_tags: a.get("forward_tags").and_then(Value::as_bool).unwrap_or_else(|| current.as_ref().is_none_or(|(c, _)| c.forward_tags)),
        tags_list,
        tags_list_type,
        destination,
        secrets,
    })
}

pub(crate) fn secrets_of(st: &AppState, r: &Row) -> Map<String, Value> {
    if r.secrets_enc.is_empty() {
        return Map::new();
    }
    st.secrets.decrypt(&r.secrets_enc).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

fn seal(st: &AppState, secrets: &Map<String, Value>) -> String {
    if secrets.is_empty() {
        String::new()
    } else {
        st.secrets.encrypt(&Value::Object(secrets.clone()).to_string())
    }
}

fn conflict(e: sqlx::Error, name: &str) -> DdError {
    match &e {
        sqlx::Error::Database(d) if d.is_unique_violation() => DdError::new(StatusCode::CONFLICT, format!("A custom destination named '{name}' already exists")),
        _ => e.into(),
    }
}

/// Inserts a destination (shared with our UI's API).
pub(crate) async fn insert(st: &AppState, org: &str, user: &str, i: &Input) -> Result<String, DdError> {
    let (count,): (i64,) = sqlx::query_as("SELECT count(*) FROM log_custom_destinations WHERE org_id = $1").bind(org).fetch_one(&st.postgres).await?;
    if count >= MAX_DESTINATIONS {
        return Err(bad_request(format!("An organization can have at most {MAX_DESTINATIONS} custom destinations")));
    }
    let (id,): (String,) = sqlx::query_as(
        "INSERT INTO log_custom_destinations (org_id, name, query, enabled, forward_tags, tags_list, tags_list_type, destination, secrets_enc, created_by, position) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10::uuid, (SELECT COALESCE(max(position), -1) + 1 FROM log_custom_destinations WHERE org_id = $1)) RETURNING id::text",
    )
    .bind(org)
    .bind(&i.name)
    .bind(&i.query)
    .bind(i.enabled)
    .bind(i.forward_tags)
    .bind(serde_json::to_value(&i.tags_list).unwrap_or_default())
    .bind(&i.tags_list_type)
    .bind(&i.destination)
    .bind(seal(st, &i.secrets))
    .bind(user)
    .fetch_one(&st.postgres)
    .await
    .map_err(|e| conflict(e, &i.name))?;
    Ok(id)
}

/// Replaces a destination's definition (shared with our UI's API).
pub(crate) async fn save(st: &AppState, org: &str, id: &str, i: &Input) -> Result<(), DdError> {
    sqlx::query(
        "UPDATE log_custom_destinations SET name = $3, query = $4, enabled = $5, forward_tags = $6, tags_list = $7, tags_list_type = $8, destination = $9, secrets_enc = $10, \
         last_error = NULL, last_error_at = NULL, updated_at = now() WHERE org_id = $1 AND id::text = $2",
    )
    .bind(org)
    .bind(id)
    .bind(&i.name)
    .bind(&i.query)
    .bind(i.enabled)
    .bind(i.forward_tags)
    .bind(serde_json::to_value(&i.tags_list).unwrap_or_default())
    .bind(&i.tags_list_type)
    .bind(&i.destination)
    .bind(seal(st, &i.secrets))
    .execute(&st.postgres)
    .await
    .map_err(|e| conflict(e, &i.name))?;
    Ok(())
}

fn attributes(raw: &Bytes) -> Result<Value, DdError> {
    let v = body(raw)?;
    let d = data(&v)?;
    if d.get("type").and_then(Value::as_str).is_some_and(|t| t != "custom_destination") {
        return Err(bad_request("data.type must be custom_destination"));
    }
    Ok(d.get("attributes").cloned().unwrap_or(json!({})))
}

pub async fn list(State(st): State<AppState>, DdAuth(ctx): DdAuth) -> DocResult {
    let rows: Vec<Row> = sqlx::query_as(&format!("{SELECT} WHERE org_id = $1 ORDER BY position, created_at")).bind(&ctx.org_id).fetch_all(&st.postgres).await?;
    ok(json!({ "data": rows.iter().map(doc).collect::<Vec<_>>() }))
}

pub async fn get(State(st): State<AppState>, DdAuth(ctx): DdAuth, Path(id): Path<String>) -> DocResult {
    ok(json!({ "data": doc(&find(&st, &ctx.org_id, &id).await?) }))
}

pub async fn create(State(st): State<AppState>, DdAuth(ctx): DdAuth, raw: Bytes) -> DocResult {
    require(&ctx, Access::Admin)?;
    let i = input(&attributes(&raw)?, None)?;
    let id = insert(&st, &ctx.org_id, &ctx.user_id, &i).await?;
    ctx.audit(&st, "log_destination.created", &i.name, "API").await;
    ok(json!({ "data": doc(&find(&st, &ctx.org_id, &id).await?) }))
}

pub async fn update(State(st): State<AppState>, DdAuth(ctx): DdAuth, Path(id): Path<String>, raw: Bytes) -> DocResult {
    require(&ctx, Access::Admin)?;
    let current = find(&st, &ctx.org_id, &id).await?;
    let old = secrets_of(&st, &current);
    let i = input(&attributes(&raw)?, Some((&current, old)))?;
    save(&st, &ctx.org_id, &current.id, &i).await?;
    ctx.audit(&st, "log_destination.updated", &i.name, "API").await;
    ok(json!({ "data": doc(&find(&st, &ctx.org_id, &current.id).await?) }))
}

pub async fn delete(State(st): State<AppState>, DdAuth(ctx): DdAuth, Path(id): Path<String>) -> Result<StatusCode, DdError> {
    require(&ctx, Access::Admin)?;
    let r = find(&st, &ctx.org_id, &id).await?;
    sqlx::query("DELETE FROM log_custom_destinations WHERE org_id = $1 AND id::text = $2").bind(&ctx.org_id).bind(&r.id).execute(&st.postgres).await?;
    ctx.audit(&st, "log_destination.deleted", &r.name, "API").await;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secrets_are_split_off() {
        let (doc, secrets) = split(&json!({"type": "http", "endpoint": "https://example.com", "auth": {"type": "basic", "username": "u", "password": "p"}})).unwrap();
        assert_eq!(doc["auth"], json!({"type": "basic"}));
        assert_eq!(secrets["password"], "p");
        let (doc, secrets) = split(&json!({"type": "splunk_hec", "endpoint": "https://example.com", "access_token": "t", "sourcetype": null})).unwrap();
        assert!(doc.get("access_token").is_none() && doc["sourcetype"].is_null());
        assert_eq!(secrets["access_token"], "t");
        let e = split(&json!({"type": "splunk_hec", "endpoint": "https://example.com", "sourcetype": 123})).unwrap_err();
        assert_eq!(e.errors[1], "Sourcetype must be a string or null");
        assert!(split(&json!({"type": "http", "endpoint": "ftp://x", "auth": {"type": "basic"}})).is_err());
        let (doc, _) = split(&json!({"type": "elasticsearch", "endpoint": "https://es.example.com", "index_name": "logs", "auth": {"username": "u", "password": "p"}})).unwrap();
        assert_eq!(doc["auth"], json!({}));
    }

    #[test]
    fn changes_keep_the_sourcetype_and_secrets() {
        let current = Row {
            id: "x".into(),
            name: "n".into(),
            query: String::new(),
            enabled: true,
            forward_tags: true,
            tags_list: json!([]),
            tags_list_type: "ALLOW_LIST".into(),
            destination: json!({"type": "splunk_hec", "endpoint": "https://a.example.com", "sourcetype": "my-sourcetype"}),
            secrets_enc: String::new(),
        };
        let mut old = Map::new();
        old.insert("access_token".into(), json!("t"));
        let i = input(&json!({"forwarder_destination": {"type": "splunk_hec", "endpoint": "https://b.example.com"}}), Some((&current, old.clone()))).unwrap();
        assert_eq!(i.destination["sourcetype"], "my-sourcetype");
        assert_eq!(i.secrets["access_token"], "t");
        let i = input(&json!({"name": "renamed"}), Some((&current, old))).unwrap();
        assert_eq!((i.name.as_str(), i.destination["endpoint"].as_str()), ("renamed", Some("https://a.example.com")));
    }
}
