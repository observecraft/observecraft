//! Datadog-compatible dashboard lists (`datadog_dashboard_list` and its data source in Datadog's
//! Terraform provider, the `dashboard_lists` of `datadog_dashboard`; `DashboardListsApi` v1 and v2 of
//! the API clients), on our `dashboard_lists` table, so lists made here show in the dashboard list's
//! sidebar and the other way round.
//!
//! - v1: `GET`/`POST /api/v1/dashboard/lists/manual`, `GET`/`PUT`/`DELETE /api/v1/dashboard/lists/manual/{id}`.
//! - v2: `GET`/`POST`/`PUT`/`DELETE /api/v2/dashboard/lists/manual/{id}/dashboards` (the list's items).
//!
//! Datadog's list ids are integers (`dashboard_lists.datadog_id`). An item's `type` is derived from
//! the dashboard (custom or integration; `free` layout → screenboard, else timeboard).

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::get;
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use utoipa::ToSchema;

use super::dashboards::{iso, parse_body, url, writer};
use super::{DdAuth, DdError};
use crate::routes::dashboards::new_id;
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/v1/dashboard/lists/manual", get(list).post(create))
        .route("/api/v1/dashboard/lists/manual/:id", get(get_one).put(update).delete(delete_one))
        .route("/api/v2/dashboard/lists/manual/:id/dashboards", get(items).post(add_items).put(replace_items).delete(remove_items))
}

const ITEM_TYPES: &[&str] = &["custom_timeboard", "custom_screenboard", "integration_timeboard", "integration_screenboard", "host_timeboard"];

/// A Datadog dashboard list (`manual_dashboard_list`).
#[derive(Serialize, ToSchema)]
pub struct DatadogDashboardList {
    pub id: i64,
    pub name: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub is_favorite: bool,
    pub dashboard_count: i64,
    pub author: DatadogListAuthor,
    pub created: String,
    pub modified: String,
    #[schema(value_type = Option<Vec<Object>>)]
    pub dashboards: Option<Vec<Value>>,
}

#[derive(Serialize, ToSchema)]
pub struct DatadogListAuthor {
    pub handle: String,
    pub name: Option<String>,
}

#[derive(Serialize, ToSchema)]
pub struct DatadogDashboardLists {
    pub dashboard_lists: Vec<DatadogDashboardList>,
}

#[derive(Serialize, ToSchema)]
pub struct DatadogDeletedDashboardList {
    pub deleted_dashboard_list_id: i64,
}

/// `{"name": "…"}`.
#[derive(Deserialize, ToSchema)]
pub struct DatadogDashboardListName {
    pub name: String,
}

/// `{"dashboards": [{"id": "abc-def-ghi", "type": "custom_timeboard"}]}`.
#[derive(Deserialize, ToSchema)]
pub struct DatadogDashboardListItemsRequest {
    pub dashboards: Vec<DatadogDashboardListItemRef>,
}

#[derive(Serialize, Deserialize, ToSchema, Clone, Debug, PartialEq, Eq)]
pub struct DatadogDashboardListItemRef {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
}

/// `GET …/dashboards`: the list's dashboards.
#[derive(Serialize, ToSchema)]
pub struct DatadogDashboardListItems {
    #[schema(value_type = Vec<Object>)]
    pub dashboards: Vec<Value>,
    pub total: i64,
}

#[derive(Serialize, ToSchema)]
pub struct DatadogAddedDashboardListItems {
    pub added_dashboards_to_list: Vec<DatadogDashboardListItemRef>,
}

#[derive(Serialize, ToSchema)]
pub struct DatadogUpdatedDashboardListItems {
    pub dashboards: Vec<DatadogDashboardListItemRef>,
}

#[derive(Serialize, ToSchema)]
pub struct DatadogDeletedDashboardListItems {
    pub deleted_dashboards_from_list: Vec<DatadogDashboardListItemRef>,
}

#[derive(sqlx::FromRow)]
struct ListRow {
    id: String,
    datadog_id: i64,
    name: String,
    dashboard_ids: Vec<String>,
    dashboard_count: i64,
    author_handle: Option<String>,
    author_name: Option<String>,
    created_at: i64,
    modified_at: i64,
}

const SELECT: &str = "SELECT l.id, l.datadog_id, l.name, l.dashboard_ids, \
    (SELECT count(*) FROM dashboards d WHERE d.org_id = l.org_id AND d.id = ANY(l.dashboard_ids) AND d.deleted_at IS NULL) AS dashboard_count, \
    u.email AS author_handle, u.name AS author_name, (extract(epoch FROM l.created_at) * 1000)::bigint AS created_at, \
    (extract(epoch FROM l.modified_at) * 1000)::bigint AS modified_at FROM dashboard_lists l LEFT JOIN users u ON u.id = l.author_id";

impl ListRow {
    fn datadog(self) -> DatadogDashboardList {
        DatadogDashboardList {
            id: self.datadog_id,
            name: self.name,
            kind: "manual_dashboard_list".into(),
            is_favorite: false,
            dashboard_count: self.dashboard_count,
            author: DatadogListAuthor { handle: self.author_handle.unwrap_or_default(), name: self.author_name },
            created: iso(self.created_at),
            modified: iso(self.modified_at),
            dashboards: None,
        }
    }
}

fn not_found(id: &str) -> DdError {
    DdError::new(StatusCode::NOT_FOUND, format!("Dashboard list {id} not found"))
}

/// Datadog's list ids are integers; anything else names no list.
fn list_id(id: &str) -> Result<i64, DdError> {
    id.parse().map_err(|_| not_found(id))
}

async fn load(st: &AppState, org: &str, id: &str) -> Result<ListRow, DdError> {
    let row: Option<ListRow> = sqlx::query_as(&format!("{SELECT} WHERE l.org_id = $1 AND l.datadog_id = $2")).bind(org).bind(list_id(id)?).fetch_optional(&st.postgres).await?;
    row.ok_or_else(|| not_found(id))
}

fn name(body: &Bytes) -> Result<String, DdError> {
    let req: DatadogDashboardListName = serde_json::from_value(parse_body(body)?).map_err(|e| DdError::new(StatusCode::BAD_REQUEST, format!("Invalid body: {e}")))?;
    // Stored as sent: Terraform compares the name it reads with its configuration.
    if req.name.trim().is_empty() || req.name.chars().count() > 200 {
        return Err(DdError::new(StatusCode::BAD_REQUEST, "name: must be 1-200 characters"));
    }
    Ok(req.name)
}

// ================================================================ v1 lists

/// The org's dashboard lists (Datadog-compatible).
#[utoipa::path(get, path = "/api/v1/dashboard/lists/manual", operation_id = "datadog_list_dashboard_lists", responses((status = 200, body = DatadogDashboardLists)), tag = "datadog-api")]
pub async fn list(State(st): State<AppState>, DdAuth(ctx): DdAuth) -> Result<Json<DatadogDashboardLists>, DdError> {
    let rows: Vec<ListRow> = sqlx::query_as(&format!("{SELECT} WHERE l.org_id = $1 ORDER BY lower(l.name), l.datadog_id")).bind(&ctx.org_id).fetch_all(&st.postgres).await?;
    Ok(Json(DatadogDashboardLists { dashboard_lists: rows.into_iter().map(ListRow::datadog).collect() }))
}

/// Creates a dashboard list (Datadog-compatible).
#[utoipa::path(post, path = "/api/v1/dashboard/lists/manual", operation_id = "datadog_create_dashboard_list", request_body = DatadogDashboardListName,
    responses((status = 200, body = DatadogDashboardList), (status = 400, body = super::DatadogErrors)), tag = "datadog-api")]
pub async fn create(State(st): State<AppState>, DdAuth(ctx): DdAuth, body: Bytes) -> Result<Json<DatadogDashboardList>, DdError> {
    writer(&ctx)?;
    let name = name(&body)?;
    let (datadog_id,): (i64,) = sqlx::query_as("INSERT INTO dashboard_lists (id, org_id, name, author_id) VALUES ($1, $2, $3, $4::uuid) RETURNING datadog_id")
        .bind(format!("l-{}", new_id()))
        .bind(&ctx.org_id)
        .bind(&name)
        .bind(&ctx.user_id)
        .fetch_one(&st.postgres)
        .await?;
    Ok(Json(load(&st, &ctx.org_id, &datadog_id.to_string()).await?.datadog()))
}

/// A dashboard list (Datadog-compatible).
#[utoipa::path(get, path = "/api/v1/dashboard/lists/manual/{id}", operation_id = "datadog_get_dashboard_list", params(("id" = i64, Path)),
    responses((status = 200, body = DatadogDashboardList), (status = 404, body = super::DatadogErrors)), tag = "datadog-api")]
pub async fn get_one(State(st): State<AppState>, DdAuth(ctx): DdAuth, Path(id): Path<String>) -> Result<Json<DatadogDashboardList>, DdError> {
    Ok(Json(load(&st, &ctx.org_id, &id).await?.datadog()))
}

/// Renames a dashboard list (Datadog-compatible).
#[utoipa::path(put, path = "/api/v1/dashboard/lists/manual/{id}", operation_id = "datadog_update_dashboard_list", params(("id" = i64, Path)), request_body = DatadogDashboardListName,
    responses((status = 200, body = DatadogDashboardList), (status = 400, body = super::DatadogErrors), (status = 404, body = super::DatadogErrors)), tag = "datadog-api")]
pub async fn update(State(st): State<AppState>, DdAuth(ctx): DdAuth, Path(id): Path<String>, body: Bytes) -> Result<Json<DatadogDashboardList>, DdError> {
    writer(&ctx)?;
    let name = name(&body)?;
    let done =
        sqlx::query("UPDATE dashboard_lists SET name = $3, modified_at = now() WHERE org_id = $1 AND datadog_id = $2").bind(&ctx.org_id).bind(list_id(&id)?).bind(&name).execute(&st.postgres).await?;
    if done.rows_affected() == 0 {
        return Err(not_found(&id));
    }
    Ok(Json(load(&st, &ctx.org_id, &id).await?.datadog()))
}

/// Deletes a dashboard list; its dashboards stay (Datadog-compatible).
#[utoipa::path(delete, path = "/api/v1/dashboard/lists/manual/{id}", operation_id = "datadog_delete_dashboard_list", params(("id" = i64, Path)),
    responses((status = 200, body = DatadogDeletedDashboardList), (status = 404, body = super::DatadogErrors)), tag = "datadog-api")]
pub async fn delete_one(State(st): State<AppState>, DdAuth(ctx): DdAuth, Path(id): Path<String>) -> Result<Json<DatadogDeletedDashboardList>, DdError> {
    writer(&ctx)?;
    let list = list_id(&id)?;
    let done = sqlx::query("DELETE FROM dashboard_lists WHERE org_id = $1 AND datadog_id = $2").bind(&ctx.org_id).bind(list).execute(&st.postgres).await?;
    if done.rows_affected() == 0 {
        return Err(not_found(&id));
    }
    Ok(Json(DatadogDeletedDashboardList { deleted_dashboard_list_id: list }))
}

// ================================================================ v2 items

#[derive(sqlx::FromRow)]
struct ItemRow {
    id: String,
    title: String,
    kind: String,
    free: bool,
    view_count: i32,
    author_handle: Option<String>,
    author_name: Option<String>,
    created_at: i64,
    modified_at: i64,
}

impl ItemRow {
    fn item_type(&self) -> String {
        let board = if self.free { "screenboard" } else { "timeboard" };
        format!("{}_{board}", if self.kind == "integration" { "integration" } else { "custom" })
    }
}

/// The org's live dashboards among `ids`, in `ids` order.
async fn dashboards(st: &AppState, org: &str, ids: &[String]) -> Result<Vec<ItemRow>, DdError> {
    let rows: Vec<ItemRow> = sqlx::query_as(
        "SELECT d.id, d.title, d.kind, d.definition->>'layoutType' = 'free' AS free, d.view_count, u.email AS author_handle, u.name AS author_name, \
         (extract(epoch FROM d.created_at) * 1000)::bigint AS created_at, (extract(epoch FROM d.modified_at) * 1000)::bigint AS modified_at \
         FROM dashboards d LEFT JOIN users u ON u.id = d.author_id WHERE d.org_id = $1 AND d.id = ANY($2) AND d.deleted_at IS NULL",
    )
    .bind(org)
    .bind(ids)
    .fetch_all(&st.postgres)
    .await?;
    let mut ordered: Vec<ItemRow> = Vec::with_capacity(rows.len());
    let mut rows = rows;
    for id in ids {
        if let Some(i) = rows.iter().position(|r| &r.id == id) {
            ordered.push(rows.swap_remove(i));
        }
    }
    Ok(ordered)
}

fn item_refs(rows: &[ItemRow]) -> Vec<DatadogDashboardListItemRef> {
    rows.iter().map(|r| DatadogDashboardListItemRef { id: r.id.clone(), kind: r.item_type() }).collect()
}

/// The items of a request; every dashboard must be one of the org's.
async fn requested(st: &AppState, org: &str, body: &Bytes, must_exist: bool) -> Result<Vec<ItemRow>, DdError> {
    let req: DatadogDashboardListItemsRequest = serde_json::from_value(parse_body(body)?).map_err(|e| DdError::new(StatusCode::BAD_REQUEST, format!("Invalid body: {e}")))?;
    if let Some(bad) = req.dashboards.iter().find(|d| !ITEM_TYPES.contains(&d.kind.as_str())) {
        return Err(DdError::new(StatusCode::BAD_REQUEST, format!("dashboards: type {:?} of {} must be one of {}", bad.kind, bad.id, ITEM_TYPES.join(", "))));
    }
    let ids: Vec<String> = req.dashboards.into_iter().map(|d| d.id).collect();
    let rows = dashboards(st, org, &ids).await?;
    if must_exist {
        if let Some(absent) = ids.iter().find(|id| !rows.iter().any(|r| &r.id == *id)) {
            return Err(DdError::new(StatusCode::NOT_FOUND, format!("Dashboard with ID {absent} not found")));
        }
    }
    Ok(rows)
}

/// The dashboards of a list (Datadog-compatible).
#[utoipa::path(get, path = "/api/v2/dashboard/lists/manual/{id}/dashboards", operation_id = "datadog_get_dashboard_list_items", params(("id" = i64, Path)),
    responses((status = 200, body = DatadogDashboardListItems), (status = 404, body = super::DatadogErrors)), tag = "datadog-api")]
pub async fn items(State(st): State<AppState>, DdAuth(ctx): DdAuth, Path(id): Path<String>) -> Result<Json<DatadogDashboardListItems>, DdError> {
    let list = load(&st, &ctx.org_id, &id).await?;
    let rows = dashboards(&st, &ctx.org_id, &list.dashboard_ids).await?;
    let dashboards: Vec<Value> = rows
        .iter()
        .map(|r| {
            json!({
                "id": r.id, "type": r.item_type(), "title": r.title, "url": url(&r.id),
                "author": { "handle": r.author_handle.clone().unwrap_or_default(), "name": r.author_name },
                "created": iso(r.created_at), "modified": iso(r.modified_at), "popularity": crate::routes::dashboards::popularity(r.view_count),
                "is_read_only": false, "is_shared": false, "is_favorite": false, "icon": null, "integration_id": null,
            })
        })
        .collect();
    Ok(Json(DatadogDashboardListItems { total: dashboards.len() as i64, dashboards }))
}

async fn set_items(st: &AppState, org: &str, list: &ListRow, ids: Vec<String>) -> Result<(), DdError> {
    sqlx::query("UPDATE dashboard_lists SET dashboard_ids = $3, modified_at = now() WHERE org_id = $1 AND id = $2").bind(org).bind(&list.id).bind(&ids).execute(&st.postgres).await?;
    Ok(())
}

/// Adds dashboards to a list (Datadog-compatible).
#[utoipa::path(post, path = "/api/v2/dashboard/lists/manual/{id}/dashboards", operation_id = "datadog_add_dashboard_list_items", params(("id" = i64, Path)), request_body = DatadogDashboardListItemsRequest,
    responses((status = 200, body = DatadogAddedDashboardListItems), (status = 400, body = super::DatadogErrors), (status = 404, body = super::DatadogErrors)), tag = "datadog-api")]
pub async fn add_items(State(st): State<AppState>, DdAuth(ctx): DdAuth, Path(id): Path<String>, body: Bytes) -> Result<Json<DatadogAddedDashboardListItems>, DdError> {
    writer(&ctx)?;
    let list = load(&st, &ctx.org_id, &id).await?;
    let rows = requested(&st, &ctx.org_id, &body, true).await?;
    let mut ids = list.dashboard_ids.clone();
    for r in &rows {
        if !ids.contains(&r.id) {
            ids.push(r.id.clone());
        }
    }
    set_items(&st, &ctx.org_id, &list, ids).await?;
    Ok(Json(DatadogAddedDashboardListItems { added_dashboards_to_list: item_refs(&rows) }))
}

/// Replaces the dashboards of a list (Datadog-compatible).
#[utoipa::path(put, path = "/api/v2/dashboard/lists/manual/{id}/dashboards", operation_id = "datadog_update_dashboard_list_items", params(("id" = i64, Path)), request_body = DatadogDashboardListItemsRequest,
    responses((status = 200, body = DatadogUpdatedDashboardListItems), (status = 400, body = super::DatadogErrors), (status = 404, body = super::DatadogErrors)), tag = "datadog-api")]
pub async fn replace_items(State(st): State<AppState>, DdAuth(ctx): DdAuth, Path(id): Path<String>, body: Bytes) -> Result<Json<DatadogUpdatedDashboardListItems>, DdError> {
    writer(&ctx)?;
    let list = load(&st, &ctx.org_id, &id).await?;
    let rows = requested(&st, &ctx.org_id, &body, true).await?;
    set_items(&st, &ctx.org_id, &list, rows.iter().map(|r| r.id.clone()).collect()).await?;
    Ok(Json(DatadogUpdatedDashboardListItems { dashboards: item_refs(&rows) }))
}

/// Removes dashboards from a list (Datadog-compatible); returns those that were in it.
#[utoipa::path(delete, path = "/api/v2/dashboard/lists/manual/{id}/dashboards", operation_id = "datadog_delete_dashboard_list_items", params(("id" = i64, Path)), request_body = DatadogDashboardListItemsRequest,
    responses((status = 200, body = DatadogDeletedDashboardListItems), (status = 400, body = super::DatadogErrors), (status = 404, body = super::DatadogErrors)), tag = "datadog-api")]
pub async fn remove_items(State(st): State<AppState>, DdAuth(ctx): DdAuth, Path(id): Path<String>, body: Bytes) -> Result<Json<DatadogDeletedDashboardListItems>, DdError> {
    writer(&ctx)?;
    let list = load(&st, &ctx.org_id, &id).await?;
    let rows = requested(&st, &ctx.org_id, &body, false).await?;
    let removed: Vec<ItemRow> = rows.into_iter().filter(|r| list.dashboard_ids.contains(&r.id)).collect();
    let ids = list.dashboard_ids.iter().filter(|id| !removed.iter().any(|r| &r.id == *id)).cloned().collect();
    set_items(&st, &ctx.org_id, &list, ids).await?;
    Ok(Json(DatadogDeletedDashboardListItems { deleted_dashboards_from_list: item_refs(&removed) }))
}

#[cfg(test)]
mod tests {
    use super::super::dashboards::tests::{adapt, call, ignored, terraform_dashboard};
    use super::super::replay::{new_org_with_roles, user_with_app_key, Feature, TestServer};
    use serde_json::{json, Value};

    #[tokio::test]
    async fn replays_the_dashboard_lists_features() {
        let (Some(v1), Some(v2)) = (Feature::load("v1/dashboard_lists"), Feature::load("v2/dashboard_lists")) else { return };
        let Some(s) = TestServer::start("dd-dash-lists").await else { return };
        let (mut failures, mut replayed) = (vec![], 0);
        for (path, feature) in [("v1/dashboard_lists", v1), ("v2/dashboard_lists", v2)] {
            for scenario in &feature.scenarios {
                let Some(interactions) = feature.cassette(scenario) else { continue };
                let mut r = s.replayer();
                r.ignore = ignored();
                // The v2 scenarios' fixture dashboards have a profile_metrics_query timeseries,
                // which we don't render; the lists are the subject here.
                let (_, problems) = r.replay_interactions(feature.sends(scenario), &scenario.name, &adapt(&interactions)).await;
                replayed += 1;
                failures.extend(problems.into_iter().map(|p| format!("{path}: {p}")));
            }
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
        assert_eq!(replayed, 14, "every recorded scenario");
    }

    #[tokio::test]
    async fn lists_are_shared_with_the_ui() {
        let Some(s) = TestServer::start("dd-dash-lists-ui").await else { return };
        let api = |p: &str| format!("{}{p}", s.base);
        let (_, ordered) = call("POST", &api("/api/v1/dashboard"), &s.app_key, true, Some(terraform_dashboard())).await;
        let mut free = terraform_dashboard();
        free["layout_type"] = json!("free");
        free.as_object_mut().unwrap().remove("reflow_type");
        let (_, free) = call("POST", &api("/api/v1/dashboard"), &s.app_key, true, Some(free)).await;
        let (ordered, free) = (ordered["id"].as_str().unwrap().to_string(), free["id"].as_str().unwrap().to_string());

        let (status, list) = call("POST", &api("/api/v1/dashboard/lists/manual"), &s.app_key, true, Some(json!({ "name": "Checkout" }))).await;
        assert_eq!(status, 200, "{list}");
        let id = list["id"].as_i64().unwrap();
        assert_eq!((list["type"].as_str(), list["dashboard_count"].as_i64(), list["dashboards"].clone()), (Some("manual_dashboard_list"), Some(0), Value::Null));
        assert!(list["author"]["handle"].as_str().unwrap().ends_with("@example.com"));
        let items = api(&format!("/api/v2/dashboard/lists/manual/{id}/dashboards"));
        let (status, added) =
            call("POST", &items, &s.app_key, true, Some(json!({ "dashboards": [{ "id": ordered, "type": "custom_timeboard" }, { "id": free, "type": "custom_screenboard" }] }))).await;
        assert_eq!(status, 200);
        assert_eq!(added, json!({ "added_dashboards_to_list": [{ "id": ordered, "type": "custom_timeboard" }, { "id": free, "type": "custom_screenboard" }] }));

        // The UI's lists show it, with its dashboards.
        let (_, ui_lists) = call("GET", &api("/api/v1/dashboard-lists"), &s.app_key, false, None).await;
        assert_eq!(ui_lists[0]["name"], "Checkout");
        assert_eq!(ui_lists[0]["dashboardIds"], json!([ordered, free]));
        let (_, page) = call(
            "POST",
            &api("/api/v1/dashboards/search"),
            &s.app_key,
            false,
            Some(json!({ "preset": "all", "listId": ui_lists[0]["id"], "sort": { "field": "title", "dir": "asc" }, "offset": 0, "limit": 10 })),
        )
        .await;
        assert_eq!(page["total"], 2);

        let (_, got) = call("GET", &items, &s.app_key, true, None).await;
        assert_eq!(got["total"], 2);
        assert_eq!((got["dashboards"][0]["id"].as_str(), got["dashboards"][0]["type"].as_str()), (Some(ordered.as_str()), Some("custom_timeboard")));
        assert_eq!(got["dashboards"][0]["title"], "Checkout service");
        assert_eq!(got["dashboards"][1]["type"], "custom_screenboard");

        // What the provider does on update: read, remove everything, put the configured items.
        let all = json!({ "dashboards": got["dashboards"].as_array().unwrap().iter().map(|d| json!({ "id": d["id"], "type": d["type"] })).collect::<Vec<_>>() });
        assert_eq!(call("DELETE", &items, &s.app_key, true, Some(all.clone())).await.1, json!({ "deleted_dashboards_from_list": all["dashboards"] }));
        assert_eq!(call("DELETE", &items, &s.app_key, true, Some(json!({ "dashboards": [] }))).await, (200, json!({ "deleted_dashboards_from_list": [] })));
        let (_, put) = call("PUT", &items, &s.app_key, true, Some(json!({ "dashboards": [{ "id": free, "type": "custom_screenboard" }] }))).await;
        assert_eq!(put, json!({ "dashboards": [{ "id": free, "type": "custom_screenboard" }] }));
        assert_eq!(call("GET", &api(&format!("/api/v1/dashboard/lists/manual/{id}")), &s.app_key, true, None).await.1["dashboard_count"], 1);

        let (status, err) = call("POST", &items, &s.app_key, true, Some(json!({ "dashboards": [{ "id": "nop-nop-nop", "type": "custom_timeboard" }] }))).await;
        assert_eq!((status, err["errors"][0].as_str()), (404, Some("Dashboard with ID nop-nop-nop not found")));
        assert_eq!(call("POST", &items, &s.app_key, true, Some(json!({ "dashboards": [{ "id": free, "type": "dashboard" }] }))).await.0, 400);

        let (_, renamed) = call("PUT", &api(&format!("/api/v1/dashboard/lists/manual/{id}")), &s.app_key, true, Some(json!({ "name": "Checkout team" }))).await;
        assert_eq!(renamed["name"], "Checkout team");
        let (_, lists) = call("GET", &api("/api/v1/dashboard/lists/manual"), &s.app_key, true, None).await;
        assert_eq!(lists["dashboard_lists"].as_array().map(Vec::len), Some(1));

        // Other orgs and read-only users.
        let other = new_org_with_roles(&s.st, "dd-lists-other").await;
        let (_, other_key) = user_with_app_key(&s.st, &other, "admin").await;
        assert_eq!(call("GET", &api(&format!("/api/v1/dashboard/lists/manual/{id}")), &other_key, true, None).await.0, 404);
        assert_eq!(call("GET", &items, &other_key, true, None).await.0, 404);
        assert_eq!(call("GET", &api("/api/v1/dashboard/lists/manual"), &other_key, true, None).await.1, json!({ "dashboard_lists": [] }));
        let (_, reader) = user_with_app_key(&s.st, &s.org, "readonly").await;
        assert_eq!(call("PUT", &items, &reader, true, Some(json!({ "dashboards": [] }))).await.0, 403);
        assert_eq!(call("DELETE", &api(&format!("/api/v1/dashboard/lists/manual/{id}")), &reader, true, None).await.0, 403);

        assert_eq!(call("DELETE", &api(&format!("/api/v1/dashboard/lists/manual/{id}")), &s.app_key, true, None).await, (200, json!({ "deleted_dashboard_list_id": id })));
        assert_eq!(call("GET", &api(&format!("/api/v1/dashboard/lists/manual/{id}")), &s.app_key, true, None).await.0, 404);
        assert_eq!(call("GET", &api("/api/v1/dashboard/lists/manual/not-a-number"), &s.app_key, true, None).await.0, 404);
        // The dashboards stay.
        assert_eq!(call("GET", &api(&format!("/api/v1/dashboard/{free}")), &s.app_key, true, None).await.0, 200);
    }
}
