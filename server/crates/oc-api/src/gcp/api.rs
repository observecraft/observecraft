//! The Google APIs the crawler calls, behind [`GcpApi`] (tests use a fake), and their implementation over
//! REST ([`Rest`]): this install's identity (a [`TokenSource`]: Application Default Credentials in
//! production) impersonates the customer's service account (IAM Credentials `generateAccessToken`,
//! cached until it expires), lists the projects it can see (Resource Manager), their metric descriptors
//! and time series (Cloud Monitoring), aligned per minute as their kind allows.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use tokio::sync::Mutex;

/// A metric type of a project.
#[derive(Clone, Debug, PartialEq)]
pub struct Descriptor {
    /// `compute.googleapis.com/instance/cpu/utilization`
    pub metric_type: String,
    /// GAUGE | DELTA | CUMULATIVE
    pub kind: String,
    /// DOUBLE | INT64 | BOOL | DISTRIBUTION | STRING | MONEY
    pub value_type: String,
}

/// One time series: its monitored resource, labels as `key:value` tags, and per-minute values.
#[derive(Clone, Debug, PartialEq)]
pub struct Series {
    pub resource_type: String,
    pub resource_labels: Vec<(String, String)>,
    pub metric_labels: Vec<(String, String)>,
    pub points: Vec<(i64, f64)>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum GcpError {
    /// This install can't impersonate the service account (Service Account Token Creator missing).
    Auth(String),
    /// A missing permission; `permission` when Google names it (`monitoring.timeSeries.list`).
    Forbidden { permission: Option<String>, message: String },
    /// The Cloud Monitoring API isn't enabled in the project.
    Disabled(String),
    Throttled(String),
    Other(String),
}

#[async_trait::async_trait]
pub trait GcpApi: Send + Sync {
    /// The ids of the active projects the service account can see.
    async fn projects(&self, service_account: &str) -> Result<Vec<String>, GcpError>;
    async fn descriptors(&self, service_account: &str, project: &str) -> Result<Vec<Descriptor>, GcpError>;
    async fn time_series(&self, service_account: &str, project: &str, d: &Descriptor, start_ms: i64, end_ms: i64) -> Result<Vec<Series>, GcpError>;
}

/// This install's own access token (cloud-platform scope).
#[async_trait::async_trait]
pub trait TokenSource: Send + Sync {
    async fn token(&self) -> Result<String, GcpError>;
}

/// Application Default Credentials: the metadata server (GCE, GKE workload identity), a key file
/// (`GOOGLE_APPLICATION_CREDENTIALS`) or gcloud's user credentials.
pub struct Adc(Arc<dyn gcp_auth::TokenProvider>);

impl Adc {
    pub async fn new() -> Result<Adc, String> {
        gcp_auth::provider().await.map(Adc).map_err(|e| e.to_string())
    }
}

#[async_trait::async_trait]
impl TokenSource for Adc {
    async fn token(&self) -> Result<String, GcpError> {
        self.0.token(&[CLOUD_PLATFORM]).await.map(|t| t.as_str().to_string()).map_err(|e| GcpError::Auth(format!("this install's GCP credentials: {e}")))
    }
}

const CLOUD_PLATFORM: &str = "https://www.googleapis.com/auth/cloud-platform";

pub struct Endpoints {
    pub iam_credentials: String,
    pub resource_manager: String,
    pub monitoring: String,
}

impl Default for Endpoints {
    fn default() -> Self {
        Endpoints {
            iam_credentials: "https://iamcredentials.googleapis.com".into(),
            resource_manager: "https://cloudresourcemanager.googleapis.com".into(),
            monitoring: "https://monitoring.googleapis.com".into(),
        }
    }
}

/// The aligner for a descriptor (None: not collected): gauges are averaged per minute, deltas summed,
/// cumulative counters differenced, booleans as the fraction of true, distributions by their mean.
pub fn aligner(d: &Descriptor) -> Option<&'static str> {
    match (d.kind.as_str(), d.value_type.as_str()) {
        ("GAUGE", "DOUBLE" | "INT64") => Some("ALIGN_MEAN"),
        ("GAUGE", "BOOL") => Some("ALIGN_FRACTION_TRUE"),
        ("DELTA", "DOUBLE" | "INT64") => Some("ALIGN_SUM"),
        ("CUMULATIVE", "DOUBLE" | "INT64") => Some("ALIGN_DELTA"),
        ("DELTA" | "CUMULATIVE", "DISTRIBUTION") => Some("ALIGN_DELTA"),
        _ => None,
    }
}

/// Classifies a Google API error response.
pub fn classify(status: u16, body: &str) -> GcpError {
    let v: Value = serde_json::from_str(body).unwrap_or_default();
    let message = v["error"]["message"].as_str().unwrap_or(body).to_string();
    let reason = v["error"]["details"].as_array().and_then(|d| d.iter().find_map(|x| x["reason"].as_str())).unwrap_or_default();
    match status {
        401 => GcpError::Auth(message),
        403 if reason == "SERVICE_DISABLED" || message.contains("has not been used in project") => GcpError::Disabled(message),
        403 => {
            let permission = message.split("Permission '").nth(1).or_else(|| message.split("permission '").nth(1)).and_then(|s| s.split('\'').next()).map(str::to_string);
            GcpError::Forbidden { permission, message }
        }
        429 => GcpError::Throttled(message),
        _ => GcpError::Other(format!("HTTP {status}: {message}")),
    }
}

/// An access token and when it stops being reused.
type Token = (String, Instant);

pub struct Rest {
    http: reqwest::Client,
    endpoints: Endpoints,
    identity: Arc<dyn TokenSource>,
    tokens: Mutex<HashMap<String, Token>>,
}

fn rfc3339(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms).unwrap_or_default().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

fn labels(v: &Value) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = v.as_object().map(|m| m.iter().map(|(k, v)| (k.clone(), v.as_str().unwrap_or_default().to_string())).collect()).unwrap_or_default();
    out.sort();
    out
}

fn value(v: &Value) -> Option<f64> {
    v["doubleValue"].as_f64().or_else(|| v["int64Value"].as_str().and_then(|s| s.parse().ok())).or_else(|| v["int64Value"].as_f64()).or_else(|| v["distributionValue"]["mean"].as_f64())
}

impl Rest {
    pub fn new(http: reqwest::Client, endpoints: Endpoints, identity: Arc<dyn TokenSource>) -> Self {
        Rest { http, endpoints, identity, tokens: Mutex::default() }
    }

    async fn send(&self, req: reqwest::RequestBuilder) -> Result<Value, GcpError> {
        let res = req.send().await.map_err(|e| GcpError::Other(e.to_string()))?;
        let status = res.status().as_u16();
        let body = res.text().await.unwrap_or_default();
        if !(200..300).contains(&status) {
            return Err(classify(status, &body));
        }
        serde_json::from_str(&body).map_err(|e| GcpError::Other(e.to_string()))
    }

    /// A token of the customer's service account, impersonated with this install's identity.
    async fn impersonate(&self, service_account: &str) -> Result<String, GcpError> {
        if let Some((t, until)) = self.tokens.lock().await.get(service_account) {
            if Instant::now() < *until {
                return Ok(t.clone());
            }
        }
        let own = self.identity.token().await?;
        let url = format!("{}/v1/projects/-/serviceAccounts/{service_account}:generateAccessToken", self.endpoints.iam_credentials);
        let v = self.send(self.http.post(url).bearer_auth(own).json(&json!({ "scope": [CLOUD_PLATFORM], "lifetime": "3600s" }))).await.map_err(|e| match e {
            GcpError::Forbidden { message, .. } | GcpError::Other(message) => GcpError::Auth(message),
            e => e,
        })?;
        let token = v["accessToken"].as_str().ok_or_else(|| GcpError::Auth("no access token".into()))?.to_string();
        self.tokens.lock().await.insert(service_account.to_string(), (token.clone(), Instant::now() + Duration::from_secs(3300)));
        Ok(token)
    }

    /// Every page of a list (`items`, `nextPageToken`).
    async fn pages(&self, token: &str, url: &str, query: &[(&str, String)], items: &str) -> Result<Vec<Value>, GcpError> {
        let (mut out, mut page) = (vec![], None::<String>);
        loop {
            let mut q: Vec<(&str, String)> = query.to_vec();
            if let Some(p) = &page {
                q.push(("pageToken", p.clone()));
            }
            let v = self.send(self.http.get(url).query(&q).bearer_auth(token)).await?;
            out.extend(v[items].as_array().cloned().unwrap_or_default());
            match v["nextPageToken"].as_str().filter(|t| !t.is_empty()) {
                Some(t) => page = Some(t.to_string()),
                None => return Ok(out),
            }
        }
    }
}

#[async_trait::async_trait]
impl GcpApi for Rest {
    async fn projects(&self, service_account: &str) -> Result<Vec<String>, GcpError> {
        let token = self.impersonate(service_account).await?;
        let url = format!("{}/v1/projects", self.endpoints.resource_manager);
        let all = self.pages(&token, &url, &[("filter", "lifecycleState:ACTIVE".into())], "projects").await?;
        Ok(all.iter().filter_map(|p| p["projectId"].as_str().map(str::to_string)).collect())
    }

    async fn descriptors(&self, service_account: &str, project: &str) -> Result<Vec<Descriptor>, GcpError> {
        let token = self.impersonate(service_account).await?;
        let url = format!("{}/v3/projects/{project}/metricDescriptors", self.endpoints.monitoring);
        let all = self.pages(&token, &url, &[], "metricDescriptors").await?;
        Ok(all
            .iter()
            .filter_map(|d| Some(Descriptor { metric_type: d["type"].as_str()?.to_string(), kind: d["metricKind"].as_str()?.to_string(), value_type: d["valueType"].as_str()?.to_string() }))
            .collect())
    }

    async fn time_series(&self, service_account: &str, project: &str, d: &Descriptor, start_ms: i64, end_ms: i64) -> Result<Vec<Series>, GcpError> {
        let Some(aligner) = aligner(d) else { return Ok(vec![]) };
        let token = self.impersonate(service_account).await?;
        let url = format!("{}/v3/projects/{project}/timeSeries", self.endpoints.monitoring);
        let query = [
            ("filter", format!("metric.type = \"{}\"", d.metric_type)),
            ("interval.startTime", rfc3339(start_ms)),
            ("interval.endTime", rfc3339(end_ms)),
            ("aggregation.alignmentPeriod", "60s".into()),
            ("aggregation.perSeriesAligner", aligner.into()),
        ];
        let all = self.pages(&token, &url, &query, "timeSeries").await?;
        Ok(all
            .iter()
            .map(|ts| Series {
                resource_type: ts["resource"]["type"].as_str().unwrap_or_default().to_string(),
                resource_labels: labels(&ts["resource"]["labels"]),
                metric_labels: labels(&ts["metric"]["labels"]),
                points: ts["points"]
                    .as_array()
                    .map(Vec::as_slice)
                    .unwrap_or_default()
                    .iter()
                    .filter_map(|p| Some((chrono::DateTime::parse_from_rfc3339(p["interval"]["endTime"].as_str()?).ok()?.timestamp_millis(), value(&p["value"])?)))
                    .collect(),
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::extract::{Path, Query};
    use axum::http::{HeaderMap, StatusCode};
    use axum::routing::{get, post};
    use axum::{Json, Router};
    use std::sync::Mutex as StdMutex;

    #[test]
    fn errors_and_aligners() {
        assert_eq!(
            classify(403, r#"{"error":{"code":403,"message":"Permission 'monitoring.timeSeries.list' denied on resource (or it may not exist).","status":"PERMISSION_DENIED"}}"#),
            GcpError::Forbidden { permission: Some("monitoring.timeSeries.list".into()), message: "Permission 'monitoring.timeSeries.list' denied on resource (or it may not exist).".into() }
        );
        assert!(matches!(classify(403, r#"{"error":{"message":"Cloud Monitoring API has not been used in project 123 before or it is disabled.","details":[{"reason":"SERVICE_DISABLED"}]}}"#), GcpError::Disabled(_)));
        assert!(matches!(classify(429, "{}"), GcpError::Throttled(_)));
        let d = |kind: &str, value: &str| Descriptor { metric_type: "x".into(), kind: kind.into(), value_type: value.into() };
        assert_eq!(aligner(&d("GAUGE", "DOUBLE")), Some("ALIGN_MEAN"));
        assert_eq!(aligner(&d("DELTA", "INT64")), Some("ALIGN_SUM"));
        assert_eq!(aligner(&d("CUMULATIVE", "INT64")), Some("ALIGN_DELTA"));
        assert_eq!(aligner(&d("DELTA", "DISTRIBUTION")), Some("ALIGN_DELTA"));
        assert_eq!(aligner(&d("GAUGE", "STRING")), None);
    }

    struct Static;

    #[async_trait::async_trait]
    impl TokenSource for Static {
        async fn token(&self) -> Result<String, GcpError> {
            Ok("install-token".into())
        }
    }

    /// A stand-in for Google: IAM Credentials, Resource Manager (paged) and Cloud Monitoring.
    async fn fake_google() -> (String, Arc<StdMutex<Vec<String>>>) {
        let seen: Arc<StdMutex<Vec<String>>> = Arc::default();
        let bearer = |h: &HeaderMap| h.get("authorization").and_then(|v| v.to_str().ok()).unwrap_or_default().to_string();
        let log = seen.clone();
        let app = Router::new()
            .route(
                "/v1/projects/-/serviceAccounts/{sa}",
                post(move |h: HeaderMap, Path(sa): Path<String>| {
                    let log = log.clone();
                    async move {
                        assert_eq!(bearer(&h), "Bearer install-token");
                        log.lock().unwrap().push(format!("impersonate {sa}"));
                        if sa.starts_with("denied@") {
                            return (StatusCode::FORBIDDEN, Json(json!({ "error": { "message": "Permission 'iam.serviceAccounts.getAccessToken' denied on resource" } })));
                        }
                        (StatusCode::OK, Json(json!({ "accessToken": "customer-token", "expireTime": "2099-01-01T00:00:00Z" })))
                    }
                }),
            )
            .route(
                "/v1/projects",
                get(move |h: HeaderMap, Query(q): Query<HashMap<String, String>>| async move {
                    assert_eq!(bearer(&h), "Bearer customer-token");
                    if q.get("pageToken").map(String::as_str) == Some("p2") {
                        return Json(json!({ "projects": [{ "projectId": "example-catalog" }] }));
                    }
                    Json(json!({ "projects": [{ "projectId": "example-storefront" }], "nextPageToken": "p2" }))
                }),
            )
            .route(
                "/v3/projects/example-storefront/metricDescriptors",
                get(|| async { Json(json!({ "metricDescriptors": [{ "type": "compute.googleapis.com/instance/cpu/utilization", "metricKind": "GAUGE", "valueType": "DOUBLE" }] })) }),
            )
            .route(
                "/v3/projects/example-storefront/timeSeries",
                get(|Query(q): Query<HashMap<String, String>>| async move {
                    assert_eq!(q["filter"], "metric.type = \"compute.googleapis.com/instance/cpu/utilization\"");
                    assert_eq!((q["aggregation.alignmentPeriod"].as_str(), q["aggregation.perSeriesAligner"].as_str()), ("60s", "ALIGN_MEAN"));
                    Json(json!({ "timeSeries": [{
                        "metric": { "type": "compute.googleapis.com/instance/cpu/utilization", "labels": { "instance_name": "web-1" } },
                        "resource": { "type": "gce_instance", "labels": { "instance_id": "1234567890", "zone": "europe-west1-b", "project_id": "example-storefront" } },
                        "points": [{ "interval": { "endTime": "2023-11-14T22:14:00Z" }, "value": { "doubleValue": 0.42 } }, { "interval": { "endTime": "2023-11-14T22:13:00Z" }, "value": { "int64Value": "7" } }]
                    }] }))
                }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (base, seen)
    }

    fn rest(base: &str) -> Rest {
        Rest::new(reqwest::Client::new(), Endpoints { iam_credentials: base.into(), resource_manager: base.into(), monitoring: base.into() }, Arc::new(Static))
    }

    #[tokio::test]
    async fn the_rest_client_impersonates_lists_and_reads_time_series() {
        let (base, seen) = fake_google().await;
        let api = rest(&base);
        let sa = "observecraft@example-storefront.iam.gserviceaccount.com";
        assert_eq!(api.projects(sa).await.unwrap(), ["example-storefront", "example-catalog"]);
        let descriptors = api.descriptors(sa, "example-storefront").await.unwrap();
        assert_eq!(descriptors[0].kind, "GAUGE");
        let series = api.time_series(sa, "example-storefront", &descriptors[0], 1_700_000_000_000, 1_700_000_600_000).await.unwrap();
        assert_eq!(series[0].resource_type, "gce_instance");
        assert_eq!(series[0].resource_labels[0], ("instance_id".to_string(), "1234567890".to_string()));
        assert_eq!(series[0].points, [(1_700_000_040_000, 0.42), (1_699_999_980_000, 7.0)]);
        // One impersonation, cached.
        assert_eq!(seen.lock().unwrap().iter().filter(|s| s.starts_with("impersonate")).count(), 1);
        assert!(matches!(api.projects("denied@example-storefront.iam.gserviceaccount.com").await, Err(GcpError::Auth(m)) if m.contains("getAccessToken")));
    }
}
