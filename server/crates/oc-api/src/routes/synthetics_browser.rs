//! Browser test runs (Synthetic Monitoring): sent to oc-browser, recorded like API test runs, with each step's
//! screenshot and the run's video stored apart in ClickHouse (`otel.synthetics_artifacts`, 30 days) and served to
//! the result page. Passing scheduled runs keep no screenshots or video, as API tests keep no body then.

use std::collections::HashMap;

use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::header;
use axum::response::Response;
use base64::Engine;

use super::synthetics_exec::{SyntheticRequest, SyntheticRunDetail};
use crate::auth::AuthContext;
use crate::browser::client::{BrowserClient, BrowserRunResult, RunRequest, VideoMode};
use crate::browser::steps::Step;
use crate::error::ApiError;
use crate::state::AppState;

/// A screenshot or video of a run, base64.
#[derive(Debug, Clone, PartialEq)]
pub struct Artifact {
    /// `step-<n>.jpg` (n from 0, the start URL) or `video.webm`.
    pub name: String,
    pub content_type: &'static str,
    pub data: String,
}

/// The stored run and its artifacts: the images leave the result, which only says they exist.
pub fn split(mut result: BrowserRunResult) -> (SyntheticRunDetail, Vec<Artifact>) {
    let mut artifacts = Vec::new();
    for (i, step) in result.steps.iter_mut().enumerate() {
        if let Some(data) = step.screenshot.take().filter(|d| !d.is_empty()) {
            step.has_screenshot = true;
            artifacts.push(Artifact { name: format!("step-{i}.jpg"), content_type: "image/jpeg", data });
        }
    }
    if let Some(data) = result.video.take().filter(|d| !d.is_empty()) {
        result.has_video = true;
        artifacts.push(Artifact { name: "video.webm".into(), content_type: "video/webm", data });
    }
    let detail = SyntheticRunDetail {
        passed: result.passed,
        error: if result.passed { None } else { result.error.clone().or_else(|| Some("The run failed".into())) },
        duration_ms: result.duration_ms,
        url: result.steps.iter().rev().find(|s| s.status != "skipped" && !s.url.is_empty()).map(|s| s.url.clone()),
        browser: Some(result),
        ..Default::default()
    };
    (detail, artifacts)
}

/// The content type of a stored artifact's name, or `None` for names runs don't produce.
pub fn content_type(name: &str) -> Option<&'static str> {
    if name == "video.webm" {
        return Some("video/webm");
    }
    let n = name.strip_prefix("step-")?.strip_suffix(".jpg")?;
    (!n.is_empty() && n.len() <= 3 && n.bytes().all(|b| b.is_ascii_digit())).then_some("image/jpeg")
}

#[derive(clickhouse::Row, serde::Serialize)]
struct ArtifactRow<'a> {
    #[serde(rename = "OrgId")]
    org_id: &'a str,
    /// Epoch milliseconds.
    #[serde(rename = "Timestamp")]
    timestamp: i64,
    #[serde(rename = "TestId")]
    test_id: &'a str,
    #[serde(rename = "ResultId")]
    result_id: i64,
    #[serde(rename = "Name")]
    name: &'a str,
    #[serde(rename = "ContentType")]
    content_type: &'a str,
    #[serde(rename = "Data", with = "serde_bytes")]
    data: Vec<u8>,
}

#[derive(clickhouse::Row, serde::Deserialize)]
struct StoredArtifact {
    #[serde(rename = "Data", with = "serde_bytes")]
    data: Vec<u8>,
}

/// Stores a recorded run's artifacts (the test's org, never request input).
pub async fn store(st: &AppState, org: &str, test_id: &str, result_id: i64, artifacts: &[Artifact]) {
    if artifacts.is_empty() {
        return;
    }
    let timestamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0);
    let result: Result<(), clickhouse::error::Error> = async {
        // The client quotes the table name as one identifier, so the database is set apart.
        let mut insert = st.ch(org).with_database("otel").insert::<ArtifactRow>("synthetics_artifacts").await?;
        for a in artifacts {
            let Ok(data) = base64::engine::general_purpose::STANDARD.decode(&a.data) else { continue };
            insert.write(&ArtifactRow { org_id: org, timestamp, test_id, result_id, name: &a.name, content_type: a.content_type, data }).await?;
        }
        insert.end().await
    }
    .await;
    if let Err(e) = result {
        tracing::warn!(test = %test_id, result = result_id, error = %e, "storing browser test screenshots failed");
    }
}

/// One screenshot (`step-<n>.jpg`) or the video (`video.webm`) of a browser test result.
#[utoipa::path(get, path = "/api/v1/synthetics/tests/{id}/results/{result_id}/artifacts/{name}", operation_id = "get_synthetic_result_artifact",
    params(("id" = String, Path), ("result_id" = i64, Path), ("name" = String, Path)),
    responses((status = 200, description = "The image or video", content_type = "application/octet-stream"), (status = 404, body = crate::error::ErrorBody)), tag = "synthetics")]
pub async fn artifact(State(st): State<AppState>, ctx: AuthContext, Path((id, result_id, name)): Path<(String, i64, String)>) -> Result<Response, ApiError> {
    let not_found = || ApiError::NotFound("Screenshot or video not found".into());
    let kind = content_type(&name).ok_or_else(not_found)?;
    let stored: Option<StoredArtifact> = st
        .ch(&ctx.org_id)
        .query("SELECT Data FROM otel.synthetics_artifacts WHERE OrgId = {oc_org:String} AND TestId = ? AND ResultId = ? AND Name = ? LIMIT 1")
        .bind(&id)
        .bind(result_id)
        .bind(&name)
        .fetch_optional()
        .await?;
    let bytes = stored.ok_or_else(not_found)?.data;
    Response::builder()
        .header(header::CONTENT_TYPE, kind)
        .header(header::CACHE_CONTROL, "private, max-age=86400")
        .body(Body::from(bytes))
        .map_err(|e| ApiError::Internal(e.into()))
}

/// Runs a browser test in oc-browser; a run it can't start is a failed run naming why.
pub async fn run(st: &AppState, request: &SyntheticRequest, steps: &[Step], variables: &HashMap<String, String>, secrets: &[String], video: VideoMode) -> (SyntheticRunDetail, Vec<Artifact>) {
    let Some(settings) = st.config.browser.clone() else {
        return (SyntheticRunDetail { error: Some("Browser tests need oc-browser, which this server doesn't use (OC_BROWSER_URL)".into()), ..Default::default() }, Vec::new());
    };
    let req = RunRequest { start_url: &request.url, steps, variables, secrets, device_id: &request.device_id, video };
    match BrowserClient::new(settings).run(&req).await {
        Ok(result) => split(result),
        Err(e) => (SyntheticRunDetail { error: Some(e), ..Default::default() }, Vec::new()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn result(passed: bool) -> BrowserRunResult {
        serde_json::from_value(json!({
            "passed": passed, "durationMs": 2500, "startUrl": "https://shop.example.com", "browserVersion": "158.0",
            "error": if passed { serde_json::Value::Null } else { json!("Cannot locate element #pay") },
            "failure": if passed { serde_json::Value::Null } else { json!({"code": "LOCATE_ELEMENT", "message": "Cannot locate element #pay"}) },
            "device": {"id": "chrome.laptop_large", "width": 1440, "height": 1100},
            "steps": [
                {"name": "Navigate to https://shop.example.com", "type": "goToUrl", "status": "passed", "durationMs": 900, "url": "https://shop.example.com/", "screenshot": "/9j/one", "browserErrors": []},
                {"name": "Click on #pay", "type": "click", "status": if passed { "passed" } else { "failed" }, "durationMs": 1600, "url": "https://shop.example.com/cart", "browserErrors": []}
            ],
            "video": if passed { serde_json::Value::Null } else { json!("GkXf") }
        }))
        .unwrap()
    }

    #[test]
    fn screenshots_and_video_leave_the_stored_result() {
        let (detail, artifacts) = split(result(false));
        assert!(!detail.passed);
        assert_eq!(detail.error.as_deref(), Some("Cannot locate element #pay"));
        assert_eq!(detail.duration_ms, 2500.0);
        assert_eq!(detail.url.as_deref(), Some("https://shop.example.com/cart"), "where the run ended");
        let browser = detail.browser.unwrap();
        assert!(browser.steps[0].has_screenshot && !browser.steps[1].has_screenshot);
        assert!(browser.steps.iter().all(|s| s.screenshot.is_none()) && browser.video.is_none() && browser.has_video);
        assert_eq!(
            artifacts,
            vec![
                Artifact { name: "step-0.jpg".into(), content_type: "image/jpeg", data: "/9j/one".into() },
                Artifact { name: "video.webm".into(), content_type: "video/webm", data: "GkXf".into() },
            ]
        );
        let (ok, none) = split(result(true));
        assert!(ok.passed && ok.error.is_none());
        assert_eq!(none.len(), 1, "only the screenshot");
    }

    /// A browser test end to end against the live databases and a scripted oc-browser: created, run by hand,
    /// recorded with its screenshot and video, which only its org can read.
    #[tokio::test]
    async fn a_run_by_hand_keeps_its_screenshots_for_its_org_only() {
        use crate::auth::Writer;
        use crate::browser::client::{testing, BrowserSettings};
        use axum::Json;
        let (url, seen) = testing::fake(0, 200, testing::answer()).await;
        let settings = BrowserSettings { url, token: "0123456789abcdef".into() };
        let Some((_, states)) = crate::test_support::replicas_with(1, move |c| c.browser = Some(settings.clone())).await else { return };
        let st = &states[0];
        let mut ctx = crate::test_support::admin_of_new_org(st, "browser-run").await;
        let (user,): (String,) = sqlx::query_as("INSERT INTO users (id, org_id, email, name) VALUES (gen_random_uuid(), $1, 'qa@example.com', 'QA') RETURNING id::text")
            .bind(&ctx.org_id)
            .fetch_one(&st.postgres)
            .await
            .unwrap();
        ctx.user_id = user;
        let input: super::super::synthetics::SyntheticTestInput = serde_json::from_value(json!({
            "name": "Checkout", "type": "browser", "request": {"url": "https://shop.example.com"}, "frequencySeconds": 900,
            "steps": [{"type": "click", "params": {"element": {"userLocator": {"values": [{"type": "css", "value": "#pay"}]}}}}]
        }))
        .unwrap();
        let Json(test) = super::super::synthetics::create(State(st.clone()), Writer(ctx.clone()), Json(input)).await.unwrap();
        assert_eq!(test.steps[0].name, "Click on #pay");
        let Json(run) = super::super::synthetics::run_now(State(st.clone()), Writer(ctx.clone()), Path(test.id.clone())).await.unwrap();
        assert!(!run.detail.passed);
        let sent = seen.lock().unwrap()[0].1.clone();
        assert_eq!((sent["startUrl"].as_str(), sent["video"].as_str()), (Some("https://shop.example.com"), Some("always")));
        let browser = run.detail.browser.clone().unwrap();
        assert!(browser.steps[0].has_screenshot && browser.has_video);

        let fetch = |ctx: AuthContext, name: &str| artifact(State(st.clone()), ctx, Path((test.id.clone(), run.id, name.to_string())));
        let image = fetch(ctx.clone(), "step-0.jpg").await.unwrap();
        assert_eq!(image.headers()[header::CONTENT_TYPE], "image/jpeg");
        let bytes = axum::body::to_bytes(image.into_body(), usize::MAX).await.unwrap();
        assert_eq!(bytes.as_ref(), base64::engine::general_purpose::STANDARD.decode("/9j/AAA=").unwrap());
        assert_eq!(fetch(ctx.clone(), "video.webm").await.unwrap().headers()[header::CONTENT_TYPE], "video/webm");
        assert!(matches!(fetch(ctx.clone(), "step-1.jpg").await, Err(ApiError::NotFound(_))), "no screenshot for that step");
        let other = crate::test_support::admin_of_new_org(st, "browser-run-other").await;
        assert!(matches!(fetch(other, "step-0.jpg").await, Err(ApiError::NotFound(_))), "another org sees nothing");
        // The stored result says what was kept.
        let Json(stored) = super::super::synthetics::result(State(st.clone()), ctx.clone(), Path((test.id.clone(), run.id))).await.unwrap();
        assert_eq!(stored.detail.browser.unwrap().steps[1].failure.as_ref().unwrap().code, "LOCATE_ELEMENT");
    }

    #[test]
    fn artifact_names_are_checked() {
        assert_eq!(content_type("step-0.jpg"), Some("image/jpeg"));
        assert_eq!(content_type("step-99.jpg"), Some("image/jpeg"));
        assert_eq!(content_type("video.webm"), Some("video/webm"));
        for bad in ["step-.jpg", "step-x.jpg", "../step-0.jpg", "video.mp4", "step-0.jpg.exe", ""] {
            assert_eq!(content_type(bad), None, "{bad}");
        }
    }
}
