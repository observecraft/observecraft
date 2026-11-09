//! The Azure APIs the crawler calls, behind [`AzureApi`] (tests use a fake), and their implementation
//! over REST ([`Rest`]): an app registration's token (OAuth client credentials, cached until it
//! expires), the subscriptions it can read, their resources and metric definitions (Azure Resource
//! Manager), Resource Health's availability of each resource, and metric values from the Azure Monitor
//! batch API (`metrics:getBatch`: up to 50 resources of one type and region and 20 metrics per call), or,
//! for global resources (location `global`: DNS zones, Front Door, Traffic Manager), which no regional
//! endpoint serves, from Azure Resource Manager's per-resource metrics API.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use serde_json::Value;
use tokio::sync::Mutex;

/// An app registration's credentials.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Credentials {
    pub tenant_id: String,
    pub client_id: String,
    pub client_secret: String,
}

/// An Azure resource.
#[derive(Clone, Debug, PartialEq)]
pub struct Resource {
    pub id: String,
    pub name: String,
    /// `Microsoft.Compute/virtualMachines`
    pub kind: String,
    pub location: String,
    pub resource_group: String,
    /// `key:value`
    pub tags: Vec<String>,
}

/// What Azure Monitor has for one metric of a resource type (its metric definition).
#[derive(Clone, Debug, PartialEq)]
pub struct MetricDef {
    pub name: String,
    /// `primaryAggregationType`: `Average`, `Total`, `Count`, `Maximum`, `Minimum` or `None`.
    pub primary: String,
    /// `supportedAggregationTypes` (empty when Azure doesn't say).
    pub aggregations: Vec<String>,
    /// Its finest time grain (one minute when Azure doesn't say).
    pub grain_ms: i64,
    /// Its dimensions (`ApiName`, `Instance`, …).
    pub dimensions: Vec<String>,
}

/// One metric and aggregation of one resource (and one combination of the split dimensions' values).
#[derive(Clone, Debug, PartialEq)]
pub struct Series {
    pub resource_id: String,
    pub metric: String,
    pub aggregation: String,
    /// The split dimensions' values, as Azure names them (`apiname`, `GetBlob`).
    pub dimensions: Vec<(String, String)>,
    pub points: Vec<(i64, f64)>,
}

/// A metrics read: `aggregations` of `metrics` of `resource_ids` (one type and region; at most 50 resources and 20
/// metrics) at one time grain, split by `dimensions` (each one the metrics have).
#[derive(Clone, Copy, Debug)]
pub struct MetricsQuery<'a> {
    pub subscription: &'a str,
    pub region: &'a str,
    pub resource_type: &'a str,
    pub resource_ids: &'a [String],
    pub metrics: &'a [String],
    pub aggregations: &'a [&'a str],
    pub grain_ms: i64,
    pub dimensions: &'a [String],
    pub start_ms: i64,
    pub end_ms: i64,
}

/// Time series per metric and resource when splitting by dimensions (the APIs' `top`, 10 by default).
pub const TOP: usize = 50;

/// Whether a region is global: no regional metrics endpoint serves its resources.
pub fn is_global(region: &str) -> bool {
    region.is_empty() || region.eq_ignore_ascii_case("global")
}

/// An ISO 8601 time grain (`PT1M`, `PT6H`, `P1D`) in milliseconds.
pub fn grain_ms(grain: &str) -> Option<i64> {
    let (rest, unit_ms) = if let Some(r) = grain.strip_prefix("PT") {
        match r.chars().last()? {
            'M' => (&r[..r.len() - 1], 60_000),
            'H' => (&r[..r.len() - 1], 3_600_000),
            'S' => (&r[..r.len() - 1], 1_000),
            _ => return None,
        }
    } else {
        (grain.strip_prefix('P')?.strip_suffix('D')?, 86_400_000)
    };
    rest.parse::<i64>().ok().filter(|n| *n > 0).map(|n| n * unit_ms)
}

/// Milliseconds as an ISO 8601 time grain.
pub fn iso_grain(ms: i64) -> String {
    match ms {
        m if m % 86_400_000 == 0 => format!("P{}D", m / 86_400_000),
        m if m % 3_600_000 == 0 => format!("PT{}H", m / 3_600_000),
        m => format!("PT{}M", (m / 60_000).max(1)),
    }
}

/// A metrics API filter splitting by every one of `dimensions` (`ApiName eq '*' and ResponseType eq '*'`).
fn split_filter(dimensions: &[String]) -> Option<String> {
    (!dimensions.is_empty()).then(|| dimensions.iter().map(|d| format!("{d} eq '*'")).collect::<Vec<_>>().join(" and "))
}

/// The series of one resource's metrics response (`value`: metrics, each with `timeseries`).
fn parse_series(resource_id: &str, metrics: &[Value], aggregations: &[&str], out: &mut Vec<Series>) {
    for metric in metrics {
        let name = metric["name"]["value"].as_str().unwrap_or_default();
        for ts in metric["timeseries"].as_array().map(Vec::as_slice).unwrap_or_default() {
            let dimensions: Vec<(String, String)> = ts["metadatavalues"]
                .as_array()
                .map(Vec::as_slice)
                .unwrap_or_default()
                .iter()
                .filter_map(|m| Some((m["name"]["value"].as_str()?.to_string(), m["value"].as_str().unwrap_or_default().to_string())))
                .collect();
            let data = ts["data"].as_array().map(Vec::as_slice).unwrap_or_default();
            for aggregation in aggregations {
                let points: Vec<(i64, f64)> = data.iter().filter_map(|d| Some((chrono::DateTime::parse_from_rfc3339(d["timeStamp"].as_str()?).ok()?.timestamp_millis(), d[*aggregation].as_f64()?))).collect();
                if !points.is_empty() {
                    out.push(Series { resource_id: resource_id.to_string(), metric: name.to_string(), aggregation: aggregation.to_string(), dimensions: dimensions.clone(), points });
                }
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum AzureError {
    /// The app registration can't sign in (tenant, client id, secret).
    Auth(String),
    /// A missing role assignment; `action` when Azure names it (`Microsoft.Insights/metrics/read`).
    Forbidden { action: Option<String>, message: String },
    Throttled(String),
    /// A request Azure Monitor refuses for this resource type or these metrics (HTTP 400): a namespace
    /// without platform metrics, a time grain the metrics don't have.
    Rejected(String),
    Other(String),
}

#[async_trait::async_trait]
pub trait AzureApi: Send + Sync {
    /// The ids of the enabled subscriptions the app registration can read.
    async fn subscriptions(&self, c: &Credentials) -> Result<Vec<String>, AzureError>;
    async fn resources(&self, c: &Credentials, subscription: &str) -> Result<Vec<Resource>, AzureError>;
    /// The metric definitions of a resource (one per resource type is enough).
    async fn metric_definitions(&self, c: &Credentials, resource_id: &str) -> Result<Vec<MetricDef>, AzureError>;
    async fn metrics(&self, c: &Credentials, q: &MetricsQuery<'_>) -> Result<Vec<Series>, AzureError>;
    /// Resource Health's availability state of the subscription's resources (`Available`, `Degraded`,
    /// `Unavailable`, `Unknown`), by lowercase resource id.
    async fn availability(&self, c: &Credentials, subscription: &str) -> Result<HashMap<String, String>, AzureError>;
}

/// Azure's public cloud endpoints.
pub struct Endpoints {
    pub login: String,
    pub management: String,
    /// `{region}` is replaced by the resources' region.
    pub metrics: String,
    /// Blob Storage; `{account}` is replaced by the storage account.
    pub blob: String,
}

impl Default for Endpoints {
    fn default() -> Self {
        Endpoints { login: "https://login.microsoftonline.com".into(), management: "https://management.azure.com".into(), metrics: "https://{region}.metrics.monitor.azure.com".into(), blob: "https://{account}.blob.core.windows.net".into() }
    }
}

const MANAGEMENT_SCOPE: &str = "https://management.azure.com/.default";
const METRICS_SCOPE: &str = "https://metrics.monitor.azure.com/.default";
const STORAGE_SCOPE: &str = "https://storage.azure.com/.default";
/// The Blob Storage REST version the calls ask for.
const BLOB_VERSION: &str = "2021-08-06";
/// What reading a cost export's blobs needs (the Storage Blob Data Reader role).
pub const BLOB_READ: &str = "Microsoft.Storage/storageAccounts/blobServices/containers/blobs/read";

/// An access token and when it stops being reused.
type Token = (String, Instant);

pub struct Rest {
    http: reqwest::Client,
    endpoints: Endpoints,
    tokens: Mutex<HashMap<(Credentials, &'static str), Token>>,
}

/// Classifies an Azure error response.
pub fn classify(status: u16, body: &str) -> AzureError {
    let v: Value = serde_json::from_str(body).unwrap_or_default();
    let message = v["error"]["message"].as_str().or(v["error_description"].as_str()).or(v["error"].as_str()).or(v["message"].as_str()).unwrap_or(body).to_string();
    match status {
        401 => AzureError::Auth(message),
        400 if v["error"].is_string() => AzureError::Auth(message),
        400 => AzureError::Rejected(message),
        403 => {
            let action = message.split("perform action '").nth(1).and_then(|s| s.split('\'').next()).map(str::to_string);
            AzureError::Forbidden { action, message }
        }
        429 | 503 | 529 => AzureError::Throttled(message),
        _ => AzureError::Other(format!("HTTP {status}: {message}")),
    }
}

/// A Blob Storage error (XML: `<Error><Code>…</Code><Message>…</Message></Error>`).
fn blob_error(status: u16, body: &str) -> AzureError {
    let tag = |t: &str| body.split(&format!("<{t}>")).nth(1).and_then(|s| s.split(&format!("</{t}>")).next()).unwrap_or_default().to_string();
    let message = [tag("Code"), tag("Message")].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(": ");
    match status {
        401 => AzureError::Auth(message),
        403 => AzureError::Forbidden { action: Some(BLOB_READ.into()), message },
        429 | 503 => AzureError::Throttled(message),
        _ => AzureError::Other(format!("HTTP {status}: {message}")),
    }
}

fn unescape_xml(s: &str) -> String {
    s.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'").replace("&amp;", "&")
}

fn iso(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms).unwrap_or_default().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

impl Rest {
    pub fn new(http: reqwest::Client, endpoints: Endpoints) -> Self {
        Rest { http, endpoints, tokens: Mutex::default() }
    }

    async fn token(&self, c: &Credentials, scope: &'static str) -> Result<String, AzureError> {
        let key = (c.clone(), scope);
        if let Some((t, until)) = self.tokens.lock().await.get(&key) {
            if Instant::now() < *until {
                return Ok(t.clone());
            }
        }
        let url = format!("{}/{}/oauth2/v2.0/token", self.endpoints.login, c.tenant_id);
        let form = [("grant_type", "client_credentials"), ("client_id", c.client_id.as_str()), ("client_secret", c.client_secret.as_str()), ("scope", scope)];
        let res = self.http.post(url).form(&form).send().await.map_err(|e| AzureError::Other(e.to_string()))?;
        let status = res.status().as_u16();
        let body = res.text().await.unwrap_or_default();
        if status != 200 {
            return Err(match classify(status, &body) {
                AzureError::Other(m) | AzureError::Rejected(m) => AzureError::Auth(m),
                e => e,
            });
        }
        let v: Value = serde_json::from_str(&body).map_err(|e| AzureError::Other(e.to_string()))?;
        let token = v["access_token"].as_str().ok_or_else(|| AzureError::Auth("no access token".into()))?.to_string();
        let ttl = v["expires_in"].as_u64().unwrap_or(3600).saturating_sub(300);
        self.tokens.lock().await.insert(key, (token.clone(), Instant::now() + Duration::from_secs(ttl)));
        Ok(token)
    }

    async fn send(&self, req: reqwest::RequestBuilder) -> Result<Value, AzureError> {
        let res = req.send().await.map_err(|e| AzureError::Other(e.to_string()))?;
        let status = res.status().as_u16();
        let body = res.text().await.unwrap_or_default();
        if !(200..300).contains(&status) {
            return Err(classify(status, &body));
        }
        serde_json::from_str(&body).map_err(|e| AzureError::Other(e.to_string()))
    }

    fn blob_url(&self, account: &str, container: &str, name: Option<&str>) -> Result<url::Url, AzureError> {
        let mut url = url::Url::parse(&self.endpoints.blob.replace("{account}", account)).map_err(|e| AzureError::Other(format!("storage account {account}: {e}")))?;
        {
            let mut segments = url.path_segments_mut().map_err(|_| AzureError::Other("blob endpoint".into()))?;
            segments.pop_if_empty().push(container);
            for part in name.unwrap_or_default().split('/').filter(|p| !p.is_empty()) {
                segments.push(part);
            }
        }
        Ok(url)
    }

    async fn blob_request(&self, c: &Credentials, url: url::Url) -> Result<reqwest::Response, AzureError> {
        let token = self.token(c, STORAGE_SCOPE).await?;
        let res = self.http.get(url).bearer_auth(token).header("x-ms-version", BLOB_VERSION).send().await.map_err(|e| AzureError::Other(e.to_string()))?;
        let status = res.status().as_u16();
        if (200..300).contains(&status) {
            return Ok(res);
        }
        Err(blob_error(status, &res.text().await.unwrap_or_default()))
    }

    /// The blobs under `prefix` in a container, with their last modification (epoch ms).
    pub async fn list_blobs(&self, c: &Credentials, account: &str, container: &str, prefix: &str) -> Result<Vec<(String, i64)>, AzureError> {
        let (mut out, mut marker) = (vec![], String::new());
        loop {
            let mut url = self.blob_url(account, container, None)?;
            url.query_pairs_mut().append_pair("restype", "container").append_pair("comp", "list").append_pair("prefix", prefix);
            if !marker.is_empty() {
                url.query_pairs_mut().append_pair("marker", &marker);
            }
            let body = self.blob_request(c, url).await?.text().await.map_err(|e| AzureError::Other(e.to_string()))?;
            let tag = |s: &str, t: &str| s.split(&format!("<{t}>")).nth(1).and_then(|s| s.split(&format!("</{t}>")).next()).map(str::to_string);
            for blob in body.split("<Blob>").skip(1) {
                let Some(name) = tag(blob, "Name") else { continue };
                let modified = tag(blob, "Last-Modified").and_then(|m| chrono::DateTime::parse_from_rfc2822(&m).ok()).map_or(0, |d| d.timestamp_millis());
                out.push((unescape_xml(&name), modified));
            }
            match tag(&body, "NextMarker").filter(|m| !m.is_empty()) {
                Some(next) => marker = next,
                None => return Ok(out),
            }
        }
    }

    pub async fn get_blob(&self, c: &Credentials, account: &str, container: &str, name: &str) -> Result<Vec<u8>, AzureError> {
        let res = self.blob_request(c, self.blob_url(account, container, Some(name))?).await?;
        Ok(res.bytes().await.map_err(|e| AzureError::Other(e.to_string()))?.to_vec())
    }

    /// Every page of a Resource Manager list (`value`, `nextLink`).
    async fn pages(&self, c: &Credentials, first: String) -> Result<Vec<Value>, AzureError> {
        let token = self.token(c, MANAGEMENT_SCOPE).await?;
        let (mut out, mut next) = (vec![], Some(first));
        while let Some(url) = next.take() {
            let page = self.send(self.http.get(url).bearer_auth(&token)).await?;
            out.extend(page["value"].as_array().cloned().unwrap_or_default());
            next = page["nextLink"].as_str().map(str::to_string);
        }
        Ok(out)
    }
}

fn resource_group(id: &str) -> String {
    let parts: Vec<&str> = id.split('/').collect();
    parts.iter().position(|p| p.eq_ignore_ascii_case("resourceGroups")).and_then(|i| parts.get(i + 1)).map(|g| g.to_ascii_lowercase()).unwrap_or_default()
}

#[async_trait::async_trait]
impl AzureApi for Rest {
    async fn subscriptions(&self, c: &Credentials) -> Result<Vec<String>, AzureError> {
        let all = self.pages(c, format!("{}/subscriptions?api-version=2022-12-01", self.endpoints.management)).await?;
        Ok(all.iter().filter(|s| s["state"].as_str().is_none_or(|st| st == "Enabled")).filter_map(|s| s["subscriptionId"].as_str().map(str::to_string)).collect())
    }

    async fn resources(&self, c: &Credentials, subscription: &str) -> Result<Vec<Resource>, AzureError> {
        let all = self.pages(c, format!("{}/subscriptions/{subscription}/resources?api-version=2021-04-01", self.endpoints.management)).await?;
        Ok(all
            .iter()
            .filter_map(|r| {
                let id = r["id"].as_str()?.to_string();
                Some(Resource {
                    resource_group: resource_group(&id),
                    name: r["name"].as_str().unwrap_or_default().to_string(),
                    kind: r["type"].as_str()?.to_string(),
                    location: r["location"].as_str().unwrap_or_default().to_string(),
                    tags: r["tags"].as_object().map(|t| t.iter().map(|(k, v)| format!("{}:{}", k.to_ascii_lowercase(), v.as_str().unwrap_or_default())).collect()).unwrap_or_default(),
                    id,
                })
            })
            .collect())
    }

    async fn metric_definitions(&self, c: &Credentials, resource_id: &str) -> Result<Vec<MetricDef>, AzureError> {
        let all = self.pages(c, format!("{}{resource_id}/providers/Microsoft.Insights/metricDefinitions?api-version=2018-01-01", self.endpoints.management)).await?;
        let strings = |v: &Value, key: &str| -> Vec<String> { v.as_array().map(Vec::as_slice).unwrap_or_default().iter().filter_map(|x| if key.is_empty() { x.as_str() } else { x[key].as_str() }.map(str::to_string)).collect() };
        Ok(all
            .iter()
            .filter_map(|d| {
                let grain_ms = d["metricAvailabilities"].as_array().map(Vec::as_slice).unwrap_or_default().iter().filter_map(|g| grain_ms(g["timeGrain"].as_str()?)).min().unwrap_or(60_000);
                Some(MetricDef {
                    name: d["name"]["value"].as_str()?.to_string(),
                    primary: d["primaryAggregationType"].as_str().unwrap_or("Average").to_string(),
                    aggregations: strings(&d["supportedAggregationTypes"], ""),
                    grain_ms,
                    dimensions: strings(&d["dimensions"], "value"),
                })
            })
            .collect())
    }

    async fn metrics(&self, c: &Credentials, q: &MetricsQuery<'_>) -> Result<Vec<Series>, AzureError> {
        let names = q.metrics.iter().map(|m| m.replace(',', "%2")).collect::<Vec<_>>().join(",");
        let filter = split_filter(q.dimensions);
        let mut common = vec![("interval", iso_grain(q.grain_ms)), ("metricnamespace", q.resource_type.to_string()), ("metricnames", names), ("aggregation", q.aggregations.join(","))];
        if filter.is_some() {
            common.push(("top", TOP.to_string()));
        }
        let mut out = vec![];
        if is_global(q.region) {
            // Resource Manager's per-resource API (`$filter`, `timespan`), one resource a call.
            let token = self.token(c, MANAGEMENT_SCOPE).await?;
            for id in q.resource_ids {
                let mut query = common.clone();
                query.extend([("timespan", format!("{}/{}", iso(q.start_ms), iso(q.end_ms))), ("api-version", "2023-10-01".into())]);
                if let Some(f) = &filter {
                    query.push(("$filter", f.clone()));
                }
                let url = format!("{}{id}/providers/Microsoft.Insights/metrics", self.endpoints.management);
                let v = self.send(self.http.get(url).query(&query).bearer_auth(&token)).await?;
                parse_series(id, v["value"].as_array().map(Vec::as_slice).unwrap_or_default(), q.aggregations, &mut out);
            }
            return Ok(out);
        }
        let token = self.token(c, METRICS_SCOPE).await?;
        let url = format!("{}/subscriptions/{}/metrics:getBatch", self.endpoints.metrics.replace("{region}", q.region), q.subscription);
        let mut query = common;
        query.extend([("starttime", iso(q.start_ms)), ("endtime", iso(q.end_ms)), ("api-version", "2024-02-01".into())]);
        if let Some(f) = filter {
            query.push(("filter", f));
        }
        let body = serde_json::json!({ "resourceids": q.resource_ids });
        let v = self.send(self.http.post(url).query(&query).bearer_auth(token).json(&body)).await?;
        for resource in v["values"].as_array().map(Vec::as_slice).unwrap_or_default() {
            parse_series(resource["resourceid"].as_str().unwrap_or_default(), resource["value"].as_array().map(Vec::as_slice).unwrap_or_default(), q.aggregations, &mut out);
        }
        Ok(out)
    }

    async fn availability(&self, c: &Credentials, subscription: &str) -> Result<HashMap<String, String>, AzureError> {
        let all = self.pages(c, format!("{}/subscriptions/{subscription}/providers/Microsoft.ResourceHealth/availabilityStatuses?api-version=2022-10-01", self.endpoints.management)).await?;
        Ok(all
            .iter()
            .filter_map(|s| {
                let id = s["id"].as_str()?.to_ascii_lowercase();
                let resource = id.split("/providers/microsoft.resourcehealth/").next()?.to_string();
                Some((resource, s["properties"]["availabilityState"].as_str()?.to_string()))
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::extract::{Query, State};
    use axum::http::HeaderMap;
    use axum::routing::{get, post};
    use axum::{Form, Json, Router};
    use serde_json::json;
    use std::sync::{Arc, Mutex as StdMutex};

    #[test]
    fn errors_are_classified() {
        assert!(matches!(classify(401, r#"{"error":"invalid_client","error_description":"AADSTS7000215: Invalid client secret provided."}"#), AzureError::Auth(m) if m.contains("AADSTS7000215")));
        assert_eq!(
            classify(403, r#"{"error":{"code":"AuthorizationFailed","message":"The client 'c' with object id 'o' does not have authorization to perform action 'Microsoft.Insights/metrics/read' over scope '/subscriptions/s'"}}"#),
            AzureError::Forbidden { action: Some("Microsoft.Insights/metrics/read".into()), message: "The client 'c' with object id 'o' does not have authorization to perform action 'Microsoft.Insights/metrics/read' over scope '/subscriptions/s'".into() }
        );
        assert!(matches!(classify(429, "{}"), AzureError::Throttled(_)));
        // The metrics data plane throttles with 529; 503 is a busy backend.
        assert!(matches!(classify(529, "{}"), AzureError::Throttled(_)));
        assert!(matches!(classify(503, "{}"), AzureError::Throttled(_)));
        assert_eq!(grain_ms("PT1M"), Some(60_000));
        assert_eq!(grain_ms("PT30M"), Some(1_800_000));
        assert_eq!(grain_ms("PT12H"), Some(43_200_000));
        assert_eq!(grain_ms("P1D"), Some(86_400_000));
        assert_eq!(grain_ms("bogus"), None);
        assert_eq!(iso_grain(300_000), "PT5M");
        assert_eq!(iso_grain(3_600_000), "PT1H");
        assert_eq!(iso_grain(86_400_000), "P1D");
        assert!(matches!(classify(400, r#"{"code":"BadRequest","message":"Microsoft.Compute/restorePointCollections is not a supported platform metric namespace"}"#), AzureError::Rejected(m) if m.contains("restorePointCollections")));
        assert_eq!(resource_group("/subscriptions/s/resourceGroups/Storefront-RG/providers/Microsoft.Compute/virtualMachines/web-1"), "storefront-rg");
    }

    /// A stand-in for Azure: login, Resource Manager (paged) and the metrics batch API.
    async fn fake_azure() -> (String, Arc<StdMutex<Vec<String>>>) {
        let seen: Arc<StdMutex<Vec<String>>> = Arc::default();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let next_link = format!("{base}/subscriptions?api-version=2022-12-01&page2=1");
        let bearer = |h: &HeaderMap| h.get("authorization").and_then(|v| v.to_str().ok()).unwrap_or_default().to_string();
        let app = Router::new()
            .route(
                "/tenant-1/oauth2/v2.0/token",
                post(|State(seen): State<Arc<StdMutex<Vec<String>>>>, Form(f): Form<HashMap<String, String>>| async move {
                    seen.lock().unwrap().push(format!("token {}", f["scope"]));
                    if f["client_secret"] != "s3cret" {
                        return (axum::http::StatusCode::UNAUTHORIZED, Json(json!({ "error": "invalid_client", "error_description": "AADSTS7000215: Invalid client secret provided." })));
                    }
                    (axum::http::StatusCode::OK, Json(json!({ "access_token": format!("tok-{}", f["scope"]), "expires_in": 3599 })))
                }),
            )
            .route(
                "/subscriptions",
                get(move |h: HeaderMap, Query(q): Query<HashMap<String, String>>| async move {
                    assert_eq!(bearer(&h), "Bearer tok-https://management.azure.com/.default");
                    if q.contains_key("page2") {
                        return Json(json!({ "value": [{ "subscriptionId": "sub-b", "state": "Disabled" }] }));
                    }
                    Json(json!({ "value": [{ "subscriptionId": "sub-a", "state": "Enabled" }], "nextLink": next_link }))
                }),
            )
            .route(
                "/subscriptions/sub-a/resources",
                get(|| async {
                    Json(json!({ "value": [{ "id": "/subscriptions/sub-a/resourceGroups/Storefront/providers/Microsoft.Compute/virtualMachines/web-1", "name": "web-1", "type": "Microsoft.Compute/virtualMachines", "location": "westeurope", "tags": { "Env": "prod" } }] }))
                }),
            )
            .route(
                "/subscriptions/sub-a/resourceGroups/Storefront/providers/Microsoft.Compute/virtualMachines/web-1/providers/Microsoft.Insights/metricDefinitions",
                get(|| async {
                    Json(json!({ "value": [
                        { "name": { "value": "Percentage CPU" }, "primaryAggregationType": "Average", "supportedAggregationTypes": ["None", "Average", "Minimum", "Maximum", "Total", "Count"], "metricAvailabilities": [{ "timeGrain": "PT1H" }, { "timeGrain": "PT1M" }] },
                        { "name": { "value": "Network In Total" }, "primaryAggregationType": "Total" },
                        { "name": { "value": "Disk Read Bytes/sec" }, "primaryAggregationType": "Average", "dimensions": [{ "value": "LUN", "localizedValue": "LUN" }], "metricAvailabilities": [{ "timeGrain": "PT1M" }] },
                        { "name": { "value": "StorageUsed" }, "primaryAggregationType": "Average", "metricAvailabilities": [{ "timeGrain": "PT1H" }, { "timeGrain": "P1D" }] }
                    ] }))
                }),
            )
            .route(
                "/subscriptions/sub-a/metrics:getBatch",
                post(move |h: HeaderMap, Query(q): Query<HashMap<String, String>>, Json(body): Json<Value>| async move {
                    assert_eq!(bearer(&h), "Bearer tok-https://metrics.monitor.azure.com/.default");
                    assert_eq!(q["metricnamespace"], "Microsoft.Storage/storageAccounts");
                    assert_eq!(q["aggregation"], "total,maximum");
                    assert_eq!(q["interval"], "PT1H");
                    assert_eq!((q["filter"].as_str(), q["top"].as_str()), ("ApiName eq '*' and ResponseType eq '*'", "50"));
                    Json(json!({ "values": [{ "resourceid": body["resourceids"][0], "value": [{ "name": { "value": "Transactions" }, "timeseries": [
                        { "metadatavalues": [{ "name": { "value": "apiname" }, "value": "GetBlob" }, { "name": { "value": "responsetype" }, "value": "Success" }], "data": [
                            { "timeStamp": "2023-11-14T22:00:00Z", "total": 12.0, "maximum": 4.0 },
                            { "timeStamp": "2023-11-14T23:00:00Z", "total": 13.0 }
                        ] }
                    ] }] }] }))
                }),
            )
            .route(
                "/subscriptions/sub-a/resourceGroups/dns/providers/Microsoft.Network/dnszones/example.com/providers/Microsoft.Insights/metrics",
                get(move |h: HeaderMap, Query(q): Query<HashMap<String, String>>| async move {
                    // A global resource: Azure Resource Manager's per-resource metrics API.
                    assert_eq!(bearer(&h), "Bearer tok-https://management.azure.com/.default");
                    assert_eq!((q["metricnamespace"].as_str(), q["metricnames"].as_str(), q["aggregation"].as_str(), q["interval"].as_str()), ("Microsoft.Network/dnszones", "QueryVolume", "total", "PT5M"));
                    assert_eq!(q["timespan"], "2023-11-14T22:10:00Z/2023-11-14T22:20:00Z");
                    assert!(!q.contains_key("$filter"));
                    Json(json!({ "value": [{ "name": { "value": "QueryVolume" }, "timeseries": [{ "data": [{ "timeStamp": "2023-11-14T22:10:00Z", "total": 7.0 }, { "timeStamp": "2023-11-14T22:15:00Z", "total": 9.0 }] }] }] }))
                }),
            )
            .route(
                "/subscriptions/sub-a/providers/Microsoft.ResourceHealth/availabilityStatuses",
                get(|| async {
                    Json(json!({ "value": [
                        { "id": "/subscriptions/sub-a/resourceGroups/Storefront/providers/Microsoft.Compute/virtualMachines/web-1/providers/Microsoft.ResourceHealth/availabilityStatuses/current", "properties": { "availabilityState": "Available" } },
                        { "id": "/subscriptions/sub-a/resourceGroups/Storefront/providers/Microsoft.Web/sites/api/providers/Microsoft.ResourceHealth/availabilityStatuses/current", "properties": { "availabilityState": "Degraded" } }
                    ] }))
                }),
            )
            .route(
                "/examplecostexports/costs",
                get(move |h: HeaderMap, Query(q): Query<HashMap<String, String>>| async move {
                    assert_eq!(bearer(&h), "Bearer tok-https://storage.azure.com/.default");
                    assert!(h.contains_key("x-ms-version"));
                    assert_eq!((q["restype"].as_str(), q["comp"].as_str(), q["prefix"].as_str()), ("container", "list", "cost-exports/observecraft-actual/"));
                    let blob = |name: &str, modified: &str| format!("<Blob><Name>{name}</Name><Properties><Last-Modified>{modified}</Last-Modified></Properties></Blob>");
                    if q.get("marker").map(String::as_str) == Some("page-2") {
                        return format!("<?xml version=\"1.0\" encoding=\"utf-8\"?><EnumerationResults><Blobs>{}</Blobs><NextMarker /></EnumerationResults>", blob("cost-exports/observecraft-actual/20261001-20261031/run-2/manifest.json", "Mon, 02 Nov 2026 10:00:00 GMT"));
                    }
                    format!("<?xml version=\"1.0\" encoding=\"utf-8\"?><EnumerationResults><Blobs>{}</Blobs><NextMarker>page-2</NextMarker></EnumerationResults>", blob("cost-exports/observecraft-actual/a&amp;b.csv", "Sun, 01 Nov 2026 10:00:00 GMT"))
                }),
            )
            .route("/examplecostexports/costs/{*name}", get(|axum::extract::Path(name): axum::extract::Path<String>| async move { format!("bytes of {name}") }))
            .route(
                "/deniedaccount/costs",
                get(|| async { (axum::http::StatusCode::FORBIDDEN, "<?xml version=\"1.0\" encoding=\"utf-8\"?><Error><Code>AuthorizationPermissionMismatch</Code><Message>This request is not authorized to perform this operation using this permission.</Message></Error>") }),
            )
            .with_state(seen.clone());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (base, seen)
    }

    #[tokio::test]
    async fn blobs_are_listed_page_by_page_and_downloaded() {
        let (base, _) = fake_azure().await;
        let rest = Rest::new(reqwest::Client::new(), Endpoints { login: base.clone(), management: base.clone(), metrics: base.clone(), blob: format!("{base}/{{account}}") });
        let c = Credentials { tenant_id: "tenant-1".into(), client_id: "client-1".into(), client_secret: "s3cret".into() };
        let blobs = rest.list_blobs(&c, "examplecostexports", "costs", "cost-exports/observecraft-actual/").await.unwrap();
        assert_eq!(
            blobs,
            [("cost-exports/observecraft-actual/a&b.csv".to_string(), 1_793_527_200_000), ("cost-exports/observecraft-actual/20261001-20261031/run-2/manifest.json".to_string(), 1_793_613_600_000)]
        );
        assert_eq!(rest.get_blob(&c, "examplecostexports", "costs", "cost-exports/x y.json").await.unwrap(), b"bytes of cost-exports/x y.json");
        let denied = rest.list_blobs(&c, "deniedaccount", "costs", "p/").await.unwrap_err();
        assert!(matches!(&denied, AzureError::Forbidden { action: Some(a), message } if a == BLOB_READ && message.contains("not authorized")), "{denied:?}");
    }

    #[tokio::test]
    async fn the_rest_client_signs_in_lists_and_reads_batches() {
        let (base, seen) = fake_azure().await;
        let rest = Rest::new(reqwest::Client::new(), Endpoints { login: base.clone(), management: base.clone(), metrics: base.clone(), blob: base.clone() });
        let c = Credentials { tenant_id: "tenant-1".into(), client_id: "client-1".into(), client_secret: "s3cret".into() };
        let resources = rest.resources(&c, "sub-a").await.unwrap();
        assert_eq!(resources.len(), 1);
        assert_eq!((resources[0].kind.as_str(), resources[0].resource_group.as_str(), resources[0].tags.clone()), ("Microsoft.Compute/virtualMachines", "storefront", vec!["env:prod".to_string()]));
        // Each metric's primary and supported aggregations, finest grain and dimensions.
        let defs = rest.metric_definitions(&c, &resources[0].id).await.unwrap();
        let s = |a: &[&str]| a.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        assert_eq!(defs[0], MetricDef { name: "Percentage CPU".into(), primary: "Average".into(), aggregations: s(&["None", "Average", "Minimum", "Maximum", "Total", "Count"]), grain_ms: 60_000, dimensions: vec![] });
        assert_eq!((defs[1].grain_ms, defs[1].primary.as_str()), (60_000, "Total"));
        assert_eq!(defs[2].dimensions, ["LUN"]);
        assert_eq!(defs[3].grain_ms, 3_600_000);
        let storage = "/subscriptions/sub-a/resourceGroups/Storefront/providers/Microsoft.Storage/storageAccounts/assets".to_string();
        let q = MetricsQuery {
            subscription: "sub-a",
            region: "westeurope",
            resource_type: "Microsoft.Storage/storageAccounts",
            resource_ids: std::slice::from_ref(&storage),
            metrics: &["Transactions".into()],
            aggregations: &["total", "maximum"],
            grain_ms: 3_600_000,
            dimensions: &["ApiName".into(), "ResponseType".into()],
            start_ms: 1_699_999_200_000,
            end_ms: 1_700_006_400_000,
        };
        let series = rest.metrics(&c, &q).await.unwrap();
        assert_eq!(series.len(), 2);
        assert_eq!(series[0].points, [(1_699_999_200_000, 12.0), (1_700_002_800_000, 13.0)]);
        assert_eq!(series[0].dimensions, [("apiname".to_string(), "GetBlob".to_string()), ("responsetype".to_string(), "Success".to_string())]);
        assert_eq!((series[1].aggregation.as_str(), series[1].points.len()), ("maximum", 1));
        // A global resource (DNS zones, Front Door, Traffic Manager) is read through Azure Resource Manager, one resource a call.
        let zone = "/subscriptions/sub-a/resourceGroups/dns/providers/Microsoft.Network/dnszones/example.com".to_string();
        let q = MetricsQuery { region: "global", resource_type: "Microsoft.Network/dnszones", resource_ids: std::slice::from_ref(&zone), metrics: &["QueryVolume".into()], aggregations: &["total"], grain_ms: 300_000, dimensions: &[], start_ms: 1_700_000_000_000 - 200_000, end_ms: 1_700_000_400_000, ..q };
        let series = rest.metrics(&c, &q).await.unwrap();
        assert_eq!((series.len(), series[0].resource_id.as_str(), series[0].points.len()), (1, zone.as_str(), 2));
        // Resource Health: each resource's availability, by lowercase id.
        let health = rest.availability(&c, "sub-a").await.unwrap();
        assert_eq!(health.get(&resources[0].id.to_ascii_lowercase()).map(String::as_str), Some("Available"));
        assert_eq!(health.len(), 2);
        // One token per scope, cached.
        rest.resources(&c, "sub-a").await.unwrap();
        let tokens: Vec<String> = seen.lock().unwrap().iter().filter(|s| s.starts_with("token")).cloned().collect();
        assert_eq!(tokens, ["token https://management.azure.com/.default", "token https://metrics.monitor.azure.com/.default"]);
        // A wrong secret is an authentication issue.
        let bad = Credentials { client_secret: "wrong".into(), ..c.clone() };
        assert!(matches!(rest.subscriptions(&bad).await, Err(AzureError::Auth(m)) if m.contains("AADSTS7000215")));
    }

    #[tokio::test]
    async fn subscriptions_follow_next_links_and_skip_disabled_ones() {
        let (base, _) = fake_azure().await;
        let rest = Rest::new(reqwest::Client::new(), Endpoints { login: base.clone(), management: base.clone(), metrics: base.clone(), blob: base });
        let c = Credentials { tenant_id: "tenant-1".into(), client_id: "client-1".into(), client_secret: "s3cret".into() };
        assert_eq!(rest.subscriptions(&c).await.unwrap(), ["sub-a"]);
    }
}
