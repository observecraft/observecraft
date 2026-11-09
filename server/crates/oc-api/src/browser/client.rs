//! The server's side of oc-browser (`browser/` at the repo root): runs a browser test there and reads its
//! result. oc-browser answers 503 while every browser slot is busy; the client waits for a slot.

use std::time::Duration;

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use super::steps::Step;

/// How long a run waits for a free browser slot, at most.
const MAX_QUEUE: Duration = Duration::from_secs(300);
/// A run's answer, at most (oc-browser stops a run after 900 s).
const MAX_RUN: Duration = Duration::from_secs(960);

#[derive(Clone, PartialEq)]
pub struct BrowserSettings {
    pub url: String,
    pub token: String,
}

impl std::fmt::Debug for BrowserSettings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BrowserSettings").field("url", &self.url).finish_non_exhaustive()
    }
}

impl BrowserSettings {
    /// `OC_BROWSER_URL` (none: browser tests are off) and `OC_BROWSER_TOKEN` (required with it, ≥ 16 characters).
    pub fn from_env(var: impl Fn(&str) -> Option<String>) -> Result<Option<Self>, String> {
        let get = |k: &str| var(k).map(|v| v.trim().to_string()).filter(|v| !v.is_empty());
        let Some(url) = get("OC_BROWSER_URL") else { return Ok(None) };
        if !(url.starts_with("http://") || url.starts_with("https://")) || url::Url::parse(&url).is_err() {
            return Err(format!("OC_BROWSER_URL must be oc-browser's http(s) address, not {url}"));
        }
        let token = get("OC_BROWSER_TOKEN").ok_or("OC_BROWSER_TOKEN is required with OC_BROWSER_URL")?;
        if token.len() < 16 {
            return Err("OC_BROWSER_TOKEN must be at least 16 characters".into());
        }
        Ok(Some(BrowserSettings { url: url.trim_end_matches('/').to_string(), token }))
    }
}

/// When the run's video comes back.
#[derive(Serialize, Clone, Copy, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum VideoMode {
    OnFailure,
    Always,
}

#[derive(Serialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct RunRequest<'a> {
    pub start_url: &'a str,
    pub steps: &'a [Step],
    pub variables: &'a std::collections::HashMap<String, String>,
    pub secrets: &'a [String],
    pub device_id: &'a str,
    pub video: VideoMode,
}

#[derive(Serialize, Deserialize, ToSchema, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BrowserError {
    #[serde(rename = "type")]
    pub kind: String,
    pub name: String,
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<u16>,
}

#[derive(Serialize, Deserialize, ToSchema, Clone, Debug, PartialEq)]
pub struct BrowserFailure {
    /// Datadog's failure codes: LOCATE_ELEMENT, ASSERTION_FAILURE, OPEN_URL, …
    pub code: String,
    pub message: String,
}

#[derive(Serialize, Deserialize, ToSchema, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BrowserDevice {
    pub id: String,
    pub width: u32,
    pub height: u32,
    #[serde(default)]
    pub is_mobile: bool,
}

/// One step of a run, as oc-browser reports it; the screenshot travels apart from the stored result.
#[derive(Serialize, Deserialize, ToSchema, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BrowserStepResult {
    pub name: String,
    #[serde(rename = "type")]
    pub kind: String,
    /// passed | failed | skipped
    pub status: String,
    #[serde(default)]
    pub allow_failure: bool,
    #[serde(default)]
    pub is_critical: bool,
    pub duration_ms: f64,
    #[serde(default)]
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure: Option<BrowserFailure>,
    /// The extracted value of extract steps.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    /// JPEG, base64 (only in oc-browser's answer).
    #[serde(default, skip_serializing)]
    #[schema(ignore)]
    pub screenshot: Option<String>,
    /// A screenshot is stored for the step.
    #[serde(default)]
    pub has_screenshot: bool,
    #[serde(default)]
    pub browser_errors: Vec<BrowserError>,
}

/// A browser test run, as oc-browser reports it.
#[derive(Serialize, Deserialize, ToSchema, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BrowserRunResult {
    pub passed: bool,
    pub duration_ms: f64,
    pub start_url: String,
    #[serde(default)]
    pub browser_version: String,
    pub device: BrowserDevice,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure: Option<BrowserFailure>,
    pub steps: Vec<BrowserStepResult>,
    /// WebM, base64 (only in oc-browser's answer).
    #[serde(default, skip_serializing)]
    #[schema(ignore)]
    pub video: Option<String>,
    /// A video is stored for the run.
    #[serde(default)]
    pub has_video: bool,
}

/// An agent session to open (AI Testing): where it starts and the hosts its page may navigate to.
#[derive(Serialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct OpenSession<'a> {
    pub start_url: &'a str,
    pub device_id: &'a str,
    pub allowed_hosts: Option<Vec<String>>,
}

/// What an agent session's action did: the page after it and, for actions a test can repeat, the step.
#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ActionResult {
    /// Set when a session was opened.
    #[serde(default)]
    pub id: Option<String>,
    pub ok: bool,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub url_before: String,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub snapshot: String,
    #[serde(default)]
    pub step: Option<Step>,
    /// API calls the page made: `{method, url, status}`.
    #[serde(default)]
    pub requests: Vec<serde_json::Value>,
}

/// What AI Testing needs of a browser: agent sessions, and test runs to verify what it proposes.
pub trait Browser: Send + Sync {
    fn open_session(&self, req: &OpenSession<'_>) -> impl std::future::Future<Output = Result<ActionResult, String>> + Send;
    fn act(&self, id: &str, action: &serde_json::Value, variables: &std::collections::HashMap<String, String>) -> impl std::future::Future<Output = Result<ActionResult, String>> + Send;
    fn end_session(&self, id: &str) -> impl std::future::Future<Output = ()> + Send;
    fn run_test(&self, req: &RunRequest<'_>) -> impl std::future::Future<Output = Result<BrowserRunResult, String>> + Send;
}

pub struct BrowserClient {
    settings: BrowserSettings,
    http: reqwest::Client,
    /// Pause between tries while oc-browser is busy (its Retry-After, capped by this in tests).
    retry_cap: Duration,
}

impl BrowserClient {
    pub fn new(settings: BrowserSettings) -> Self {
        BrowserClient { settings, http: reqwest::Client::new(), retry_cap: Duration::from_secs(15) }
    }

    /// Runs a browser test; `Err` when oc-browser can't run it (unreachable, busy for too long, refused).
    pub async fn run(&self, request: &RunRequest<'_>) -> Result<BrowserRunResult, String> {
        let started = std::time::Instant::now();
        loop {
            let res = self
                .http
                .post(format!("{}/v1/runs", self.settings.url))
                .bearer_auth(&self.settings.token)
                .timeout(MAX_RUN)
                .json(request)
                .send()
                .await
                .map_err(|e| format!("oc-browser can't be reached: {e}"))?;
            let status = res.status();
            if status == reqwest::StatusCode::SERVICE_UNAVAILABLE {
                if started.elapsed() >= MAX_QUEUE {
                    return Err("Every browser of oc-browser stayed busy; the run was not started".into());
                }
                let wait = res.headers().get("retry-after").and_then(|v| v.to_str().ok()).and_then(|v| v.parse().ok()).map(Duration::from_secs).unwrap_or(Duration::from_secs(5));
                tokio::time::sleep(wait.min(self.retry_cap)).await;
                continue;
            }
            if !status.is_success() {
                let body: serde_json::Value = res.json().await.unwrap_or_default();
                return Err(format!("oc-browser refused the run ({status}): {}", body["error"].as_str().unwrap_or("no reason given")));
            }
            return res.json().await.map_err(|e| format!("oc-browser's answer could not be read: {e}"));
        }
    }
}

impl BrowserClient {
    /// A JSON call to oc-browser, waiting for a free slot while it answers 503.
    async fn call(&self, method: reqwest::Method, path: &str, body: Option<serde_json::Value>, timeout: Duration) -> Result<reqwest::Response, String> {
        let started = std::time::Instant::now();
        loop {
            let mut req = self.http.request(method.clone(), format!("{}{path}", self.settings.url)).bearer_auth(&self.settings.token).timeout(timeout);
            if let Some(b) = &body {
                req = req.json(b);
            }
            let res = req.send().await.map_err(|e| format!("oc-browser can't be reached: {e}"))?;
            if res.status() == reqwest::StatusCode::SERVICE_UNAVAILABLE && started.elapsed() < MAX_QUEUE {
                let wait = res.headers().get("retry-after").and_then(|v| v.to_str().ok()).and_then(|v| v.parse().ok()).map(Duration::from_secs).unwrap_or(Duration::from_secs(5));
                tokio::time::sleep(wait.min(self.retry_cap)).await;
                continue;
            }
            if !res.status().is_success() {
                let status = res.status();
                let body: serde_json::Value = res.json().await.unwrap_or_default();
                return Err(format!("oc-browser refused ({status}): {}", body["error"].as_str().unwrap_or("no reason given")));
            }
            return Ok(res);
        }
    }
}

impl Browser for BrowserClient {
    async fn open_session(&self, req: &OpenSession<'_>) -> Result<ActionResult, String> {
        let body = serde_json::to_value(req).map_err(|e| e.to_string())?;
        let res = self.call(reqwest::Method::POST, "/v1/sessions", Some(body), Duration::from_secs(60)).await?;
        res.json().await.map_err(|e| format!("oc-browser's answer could not be read: {e}"))
    }
    async fn act(&self, id: &str, action: &serde_json::Value, variables: &std::collections::HashMap<String, String>) -> Result<ActionResult, String> {
        let path = format!("/v1/sessions/{}/actions", id.replace(|c: char| !c.is_ascii_alphanumeric() && c != '-', ""));
        let res = self.call(reqwest::Method::POST, &path, Some(serde_json::json!({"action": action, "variables": variables})), Duration::from_secs(90)).await?;
        res.json().await.map_err(|e| format!("oc-browser's answer could not be read: {e}"))
    }
    async fn end_session(&self, id: &str) {
        let path = format!("/v1/sessions/{}", id.replace(|c: char| !c.is_ascii_alphanumeric() && c != '-', ""));
        if let Err(e) = self.call(reqwest::Method::DELETE, &path, None, Duration::from_secs(15)).await {
            tracing::debug!(error = %e, "ending a browser session");
        }
    }
    async fn run_test(&self, req: &RunRequest<'_>) -> Result<BrowserRunResult, String> {
        self.run(req).await
    }
}

/// A scripted oc-browser for tests.
#[cfg(test)]
pub mod testing {
    use std::sync::{Arc, Mutex};

    /// A scripted oc-browser: answers 503 `busy` times, then the result; keeps what it was sent.
    pub async fn fake(busy: usize, status: u16, body: serde_json::Value) -> (String, Arc<Mutex<Vec<(Option<String>, serde_json::Value)>>>) {
        use axum::response::IntoResponse;
        let seen = Arc::new(Mutex::new(Vec::new()));
        let left = Arc::new(Mutex::new(busy));
        let s = seen.clone();
        let app = axum::Router::new().route(
            "/v1/runs",
            axum::routing::post(move |headers: axum::http::HeaderMap, axum::Json(req): axum::Json<serde_json::Value>| {
                let (s, left, body) = (s.clone(), left.clone(), body.clone());
                async move {
                    s.lock().unwrap().push((headers.get("authorization").and_then(|v| v.to_str().ok()).map(Into::into), req));
                    let mut l = left.lock().unwrap();
                    if *l > 0 {
                        *l -= 1;
                        return (axum::http::StatusCode::SERVICE_UNAVAILABLE, [("retry-after", "1")], "{\"error\":\"busy\"}").into_response();
                    }
                    (axum::http::StatusCode::from_u16(status).unwrap(), axum::Json(body)).into_response()
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (format!("http://{addr}"), seen)
    }

    pub fn answer() -> serde_json::Value {
        serde_json::json!({
            "passed": false, "durationMs": 1234, "startUrl": "https://shop.example.com", "browserVersion": "158.0", "error": "Cannot locate element #pay",
            "failure": {"code": "LOCATE_ELEMENT", "message": "Cannot locate element #pay"},
            "device": {"id": "chrome.laptop_large", "width": 1440, "height": 1100, "isMobile": false},
            "steps": [
                {"name": "Navigate to https://shop.example.com", "type": "goToUrl", "status": "passed", "allowFailure": false, "isCritical": false, "durationMs": 800, "url": "https://shop.example.com/", "screenshot": "/9j/AAA=", "browserErrors": []},
                {"name": "Click on #pay", "type": "click", "status": "failed", "allowFailure": false, "isCritical": false, "durationMs": 400, "url": "https://shop.example.com/", "error": "Cannot locate element #pay",
                 "failure": {"code": "LOCATE_ELEMENT", "message": "Cannot locate element #pay"}, "browserErrors": [{"type": "network", "name": "https://shop.example.com/api/cart", "description": "500 Internal Server Error", "status": 500}]}
            ],
            "video": "GkXfow=="
        })
    }
}

#[cfg(test)]
mod tests {
    use super::testing::{answer, fake};
    use super::*;
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    fn env(vars: &[(&str, &str)]) -> Result<Option<BrowserSettings>, String> {
        let vars: HashMap<String, String> = vars.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        BrowserSettings::from_env(|k| vars.get(k).cloned())
    }

    #[test]
    fn settings_come_from_the_environment() {
        assert_eq!(env(&[]), Ok(None));
        assert_eq!(
            env(&[("OC_BROWSER_URL", "http://oc-browser:8095/"), ("OC_BROWSER_TOKEN", "0123456789abcdef")]),
            Ok(Some(BrowserSettings { url: "http://oc-browser:8095".into(), token: "0123456789abcdef".into() }))
        );
        assert!(env(&[("OC_BROWSER_URL", "http://oc-browser:8095")]).unwrap_err().contains("OC_BROWSER_TOKEN"));
        assert!(env(&[("OC_BROWSER_URL", "http://oc-browser:8095"), ("OC_BROWSER_TOKEN", "short")]).unwrap_err().contains("16"));
        assert!(env(&[("OC_BROWSER_URL", "oc-browser"), ("OC_BROWSER_TOKEN", "0123456789abcdef")]).unwrap_err().contains("OC_BROWSER_URL"));
        let shown = format!("{:?}", env(&[("OC_BROWSER_URL", "http://b:1"), ("OC_BROWSER_TOKEN", "0123456789abcdef")]).unwrap().unwrap());
        assert!(!shown.contains("0123456789abcdef"), "{shown}");
    }

    fn client(url: String) -> BrowserClient {
        BrowserClient { settings: BrowserSettings { url, token: "0123456789abcdef".into() }, http: reqwest::Client::new(), retry_cap: Duration::from_millis(20) }
    }

    #[tokio::test]
    async fn a_run_sends_the_test_and_reads_the_result() {
        let (url, seen) = fake(0, 200, answer()).await;
        let steps: Vec<Step> = vec![serde_json::from_value(serde_json::json!({"name": "Click on #pay", "type": "click", "params": {"element": {"userLocator": {"values": [{"type": "css", "value": "#pay"}]}}}})).unwrap()];
        let vars = HashMap::from([("USER".to_string(), "ana".to_string())]);
        let req = RunRequest { start_url: "https://shop.example.com", steps: &steps, variables: &vars, secrets: &["pw".into()], device_id: "chrome.laptop_large", video: VideoMode::OnFailure };
        let r = client(url).run(&req).await.unwrap();
        assert!(!r.passed);
        assert_eq!(r.failure.as_ref().unwrap().code, "LOCATE_ELEMENT");
        assert_eq!(r.steps[0].screenshot.as_deref(), Some("/9j/AAA="));
        assert_eq!(r.steps[1].browser_errors[0].status, Some(500));
        assert_eq!(r.video.as_deref(), Some("GkXfow=="));
        let (auth, body) = seen.lock().unwrap()[0].clone();
        assert_eq!(auth.as_deref(), Some("Bearer 0123456789abcdef"));
        assert_eq!(body["startUrl"], "https://shop.example.com");
        assert_eq!(body["steps"][0]["params"]["element"]["userLocator"]["values"][0]["value"], "#pay");
        assert_eq!(body["variables"]["USER"], "ana");
        assert_eq!(body["video"], "onFailure");
        // Stored results keep no image data.
        let stored = serde_json::to_value(&r).unwrap();
        assert!(stored.get("video").is_none() && stored["steps"][0].get("screenshot").is_none(), "{stored}");
    }

    #[tokio::test]
    async fn a_busy_runner_is_waited_for() {
        let (url, seen) = fake(2, 200, answer()).await;
        let r = client(url).run(&RunRequest { start_url: "https://shop.example.com", steps: &[], variables: &HashMap::new(), secrets: &[], device_id: "", video: VideoMode::OnFailure }).await;
        assert!(r.is_ok(), "{r:?}");
        assert_eq!(seen.lock().unwrap().len(), 3);
    }

    #[tokio::test]
    async fn sessions_open_act_and_end() {
        use axum::response::IntoResponse;
        let seen = Arc::new(Mutex::new(Vec::<(String, serde_json::Value)>::new()));
        let s = seen.clone();
        let app = axum::Router::new().fallback(move |method: axum::http::Method, uri: axum::http::Uri, body: String| {
            let s = s.clone();
            async move {
                s.lock().unwrap().push((format!("{method} {}", uri.path()), serde_json::from_str(&body).unwrap_or_default()));
                match uri.path() {
                    "/v1/sessions" => axum::Json(serde_json::json!({"id": "s1", "ok": true, "urlBefore": "", "url": "https://shop.example.com/", "title": "Shop", "snapshot": "URL: https://shop.example.com/", "requests": []})).into_response(),
                    "/v1/sessions/s1/actions" => axum::Json(serde_json::json!({"ok": true, "urlBefore": "https://shop.example.com/", "url": "https://shop.example.com/cart", "title": "Cart", "snapshot": "URL: …/cart",
                        "step": {"name": "Click on Cart", "type": "click", "params": {"element": {"userLocator": {"values": [{"type": "css", "value": "#cart"}]}}}}, "requests": []})).into_response(),
                    "/v1/sessions/s1" => axum::http::StatusCode::NO_CONTENT.into_response(),
                    _ => (axum::http::StatusCode::NOT_FOUND, axum::Json(serde_json::json!({"error": "Unknown or expired session"}))).into_response(),
                }
            }
        });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let c = client(format!("http://{addr}"));
        let opened = c.open_session(&OpenSession { start_url: "https://shop.example.com", device_id: "chrome.laptop_large", allowed_hosts: Some(vec!["shop.example.com".into()]) }).await.unwrap();
        assert_eq!((opened.id.as_deref(), opened.title.as_str()), (Some("s1"), "Shop"));
        let vars = HashMap::from([("SHOP_PASSWORD".to_string(), "hunter2".to_string())]);
        let clicked = c.act("s1", &serde_json::json!({"type": "click", "ref": "e3"}), &vars).await.unwrap();
        assert_eq!(clicked.step.unwrap().name, "Click on Cart");
        assert!(c.act("nope", &serde_json::json!({"type": "snapshot"}), &HashMap::new()).await.unwrap_err().contains("expired"));
        c.end_session("s1").await;
        let seen = seen.lock().unwrap();
        assert_eq!(seen[0].0, "POST /v1/sessions");
        assert_eq!(seen[0].1["allowedHosts"][0], "shop.example.com");
        assert_eq!(seen[1].1["variables"]["SHOP_PASSWORD"], "hunter2");
        assert_eq!(seen[3].0, "DELETE /v1/sessions/s1");
    }

    #[tokio::test]
    async fn refusals_and_unreachable_runners_are_errors() {
        let (url, _) = fake(0, 400, serde_json::json!({"error": "startUrl must be an http or https URL"})).await;
        let req = RunRequest { start_url: "https://shop.example.com", steps: &[], variables: &HashMap::new(), secrets: &[], device_id: "", video: VideoMode::OnFailure };
        assert!(client(url).run(&req).await.unwrap_err().contains("startUrl must be"));
        assert!(client("http://127.0.0.1:1".into()).run(&req).await.unwrap_err().contains("oc-browser"));
    }
}
