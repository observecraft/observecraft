//! `GET /api/v2/current_user`: the user an application key acts as, as a Datadog v2 user document
//! with its organization and roles (the Terraform provider's `datadog_current_user` data source).

use axum::extract::State;
use axum::Json;
use serde::Serialize;
use serde_json::{json, Value};
use utoipa::ToSchema;

use super::{DdAuth, DdError, DdResult};
use crate::state::AppState;

/// A Datadog v2 user response: `data` (type `users`) and the user's organization in `included`.
#[derive(Serialize, ToSchema)]
pub struct DatadogUserResponse {
    pub data: Value,
    pub included: Vec<Value>,
}

#[derive(sqlx::FromRow)]
struct Row {
    email: String,
    name: String,
    status: String,
    created_at: i64,
    org_name: String,
    org_created_at: i64,
    role_ids: Vec<String>,
}

fn iso(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms).map(|t| t.to_rfc3339()).unwrap_or_default()
}

/// The authenticated application key's user (Datadog-compatible).
#[utoipa::path(get, path = "/api/v2/current_user", operation_id = "datadog_current_user",
    responses((status = 200, body = DatadogUserResponse), (status = 403, body = super::DatadogErrors)), tag = "datadog-api")]
pub async fn current_user(State(st): State<AppState>, DdAuth(ctx): DdAuth) -> DdResult<DatadogUserResponse> {
    let row: Option<Row> = sqlx::query_as(
        "SELECT u.email, u.name, u.status, (extract(epoch FROM u.created_at) * 1000)::bigint AS created_at, o.name AS org_name, \
         (extract(epoch FROM o.created_at) * 1000)::bigint AS org_created_at, \
         COALESCE((SELECT array_agg(r.role_id ORDER BY r.role_id) FROM user_roles r WHERE r.user_id = u.id), '{}') AS role_ids \
         FROM users u JOIN orgs o ON o.id = u.org_id WHERE u.id = $1::uuid AND u.org_id = $2",
    )
    .bind(&ctx.user_id)
    .bind(&ctx.org_id)
    .fetch_optional(&st.postgres)
    .await?;
    let Some(r) = row else { return Err(DdError::forbidden()) };
    let status = match r.status.as_str() {
        "active" => "Active",
        "pending" => "Pending",
        _ => "Disabled",
    };
    let created = iso(r.created_at);
    let roles: Vec<Value> = r.role_ids.iter().map(|id| json!({ "type": "roles", "id": id })).collect();
    let data = json!({
        "type": "users",
        "id": ctx.user_id,
        "attributes": {
            "email": r.email, "handle": r.email, "name": r.name, "status": status, "disabled": status == "Disabled",
            "service_account": false, "verified": true, "created_at": created, "modified_at": created,
        },
        "relationships": { "org": { "data": { "type": "orgs", "id": ctx.org_id } }, "roles": { "data": roles } },
    });
    let org = json!({
        "type": "orgs",
        "id": ctx.org_id,
        "attributes": { "name": r.org_name, "public_id": ctx.org_id, "created_at": iso(r.org_created_at), "disabled": false },
    });
    Ok(Json(DatadogUserResponse { data, included: vec![org] }))
}

#[cfg(test)]
mod tests {
    use super::super::replay::TestServer;
    use serde_json::{json, Value};

    #[tokio::test]
    async fn current_user_is_the_keys_user_and_org() {
        let Some(s) = TestServer::start("dd-current-user").await else { return };
        let res = reqwest::Client::new().get(format!("{}/api/v2/current_user", s.base)).header("DD-APPLICATION-KEY", &s.app_key).send().await.unwrap();
        assert_eq!(res.status(), 200);
        let body: Value = res.json().await.unwrap();
        let user = &body["data"];
        assert_eq!((user["type"].as_str(), user["id"].as_str()), (Some("users"), Some(s.user_id.as_str())));
        let email = user["attributes"]["email"].as_str().unwrap();
        assert!(email.ends_with("@example.com"), "{body}");
        assert_eq!(user["attributes"]["handle"], json!(email));
        assert_eq!(user["attributes"]["name"], "Terraform Test");
        assert_eq!((user["attributes"]["status"].as_str(), user["attributes"]["disabled"].as_bool(), user["attributes"]["service_account"].as_bool()), (Some("Active"), Some(false), Some(false)));
        assert!(user["attributes"]["created_at"].is_string());
        assert_eq!(user["relationships"]["org"]["data"], json!({ "type": "orgs", "id": s.org }));
        assert_eq!(user["relationships"]["roles"]["data"], json!([{ "type": "roles", "id": format!("{}-admin", s.org) }]));
        let org = &body["included"][0];
        assert_eq!((org["type"].as_str(), org["id"].as_str()), (Some("orgs"), Some(s.org.as_str())));
        assert_eq!(org["attributes"]["name"], json!(s.org));
        assert_eq!(org["attributes"]["public_id"], json!(s.org));

        let res = reqwest::Client::new().get(format!("{}/api/v2/current_user", s.base)).header("DD-API-KEY", &s.api_key).send().await.unwrap();
        assert_eq!(res.status(), 403);
    }
}
