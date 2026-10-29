//! Key validation, as Datadog's clients call it at startup.
//!
//! - `GET /api/v1/validate`: the API key (`DD-API-KEY`, an ingest key `ocik_…`) is valid. Clients that
//!   authenticate with a bearer token send no API key; an application key in `Authorization: Bearer`
//!   is accepted instead.
//! - `GET /api/v2/validate_keys`: the application key (`DD-APPLICATION-KEY` or `Authorization: Bearer`,
//!   `ocak_…`) is valid, and so is the API key when one is sent. The Terraform provider calls it with
//!   its `bearer_token` in `DD-APPLICATION-KEY`.

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use serde::Serialize;
use utoipa::ToSchema;

use super::{app_key_user, bearer, ingest_key_org, keys, DdAuth, DdError, DdResult};
use crate::state::AppState;

/// Answer of `GET /api/v1/validate`.
#[derive(Serialize, ToSchema)]
pub struct DatadogKeyValidation {
    pub valid: bool,
}

/// Answer of `GET /api/v2/validate_keys`: always `ok`.
#[derive(Serialize, ToSchema)]
pub struct DatadogKeysStatus {
    pub status: String,
}

/// Datadog-compatible API key check (`DD-API-KEY: ocik_…`).
#[utoipa::path(get, path = "/api/v1/validate", operation_id = "datadog_validate_api_key",
    responses((status = 200, body = DatadogKeyValidation), (status = 403, body = super::DatadogErrors)),
    params(("DD-API-KEY" = Option<String>, Header, description = "Ingest key (ocik_…)")), tag = "datadog-api")]
pub async fn validate(State(st): State<AppState>, headers: HeaderMap) -> DdResult<DatadogKeyValidation> {
    let valid = match (keys(&headers).api_key, bearer(&headers)) {
        (Some(api_key), _) => ingest_key_org(&st, &api_key).await?.is_some(),
        (None, Some(token)) => app_key_user(&st, &token, None).await?.is_some(),
        (None, None) => return Err(DdError::new(StatusCode::FORBIDDEN, "API key required")),
    };
    if !valid {
        return Err(DdError::forbidden());
    }
    Ok(Json(DatadogKeyValidation { valid: true }))
}

/// Datadog-compatible check of the application key (and the API key, when sent).
#[utoipa::path(get, path = "/api/v2/validate_keys", operation_id = "datadog_validate_keys",
    responses((status = 200, body = DatadogKeysStatus), (status = 403, body = super::DatadogErrors)),
    params(("DD-APPLICATION-KEY" = Option<String>, Header, description = "Application key (ocak_…); or send it as Authorization: Bearer")), tag = "datadog-api")]
pub async fn validate_keys(_: DdAuth) -> Json<DatadogKeysStatus> {
    Json(DatadogKeysStatus { status: "ok".into() })
}

#[cfg(test)]
mod tests {
    use super::super::replay::{ingest_key, new_org_with_roles, user_with_app_key, Feature, TestServer};
    use super::super::DdAuth;
    use axum::routing::get;
    use reqwest::StatusCode;
    use serde_json::{json, Value};

    async fn get_json(url: &str, headers: &[(&str, &str)]) -> (StatusCode, Value) {
        let mut req = reqwest::Client::new().get(url);
        for (k, v) in headers {
            req = req.header(*k, *v);
        }
        let res = req.send().await.expect("request");
        let status = res.status();
        (status, res.json().await.unwrap_or(Value::Null))
    }

    #[tokio::test]
    async fn validate_accepts_ingest_keys_as_the_api_key() {
        let Some(s) = TestServer::start("dd-validate").await else { return };
        let url = format!("{}/api/v1/validate", s.base);
        assert_eq!(get_json(&url, &[("DD-API-KEY", &s.api_key)]).await, (StatusCode::OK, json!({ "valid": true })));
        assert_eq!(get_json(&url, &[("dd-api-key", &s.api_key), ("DD-APPLICATION-KEY", &s.app_key)]).await.0, StatusCode::OK);
        // The SDKs' bearer authentication sends no API key.
        assert_eq!(get_json(&url, &[("Authorization", &format!("Bearer {}", s.app_key))]).await.0, StatusCode::OK);

        let forbidden = (StatusCode::FORBIDDEN, json!({ "errors": ["Forbidden"] }));
        assert_eq!(get_json(&url, &[("DD-API-KEY", "ocik_not-a-key")]).await, forbidden);
        // An application key is not an API key.
        assert_eq!(get_json(&url, &[("DD-API-KEY", &s.app_key)]).await, forbidden);
        assert_eq!(get_json(&url, &[("Authorization", "Bearer ocak_not-a-key")]).await, forbidden);
        assert_eq!(get_json(&url, &[]).await, (StatusCode::FORBIDDEN, json!({ "errors": ["API key required"] })));

        sqlx::query("UPDATE ingest_keys SET revoked_at = now() WHERE org_id = $1").bind(&s.org).execute(&s.st.postgres).await.unwrap();
        assert_eq!(get_json(&url, &[("DD-API-KEY", &s.api_key)]).await, forbidden);
    }

    #[tokio::test]
    async fn validate_keys_checks_the_application_key() {
        let Some(s) = TestServer::start("dd-validate-keys").await else { return };
        let url = format!("{}/api/v2/validate_keys", s.base);
        let ok = (StatusCode::OK, json!({ "status": "ok" }));
        // The provider's bearer_token validation: the token in DD-APPLICATION-KEY, no API key.
        assert_eq!(get_json(&url, &[("DD-APPLICATION-KEY", &s.app_key)]).await, ok);
        assert_eq!(get_json(&url, &[("Authorization", &format!("Bearer {}", s.app_key))]).await, ok);
        assert_eq!(get_json(&url, &[("DD-API-KEY", &s.api_key), ("DD-APPLICATION-KEY", &s.app_key)]).await, ok);

        let forbidden = (StatusCode::FORBIDDEN, json!({ "errors": ["Forbidden"] }));
        // DD-API-KEY alone only passes /api/v1/validate.
        assert_eq!(get_json(&url, &[("DD-API-KEY", &s.api_key)]).await, forbidden);
        assert_eq!(get_json(&url, &[("DD-APPLICATION-KEY", "ocak_not-a-key")]).await, forbidden);
        assert_eq!(get_json(&url, &[("DD-APPLICATION-KEY", &s.api_key)]).await, forbidden);
        assert_eq!(get_json(&url, &[("DD-API-KEY", "ocik_not-a-key"), ("DD-APPLICATION-KEY", &s.app_key)]).await, forbidden);
        // Keys of two organizations don't make a pair.
        let other = new_org_with_roles(&s.st, "dd-other").await;
        let other_api_key = ingest_key(&s.st, &other).await;
        assert_eq!(get_json(&url, &[("DD-API-KEY", &other_api_key), ("DD-APPLICATION-KEY", &s.app_key)]).await, forbidden);

        sqlx::query("UPDATE app_keys SET revoked_at = now() WHERE org_id = $1").bind(&s.org).execute(&s.st.postgres).await.unwrap();
        assert_eq!(get_json(&url, &[("DD-APPLICATION-KEY", &s.app_key)]).await, forbidden);
    }

    #[tokio::test]
    async fn datadog_headers_dont_authenticate_our_own_routes() {
        let Some(s) = TestServer::start("dd-own-routes").await else { return };
        let url = format!("{}/api/v1/roles", s.base);
        assert_eq!(get_json(&url, &[("DD-APPLICATION-KEY", &s.app_key)]).await.0, StatusCode::UNAUTHORIZED);
        assert_eq!(get_json(&url, &[("DD-API-KEY", &s.api_key), ("DD-APPLICATION-KEY", &s.app_key)]).await.0, StatusCode::UNAUTHORIZED);
        // Our own application-key authentication is unchanged.
        assert_eq!(get_json(&url, &[("Authorization", &format!("Bearer {}", s.app_key))]).await.0, StatusCode::OK);
    }

    #[tokio::test]
    async fn dd_auth_acts_as_the_keys_user_with_its_role() {
        let Some(s) = TestServer::start("dd-auth").await else { return };
        let (reader, reader_key) = user_with_app_key(&s.st, &s.org, "readonly").await;
        let app = axum::Router::new()
            .route("/whoami", get(|DdAuth(ctx): DdAuth| async move { axum::Json(json!({ "user": ctx.user_id, "org": ctx.org_id, "access": ctx.access })) }))
            .with_state(s.st.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/whoami", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

        let me = json!({ "user": s.user_id, "org": s.org, "access": "admin" });
        assert_eq!(get_json(&url, &[("DD-APPLICATION-KEY", &s.app_key)]).await, (StatusCode::OK, me.clone()));
        assert_eq!(get_json(&url, &[("Authorization", &format!("Bearer {}", s.app_key))]).await, (StatusCode::OK, me.clone()));
        assert_eq!(get_json(&url, &[("DD-API-KEY", &s.api_key), ("DD-APPLICATION-KEY", &s.app_key)]).await, (StatusCode::OK, me));
        assert_eq!(get_json(&url, &[("DD-API-KEY", &s.api_key)]).await, (StatusCode::FORBIDDEN, json!({ "errors": ["Forbidden"] })));
        assert_eq!(get_json(&url, &[]).await, (StatusCode::FORBIDDEN, json!({ "errors": ["Forbidden"] })));
        assert_eq!(get_json(&url, &[("DD-APPLICATION-KEY", &reader_key)]).await.1, json!({ "user": reader, "org": s.org, "access": "read_only" }));
    }

    /// Datadog's recorded Authentication scenarios.
    #[tokio::test]
    async fn authentication_cassettes_replay() {
        let Some(s) = TestServer::start("dd-auth-cassettes").await else { return };
        let feature = Feature::load("v1/authentication");
        assert_eq!(feature.scenarios.len(), 2);
        for scenario in &feature.scenarios {
            let (exchanges, problems) = s.replayer().replay(&feature, scenario).await;
            assert!(problems.is_empty(), "{problems:#?}");
            let [one] = exchanges.as_slice() else { panic!("{exchanges:?}") };
            assert_eq!((one.path.as_str(), one.status), ("/api/v1/validate", one.recorded));
            assert!(one.body.get("valid").is_some() || one.body.get("errors").is_some(), "{}", one.body);
        }
    }
}
