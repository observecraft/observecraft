//! Datadog-compatible SLO corrections (`/api/v1/slo/correction`, `/api/v1/slo/{id}/corrections`), for
//! the Terraform provider's `datadog_slo_correction` and Datadog's API clients.
//!
//! A correction translates onto ours (`slo_corrections`, so it shows in the UI): `slo_id`, `category`
//! (`Scheduled Maintenance`, `Outside Business Hours`, `Deployment`, `Other`), `description` and the
//! one-off window `start`–`end` (epoch seconds). The attributes are stored as received
//! (`datadog_slo_correction_documents`) and returned with the server-owned fields (`created_at`,
//! `modified_at`, `creator`, `modifier`); parts changed in the UI since are taken from the correction.
//!
//! Not translated, rejected with 400: recurring corrections (`rrule`, `duration`) and corrections of
//! the SLOs matching a query (`slo_query`), which our corrections can't express. `timezone` only
//! matters for recurrences: it is stored and returned.

use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::routing::get;
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use utoipa::{IntoParams, ToSchema};

use super::slos::{hex_id, json_body, uuid_of, writer};
use super::{DdAuth, DdError};
use crate::routes::slos::{self as model, CorrectionCategory, SloCorrectionInput};
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/v1/slo/correction", get(list).post(create))
        .route("/api/v1/slo/correction/:id", get(get_one).patch(update).delete(delete))
        .route("/api/v1/slo/:id/corrections", get(for_slo))
}

/// Fields the server owns: never stored, always filled in on reads.
const SERVER_FIELDS: [&str; 4] = ["created_at", "modified_at", "creator", "modifier"];

// ================================================================ mapping

fn invalid(message: impl std::fmt::Display) -> String {
    format!("Invalid payload: {message}")
}

fn category_of(label: &str) -> Option<CorrectionCategory> {
    Some(match label {
        "Scheduled Maintenance" => CorrectionCategory::ScheduledMaintenance,
        "Outside Business Hours" => CorrectionCategory::OutsideBusinessHours,
        "Deployment" => CorrectionCategory::Deployment,
        "Other" => CorrectionCategory::Other,
        _ => return None,
    })
}

fn label_of(c: CorrectionCategory) -> &'static str {
    match c {
        CorrectionCategory::ScheduledMaintenance => "Scheduled Maintenance",
        CorrectionCategory::OutsideBusinessHours => "Outside Business Hours",
        CorrectionCategory::Deployment => "Deployment",
        CorrectionCategory::Other => "Other",
    }
}

fn present(attrs: &Map<String, Value>, k: &str) -> bool {
    attrs.get(k).is_some_and(|v| !v.is_null() && v.as_str() != Some(""))
}

/// Our correction input from a correction's Datadog attributes (`slo_id` as our UUID, times in ms).
pub fn to_input(attrs: &Value) -> Result<SloCorrectionInput, String> {
    let Some(a) = attrs.as_object() else { return Err(invalid("data.attributes must be an object")) };
    if present(a, "rrule") || present(a, "duration") {
        return Err(invalid("recurring corrections (rrule, duration) are not supported; set start and end"));
    }
    if present(a, "slo_query") {
        return Err(invalid("corrections for the SLOs matching a query (slo_query) are not supported; set slo_id"));
    }
    let slo = a.get("slo_id").and_then(Value::as_str).filter(|s| !s.is_empty()).ok_or_else(|| invalid("must specify slo_id"))?;
    let label = a.get("category").and_then(Value::as_str).unwrap_or_default();
    let category = category_of(label).ok_or_else(|| invalid(format!("unknown category '{label}' (Scheduled Maintenance, Outside Business Hours, Deployment or Other)")))?;
    let seconds = |k: &str| a.get(k).and_then(Value::as_i64).ok_or_else(|| invalid(format!("{k} is required (epoch seconds)")));
    let description = match a.get("description") {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(s)) => s.clone(),
        Some(_) => return Err(invalid("description must be a string")),
    };
    Ok(SloCorrectionInput {
        // An id that isn't one of ours can't name an SLO: it is reported as a missing SLO.
        slo_id: Some(uuid_of(slo).unwrap_or_else(|| slo.to_string())),
        category,
        description,
        start: seconds("start")?.saturating_mul(1000),
        end: seconds("end")?.saturating_mul(1000),
    })
}

/// A correction of ours, as stored in the table (times in ms).
#[derive(Clone, Debug, PartialEq)]
pub struct Current {
    pub slo_id: Option<String>,
    pub category: CorrectionCategory,
    pub description: String,
    pub start: i64,
    pub end: i64,
}

fn same(expected: &SloCorrectionInput, c: &Current) -> [bool; 4] {
    [expected.slo_id == c.slo_id, expected.category == c.category, expected.description.trim() == c.description, expected.start / 1000 == c.start / 1000 && expected.end / 1000 == c.end / 1000]
}

/// The attributes to return: the stored ones, with the parts our model holds taken from the
/// correction where it changed since; derived ones when none were stored.
pub fn reconcile(stored: Option<&Value>, c: &Current) -> Map<String, Value> {
    let expected = stored.map(to_input);
    let mut attrs = match (stored.and_then(Value::as_object), &expected) {
        (Some(a), Some(Ok(_))) => a.clone(),
        _ => Map::from_iter([("timezone".to_string(), json!("UTC"))]),
    };
    let [slo, category, description, window] = match &expected {
        Some(Ok(e)) => same(e, c),
        _ => [false; 4],
    };
    if !slo {
        attrs.insert("slo_id".into(), c.slo_id.as_deref().map_or(Value::Null, |s| json!(hex_id(s))));
    }
    if !category {
        attrs.insert("category".into(), json!(label_of(c.category)));
    }
    if !description {
        attrs.insert("description".into(), json!(c.description));
    }
    if !window {
        attrs.insert("start".into(), json!(c.start / 1000));
        attrs.insert("end".into(), json!(c.end / 1000));
    }
    attrs
}

/// A user as Datadog embeds it in a correction.
pub fn user(id: &str, name: &str, email: &str) -> Value {
    json!({ "data": { "type": "users", "id": id, "attributes": { "uuid": id, "handle": email, "email": email, "name": name, "icon": "" } } })
}

/// What the server adds to a correction on reads.
pub struct CorrectionServerFields {
    pub id: String,
    pub created_at: i64,
    pub modified_at: i64,
    pub creator: Option<Value>,
    pub modifier: Option<Value>,
}

/// A correction as Datadog returns it.
pub fn respond(mut attrs: Map<String, Value>, server: CorrectionServerFields) -> Value {
    attrs.insert("created_at".into(), json!(server.created_at));
    attrs.insert("modified_at".into(), json!(server.modified_at));
    attrs.insert("creator".into(), server.creator.unwrap_or(Value::Null));
    attrs.insert("modifier".into(), server.modifier.unwrap_or(Value::Null));
    for k in ["rrule", "duration"] {
        attrs.entry(k).or_insert(Value::Null);
    }
    attrs.entry("description").or_insert(json!(""));
    json!({ "type": "correction", "id": server.id, "attributes": attrs })
}

fn to_store(attrs: &Map<String, Value>) -> Value {
    let mut a = attrs.clone();
    for k in SERVER_FIELDS {
        a.remove(k);
    }
    Value::Object(a)
}

/// The attributes of a create or update body (`{"data": {"type": "correction", "attributes": {…}}}`).
fn attributes_of(body: &Value) -> Result<Map<String, Value>, String> {
    let data = body.get("data").ok_or_else(|| invalid("data is required"))?;
    if let Some(t) = data.get("type").and_then(Value::as_str).filter(|t| *t != "correction") {
        return Err(invalid(format!("data.type must be 'correction', not '{t}'")));
    }
    data.get("attributes").and_then(Value::as_object).cloned().ok_or_else(|| invalid("data.attributes is required"))
}

// ================================================================ storage

#[derive(sqlx::FromRow)]
struct Row {
    id: String,
    slo_id: Option<String>,
    category: String,
    description: String,
    start_s: i64,
    end_s: i64,
    created_at: i64,
    modified_at: Option<i64>,
    creator_id: Option<String>,
    creator_name: Option<String>,
    creator_email: Option<String>,
    modifier_id: Option<String>,
    modifier_name: Option<String>,
    modifier_email: Option<String>,
    attributes: Option<Value>,
}

const SELECT: &str = "SELECT c.id::text AS id, c.slo_id::text AS slo_id, c.category, c.description, \
    floor(extract(epoch FROM c.start_at))::bigint AS start_s, floor(extract(epoch FROM c.end_at))::bigint AS end_s, \
    extract(epoch FROM c.created_at)::bigint AS created_at, extract(epoch FROM d.modified_at)::bigint AS modified_at, \
    u.id::text AS creator_id, u.name AS creator_name, u.email AS creator_email, m.id::text AS modifier_id, m.name AS modifier_name, m.email AS modifier_email, d.attributes \
    FROM slo_corrections c LEFT JOIN datadog_slo_correction_documents d ON d.correction_id = c.id AND d.org_id = c.org_id \
    LEFT JOIN users u ON u.id = c.created_by AND u.org_id = c.org_id LEFT JOIN users m ON m.id = d.modified_by AND m.org_id = c.org_id";

impl Row {
    fn current(&self) -> Current {
        Current { slo_id: self.slo_id.clone(), category: CorrectionCategory::parse(&self.category), description: self.description.clone(), start: self.start_s * 1000, end: self.end_s * 1000 }
    }

    fn document(&self) -> Value {
        let person = |id: &Option<String>, name: &Option<String>, email: &Option<String>| id.as_ref().map(|id| user(id, name.as_deref().unwrap_or_default(), email.as_deref().unwrap_or_default()));
        respond(
            reconcile(self.attributes.as_ref(), &self.current()),
            CorrectionServerFields {
                id: self.id.clone(),
                created_at: self.created_at,
                modified_at: self.modified_at.unwrap_or(self.created_at),
                creator: person(&self.creator_id, &self.creator_name, &self.creator_email),
                modifier: person(&self.modifier_id, &self.modifier_name, &self.modifier_email),
            },
        )
    }
}

fn not_found(id: &str) -> DdError {
    DdError::new(StatusCode::NOT_FOUND, format!("SLO correction not found: {id} not found"))
}

/// A correction id is a UUID.
fn valid_id(id: &str) -> bool {
    uuid_of(id).is_some() && id.len() == 36
}

async fn load(st: &AppState, org: &str, id: &str) -> Result<Row, DdError> {
    if !valid_id(id) {
        return Err(not_found(id));
    }
    sqlx::query_as::<_, Row>(&format!("{SELECT} WHERE c.org_id = $1 AND c.id = $2::uuid")).bind(org).bind(id).fetch_optional(&st.postgres).await?.ok_or_else(|| not_found(id))
}

async fn store(conn: &mut sqlx::PgConnection, org: &str, id: &str, user: &str, attrs: &Map<String, Value>) -> Result<(), DdError> {
    sqlx::query(
        "INSERT INTO datadog_slo_correction_documents (correction_id, org_id, attributes, modified_by, modified_at) VALUES ($1::uuid, $2, $3, $4::uuid, now()) \
         ON CONFLICT (correction_id) DO UPDATE SET attributes = EXCLUDED.attributes, modified_by = EXCLUDED.modified_by, modified_at = now() \
         WHERE datadog_slo_correction_documents.org_id = EXCLUDED.org_id",
    )
    .bind(id)
    .bind(org)
    .bind(to_store(attrs))
    .bind(user)
    .execute(conn)
    .await?;
    Ok(())
}

fn bad_request(message: String) -> DdError {
    DdError::new(StatusCode::BAD_REQUEST, message)
}

// ================================================================ handlers

/// `{"data": correction}` or `{"data": [corrections], "meta": …}`.
#[derive(Serialize, ToSchema)]
pub struct DatadogSloCorrectionResponse {
    pub data: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub links: Option<Value>,
}

fn one(data: Value) -> Json<DatadogSloCorrectionResponse> {
    Json(DatadogSloCorrectionResponse { data, meta: None, links: None })
}

/// Create an SLO correction (Datadog-compatible).
#[utoipa::path(post, path = "/api/v1/slo/correction", operation_id = "datadog_create_slo_correction", request_body = Value,
    responses((status = 200, body = DatadogSloCorrectionResponse), (status = 400, body = super::DatadogErrors), (status = 404, body = super::DatadogErrors)), tag = "datadog-api")]
pub async fn create(State(st): State<AppState>, DdAuth(ctx): DdAuth, body: Bytes) -> Result<Json<DatadogSloCorrectionResponse>, DdError> {
    writer(&ctx)?;
    let attrs = attributes_of(&json_body(&body)?).map_err(bad_request)?;
    let input = to_input(&Value::Object(attrs.clone())).map_err(bad_request)?;
    let input = model::checked_correction(&st, &ctx.org_id, input).await?;
    let mut tx = st.postgres.begin().await?;
    let id = model::insert_correction(&mut tx, &ctx.org_id, &ctx.user_id, &input).await?;
    store(&mut tx, &ctx.org_id, &id, &ctx.user_id, &attrs).await?;
    tx.commit().await?;
    model::forget(&ctx.org_id);
    ctx.audit(&st, "slo_correction.create", &format!("slo_correction:{id}"), &input.description).await;
    Ok(one(load(&st, &ctx.org_id, &id).await?.document()))
}

/// Get an SLO correction (Datadog-compatible).
#[utoipa::path(get, path = "/api/v1/slo/correction/{id}", operation_id = "datadog_get_slo_correction", params(("id" = String, Path)),
    responses((status = 200, body = DatadogSloCorrectionResponse), (status = 404, body = super::DatadogErrors)), tag = "datadog-api")]
pub async fn get_one(State(st): State<AppState>, DdAuth(ctx): DdAuth, Path(id): Path<String>) -> Result<Json<DatadogSloCorrectionResponse>, DdError> {
    Ok(one(load(&st, &ctx.org_id, &id).await?.document()))
}

/// Update an SLO correction (Datadog-compatible): the attributes sent replace the stored ones; its SLO can't change.
#[utoipa::path(patch, path = "/api/v1/slo/correction/{id}", operation_id = "datadog_update_slo_correction", params(("id" = String, Path)), request_body = Value,
    responses((status = 200, body = DatadogSloCorrectionResponse), (status = 400, body = super::DatadogErrors), (status = 404, body = super::DatadogErrors)), tag = "datadog-api")]
pub async fn update(State(st): State<AppState>, DdAuth(ctx): DdAuth, Path(id): Path<String>, body: Bytes) -> Result<Json<DatadogSloCorrectionResponse>, DdError> {
    writer(&ctx)?;
    let existing = load(&st, &ctx.org_id, &id).await?;
    let changes = attributes_of(&json_body(&body)?).map_err(bad_request)?;
    let mut attrs = reconcile(existing.attributes.as_ref(), &existing.current());
    for (k, v) in changes {
        if k != "slo_id" {
            attrs.insert(k, v);
        }
    }
    let input = to_input(&Value::Object(attrs.clone())).map_err(bad_request)?;
    let input = model::checked_correction(&st, &ctx.org_id, input).await?;
    let mut tx = st.postgres.begin().await?;
    if !model::update_correction_row(&mut tx, &ctx.org_id, &existing.id, &input).await? {
        return Err(not_found(&id));
    }
    store(&mut tx, &ctx.org_id, &existing.id, &ctx.user_id, &attrs).await?;
    tx.commit().await?;
    model::forget(&ctx.org_id);
    ctx.audit(&st, "slo_correction.update", &format!("slo_correction:{}", existing.id), &input.description).await;
    Ok(one(load(&st, &ctx.org_id, &existing.id).await?.document()))
}

/// Delete an SLO correction (Datadog-compatible).
#[utoipa::path(delete, path = "/api/v1/slo/correction/{id}", operation_id = "datadog_delete_slo_correction", params(("id" = String, Path)),
    responses((status = 204), (status = 404, body = super::DatadogErrors)), tag = "datadog-api")]
pub async fn delete(State(st): State<AppState>, DdAuth(ctx): DdAuth, Path(id): Path<String>) -> Result<StatusCode, DdError> {
    writer(&ctx)?;
    if !valid_id(&id) {
        return Err(not_found(&id));
    }
    let r = sqlx::query("DELETE FROM slo_corrections WHERE org_id = $1 AND id = $2::uuid").bind(&ctx.org_id).bind(&id).execute(&st.postgres).await?;
    if r.rows_affected() == 0 {
        return Err(not_found(&id));
    }
    model::forget(&ctx.org_id);
    ctx.audit(&st, "slo_correction.delete", &format!("slo_correction:{id}"), "").await;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize, IntoParams, Default)]
#[serde(default)]
pub struct DatadogListSloCorrectionsParams {
    pub offset: Option<i64>,
    /// Default 25, at most 1,000.
    pub limit: Option<i64>,
}

/// List SLO corrections, newest first (Datadog-compatible).
#[utoipa::path(get, path = "/api/v1/slo/correction", operation_id = "datadog_list_slo_corrections", params(DatadogListSloCorrectionsParams),
    responses((status = 200, body = DatadogSloCorrectionResponse)), tag = "datadog-api")]
pub async fn list(State(st): State<AppState>, DdAuth(ctx): DdAuth, Query(p): Query<DatadogListSloCorrectionsParams>) -> Result<Json<DatadogSloCorrectionResponse>, DdError> {
    let limit = p.limit.filter(|l| *l > 0).unwrap_or(25).min(1000);
    let offset = p.offset.unwrap_or(0).max(0);
    let (total,): (i64,) = sqlx::query_as("SELECT count(*) FROM slo_corrections WHERE org_id = $1").bind(&ctx.org_id).fetch_one(&st.postgres).await?;
    let rows: Vec<Row> =
        sqlx::query_as(&format!("{SELECT} WHERE c.org_id = $1 ORDER BY c.created_at DESC, c.id LIMIT $2 OFFSET $3")).bind(&ctx.org_id).bind(limit).bind(offset).fetch_all(&st.postgres).await?;
    let meta = json!({ "page": { "total_count": total, "total_filtered_count": rows.len() } });
    Ok(Json(DatadogSloCorrectionResponse { data: Value::Array(rows.iter().map(Row::document).collect()), meta: Some(meta), links: None }))
}

#[derive(Deserialize, IntoParams, Default)]
#[serde(default)]
pub struct DatadogSloCorrectionsPageParams {
    #[serde(rename = "page[offset]")]
    #[param(rename = "page[offset]")]
    pub offset: Option<i64>,
    /// Default 10, at most 1,000.
    #[serde(rename = "page[limit]")]
    #[param(rename = "page[limit]")]
    pub limit: Option<i64>,
}

/// The corrections that apply to an SLO: its own and the organization-wide ones (Datadog-compatible).
#[utoipa::path(get, path = "/api/v1/slo/{id}/corrections", operation_id = "datadog_get_slo_corrections", params(("id" = String, Path), DatadogSloCorrectionsPageParams),
    responses((status = 200, body = DatadogSloCorrectionResponse), (status = 404, body = super::DatadogErrors)), tag = "datadog-api")]
pub async fn for_slo(State(st): State<AppState>, DdAuth(ctx): DdAuth, Path(id): Path<String>, Query(p): Query<DatadogSloCorrectionsPageParams>) -> Result<Json<DatadogSloCorrectionResponse>, DdError> {
    let missing = || DdError::new(StatusCode::NOT_FOUND, format!("SLO not found: {id} not found"));
    let slo = uuid_of(&id).ok_or_else(missing)?;
    let exists: Option<(i32,)> = sqlx::query_as("SELECT 1 FROM slos WHERE org_id = $1 AND id = $2::uuid").bind(&ctx.org_id).bind(&slo).fetch_optional(&st.postgres).await?;
    exists.ok_or_else(missing)?;
    let limit = p.limit.filter(|l| *l > 0).unwrap_or(10).min(1000);
    let offset = p.offset.unwrap_or(0).max(0);
    let rows: Vec<Row> = sqlx::query_as(&format!("{SELECT} WHERE c.org_id = $1 AND (c.slo_id IS NULL OR c.slo_id = $2::uuid) ORDER BY c.start_at DESC, c.id"))
        .bind(&ctx.org_id)
        .bind(&slo)
        .fetch_all(&st.postgres)
        .await?;
    let total = rows.len() as i64;
    let data: Vec<Value> = rows.iter().skip(offset as usize).take(limit as usize).map(Row::document).collect();
    let last = if total == 0 { 0 } else { (total - 1) / limit * limit };
    let path = format!("/api/v1/slo/{id}/corrections");
    let link = |o: i64| format!("{path}?page[offset]={o}&page[limit]={limit}");
    Ok(Json(DatadogSloCorrectionResponse {
        data: Value::Array(data),
        meta: Some(
            json!({ "pagination": { "offset": offset, "first_offset": 0, "prev_offset": (offset - limit).max(0), "next_offset": offset + limit, "last_offset": last, "limit": limit, "type": "offset_limit", "total": total } }),
        ),
        links: Some(
            json!({ "self": link(offset), "first": link(0), "next": link(offset + limit), "last": link(last), "prev": if offset > 0 { json!(link((offset - limit).max(0))) } else { Value::Null } }),
        ),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SLO: &str = "26d1f492d25b598fb6ef1a0405faa153";
    const SLO_UUID: &str = "26d1f492-d25b-598f-b6ef-1a0405faa153";

    fn attrs() -> Value {
        json!({"category":"Scheduled Maintenance","description":"Database upgrade","end":1652352611,"slo_id":SLO,"start":1652349011,"timezone":"Europe/Paris"})
    }

    #[test]
    fn a_one_off_correction_maps_onto_ours() {
        let i = to_input(&attrs()).unwrap();
        assert_eq!(i.slo_id.as_deref(), Some(SLO_UUID));
        assert_eq!((i.category, i.description.as_str(), i.start, i.end), (CorrectionCategory::ScheduledMaintenance, "Database upgrade", 1652349011000, 1652352611000));
        for (label, c) in [("Outside Business Hours", CorrectionCategory::OutsideBusinessHours), ("Deployment", CorrectionCategory::Deployment), ("Other", CorrectionCategory::Other)] {
            let mut a = attrs();
            a["category"] = json!(label);
            assert_eq!(to_input(&a).unwrap().category, c);
            assert_eq!(label_of(c), label);
        }
    }

    #[test]
    fn what_our_corrections_cant_express_is_rejected() {
        let err = |f: &dyn Fn(&mut Value)| {
            let mut a = attrs();
            f(&mut a);
            to_input(&a).unwrap_err()
        };
        assert_eq!(
            err(&|a| {
                a["rrule"] = json!("FREQ=DAILY;INTERVAL=10;COUNT=5");
                a["duration"] = json!(3600);
            }),
            "Invalid payload: recurring corrections (rrule, duration) are not supported; set start and end"
        );
        assert!(err(&|a| a["slo_query"] = json!("env:prod service:checkout")).contains("slo_query"));
        assert!(err(&|a| a["category"] = json!("Invalid Test")).contains("unknown category 'Invalid Test'"));
        assert!(err(&|a| a.as_object_mut().unwrap().remove("end").map(|_| ()).unwrap()).contains("end is required"));
        assert!(err(&|a| a.as_object_mut().unwrap().remove("slo_id").map(|_| ()).unwrap()).contains("must specify slo_id"));
        // Explicit nulls (as the provider's update can send) are fine.
        let mut a = attrs();
        a["rrule"] = Value::Null;
        a["duration"] = Value::Null;
        assert!(to_input(&a).is_ok());
        assert!(attributes_of(&json!({"data": {"type": "downtime", "attributes": {}}})).unwrap_err().contains("'correction'"));
    }

    #[test]
    fn reads_return_the_stored_attributes_until_the_correction_changes_elsewhere() {
        let a = attrs();
        let current = Current { slo_id: Some(SLO_UUID.into()), category: CorrectionCategory::ScheduledMaintenance, description: "Database upgrade".into(), start: 1652349011000, end: 1652352611000 };
        assert_eq!(Value::Object(reconcile(Some(&a), &current)), a);
        let edited = Current { category: CorrectionCategory::Deployment, end: 1652356211000, ..current.clone() };
        let r = reconcile(Some(&a), &edited);
        assert_eq!((r["category"].as_str(), r["end"].as_i64(), r["start"].as_i64()), (Some("Deployment"), Some(1652356211), Some(1652349011)));
        assert_eq!(r["timezone"], "Europe/Paris");
        // Made in the UI (org-wide here): derived.
        let ui = reconcile(None, &Current { slo_id: None, ..current });
        assert_eq!(Value::Object(ui), json!({"slo_id":null,"category":"Scheduled Maintenance","description":"Database upgrade","start":1652349011,"end":1652352611,"timezone":"UTC"}));
    }

    #[test]
    fn responses_have_datadogs_shape() {
        let r = respond(
            reconcile(
                Some(&attrs()),
                &Current { slo_id: Some(SLO_UUID.into()), category: CorrectionCategory::ScheduledMaintenance, description: "Database upgrade".into(), start: 1652349011000, end: 1652352611000 },
            ),
            CorrectionServerFields { id: "c-1".into(), created_at: 5, modified_at: 6, creator: Some(user("u-1", "Ada", "ada@example.com")), modifier: None },
        );
        assert_eq!((r["type"].as_str(), r["id"].as_str()), (Some("correction"), Some("c-1")));
        let a = &r["attributes"];
        assert_eq!((a["slo_id"].as_str(), a["rrule"].clone(), a["duration"].clone(), a["modifier"].clone()), (Some(SLO), Value::Null, Value::Null, Value::Null));
        assert_eq!(a["creator"]["data"]["attributes"]["email"], "ada@example.com");
        assert_eq!((a["created_at"].as_i64(), a["modified_at"].as_i64()), (Some(5), Some(6)));
        assert_eq!(to_store(a.as_object().unwrap()).get("creator"), None);
    }

    // ============================================================ live (OC_TEST_POSTGRES_URL / OC_TEST_CLICKHOUSE_URL)

    use super::super::replay::{new_org_with_roles, user_with_app_key, Feature, TestServer};
    use super::super::slos::tests::{call, sent_fields};
    use std::collections::HashMap;

    async fn new_slo(s: &TestServer) -> String {
        let doc = json!({"type":"metric","name":"Checkout","query":{"numerator":"sum:trace.http.request.hits{service:order-service,!http.status_code:5xx}.as_count()","denominator":"sum:trace.http.request.hits{service:order-service}.as_count()"},
            "thresholds":[{"target":99.0,"timeframe":"7d"}]});
        let (status, body) = call("POST", &format!("{}/api/v1/slo", s.base), &s.app_key, Some(doc)).await;
        assert_eq!(status, 200, "{body}");
        body["data"][0]["id"].as_str().unwrap().to_string()
    }

    #[tokio::test]
    async fn a_correction_made_through_the_compat_api_is_one_of_ours() {
        let Some(s) = TestServer::start("dd-slo-correction").await else { return };
        let slo = new_slo(&s).await;
        let ours = format!("Bearer {}", s.app_key);
        let sent = json!({"category":"Scheduled Maintenance","description":"Database upgrade","start":1793300000,"end":1793303600,"slo_id":slo,"timezone":"Europe/Paris"});
        let (status, created) = call("POST", &format!("{}/api/v1/slo/correction", s.base), &s.app_key, Some(json!({"data": {"type": "correction", "attributes": sent}}))).await;
        assert_eq!(status, 200, "{created}");
        let id = created["data"]["id"].as_str().unwrap().to_string();
        assert_eq!(created["data"]["type"], "correction");
        sent_fields(&sent, &created["data"]["attributes"]);
        assert_eq!(created["data"]["attributes"]["creator"]["data"]["attributes"]["name"], "Terraform Test");

        // Ours: the same correction, in ms.
        let (_, mine) = call("GET", &format!("{}/api/v1/slo-corrections", s.base), &ours, None).await;
        let c = mine.as_array().unwrap().iter().find(|c| c["id"] == json!(id)).unwrap().clone();
        assert_eq!((c["category"].as_str(), c["start"].as_i64(), c["end"].as_i64()), (Some("scheduled_maintenance"), Some(1793300000000), Some(1793303600000)));
        assert_eq!(c["sloId"].as_str().map(hex_id), Some(slo.clone()));

        // Edited in the UI: the compat read shows it.
        let edit = json!({"sloId": c["sloId"], "category": "deployment", "description": "Database upgrade", "start": 1793300000000i64, "end": 1793307200000i64});
        assert_eq!(call("PUT", &format!("{}/api/v1/slo-corrections/{id}", s.base), &ours, Some(edit)).await.0, 200);
        let (_, got) = call("GET", &format!("{}/api/v1/slo/correction/{id}", s.base), &s.app_key, None).await;
        let a = &got["data"]["attributes"];
        assert_eq!((a["category"].as_str(), a["end"].as_i64(), a["timezone"].as_str()), (Some("Deployment"), Some(1793307200), Some("Europe/Paris")));

        // Patched through the compat API: the attributes sent replace the stored ones.
        let patch = json!({"data": {"type": "correction", "attributes": {"category": "Other", "description": "Rollout", "start": 1793300000, "end": 1793310800, "timezone": "UTC"}}});
        let (status, patched) = call("PATCH", &format!("{}/api/v1/slo/correction/{id}", s.base), &s.app_key, Some(patch)).await;
        assert_eq!(status, 200, "{patched}");
        let a = &patched["data"]["attributes"];
        assert_eq!((a["category"].as_str(), a["description"].as_str(), a["end"].as_i64(), a["slo_id"].as_str()), (Some("Other"), Some("Rollout"), Some(1793310800), Some(slo.as_str())));
        assert_eq!(a["modifier"]["data"]["type"], "users");

        let (_, for_slo) = call("GET", &format!("{}/api/v1/slo/{slo}/corrections", s.base), &s.app_key, None).await;
        assert_eq!(for_slo["data"].as_array().map(Vec::len), Some(1));
        assert_eq!(for_slo["meta"]["pagination"]["total"], 1);

        assert_eq!(call("DELETE", &format!("{}/api/v1/slo/correction/{id}", s.base), &s.app_key, None).await.0, 204);
        assert_eq!(call("GET", &format!("{}/api/v1/slo/correction/{id}", s.base), &s.app_key, None).await.0, 404);
        assert_eq!(call("DELETE", &format!("{}/api/v1/slo/correction/{id}", s.base), &s.app_key, None).await.0, 404);
    }

    #[tokio::test]
    async fn corrections_are_checked_and_follow_the_keys_org_and_role() {
        let Some(s) = TestServer::start("dd-slo-correction-checks").await else { return };
        let slo = new_slo(&s).await;
        let url = format!("{}/api/v1/slo/correction", s.base);
        let body = |attrs: Value| json!({"data": {"type": "correction", "attributes": attrs}});
        let recurring = body(json!({"category":"Scheduled Maintenance","slo_id":slo,"start":1793300000,"duration":3600,"rrule":"FREQ=DAILY;INTERVAL=10;COUNT=5","timezone":"UTC"}));
        let (status, err) = call("POST", &url, &s.app_key, Some(recurring)).await;
        assert_eq!((status, err["errors"][0].as_str()), (400, Some("Invalid payload: recurring corrections (rrule, duration) are not supported; set start and end")));
        let unknown_slo = body(json!({"category":"Other","slo_id":"0123456789abcdef0123456789abcdef","start":1793300000,"end":1793303600}));
        assert_eq!(call("POST", &url, &s.app_key, Some(unknown_slo)).await.0, 404);
        let backwards = body(json!({"category":"Other","slo_id":slo,"start":1793303600,"end":1793300000}));
        assert_eq!(call("POST", &url, &s.app_key, Some(backwards)).await.0, 400);

        let ok = body(json!({"category":"Other","slo_id":slo,"start":1793300000,"end":1793303600}));
        let (_, reader) = user_with_app_key(&s.st, &s.org, "readonly").await;
        assert_eq!(call("POST", &url, &reader, Some(ok.clone())).await, (403, json!({"errors": ["Forbidden"]})));
        let (_, created) = call("POST", &url, &s.app_key, Some(ok)).await;
        let id = created["data"]["id"].as_str().unwrap().to_string();
        assert_eq!(call("GET", &format!("{url}/{id}"), &reader, None).await.0, 200);
        assert_eq!(call("DELETE", &format!("{url}/{id}"), &reader, None).await.0, 403);

        let other = new_org_with_roles(&s.st, "dd-correction-other").await;
        let (_, stranger) = user_with_app_key(&s.st, &other, "admin").await;
        assert_eq!(call("GET", &format!("{url}/{id}"), &stranger, None).await.0, 404);
        assert_eq!(call("DELETE", &format!("{url}/{id}"), &stranger, None).await.0, 404);
        assert_eq!(call("GET", &url, &stranger, None).await.1["data"], json!([]));
        // Another org's SLO can't be corrected.
        let theirs = body(json!({"category":"Other","slo_id":slo,"start":1793300000,"end":1793303600}));
        assert_eq!(call("POST", &url, &stranger, Some(theirs)).await.0, 404);
    }

    /// Datadog's recorded SLO correction scenarios.
    #[tokio::test]
    async fn slo_correction_cassettes_replay() {
        let Some(s) = TestServer::start("dd-correction-cassettes").await else { return };
        let feature = Feature::load("v1/service_level_objective_corrections");
        // Scenario → why it isn't replayed.
        let unsupported: HashMap<&str, &str> = HashMap::from([
            ("Create an SLO correction with rrule returns \"OK\" response", "recurring corrections (rrule) are rejected with 400: ours are one-off"),
            ("Create an SLO correction with slo_query returns \"OK\" response", "corrections by SLO query are rejected with 400: ours name an SLO"),
            ("Update an SLO correction with slo_query returns \"OK\" response", "corrections by SLO query are rejected with 400: ours name an SLO"),
        ]);
        let mut replayed = 0;
        for scenario in &feature.scenarios {
            if feature.cassette(scenario).is_none() {
                continue;
            }
            let (exchanges, problems) = s.replayer().replay(&feature, scenario).await;
            if let Some(reason) = unsupported.get(scenario.name.as_str()) {
                // Rejected as documented, in Datadog's error shape.
                let rejected = exchanges.iter().any(|e| e.path.starts_with("/api/v1/slo/correction") && e.recorded == 200 && e.status == 400 && e.body["errors"].is_array());
                assert!(rejected, "{}: {reason}: {exchanges:#?}", scenario.name);
                continue;
            }
            assert!(problems.is_empty(), "{}: {problems:#?}", scenario.name);
            replayed += 1;
        }
        assert_eq!(replayed, 5);
    }
}
