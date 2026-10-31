//! Personal Settings beyond the profile (`identity::me`): the member's preferences (home page,
//! time format, accessibility), the emails ObserveCraft sends them (dashboard reports they receive,
//! monitors that notify them) and email verification.
//!
//! - Preferences: Postgres `user_preferences`, one JSON document per member.
//! - Email verification: a single-use link (`email_verifications`, stored as SHA-256) sent through
//!   the notification SMTP settings (`OC_SMTP_URL`); opening it sets `users.email_verified_at`.
//! - Tenancy: every query filters on the org and member from the session.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::Redirect;
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;

use crate::auth::{crypto, AuthContext};
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;

// ================================================================ preferences

#[derive(Serialize, Deserialize, ToSchema, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Preferences {
    /// The page opened at `/` (an app path such as `/apm` or `/dashboards/abc-123`).
    pub home_page: Option<String>,
    /// 12h | 24h
    pub time_format: String,
    /// monday | sunday
    pub week_start: String,
    pub reduced_motion: bool,
    pub high_contrast: bool,
    pub underline_links: bool,
    /// Charts use a palette told apart without red/green.
    pub color_blind_palette: bool,
}

impl Default for Preferences {
    fn default() -> Self {
        Preferences {
            home_page: None,
            time_format: "12h".into(),
            week_start: "monday".into(),
            reduced_motion: false,
            high_contrast: false,
            underline_links: false,
            color_blind_palette: false,
        }
    }
}

impl Preferences {
    fn validate(mut self) -> Result<Self, ApiError> {
        if !matches!(self.time_format.as_str(), "12h" | "24h") {
            return Err(ApiError::BadRequest("time format is 12h or 24h".into()));
        }
        if !matches!(self.week_start.as_str(), "monday" | "sunday") {
            return Err(ApiError::BadRequest("the week starts on monday or sunday".into()));
        }
        self.home_page = self.home_page.map(|p| p.trim().to_string()).filter(|p| !p.is_empty() && p != "/");
        if let Some(p) = &self.home_page {
            // An app path only: no scheme, no protocol-relative URL.
            if !p.starts_with('/') || p.starts_with("//") || p.len() > 300 || p.contains(['\\', '\n']) {
                return Err(ApiError::BadRequest("the home page is a path of this app, such as /apm".into()));
            }
        }
        Ok(self)
    }
}

#[utoipa::path(get, path = "/api/v1/me/preferences", operation_id = "get_my_preferences", responses((status = 200, body = Preferences)), tag = "identity")]
pub async fn get_preferences(State(st): State<AppState>, ctx: AuthContext) -> ApiResult<Preferences> {
    let row: Option<(Value,)> = sqlx::query_as("SELECT prefs FROM user_preferences WHERE org_id = $1 AND user_id = $2::uuid").bind(&ctx.org_id).bind(&ctx.user_id).fetch_optional(&st.postgres).await?;
    Ok(Json(row.and_then(|(v,)| serde_json::from_value(v).ok()).unwrap_or_default()))
}

#[utoipa::path(put, path = "/api/v1/me/preferences", operation_id = "set_my_preferences", request_body = Preferences, responses((status = 200, body = Preferences)), tag = "identity")]
pub async fn set_preferences(State(st): State<AppState>, ctx: AuthContext, Json(p): Json<Preferences>) -> ApiResult<Preferences> {
    let p = p.validate()?;
    sqlx::query(
        "INSERT INTO user_preferences (org_id, user_id, prefs, updated_at) VALUES ($1, $2::uuid, $3, now()) \
         ON CONFLICT (org_id, user_id) DO UPDATE SET prefs = EXCLUDED.prefs, updated_at = now()",
    )
    .bind(&ctx.org_id)
    .bind(&ctx.user_id)
    .bind(serde_json::to_value(&p).map_err(anyhow::Error::from)?)
    .execute(&st.postgres)
    .await?;
    Ok(Json(p))
}

// ================================================================ email subscriptions

#[derive(Serialize, ToSchema, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct ReportSubscription {
    pub id: String,
    pub name: String,
    pub dashboard_id: String,
    pub dashboard_title: String,
    /// {"frequency": "weekly", "time": "09:00", …}
    #[schema(value_type = Object)]
    pub schedule: Value,
}

#[derive(Serialize, ToSchema, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct MonitorSubscription {
    pub id: i64,
    pub name: String,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct EmailSubscriptions {
    pub email: String,
    pub verified: bool,
    /// Email can be sent from this install (`OC_SMTP_URL`).
    pub email_enabled: bool,
    /// Scheduled dashboard reports sent to this address.
    pub reports: Vec<ReportSubscription>,
    /// Monitors whose message mentions `@<this address>`.
    pub monitors: Vec<MonitorSubscription>,
}

#[utoipa::path(get, path = "/api/v1/me/email-subscriptions", operation_id = "get_my_email_subscriptions", responses((status = 200, body = EmailSubscriptions)), tag = "identity")]
pub async fn email_subscriptions(State(st): State<AppState>, ctx: AuthContext) -> ApiResult<EmailSubscriptions> {
    let (email, verified): (String, bool) = sqlx::query_as("SELECT email, email_verified_at IS NOT NULL FROM users WHERE org_id = $1 AND id = $2::uuid")
        .bind(&ctx.org_id)
        .bind(&ctx.user_id)
        .fetch_one(&st.postgres)
        .await?;
    let reports: Vec<ReportSubscription> = sqlx::query_as(
        "SELECT r.id::text AS id, r.name, r.dashboard_id, d.title AS dashboard_title, r.schedule FROM dashboard_reports r JOIN dashboards d ON d.id = r.dashboard_id AND d.org_id = r.org_id \
         WHERE r.org_id = $1 AND lower($2) = ANY (SELECT lower(x) FROM unnest(r.recipients) x) ORDER BY r.name",
    )
    .bind(&ctx.org_id)
    .bind(&email)
    .fetch_all(&st.postgres)
    .await?;
    let monitors: Vec<MonitorSubscription> = sqlx::query_as("SELECT id, name FROM monitors WHERE org_id = $1 AND strpos(lower(message), lower('@' || $2)) > 0 ORDER BY name LIMIT 500")
        .bind(&ctx.org_id)
        .bind(&email)
        .fetch_all(&st.postgres)
        .await?;
    Ok(Json(EmailSubscriptions { email, verified, email_enabled: st.config.smtp_url.is_some(), reports, monitors }))
}

/// Stops a dashboard report from being sent to the member.
#[utoipa::path(delete, path = "/api/v1/me/email-subscriptions/reports/{id}", operation_id = "unsubscribe_my_report", params(("id" = String, Path)), responses((status = 204)), tag = "identity")]
pub async fn unsubscribe_report(State(st): State<AppState>, ctx: AuthContext, Path(id): Path<String>) -> Result<StatusCode, ApiError> {
    let n = sqlx::query(
        "UPDATE dashboard_reports r SET recipients = ARRAY(SELECT x FROM unnest(r.recipients) x WHERE lower(x) <> lower(u.email)) \
         FROM users u WHERE r.org_id = $1 AND r.id::text = $3 AND u.org_id = $1 AND u.id = $2::uuid AND lower(u.email) = ANY (SELECT lower(x) FROM unnest(r.recipients) x)",
    )
    .bind(&ctx.org_id)
    .bind(&ctx.user_id)
    .bind(&id)
    .execute(&st.postgres)
    .await?
    .rows_affected();
    if n == 0 {
        return Err(ApiError::NotFound("You don't receive this report".into()));
    }
    ctx.audit(&st, "dashboard_report.unsubscribe", &format!("dashboard_report:{id}"), "").await;
    Ok(StatusCode::NO_CONTENT)
}

// ================================================================ email verification

#[derive(Serialize, ToSchema)]
pub struct VerificationSent {
    pub email: String,
}

#[utoipa::path(post, path = "/api/v1/me/verify-email", operation_id = "send_email_verification", responses((status = 200, body = VerificationSent), (status = 400, body = crate::error::ErrorBody)), tag = "identity")]
pub async fn send_verification(State(st): State<AppState>, ctx: AuthContext) -> ApiResult<VerificationSent> {
    use lettre::message::header::ContentType;
    use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};
    let Some(smtp) = &st.config.smtp_url else {
        return Err(ApiError::BadRequest("Email isn't configured on this install (OC_SMTP_URL)".into()));
    };
    let (email, verified): (String, bool) = sqlx::query_as("SELECT email, email_verified_at IS NOT NULL FROM users WHERE org_id = $1 AND id = $2::uuid")
        .bind(&ctx.org_id)
        .bind(&ctx.user_id)
        .fetch_one(&st.postgres)
        .await?;
    if verified {
        return Err(ApiError::BadRequest("Your email is already verified".into()));
    }
    let token = crypto::random_token("ocev_");
    sqlx::query("DELETE FROM email_verifications WHERE org_id = $1 AND user_id = $2::uuid").bind(&ctx.org_id).bind(&ctx.user_id).execute(&st.postgres).await?;
    sqlx::query("INSERT INTO email_verifications (token_hash, org_id, user_id, email, expires_at) VALUES ($1, $2, $3::uuid, $4, now() + interval '1 day')")
        .bind(crypto::sha256(&token))
        .bind(&ctx.org_id)
        .bind(&ctx.user_id)
        .bind(&email)
        .execute(&st.postgres)
        .await?;
    let link = format!("{}/api/v1/auth/verify-email/{token}", st.config.public_url);
    let (Ok(from), Ok(to)) = (st.config.smtp_from.parse(), email.parse()) else {
        return Err(ApiError::BadRequest(format!("Invalid email address {email}")));
    };
    let body = format!("Open this link within a day to confirm that {email} is your address on ObserveCraft:\n\n{link}\n\nIf you didn't ask for it, ignore this email.\n");
    let msg = Message::builder().from(from).to(to).subject("Verify your ObserveCraft email").header(ContentType::TEXT_PLAIN).body(body).map_err(|e| ApiError::BadRequest(e.to_string()))?;
    let transport = AsyncSmtpTransport::<Tokio1Executor>::from_url(smtp).map_err(|e| ApiError::BadRequest(format!("Invalid OC_SMTP_URL: {e}")))?.build();
    transport.send(msg).await.map_err(|e| ApiError::BadRequest(format!("SMTP: {e}")))?;
    Ok(Json(VerificationSent { email }))
}

/// The link of the verification email: marks the address verified and opens Personal Settings.
#[utoipa::path(get, path = "/api/v1/auth/verify-email/{token}", operation_id = "verify_email", params(("token" = String, Path)), responses((status = 303)), tag = "auth")]
pub async fn verify(State(st): State<AppState>, Path(token): Path<String>) -> Result<Redirect, ApiError> {
    let row: Option<(String, String, String)> = sqlx::query_as("DELETE FROM email_verifications WHERE token_hash = $1 AND expires_at > now() RETURNING org_id, user_id::text, email")
        .bind(crypto::sha256(&token))
        .fetch_optional(&st.postgres)
        .await?;
    let base = &st.config.public_url;
    let Some((org, user, email)) = row else {
        return Ok(Redirect::to(&format!("{base}/account/profile?verified=expired")));
    };
    // Only if the address is still the one the link was sent to.
    sqlx::query("UPDATE users SET email_verified_at = now() WHERE org_id = $1 AND id = $2::uuid AND lower(email) = lower($3)").bind(&org).bind(&user).bind(&email).execute(&st.postgres).await?;
    Ok(Redirect::to(&format!("{base}/account/profile?verified=1")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preferences_are_checked() {
        let ok = Preferences { home_page: Some(" /apm ".into()), ..Default::default() }.validate().unwrap();
        assert_eq!(ok.home_page.as_deref(), Some("/apm"));
        assert_eq!(Preferences { home_page: Some("/".into()), ..Default::default() }.validate().unwrap().home_page, None);
        for bad in ["https://evil.example.com", "//evil.example.com", "apm"] {
            assert!(Preferences { home_page: Some(bad.into()), ..Default::default() }.validate().is_err(), "{bad}");
        }
        assert!(Preferences { time_format: "13h".into(), ..Default::default() }.validate().is_err());
        let p: Preferences = serde_json::from_value(serde_json::json!({"highContrast": true})).unwrap();
        assert!(p.high_contrast && p.time_format == "12h");
    }

    /// Preferences round-trip, and a report sent to the member shows and can be left. Skipped
    /// unless OC_TEST_CLICKHOUSE_URL and OC_TEST_POSTGRES_URL are set.
    #[tokio::test]
    async fn preferences_and_report_subscriptions() {
        let Some((_, states)) = crate::test_support::replicas(1).await else { return };
        let st = states[0].clone();
        let mut ctx = crate::test_support::admin_of_new_org(&st, "personal").await;
        let (user,): (String,) = sqlx::query_as("INSERT INTO users (id, org_id, email, name) VALUES (gen_random_uuid(), $1, $2, 'Migration Tester') RETURNING id::text")
            .bind(&ctx.org_id)
            .bind(format!("{}@example.com", ctx.org_id))
            .fetch_one(&st.postgres)
            .await
            .unwrap();
        ctx.user_id = user;
        assert_eq!(get_preferences(State(st.clone()), ctx.clone()).await.unwrap().0, Preferences::default());
        let p = Preferences { home_page: Some("/apm".into()), time_format: "24h".into(), high_contrast: true, ..Default::default() };
        let _ = set_preferences(State(st.clone()), ctx.clone(), Json(p.clone())).await.unwrap();
        assert_eq!(get_preferences(State(st.clone()), ctx.clone()).await.unwrap().0, p);

        let (email,): (String,) = sqlx::query_as("SELECT email FROM users WHERE id = $1::uuid").bind(&ctx.user_id).fetch_one(&st.postgres).await.unwrap();
        let dash = super::super::dashboards::new_id();
        sqlx::query("INSERT INTO dashboards (id, org_id, title, kind, teams, definition) VALUES ($1, $2, 'Shop', 'custom', '{}', '{}')").bind(&dash).bind(&ctx.org_id).execute(&st.postgres).await.unwrap();
        let (report,): (String,) = sqlx::query_as("INSERT INTO dashboard_reports (org_id, dashboard_id, name, time_frame, schedule, recipients) VALUES ($1, $2, 'Weekly', '1w', '{}', $3) RETURNING id::text")
            .bind(&ctx.org_id)
            .bind(&dash)
            .bind(vec![email.to_uppercase(), "slack-ops".to_string()])
            .fetch_one(&st.postgres)
            .await
            .unwrap();
        let subs = email_subscriptions(State(st.clone()), ctx.clone()).await.unwrap().0;
        assert_eq!(subs.reports.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(), [report.as_str()]);
        assert!(!subs.verified);
        assert_eq!(unsubscribe_report(State(st.clone()), ctx.clone(), Path(report.clone())).await.unwrap(), StatusCode::NO_CONTENT);
        let (left,): (Vec<String>,) = sqlx::query_as("SELECT recipients FROM dashboard_reports WHERE id::text = $1").bind(&report).fetch_one(&st.postgres).await.unwrap();
        assert_eq!(left, ["slack-ops"]);
        assert!(unsubscribe_report(State(st.clone()), ctx.clone(), Path(report)).await.is_err());
    }
}

