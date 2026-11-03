//! Assist AI in Organization Settings: the organization's own model provider, used instead of the
//! install's (`OC_ASSIST_*`). Admins only; the API key is sealed and never shown again.

use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::assist::agent::Model;
use crate::assist::http::{HttpModel, Reach};
use crate::assist::llm::{Msg, Provider};
use crate::assist::{org_provider, private_target, ProviderSettings};
use crate::auth::{Admin, AuthContext};
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;

/// The model a conversation uses, and where its configuration comes from.
pub struct Resolved {
    pub provider: Provider,
    pub reach: Reach,
    /// `org` or `install`.
    pub source: &'static str,
}

#[derive(sqlx::FromRow)]
struct Row {
    provider: String,
    model: Option<String>,
    base_url: Option<String>,
    fallbacks: Option<bool>,
    api_key_enc: Option<String>,
    updated_by: String,
    updated_at: i64,
}

impl Row {
    fn settings(&self) -> ProviderSettings {
        ProviderSettings { provider: self.provider.clone(), model: self.model.clone(), base_url: self.base_url.clone(), fallbacks: self.fallbacks }
    }
}

async fn load(st: &AppState, org: &str) -> Result<Option<Row>, ApiError> {
    Ok(sqlx::query_as::<_, Row>("SELECT provider, model, base_url, fallbacks, api_key_enc, updated_by, (extract(epoch FROM updated_at) * 1000)::bigint AS updated_at FROM assist_org_providers WHERE org_id = $1")
        .bind(org)
        .fetch_optional(&st.postgres)
        .await?)
}

fn unseal(st: &AppState, row: &Row) -> Result<Option<String>, ApiError> {
    row.api_key_enc.as_deref().map(|k| st.secrets.decrypt(k)).transpose().map_err(|e| ApiError::Internal(e.context("unsealing the organization's Assist API key")))
}

fn org_reach(st: &AppState) -> Reach {
    Reach::Public { allow_private: st.config.assist_allow_private }
}

/// The organization's own provider, else the install's; `None` when neither is set up.
pub async fn resolve(st: &AppState, org: &str) -> Result<Option<Resolved>, ApiError> {
    if let Some(row) = load(st, org).await? {
        let provider = org_provider(&row.settings(), unseal(st, &row)?, st.config.assist_allow_private)
            .map_err(|e| ApiError::BadRequest(format!("This organization's Assist provider can't be used: {e}. An admin can fix it in Organization Settings → Assist AI.")))?;
        return Ok(Some(Resolved { provider, reach: org_reach(st), source: "org" }));
    }
    Ok(install(st).map(|p| Resolved { provider: p.clone(), reach: Reach::Trusted, source: "install" }))
}

fn install(st: &AppState) -> Option<&Provider> {
    st.config.assist.as_ref().ok().and_then(Option::as_ref)
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AssistOrgProvider {
    #[serde(flatten)]
    pub settings: ProviderSettings,
    /// An API key is stored (it is never returned).
    pub has_key: bool,
    pub updated_by: String,
    /// Epoch ms.
    pub updated_at: i64,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AssistInstallProvider {
    pub provider: String,
    pub model: String,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AssistProviderView {
    /// What Assist uses for this organization: `org`, `install` or `none`.
    pub source: String,
    pub org: Option<AssistOrgProvider>,
    /// The install's default (`OC_ASSIST_*`), used when the organization sets none.
    pub install: Option<AssistInstallProvider>,
    /// Organizations may use private addresses (`OC_ASSIST_ALLOW_PRIVATE`).
    pub allow_private: bool,
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AssistProviderUpdate {
    #[serde(flatten)]
    pub settings: ProviderSettings,
    /// A new key; omitted keeps the stored one, empty removes it.
    pub api_key: Option<String>,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AssistProviderTest {
    pub ok: bool,
    pub message: String,
}

#[utoipa::path(get, path = "/api/v1/assist/provider", operation_id = "assist_get_provider", responses((status = 200, body = AssistProviderView)), tag = "assist")]
pub async fn get_provider(State(st): State<AppState>, Admin(ctx): Admin) -> ApiResult<AssistProviderView> {
    let row = load(&st, &ctx.org_id).await?;
    let install = install(&st).map(|p| AssistInstallProvider { provider: p.kind().into(), model: p.model().into() });
    let source = if row.is_some() { "org" } else if install.is_some() { "install" } else { "none" };
    Ok(Json(AssistProviderView {
        source: source.into(),
        org: row.map(|r| AssistOrgProvider { settings: r.settings(), has_key: r.api_key_enc.is_some(), updated_by: r.updated_by, updated_at: r.updated_at }),
        install,
        allow_private: st.config.assist_allow_private,
    }))
}

/// The provider an update describes, with the stored key unless it brings one; checked, its host resolved.
async fn provider_of(st: &AppState, ctx: &AuthContext, u: &AssistProviderUpdate) -> Result<Provider, ApiError> {
    let key = match &u.api_key {
        Some(k) => Some(k.clone()),
        None => match load(st, &ctx.org_id).await? {
            Some(row) => unseal(st, &row)?,
            None => None,
        },
    };
    let provider = org_provider(&u.settings, key, st.config.assist_allow_private).map_err(ApiError::BadRequest)?;
    let url = url::Url::parse(provider.base_url()).map_err(|e| ApiError::BadRequest(e.to_string()))?;
    let host = url.host_str().unwrap_or_default();
    crate::routes::synthetics_net::resolve(host, url.port_or_known_default().unwrap_or(443), st.config.assist_allow_private)
        .await
        .map_err(|e| ApiError::BadRequest(private_target(e)))?;
    Ok(provider)
}

/// Sets the organization's provider. Saved without `apiKey`, the stored key stays.
#[utoipa::path(put, path = "/api/v1/assist/provider", operation_id = "assist_put_provider", request_body = AssistProviderUpdate, responses((status = 204)), tag = "assist")]
pub async fn put_provider(State(st): State<AppState>, Admin(ctx): Admin, Json(u): Json<AssistProviderUpdate>) -> Result<StatusCode, ApiError> {
    let p = provider_of(&st, &ctx, &u).await?;
    let (base_url, api_key, fallbacks) = match &p {
        Provider::Anthropic { base_url, api_key, fallbacks, .. } => (base_url, Some(api_key), Some(*fallbacks)),
        Provider::OpenAi { base_url, api_key, .. } => (base_url, api_key.as_ref(), None),
    };
    sqlx::query(
        "INSERT INTO assist_org_providers (org_id, provider, model, base_url, fallbacks, api_key_enc, updated_by, updated_at) VALUES ($1, $2, $3, $4, $5, $6, $7, now())
         ON CONFLICT (org_id) DO UPDATE SET provider = $2, model = $3, base_url = $4, fallbacks = $5, api_key_enc = $6, updated_by = $7, updated_at = now()",
    )
    .bind(&ctx.org_id)
    .bind(p.kind())
    .bind(p.model())
    .bind(base_url)
    .bind(fallbacks)
    .bind(api_key.map(|k| st.secrets.encrypt(k)))
    .bind(&ctx.name)
    .execute(&st.postgres)
    .await?;
    let key_note = match &u.api_key {
        Some(k) if k.trim().is_empty() => ", key removed",
        Some(_) => ", new key",
        None => "",
    };
    ctx.audit(&st, "assist.provider.update", p.kind(), &format!("{} at {}{key_note}", p.model(), base_url)).await;
    Ok(StatusCode::NO_CONTENT)
}

/// Removes the organization's provider: Assist uses the install's again, if any.
#[utoipa::path(delete, path = "/api/v1/assist/provider", operation_id = "assist_delete_provider", responses((status = 204)), tag = "assist")]
pub async fn delete_provider(State(st): State<AppState>, Admin(ctx): Admin) -> Result<StatusCode, ApiError> {
    let gone = sqlx::query("DELETE FROM assist_org_providers WHERE org_id = $1").bind(&ctx.org_id).execute(&st.postgres).await?.rows_affected();
    if gone > 0 {
        ctx.audit(&st, "assist.provider.delete", "", "back to the install's provider").await;
    }
    Ok(StatusCode::NO_CONTENT)
}

/// Sends one short request to the provider described (with the stored key unless it brings one), so an
/// admin knows it answers before saving it.
#[utoipa::path(post, path = "/api/v1/assist/provider/test", operation_id = "assist_test_provider", request_body = AssistProviderUpdate, responses((status = 200, body = AssistProviderTest)), tag = "assist")]
pub async fn test_provider(State(st): State<AppState>, Admin(ctx): Admin, Json(u): Json<AssistProviderUpdate>) -> ApiResult<AssistProviderTest> {
    let model = HttpModel { provider: provider_of(&st, &ctx, &u).await?, max_tokens: 256, reach: org_reach(&st) };
    let reply = tokio::time::timeout(std::time::Duration::from_secs(60), model.turn("Reply with the single word OK.", &[], &[Msg::User("ping".into())], &mut |_| {})).await;
    Ok(Json(match reply {
        Ok(Ok(_)) => AssistProviderTest { ok: true, message: format!("{} answered", model.provider.model()) },
        Ok(Err(e)) => AssistProviderTest { ok: false, message: e },
        Err(_) => AssistProviderTest { ok: false, message: "No answer within 60 s".into() },
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assist::testing::{events, fake_anthropic, sse_text};
    use crate::routes::assist::{chat, ChatRequest};

    fn anthropic(base_url: &str, model: &str, api_key: Option<&str>) -> AssistProviderUpdate {
        AssistProviderUpdate {
            settings: ProviderSettings { provider: "anthropic".into(), model: Some(model.into()), base_url: Some(base_url.into()), fallbacks: Some(false) },
            api_key: api_key.map(Into::into),
        }
    }

    /// An org's provider replaces the install's for that org only, its key is sealed and never returned,
    /// a save without a key keeps it, changes are audited, and removing it falls back to the install's.
    /// Skipped unless OC_TEST_POSTGRES_URL and OC_TEST_CLICKHOUSE_URL are set.
    #[tokio::test]
    async fn an_org_provider_replaces_the_install_one_for_that_org() {
        let (base_url, seen) = fake_anthropic(vec![sse_text("OK"), sse_text("Hello from the org's model")]).await;
        let install = Provider::Anthropic { base_url: "https://install.invalid".into(), api_key: "sk-install".into(), model: "claude-opus-5".into(), fallbacks: true };
        let Some((_, states)) = crate::test_support::replicas_with(1, |cfg| {
            cfg.assist = Ok(Some(install.clone()));
            cfg.assist_allow_private = true;
        })
        .await
        else {
            return;
        };
        let st = states[0].clone();
        let admin = crate::test_support::admin_of_new_org(&st, "assistp").await;
        let (user_id,): (String,) = sqlx::query_as("INSERT INTO users (id, org_id, email, name) VALUES (gen_random_uuid(), $1, $2, 'Test Admin') RETURNING id::text")
            .bind(&admin.org_id)
            .bind(format!("{}@example.com", admin.org_id))
            .fetch_one(&st.postgres)
            .await
            .unwrap();
        let ctx = AuthContext { user_id, ..admin };
        let other = crate::test_support::admin_of_new_org(&st, "assistq").await;

        // The install's model until the org sets one.
        let v = get_provider(State(st.clone()), Admin(ctx.clone())).await.unwrap().0;
        assert_eq!((v.source.as_str(), v.org.is_none()), ("install", true));
        assert_eq!(v.install.as_ref().map(|i| i.model.as_str()), Some("claude-opus-5"));

        // Set: used for this org, sealed, never shown.
        put_provider(State(st.clone()), Admin(ctx.clone()), Json(anthropic(&base_url, "claude-sonnet-5", Some("sk-org-secret")))).await.unwrap();
        let v = get_provider(State(st.clone()), Admin(ctx.clone())).await.unwrap().0;
        assert_eq!(v.source, "org");
        let org = v.org.as_ref().unwrap();
        assert!(org.has_key && org.settings.model.as_deref() == Some("claude-sonnet-5"));
        assert!(!serde_json::to_string(&v).unwrap().contains("sk-org-secret"));
        let sealed: String = sqlx::query_scalar("SELECT api_key_enc FROM assist_org_providers WHERE org_id = $1").bind(&ctx.org_id).fetch_one(&st.postgres).await.unwrap();
        assert!(!sealed.contains("sk-org-secret"));

        // Saved again without a key: the key stays.
        put_provider(State(st.clone()), Admin(ctx.clone()), Json(anthropic(&base_url, "claude-opus-5", None))).await.unwrap();
        assert!(get_provider(State(st.clone()), Admin(ctx.clone())).await.unwrap().0.org.unwrap().has_key);

        // The connection test and the chat both reach the org's model with the org's key.
        let t = test_provider(State(st.clone()), Admin(ctx.clone()), Json(anthropic(&base_url, "claude-opus-5", None))).await.unwrap().0;
        assert!(t.ok, "{}", t.message);
        let ask = ChatRequest { conversation_id: None, message: "hello".into(), surface: None, auto_approve: false };
        let ev = events(chat(State(st.clone()), ctx.clone(), Json(ask)).await.unwrap()).await;
        assert!(ev.iter().any(|(name, data)| name == "text" && data["text"] == "Hello from the org's model"), "{ev:?}");
        let requests = seen.lock().unwrap().clone();
        assert_eq!(requests.len(), 2);
        assert!(requests.iter().all(|r| r["_api_key"] == "sk-org-secret" && r["model"] == "claude-opus-5"), "{requests:?}");

        // Other orgs keep the install's model.
        assert_eq!(get_provider(State(st.clone()), Admin(other)).await.unwrap().0.source, "install");

        // Audited, and removed: back to the install's.
        let audited: i64 = sqlx::query_scalar("SELECT count(*) FROM audit_events WHERE org_id = $1 AND action LIKE 'assist.provider.%'").bind(&ctx.org_id).fetch_one(&st.postgres).await.unwrap();
        assert_eq!(audited, 2);
        delete_provider(State(st.clone()), Admin(ctx.clone())).await.unwrap();
        assert_eq!(get_provider(State(st.clone()), Admin(ctx.clone())).await.unwrap().0.source, "install");
    }

    /// A provider that can't be used is refused before it is saved.
    #[tokio::test]
    async fn an_invalid_provider_is_not_saved() {
        let Some((_, states)) = crate::test_support::replicas_with(1, |_| {}).await else { return };
        let st = states[0].clone();
        let ctx = crate::test_support::admin_of_new_org(&st, "assistr").await;
        let bad = anthropic("https://169.254.169.254/latest", "claude-opus-5", Some("sk"));
        assert!(matches!(put_provider(State(st.clone()), Admin(ctx.clone()), Json(bad)).await, Err(ApiError::BadRequest(_))));
        let no_key = anthropic("https://api.anthropic.com", "claude-opus-5", None);
        assert!(matches!(put_provider(State(st.clone()), Admin(ctx.clone()), Json(no_key)).await, Err(ApiError::BadRequest(_))));
        assert_eq!(get_provider(State(st.clone()), Admin(ctx)).await.unwrap().0.source, "none");
    }
}
