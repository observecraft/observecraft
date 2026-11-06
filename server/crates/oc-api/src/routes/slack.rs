//! The Slack integration: workspaces connected through a Slack app, and the channels monitors (and
//! On-Call, workflows, reports, surveys) post to.
//!
//! - **Workspaces** (`slack_accounts`) are connected with "Add to Slack" through this install's Slack
//!   app (OAuth v2; `OC_SLACK_CLIENT_ID`, `OC_SLACK_CLIENT_SECRET`, `OC_SLACK_SIGNING_SECRET`), or with
//!   the bot token of the workspace's own app, created from the manifest [`app_manifest`] gives
//!   (self-hosted installs without a shared app). The account name is Datadog's `account_name`.
//! - **Channels** (`slack_channels`) are Datadog's `datadog_integration_slack_channel`
//!   (`datadog_api::slack_integration`): a name and the display options `message`, `notified`,
//!   `snapshot`, `tags`, `mute_buttons`. Monitors mention them as `@slack-<account>-<channel>`, or
//!   `@slack-<channel>` when the channel name is unique across the org's workspaces (else the oldest
//!   workspace's). Handles that match no configured channel fall back to incoming-webhook channels
//!   (`notification_channels`, kind `slack`).
//! - **Messages** are posted with `chat.postMessage` as the bot: monitor transitions as a card (status
//!   color, title linking to the monitor, message, scope and tags, notified handles, a "View monitor"
//!   button and, with `mute_buttons`, Mute 1h / 4h / 1d). Snapshots are stored but not drawn (no graph
//!   renderer on the server yet).
//! - **Mute buttons** come back on `POST /api/v1/public/slack/interactive`, verified with the app's
//!   signing secret. The Slack user is matched by email (`users.info`) to an active member of the
//!   organization with at least the Standard role; the mute is a downtime on the monitor (and the
//!   alerting group), audited as that member.

use std::time::Duration;

use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Redirect, Response};
use axum::Json;
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::Sha256;
use utoipa::ToSchema;

use super::monitor_engine::{parse_group_key, recipients};
use super::notifications::{Notice, Outcome, Outgoing};
use crate::auth::{access_from_roles, crypto, Access, Admin, AuthContext};
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;

/// The bot scopes the app asks for: post (also to public channels it isn't in), list channels for the
/// picker and resolve names, and read the email of whoever presses a mute button.
pub const BOT_SCOPES: [&str; 6] = ["chat:write", "chat:write.public", "channels:read", "groups:read", "users:read", "users:read.email"];
/// Mute durations of the buttons, in hours.
const MUTE_HOURS: [i64; 3] = [1, 4, 24];

// ================================================================ display options

/// What a monitor notification shows (Datadog's `SlackIntegrationChannelDisplay`).
#[derive(Serialize, Deserialize, ToSchema, Clone, Copy, Debug, PartialEq, Eq)]
pub struct Display {
    /// The monitor's message.
    pub message: bool,
    /// The @-handles the message notified.
    pub notified: bool,
    /// A graph snapshot (stored; not drawn yet).
    pub snapshot: bool,
    /// The scope that alerted and the monitor's tags.
    pub tags: bool,
    /// Buttons to mute the monitor from Slack.
    pub mute_buttons: bool,
}

impl Default for Display {
    fn default() -> Self {
        Display { message: true, notified: true, snapshot: true, tags: true, mute_buttons: false }
    }
}

impl Display {
    /// `base` with the booleans `v` carries (absent or null keep `base`'s).
    pub fn merged(base: Display, v: Option<&Value>) -> Result<Display, String> {
        let mut d = base;
        let Some(v) = v.filter(|v| !v.is_null()) else { return Ok(d) };
        let Some(o) = v.as_object() else { return Err("display must be an object".into()) };
        for (key, slot) in [("message", &mut d.message), ("notified", &mut d.notified), ("snapshot", &mut d.snapshot), ("tags", &mut d.tags), ("mute_buttons", &mut d.mute_buttons)] {
            match o.get(key) {
                None | Some(Value::Null) => {}
                Some(Value::Bool(b)) => *slot = *b,
                Some(_) => return Err(format!("display.{key} must be a boolean")),
            }
        }
        Ok(d)
    }
    fn from_json(v: &Value) -> Display {
        Display::merged(Display::default(), Some(v)).unwrap_or_default()
    }
}

// ================================================================ names

/// Account names: what `@slack-<account>-…` and Datadog's `account_name` carry.
pub fn account_name(raw: &str) -> Result<String, String> {
    let n = raw.trim().to_lowercase();
    if n.is_empty() || n.len() > 60 || !n.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_') {
        return Err("Account names are 1–60 lowercase letters, digits, dashes or underscores".into());
    }
    Ok(n)
}

/// A channel name as configured (`#checkout-alerts` or `checkout-alerts`), checked against Slack's
/// rules; returns it trimmed, as given.
pub fn channel_name(raw: &str) -> Result<String, String> {
    let n = raw.trim();
    let bare = n.strip_prefix('#').unwrap_or(n);
    if bare.is_empty() || bare.chars().count() > 80 || !bare.chars().all(|c| c.is_alphanumeric() || matches!(c, '-' | '_' | '.')) {
        return Err("Channel names are 1–80 letters, digits, dashes, underscores or periods, with an optional leading #".into());
    }
    Ok(n.to_string())
}

/// The comparable form of a channel name: no '#', lowercase.
pub fn bare(name: &str) -> String {
    name.trim().trim_start_matches('#').to_lowercase()
}

/// The handle monitors mention.
pub fn handle(account: &str, channel: &str) -> String {
    format!("slack-{account}-{}", bare(channel))
}

// ================================================================ storage

#[derive(sqlx::FromRow, Debug, Clone)]
pub struct AccountRow {
    pub id: String,
    pub org_id: String,
    pub name: String,
    pub team_id: String,
    pub team_name: String,
    pub bot_user_id: String,
    pub bot_token_enc: String,
    pub signing_secret_enc: Option<String>,
    pub scopes: Vec<String>,
    pub installed_via: String,
    pub created_at: i64,
}

const ACCOUNT_SELECT: &str = "SELECT id::text AS id, org_id, name, team_id, team_name, bot_user_id, bot_token_enc, signing_secret_enc, scopes, installed_via, \
    (extract(epoch FROM created_at) * 1000)::bigint AS created_at FROM slack_accounts";

#[derive(sqlx::FromRow, Debug, Clone)]
pub struct ChannelRow {
    pub id: String,
    pub account_id: String,
    pub name: String,
    pub channel_id: Option<String>,
    pub display: sqlx::types::Json<Value>,
    pub created_at: i64,
}

impl ChannelRow {
    pub fn display(&self) -> Display {
        Display::from_json(&self.display.0)
    }
}

const CHANNEL_SELECT: &str = "SELECT id::text AS id, account_id::text AS account_id, name, channel_id, display, (extract(epoch FROM created_at) * 1000)::bigint AS created_at FROM slack_channels";

pub async fn accounts(st: &AppState, org: &str) -> Result<Vec<AccountRow>, sqlx::Error> {
    sqlx::query_as(sqlx::AssertSqlSafe(format!("{ACCOUNT_SELECT} WHERE org_id = $1 ORDER BY created_at, name"))).bind(org).fetch_all(&st.postgres).await
}

pub async fn account_by_name(st: &AppState, org: &str, name: &str) -> Result<Option<AccountRow>, sqlx::Error> {
    sqlx::query_as(sqlx::AssertSqlSafe(format!("{ACCOUNT_SELECT} WHERE org_id = $1 AND name = $2"))).bind(org).bind(name.trim().to_lowercase()).fetch_optional(&st.postgres).await
}

async fn account_by_id(st: &AppState, org: &str, id: &str) -> Result<Option<AccountRow>, sqlx::Error> {
    sqlx::query_as(sqlx::AssertSqlSafe(format!("{ACCOUNT_SELECT} WHERE org_id = $1 AND id::text = $2"))).bind(org).bind(id).fetch_optional(&st.postgres).await
}

pub async fn channels(st: &AppState, org: &str, account_id: &str) -> Result<Vec<ChannelRow>, sqlx::Error> {
    sqlx::query_as(sqlx::AssertSqlSafe(format!("{CHANNEL_SELECT} WHERE org_id = $1 AND account_id::text = $2 ORDER BY lower(ltrim(name, '#'))")))
        .bind(org)
        .bind(account_id)
        .fetch_all(&st.postgres)
        .await
}

async fn all_channels(st: &AppState, org: &str) -> Result<Vec<ChannelRow>, sqlx::Error> {
    sqlx::query_as(sqlx::AssertSqlSafe(format!("{CHANNEL_SELECT} WHERE org_id = $1 ORDER BY lower(ltrim(name, '#'))"))).bind(org).fetch_all(&st.postgres).await
}

pub async fn channel_by_name(st: &AppState, org: &str, account_id: &str, name: &str) -> Result<Option<ChannelRow>, sqlx::Error> {
    sqlx::query_as(sqlx::AssertSqlSafe(format!("{CHANNEL_SELECT} WHERE org_id = $1 AND account_id::text = $2 AND lower(ltrim(name, '#')) = $3")))
        .bind(org)
        .bind(account_id)
        .bind(bare(name))
        .fetch_optional(&st.postgres)
        .await
}

async fn channel_by_id(st: &AppState, org: &str, id: &str) -> Result<Option<ChannelRow>, sqlx::Error> {
    sqlx::query_as(sqlx::AssertSqlSafe(format!("{CHANNEL_SELECT} WHERE org_id = $1 AND id::text = $2"))).bind(org).bind(id).fetch_optional(&st.postgres).await
}

/// Why a channel write failed.
#[derive(Debug)]
pub enum ChannelError {
    Invalid(String),
    Duplicate(String),
    NotFound,
    Db(sqlx::Error),
}

impl From<sqlx::Error> for ChannelError {
    fn from(e: sqlx::Error) -> Self {
        ChannelError::Db(e)
    }
}

impl From<ChannelError> for ApiError {
    fn from(e: ChannelError) -> Self {
        match e {
            ChannelError::Invalid(m) => ApiError::BadRequest(m),
            ChannelError::Duplicate(m) => ApiError::Conflict(m),
            ChannelError::NotFound => ApiError::NotFound("Slack channel not found".into()),
            ChannelError::Db(e) => e.into(),
        }
    }
}

fn unique(e: sqlx::Error, name: &str) -> ChannelError {
    match &e {
        sqlx::Error::Database(d) if d.is_unique_violation() => ChannelError::Duplicate(format!("{name} is already configured for this Slack account")),
        _ => ChannelError::Db(e),
    }
}

pub async fn create_channel(st: &AppState, org: &str, account_id: &str, name: &str, display: Display) -> Result<ChannelRow, ChannelError> {
    let name = channel_name(name).map_err(ChannelError::Invalid)?;
    let id: (String,) = sqlx::query_as("INSERT INTO slack_channels (org_id, account_id, name, display) VALUES ($1, $2::uuid, $3, $4) RETURNING id::text")
        .bind(org)
        .bind(account_id)
        .bind(&name)
        .bind(sqlx::types::Json(json!(display)))
        .fetch_one(&st.postgres)
        .await
        .map_err(|e| unique(e, &name))?;
    channel_by_id(st, org, &id.0).await?.ok_or(ChannelError::NotFound)
}

/// Renames (the Slack conversation id is resolved again) and/or changes the display options.
pub async fn update_channel(st: &AppState, org: &str, current: &ChannelRow, new_name: Option<&str>, display: Display) -> Result<ChannelRow, ChannelError> {
    let name = match new_name {
        Some(n) => channel_name(n).map_err(ChannelError::Invalid)?,
        None => current.name.clone(),
    };
    let renamed = bare(&name) != bare(&current.name);
    sqlx::query("UPDATE slack_channels SET name = $3, display = $4, channel_id = CASE WHEN $5 THEN NULL ELSE channel_id END, updated_at = now() WHERE org_id = $1 AND id::text = $2")
        .bind(org)
        .bind(&current.id)
        .bind(&name)
        .bind(sqlx::types::Json(json!(display)))
        .bind(renamed)
        .execute(&st.postgres)
        .await
        .map_err(|e| unique(e, &name))?;
    channel_by_id(st, org, &current.id).await?.ok_or(ChannelError::NotFound)
}

pub async fn delete_channel(st: &AppState, org: &str, id: &str) -> Result<u64, sqlx::Error> {
    Ok(sqlx::query("DELETE FROM slack_channels WHERE org_id = $1 AND id::text = $2").bind(org).bind(id).execute(&st.postgres).await?.rows_affected())
}

/// Connects a workspace, or replaces the token of the account with that name (reconnect).
#[allow(clippy::too_many_arguments)]
async fn upsert_account(st: &AppState, org: &str, user: Option<&str>, name: &str, team: &TeamInfo, token: &str, signing_secret: Option<&str>, via: &str) -> Result<AccountRow, ApiError> {
    let other: Option<(String,)> = sqlx::query_as("SELECT name FROM slack_accounts WHERE org_id = $1 AND team_id = $2 AND name <> $3").bind(org).bind(&team.team_id).bind(name).fetch_optional(&st.postgres).await?;
    if let Some((other,)) = other {
        return Err(ApiError::Conflict(format!("The {} workspace is already connected as {other}", team.team_name)));
    }
    sqlx::query(
        "INSERT INTO slack_accounts (org_id, name, team_id, team_name, bot_user_id, bot_token_enc, signing_secret_enc, scopes, installed_via, created_by) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10::uuid) \
         ON CONFLICT (org_id, name) DO UPDATE SET team_id = EXCLUDED.team_id, team_name = EXCLUDED.team_name, bot_user_id = EXCLUDED.bot_user_id, \
         bot_token_enc = EXCLUDED.bot_token_enc, signing_secret_enc = COALESCE(EXCLUDED.signing_secret_enc, slack_accounts.signing_secret_enc), \
         scopes = EXCLUDED.scopes, installed_via = EXCLUDED.installed_via, updated_at = now()",
    )
    .bind(org)
    .bind(name)
    .bind(&team.team_id)
    .bind(&team.team_name)
    .bind(&team.bot_user_id)
    .bind(st.secrets.encrypt(token))
    .bind(signing_secret.map(|s| st.secrets.encrypt(s)))
    .bind(&team.scopes)
    .bind(via)
    .bind(user)
    .execute(&st.postgres)
    .await?;
    // Channels keep their names; their conversation ids belong to the old workspace if it changed.
    sqlx::query("UPDATE slack_channels c SET channel_id = NULL FROM slack_accounts a WHERE a.id = c.account_id AND a.org_id = $1 AND a.name = $2").bind(org).bind(name).execute(&st.postgres).await?;
    account_by_name(st, org, name).await?.ok_or_else(|| ApiError::NotFound("Slack account".into()))
}

// ================================================================ Slack Web API

#[derive(Debug, PartialEq)]
pub enum SlackError {
    /// Slack answered `ok: false` with this code (`channel_not_found`, `not_in_channel`, `invalid_auth`, …).
    Api(String),
    /// Rate limited, unreachable or a 5xx: worth retrying.
    Transient(String),
}

impl SlackError {
    fn text(&self) -> String {
        match self {
            SlackError::Api(c) | SlackError::Transient(c) => c.clone(),
        }
    }
}

fn client() -> reqwest::Client {
    reqwest::Client::builder().timeout(Duration::from_secs(10)).user_agent("ObserveCraft-Slack").build().unwrap_or_default()
}

async fn answer(r: Result<reqwest::Response, reqwest::Error>) -> Result<Value, SlackError> {
    let r = r.map_err(|e| SlackError::Transient(format!("Slack is unreachable: {e}")))?;
    let status = r.status();
    if status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error() {
        return Err(SlackError::Transient(format!("Slack answered {status}")));
    }
    let v: Value = r.json().await.map_err(|e| SlackError::Transient(format!("Slack's answer can't be read: {e}")))?;
    if v["ok"].as_bool() == Some(true) {
        Ok(v)
    } else {
        Err(SlackError::Api(v["error"].as_str().unwrap_or("unknown_error").to_string()))
    }
}

/// A Web API method with a JSON body, as the bot.
async fn call_json(st: &AppState, token: &str, method: &str, body: &Value) -> Result<Value, SlackError> {
    answer(client().post(format!("{}/{method}", st.config.slack_api_url)).bearer_auth(token).json(body).send().await).await
}

/// A Web API method with form / query arguments, as the bot (or with none, for `oauth.v2.access`).
async fn call_form(st: &AppState, token: Option<&str>, method: &str, args: &[(&str, &str)]) -> Result<Value, SlackError> {
    let mut req = client().post(format!("{}/{method}", st.config.slack_api_url)).form(args);
    if let Some(t) = token {
        req = req.bearer_auth(t);
    }
    answer(req.send().await).await
}

#[derive(Debug, Clone)]
struct TeamInfo {
    team_id: String,
    team_name: String,
    bot_user_id: String,
    scopes: Vec<String>,
}

/// Checks a bot token (`auth.test`) and reads its workspace.
async fn team_of_token(st: &AppState, token: &str) -> Result<TeamInfo, SlackError> {
    let v = call_form(st, Some(token), "auth.test", &[]).await?;
    if v["bot_id"].as_str().is_none() {
        return Err(SlackError::Api("not_a_bot_token".into()));
    }
    Ok(TeamInfo {
        team_id: v["team_id"].as_str().unwrap_or_default().to_string(),
        team_name: v["team"].as_str().unwrap_or_default().to_string(),
        bot_user_id: v["user_id"].as_str().unwrap_or_default().to_string(),
        scopes: Vec::new(),
    })
}

/// What a Slack error code means for whoever configures the integration.
fn explain(code: &str, channel: &str, bot: &str) -> String {
    let bot = if bot.is_empty() { "the ObserveCraft app".to_string() } else { format!("<@{bot}>") };
    match code {
        "not_in_channel" => format!("The bot isn't in {channel}: invite {bot} to it (/invite) — private channels need it"),
        "channel_not_found" => format!("Slack has no channel {channel} the bot can see (private channels need the bot invited)"),
        "is_archived" => format!("{channel} is archived"),
        "invalid_auth" | "token_revoked" | "account_inactive" | "not_authed" | "token_expired" => "The Slack token is no longer valid: reconnect the workspace in Integrations → Slack".into(),
        "missing_scope" => "The Slack app lacks a permission: reinstall it with the scopes Integrations → Slack lists".into(),
        "not_a_bot_token" => "That isn't a bot token: use the Bot User OAuth Token (xoxb-…) from the app's OAuth & Permissions page".into(),
        "msg_too_long" | "invalid_blocks" => format!("Slack refused the message ({code})"),
        other => format!("Slack answered {other}"),
    }
}

/// The Slack conversation id of a channel name the bot can see, if any (public and the private ones it is in).
async fn find_conversation(st: &AppState, token: &str, name: &str) -> Result<Option<String>, SlackError> {
    let want = bare(name);
    let mut cursor = String::new();
    for _ in 0..20 {
        let v = call_form(st, Some(token), "conversations.list", &[("types", "public_channel,private_channel"), ("exclude_archived", "true"), ("limit", "1000"), ("cursor", &cursor)]).await?;
        if let Some(c) = v["channels"].as_array().and_then(|cs| cs.iter().find(|c| c["name"].as_str().map(str::to_lowercase).as_deref() == Some(want.as_str()))) {
            return Ok(c["id"].as_str().map(str::to_string));
        }
        cursor = v["response_metadata"]["next_cursor"].as_str().unwrap_or_default().to_string();
        if cursor.is_empty() {
            break;
        }
    }
    Ok(None)
}

// ================================================================ messages

/// Monitor messages are Markdown; Slack's mrkdwn escapes `& < >`, bolds with one `*` and links as `<url|text>`.
pub fn to_mrkdwn(md: &str) -> String {
    let escaped = md.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;");
    let bolded = escaped.replace("**", "*").replace("__", "_");
    // [text](url) → <url|text>
    let mut out = String::with_capacity(bolded.len());
    let mut rest = bolded.as_str();
    while let Some(open) = rest.find('[') {
        let after = &rest[open + 1..];
        let link = after.find("](").and_then(|close| {
            let text = &after[..close];
            let tail = &after[close + 2..];
            tail.find(')').map(|end| (text, &tail[..end], &tail[end + 1..]))
        });
        match link {
            Some((text, url, tail)) if !text.contains('[') && (url.starts_with("http://") || url.starts_with("https://")) && !url.contains(' ') => {
                out.push_str(&rest[..open]);
                out.push_str(&format!("<{url}|{text}>"));
                rest = tail;
            }
            _ => {
                out.push_str(&rest[..open + 1]);
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        format!("{}…", s.chars().take(max - 1).collect::<String>())
    }
}

/// Status color of the card's bar.
fn color(to: &str) -> &'static str {
    match to {
        "alert" => "#d33d3d",
        "warn" => "#e5a00d",
        "nodata" => "#8c8c99",
        _ => "#2ea44f",
    }
}

/// A monitor transition as a Slack message: fallback text, then a colored attachment of blocks.
pub fn monitor_message(n: &Notice, link: &str, display: Display, account_id: &str) -> Value {
    let title = n.title();
    let value = n.value.map(|v| format!("\nValue: `{v}`")).unwrap_or_default();
    let mut blocks = vec![json!({ "type": "section", "text": { "type": "mrkdwn", "text": format!("*<{link}|{}>*{value}", to_mrkdwn(&title)) } })];
    let message = n.message.trim();
    if display.message && !message.is_empty() {
        blocks.push(json!({ "type": "section", "text": { "type": "mrkdwn", "text": truncate(&to_mrkdwn(message), 2900) } }));
    }
    let mut context = Vec::new();
    if display.tags {
        if !n.group.is_empty() && n.group != "*" {
            context.push(format!("Scope: `{}`", n.group.replace(',', "`, `")));
        }
        if !n.tags.is_empty() {
            context.push(format!("Tags: {}", n.tags.iter().map(|t| format!("`{t}`")).collect::<Vec<_>>().join(" ")));
        }
    }
    if display.notified {
        let who = recipients(&n.message);
        if !who.is_empty() {
            context.push(format!("Notified: {}", who.iter().map(|w| format!("@{w}")).collect::<Vec<_>>().join(" ")));
        }
    }
    if !context.is_empty() {
        blocks.push(json!({ "type": "context", "elements": context.iter().map(|c| json!({ "type": "mrkdwn", "text": truncate(&to_mrkdwn(c).replace("&lt;", "<").replace("&gt;", ">"), 2900) })).collect::<Vec<_>>() }));
    }
    let mut actions = vec![json!({ "type": "button", "text": { "type": "plain_text", "text": "View monitor" }, "url": link, "action_id": "oc_view" })];
    let group = if n.group == "*" { "" } else { n.group.as_str() };
    if display.mute_buttons && !n.test && n.monitor_id > 0 && n.to != "ok" {
        for h in MUTE_HOURS {
            let value = format!("{account_id}|{}|{h}|{group}", n.monitor_id);
            if value.len() <= 2000 {
                let label = if h < 24 { format!("Mute {h}h") } else { format!("Mute {}d", h / 24) };
                actions.push(json!({ "type": "button", "text": { "type": "plain_text", "text": label }, "action_id": format!("oc_mute_{h}"), "value": value }));
            }
        }
    }
    blocks.push(json!({ "type": "actions", "elements": actions }));
    json!({
        "text": title,
        "attachments": [{ "color": color(&n.to), "fallback": title, "blocks": blocks }],
        "unfurl_links": false,
        "unfurl_media": false,
    })
}

/// Any other message (On-Call pages, workflows, reports, surveys): its mrkdwn text.
fn plain_message(out: &Outgoing) -> Value {
    json!({ "text": truncate(&out.slack_text, 39_000), "mrkdwn": true, "unfurl_links": false })
}

// ================================================================ delivery

/// The configured channel a `slack-…` handle (without `@`) names, with its workspace.
pub async fn resolve(st: &AppState, org: &str, recipient: &str) -> Result<Option<(AccountRow, ChannelRow)>, sqlx::Error> {
    let Some(rest) = recipient.strip_prefix("slack-") else { return Ok(None) };
    let rest = rest.to_lowercase();
    let mut accts = accounts(st, org).await?;
    if accts.is_empty() {
        return Ok(None);
    }
    // `slack-<account>-<channel>`, the longest account name first (names may contain dashes).
    accts.sort_by_key(|a| std::cmp::Reverse(a.name.len()));
    for a in &accts {
        if let Some(ch) = rest.strip_prefix(&format!("{}-", a.name)) {
            if let Some(c) = channel_by_name(st, org, &a.id, ch).await? {
                return Ok(Some((a.clone(), c)));
            }
        }
    }
    // `slack-<channel>`: the oldest workspace that has it.
    accts.sort_by_key(|a| a.created_at);
    let all = all_channels(st, org).await?;
    for a in &accts {
        if let Some(c) = all.iter().find(|c| c.account_id == a.id && bare(&c.name) == rest) {
            return Ok(Some((a.clone(), c.clone())));
        }
    }
    Ok(None)
}

/// Posts to a configured Slack channel; None when the handle names none (the caller then tries
/// incoming-webhook channels).
pub(crate) async fn deliver(st: &AppState, recipient: &str, out: &Outgoing, notice: Option<(&Notice, &str)>) -> Option<Outcome> {
    let (account, channel) = match resolve(st, &out.org, recipient).await {
        Ok(Some(found)) => found,
        Ok(None) => return None,
        Err(e) => return Some(Outcome::Failed(e.to_string())),
    };
    let body = match notice {
        Some((n, link)) => monitor_message(n, link, channel.display(), &account.id),
        None => plain_message(out),
    };
    Some(post(st, &account, &channel, body).await)
}

async fn post(st: &AppState, account: &AccountRow, channel: &ChannelRow, mut body: Value) -> Outcome {
    let Ok(token) = st.secrets.decrypt(&account.bot_token_enc) else {
        return Outcome::Failed("The Slack token can't be decrypted (was OC_SECRET_KEY changed?)".into());
    };
    let target = match &channel.channel_id {
        Some(id) => id.clone(),
        None => match find_conversation(st, &token, &channel.name).await {
            Ok(Some(id)) => {
                let _ = sqlx::query("UPDATE slack_channels SET channel_id = $2 WHERE id::text = $1").bind(&channel.id).bind(&id).execute(&st.postgres).await;
                id
            }
            // Not visible to the bot: Slack still takes a public channel's name with chat:write.public.
            Ok(None) => bare(&channel.name),
            Err(SlackError::Transient(e)) => return Outcome::Failed(e),
            Err(SlackError::Api(code)) => return Outcome::Unknown(explain(&code, &channel.name, &account.bot_user_id)),
        },
    };
    body["channel"] = json!(target);
    match call_json(st, &token, "chat.postMessage", &body).await {
        Ok(_) => Outcome::Sent,
        Err(SlackError::Transient(e)) => Outcome::Failed(e),
        // A stale conversation id (channel renamed / recreated): look it up again next time.
        Err(SlackError::Api(code)) if code == "channel_not_found" && channel.channel_id.is_some() => {
            let _ = sqlx::query("UPDATE slack_channels SET channel_id = NULL WHERE id::text = $1").bind(&channel.id).execute(&st.postgres).await;
            Outcome::Failed(explain(&code, &channel.name, &account.bot_user_id))
        }
        Err(SlackError::Api(code)) => Outcome::Unknown(explain(&code, &channel.name, &account.bot_user_id)),
    }
}

// ================================================================ UI API

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SlackChannel {
    pub id: String,
    /// As configured: `#checkout-alerts`.
    pub name: String,
    /// Mention it as `@<handle>`.
    pub handle: String,
    pub display: Display,
    /// Slack's conversation id once a message was delivered.
    pub conversation_id: Option<String>,
    pub created_at: i64,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SlackAccount {
    pub id: String,
    pub name: String,
    pub team_id: String,
    pub team_name: String,
    pub bot_user_id: String,
    /// oauth | token
    pub installed_via: String,
    /// Mute buttons can be verified (this install's signing secret, or the workspace app's own).
    pub interactive: bool,
    /// Scopes of [`BOT_SCOPES`] the OAuth install didn't grant (empty for token installs: not reported by Slack).
    pub missing_scopes: Vec<String>,
    pub channels: Vec<SlackChannel>,
    pub created_at: i64,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SlackOverview {
    /// "Add to Slack" is available (this install has a Slack app).
    pub oauth_available: bool,
    /// Where Slack sends OAuth installs back.
    pub redirect_url: String,
    /// Where Slack posts button presses (the app's Interactivity Request URL).
    pub interactivity_url: String,
    /// A Slack app manifest for workspaces that connect with their own app's bot token.
    pub app_manifest: String,
    pub scopes: Vec<String>,
    pub accounts: Vec<SlackAccount>,
}

fn redirect_url(st: &AppState) -> String {
    format!("{}/api/v1/public/slack/oauth/callback", st.config.public_url)
}

fn interactivity_url(st: &AppState) -> String {
    format!("{}/api/v1/public/slack/interactive", st.config.public_url)
}

/// The manifest of a Slack app for this install (Slack: Create New App → From a manifest).
pub fn app_manifest(interactivity: &str) -> String {
    let scopes = BOT_SCOPES.iter().map(|s| format!("      - {s}")).collect::<Vec<_>>().join("\n");
    format!(
        "display_information:\n  name: ObserveCraft\n  description: Monitor alerts, On-Call pages and reports from ObserveCraft\n  background_color: \"#1f2937\"\n\
features:\n  bot_user:\n    display_name: ObserveCraft\n    always_online: true\n\
oauth_config:\n  scopes:\n    bot:\n{scopes}\n\
settings:\n  interactivity:\n    is_enabled: true\n    request_url: {interactivity}\n  org_deploy_enabled: false\n  socket_mode_enabled: false\n  token_rotation_enabled: false\n"
    )
}

fn channel_view(account: &AccountRow, c: ChannelRow) -> SlackChannel {
    SlackChannel { handle: handle(&account.name, &c.name), display: c.display(), id: c.id, name: c.name, conversation_id: c.channel_id, created_at: c.created_at }
}

async fn account_view(st: &AppState, a: AccountRow) -> Result<SlackAccount, ApiError> {
    let chans = channels(st, &a.org_id, &a.id).await?;
    let missing_scopes = if a.installed_via == "oauth" { BOT_SCOPES.iter().filter(|s| !a.scopes.iter().any(|g| g == *s)).map(|s| s.to_string()).collect() } else { Vec::new() };
    let interactive = a.signing_secret_enc.is_some() || (a.installed_via == "oauth" && st.config.slack_signing_secret.is_some());
    Ok(SlackAccount {
        channels: chans.into_iter().map(|c| channel_view(&a, c)).collect(),
        id: a.id,
        name: a.name,
        team_id: a.team_id,
        team_name: a.team_name,
        bot_user_id: a.bot_user_id,
        installed_via: a.installed_via,
        interactive,
        missing_scopes,
        created_at: a.created_at,
    })
}

/// The org's Slack workspaces and channels, and what connecting one needs.
#[utoipa::path(get, path = "/api/v1/integrations/slack", operation_id = "get_slack_integration", responses((status = 200, body = SlackOverview)), tag = "notifications")]
pub async fn overview(State(st): State<AppState>, ctx: AuthContext) -> ApiResult<SlackOverview> {
    let mut out = Vec::new();
    for a in accounts(&st, &ctx.org_id).await? {
        out.push(account_view(&st, a).await?);
    }
    Ok(Json(SlackOverview {
        oauth_available: st.config.slack_client_id.is_some() && st.config.slack_client_secret.is_some(),
        redirect_url: redirect_url(&st),
        interactivity_url: interactivity_url(&st),
        app_manifest: app_manifest(&interactivity_url(&st)),
        scopes: BOT_SCOPES.iter().map(|s| s.to_string()).collect(),
        accounts: out,
    }))
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ConnectSlackAccount {
    /// Account name (`@slack-<name>-<channel>`).
    pub name: String,
    /// Bot User OAuth Token (`xoxb-…`) of the workspace's Slack app.
    pub bot_token: String,
    /// The app's signing secret, for mute buttons.
    pub signing_secret: Option<String>,
}

/// Connects a workspace with its own Slack app's bot token (or reconnects the account with that name).
#[utoipa::path(post, path = "/api/v1/integrations/slack/accounts", operation_id = "connect_slack_account", request_body = ConnectSlackAccount, responses((status = 200, body = SlackAccount)), tag = "notifications")]
pub async fn connect(State(st): State<AppState>, Admin(ctx): Admin, Json(req): Json<ConnectSlackAccount>) -> ApiResult<SlackAccount> {
    let name = account_name(&req.name).map_err(ApiError::BadRequest)?;
    let token = req.bot_token.trim();
    if !token.starts_with("xoxb-") {
        return Err(ApiError::BadRequest(explain("not_a_bot_token", "", "")));
    }
    let team = team_of_token(&st, token).await.map_err(|e| match e {
        SlackError::Api(code) => ApiError::BadRequest(explain(&code, "", "")),
        SlackError::Transient(m) => ApiError::BadRequest(m),
    })?;
    let secret = req.signing_secret.as_deref().map(str::trim).filter(|s| !s.is_empty());
    let a = upsert_account(&st, &ctx.org_id, Some(&ctx.user_id), &name, &team, token, secret, "token").await?;
    ctx.audit(&st, "slack_account.connected", &name, &team.team_name).await;
    Ok(Json(account_view(&st, a).await?))
}

#[utoipa::path(delete, path = "/api/v1/integrations/slack/accounts/{id}", operation_id = "disconnect_slack_account", params(("id" = String, Path)), responses((status = 204)), tag = "notifications")]
pub async fn disconnect(State(st): State<AppState>, Admin(ctx): Admin, Path(id): Path<String>) -> Result<StatusCode, ApiError> {
    let row: Option<(String,)> = sqlx::query_as("DELETE FROM slack_accounts WHERE org_id = $1 AND id::text = $2 RETURNING name").bind(&ctx.org_id).bind(&id).fetch_optional(&st.postgres).await?;
    let Some((name,)) = row else { return Err(ApiError::NotFound("Slack account not found".into())) };
    ctx.audit(&st, "slack_account.disconnected", &name, "").await;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SlackConversation {
    pub id: String,
    pub name: String,
    pub is_private: bool,
    /// The bot is in it (private channels need that).
    pub is_member: bool,
}

/// The workspace's channels the bot can see, for the channel picker.
#[utoipa::path(get, path = "/api/v1/integrations/slack/accounts/{id}/conversations", operation_id = "list_slack_conversations", params(("id" = String, Path)), responses((status = 200, body = Vec<SlackConversation>)), tag = "notifications")]
pub async fn conversations(State(st): State<AppState>, ctx: AuthContext, Path(id): Path<String>) -> ApiResult<Vec<SlackConversation>> {
    let a = account_by_id(&st, &ctx.org_id, &id).await?.ok_or_else(|| ApiError::NotFound("Slack account not found".into()))?;
    let token = st.secrets.decrypt(&a.bot_token_enc).map_err(|_| ApiError::BadRequest("The Slack token can't be decrypted".into()))?;
    let mut out = Vec::new();
    let mut cursor = String::new();
    for _ in 0..10 {
        let v = call_form(&st, Some(&token), "conversations.list", &[("types", "public_channel,private_channel"), ("exclude_archived", "true"), ("limit", "1000"), ("cursor", &cursor)])
            .await
            .map_err(|e| ApiError::BadRequest(match e {
                SlackError::Api(code) => explain(&code, "", &a.bot_user_id),
                SlackError::Transient(m) => m,
            }))?;
        for c in v["channels"].as_array().into_iter().flatten() {
            out.push(SlackConversation {
                id: c["id"].as_str().unwrap_or_default().into(),
                name: c["name"].as_str().unwrap_or_default().into(),
                is_private: c["is_private"].as_bool().unwrap_or(false),
                is_member: c["is_member"].as_bool().unwrap_or(false),
            });
        }
        cursor = v["response_metadata"]["next_cursor"].as_str().unwrap_or_default().to_string();
        if cursor.is_empty() {
            break;
        }
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(Json(out))
}

#[derive(Deserialize, ToSchema)]
pub struct SlackChannelInput {
    /// `#checkout-alerts`; optional on update.
    pub name: Option<String>,
    /// Display options; missing ones keep their current (or default) value.
    #[schema(value_type = Option<Display>)]
    pub display: Option<Value>,
}

#[utoipa::path(post, path = "/api/v1/integrations/slack/accounts/{id}/channels", operation_id = "create_slack_channel", params(("id" = String, Path)), request_body = SlackChannelInput, responses((status = 200, body = SlackChannel)), tag = "notifications")]
pub async fn add_channel(State(st): State<AppState>, Admin(ctx): Admin, Path(id): Path<String>, Json(req): Json<SlackChannelInput>) -> ApiResult<SlackChannel> {
    let a = account_by_id(&st, &ctx.org_id, &id).await?.ok_or_else(|| ApiError::NotFound("Slack account not found".into()))?;
    let name = req.name.ok_or_else(|| ApiError::BadRequest("name is required".into()))?;
    let display = Display::merged(Display::default(), req.display.as_ref()).map_err(ApiError::BadRequest)?;
    let c = create_channel(&st, &ctx.org_id, &a.id, &name, display).await?;
    ctx.audit(&st, "slack_channel.created", &handle(&a.name, &c.name), "").await;
    Ok(Json(channel_view(&a, c)))
}

async fn channel_and_account(st: &AppState, org: &str, id: &str) -> Result<(AccountRow, ChannelRow), ApiError> {
    let c = channel_by_id(st, org, id).await?.ok_or_else(|| ApiError::NotFound("Slack channel not found".into()))?;
    let a = account_by_id(st, org, &c.account_id).await?.ok_or_else(|| ApiError::NotFound("Slack account not found".into()))?;
    Ok((a, c))
}

#[utoipa::path(patch, path = "/api/v1/integrations/slack/channels/{id}", operation_id = "update_slack_channel", params(("id" = String, Path)), request_body = SlackChannelInput, responses((status = 200, body = SlackChannel)), tag = "notifications")]
pub async fn edit_channel(State(st): State<AppState>, Admin(ctx): Admin, Path(id): Path<String>, Json(req): Json<SlackChannelInput>) -> ApiResult<SlackChannel> {
    let (a, c) = channel_and_account(&st, &ctx.org_id, &id).await?;
    let display = Display::merged(c.display(), req.display.as_ref()).map_err(ApiError::BadRequest)?;
    let c = update_channel(&st, &ctx.org_id, &c, req.name.as_deref(), display).await?;
    ctx.audit(&st, "slack_channel.updated", &handle(&a.name, &c.name), "").await;
    Ok(Json(channel_view(&a, c)))
}

#[utoipa::path(delete, path = "/api/v1/integrations/slack/channels/{id}", operation_id = "delete_slack_channel", params(("id" = String, Path)), responses((status = 204)), tag = "notifications")]
pub async fn remove_channel(State(st): State<AppState>, Admin(ctx): Admin, Path(id): Path<String>) -> Result<StatusCode, ApiError> {
    let (a, c) = channel_and_account(&st, &ctx.org_id, &id).await?;
    delete_channel(&st, &ctx.org_id, &c.id).await?;
    ctx.audit(&st, "slack_channel.deleted", &handle(&a.name, &c.name), "").await;
    Ok(StatusCode::NO_CONTENT)
}

/// Sends a sample monitor alert to the channel.
#[utoipa::path(post, path = "/api/v1/integrations/slack/channels/{id}/test", operation_id = "test_slack_channel", params(("id" = String, Path)), responses((status = 200, body = super::notifications::TestDelivery)), tag = "notifications")]
pub async fn test_channel(State(st): State<AppState>, Admin(ctx): Admin, Path(id): Path<String>) -> ApiResult<super::notifications::TestDelivery> {
    let (a, c) = channel_and_account(&st, &ctx.org_id, &id).await?;
    let h = handle(&a.name, &c.name);
    let n = Notice {
        org: ctx.org_id.clone(),
        monitor_id: 0,
        monitor_name: "Sample monitor".into(),
        priority: None,
        tags: vec!["team:example".into()],
        group: "service:storefront-api".into(),
        from: "ok".into(),
        to: "alert".into(),
        value: Some(42.0),
        message: format!("This is a test notification from ObserveCraft to @{h}. Monitors mention this channel as **@{h}**."),
        at: 0,
        test: true,
    };
    let link = format!("{}/monitors", st.config.public_url);
    let outcome = post(&st, &a, &c, monitor_message(&n, &link, c.display(), &a.id)).await;
    ctx.audit(&st, "slack_channel.tested", &h, "").await;
    let (status, error) = match outcome {
        Outcome::Sent => ("sent", None),
        Outcome::Unknown(e) => ("unknown_recipient", Some(e)),
        Outcome::Failed(e) => ("failed", Some(e)),
    };
    Ok(Json(super::notifications::TestDelivery { recipient: h, status: status.into(), error }))
}

// ================================================================ OAuth ("Add to Slack")

#[derive(Deserialize, ToSchema)]
pub struct StartSlackOAuth {
    /// Account name for the workspace being connected.
    pub name: String,
}

#[derive(Serialize, ToSchema)]
pub struct SlackOAuthStart {
    /// Slack's authorize page; open it in the browser.
    pub url: String,
}

/// Starts "Add to Slack": the URL of Slack's consent page for this install's app.
#[utoipa::path(post, path = "/api/v1/integrations/slack/oauth", operation_id = "start_slack_oauth", request_body = StartSlackOAuth, responses((status = 200, body = SlackOAuthStart)), tag = "notifications")]
pub async fn oauth_start(State(st): State<AppState>, Admin(ctx): Admin, Json(req): Json<StartSlackOAuth>) -> ApiResult<SlackOAuthStart> {
    let (Some(client_id), Some(_)) = (&st.config.slack_client_id, &st.config.slack_client_secret) else {
        return Err(ApiError::BadRequest("This install has no Slack app (OC_SLACK_CLIENT_ID / OC_SLACK_CLIENT_SECRET): connect with a bot token instead".into()));
    };
    let name = account_name(&req.name).map_err(ApiError::BadRequest)?;
    let state = crypto::random_token("ocsl_");
    sqlx::query("DELETE FROM slack_oauth_states WHERE created_at < now() - interval '10 minutes'").execute(&st.postgres).await?;
    sqlx::query("INSERT INTO slack_oauth_states (state, org_id, user_id, name) VALUES ($1, $2, $3::uuid, $4)").bind(&state).bind(&ctx.org_id).bind(&ctx.user_id).bind(&name).execute(&st.postgres).await?;
    let mut url = url::Url::parse("https://slack.com/oauth/v2/authorize").expect("static url");
    url.query_pairs_mut().append_pair("client_id", client_id).append_pair("scope", &BOT_SCOPES.join(",")).append_pair("redirect_uri", &redirect_url(&st)).append_pair("state", &state);
    Ok(Json(SlackOAuthStart { url: url.into() }))
}

#[derive(Deserialize)]
pub struct OAuthCallback {
    code: Option<String>,
    state: Option<String>,
    error: Option<String>,
}

fn back(st: &AppState, key: &str, value: &str) -> Response {
    let mut url = url::Url::parse(&format!("{}/integrations/slack", st.config.public_url)).unwrap_or_else(|_| url::Url::parse("http://localhost/integrations/slack").expect("static url"));
    url.query_pairs_mut().append_pair(key, value);
    Redirect::to(url.as_str()).into_response()
}

/// Slack sends the browser back here after the consent page; the state names the org and member.
#[utoipa::path(get, path = "/api/v1/public/slack/oauth/callback", operation_id = "slack_oauth_callback", params(("code" = Option<String>, Query), ("state" = Option<String>, Query)), responses((status = 303)), tag = "notifications")]
pub async fn oauth_callback(State(st): State<AppState>, Query(p): Query<OAuthCallback>) -> Response {
    let Some(state) = p.state else { return back(&st, "error", "Missing state") };
    let row: Option<(String, String, String)> = sqlx::query_as("DELETE FROM slack_oauth_states WHERE state = $1 AND created_at > now() - interval '10 minutes' RETURNING org_id, user_id::text, name")
        .bind(&state)
        .fetch_optional(&st.postgres)
        .await
        .unwrap_or(None);
    let Some((org, user, name)) = row else { return back(&st, "error", "This Slack install expired. Please try again.") };
    if let Some(e) = p.error {
        return back(&st, "error", &format!("Slack didn't connect: {e}"));
    }
    let (Some(code), Some(client_id), Some(secret)) = (p.code, &st.config.slack_client_id, &st.config.slack_client_secret) else { return back(&st, "error", "Missing authorization code") };
    let v = match call_form(&st, None, "oauth.v2.access", &[("client_id", client_id), ("client_secret", secret), ("code", &code), ("redirect_uri", &redirect_url(&st))]).await {
        Ok(v) => v,
        Err(e) => return back(&st, "error", &format!("Slack refused the install: {}", e.text())),
    };
    let Some(token) = v["access_token"].as_str().filter(|t| t.starts_with("xoxb-")) else { return back(&st, "error", "Slack returned no bot token") };
    let team = TeamInfo {
        team_id: v["team"]["id"].as_str().unwrap_or_default().into(),
        team_name: v["team"]["name"].as_str().unwrap_or_default().into(),
        bot_user_id: v["bot_user_id"].as_str().unwrap_or_default().into(),
        scopes: v["scope"].as_str().unwrap_or_default().split(',').map(str::trim).filter(|s| !s.is_empty()).map(str::to_string).collect(),
    };
    match upsert_account(&st, &org, Some(&user), &name, &team, token, None, "oauth").await {
        Ok(_) => {
            crate::auth::audit(&st.postgres, &org, Some(&user), "Slack install", "slack_account.connected", &name, &team.team_name, None).await;
            back(&st, "connected", &name)
        }
        Err(ApiError::Conflict(m)) => back(&st, "error", &m),
        Err(e) => {
            tracing::error!(error = %e, "storing a Slack install failed");
            back(&st, "error", "The workspace couldn't be saved")
        }
    }
}

// ================================================================ interactive buttons

/// Slack's request signature: `v0=` HMAC-SHA256(signing secret, `v0:<timestamp>:<body>`), hex.
#[cfg(test)]
pub fn signature(secret: &str, timestamp: &str, body: &[u8]) -> String {
    let mut mac = <Hmac<Sha256> as hmac::KeyInit>::new_from_slice(secret.as_bytes()).expect("hmac key");
    mac.update(format!("v0:{timestamp}:").as_bytes());
    mac.update(body);
    format!("v0={}", mac.finalize().into_bytes().iter().map(|b| format!("{b:02x}")).collect::<String>())
}

fn signed_by(secret: &str, timestamp: &str, body: &[u8], sent: &str) -> bool {
    let Some(hex) = sent.strip_prefix("v0=") else { return false };
    let Ok(sent) = (0..hex.len()).step_by(2).map(|i| hex.get(i..i + 2).and_then(|b| u8::from_str_radix(b, 16).ok()).ok_or(())).collect::<Result<Vec<u8>, ()>>() else { return false };
    let mut mac = <Hmac<Sha256> as hmac::KeyInit>::new_from_slice(secret.as_bytes()).expect("hmac key");
    mac.update(format!("v0:{timestamp}:").as_bytes());
    mac.update(body);
    mac.verify_slice(&sent).is_ok()
}

/// A mute button's value: `<account id>|<monitor id>|<hours>|<group>`.
#[derive(Debug, PartialEq)]
pub struct MuteRequest {
    pub account_id: String,
    pub monitor_id: i64,
    pub hours: i64,
    pub group: String,
}

pub fn parse_mute(value: &str) -> Option<MuteRequest> {
    let mut parts = value.splitn(4, '|');
    let account_id = parts.next()?.to_string();
    let monitor_id = parts.next()?.parse().ok()?;
    let hours: i64 = parts.next()?.parse().ok()?;
    if !MUTE_HOURS.contains(&hours) {
        return None;
    }
    Some(MuteRequest { account_id, monitor_id, hours, group: parts.next().unwrap_or_default().to_string() })
}

/// Button presses on messages the bot posted (Mute 1h / 4h / 1d). Always answers 200 quickly, as
/// Slack expects; the outcome goes back to the presser as an ephemeral message.
#[utoipa::path(post, path = "/api/v1/public/slack/interactive", operation_id = "slack_interactive", request_body(content = String, content_type = "application/x-www-form-urlencoded"), responses((status = 200), (status = 401)), tag = "notifications")]
pub async fn interactive(State(st): State<AppState>, headers: HeaderMap, body: Bytes) -> Response {
    let header = |n: &str| headers.get(n).and_then(|v| v.to_str().ok()).unwrap_or_default().to_string();
    let (timestamp, sent) = (header("x-slack-request-timestamp"), header("x-slack-signature"));
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs() as i64;
    if timestamp.parse::<i64>().map(|t| (now - t).abs() > 300).unwrap_or(true) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let Some(payload) = url::form_urlencoded::parse(&body).find(|(k, _)| k == "payload").and_then(|(_, v)| serde_json::from_str::<Value>(&v).ok()) else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    let team = payload["team"]["id"].as_str().or(payload["user"]["team_id"].as_str()).unwrap_or_default().to_string();
    let candidates: Vec<AccountRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!("{ACCOUNT_SELECT} WHERE team_id = $1"))).bind(&team).fetch_all(&st.postgres).await.unwrap_or_default();
    let verified: Vec<AccountRow> = candidates
        .into_iter()
        .filter(|a| {
            let secret = match &a.signing_secret_enc {
                Some(s) => st.secrets.decrypt(s).ok(),
                None if a.installed_via == "oauth" => st.config.slack_signing_secret.clone(),
                None => None,
            };
            secret.is_some_and(|s| signed_by(&s, &timestamp, &body, &sent))
        })
        .collect();
    if verified.is_empty() {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let action = &payload["actions"][0];
    let Some(mute) = action["value"].as_str().and_then(parse_mute).filter(|_| action["action_id"].as_str().is_some_and(|a| a.starts_with("oc_mute_"))) else {
        // "View monitor" (a link) and anything else: nothing to do.
        return StatusCode::OK.into_response();
    };
    let Some(account) = verified.into_iter().find(|a| a.id == mute.account_id) else { return StatusCode::UNAUTHORIZED.into_response() };
    let slack_user = payload["user"]["id"].as_str().unwrap_or_default().to_string();
    let response_url = payload["response_url"].as_str().unwrap_or_default().to_string();
    tokio::spawn(async move {
        let text = match mute_from_slack(&st, &account, &slack_user, &mute).await {
            Ok(t) | Err(t) => t,
        };
        if response_url.starts_with("https://hooks.slack.com/") {
            let _ = client().post(&response_url).json(&json!({ "response_type": "ephemeral", "replace_original": false, "text": text })).send().await;
        }
    });
    StatusCode::OK.into_response()
}

/// Mutes the monitor (and group) as the organization member the Slack user is.
async fn mute_from_slack(st: &AppState, account: &AccountRow, slack_user: &str, m: &MuteRequest) -> Result<String, String> {
    let token = st.secrets.decrypt(&account.bot_token_enc).map_err(|_| "The Slack token can't be decrypted".to_string())?;
    let info = call_form(st, Some(&token), "users.info", &[("user", slack_user)]).await.map_err(|e| format!("Couldn't identify you: {}", explain(&e.text(), "", "")))?;
    let email = info["user"]["profile"]["email"].as_str().unwrap_or_default().to_string();
    if email.is_empty() {
        return Err("Your Slack profile has no email ObserveCraft can read (users:read.email)".into());
    }
    let member: Option<(String, String, Vec<String>)> = sqlx::query_as(
        "SELECT u.id::text, u.name, COALESCE((SELECT array_agg(role_id) FROM user_roles WHERE user_id = u.id), '{}') \
         FROM users u WHERE u.org_id = $1 AND lower(u.email) = lower($2) AND u.status = 'active'",
    )
    .bind(&account.org_id)
    .bind(&email)
    .fetch_optional(&st.postgres)
    .await
    .map_err(|e| e.to_string())?;
    let Some((user_id, user_name, roles)) = member else { return Err(format!("{email} isn't a member of this ObserveCraft organization")) };
    if access_from_roles(&account.org_id, &roles) < Access::Standard {
        return Err("Muting monitors needs the Standard or Admin role".into());
    }
    let monitor: Option<(String,)> = sqlx::query_as("SELECT name FROM monitors WHERE org_id = $1 AND id = $2").bind(&account.org_id).bind(m.monitor_id).fetch_optional(&st.postgres).await.map_err(|e| e.to_string())?;
    let Some((monitor_name,)) = monitor else { return Err("That monitor no longer exists".into()) };
    let scope: Vec<String> = parse_group_key(&m.group).into_iter().map(|(k, v)| format!("{k}:{v}")).collect();
    let message = format!("Muted from Slack by {user_name}");
    let (id,): (i64,) = sqlx::query_as(
        "INSERT INTO downtimes (org_id, monitor_id, monitor_tags, scope, message, start_at, end_at, created_by) \
         VALUES ($1, $2, '{}', $3, $4, now(), now() + make_interval(hours => $5), $6::uuid) RETURNING id",
    )
    .bind(&account.org_id)
    .bind(m.monitor_id)
    .bind(&scope)
    .bind(&message)
    .bind(m.hours as i32)
    .bind(&user_id)
    .fetch_one(&st.postgres)
    .await
    .map_err(|e| e.to_string())?;
    crate::auth::audit(&st.postgres, &account.org_id, Some(&user_id), &user_name, "downtime.create", &format!("downtime:{id}"), &format!("{message} ({}h)", m.hours), None).await;
    let what = if scope.is_empty() { monitor_name } else { format!("{monitor_name} on {}", scope.join(",")) };
    let how_long = if m.hours < 24 { format!("{}h", m.hours) } else { format!("{}d", m.hours / 24) };
    Ok(format!("Muted {what} for {how_long}."))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn notice(to: &str) -> Notice {
        Notice {
            org: "o".into(),
            monitor_id: 7,
            monitor_name: "High error rate".into(),
            priority: None,
            tags: vec!["team:checkout".into()],
            group: "service:billing".into(),
            from: "ok".into(),
            to: to.into(),
            value: Some(5.0),
            message: "Errors are **up** on billing. See [the runbook](https://wiki.example.com/rb). @slack-ops-alerts @ana@example.com".into(),
            at: 0,
            test: false,
        }
    }

    #[test]
    fn display_defaults_and_merges() {
        assert_eq!(Display::default(), Display { message: true, notified: true, snapshot: true, tags: true, mute_buttons: false });
        let d = Display::merged(Display::default(), Some(&json!({ "mute_buttons": true, "snapshot": false }))).unwrap();
        assert!(d.mute_buttons && !d.snapshot && d.message);
        assert!(Display::merged(Display::default(), Some(&json!({ "tags": "yes" }))).is_err());
        assert_eq!(Display::merged(d, None).unwrap(), d);
    }

    #[test]
    fn names_and_handles() {
        assert_eq!(account_name(" Example-Workspace ").unwrap(), "example-workspace");
        assert!(account_name("a b").is_err());
        assert_eq!(channel_name(" #checkout-alerts ").unwrap(), "#checkout-alerts");
        assert!(channel_name("#").is_err());
        assert!(channel_name("bad name").is_err());
        assert_eq!(handle("main", "#Checkout-Alerts"), "slack-main-checkout-alerts");
    }

    #[test]
    fn markdown_becomes_mrkdwn() {
        assert_eq!(to_mrkdwn("**up** on <b> & [runbook](https://wiki.example.com/x)"), "*up* on &lt;b&gt; &amp; <https://wiki.example.com/x|runbook>");
        assert_eq!(to_mrkdwn("[not a link](ftp://x) [x]"), "[not a link](ftp://x) [x]");
    }

    #[test]
    fn a_monitor_card_follows_the_display_options() {
        let all = Display { message: true, notified: true, snapshot: true, tags: true, mute_buttons: true };
        let v = monitor_message(&notice("alert"), "https://oc.example.com/monitors/7", all, "acc");
        assert_eq!(v["text"], "[Triggered] High error rate on service:billing");
        let a = &v["attachments"][0];
        assert_eq!(a["color"], "#d33d3d");
        let blocks = a["blocks"].as_array().unwrap();
        assert_eq!(blocks[0]["text"]["text"], "*<https://oc.example.com/monitors/7|[Triggered] High error rate on service:billing>*\nValue: `5`");
        assert!(blocks[1]["text"]["text"].as_str().unwrap().contains("<https://wiki.example.com/rb|the runbook>"));
        let ctx = blocks[2]["elements"].as_array().unwrap();
        assert_eq!(ctx[0]["text"], "Scope: `service:billing`");
        assert_eq!(ctx[1]["text"], "Tags: `team:checkout`");
        assert_eq!(ctx[2]["text"], "Notified: @slack-ops-alerts @ana@example.com");
        let buttons = blocks[3]["elements"].as_array().unwrap();
        assert_eq!(buttons.len(), 4);
        assert_eq!(buttons[0]["url"], "https://oc.example.com/monitors/7");
        assert_eq!(buttons[1]["value"], "acc|7|1|service:billing");
        assert_eq!(buttons[3]["text"]["text"], "Mute 1d");

        // Nothing optional, and no mute buttons on recoveries.
        let none = Display { message: false, notified: false, snapshot: false, tags: false, mute_buttons: true };
        let v = monitor_message(&notice("ok"), "https://oc.example.com/monitors/7", none, "acc");
        let blocks = v["attachments"][0]["blocks"].as_array().unwrap();
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[1]["elements"].as_array().unwrap().len(), 1);
        assert_eq!(v["attachments"][0]["color"], "#2ea44f");
    }

    #[test]
    fn mute_values_round_trip() {
        assert_eq!(parse_mute("acc|7|4|service:billing,host:i-0a1b2c3d4e5f60001"), Some(MuteRequest { account_id: "acc".into(), monitor_id: 7, hours: 4, group: "service:billing,host:i-0a1b2c3d4e5f60001".into() }));
        assert_eq!(parse_mute("acc|7|1|").unwrap().group, "");
        assert_eq!(parse_mute("acc|7|5|"), None);
        assert_eq!(parse_mute("acc|x|1|"), None);
    }

    #[test]
    fn slack_signatures() {
        // Slack's documented example.
        let body = b"token=xyzz0WbapA4vBCDEFasx0q6G&team_id=T1DC2JH3J&team_domain=testteamnow&channel_id=G8PSS9T3V&channel_name=foobar&user_id=U2CERLKJA&user_name=roadrunner&command=%2Fwebhook-collect&text=&response_url=https%3A%2F%2Fhooks.slack.com%2Fcommands%2FT1DC2JH3J%2F397700885554%2F96rGlfmibIGlgcZRskXaIFfN&trigger_id=398738663015.47445629121.803a0bc887a14d10d2c447fce8b6703c";
        let sig = signature("8f742231b10e8888abcd99yyyzzz85a5", "1531420618", body);
        assert_eq!(sig, "v0=a2114d57b48eac39b9ad189dd8316235a7b4a8d21a10bd27519666489c69b503");
        assert!(signed_by("8f742231b10e8888abcd99yyyzzz85a5", "1531420618", body, &sig));
        assert!(!signed_by("other", "1531420618", body, &sig));
        assert!(!signed_by("8f742231b10e8888abcd99yyyzzz85a5", "1531420619", body, &sig));
        assert!(!signed_by("8f742231b10e8888abcd99yyyzzz85a5", "1531420618", body, "v0=zz"));
    }

    #[test]
    fn the_manifest_asks_for_every_scope() {
        let m = app_manifest("https://oc.example.com/api/v1/public/slack/interactive");
        for s in BOT_SCOPES {
            assert!(m.contains(&format!("      - {s}\n")), "{s}");
        }
        assert!(m.contains("request_url: https://oc.example.com/api/v1/public/slack/interactive"));
    }

    // ------------------------------------------------------------ against Postgres and a fake Slack

    use std::sync::{Arc, Mutex};

    /// A fake Slack Web API: `auth.test`, `conversations.list`, `chat.postMessage` (recorded), `users.info`.
    async fn fake_slack() -> (String, Arc<Mutex<Vec<Value>>>) {
        use axum::routing::post as p;
        let posted: Arc<Mutex<Vec<Value>>> = Arc::default();
        let rec = posted.clone();
        let app = axum::Router::new()
            .route("/auth.test", p(|| async { Json(json!({ "ok": true, "team": "Example Shop", "team_id": "T0EXAMPLE", "user_id": "U0BOT", "bot_id": "B0BOT" })) }))
            .route(
                "/conversations.list",
                p(|| async { Json(json!({ "ok": true, "channels": [{ "id": "C0CHECKOUT", "name": "checkout-alerts", "is_private": false, "is_member": false }, { "id": "C0PRIV", "name": "billing-private", "is_private": true, "is_member": true }], "response_metadata": { "next_cursor": "" } })) }),
            )
            .route(
                "/chat.postMessage",
                p(move |Json(body): Json<Value>| {
                    let rec = rec.clone();
                    async move {
                        if body["channel"] == "gone" {
                            return Json(json!({ "ok": false, "error": "channel_not_found" }));
                        }
                        rec.lock().unwrap().push(body);
                        Json(json!({ "ok": true, "ts": "1.2" }))
                    }
                }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (format!("http://{addr}"), posted)
    }

    #[tokio::test]
    async fn monitors_post_to_configured_channels_of_their_org() {
        let (url, posted) = fake_slack().await;
        let Some((_, states)) = crate::test_support::replicas_with(1, |c| c.slack_api_url = url.clone()).await else { return };
        let st = &states[0];
        let (org, other) = (crate::test_support::new_org(st, "slack").await, crate::test_support::new_org(st, "slack-other").await);
        let team = team_of_token(st, "xoxb-test").await.unwrap();
        assert_eq!((team.team_id.as_str(), team.bot_user_id.as_str()), ("T0EXAMPLE", "U0BOT"));
        let a = upsert_account(st, &org, None, "main", &team, "xoxb-test", None, "token").await.unwrap();
        let c = create_channel(st, &org, &a.id, "#checkout-alerts", Display { mute_buttons: true, ..Display::default() }).await.unwrap();
        assert!(matches!(create_channel(st, &org, &a.id, "Checkout-Alerts", Display::default()).await, Err(ChannelError::Duplicate(_))));

        // Both handle forms resolve; another org's handles don't.
        assert_eq!(resolve(st, &org, "slack-main-checkout-alerts").await.unwrap().map(|(_, c)| c.id), Some(c.id.clone()));
        assert_eq!(resolve(st, &org, "slack-checkout-alerts").await.unwrap().map(|(_, c)| c.id), Some(c.id.clone()));
        assert!(resolve(st, &other, "slack-main-checkout-alerts").await.unwrap().is_none());
        assert!(resolve(st, &org, "slack-main-nowhere").await.unwrap().is_none());

        let mut n = notice("alert");
        n.org = org.clone();
        let out = Outgoing { org: org.clone(), title: n.title(), email_body: String::new(), slack_text: "plain".into(), webhook: json!({}) };
        assert!(matches!(deliver(st, "slack-main-checkout-alerts", &out, Some((&n, "https://oc.example.com/monitors/7"))).await, Some(Outcome::Sent)));
        assert!(deliver(st, "slack-elsewhere", &out, None).await.is_none());
        assert!(matches!(deliver(st, "slack-checkout-alerts", &out, None).await, Some(Outcome::Sent)));
        let sent = posted.lock().unwrap().clone();
        assert_eq!(sent.len(), 2);
        // The conversation id was looked up and kept.
        assert_eq!(sent[0]["channel"], "C0CHECKOUT");
        assert_eq!(sent[0]["attachments"][0]["blocks"][3]["elements"].as_array().unwrap().len(), 4);
        assert_eq!(sent[1]["text"], "plain");
        assert_eq!(channel_by_name(st, &org, &a.id, "checkout-alerts").await.unwrap().unwrap().channel_id.as_deref(), Some("C0CHECKOUT"));

        // Renaming forgets the conversation id; deleting the account removes its channels.
        let renamed = update_channel(st, &org, &c, Some("#billing-private"), Display::default()).await.unwrap();
        assert_eq!((renamed.name.as_str(), renamed.channel_id.as_deref()), ("#billing-private", None));
        sqlx::query("DELETE FROM slack_accounts WHERE id::text = $1").bind(&a.id).execute(&st.postgres).await.unwrap();
        assert!(channels(st, &org, &a.id).await.unwrap().is_empty());
    }
}
