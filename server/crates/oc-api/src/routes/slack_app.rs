//! What Slack sends the ObserveCraft Slack app (ADR-0010): `/observecraft` commands, Events API callbacks and the
//! organization choice. Requests are verified against the signing secret of a connected workspace's app; who acts is
//! the organization member whose email the Slack user's profile has, never anything the request says.

use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::{json, Value};

use super::slack::{email_of, respond, verified, AccountRow};
use crate::auth::{access_from_roles, Access};
use crate::state::AppState;

/// An `/observecraft` command.
#[derive(Debug, PartialEq)]
pub enum Command {
    Help,
    Org,
    Unknown(String),
}

pub fn parse_command(text: &str) -> Command {
    match text.split_whitespace().next().map(str::to_lowercase).as_deref() {
        None | Some("help") => Command::Help,
        Some("org") => Command::Org,
        Some(other) => Command::Unknown(other.to_string()),
    }
}

fn help_text() -> String {
    "*ObserveCraft in Slack*\n\
• `/observecraft help`: this list\n\
• `/observecraft org`: the organization you act in, and switching it when this workspace is connected to several\n\
You act as the ObserveCraft member with your Slack profile's email, with that member's role."
        .into()
}

fn ephemeral(text: String) -> Value {
    json!({ "response_type": "ephemeral", "text": text })
}

/// The organization member a Slack user acts as.
#[derive(Debug, Clone)]
pub struct Member {
    pub user_id: String,
    pub name: String,
    pub email: String,
    pub access: Access,
}

/// The active member of `org` with this email.
pub async fn member_in(st: &AppState, org: &str, email: &str) -> Result<Option<Member>, sqlx::Error> {
    let row: Option<(String, String, String, Vec<String>)> = sqlx::query_as(
        "SELECT u.id::text, u.name, u.email, COALESCE((SELECT array_agg(role_id) FROM user_roles WHERE user_id = u.id), '{}') \
         FROM users u WHERE u.org_id = $1 AND lower(u.email) = lower($2) AND u.status = 'active'",
    )
    .bind(org)
    .bind(email)
    .fetch_optional(&st.postgres)
    .await?;
    Ok(row.map(|(user_id, name, email, roles)| Member { user_id, name, email, access: access_from_roles(org, &roles) }))
}

/// Where a Slack user's request runs.
pub enum Acting {
    /// In this workspace's organization, as this member.
    As(Box<AccountRow>, Member),
    /// The user is a member of several of the organizations the workspace is connected to: they pick one (with
    /// each organization's name).
    Ask(Vec<(AccountRow, String)>),
    /// Not a member of any: the reason, for the user.
    Refused(String),
}

/// Which of the workspace's connections (`accounts`, all verified for the request) a Slack user acts in: the one
/// of the organization they are a member of, the one they chose when they are in several, or a question.
pub async fn acting(st: &AppState, accounts: Vec<AccountRow>, slack_user: &str) -> Acting {
    let Some(first) = accounts.first() else { return Acting::Refused("This Slack workspace isn't connected to ObserveCraft".into()) };
    let email = match email_of(st, first, slack_user).await {
        Ok(e) => e,
        Err(why) => return Acting::Refused(why),
    };
    let mut theirs = Vec::new();
    for a in accounts {
        match member_in(st, &a.org_id, &email).await {
            Ok(Some(m)) => theirs.push((a, m)),
            Ok(None) => {}
            Err(e) => return Acting::Refused(format!("ObserveCraft couldn't look you up: {e}")),
        }
    }
    if theirs.len() > 1 {
        let chosen: Option<(String,)> = sqlx::query_as("SELECT account_id::text FROM slack_user_orgs WHERE team_id = $1 AND slack_user_id = $2").bind(&theirs[0].0.team_id).bind(slack_user).fetch_optional(&st.postgres).await.unwrap_or(None);
        if let Some(i) = chosen.and_then(|(id,)| theirs.iter().position(|(a, _)| a.id == id)) {
            theirs.swap(0, i);
            theirs.truncate(1);
        }
    }
    match theirs.len() {
        0 => Acting::Refused(format!("{email} isn't a member of an ObserveCraft organization connected to this Slack workspace")),
        1 => {
            let (a, m) = theirs.remove(0);
            Acting::As(Box::new(a), m)
        }
        _ => {
            let mut options = Vec::new();
            for (a, _) in theirs {
                let name: Option<(String,)> = sqlx::query_as("SELECT name FROM orgs WHERE id = $1").bind(&a.org_id).fetch_optional(&st.postgres).await.unwrap_or(None);
                let name = name.map(|(n,)| n).unwrap_or_else(|| a.org_id.clone());
                options.push((a, name));
            }
            Acting::Ask(options)
        }
    }
}

/// The question when a Slack user is a member of several of the workspace's organizations.
fn org_choice(options: &[(AccountRow, String)]) -> Value {
    let buttons: Vec<Value> = options.iter().enumerate().map(|(i, (a, name))| json!({ "type": "button", "text": { "type": "plain_text", "text": name }, "action_id": format!("oc_org_{i}"), "value": a.id })).collect();
    json!({
        "response_type": "ephemeral",
        "text": "Which ObserveCraft organization do you act in from this workspace?",
        "blocks": [
            { "type": "section", "text": { "type": "mrkdwn", "text": "This Slack workspace is connected to several ObserveCraft organizations you're a member of. Which one do you act in from here? You can change it with `/observecraft org`." } },
            { "type": "actions", "elements": buttons },
        ],
    })
}

/// Records the organization (through its workspace connection) a Slack user acts in, when they are its member.
pub async fn choose_org(st: &AppState, account: &AccountRow, slack_user: &str) -> Result<String, String> {
    let email = email_of(st, account, slack_user).await?;
    let member = member_in(st, &account.org_id, &email).await.map_err(|e| e.to_string())?.ok_or_else(|| format!("{email} isn't a member of that organization"))?;
    sqlx::query("INSERT INTO slack_user_orgs (team_id, slack_user_id, account_id) VALUES ($1, $2, $3::uuid) ON CONFLICT (team_id, slack_user_id) DO UPDATE SET account_id = EXCLUDED.account_id, chosen_at = now()")
        .bind(&account.team_id)
        .bind(slack_user)
        .bind(&account.id)
        .execute(&st.postgres)
        .await
        .map_err(|e| e.to_string())?;
    let org: Option<(String,)> = sqlx::query_as("SELECT name FROM orgs WHERE id = $1").bind(&account.org_id).fetch_optional(&st.postgres).await.unwrap_or(None);
    Ok(format!("From this workspace you act in *{}* as {}.", org.map(|(n,)| n).unwrap_or_else(|| account.org_id.clone()), member.name))
}

/// Whether this is the first delivery of an Events API event (Slack retries until it gets a 200 in time).
pub async fn first_delivery(st: &AppState, event_id: &str) -> bool {
    let _ = sqlx::query("DELETE FROM slack_events WHERE received_at < now() - interval '1 day'").execute(&st.postgres).await;
    match sqlx::query("INSERT INTO slack_events (event_id) VALUES ($1) ON CONFLICT DO NOTHING").bind(event_id).execute(&st.postgres).await {
        Ok(r) => r.rows_affected() == 1,
        // Rather handled twice than never.
        Err(_) => true,
    }
}

/// `/observecraft …`
#[utoipa::path(post, path = "/api/v1/public/slack/commands", operation_id = "slack_command", request_body(content = String, content_type = "application/x-www-form-urlencoded"), responses((status = 200), (status = 401)), tag = "notifications")]
pub async fn commands(State(st): State<AppState>, headers: HeaderMap, body: Bytes) -> Response {
    let field = |k: &str| url::form_urlencoded::parse(&body).find(|(n, _)| n == k).map(|(_, v)| v.into_owned()).unwrap_or_default();
    let (team, slack_user, text, response_url) = (field("team_id"), field("user_id"), field("text"), field("response_url"));
    let Some(accounts) = verified(&st, &headers, &body, &team).await else { return StatusCode::UNAUTHORIZED.into_response() };
    match parse_command(&text) {
        Command::Help => Json(ephemeral(help_text())).into_response(),
        Command::Unknown(c) => Json(ephemeral(format!("Unknown command `{c}`.\n\n{}", help_text()))).into_response(),
        // Looking the user up calls Slack: answered after the 200, through the response URL.
        Command::Org => {
            tokio::spawn(async move {
                let message = match acting(&st, accounts, &slack_user).await {
                    Acting::As(a, m) => {
                        let org: Option<(String,)> = sqlx::query_as("SELECT name FROM orgs WHERE id = $1").bind(&a.org_id).fetch_optional(&st.postgres).await.unwrap_or(None);
                        ephemeral(format!("You act in *{}* as {} ({}).", org.map(|(n,)| n).unwrap_or(a.org_id), m.name, m.email))
                    }
                    Acting::Ask(options) => org_choice(&options),
                    Acting::Refused(why) => ephemeral(why),
                };
                respond(&st, &response_url, &message).await;
            });
            StatusCode::OK.into_response()
        }
    }
}

/// Events API callbacks (and the URL check Slack makes when the app's events URL is saved).
#[utoipa::path(post, path = "/api/v1/public/slack/events", operation_id = "slack_events", request_body = Object, responses((status = 200), (status = 401)), tag = "notifications")]
pub async fn events(State(st): State<AppState>, headers: HeaderMap, body: Bytes) -> Response {
    // The raw body is checked before anything reads it: one of this install's Slack apps signed it.
    if !super::slack::signed_by_known_app(&st, &headers, &body).await {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let Ok(payload) = serde_json::from_slice::<Value>(&body) else { return StatusCode::BAD_REQUEST.into_response() };
    if payload["type"] == "url_verification" {
        return Json(json!({ "challenge": payload["challenge"] })).into_response();
    }
    // Then that it is the app of a workspace connected to ObserveCraft as this team.
    let team = payload["team_id"].as_str().unwrap_or_default();
    if verified(&st, &headers, &body, team).await.is_none() {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    if let Some(id) = payload["event_id"].as_str() {
        if !first_delivery(&st, id).await {
            return StatusCode::OK.into_response();
        }
    }
    // Mentions, direct messages and shared links are handled from Stage 5 (Assist in Slack, link previews).
    StatusCode::OK.into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::routes::slack::signature;
    use std::sync::{Arc, Mutex};

    const SECRET: &str = "example-signing-secret";

    #[test]
    fn commands_are_read_from_the_text() {
        assert_eq!(parse_command(""), Command::Help);
        assert_eq!(parse_command("  help "), Command::Help);
        assert_eq!(parse_command("ORG"), Command::Org);
        assert_eq!(parse_command("deploy now"), Command::Unknown("deploy".into()));
    }

    // ------------------------------------------------------------ against Postgres and a fake Slack

    /// A fake Slack: `users.info` (U0ANA is ana@example.com, U0BEN ben@example.com, others have no email) and
    /// `response_url` hooks under `/hooks/`, recorded.
    async fn fake_slack() -> (String, Arc<Mutex<Vec<Value>>>) {
        use axum::routing::post as p;
        let hooks: Arc<Mutex<Vec<Value>>> = Arc::default();
        let rec = hooks.clone();
        let app = axum::Router::new()
            .route(
                "/users.info",
                p(|axum::Form(f): axum::Form<std::collections::HashMap<String, String>>| async move {
                    let email = match f.get("user").map(String::as_str) {
                        Some("U0ANA") => "ana@example.com",
                        Some("U0BEN") => "ben@example.com",
                        _ => "",
                    };
                    Json(json!({ "ok": true, "user": { "profile": { "email": email } } }))
                }),
            )
            .route(
                "/hooks/{*rest}",
                p(move |Json(body): Json<Value>| {
                    let rec = rec.clone();
                    async move {
                        rec.lock().unwrap().push(body);
                        "ok"
                    }
                }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (format!("http://{addr}"), hooks)
    }

    async fn state() -> Option<(AppState, Arc<Mutex<Vec<Value>>>, String)> {
        let (url, hooks) = fake_slack().await;
        let api = url.clone();
        let hook_base = format!("{url}/hooks/");
        let (_, states) = crate::test_support::replicas_with(1, |c| {
            c.slack_api_url = api.clone();
            c.slack_hooks_url = hook_base.clone();
        })
        .await?;
        Some((states[0].clone(), hooks, url))
    }

    /// A workspace (team) connected to the org with its own app's token and signing secret.
    async fn account(st: &AppState, org: &str, name: &str, team: &str) -> AccountRow {
        sqlx::query("INSERT INTO slack_accounts (org_id, name, team_id, team_name, bot_token_enc, signing_secret_enc, installed_via) VALUES ($1, $2, $3, 'Example Shop', $4, $5, 'token')")
            .bind(org)
            .bind(name)
            .bind(team)
            .bind(st.secrets.encrypt("xoxb-test"))
            .bind(st.secrets.encrypt(SECRET))
            .execute(&st.postgres)
            .await
            .unwrap();
        super::super::slack::accounts(st, org).await.unwrap().into_iter().find(|a| a.name == name).unwrap()
    }

    async fn user(st: &AppState, org: &str, email: &str, role: &str) -> String {
        let (id,): (String,) = sqlx::query_as("INSERT INTO users (id, org_id, email, name) VALUES (gen_random_uuid(), $1, $2, $3) RETURNING id::text").bind(org).bind(email).bind(email.split('@').next().unwrap()).fetch_one(&st.postgres).await.unwrap();
        sqlx::query("INSERT INTO roles (id, org_id, name, description, managed) VALUES ($1 || '-' || $2, $1, $2, '', true) ON CONFLICT DO NOTHING").bind(org).bind(role).execute(&st.postgres).await.unwrap();
        sqlx::query("INSERT INTO user_roles (user_id, role_id) VALUES ($1::uuid, $2)").bind(&id).bind(format!("{org}-{role}")).execute(&st.postgres).await.unwrap();
        id
    }

    fn now() -> String {
        (std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs()).to_string()
    }

    fn signed(secret: &str, timestamp: &str, body: &str) -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert("x-slack-request-timestamp", timestamp.parse().unwrap());
        h.insert("x-slack-signature", signature(secret, timestamp, body.as_bytes()).parse().unwrap());
        h
    }

    fn command_body(team: &str, user: &str, text: &str, hooks: &str) -> String {
        url::form_urlencoded::Serializer::new(String::new())
            .append_pair("team_id", team)
            .append_pair("user_id", user)
            .append_pair("command", "/observecraft")
            .append_pair("text", text)
            .append_pair("channel_id", "C0ALERTS")
            .append_pair("response_url", &format!("{hooks}/hooks/commands/1"))
            .finish()
    }

    async fn text_of(r: Response) -> (StatusCode, Value) {
        let status = r.status();
        let bytes = axum::body::to_bytes(r.into_body(), 1 << 20).await.unwrap();
        (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
    }

    async fn wait_for(hooks: &Arc<Mutex<Vec<Value>>>, n: usize) -> Vec<Value> {
        for _ in 0..50 {
            if hooks.lock().unwrap().len() >= n {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        hooks.lock().unwrap().clone()
    }

    #[tokio::test]
    async fn only_signed_recent_commands_of_connected_workspaces_are_answered() {
        let Some((st, _, url)) = state().await else { return };
        let org = crate::test_support::new_org(&st, "slackcmd").await;
        let team = format!("T{}", org.to_uppercase().replace('-', ""));
        account(&st, &org, "main", &team).await;
        let body = command_body(&team, "U0ANA", "help", &url);
        let call = |h: HeaderMap, b: String| commands(State(st.clone()), h, Bytes::from(b));

        assert_eq!(call(HeaderMap::new(), body.clone()).await.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(call(signed("other-secret", &now(), &body), body.clone()).await.status(), StatusCode::UNAUTHORIZED);
        let stale = (now().parse::<i64>().unwrap() - 600).to_string();
        assert_eq!(call(signed(SECRET, &stale, &body), body.clone()).await.status(), StatusCode::UNAUTHORIZED);
        let elsewhere = command_body("T0UNKNOWN", "U0ANA", "help", &url);
        assert_eq!(call(signed(SECRET, &now(), &elsewhere), elsewhere).await.status(), StatusCode::UNAUTHORIZED);

        let (status, v) = text_of(call(signed(SECRET, &now(), &body), body).await).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(v["response_type"], "ephemeral");
        assert!(v["text"].as_str().unwrap().contains("/observecraft org"), "{v}");
        let unknown = command_body(&team, "U0ANA", "deploy", &url);
        let (_, v) = text_of(call(signed(SECRET, &now(), &unknown), unknown).await).await;
        assert!(v["text"].as_str().unwrap().starts_with("Unknown command `deploy`"), "{v}");
    }

    #[tokio::test]
    async fn the_slack_user_acts_as_the_member_with_their_email() {
        let Some((st, hooks, url)) = state().await else { return };
        let org = crate::test_support::new_org(&st, "slackwho").await;
        let team = format!("T{}", org.to_uppercase().replace('-', ""));
        let a = account(&st, &org, "main", &team).await;
        let ana = user(&st, &org, "ana@example.com", "standard").await;
        user(&st, &org, "ben@example.com", "readonly").await;

        match acting(&st, vec![a.clone()], "U0ANA").await {
            Acting::As(acc, m) => {
                assert_eq!((acc.id, m.user_id, m.access), (a.id.clone(), ana, Access::Standard));
            }
            _ => panic!("Ana is a member"),
        }
        assert!(matches!(acting(&st, vec![a.clone()], "U0BEN").await, Acting::As(_, Member { access: Access::ReadOnly, .. })));
        match acting(&st, vec![a.clone()], "U0NOEMAIL").await {
            Acting::Refused(why) => assert!(why.contains("no email"), "{why}"),
            _ => panic!("no email, no member"),
        }

        // `/observecraft org` answers through the response URL, after the 200.
        let body = command_body(&team, "U0ANA", "org", &url);
        let r = commands(State(st.clone()), signed(SECRET, &now(), &body), Bytes::from(body)).await;
        assert_eq!(r.status(), StatusCode::OK);
        let posted = wait_for(&hooks, 1).await;
        assert_eq!(posted[0]["response_type"], "ephemeral");
        assert!(posted[0]["text"].as_str().unwrap().contains(&format!("*{org}* as ana")), "{}", posted[0]);
    }

    #[tokio::test]
    async fn a_workspace_connected_to_several_organizations_asks_once() {
        let Some((st, hooks, url)) = state().await else { return };
        let (first, second, third) = (crate::test_support::new_org(&st, "slackmany").await, crate::test_support::new_org(&st, "slackmany").await, crate::test_support::new_org(&st, "slackmany").await);
        let team = format!("T{}", first.to_uppercase().replace('-', ""));
        let (a1, a2, a3) = (account(&st, &first, "main", &team).await, account(&st, &second, "main", &team).await, account(&st, &third, "main", &team).await);
        user(&st, &first, "ana@example.com", "admin").await;
        user(&st, &second, "ana@example.com", "standard").await;
        let all = vec![a1.clone(), a2.clone(), a3.clone()];

        // A member of two of the three: asked to pick between those two.
        match acting(&st, all.clone(), "U0ANA").await {
            Acting::Ask(options) => {
                let mut orgs: Vec<String> = options.iter().map(|(a, _)| a.org_id.clone()).collect();
                orgs.sort();
                let mut want = vec![first.clone(), second.clone()];
                want.sort();
                assert_eq!(orgs, want);
            }
            _ => panic!("Ana must choose"),
        }
        // The choice is kept for the next requests; a workspace of an org they aren't in can't be chosen.
        assert!(choose_org(&st, &a3, "U0ANA").await.is_err());
        assert!(choose_org(&st, &a2, "U0ANA").await.unwrap().contains(&second));
        assert!(matches!(acting(&st, all.clone(), "U0ANA").await, Acting::As(a, _) if a.id == a2.id));

        // The command asks with one button per organization.
        sqlx::query("DELETE FROM slack_user_orgs WHERE team_id = $1").bind(&team).execute(&st.postgres).await.unwrap();
        let body = command_body(&team, "U0ANA", "org", &url);
        commands(State(st.clone()), signed(SECRET, &now(), &body), Bytes::from(body)).await;
        let posted = wait_for(&hooks, 1).await;
        let buttons = posted[0]["blocks"].as_array().unwrap().iter().find(|b| b["type"] == "actions").unwrap()["elements"].as_array().unwrap().clone();
        assert_eq!(buttons.len(), 2);
        assert!(buttons.iter().all(|b| b["action_id"].as_str().unwrap().starts_with("oc_org")));
    }

    #[tokio::test]
    async fn events_are_verified_and_handled_once() {
        let Some((st, _, _)) = state().await else { return };
        let org = crate::test_support::new_org(&st, "slackev").await;
        let team = format!("T{}", org.to_uppercase().replace('-', ""));
        account(&st, &org, "main", &team).await;

        // Slack checks the events URL with a challenge, signed by the app.
        let check = json!({ "type": "url_verification", "token": "x", "challenge": "c-123" }).to_string();
        let (status, v) = text_of(events(State(st.clone()), signed(SECRET, &now(), &check), Bytes::from(check.clone())).await).await;
        assert_eq!((status, v["challenge"].as_str()), (StatusCode::OK, Some("c-123")));
        assert_eq!(events(State(st.clone()), signed("other-secret", &now(), &check), Bytes::from(check)).await.status(), StatusCode::UNAUTHORIZED);
        // Nothing unsigned is read, not even as JSON (events, and button presses' payload).
        assert_eq!(events(State(st.clone()), HeaderMap::new(), Bytes::from_static(b"{not json")).await.status(), StatusCode::UNAUTHORIZED);
        let press = Bytes::from_static(b"payload=%7Bnot+json");
        assert_eq!(super::super::slack::interactive(State(st.clone()), HeaderMap::new(), press).await.status(), StatusCode::UNAUTHORIZED);

        let event_id = format!("Ev{}", org.replace('-', ""));
        let callback = json!({ "type": "event_callback", "team_id": team, "event_id": event_id, "event": { "type": "app_mention", "user": "U0ANA", "text": "<@U0BOT> hi" } }).to_string();
        assert_eq!(events(State(st.clone()), signed(SECRET, &now(), &callback), Bytes::from(callback.clone())).await.status(), StatusCode::OK);
        assert!(!first_delivery(&st, &event_id).await, "the callback recorded its event");
        assert!(first_delivery(&st, &format!("{event_id}-other")).await);
    }
}
