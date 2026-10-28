//! Monitor notifications: where the `@handles` in a monitor's message are delivered.
//!
//! - `@webhook-<name>`: JSON POST to the org's webhook channel, signed with HMAC-SHA256
//!   (`X-ObserveCraft-Signature: sha256=<hex>` over the raw body) when the channel has a secret.
//! - `@slack-<name>`: Slack incoming webhook.
//! - `@someone@example.com`: email through the install's SMTP server (`OC_SMTP_URL`). Only addresses
//!   of the organization's members are mailed, so monitors can't be used to send mail to anyone.
//! - `@oncall-<team>`: pages the team through On-Call (its escalation policy); a recovery resolves
//!   the monitor's open pages for that group. Tests only check that the team exists.
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
    /// Host of the URL (the full URL is a credential and never returned).
    #[sqlx(skip)]
    pub target: String,
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
    }
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
fn check_url(raw: &str) -> Result<url::Url, ApiError> {
    let u = url::Url::parse(raw.trim()).map_err(|_| ApiError::BadRequest("Enter a valid URL".into()))?;
    if !matches!(u.scheme(), "http" | "https") {
        return Err(ApiError::BadRequest("Only http(s) URLs are supported".into()));
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
}

impl Notice {
    pub(crate) fn title(&self) -> String {
        let what = match self.to.as_str() {
            "alert" => "Triggered",
            "warn" => "Warn",
            "nodata" => "No Data",
            _ => "Recovered",
        };
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
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).expect("hmac key");
    mac.update(body);
    mac.finalize().into_bytes().iter().map(|b| format!("{b:02x}")).collect()
}

/// One message, rendered for each kind of channel.
pub(crate) struct Outgoing {
    pub org: String,
    /// Email subject and the bold first line elsewhere.
    pub title: String,
    pub email_body: String,
    pub slack_text: String,
    pub webhook: serde_json::Value,
}

impl Notice {
    fn outgoing(&self, link: &str) -> Outgoing {
        let value = self.value.map(|v| format!(" (value {v})")).unwrap_or_default();
        let email_value = self.value.map(|v| format!("Value: {v}\n")).unwrap_or_default();
        Outgoing {
            org: self.org.clone(),
            title: self.title(),
            email_body: format!("{}\n\n{}\n\n{email_value}Monitor: {link}\n", self.title(), self.message.trim()),
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
    let link = format!("{}/monitors/{}", st.config.public_url, n.monitor_id);
    send(st, recipient, &n.outgoing(&link)).await
}

/// Sends a message to an email address (of a member) or a channel handle (`slack-…`, `webhook-…`).
pub(crate) async fn send(st: &AppState, recipient: &str, out: &Outgoing) -> Outcome {
    if recipient.contains('@') {
        return send_email(st, recipient, out).await;
    }
    let row: Option<(String, String, Option<String>)> =
        match sqlx::query_as("SELECT kind, url_enc, secret_enc FROM notification_channels WHERE org_id = $1 AND handle = $2").bind(&out.org).bind(recipient).fetch_optional(&st.postgres).await {
            Ok(r) => r,
            Err(e) => return Outcome::Failed(e.to_string()),
        };
    let Some((kind, url_enc, secret_enc)) = row else {
        return Outcome::Unknown(format!("No channel named @{recipient}. Add it under Monitors → Settings → Notification Channels."));
    };
    let (Ok(url), secret) = (st.secrets.decrypt(&url_enc), secret_enc.and_then(|s| st.secrets.decrypt(&s).ok())) else {
        return Outcome::Failed("The channel's URL can't be decrypted (was OC_SECRET_KEY changed?)".into());
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
    use lettre::message::header::ContentType;
    use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};
    let Some(smtp) = &st.config.smtp_url else {
        return Outcome::Unknown("Email isn't configured on this install (OC_SMTP_URL)".into());
    };
    let member: Result<bool, _> = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM users WHERE org_id = $1 AND lower(email) = lower($2) AND status = 'active')").bind(&out.org).bind(to).fetch_one(&st.postgres).await;
    if !member.unwrap_or(false) {
        return Outcome::Unknown(format!("{to} isn't an active member of this organization"));
    }
    let (Ok(from), Ok(to_addr)) = (st.config.smtp_from.parse(), to.parse()) else {
        return Outcome::Unknown(format!("Invalid email address {to}"));
    };
    let email = match Message::builder().from(from).to(to_addr).subject(out.title.clone()).header(ContentType::TEXT_PLAIN).body(out.email_body.clone()) {
        Ok(m) => m,
        Err(e) => return Outcome::Failed(e.to_string()),
    };
    let transport = match AsyncSmtpTransport::<Tokio1Executor>::from_url(smtp) {
        Ok(t) => t.build(),
        Err(e) => return Outcome::Failed(format!("Invalid OC_SMTP_URL: {e}")),
    };
    match transport.send(email).await {
        Ok(_) => Outcome::Sent,
        Err(e) => Outcome::Failed(format!("SMTP: {e}")),
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
         (extract(epoch FROM e.created_at) * 1000)::bigint AS at \
         FROM notification_deliveries d JOIN monitor_events e ON e.id = d.event_id JOIN monitors m ON m.id = e.monitor_id \
         WHERE d.status = 'pending' AND d.next_attempt_at <= now() ORDER BY d.next_attempt_at LIMIT 50 FOR UPDATE OF d SKIP LOCKED",
    )
    .fetch_all(&mut *tx)
    .await?;
    for d in due {
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
    }

    #[test]
    fn titles() {
        let n = Notice {
            org: "o".into(),
            monitor_id: 1,
            monitor_name: "High error rate".into(),
            priority: None,
            tags: vec![],
            group: "service:billing".into(),
            from: "ok".into(),
            to: "alert".into(),
            value: Some(5.0),
            message: String::new(),
            at: 0,
            test: false,
        };
        assert_eq!(n.title(), "[Triggered] High error rate on service:billing");
        let out = n.outgoing("https://oc.example.com/monitors/1");
        assert_eq!(out.slack_text, "*[Triggered] High error rate on service:billing* (value 5)\n\n<https://oc.example.com/monitors/1|View monitor>");
        assert_eq!(out.email_body, "[Triggered] High error rate on service:billing\n\n\n\nValue: 5\nMonitor: https://oc.example.com/monitors/1\n");
        assert_eq!(out.webhook["transition"]["to"], "alert");
        assert_eq!(out.webhook["monitor"]["url"], "https://oc.example.com/monitors/1");
    }
}
