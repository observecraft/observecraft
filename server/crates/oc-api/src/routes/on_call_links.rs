//! Acknowledge and Resolve from an On-Call email, without signing in.
//!
//! The email's buttons carry a signed token ([`crate::email::signed_link`]) naming the org, the
//! page, the responder and the action; it expires after 24 h. Opening the link (GET) only shows a
//! confirmation page: mail scanners that fetch links can't act. The page's button POSTs the token,
//! which records its nonce (Postgres `email_action_uses`, single use) and acknowledges or resolves
//! the page as the responder. Org, page and actor come from the token alone, never from the request.

use axum::extract::{Form, Query, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use utoipa::IntoParams;

use super::on_call::set_page_status;
use super::on_call_escalation::PageAction;
use crate::email::frame::{colors, escape, FONT, INK};
use crate::email::signed_link::{self, Action, Claims, LinkError};
use crate::state::AppState;

/// The link the email's buttons open.
pub(crate) fn action_url(public_url: &str, secret: &str, org: &str, page: i64, user: &str, action: Action) -> String {
    format!("{public_url}/api/v1/public/on-call/actions?token={}", signed_link::issue(secret, org, page, user, action, now_s()))
}

fn now_s() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

#[derive(Deserialize, IntoParams)]
pub struct OnCallActionToken {
    /// The signed token of the email's link.
    pub token: String,
}

/// A small standalone page (no scripts, inline styles).
fn page(status: StatusCode, title: &str, body: &str) -> Response {
    let html = format!(
        "<!DOCTYPE html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><meta name=\"robots\" content=\"noindex\">\
         <title>{t} · ObserveCraft On-Call</title></head><body style=\"margin:0;background:#eef2f1;font-family:{FONT};color:{INK};\">\
         <div style=\"max-width:460px;margin:48px auto;padding:0 16px;\"><div style=\"background:#fff;border:1px solid #dfe7e5;border-radius:10px;overflow:hidden;\">\
         <div style=\"padding:14px 20px;font-weight:700;color:#073f3d;border-bottom:1px solid #edf1f0;\">ObserveCraft On-Call</div>\
         <div style=\"padding:20px;\"><h1 style=\"margin:0 0 10px;font-size:18px;\">{t}</h1>{body}</div></div></div></body></html>",
        t = escape(title)
    );
    let headers = [(header::CONTENT_TYPE, "text/html; charset=utf-8"), (header::CACHE_CONTROL, "no-store"), (header::REFERRER_POLICY, "no-referrer")];
    (status, headers, html).into_response()
}

fn para(text: &str) -> String {
    format!("<p style=\"margin:0 0 12px;line-height:1.5;color:#61716e;\">{}</p>", escape(text))
}

fn link_problem(e: LinkError) -> Response {
    match e {
        LinkError::Expired => page(StatusCode::GONE, "This link has expired", &para("Links in On-Call emails work for 24 hours. Open the page in ObserveCraft instead.")),
        LinkError::Invalid => page(StatusCode::BAD_REQUEST, "This link isn't valid", &para("It may be incomplete. Open the page in ObserveCraft instead.")),
    }
}

async fn used(st: &AppState, c: &Claims) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM email_action_uses WHERE org_id = $1 AND nonce = $2)").bind(&c.org).bind(&c.nonce).fetch_one(&st.postgres).await
}

/// The page and the responder's name, if the responder is still an active member and a responder of it.
async fn target(st: &AppState, c: &Claims) -> Result<Option<(String, String, String)>, sqlx::Error> {
    sqlx::query_as(
        "SELECT p.title, p.status, u.name FROM on_call_pages p JOIN on_call_page_responders r ON r.page_id = p.id \
         JOIN users u ON u.id = r.user_id AND u.org_id = p.org_id AND u.status = 'active' WHERE p.org_id = $1 AND p.id = $2 AND u.id::text = $3",
    )
    .bind(&c.org)
    .bind(c.page)
    .bind(&c.user)
    .fetch_optional(&st.postgres)
    .await
}

fn open_link(st: &AppState, c: &Claims) -> String {
    format!(
        "<a href=\"{}\" style=\"color:#0b6e69;\">Open the page in ObserveCraft</a>",
        escape(&format!("{}/on-call/pages/{}", st.config.public_url, c.page))
    )
}

fn server_error(e: sqlx::Error) -> Response {
    tracing::warn!(error = %e, "on-call email action");
    page(StatusCode::INTERNAL_SERVER_ERROR, "Something went wrong", &para("Try again in a moment, or open the page in ObserveCraft."))
}

/// Opening the link: a confirmation page with a button; nothing changes.
#[utoipa::path(get, path = "/api/v1/public/on-call/actions", operation_id = "on_call_email_action_page", params(OnCallActionToken),
    responses((status = 200, description = "Confirmation page (HTML)"), (status = 400), (status = 410)), tag = "on-call")]
pub async fn confirm(State(st): State<AppState>, Query(q): Query<OnCallActionToken>) -> Response {
    let c = match signed_link::verify(&st.config.secret_key, &q.token, now_s()) {
        Ok(c) => c,
        Err(e) => return link_problem(e),
    };
    match used(&st, &c).await {
        Ok(true) => return page(StatusCode::GONE, "This link was already used", &format!("{}{}", para("Each link works once."), open_link(&st, &c))),
        Ok(false) => {}
        Err(e) => return server_error(e),
    }
    let (title, status, name) = match target(&st, &c).await {
        Ok(Some(t)) => t,
        Ok(None) => return page(StatusCode::NOT_FOUND, "Page not found", &para("The page no longer exists or you're no longer one of its responders.")),
        Err(e) => return server_error(e),
    };
    let verb = c.action.verb();
    let body = format!(
        "{}{}<form method=\"post\" action=\"{}/api/v1/public/on-call/actions\" style=\"margin:16px 0;\"><input type=\"hidden\" name=\"token\" value=\"{}\">\
         <button type=\"submit\" style=\"font:600 14px/20px {FONT};padding:10px 18px;border-radius:6px;border:1px solid {bg};background:{bg};color:#fff;cursor:pointer;\">{verb} page #{}</button></form>{}",
        para(&format!("Page #{} · {title}", c.page)),
        para(&format!("Status: {status}. You act as {name}.")),
        escape(&st.config.public_url),
        escape(&q.token),
        c.page,
        open_link(&st, &c),
        bg = if c.action == Action::Acknowledge { colors::ALERT } else { "#0b6e69" },
    );
    page(StatusCode::OK, &format!("{verb} this page?"), &body)
}

/// The confirmation's button: acknowledges or resolves the page as the responder, once.
#[utoipa::path(post, path = "/api/v1/public/on-call/actions", operation_id = "on_call_email_action", request_body(content = String, content_type = "application/x-www-form-urlencoded", description = "token=<the link's token>"),
    responses((status = 200, description = "Result page (HTML)"), (status = 400), (status = 410)), tag = "on-call")]
pub async fn act(State(st): State<AppState>, Form(f): Form<OnCallActionToken>) -> Response {
    let c = match signed_link::verify(&st.config.secret_key, &f.token, now_s()) {
        Ok(c) => c,
        Err(e) => return link_problem(e),
    };
    let name = match target(&st, &c).await {
        Ok(Some((_, _, name))) => name,
        Ok(None) => return page(StatusCode::NOT_FOUND, "Page not found", &para("The page no longer exists or you're no longer one of its responders.")),
        Err(e) => return server_error(e),
    };
    // Single use: the first request to record the nonce acts.
    let first = sqlx::query("INSERT INTO email_action_uses (org_id, nonce) VALUES ($1, $2) ON CONFLICT DO NOTHING").bind(&c.org).bind(&c.nonce).execute(&st.postgres).await;
    match first {
        Ok(r) if r.rows_affected() == 0 => return page(StatusCode::GONE, "This link was already used", &format!("{}{}", para("Each link works once."), open_link(&st, &c))),
        Ok(_) => {}
        Err(e) => return server_error(e),
    }
    // Links live a day: older records can go.
    let _ = sqlx::query("DELETE FROM email_action_uses WHERE org_id = $1 AND used_at < now() - interval '2 days'").bind(&c.org).execute(&st.postgres).await;
    let action = if c.action == Action::Acknowledge { PageAction::Acknowledge } else { PageAction::Resolve };
    match set_page_status(&st, &c.org, c.page, action, &name).await {
        Ok(()) => {
            let done = if c.action == Action::Acknowledge { "acknowledged" } else { "resolved" };
            page(StatusCode::OK, &format!("Page #{} {done}", c.page), &format!("{}{}", para(&format!("Recorded as {name}.")), open_link(&st, &c)))
        }
        Err(e) => page(StatusCode::CONFLICT, "Nothing to do", &format!("{}{}", para(&e.to_string()), open_link(&st, &c))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn status_and_actor(st: &AppState, org: &str, page: i64) -> (String, Option<String>) {
        sqlx::query_as("SELECT status, acknowledged_by FROM on_call_pages WHERE org_id = $1 AND id = $2").bind(org).bind(page).fetch_one(&st.postgres).await.unwrap()
    }

    fn token_of(url: &str) -> String {
        url.split_once("token=").unwrap().1.to_string()
    }

    async fn body(r: Response) -> (StatusCode, String) {
        let status = r.status();
        let bytes = axum::body::to_bytes(r.into_body(), 1 << 20).await.unwrap();
        (status, String::from_utf8_lossy(&bytes).into_owned())
    }

    /// GET shows and doesn't act; POST acts once as the responder; replays, other orgs' or pages'
    /// tokens and expired ones are refused. Skipped unless OC_TEST_CLICKHOUSE_URL and
    /// OC_TEST_POSTGRES_URL are set.
    #[tokio::test]
    async fn signed_links_act_once_and_only_on_post() {
        let Some((_, states)) = crate::test_support::replicas(1).await else { return };
        let st = states[0].clone();
        let org = crate::test_support::new_org(&st, "oncall-links").await;
        let other = crate::test_support::new_org(&st, "oncall-links-other").await;
        let (user,): (String,) = sqlx::query_as("INSERT INTO users (id, org_id, email, name) VALUES (gen_random_uuid(), $1, $2, 'Ana Example') RETURNING id::text")
            .bind(&org)
            .bind(format!("ana-{org}@example.com"))
            .fetch_one(&st.postgres)
            .await
            .unwrap();
        let mut pages = Vec::new();
        for o in [&org, &org, &other] {
            let (id,): (i64,) = sqlx::query_as("INSERT INTO on_call_pages (org_id, title, source, created_by) VALUES ($1, 'order-service 5xx', 'manual', 'test') RETURNING id").bind(o).fetch_one(&st.postgres).await.unwrap();
            pages.push(id);
        }
        for p in &pages[..2] {
            sqlx::query("INSERT INTO on_call_page_responders (page_id, user_id) VALUES ($1, $2::uuid)").bind(p).bind(&user).execute(&st.postgres).await.unwrap();
        }
        let key = st.config.secret_key.clone();
        let ack = token_of(&action_url("https://oc.example.com", &key, &org, pages[0], &user, Action::Acknowledge));

        // GET: a confirmation, no change, the token not used.
        let (code, html) = body(confirm(State(st.clone()), Query(OnCallActionToken { token: ack.clone() })).await).await;
        assert_eq!(code, StatusCode::OK);
        assert!(html.contains("method=\"post\"") && html.contains("Acknowledge page #"));
        assert_eq!(status_and_actor(&st, &org, pages[0]).await, ("triggered".into(), None));

        // POST: acknowledged as the responder; the replay is refused.
        let (code, _) = body(act(State(st.clone()), Form(OnCallActionToken { token: ack.clone() })).await).await;
        assert_eq!(code, StatusCode::OK);
        assert_eq!(status_and_actor(&st, &org, pages[0]).await, ("acknowledged".into(), Some("Ana Example".into())));
        assert_eq!(body(act(State(st.clone()), Form(OnCallActionToken { token: ack.clone() })).await).await.0, StatusCode::GONE);
        assert_eq!(body(confirm(State(st.clone()), Query(OnCallActionToken { token: ack })).await).await.0, StatusCode::GONE);

        // A token for another org's page, or a page the user doesn't respond to, finds nothing.
        let foreign = token_of(&action_url("https://oc.example.com", &key, &other, pages[2], &user, Action::Resolve));
        assert_eq!(body(act(State(st.clone()), Form(OnCallActionToken { token: foreign })).await).await.0, StatusCode::NOT_FOUND);
        assert_eq!(status_and_actor(&st, &other, pages[2]).await.0, "triggered");
        let cross = token_of(&action_url("https://oc.example.com", &key, &org, pages[2], &user, Action::Resolve));
        assert_eq!(body(act(State(st.clone()), Form(OnCallActionToken { token: cross })).await).await.0, StatusCode::NOT_FOUND);

        // Expired, or signed with another key: refused before anything is read.
        let old = signed_link::issue(&key, &org, pages[1], &user, Action::Resolve, now_s() - signed_link::TTL_S - 1);
        assert_eq!(body(act(State(st.clone()), Form(OnCallActionToken { token: old })).await).await.0, StatusCode::GONE);
        let forged = signed_link::issue("another-install", &org, pages[1], &user, Action::Resolve, now_s());
        assert_eq!(body(act(State(st.clone()), Form(OnCallActionToken { token: forged })).await).await.0, StatusCode::BAD_REQUEST);
        assert_eq!(status_and_actor(&st, &org, pages[1]).await.0, "triggered");
    }
}
