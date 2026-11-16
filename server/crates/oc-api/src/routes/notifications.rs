//! Monitor notifications: where the `@handles` in a monitor's message are delivered.
//!
//! - `@webhook-<name>`: JSON POST to the org's webhook channel, signed with HMAC-SHA256
//!   (`X-ObserveCraft-Signature: sha256=<hex>` over the raw body) when the channel has a secret.
//! - `@slack-<account>-<channel>` / `@slack-<channel>`: a channel of a connected Slack workspace
//!   ([`super::slack`]); otherwise `@slack-<name>` is a Slack incoming-webhook channel.
//! - `@someone@example.com`: email through the install's SMTP server (`OC_SMTP_URL`). Only addresses
//!   of the organization's members are mailed, so monitors can't be used to send mail to anyone.
//!   Emails are HTML and text ([`crate::email`]) with the group's graph; each transition has its
//!   own Message-ID and answers the one that opened the episode, so a recovery joins its alert's thread.
//! - `@oncall-<team>`: pages the team through On-Call (its escalation policy); a recovery resolves
//!   the monitor's open pages for that group. Tests only check that the team exists.
//! - `@workflow-<handle>`: starts the published workflow when it has a monitor trigger for the new
//!   state. Tests only check that it can be started.
//!
//! On-Call pages reuse the same channels to reach responders ([`send`]).
//! Each monitor transition that notifies creates one delivery per recipient; a background worker
//! sends them and retries failures with backoff (30 s, 2 min, 10 min, 30 min, then gives up).

use std::time::Duration;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::Sha256;
use utoipa::ToSchema;

use super::monitor_engine::recipients;
use crate::auth::{Admin, AuthContext, Writer};
use crate::email::{self, templates, Email};
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;

const MAX_ATTEMPTS: i32 = 5;
/// Seconds before retry n (1-based).
const BACKOFF_S: [i32; 4] = [30, 120, 600, 1800];

// ================================================================ channels

#[derive(Serialize, ToSchema, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct NotificationChannel {
    pub id: String,
    /// Mention it as `@<handle>` in a monitor message.
    pub handle: String,
    /// webhook | slack
    pub kind: String,
    /// Host of the URL (the full URL is a credential and never returned), or the Slack workspace and channel.
    #[sqlx(skip)]
    pub target: String,
    /// webhook: an incoming-webhook channel of this list; slack_app: a channel of a connected Slack
    /// workspace, managed in Integrations → Slack.
    #[sqlx(skip)]
    pub source: String,
    #[serde(skip)]
    pub url_enc: String,
    pub signed: bool,
    pub created_at: i64,
}

async fn channels(st: &AppState, org: &str) -> Result<Vec<NotificationChannel>, ApiError> {
    let mut rows: Vec<NotificationChannel> = sqlx::query_as(
        "SELECT id::text, handle, kind, url_enc, (secret_enc IS NOT NULL) AS signed, (extract(epoch FROM created_at) * 1000)::bigint AS created_at \
         FROM notification_channels WHERE org_id = $1 ORDER BY handle",
    )
    .bind(org)
    .fetch_all(&st.postgres)
    .await?;
    for r in &mut rows {
        r.target = st.secrets.decrypt(&r.url_enc).ok().and_then(|u| url::Url::parse(&u).ok()).and_then(|u| u.host_str().map(str::to_string)).unwrap_or_default();
        r.source = "webhook".into();
    }
    for a in super::slack::accounts(st, org).await? {
        for c in super::slack::channels(st, org, &a.id).await? {
            rows.push(NotificationChannel {
                id: c.id,
                handle: super::slack::handle(&a.name, &c.name),
                kind: "slack".into(),
                target: format!("{} · {}", if a.team_name.is_empty() { &a.name } else { &a.team_name }, c.name),
                source: "slack_app".into(),
                url_enc: String::new(),
                signed: false,
                created_at: c.created_at,
            });
        }
    }
    rows.sort_by(|a, b| a.handle.cmp(&b.handle));
    Ok(rows)
}

#[utoipa::path(get, path = "/api/v1/notification-channels", operation_id = "list_notification_channels", responses((status = 200, body = Vec<NotificationChannel>)), tag = "notifications")]
pub async fn list(State(st): State<AppState>, ctx: AuthContext) -> ApiResult<Vec<NotificationChannel>> {
    Ok(Json(channels(&st, &ctx.org_id).await?))
}

#[derive(Deserialize, ToSchema)]
pub struct CreateChannel {
    /// webhook | slack
    pub kind: String,
    /// Name after the prefix: `oncall` → `@webhook-oncall`.
    pub name: String,
    pub url: String,
    /// Webhooks only: signing secret.
    pub secret: Option<String>,
}

/// Webhook targets must be http(s) and not the cloud metadata endpoint.
/// Webhook URLs often carry a credential (Slack's do) and the payload a signature: they go over TLS.
/// Plain http only reaches a receiver on this machine (development).
fn encrypted_transport(u: &url::Url) -> bool {
    u.scheme() == "https" || (u.scheme() == "http" && matches!(u.host(), Some(url::Host::Domain("localhost")) | Some(url::Host::Ipv4(std::net::Ipv4Addr::LOCALHOST)) | Some(url::Host::Ipv6(std::net::Ipv6Addr::LOCALHOST))))
}

fn check_url(raw: &str) -> Result<url::Url, ApiError> {
    let u = url::Url::parse(raw.trim()).map_err(|_| ApiError::BadRequest("Enter a valid URL".into()))?;
    if !matches!(u.scheme(), "http" | "https") {
        return Err(ApiError::BadRequest("Only http(s) URLs are supported".into()));
    }
    if !encrypted_transport(&u) {
        return Err(ApiError::BadRequest("Webhook URLs must use https (plain http only to localhost)".into()));
    }
    if matches!(u.host_str(), Some("169.254.169.254") | Some("metadata.google.internal") | None) {
        return Err(ApiError::BadRequest("That host isn't allowed".into()));
    }
    Ok(u)
}

#[utoipa::path(post, path = "/api/v1/notification-channels", operation_id = "create_notification_channel", request_body = CreateChannel, responses((status = 200, body = NotificationChannel)), tag = "notifications")]
pub async fn create(State(st): State<AppState>, Admin(ctx): Admin, Json(req): Json<CreateChannel>) -> ApiResult<NotificationChannel> {
    if req.kind != "webhook" && req.kind != "slack" {
        return Err(ApiError::BadRequest("kind must be webhook or slack".into()));
    }
    let name = req.name.trim().to_lowercase();
    if name.is_empty() || name.len() > 60 || !name.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_') {
        return Err(ApiError::BadRequest("Names are 1–60 lowercase letters, digits, dashes or underscores".into()));
    }
    let url = check_url(&req.url)?;
    if req.kind == "slack" && url.host_str() != Some("hooks.slack.com") {
        return Err(ApiError::BadRequest("Slack channels use an incoming-webhook URL (https://hooks.slack.com/services/…)".into()));
    }
    let handle = format!("{}-{name}", req.kind);
    let secret = req.secret.as_deref().map(str::trim).filter(|s| !s.is_empty() && req.kind == "webhook").map(|s| st.secrets.encrypt(s));
    sqlx::query("INSERT INTO notification_channels (org_id, handle, kind, url_enc, secret_enc, created_by) VALUES ($1, $2, $3, $4, $5, $6::uuid)")
        .bind(&ctx.org_id)
        .bind(&handle)
        .bind(&req.kind)
        .bind(st.secrets.encrypt(url.as_str()))
        .bind(secret)
        .bind(&ctx.user_id)
        .execute(&st.postgres)
        .await
        .map_err(|e| match &e {
            sqlx::Error::Database(d) if d.is_unique_violation() => ApiError::BadRequest(format!("@{handle} already exists")),
            _ => e.into(),
        })?;
    ctx.audit(&st, "notification_channel.created", &handle, url.host_str().unwrap_or("")).await;
    let all = channels(&st, &ctx.org_id).await?;
    all.into_iter().find(|c| c.handle == handle).map(Json).ok_or_else(|| ApiError::NotFound("channel".into()))
}

#[utoipa::path(delete, path = "/api/v1/notification-channels/{id}", operation_id = "delete_notification_channel", params(("id" = String, Path)), responses((status = 204)), tag = "notifications")]
pub async fn delete(State(st): State<AppState>, Admin(ctx): Admin, Path(id): Path<String>) -> Result<StatusCode, ApiError> {
    let row: Option<(String,)> = sqlx::query_as("DELETE FROM notification_channels WHERE org_id = $1 AND id::text = $2 RETURNING handle").bind(&ctx.org_id).bind(&id).fetch_optional(&st.postgres).await?;
    let Some((handle,)) = row else { return Err(ApiError::NotFound("Channel not found".into())) };
    ctx.audit(&st, "notification_channel.deleted", &handle, "").await;
    Ok(StatusCode::NO_CONTENT)
}

// ================================================================ sending

/// What a delivery says about the transition.
pub(crate) struct Notice {
    pub org: String,
    pub monitor_id: i64,
    pub monitor_name: String,
    pub priority: Option<i16>,
    pub tags: Vec<String>,
    pub group: String,
    pub from: String,
    pub to: String,
    pub value: Option<f64>,
    pub message: String,
    pub at: i64,
    pub test: bool,
    pub facts: EventFacts,
}

/// What the email adds to a transition: the monitor's query, the event and its thread.
#[derive(Default, Clone)]
pub(crate) struct EventFacts {
    pub event_id: Option<i64>,
    /// The event that opened this episode (the last one that left OK before this one).
    pub thread_event_id: Option<i64>,
    /// metric | apm | log | synthetics
    pub kind: String,
    pub query: String,
    pub definition: Option<serde_json::Value>,
    pub recipients: Vec<String>,
}

impl Notice {
    pub(crate) fn title(&self) -> String {
        let what = templates::state_label(&self.to);
        let group = if self.group.is_empty() || self.group == "*" { String::new() } else { format!(" on {}", self.group) };
        format!("{}[{what}] {}{group}", if self.test { "[Test] " } else { "" }, self.monitor_name)
    }
}

pub(crate) enum Outcome {
    Sent,
    /// Not a channel, member or supported handle: nothing to retry.
    Unknown(String),
    Failed(String),
}

fn sign(secret: &str, body: &[u8]) -> String {
    let mut mac = <Hmac<Sha256> as hmac::KeyInit>::new_from_slice(secret.as_bytes()).expect("hmac key");
    mac.update(body);
    mac.finalize().into_bytes().iter().map(|b| format!("{b:02x}")).collect()
}

/// One message, rendered for each kind of channel.
pub(crate) struct Outgoing {
    pub org: String,
    pub email: Email,
    pub slack_text: String,
    pub webhook: serde_json::Value,
}

impl Notice {
    /// The email: `graph` is the group's PNG when it could be drawn.
    pub(crate) fn email(&self, public_url: &str, domain: &str, graph: Option<Vec<u8>>) -> Email {
        let options: Option<super::monitor_engine::MonitorOptions> = self.facts.definition.as_ref().and_then(|d| serde_json::from_value(d.clone()).ok());
        let query = options.as_ref().map(|o| o.full_query(&self.facts.query)).unwrap_or_else(|| self.facts.query.clone());
        let facts = templates::MonitorFacts {
            monitor_id: self.monitor_id,
            name: &self.monitor_name,
            kind: &self.facts.kind,
            priority: self.priority,
            group: &self.group,
            to: &self.to,
            value: self.value,
            message: &self.message,
            at: self.at,
            test: self.test,
            query: &query,
            options: options.as_ref(),
            notified: &self.facts.recipients,
        };
        let mut e = templates::monitor(public_url, &self.title(), &facts, graph);
        if !self.test {
            e.message_id = self.facts.event_id.map(|id| templates::monitor_message_id(self.monitor_id, id, domain));
            e.in_reply_to = self.facts.thread_event_id.map(|id| templates::monitor_message_id(self.monitor_id, id, domain));
        }
        e
    }

    fn outgoing(&self, link: &str, public_url: &str, domain: &str, graph: Option<Vec<u8>>) -> Outgoing {
        let value = self.value.map(|v| format!(" (value {v})")).unwrap_or_default();
        Outgoing {
            org: self.org.clone(),
            email: self.email(public_url, domain, graph),
            slack_text: format!("*{}*{value}\n{}\n<{link}|View monitor>", self.title(), self.message.trim()),
            webhook: json!({
                "event": format!("monitor.{}", self.to),
                "test": self.test,
                "title": self.title(),
                "organization": self.org,
                "monitor": { "id": self.monitor_id, "name": self.monitor_name, "priority": self.priority, "tags": self.tags, "url": link },
                "group": self.group,
                "transition": { "from": self.from, "to": self.to },
                "value": self.value,
                "message": self.message,
                "timestamp": self.at,
            }),
        }
    }
}

pub(crate) async fn deliver(st: &AppState, recipient: &str, n: &Notice) -> Outcome {
    if let Some(team) = recipient.strip_prefix("oncall-") {
        return super::on_call::page_from_monitor(st, team, n).await;
    }
    if let Some(handle) = recipient.strip_prefix("workflow-") {
        return super::workflows::start_from_monitor(st, handle, n).await;
    }
    let link = format!("{}/monitors/{}", st.config.public_url, n.monitor_id);
    // Only emails show the graph: read it for them alone.
    let graph = if recipient.contains('@') { graph(st, n).await } else { None };
    let out = n.outgoing(&link, &st.config.public_url, &email::id_domain(&st.config.smtp_from), graph);
    if recipient.starts_with("slack-") {
        if let Some(outcome) = super::slack::deliver(st, recipient, &out, Some((n, &link))).await {
            return outcome;
        }
    }
    send(st, recipient, &out).await
}

/// How long reading a notification graph's series may take before the email goes without it.
const GRAPH_TIMEOUT: Duration = Duration::from_secs(5);

/// The group's series over the hour around the transition, drawn; None (logged) when it can't be
/// read or drawn: the email is sent without it.
async fn graph(st: &AppState, n: &Notice) -> Option<Vec<u8>> {
    if n.test || n.facts.event_id.is_none() || n.facts.kind == "synthetics" || n.facts.query.trim().is_empty() {
        return None;
    }
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis() as i64;
    let (from, to) = (n.at - 50 * 60_000, (n.at + 10 * 60_000).min(now.max(n.at + 60_000)));
    let read = tokio::time::timeout(GRAPH_TIMEOUT, super::monitors::group_series(st, &n.org, &n.facts.kind, &n.facts.query, &n.group, from, to)).await;
    let points = match read {
        Ok(Ok(p)) => p,
        Ok(Err(e)) => {
            tracing::warn!(monitor = n.monitor_id, error = %e, "notification graph: series not read, sending without it");
            return None;
        }
        Err(_) => {
            tracing::warn!(monitor = n.monitor_id, "notification graph: series timed out, sending without it");
            return None;
        }
    };
    let options: Option<super::monitor_engine::MonitorOptions> = n.facts.definition.as_ref().and_then(|d| serde_json::from_value(d.clone()).ok());
    let png = email::chart::render(&templates::monitor_chart(&n.to, options.as_ref(), points, from, to, n.at));
    if png.is_none() {
        tracing::info!(monitor = n.monitor_id, group = %n.group, "notification graph: no points to draw, sending without it");
    }
    png
}

/// Sends a message to an email address (of a member) or a channel handle (`slack-…`, `webhook-…`).
pub(crate) async fn send(st: &AppState, recipient: &str, out: &Outgoing) -> Outcome {
    if recipient.contains('@') {
        return send_email(st, recipient, out).await;
    }
    if recipient.starts_with("slack-") {
        if let Some(outcome) = super::slack::deliver(st, recipient, out, None).await {
            return outcome;
        }
    }
    let row: Option<(String, String, Option<String>)> =
        match sqlx::query_as("SELECT kind, url_enc, secret_enc FROM notification_channels WHERE org_id = $1 AND handle = $2").bind(&out.org).bind(recipient).fetch_optional(&st.postgres).await {
            Ok(r) => r,
            Err(e) => return Outcome::Failed(e.to_string()),
        };
    let Some((kind, url_enc, secret_enc)) = row else {
        let hint = if recipient.starts_with("slack-") { "Add the Slack channel under Integrations → Slack" } else { "Add it under Monitors → Settings → Notification Channels" };
        return Outcome::Unknown(format!("No channel named @{recipient}. {hint}."));
    };
    let (Ok(url), secret) = (st.secrets.decrypt(&url_enc), secret_enc.and_then(|s| st.secrets.decrypt(&s).ok())) else {
        return Outcome::Failed("The channel's URL can't be decrypted (was OC_SECRET_KEY changed?)".into());
    };
    // Channels created before https was required are refused here rather than sent in the clear.
    let url = match url::Url::parse(&url) {
        Ok(u) if encrypted_transport(&u) => u,
        _ => return Outcome::Failed("The channel's URL doesn't use https: edit the channel".into()),
    };
    let body = if kind == "slack" { json!({ "text": out.slack_text }) } else { out.webhook.clone() };
    let raw = serde_json::to_vec(&body).unwrap_or_default();
    let mut req = reqwest::Client::new().post(url).timeout(Duration::from_secs(10)).header("content-type", "application/json").header("user-agent", "ObserveCraft-Notifier");
    if let Some(s) = &secret {
        req = req.header("x-observecraft-signature", format!("sha256={}", sign(s, &raw)));
    }
    match req.body(raw).send().await {
        Ok(r) if r.status().is_success() => Outcome::Sent,
        Ok(r) => Outcome::Failed(format!("{} answered {}", if kind == "slack" { "Slack" } else { "The webhook" }, r.status())),
        Err(e) => Outcome::Failed(format!("Unreachable: {e}")),
    }
}

async fn send_email(st: &AppState, to: &str, out: &Outgoing) -> Outcome {
    let Some(smtp) = &st.config.smtp_url else {
        return Outcome::Unknown("Email isn't configured on this install (OC_SMTP_URL)".into());
    };
    let member: Result<bool, _> = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM users WHERE org_id = $1 AND lower(email) = lower($2) AND status = 'active')").bind(&out.org).bind(to).fetch_one(&st.postgres).await;
    if !member.unwrap_or(false) {
        return Outcome::Unknown(format!("{to} isn't an active member of this organization"));
    }
    match email::send(smtp, &st.config.smtp_from, to, &out.email).await {
        Ok(()) => Outcome::Sent,
        Err(email::SendError::Invalid(e)) => Outcome::Unknown(e),
        Err(email::SendError::Failed(e)) => Outcome::Failed(e),
    }
}

// ================================================================ test

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct TestNotification {
    /// Monitor (for its link, name, tags); absent while creating one.
    pub monitor_id: Option<i64>,
    pub monitor_name: String,
    /// The rendered message; its `@handles` are the recipients.
    pub message: String,
    /// alert | warn | ok | nodata
    pub status: String,
    pub value: Option<f64>,
}

#[derive(Serialize, ToSchema)]
pub struct TestDelivery {
    pub recipient: String,
    /// sent | failed | unknown_recipient
    pub status: String,
    pub error: Option<String>,
}

/// Sends one transition's message to its recipients now, marked as a test.
#[utoipa::path(post, path = "/api/v1/monitors/test-notification", operation_id = "test_monitor_notification", request_body = TestNotification, responses((status = 200, body = Vec<TestDelivery>)), tag = "notifications")]
pub async fn test(State(st): State<AppState>, Writer(ctx): Writer, Json(req): Json<TestNotification>) -> ApiResult<Vec<TestDelivery>> {
    let who = recipients(&req.message);
    if who.is_empty() {
        return Err(ApiError::BadRequest("The message mentions no @recipients".into()));
    }
    if who.len() > 20 {
        return Err(ApiError::BadRequest("At most 20 recipients per test".into()));
    }
    let n = Notice {
        org: ctx.org_id.clone(),
        monitor_id: req.monitor_id.unwrap_or(0),
        monitor_name: req.monitor_name.chars().take(200).collect(),
        priority: None,
        tags: Vec::new(),
        group: String::new(),
        from: if req.status == "ok" { "alert".into() } else { "ok".into() },
        to: req.status.clone(),
        value: req.value,
        message: req.message.chars().take(10_000).collect(),
        at: std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis() as i64,
        test: true,
        facts: EventFacts::default(),
    };
    let mut out = Vec::new();
    for r in who {
        let (status, error) = match deliver(&st, &r, &n).await {
            Outcome::Sent => ("sent", None),
            Outcome::Unknown(e) => ("unknown_recipient", Some(e)),
            Outcome::Failed(e) => ("failed", Some(e)),
        };
        out.push(TestDelivery { recipient: r, status: status.into(), error });
    }
    ctx.audit(&st, "monitor.test_notification", &req.monitor_name, &format!("{} recipients", out.len())).await;
    Ok(Json(out))
}

// ================================================================ deliveries

#[derive(Serialize, ToSchema, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct Delivery {
    pub id: i64,
    pub monitor_id: i64,
    pub monitor_name: String,
    pub group_key: String,
    pub to_status: String,
    pub recipient: String,
    /// pending | sent | failed | unknown_recipient
    pub status: String,
    pub attempts: i32,
    pub error: Option<String>,
    pub created_at: i64,
    pub sent_at: Option<i64>,
}

/// The org's most recent notification deliveries.
#[utoipa::path(get, path = "/api/v1/notification-deliveries", operation_id = "list_notification_deliveries", responses((status = 200, body = Vec<Delivery>)), tag = "notifications")]
pub async fn deliveries(State(st): State<AppState>, ctx: AuthContext) -> ApiResult<Vec<Delivery>> {
    let rows = sqlx::query_as(
        "SELECT d.id, e.monitor_id, m.name AS monitor_name, e.group_key, e.to_status, d.recipient, d.status, d.attempts, d.error, \
         (extract(epoch FROM d.created_at) * 1000)::bigint AS created_at, (extract(epoch FROM d.sent_at) * 1000)::bigint AS sent_at \
         FROM notification_deliveries d JOIN monitor_events e ON e.id = d.event_id JOIN monitors m ON m.id = e.monitor_id \
         WHERE d.org_id = $1 ORDER BY d.created_at DESC LIMIT 200",
    )
    .bind(&ctx.org_id)
    .fetch_all(&st.postgres)
    .await?;
    Ok(Json(rows))
}

#[derive(sqlx::FromRow)]
struct Due {
    id: i64,
    org_id: String,
    recipient: String,
    attempts: i32,
    monitor_id: i64,
    monitor_name: String,
    priority: Option<i16>,
    tags: Vec<String>,
    group_key: String,
    from_status: String,
    to_status: String,
    value: Option<f64>,
    message: String,
    at: i64,
    event_id: i64,
    thread_event_id: Option<i64>,
    kind: String,
    query: String,
    definition: Option<serde_json::Value>,
    recipients: Vec<String>,
}

/// Sends due deliveries every 10 seconds (rows are claimed with SKIP LOCKED, so several servers can run it).
pub async fn run_notifier(st: AppState) {
    let mut tick = tokio::time::interval(Duration::from_secs(10));
    loop {
        tick.tick().await;
        if let Err(e) = notify_tick(&st).await {
            tracing::warn!(error = %e, "notification delivery tick failed");
        }
    }
}

async fn notify_tick(st: &AppState) -> Result<(), ApiError> {
    let mut tx = st.postgres.begin().await?;
    let due: Vec<Due> = sqlx::query_as(
        "SELECT d.id, d.org_id, d.recipient, d.attempts, e.monitor_id, m.name AS monitor_name, m.priority, m.tags, e.group_key, e.from_status, e.to_status, e.value, e.message, \
         (extract(epoch FROM e.created_at) * 1000)::bigint AS at, e.id AS event_id, m.type AS kind, m.query, m.definition, e.recipients, \
         (SELECT p.id FROM monitor_events p WHERE p.monitor_id = e.monitor_id AND p.group_key = e.group_key AND p.id < e.id AND p.from_status = 'ok' ORDER BY p.id DESC LIMIT 1) AS thread_event_id \
         FROM notification_deliveries d JOIN monitor_events e ON e.id = d.event_id JOIN monitors m ON m.id = e.monitor_id \
         WHERE d.status = 'pending' AND d.next_attempt_at <= now() ORDER BY d.next_attempt_at LIMIT 50 FOR UPDATE OF d SKIP LOCKED",
    )
    .fetch_all(&mut *tx)
    .await?;
    for d in due {
        // A transition from OK opens its own thread.
        let opens_thread = d.from_status == "ok";
        let n = Notice {
            org: d.org_id,
            monitor_id: d.monitor_id,
            monitor_name: d.monitor_name,
            priority: d.priority,
            tags: d.tags,
            group: d.group_key,
            from: d.from_status,
            to: d.to_status,
            value: d.value,
            message: d.message,
            at: d.at,
            test: false,
            facts: EventFacts {
                event_id: Some(d.event_id),
                thread_event_id: d.thread_event_id.filter(|_| !opens_thread),
                kind: d.kind,
                query: d.query,
                definition: d.definition,
                recipients: d.recipients,
            },
        };
        let attempts = d.attempts + 1;
        let (status, error, retry_in) = match deliver(st, &d.recipient, &n).await {
            Outcome::Sent => ("sent", None, None),
            Outcome::Unknown(e) => ("unknown_recipient", Some(e), None),
            Outcome::Failed(e) if attempts >= MAX_ATTEMPTS => ("failed", Some(e), None),
            Outcome::Failed(e) => ("pending", Some(e), Some(BACKOFF_S[(attempts as usize - 1).min(BACKOFF_S.len() - 1)])),
        };
        sqlx::query(
            "UPDATE notification_deliveries SET status = $2, attempts = $3, error = $4, sent_at = CASE WHEN $2 = 'sent' THEN now() ELSE sent_at END, \
             next_attempt_at = now() + make_interval(secs => $5) WHERE id = $1",
        )
        .bind(d.id)
        .bind(status)
        .bind(attempts)
        .bind(error)
        .bind(retry_in.unwrap_or(0) as f64)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn webhook_signature_is_hmac_sha256_hex() {
        // RFC 4231 test case 2.
        assert_eq!(sign("Jefe", b"what do ya want for nothing?"), "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843");
    }

    #[test]
    fn webhook_urls_are_checked() {
        assert!(check_url("https://hooks.example.com/x").is_ok());
        assert!(check_url("ftp://example.com").is_err());
        assert!(check_url("http://169.254.169.254/latest/meta-data").is_err());
        assert!(check_url("not a url").is_err());
        // Over TLS only, except to this machine.
        assert!(check_url("http://hooks.example.com/x").is_err());
        assert!(check_url("http://localhost:9000/hook").is_ok());
        assert!(check_url("http://127.0.0.1:9000/hook").is_ok());
    }

    fn notice(from: &str, to: &str) -> Notice {
        Notice {
            org: "o".into(),
            monitor_id: 1,
            monitor_name: "High error rate".into(),
            priority: None,
            tags: vec![],
            group: "service:billing".into(),
            from: from.into(),
            to: to.into(),
            value: Some(5.0),
            message: String::new(),
            at: 0,
            test: false,
            facts: EventFacts { event_id: Some(9), thread_event_id: Some(7), kind: "metric".into(), query: "avg:system.load.1{*} by {service}".into(), ..EventFacts::default() },
        }
    }

    #[test]
    fn titles() {
        let n = notice("ok", "alert");
        assert_eq!(n.title(), "[Triggered] High error rate on service:billing");
        let out = n.outgoing("https://oc.example.com/monitors/1", "https://oc.example.com", "oc.example.com", None);
        assert_eq!(out.slack_text, "*[Triggered] High error rate on service:billing* (value 5)\n\n<https://oc.example.com/monitors/1|View monitor>");
        assert_eq!(out.email.subject, "[Triggered] High error rate on service:billing");
        assert_eq!(out.webhook["transition"]["to"], "alert");
        assert_eq!(out.webhook["monitor"]["url"], "https://oc.example.com/monitors/1");
        let subject = |to: &str| notice("alert", to).title();
        assert_eq!(subject("warn"), "[Warn] High error rate on service:billing");
        assert_eq!(subject("nodata"), "[No data] High error rate on service:billing");
        assert_eq!(subject("ok"), "[Recovered] High error rate on service:billing");
        assert_eq!(Notice { test: true, ..notice("ok", "alert") }.title(), "[Test] [Triggered] High error rate on service:billing");
    }

    #[test]
    fn emails_thread_each_episode() {
        // The recovery answers the alert that opened the episode.
        let e = notice("alert", "ok").email("https://oc.example.com", "oc.example.com", None);
        assert_eq!(e.message_id.as_deref(), Some("<monitor-1.event-9@oc.example.com>"));
        assert_eq!(e.in_reply_to.as_deref(), Some("<monitor-1.event-7@oc.example.com>"));
        // Tests get the transport's own id and no thread.
        let t = Notice { test: true, ..notice("alert", "ok") }.email("https://oc.example.com", "oc.example.com", None);
        assert_eq!((t.message_id, t.in_reply_to), (None, None));
    }

    #[test]
    fn emails_go_without_the_graph_when_there_is_none() {
        let e = notice("ok", "alert").email("https://oc.example.com", "oc.example.com", None);
        assert!(!e.html.contains(templates::GRAPH_CID));
        assert_eq!(e.images.len(), 1);
        let png = email::chart::render(&templates::monitor_chart("alert", None, vec![(0, 1.0), (60_000, 6.0)], 0, 120_000, 60_000));
        let e = notice("ok", "alert").email("https://oc.example.com", "oc.example.com", png);
        assert!(e.html.contains(templates::GRAPH_CID));
        assert_eq!(e.images.len(), 2);
    }
}
