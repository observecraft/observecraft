//! Cloud Security's account scanning (docs/adr/0006-cloud-posture-scanning.md).
//!
//! Prowler, pinned by digest, runs in the separate `scanner` deployment (its loop is
//! `deploy/helm/observecraft/files/scanner/scan.py`, used by the chart and by compose). This module is the server's
//! side:
//!
//! - **Queue** ([`run`]): every minute the due scans are queued ([`store::plan`]): one per AWS account whose
//!   `resources_config.cloud_security_posture_management_collection` is on, every `OC_POSTURE_SCAN_INTERVAL_HOURS`.
//!   Nothing is queued without `OC_SCANNER_TOKEN`, since no scanner could take it.
//! - **Scanner API** ([`internal_router`], on the internal listener next to `/ingest/*`, bearer `OC_SCANNER_TOKEN`):
//!   `POST /internal/posture/claim` hands out the oldest queued scan with short-lived credentials
//!   ([`crate::aws::api::Sdk::scan_credentials`]: the customer's role assumed with its external id, narrowed to
//!   read-only session policies), `…/scans/{id}/heartbeat` renews its lease while Prowler runs,
//!   `…/scans/{id}/results` takes Prowler's `json-ocsf` file (gzip accepted) and `…/scans/{id}/fail` an error.
//! - **Findings** ([`store`]) are served by Datadog's security findings API (`routes/datadog_api/security_findings.rs`,
//!   documents in [`datadog`]) and Security → Cloud Security → Findings (`routes/cloud_security.rs`).

pub mod datadog;
pub mod ocsf;
pub mod store;

use std::io::Read;
use std::sync::Arc;
use std::time::Duration;

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::sync::OnceCell;

use crate::aws::api::{Sdk, TempCredentials};
use crate::routes::datadog_api::aws_integration as accounts;
use crate::state::AppState;

/// How long a scan's credentials last; the scanner renews nothing, so a scan must finish within it.
const CREDENTIAL_SECS: i32 = 3600;
/// Largest uncompressed results file.
const MAX_RESULTS: u64 = 512 * 1024 * 1024;

static SDK: OnceCell<Arc<Sdk>> = OnceCell::const_new();

async fn sdk() -> Arc<Sdk> {
    SDK.get_or_init(Sdk::from_env).await.clone()
}

/// Queues due scans and fails abandoned ones, every minute.
pub async fn run(st: AppState) {
    let mut tick = tokio::time::interval(Duration::from_secs(60));
    loop {
        tick.tick().await;
        if st.config.scanner_token.is_none() {
            continue;
        }
        if let Err(e) = store::expire(&st.postgres).await {
            tracing::warn!(error = %e, "expiring posture scans failed");
        }
        match store::plan(&st.postgres, st.config.posture_scan_interval_hours).await {
            Ok(n) if n > 0 => tracing::info!(queued = n, "posture scans queued"),
            Ok(_) => {}
            Err(e) => tracing::warn!(error = %e, "planning posture scans failed"),
        }
    }
}

/// The scanner's routes (merged into the internal listener).
pub fn internal_router(st: AppState) -> Router {
    Router::new()
        .route("/internal/posture/claim", post(claim))
        .route("/internal/posture/scans/{id}/heartbeat", post(heartbeat))
        .route("/internal/posture/scans/{id}/results", post(results))
        .route("/internal/posture/scans/{id}/fail", post(fail))
        .with_state(st)
}

/// Constant-time comparison of the bearer token with `OC_SCANNER_TOKEN` (no token: nothing is authorized).
fn authorized(st: &AppState, headers: &HeaderMap) -> bool {
    token_matches(st.config.scanner_token.as_deref(), headers)
}

fn token_matches(token: Option<&str>, headers: &HeaderMap) -> bool {
    let Some(token) = token else { return false };
    let Some(sent) = headers.get("authorization").and_then(|v| v.to_str().ok()).and_then(|v| v.strip_prefix("Bearer ")) else { return false };
    sent.len() == token.len() && sent.bytes().zip(token.bytes()).fold(0u8, |acc, (a, b)| acc | (a ^ b)) == 0
}

/// A scan for the scanner to run.
#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Assignment {
    pub scan_id: String,
    pub provider: String,
    pub account_id: String,
    pub partition: String,
    /// Regions to check (empty: every enabled region).
    pub regions: Vec<String>,
    pub credentials: TempCredentials,
}

async fn claim(State(st): State<AppState>, headers: HeaderMap) -> Response {
    if !authorized(&st, &headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let scanner = headers.get("x-oc-scanner").and_then(|v| v.to_str().ok()).unwrap_or("scanner").chars().take(120).collect::<String>();
    // A scan whose account can't be reached fails and the next one is tried.
    for _ in 0..5 {
        let scan = match store::claim(&st.postgres, &scanner).await {
            Ok(Some(s)) => s,
            Ok(None) => return StatusCode::NO_CONTENT.into_response(),
            Err(e) => {
                tracing::warn!(error = %e, "claiming a posture scan failed");
                return StatusCode::SERVICE_UNAVAILABLE.into_response();
            }
        };
        match assignment(&st, &scan).await {
            Ok(a) => {
                tracing::info!(scan = %scan.id, org = %scan.org_id, account = %scan.account_id, scanner = %scanner, "posture scan claimed");
                return Json(a).into_response();
            }
            Err(message) => {
                tracing::info!(scan = %scan.id, account = %scan.account_id, error = %message, "posture scan can't start");
                let _ = store::fail(&st.postgres, &scan.id, &message).await;
            }
        }
    }
    StatusCode::NO_CONTENT.into_response()
}

async fn assignment(st: &AppState, scan: &store::Claimed) -> Result<Assignment, String> {
    let account = accounts::get_row(st, &scan.org_id, &scan.account_ref).await.map_err(|e| e.to_string())?.ok_or("The AWS account was removed")?;
    let auth = crate::aws::crawler::target_auth(st, &scan.org_id, &account).await.map_err(|e| e.to_string())?;
    let credentials = sdk()
        .await
        .scan_credentials(&account.aws_partition, &account.aws_account_id, &auth, CREDENTIAL_SECS)
        .await
        .map_err(|e| format!("Couldn't get read-only credentials for the scan: {}", e.message()))?;
    Ok(Assignment {
        scan_id: scan.id.clone(),
        provider: scan.provider.clone(),
        account_id: account.aws_account_id.clone(),
        regions: crate::aws::crawler::regions(&account.aws_partition, &account.aws_regions),
        partition: account.aws_partition,
        credentials,
    })
}

async fn heartbeat(State(st): State<AppState>, headers: HeaderMap, Path(id): Path<String>) -> StatusCode {
    if !authorized(&st, &headers) {
        return StatusCode::UNAUTHORIZED;
    }
    match store::heartbeat(&st.postgres, &id).await {
        Ok(true) => StatusCode::NO_CONTENT,
        Ok(false) => StatusCode::CONFLICT,
        Err(_) => StatusCode::SERVICE_UNAVAILABLE,
    }
}

#[derive(Deserialize)]
struct Failure {
    error: String,
}

async fn fail(State(st): State<AppState>, headers: HeaderMap, Path(id): Path<String>, Json(f): Json<Failure>) -> StatusCode {
    if !authorized(&st, &headers) {
        return StatusCode::UNAUTHORIZED;
    }
    match store::fail(&st.postgres, &id, &f.error).await {
        Ok(()) => StatusCode::NO_CONTENT,
        Err(_) => StatusCode::SERVICE_UNAVAILABLE,
    }
}

fn gunzip(headers: &HeaderMap, body: &[u8]) -> Result<Vec<u8>, String> {
    let gz = headers.get("content-encoding").and_then(|v| v.to_str().ok()).is_some_and(|v| v.contains("gzip")) || body.starts_with(&[0x1f, 0x8b]);
    if !gz {
        return Ok(body.to_vec());
    }
    let mut out = Vec::new();
    flate2::read::GzDecoder::new(body).take(MAX_RESULTS + 1).read_to_end(&mut out).map_err(|e| format!("gzip: {e}"))?;
    if out.len() as u64 > MAX_RESULTS {
        return Err("results larger than 512 MiB".into());
    }
    Ok(out)
}

async fn results(State(st): State<AppState>, headers: HeaderMap, Path(id): Path<String>, body: Bytes) -> Response {
    if !authorized(&st, &headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let scan = match store::running(&st.postgres, &id).await {
        Ok(Some(s)) => s,
        Ok(None) => return (StatusCode::CONFLICT, Json(json!({ "error": "the scan isn't running" }))).into_response(),
        Err(_) => return StatusCode::SERVICE_UNAVAILABLE.into_response(),
    };
    let parsed = gunzip(&headers, &body).and_then(|b| ocsf::parse(&b));
    let parsed = match parsed {
        Ok(p) => p,
        Err(e) => {
            let _ = store::fail(&st.postgres, &id, &format!("Unreadable scanner results: {e}")).await;
            return (StatusCode::BAD_REQUEST, Json(json!({ "error": e }))).into_response();
        }
    };
    match store::store_results(&st.postgres, &scan, &parsed).await {
        Ok((passed, failed)) => {
            tracing::info!(scan = %id, org = %scan.org_id, account = %scan.account_id, passed, failed, skipped = parsed.skipped, version = %parsed.tool_version, "posture scan stored");
            Json(json!({ "passed": passed, "failed": failed })).into_response()
        }
        Err(e) => {
            tracing::warn!(scan = %id, error = %e, "storing posture results failed");
            StatusCode::SERVICE_UNAVAILABLE.into_response()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_scan_is_queued_claimed_and_stored() {
        let Some((_, states)) = crate::test_support::replicas_with(1, |c| c.scanner_token = Some("scanner-token-0123456789".into())).await else { return };
        let st = &states[0];
        let org = format!("posture-{}", std::process::id());
        sqlx::query("INSERT INTO orgs (id, name) VALUES ($1, $1) ON CONFLICT DO NOTHING").bind(&org).execute(&st.postgres).await.unwrap();
        sqlx::query("DELETE FROM posture_scans WHERE org_id = $1").bind(&org).execute(&st.postgres).await.unwrap();
        sqlx::query("DELETE FROM aws_accounts WHERE org_id = $1").bind(&org).execute(&st.postgres).await.unwrap();
        let account: String = sqlx::query_scalar(
            "INSERT INTO aws_accounts (org_id, aws_account_id, role_name, external_id, resources_config) \
             VALUES ($1, '123456789012', 'ObserveCraftIntegrationRole', 'ext-0123', '{\"cloud_security_posture_management_collection\": true}') RETURNING id::text",
        )
        .bind(&org)
        .fetch_one(&st.postgres)
        .await
        .unwrap();

        store::plan(&st.postgres, 24).await.unwrap();
        store::plan(&st.postgres, 24).await.unwrap();
        let queued: i64 = sqlx::query_scalar("SELECT count(*) FROM posture_scans WHERE org_id = $1").bind(&org).fetch_one(&st.postgres).await.unwrap();
        assert_eq!(queued, 1, "one scan per account at a time");

        // Other tests' scans may be queued too: claim until ours comes.
        let scan = loop {
            let s = store::claim(&st.postgres, "test").await.unwrap().expect("a queued scan");
            if s.org_id == org {
                break s;
            }
            store::fail(&st.postgres, &s.id, "not this test's").await.unwrap();
        };
        assert_eq!((scan.account_ref.as_str(), scan.account_id.as_str()), (account.as_str(), "123456789012"));
        assert!(store::heartbeat(&st.postgres, &scan.id).await.unwrap());

        let mut results = ocsf::parse(ocsf::tests::sample().to_string().as_bytes()).unwrap();
        // An evaluation of another account is dropped.
        let mut foreign = results.evaluations[0].clone();
        foreign.account_id = "210987654321".into();
        foreign.resource_uid = "arn:aws:s3:::someone-elses".into();
        results.evaluations.push(foreign);
        assert_eq!(store::store_results(&st.postgres, &scan, &results).await.unwrap(), (1, 1));
        let rows = store::findings(&st.postgres, &org, false).await.unwrap();
        assert_eq!(rows.len(), 2);
        assert!(rows.iter().all(|r| r.account_id == "123456789012"));
        let failing = store::findings(&st.postgres, &org, true).await.unwrap();
        assert_eq!(failing.len(), 1);
        assert_eq!(failing[0].title.as_deref(), Some("Ensure S3 buckets are not publicly accessible"));
        assert!(!store::heartbeat(&st.postgres, &scan.id).await.unwrap(), "a stored scan is done");

        // Mute, then a later scan without the passing bucket: its row goes, the muted one stays muted.
        assert_eq!(store::set_mute(&st.postgres, &org, std::slice::from_ref(&failing[0].id), Some(("RISK_ACCEPTED", "Public assets", None)), "user-1").await.unwrap(), 1);
        assert_eq!(store::set_mute(&st.postgres, "another-org", std::slice::from_ref(&failing[0].id), None, "user-2").await.unwrap(), 0, "another org can't unmute it");
        sqlx::query("INSERT INTO posture_scans (org_id, provider, account_ref, account_id, status, started_at) VALUES ($1, 'aws', $2::uuid, '123456789012', 'running', now())").bind(&org).bind(&account).execute(&st.postgres).await.unwrap();
        let second = store::latest_scans(&st.postgres, &org).await.unwrap().remove(0);
        let second = store::Claimed { id: second.id, org_id: org.clone(), provider: "aws".into(), account_ref: account.clone(), account_id: "123456789012".into() };
        results.evaluations.retain(|e| !e.pass);
        store::store_results(&st.postgres, &second, &results).await.unwrap();
        let rows = store::findings(&st.postgres, &org, false).await.unwrap();
        assert_eq!(rows.len(), 1);
        assert!(rows[0].muted);
        assert_eq!(rows[0].mute_reason.as_deref(), Some("RISK_ACCEPTED"));
    }

    #[test]
    fn only_the_scanner_token_is_authorized() {
        let mut headers = HeaderMap::new();
        headers.insert("authorization", "Bearer scanner-token-0123456789".parse().unwrap());
        assert!(token_matches(Some("scanner-token-0123456789"), &headers));
        assert!(!token_matches(Some("scanner-token-0123456780"), &headers));
        assert!(!token_matches(None, &headers), "without OC_SCANNER_TOKEN nothing is handed out");
        assert!(!token_matches(Some("scanner-token-0123456789"), &HeaderMap::new()));
    }
}
