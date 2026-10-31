//! RUM applications, Datadog-compatible (`/api/v2/rum/applications`): the provider's
//! `datadog_rum_application` resource and data source, on our `rum_applications` table
//! ([`crate::routes::rum`]).
//!
//! - An application's id is its `application_id`; its client token is generated on create and never
//!   changes (`hash` repeats it, as Datadog's documents do; `api_key_id` is a number derived from it).
//! - `product_scales` reports the event processing (`ALL`, `ERROR_FOCUSED_MODE`, `NONE`) and Product
//!   Analytics retention (`MAX`, `NONE`) states, set with `rum_event_processing_state` and
//!   `product_analytics_retention_state`.
//! - `is_active` is false (Datadog sets it once the application has sent events).
//! - Not sent: the replay sample rate and the `rum_enabled*` / `product_analytics_enabled*` fields of
//!   older documents, which the Terraform provider's client rejects as unknown (`product_scales`
//!   carries the same states).
//! - Standard users create and change applications, as in our API.

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use serde_json::{json, Value};

use super::monitors::org_number;
use super::users::{bad_request, body, data, not_found, ok, require, DocResult};
use super::{DdAuth, DdError};
use crate::auth::Access;
use crate::routes::rum::{change_app, check_input, find_app, insert_app, list_apps, remove_app, RumApplication, RumApplicationInput};
use crate::state::AppState;

fn doc(a: &RumApplication, org: &str) -> Value {
    json!({
        "type": "rum_application",
        "id": a.id,
        "attributes": {
            "application_id": a.id, "name": a.name, "type": a.kind, "tags": a.tags,
            "client_token": a.client_token, "hash": a.client_token, "is_active": false, "api_key_id": org_number(&a.client_token) & 0x7fff_ffff,
            // An int32 in Datadog's RUM documents (org ids elsewhere are 64-bit).
            "org_id": org_number(org) & 0x7fff_ffff, "created_at": a.created_at, "updated_at": a.updated_at,
            "created_by_handle": a.created_by_handle.clone().unwrap_or_default(), "updated_by_handle": a.updated_by_handle.clone().unwrap_or_default(),
            "product_scales": {
                "rum_event_processing_scale": { "state": a.rum_event_processing_state, "last_modified_at": a.rum_event_processing_modified_at },
                "product_analytics_retention_scale": { "state": a.product_analytics_retention_state, "last_modified_at": a.product_analytics_modified_at },
            },
        },
    })
}

/// The list form: Datadog's `rum_application_list` documents (no client token).
fn list_doc(a: &RumApplication, org: &str) -> Value {
    let mut d = doc(a, org);
    d["type"] = json!("rum_application");
    if let Some(attrs) = d["attributes"].as_object_mut() {
        attrs.remove("client_token");
    }
    d
}

/// The attributes of a create or update body, checked.
fn input(raw: &Bytes, kind: &str, creating: bool) -> Result<(Option<String>, RumApplicationInput), DdError> {
    let v = body(raw)?;
    let d = data(&v)?;
    if d.get("type").and_then(Value::as_str) != Some(kind) {
        return Err(bad_request(format!("`data.type` must be {kind}")));
    }
    let attrs = d.get("attributes").cloned().unwrap_or_else(|| json!({}));
    let s = |k: &str| -> Result<Option<String>, DdError> {
        match attrs.get(k) {
            None | Some(Value::Null) => Ok(None),
            Some(Value::String(s)) => Ok(Some(s.clone())),
            Some(_) => Err(bad_request(format!("`{k}` must be a string"))),
        }
    };
    let tags = match attrs.get("tags") {
        None | Some(Value::Null) => None,
        Some(Value::Array(a)) => Some(a.iter().map(|t| t.as_str().map(str::to_string).ok_or_else(|| bad_request("`tags` must be strings"))).collect::<Result<Vec<_>, _>>()?),
        Some(_) => return Err(bad_request("`tags` must be a list")),
    };
    let mut i = RumApplicationInput {
        name: s("name")?,
        kind: s("type")?,
        tags,
        rum_event_processing_state: s("rum_event_processing_state")?,
        product_analytics_retention_state: s("product_analytics_retention_state")?,
    };
    check_input(&mut i, creating).map_err(bad_request)?;
    Ok((d.get("id").and_then(Value::as_str).map(str::to_string), i))
}

/// List RUM applications (Datadog-compatible).
#[utoipa::path(get, path = "/api/v2/rum/applications", operation_id = "datadog_list_rum_applications", responses((status = 200, body = super::users::DatadogIamDocument), (status = 403, body = super::DatadogErrors)), tag = "datadog-api")]
pub async fn list(State(st): State<AppState>, DdAuth(ctx): DdAuth) -> DocResult {
    let apps = list_apps(&st, &ctx.org_id).await?;
    ok(json!({ "data": apps.iter().map(|a| list_doc(a, &ctx.org_id)).collect::<Vec<_>>() }))
}

/// Get a RUM application (Datadog-compatible).
#[utoipa::path(get, path = "/api/v2/rum/applications/{id}", operation_id = "datadog_get_rum_application", params(("id" = String, Path)),
    responses((status = 200, body = super::users::DatadogIamDocument), (status = 404, body = super::DatadogErrors)), tag = "datadog-api")]
pub async fn get(State(st): State<AppState>, DdAuth(ctx): DdAuth, Path(id): Path<String>) -> DocResult {
    let app = find_app(&st, &ctx.org_id, &id).await?.ok_or_else(|| not_found("Not found"))?;
    ok(json!({ "data": doc(&app, &ctx.org_id) }))
}

/// Create a RUM application (Datadog-compatible); answers 200 with the document, as Datadog does.
#[utoipa::path(post, path = "/api/v2/rum/applications", operation_id = "datadog_create_rum_application", request_body = super::users::DatadogIamDocument,
    responses((status = 200, body = super::users::DatadogIamDocument), (status = 400, body = super::DatadogErrors)), tag = "datadog-api")]
pub async fn create(State(st): State<AppState>, DdAuth(ctx): DdAuth, raw: Bytes) -> DocResult {
    require(&ctx, Access::Standard)?;
    let (_, i) = input(&raw, "rum_application_create", true)?;
    let app = insert_app(&st, &ctx, i).await?;
    ok(json!({ "data": doc(&app, &ctx.org_id) }))
}

/// Update a RUM application (Datadog-compatible): the body's id must be the URL's (422 otherwise).
#[utoipa::path(patch, path = "/api/v2/rum/applications/{id}", operation_id = "datadog_update_rum_application", params(("id" = String, Path)), request_body = super::users::DatadogIamDocument,
    responses((status = 200, body = super::users::DatadogIamDocument), (status = 404, body = super::DatadogErrors), (status = 422, body = super::DatadogErrors)), tag = "datadog-api")]
pub async fn update(State(st): State<AppState>, DdAuth(ctx): DdAuth, Path(id): Path<String>, raw: Bytes) -> DocResult {
    require(&ctx, Access::Standard)?;
    let (body_id, i) = input(&raw, "rum_application_update", false)?;
    if body_id.as_deref() != Some(id.as_str()) {
        return Err(DdError::new(StatusCode::UNPROCESSABLE_ENTITY, "The id attribute in the request body does not match the id in the URL"));
    }
    if find_app(&st, &ctx.org_id, &id).await?.is_none() {
        return Err(not_found("Not found"));
    }
    let app = change_app(&st, &ctx, &id, i).await?;
    ok(json!({ "data": doc(&app, &ctx.org_id) }))
}

/// Delete a RUM application (Datadog-compatible).
#[utoipa::path(delete, path = "/api/v2/rum/applications/{id}", operation_id = "datadog_delete_rum_application", params(("id" = String, Path)),
    responses((status = 204), (status = 404, body = super::DatadogErrors)), tag = "datadog-api")]
pub async fn delete(State(st): State<AppState>, DdAuth(ctx): DdAuth, Path(id): Path<String>) -> Result<StatusCode, DdError> {
    require(&ctx, Access::Standard)?;
    // Like Datadog: an id that isn't an application id is not found; deleting one that no longer
    // exists succeeds (the provider deletes again after a resource is gone).
    if super::users::uuid(&id).is_none() {
        return Err(not_found("Not found"));
    }
    if find_app(&st, &ctx.org_id, &id).await?.is_some() {
        remove_app(&st, &ctx, &id).await?;
    }
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::super::replay::{Feature, TestServer};
    use super::*;

    #[test]
    fn bodies_need_their_document_type() {
        let raw = Bytes::from(r#"{"data":{"attributes":{"name":"wrong_rum_application","type":"wrong_android"},"type":"wrong_rum_application_type"}}"#);
        assert_eq!(input(&raw, "rum_application_create", true).unwrap_err().status, StatusCode::BAD_REQUEST);
        let raw = Bytes::from(r#"{"data":{"attributes":{"name":"x","type":"wrong_android"},"type":"rum_application_create"}}"#);
        assert!(input(&raw, "rum_application_create", true).unwrap_err().errors[0].contains("Unknown application type"));
        let raw = Bytes::from(r#"{"data":{"id":"abc","type":"rum_application_update"}}"#);
        let (id, i) = input(&raw, "rum_application_update", false).unwrap();
        assert_eq!((id.as_deref(), i.name), (Some("abc"), None));
    }

    #[tokio::test]
    async fn replays_datadogs_rum_application_scenarios() {
        let Some(f) = Feature::load("v2/rum") else { return };
        let Some(s) = TestServer::start("dd-rum-replay").await else { return };
        let mut problems = vec![];
        let mut replayed = 0;
        for scenario in &f.scenarios {
            if f.cassette(scenario).is_none() || !f.replayed(scenario) {
                continue;
            }
            let (_, p) = s.replayer().replay(&f, scenario).await;
            // Fields of Datadog's documents that the provider's client doesn't know (see the module docs).
            let unknown = ["attributes.product_analytics_replay_sample_rate`", "attributes.rum_enabled", "attributes.rum_disabled_at`", "attributes.product_analytics_enabled", "attributes.product_analytics_disabled_at`"];
            problems.extend(p.into_iter().filter(|m| !(m.contains("missing") && unknown.iter().any(|u| m.contains(u)))));
            replayed += 1;
        }
        assert!(replayed >= 7, "{replayed} scenarios");
        assert!(problems.is_empty(), "{}", problems.join("\n"));
    }
}
