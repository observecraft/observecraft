//! The Slack integration's channels, Datadog-compatible
//! (`/api/v1/integration/slack/configuration/accounts/{account_name}/channels[/{channel_name}]`, the
//! Terraform provider's `datadog_integration_slack_channel`), on Postgres `slack_channels`
//! ([`crate::routes::slack`]).
//!
//! - The account is a workspace connected in Integrations → Slack (Datadog's API doesn't create
//!   them either); an unknown account answers 404.
//! - A channel is `{"name": "#…", "display": {message, notified, snapshot, tags, mute_buttons}}`;
//!   missing display options get Datadog's defaults (true, but `mute_buttons` false) on create and keep
//!   their value on update. The path's channel name matches with or without its `#`, ignoring case.
//! - Reads need any role, writes an admin (as notification channels). Errors are `{"errors": ["…"]}`.

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use serde_json::{json, Map, Value};

use super::users::require;
use super::{DdAuth, DdError};
use crate::auth::Access;
use crate::routes::slack::{self, AccountRow, ChannelError, ChannelRow, Display};
use crate::state::AppState;

fn bad(msg: impl Into<String>) -> DdError {
    DdError::new(StatusCode::BAD_REQUEST, msg)
}

fn not_found(what: &str) -> DdError {
    DdError::new(StatusCode::NOT_FOUND, what)
}

impl From<ChannelError> for DdError {
    fn from(e: ChannelError) -> Self {
        match e {
            ChannelError::Invalid(m) | ChannelError::Duplicate(m) => bad(m),
            ChannelError::NotFound => not_found("Slack channel not found"),
            ChannelError::Db(e) => e.into(),
        }
    }
}

/// A channel as Datadog's API answers it.
pub fn channel_doc(c: &ChannelRow) -> Value {
    json!({ "name": c.name, "display": c.display() })
}

fn body(raw: &Bytes) -> Result<Map<String, Value>, DdError> {
    match serde_json::from_slice::<Value>(raw) {
        Ok(Value::Object(m)) => Ok(m),
        _ => Err(bad("The request body must be a JSON object")),
    }
}

/// The request's channel name (required on create, optional on update).
fn name_of(b: &Map<String, Value>) -> Result<Option<String>, DdError> {
    match b.get("name") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => slack::channel_name(s).map(Some).map_err(bad),
        Some(_) => Err(bad("name must be a string")),
    }
}

async fn account(st: &AppState, org: &str, name: &str) -> Result<AccountRow, DdError> {
    slack::account_by_name(st, org, name).await?.ok_or_else(|| not_found("Slack account not found: connect the workspace in Integrations → Slack"))
}

async fn channel(st: &AppState, org: &str, account_name: &str, channel_name: &str) -> Result<(AccountRow, ChannelRow), DdError> {
    let a = account(st, org, account_name).await?;
    let c = slack::channel_by_name(st, org, &a.id, channel_name).await?.ok_or_else(|| not_found("Slack channel not found"))?;
    Ok((a, c))
}

#[utoipa::path(get, path = "/api/v1/integration/slack/configuration/accounts/{account_name}/channels", operation_id = "datadog_list_slack_channels",
    params(("account_name" = String, Path)), responses((status = 200, body = Object), (status = 404, body = Object)), tag = "datadog-api")]
pub async fn list(State(st): State<AppState>, DdAuth(ctx): DdAuth, Path(account_name): Path<String>) -> Result<Json<Value>, DdError> {
    let a = account(&st, &ctx.org_id, &account_name).await?;
    let channels = slack::channels(&st, &ctx.org_id, &a.id).await?;
    Ok(Json(Value::Array(channels.iter().map(channel_doc).collect())))
}

#[utoipa::path(post, path = "/api/v1/integration/slack/configuration/accounts/{account_name}/channels", operation_id = "datadog_create_slack_channel",
    params(("account_name" = String, Path)), request_body = Object, responses((status = 200, body = Object), (status = 400, body = Object), (status = 404, body = Object)), tag = "datadog-api")]
pub async fn create(State(st): State<AppState>, DdAuth(ctx): DdAuth, Path(account_name): Path<String>, raw: Bytes) -> Result<Json<Value>, DdError> {
    require(&ctx, Access::Admin)?;
    let b = body(&raw)?;
    let name = name_of(&b)?.ok_or_else(|| bad("name is required"))?;
    let display = Display::merged(Display::default(), b.get("display")).map_err(bad)?;
    let a = account(&st, &ctx.org_id, &account_name).await?;
    let c = slack::create_channel(&st, &ctx.org_id, &a.id, &name, display).await?;
    ctx.audit(&st, "slack_channel.created", &slack::handle(&a.name, &c.name), "datadog api").await;
    Ok(Json(channel_doc(&c)))
}

#[utoipa::path(get, path = "/api/v1/integration/slack/configuration/accounts/{account_name}/channels/{channel_name}", operation_id = "datadog_get_slack_channel",
    params(("account_name" = String, Path), ("channel_name" = String, Path)), responses((status = 200, body = Object), (status = 404, body = Object)), tag = "datadog-api")]
pub async fn get(State(st): State<AppState>, DdAuth(ctx): DdAuth, Path((account_name, channel_name)): Path<(String, String)>) -> Result<Json<Value>, DdError> {
    let (_, c) = channel(&st, &ctx.org_id, &account_name, &channel_name).await?;
    Ok(Json(channel_doc(&c)))
}

#[utoipa::path(patch, path = "/api/v1/integration/slack/configuration/accounts/{account_name}/channels/{channel_name}", operation_id = "datadog_update_slack_channel",
    params(("account_name" = String, Path), ("channel_name" = String, Path)), request_body = Object, responses((status = 200, body = Object), (status = 400, body = Object), (status = 404, body = Object)), tag = "datadog-api")]
pub async fn update(State(st): State<AppState>, DdAuth(ctx): DdAuth, Path((account_name, channel_name)): Path<(String, String)>, raw: Bytes) -> Result<Json<Value>, DdError> {
    require(&ctx, Access::Admin)?;
    let b = body(&raw)?;
    let new_name = name_of(&b)?;
    let (a, c) = channel(&st, &ctx.org_id, &account_name, &channel_name).await?;
    let display = Display::merged(c.display(), b.get("display")).map_err(bad)?;
    let c = slack::update_channel(&st, &ctx.org_id, &c, new_name.as_deref(), display).await?;
    ctx.audit(&st, "slack_channel.updated", &slack::handle(&a.name, &c.name), "datadog api").await;
    Ok(Json(channel_doc(&c)))
}

#[utoipa::path(delete, path = "/api/v1/integration/slack/configuration/accounts/{account_name}/channels/{channel_name}", operation_id = "datadog_delete_slack_channel",
    params(("account_name" = String, Path), ("channel_name" = String, Path)), responses((status = 204), (status = 404, body = Object)), tag = "datadog-api")]
pub async fn delete(State(st): State<AppState>, DdAuth(ctx): DdAuth, Path((account_name, channel_name)): Path<(String, String)>) -> Result<StatusCode, DdError> {
    require(&ctx, Access::Admin)?;
    let (a, c) = channel(&st, &ctx.org_id, &account_name, &channel_name).await?;
    slack::delete_channel(&st, &ctx.org_id, &c.id).await?;
    ctx.audit(&st, "slack_channel.deleted", &slack::handle(&a.name, &c.name), "datadog api").await;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::routes::datadog_api::replay::TestServer;

    async fn call(s: &TestServer, method: &str, path: &str, body: Option<Value>) -> (u16, Value) {
        let mut req = reqwest::Client::new().request(method.parse().unwrap(), format!("{}{path}", s.base)).header("DD-API-KEY", &s.api_key).header("DD-APPLICATION-KEY", &s.app_key);
        if let Some(b) = body {
            req = req.json(&b);
        }
        let r = req.send().await.unwrap();
        let status = r.status().as_u16();
        (status, r.json().await.unwrap_or(Value::Null))
    }

    #[tokio::test]
    async fn channels_through_datadogs_api() {
        let Some(s) = TestServer::start("dd-slack").await else { return };
        // Workspaces are connected in the UI; here, straight into the table.
        sqlx::query("INSERT INTO slack_accounts (org_id, name, team_id, bot_token_enc, installed_via) VALUES ($1, 'example-shop', 'T0EXAMPLE', $2, 'token')")
            .bind(&s.org)
            .bind(s.st.secrets.encrypt("xoxb-test"))
            .execute(&s.st.postgres)
            .await
            .unwrap();
        let base = "/api/v1/integration/slack/configuration/accounts/example-shop/channels";

        let (status, _) = call(&s, "GET", "/api/v1/integration/slack/configuration/accounts/nowhere/channels", None).await;
        assert_eq!(status, 404);
        let (status, created) = call(&s, "POST", base, Some(json!({ "name": "#checkout-alerts", "display": { "mute_buttons": true, "snapshot": false } }))).await;
        assert_eq!(status, 200, "{created}");
        assert_eq!(created, json!({ "name": "#checkout-alerts", "display": { "message": true, "notified": true, "snapshot": false, "tags": true, "mute_buttons": true } }));
        let (status, _) = call(&s, "POST", base, Some(json!({ "name": "#Checkout-Alerts" }))).await;
        assert_eq!(status, 400);
        let (status, _) = call(&s, "POST", base, Some(json!({ "display": {} }))).await;
        assert_eq!(status, 400);

        // The provider sends the name with its '#' (escaped); without works too.
        let (status, got) = call(&s, "GET", &format!("{base}/%23checkout-alerts"), None).await;
        assert_eq!((status, &got["name"]), (200, &json!("#checkout-alerts")));
        let (status, _) = call(&s, "GET", &format!("{base}/checkout-alerts"), None).await;
        assert_eq!(status, 200);
        let (status, updated) = call(&s, "PATCH", &format!("{base}/%23checkout-alerts"), Some(json!({ "name": "#checkout-alerts", "display": { "tags": false } }))).await;
        assert_eq!(status, 200);
        assert_eq!(updated["display"], json!({ "message": true, "notified": true, "snapshot": false, "tags": false, "mute_buttons": true }));
        let (status, all) = call(&s, "GET", base, None).await;
        assert_eq!((status, all.as_array().map(Vec::len)), (200, Some(1)));
        let (status, _) = call(&s, "DELETE", &format!("{base}/%23checkout-alerts"), None).await;
        assert_eq!(status, 204);
        let (status, _) = call(&s, "GET", &format!("{base}/%23checkout-alerts"), None).await;
        assert_eq!(status, 404);
    }
}
