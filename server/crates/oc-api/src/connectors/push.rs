//! Push packages for the intake (`oc-intake/src/connector_push.rs`), on the internal listener: every loaded package's
//! push definition, and an org's signing secrets of a package (the `push.signature.secret` field of its enabled
//! connections, unsealed). The org is the intake's `x-oc-org` (resolved from the request's ingest key); only that
//! one field is answered, never other secrets.

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use oc_connectors::push::PushDef;
use serde_json::{json, Value};

use super::{registry, store, Registry};
use crate::state::AppState;

/// Every package that is pushed to.
pub fn definitions(r: &Registry) -> Vec<PushDef> {
    r.packages.iter().filter_map(|p| PushDef::of(&p.package.manifest)).collect()
}

/// `GET /ingest/connectors/push`.
pub async fn list(State(st): State<AppState>, headers: HeaderMap) -> Response {
    if !crate::logproc::internal_authorized(st.config.internal_token.as_deref(), &headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    Json(json!({ "packages": definitions(registry()) })).into_response()
}

/// The org's signing secrets of a package (enabled connections; a secret that can't be unsealed is left out).
pub async fn signing_secrets(st: &AppState, r: &Registry, org: &str, id: &str) -> Result<Vec<String>, sqlx::Error> {
    let Some(field) = r.package(id).and_then(|p| p.package.manifest.push.as_ref()?.signature.as_ref().map(|s| s.secret.clone())) else { return Ok(vec![]) };
    let mut out = Vec::new();
    for c in store::list(st, org, id).await?.into_iter().filter(|c| c.enabled) {
        if let Some(sealed) = c.secrets.get(&field).and_then(Value::as_str) {
            match st.secrets.decrypt(sealed) {
                Ok(plain) => out.push(plain),
                Err(_) => tracing::warn!(connector = id, connection = %c.id, "a signing secret can't be unsealed (the server's secret key changed)"),
            }
        }
    }
    Ok(out)
}

/// `GET /ingest/connectors/push/{id}/secrets` (`x-oc-org`).
pub async fn secrets(State(st): State<AppState>, Path(id): Path<String>, headers: HeaderMap) -> Response {
    let org = match crate::routes::code_security::internal_org(&st, &headers) {
        Ok(o) => o,
        Err(s) => return s.into_response(),
    };
    match signing_secrets(&st, registry(), &org, &id).await {
        Ok(secrets) => Json(json!({ "secrets": secrets })).into_response(),
        Err(e) => {
            tracing::warn!(error = %e, "reading signing secrets failed");
            StatusCode::SERVICE_UNAVAILABLE.into_response()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PACKAGE: &str = "schema_version: 1\nid: example-hooks\nname: Example Hooks\npublisher: Example Corp\nvendor: Example Corp\ndescription: Webhooks.\n\
        config:\n  - { key: signing_secret, label: Signing secret, type: string, secret: true }\n  - { key: other, label: Other, type: string, secret: true }\n\
        push: { kind: webhook, instructions: Add a webhook, signature: { header: X-Signature, algorithm: hmac_sha256, secret: signing_secret } }\n\
        webhook: { logs: { message: '{{ record.type }}' } }\n";

    fn registry_with_package() -> Registry {
        // One directory per call: the tests run in parallel.
        static CALLS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!("oc-push-{}-{}", std::process::id(), CALLS.fetch_add(1, std::sync::atomic::Ordering::Relaxed)));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("example-hooks")).unwrap();
        std::fs::write(dir.join("example-hooks/connector.yaml"), PACKAGE).unwrap();
        let r = super::super::load(Some(dir.to_str().unwrap()));
        let _ = std::fs::remove_dir_all(&dir);
        r
    }

    #[test]
    fn push_packages_are_listed_for_the_intake() {
        let r = registry_with_package();
        assert!(r.invalid.is_empty(), "{:?}", r.invalid);
        let defs = definitions(&r);
        let d = defs.iter().find(|d| d.id == "example-hooks").expect("listed");
        assert_eq!((d.name.as_str(), d.signature().map(|s| s.header.as_str())), ("example-hooks", Some("X-Signature")));
        assert!(!defs.iter().any(|d| d.id == "netbird"), "polled packages aren't");
    }

    /// Only the signing secret of the org's enabled connections, unsealed; never another org's.
    #[tokio::test]
    async fn signing_secrets_are_the_orgs_own() {
        let Some((_, states)) = crate::test_support::replicas(1).await else { return };
        let st = &states[0];
        let r = registry_with_package();
        let a = crate::test_support::admin_of_new_org(st, "push-a").await;
        let b = crate::test_support::admin_of_new_org(st, "push-b").await;
        for (name, enabled, secret) in [("one", true, "whsec-1"), ("two", true, "whsec-2"), ("off", false, "whsec-3")] {
            let sealed = json!({ "signing_secret": st.secrets.encrypt(secret), "other": st.secrets.encrypt("not-this") });
            sqlx::query("INSERT INTO connector_connections (org_id, connector_id, name, config, secrets, streams, enabled) VALUES ($1, 'example-hooks', $2, '{}', $3, '{}', $4)")
                .bind(&a.org_id)
                .bind(name)
                .bind(sealed)
                .bind(enabled)
                .execute(&st.postgres)
                .await
                .unwrap();
        }
        let mut got = signing_secrets(st, &r, &a.org_id, "example-hooks").await.unwrap();
        got.sort();
        assert_eq!(got, ["whsec-1", "whsec-2"]);
        assert!(signing_secrets(st, &r, &b.org_id, "example-hooks").await.unwrap().is_empty());
        assert!(signing_secrets(st, &r, &a.org_id, "netbird").await.unwrap().is_empty(), "no signature: nothing");
    }
}
