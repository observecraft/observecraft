//! The Google APIs the crawler calls, behind [`GcpApi`] (tests use a fake), and their implementation over
//! REST ([`Rest`]): this install's identity (a [`TokenSource`]: Application Default Credentials in
//! production) impersonates the customer's service account (IAM Credentials `generateAccessToken`,
//! cached until it expires), lists the projects it can see (Resource Manager), their metric descriptors
//! (with their sample period and ingest delay) and time series (Cloud Monitoring), aligned per minute as
//! their kind allows, with the monitored resource's user and system labels.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use tokio::sync::Mutex;

/// A metric type of a project.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Descriptor {
    /// `compute.googleapis.com/instance/cpu/utilization`
    pub metric_type: String,
    /// GAUGE | DELTA | CUMULATIVE
    pub kind: String,
    /// DOUBLE | INT64 | BOOL | DISTRIBUTION | STRING | MONEY
    pub value_type: String,
    /// How often a point is written (`metadata.samplePeriod`; 0 when not given).
    pub sample_period_ms: i64,
    /// How long a point takes to become readable (`metadata.ingestDelay`).
    pub ingest_delay_ms: i64,
}

/// A distribution point: its count, mean, sum of squared deviations and histogram (`bounds` are the
/// finite bucket boundaries: bucket 0 is below `bounds[0]`, bucket i is `[bounds[i-1], bounds[i])`, the
/// last one above the last bound).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Distribution {
    pub count: f64,
    pub mean: f64,
    pub sumsqdev: f64,
    pub bounds: Vec<f64>,
    pub counts: Vec<f64>,
}

impl Distribution {
    /// The `q` quantile (0..1), interpolated within its bucket; the underflow and overflow buckets give
    /// their finite bound. None without samples or buckets.
    pub fn percentile(&self, q: f64) -> Option<f64> {
        let total: f64 = self.counts.iter().sum();
        if total <= 0.0 || self.bounds.is_empty() {
            return None;
        }
        let rank = q * total;
        let mut before = 0.0;
        for (i, &c) in self.counts.iter().enumerate() {
            if c <= 0.0 || before + c < rank {
                before += c;
                continue;
            }
            if i == 0 {
                return Some(self.bounds[0]);
            }
            let lo = self.bounds[(i - 1).min(self.bounds.len() - 1)];
            let Some(&hi) = self.bounds.get(i) else { return Some(lo) };
            return Some(lo + (hi - lo) * ((rank - before) / c).clamp(0.0, 1.0));
        }
        self.bounds.last().copied()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Reading {
    Number(f64),
    Distribution(Distribution),
}

/// A point: the interval it covers (a cumulative counter's start is when it was last reset) and its value.
#[derive(Clone, Debug, PartialEq)]
pub struct Sample {
    pub start_ms: i64,
    pub time_ms: i64,
    pub value: Reading,
}

/// One time series: its monitored resource (labels, and the user and system labels Cloud Monitoring adds
/// as its metadata), the metric's labels, and its points.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Series {
    pub resource_type: String,
    pub resource_labels: Vec<(String, String)>,
    pub metric_labels: Vec<(String, String)>,
    pub user_labels: Vec<(String, String)>,
    pub system_labels: Vec<(String, String)>,
    pub points: Vec<Sample>,
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
/// booleans as the fraction of true, delta and cumulative distributions merged per minute. Cumulative
/// counters and gauge distributions are read raw (`ALIGN_NONE`): their running totals and points as
/// Google has them, as Datadog ingests them.
pub fn aligner(d: &Descriptor) -> Option<&'static str> {
    match (d.kind.as_str(), d.value_type.as_str()) {
        ("GAUGE", "DOUBLE" | "INT64") => Some("ALIGN_MEAN"),
        ("GAUGE", "BOOL") => Some("ALIGN_FRACTION_TRUE"),
        ("DELTA", "DOUBLE" | "INT64") => Some("ALIGN_SUM"),
        ("CUMULATIVE", "DOUBLE" | "INT64") | ("GAUGE", "DISTRIBUTION") => Some("ALIGN_NONE"),
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
        // 503: a busy backend, retried as throttling.
        429 | 503 => GcpError::Throttled(message),
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
    // Strings only: system labels may be lists or booleans.
    let mut out: Vec<(String, String)> = v.as_object().map(|m| m.iter().filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_string()))).collect()).unwrap_or_default();
    out.sort();
    out
}

/// `"60s"`, `"0.5s"` → milliseconds.
fn duration_ms(v: &Value) -> i64 {
    v.as_str().and_then(|s| s.strip_suffix('s')).and_then(|s| s.parse::<f64>().ok()).map(|s| (s * 1000.0) as i64).unwrap_or(0)
}

/// An int64 (a JSON string) or a double.
fn number(v: &Value) -> Option<f64> {
    v.as_f64().or_else(|| v.as_str().and_then(|s| s.parse().ok()))
}

/// The finite bucket boundaries of a distribution's `bucketOptions`.
fn bounds(o: &Value) -> Vec<f64> {
    let int = |v: &Value| number(v).unwrap_or(0.0) as usize;
    if let Some(l) = o.get("linearBuckets") {
        let (width, offset) = (number(&l["width"]).unwrap_or(0.0), number(&l["offset"]).unwrap_or(0.0));
        return (0..=int(&l["numFiniteBuckets"])).map(|i| offset + width * i as f64).collect();
    }
    if let Some(e) = o.get("exponentialBuckets") {
        let (growth, scale) = (number(&e["growthFactor"]).unwrap_or(0.0), number(&e["scale"]).unwrap_or(0.0));
        return (0..=int(&e["numFiniteBuckets"])).map(|i| scale * growth.powi(i as i32)).collect();
    }
    o["explicitBuckets"]["bounds"].as_array().map(|b| b.iter().filter_map(number).collect()).unwrap_or_default()
}

fn reading(v: &Value) -> Option<Reading> {
    if let Some(d) = v.get("distributionValue") {
        return Some(Reading::Distribution(Distribution {
            count: number(&d["count"]).unwrap_or(0.0),
            mean: number(&d["mean"]).unwrap_or(0.0),
            sumsqdev: number(&d["sumOfSquaredDeviation"]).unwrap_or(0.0),
            bounds: bounds(&d["bucketOptions"]),
            counts: d["bucketCounts"].as_array().map(|c| c.iter().map(|x| number(x).unwrap_or(0.0)).collect()).unwrap_or_default(),
        }));
    }
    let n = number(&v["doubleValue"]).or_else(|| number(&v["int64Value"])).or_else(|| v["boolValue"].as_bool().map(|b| if b { 1.0 } else { 0.0 }))?;
    Some(Reading::Number(n))
}

fn instant_ms(v: &Value) -> Option<i64> {
    Some(chrono::DateTime::parse_from_rfc3339(v.as_str()?).ok()?.timestamp_millis())
}

fn sample(p: &Value) -> Option<Sample> {
    let time_ms = instant_ms(&p["interval"]["endTime"])?;
    Some(Sample { start_ms: instant_ms(&p["interval"]["startTime"]).unwrap_or(time_ms), time_ms, value: reading(&p["value"])? })
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
            .filter_map(|d| {
                Some(Descriptor {
                    metric_type: d["type"].as_str()?.to_string(),
                    kind: d["metricKind"].as_str()?.to_string(),
                    value_type: d["valueType"].as_str()?.to_string(),
                    sample_period_ms: duration_ms(&d["metadata"]["samplePeriod"]),
                    ingest_delay_ms: duration_ms(&d["metadata"]["ingestDelay"]),
                })
            })
            .collect())
    }

    async fn time_series(&self, service_account: &str, project: &str, d: &Descriptor, start_ms: i64, end_ms: i64) -> Result<Vec<Series>, GcpError> {
        let Some(aligner) = aligner(d) else { return Ok(vec![]) };
        let token = self.impersonate(service_account).await?;
        let url = format!("{}/v3/projects/{project}/timeSeries", self.endpoints.monitoring);
        let mut query = vec![("filter", format!("metric.type = \"{}\"", d.metric_type)), ("interval.startTime", rfc3339(start_ms)), ("interval.endTime", rfc3339(end_ms))];
        if aligner != "ALIGN_NONE" {
            query.extend([("aggregation.alignmentPeriod", "60s".to_string()), ("aggregation.perSeriesAligner", aligner.to_string())]);
        }
        let all = self.pages(&token, &url, &query, "timeSeries").await?;
        Ok(all
            .iter()
            .map(|ts| Series {
                resource_type: ts["resource"]["type"].as_str().unwrap_or_default().to_string(),
                resource_labels: labels(&ts["resource"]["labels"]),
                metric_labels: labels(&ts["metric"]["labels"]),
                user_labels: labels(&ts["metadata"]["userLabels"]),
                system_labels: labels(&ts["metadata"]["systemLabels"]),
                points: ts["points"].as_array().map(Vec::as_slice).unwrap_or_default().iter().filter_map(sample).collect(),
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
        assert!(matches!(classify(503, "{}"), GcpError::Throttled(_)));
        let d = |kind: &str, value: &str| Descriptor { metric_type: "x".into(), kind: kind.into(), value_type: value.into(), ..Default::default() };
        assert_eq!(aligner(&d("GAUGE", "DOUBLE")), Some("ALIGN_MEAN"));
        assert_eq!(aligner(&d("DELTA", "INT64")), Some("ALIGN_SUM"));
        // Cumulative counters are read raw: their running total, and the crawler's deltas between points.
        assert_eq!(aligner(&d("CUMULATIVE", "INT64")), Some("ALIGN_NONE"));
        assert_eq!(aligner(&d("DELTA", "DISTRIBUTION")), Some("ALIGN_DELTA"));
        assert_eq!(aligner(&d("GAUGE", "STRING")), None);
    }

    #[test]
    fn distributions_are_read_with_their_buckets_and_give_percentiles() {
        let v = json!({ "distributionValue": {
            "count": "100", "mean": 42.5, "sumOfSquaredDeviation": 1200.0,
            "bucketOptions": { "explicitBuckets": { "bounds": [10, 20, 50, 100] } },
            "bucketCounts": ["0", "10", "40", "45", "5"]
        } });
        let Some(Reading::Distribution(d)) = reading(&v) else { panic!("a distribution") };
        assert_eq!((d.count, d.mean, d.sumsqdev), (100.0, 42.5, 1200.0));
        // 50 samples below 50, 45 in [50, 100): the 95th is the top of that bucket, the 99th in the overflow.
        assert_eq!(d.percentile(0.5), Some(50.0));
        assert_eq!(d.percentile(0.95), Some(100.0));
        assert_eq!(d.percentile(0.99), Some(100.0));
        assert_eq!(d.percentile(0.3), Some(20.0 + 30.0 * 0.5));
        // Exponential and linear buckets; trailing empty buckets omitted, as Google sends them.
        assert_eq!(bounds(&json!({ "exponentialBuckets": { "numFiniteBuckets": 3, "growthFactor": 2, "scale": 1 } })), [1.0, 2.0, 4.0, 8.0]);
        assert_eq!(bounds(&json!({ "linearBuckets": { "numFiniteBuckets": 2, "width": 5, "offset": 10 } })), [10.0, 15.0, 20.0]);
        let few = Distribution { count: 4.0, bounds: vec![1.0, 2.0, 4.0, 8.0], counts: vec![0.0, 0.0, 4.0], ..Default::default() };
        assert_eq!(few.percentile(0.5), Some(3.0));
        assert_eq!(Distribution::default().percentile(0.95), None);
        assert_eq!(reading(&json!({ "boolValue": true })), Some(Reading::Number(1.0)));
        assert_eq!(reading(&json!({ "int64Value": "7" })), Some(Reading::Number(7.0)));
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
                get(|| async {
                    Json(json!({ "metricDescriptors": [
                        { "type": "compute.googleapis.com/instance/cpu/utilization", "metricKind": "GAUGE", "valueType": "DOUBLE", "metadata": { "samplePeriod": "60s", "ingestDelay": "240s" } },
                        { "type": "kubernetes.io/container/restart_count", "metricKind": "CUMULATIVE", "valueType": "INT64" }
                    ] }))
                }),
            )
            .route(
                "/v3/projects/example-storefront/timeSeries",
                get(|Query(q): Query<HashMap<String, String>>| async move {
                    if q["filter"].contains("restart_count") {
                        // Read raw: no aggregation.
                        assert!(!q.contains_key("aggregation.perSeriesAligner"));
                        return Json(json!({ "timeSeries": [{
                            "metric": { "type": "kubernetes.io/container/restart_count" },
                            "resource": { "type": "k8s_container", "labels": { "cluster_name": "storefront-prod" } },
                            "points": [{ "interval": { "startTime": "2023-11-01T00:00:00Z", "endTime": "2023-11-14T22:14:00Z" }, "value": { "int64Value": "3" } }]
                        }] }));
                    }
                    assert_eq!(q["filter"], "metric.type = \"compute.googleapis.com/instance/cpu/utilization\"");
                    assert_eq!((q["aggregation.alignmentPeriod"].as_str(), q["aggregation.perSeriesAligner"].as_str()), ("60s", "ALIGN_MEAN"));
                    Json(json!({ "timeSeries": [{
                        "metric": { "type": "compute.googleapis.com/instance/cpu/utilization", "labels": { "instance_name": "web-1" } },
                        "resource": { "type": "gce_instance", "labels": { "instance_id": "1234567890", "zone": "europe-west1-b", "project_id": "example-storefront" } },
                        "metadata": { "userLabels": { "team": "storefront" }, "systemLabels": { "machine_type": "e2-standard-4", "network_tag": ["web"], "preemptible": false } },
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
        assert_eq!((descriptors[0].kind.as_str(), descriptors[0].sample_period_ms, descriptors[0].ingest_delay_ms), ("GAUGE", 60_000, 240_000));
        assert_eq!(descriptors[1].sample_period_ms, 0);
        let series = api.time_series(sa, "example-storefront", &descriptors[0], 1_700_000_000_000, 1_700_000_600_000).await.unwrap();
        assert_eq!(series[0].resource_type, "gce_instance");
        assert_eq!(series[0].resource_labels[0], ("instance_id".to_string(), "1234567890".to_string()));
        assert_eq!(series[0].user_labels, [("team".to_string(), "storefront".to_string())]);
        assert_eq!(series[0].system_labels, [("machine_type".to_string(), "e2-standard-4".to_string())]);
        let at = |t: i64, v: f64| Sample { start_ms: t, time_ms: t, value: Reading::Number(v) };
        assert_eq!(series[0].points, [at(1_700_000_040_000, 0.42), at(1_699_999_980_000, 7.0)]);
        let restarts = api.time_series(sa, "example-storefront", &descriptors[1], 1_700_000_000_000, 1_700_000_600_000).await.unwrap();
        assert_eq!(restarts[0].points[0], Sample { start_ms: 1_698_796_800_000, time_ms: 1_700_000_040_000, value: Reading::Number(3.0) });
        // One impersonation, cached.
        assert_eq!(seen.lock().unwrap().iter().filter(|s| s.starts_with("impersonate")).count(), 1);
        assert!(matches!(api.projects("denied@example-storefront.iam.gserviceaccount.com").await, Err(GcpError::Auth(m)) if m.contains("getAccessToken")));
    }
}
