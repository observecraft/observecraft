//! "Connect" for connector packages authenticating with OAuth's authorization code flow (`auth.kind:
//! oauth2_authorization_code`, docs/connectors.md#auth): the admin is sent to the vendor's consent page and the vendor
//! sends the browser back to ObserveCraft's callback, which exchanges the code and saves the connection with its tokens.
//!
//! - The OAuth app is the operator's, registered with the vendor once per install (redirect URI
//!   `<OC_PUBLIC_URL>/api/v1/public/connectors/oauth/callback`), its client in `OC_OAUTH_<ID>_CLIENT_ID` /
//!   `OC_OAUTH_<ID>_CLIENT_SECRET` (`<ID>` the package id upper-cased, `-` and `.` as `_`), unless the package lets each
//!   connection bring its own (`client_id` over its fields).
//! - The flow's state (Postgres `connector_oauth_states`, migration 0104) is random, single use, 10 minutes, and
//!   names the org, the admin, the connector and what the admin entered (secrets sealed), with the PKCE verifier
//!   (sealed). The callback takes nothing else from the request but the code: the org and the connection come from
//!   the state.
//! - Tokens are sealed in the connection's secrets (`oauth_refresh_token`, else `oauth_access_token`), never answered;
//!   runs refresh the access token (`oauth2_refresh_token`'s grant, the rotated refresh token saved).

use axum::extract::{Path, Query, State};
use axum::response::{IntoResponse, Redirect, Response};
use axum::Json;
use oc_connectors::manifest::{Auth, Manifest, OAuthCode, OAUTH_ACCESS_TOKEN, OAUTH_CLIENT_ID, OAUTH_CLIENT_SECRET, OAUTH_REFRESH_TOKEN};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use utoipa::ToSchema;

use super::connectors::{check, ConnectionInput};
use crate::auth::{crypto, Access, Admin, AuthContext, Credential};
use crate::connectors::{registry, store, PackageConnector};
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;

/// The environment variable prefix of a package's OAuth app (`OC_OAUTH_GOOGLE_WORKSPACE_`).
pub fn env_prefix(connector: &str) -> String {
    format!("OC_OAUTH_{}_", connector.to_ascii_uppercase().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect::<String>())
}

/// The operator's OAuth app for a package: (client id, client secret).
pub fn operator_app(connector: &str) -> Option<(String, String)> {
    let p = env_prefix(connector);
    let id = std::env::var(format!("{p}CLIENT_ID")).ok().filter(|v| !v.trim().is_empty())?;
    Some((id.trim().to_string(), std::env::var(format!("{p}CLIENT_SECRET")).unwrap_or_default().trim().to_string()))
}

/// The authorization code flow a connection uses (its auth, or the choice its config picks).
fn flow<'a>(m: &'a Manifest, config: &Map<String, Value>) -> Option<&'a OAuthCode> {
    match m.auth.chosen(config) {
        Ok(Auth::Oauth2AuthorizationCode(c)) => Some(c),
        _ => None,
    }
}

/// Adds the operator's OAuth app to a connection's secrets for a run, a test or the flow (not when the connection
/// brings its own).
pub fn with_app(m: &Manifest, conn: &mut oc_connectors::engine::Connection) {
    if flow(m, &conn.config).is_some_and(|c| c.client_id.is_none()) {
        if let Some((id, secret)) = operator_app(&m.id) {
            conn.secrets.insert(OAUTH_CLIENT_ID.into(), Value::String(id));
            conn.secrets.insert(OAUTH_CLIENT_SECRET.into(), Value::String(secret));
        }
    }
}

fn redirect_uri(st: &AppState) -> String {
    format!("{}/api/v1/public/connectors/oauth/callback", st.config.public_url)
}

/// The vendor's consent page.
#[derive(Serialize, ToSchema, Debug)]
pub struct ConnectorOAuthStart {
    /// Open it in the browser: the vendor sends it back to ObserveCraft.
    pub url: String,
}

fn package(id: &str) -> Result<&'static std::sync::Arc<PackageConnector>, ApiError> {
    registry().package(id).ok_or_else(|| ApiError::NotFound(format!("No connector {id}")))
}

/// Starts "Connect" (admins): checks what was entered (`connectionId` reconnects a saved connection) and answers the
/// vendor's consent page.
#[utoipa::path(post, path = "/api/v1/connectors/{id}/oauth", operation_id = "start_connector_oauth", params(("id" = String, Path)), request_body = ConnectionInput,
    responses((status = 200, body = ConnectorOAuthStart), (status = 400, body = crate::error::ErrorBody)), tag = "integrations")]
pub async fn start(State(st): State<AppState>, Admin(ctx): Admin, Path(id): Path<String>, Json(input): Json<ConnectionInput>) -> ApiResult<ConnectorOAuthStart> {
    Ok(Json(start_with(&st, &ctx, package(&id)?, &input).await?))
}

async fn start_with(st: &AppState, ctx: &AuthContext, p: &PackageConnector, input: &ConnectionInput) -> Result<ConnectorOAuthStart, ApiError> {
    let (st, id) = (st.clone(), p.package.manifest.id.clone());
    let m = &p.package.manifest;
    let previous = match input.connection_id.as_deref() {
        Some(cid) => Some(store::get(&st, &ctx.org_id, &id, cid).await?.ok_or_else(|| ApiError::NotFound("No such connection".into()))?),
        None => None,
    };
    let checked = check(m, input, previous.as_ref(), st.config.portal_allow_private).map_err(ApiError::BadRequest)?;
    let config = oc_connectors::engine::effective_config(m, &checked.config);
    let c = flow(m, &config).ok_or_else(|| ApiError::BadRequest(format!("{} doesn't connect with OAuth", m.name)))?;
    let mut conn = oc_connectors::engine::Connection { id: "oauth".into(), name: checked.name.clone(), config: config.clone(), secrets: checked.secrets.clone() };
    if let Some(prev) = &previous {
        let kept = store::unseal(&st, &store::Saved { secrets: checked.kept.clone(), ..prev.clone() }).map_err(ApiError::BadRequest)?;
        conn.secrets.extend(kept.secrets);
    }
    with_app(m, &mut conn);
    let mut scope = Map::new();
    scope.insert("config".into(), Value::Object(conn.config.clone()));
    scope.insert("secrets".into(), Value::Object(conn.secrets.clone()));
    let state = crypto::random_token("ocoa_");
    let (verifier, challenge) = oc_connectors::auth::pkce();
    let url = oc_connectors::auth::authorize_url(c, &scope, &redirect_uri(&st), &state, c.pkce.then_some(challenge.as_str())).map_err(ApiError::BadRequest)?;
    let mut sealed = checked.kept.clone();
    sealed.extend(store::seal(&st, &checked.secrets));
    let saved_input = json!({ "name": checked.name, "config": checked.config, "streams": checked.streams, "enabled": input.enabled });
    sqlx::query("DELETE FROM connector_oauth_states WHERE created_at < now() - interval '10 minutes'").execute(&st.postgres).await?;
    sqlx::query("INSERT INTO connector_oauth_states (state, org_id, user_id, connector_id, connection_id, input, secrets, verifier) VALUES ($1, $2, $3::uuid, $4, $5::uuid, $6, $7, $8)")
        .bind(&state)
        .bind(&ctx.org_id)
        .bind(&ctx.user_id)
        .bind(&id)
        .bind(previous.as_ref().map(|p| p.id.clone()))
        .bind(saved_input)
        .bind(Value::Object(sealed))
        .bind(st.secrets.encrypt(&verifier))
        .execute(&st.postgres)
        .await?;
    ctx.audit(&st, "connectors.oauth_started", &id, &checked.name).await;
    Ok(ConnectorOAuthStart { url })
}

#[derive(Deserialize, Debug)]
pub struct Callback {
    code: Option<String>,
    state: Option<String>,
    error: Option<String>,
    error_description: Option<String>,
}

fn back(st: &AppState, connector: Option<&str>, key: &str, value: &str) -> Response {
    let page = connector.map_or_else(|| "/integrations".to_string(), |c| format!("/integrations/connectors/{c}"));
    let mut url = url::Url::parse(&format!("{}{page}", st.config.public_url)).unwrap_or_else(|_| url::Url::parse("http://localhost/integrations").expect("static url"));
    url.query_pairs_mut().append_pair(key, value);
    Redirect::to(url.as_str()).into_response()
}

type StateRow = (String, String, String, Option<String>, Value, Value, String);

/// The vendor sends the browser back here after its consent page; the state names the org, the admin and the
/// connection.
#[utoipa::path(get, path = "/api/v1/public/connectors/oauth/callback", operation_id = "connector_oauth_callback",
    params(("code" = Option<String>, Query), ("state" = Option<String>, Query), ("error" = Option<String>, Query)), responses((status = 303)), tag = "integrations")]
pub async fn callback(State(st): State<AppState>, Query(p): Query<Callback>) -> Response {
    callback_with(&st, registry(), p).await
}

async fn callback_with(st: &AppState, r: &crate::connectors::Registry, p: Callback) -> Response {
    let st = st.clone();
    let Some(state) = p.state.filter(|s| s.starts_with("ocoa_")) else { return back(&st, None, "oauth_error", "The sign-in came back without its state. Please try again.") };
    let row: Option<StateRow> = sqlx::query_as(
        "DELETE FROM connector_oauth_states WHERE state = $1 AND created_at > now() - interval '10 minutes' \
         RETURNING org_id, user_id::text, connector_id, connection_id::text, input, secrets, verifier",
    )
    .bind(&state)
    .fetch_optional(&st.postgres)
    .await
    .unwrap_or(None);
    let Some((org, user, connector, connection, input, sealed, verifier)) = row else { return back(&st, None, "oauth_error", "This sign-in expired. Please connect again.") };
    let fail = |m: &str| back(&st, Some(&connector), "oauth_error", m);
    if let Some(e) = p.error {
        return fail(&format!("The vendor didn't connect: {}", p.error_description.unwrap_or(e)));
    }
    let Some(code) = p.code.filter(|c| !c.is_empty()) else { return fail("The sign-in came back without a code") };
    let Some(pkg) = r.package(&connector) else { return fail("This connector is gone") };
    let m = &pkg.package.manifest;
    let config = oc_connectors::engine::effective_config(m, input["config"].as_object().unwrap_or(&Map::new()));
    let Some(c) = flow(m, &config) else { return fail("This connector doesn't connect with OAuth") };
    let unsealed = store::unseal(
        &st,
        &store::Saved { id: String::new(), name: String::new(), config: config.clone(), secrets: sealed.as_object().cloned().unwrap_or_default(), streams: vec![], enabled: true, updated_at: 0 },
    );
    let Ok(mut conn) = unsealed else { return fail("The entered secrets can't be read: enter them again") };
    let entered_secrets = conn.secrets.clone();
    with_app(m, &mut conn);
    let mut scope = Map::new();
    scope.insert("config".into(), Value::Object(config.clone()));
    scope.insert("secrets".into(), Value::Object(conn.secrets.clone()));
    let verifier = if c.pkce { st.secrets.decrypt(&verifier).ok() } else { None };
    let transport = crate::connectors::net::Net { allow_private: st.config.portal_allow_private };
    let answer = match oc_connectors::auth::exchange_code(c, &scope, &code, &redirect_uri(&st), verifier.as_deref(), &transport).await {
        Ok(a) => a,
        Err(e) => return fail(&e),
    };
    let mut tokens = Map::new();
    match answer["refresh_token"].as_str().filter(|t| !t.is_empty()) {
        Some(refresh) => tokens.insert(OAUTH_REFRESH_TOKEN.into(), json!(refresh)),
        None => tokens.insert(OAUTH_ACCESS_TOKEN.into(), answer["access_token"].clone()),
    };
    let ctx = AuthContext {
        user_id: user.clone(),
        org_id: org.clone(),
        account_id: user,
        instance_admin: false,
        name: "OAuth sign-in".into(),
        access: Access::Admin,
        credential: Credential::Session,
        session_hash: None,
        ip: None,
    };
    let entered = ConnectionInput {
        name: input["name"].as_str().unwrap_or_default().to_string(),
        config: input["config"].as_object().cloned().unwrap_or_default(),
        secrets: entered_secrets,
        streams: input["streams"].as_array().map(|a| a.iter().filter_map(|s| s.as_str().map(str::to_string)).collect()),
        enabled: input["enabled"].as_bool(),
        ..Default::default()
    };
    let previous = match connection {
        Some(cid) => match store::get(&st, &org, &connector, &cid).await {
            Ok(Some(p)) => Some(p),
            _ => return fail("The connection being reconnected is gone"),
        },
        None => None,
    };
    match super::connectors::save_with_tokens(&st, &ctx, pkg, previous.as_ref(), &entered, &tokens).await {
        Ok(v) => back(&st, Some(&connector), "oauth", &v.name),
        Err(ApiError::Conflict(m) | ApiError::BadRequest(m)) => fail(&m),
        Err(e) => {
            tracing::error!(error = %e, connector = %connector, "saving an OAuth connection failed");
            fail("The connection couldn't be saved")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::routing::post;

    const PACKAGE: &str = "schema_version: 1\nid: example-oauth\nname: Example OAuth\npublisher: Example Corp\nvendor: Example Corp\ndescription: Audit events.\n\
        config:\n  - { key: tenant, label: Tenant, type: string, required: true }\n\
        auth: { kind: oauth2_authorization_code, authorize_url: 'https://login.example.com/{{ config.tenant }}/authorize', token_url: 'TOKEN_URL', scopes: [audit.read], client_id: '{{ config.client_id }}', client_secret: '{{ secrets.client_secret }}' }\n\
        streams:\n  - id: events\n    name: Events\n    description: Events.\n    request: { url: 'https://api.example.com/{{ config.tenant }}/events' }\n    output:\n      logs: { message: '{{ record.id }}' }\n\
        test:\n  request: { url: 'https://api.example.com/{{ config.tenant }}/me' }\n  message: ok\n";

    fn registry_with(token_url: &str) -> crate::connectors::Registry {
        let dir = std::env::temp_dir().join(format!("oc-oauth-{}-{}", std::process::id(), token_url.len()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("example-oauth")).unwrap();
        let yaml = PACKAGE.replace("TOKEN_URL", token_url).replace(
            "config:\n",
            "config:\n  - { key: client_id, label: Client ID, type: string, required: true }\n  - { key: client_secret, label: Client secret, type: string, secret: true, required: true }\n",
        );
        std::fs::write(dir.join("example-oauth/connector.yaml"), yaml).unwrap();
        let r = crate::connectors::load(Some(dir.to_str().unwrap()));
        let _ = std::fs::remove_dir_all(&dir);
        assert!(r.invalid.is_empty(), "{:?}", r.invalid);
        r
    }

    fn redirected(r: &Response) -> url::Url {
        url::Url::parse(r.headers()["location"].to_str().unwrap()).unwrap()
    }

    /// The consent page, the callback exchanging the code with PKCE, the connection saved with its sealed refresh
    /// token; a state is used once, by its org only.
    #[tokio::test]
    async fn connect_signs_in_and_saves_the_tokens() {
        let Some((_, states)) = crate::test_support::replicas_with(1, |c| c.portal_allow_private = true).await else { return };
        let st = &states[0];
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let token_url = format!("http://{}/token", listener.local_addr().unwrap());
        let seen: std::sync::Arc<std::sync::Mutex<Vec<String>>> = Default::default();
        let got = seen.clone();
        tokio::spawn(async move {
            let app = axum::Router::new().route(
                "/token",
                post(move |body: String| async move {
                    got.lock().unwrap().push(body);
                    Json(json!({ "access_token": "at-1", "refresh_token": "rt-SECRET-1", "expires_in": 3600 }))
                }),
            );
            axum::serve(listener, app).await.unwrap();
        });
        let r = registry_with(&token_url);
        let p = r.package("example-oauth").unwrap();
        let a = crate::test_support::admin_of_new_org(st, "oauth-a").await;
        let input: ConnectionInput = serde_json::from_value(json!({ "config": { "tenant": "acme", "client_id": "their-app" }, "secrets": { "client_secret": "their-secret" } })).unwrap();
        let started = start_with(st, &a, p, &input).await.unwrap();
        let url = url::Url::parse(&started.url).unwrap();
        let q: std::collections::BTreeMap<String, String> = url.query_pairs().into_owned().collect();
        assert_eq!((url.host_str(), url.path()), (Some("login.example.com"), "/acme/authorize"));
        assert_eq!((q["client_id"].as_str(), q["redirect_uri"].ends_with("/api/v1/public/connectors/oauth/callback"), q["code_challenge_method"].as_str()), ("their-app", true, "S256"));
        let stored: Value = sqlx::query_scalar("SELECT secrets FROM connector_oauth_states WHERE state = $1").bind(&q["state"]).fetch_one(&st.postgres).await.unwrap();
        assert!(!stored.to_string().contains("their-secret"), "entered secrets are sealed");
        let back = callback_with(st, &r, Callback { code: Some("the-code".into()), state: Some(q["state"].clone()), error: None, error_description: None }).await;
        let to = redirected(&back);
        assert_eq!(to.path(), "/integrations/connectors/example-oauth");
        assert_eq!(to.query_pairs().find(|(k, _)| k == "oauth").map(|(_, v)| v.into_owned()), Some("example-oauth".into()), "{to}");
        let body = seen.lock().unwrap()[0].clone();
        assert!(body.contains("grant_type=authorization_code") && body.contains("code=the-code") && body.contains("code_verifier="), "{body}");
        let saved = store::list(st, &a.org_id, "example-oauth").await.unwrap().remove(0);
        assert!(!serde_json::to_string(&saved.secrets).unwrap().contains("rt-SECRET-1"), "sealed");
        let conn = store::unseal(st, &saved).unwrap();
        assert_eq!((conn.secrets["oauth_refresh_token"].as_str(), conn.secrets["client_secret"].as_str()), (Some("rt-SECRET-1"), Some("their-secret")));
        // Used once.
        let again = callback_with(st, &r, Callback { code: Some("the-code".into()), state: Some(q["state"].clone()), error: None, error_description: None }).await;
        assert!(redirected(&again).query_pairs().any(|(k, v)| k == "oauth_error" && v.contains("expired")));
        // Saving the connection again (streams) keeps its tokens.
        let kept = super::super::connectors::save_with_tokens(
            st,
            &a,
            p,
            Some(&saved),
            &serde_json::from_value(json!({ "config": { "tenant": "acme", "client_id": "their-app" }, "streams": ["events"] })).unwrap(),
            &Map::new(),
        )
        .await
        .unwrap();
        assert!(kept.secrets.contains(&"oauth_refresh_token".to_string()), "{:?}", kept.secrets);
        // The vendor refusing: the page says why, nothing saved.
        let b = crate::test_support::admin_of_new_org(st, "oauth-b").await;
        let other = start_with(st, &b, p, &input).await.unwrap();
        let state = url::Url::parse(&other.url).unwrap().query_pairs().find(|(k, _)| k == "state").unwrap().1.into_owned();
        let denied = callback_with(st, &r, Callback { code: None, state: Some(state), error: Some("access_denied".into()), error_description: Some("The admin declined".into()) }).await;
        assert!(redirected(&denied).query_pairs().any(|(k, v)| k == "oauth_error" && v.contains("declined")));
        assert!(store::list(st, &b.org_id, "example-oauth").await.unwrap().is_empty());
    }

    #[test]
    fn the_operators_app_is_named_after_the_package() {
        assert_eq!(env_prefix("google-workspace.audit"), "OC_OAUTH_GOOGLE_WORKSPACE_AUDIT_");
        assert_eq!(operator_app("no-such-package-for-tests"), None);
    }
}
