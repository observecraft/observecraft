//! The Azure APIs the crawler calls, behind [`AzureApi`] (tests use a fake), and their implementation
//! over REST ([`Rest`]): an app registration's token (OAuth client credentials, cached until it
//! expires), the subscriptions it can read, their resources and metric definitions (Azure Resource
//! Manager), and metric values from the Azure Monitor batch API (`metrics:getBatch`: up to 50
//! resources of one type and region and 20 metrics per call).

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

/// One metric and aggregation of one resource.
#[derive(Clone, Debug, PartialEq)]
pub struct Series {
    pub resource_id: String,
    pub metric: String,
    pub aggregation: String,
    pub points: Vec<(i64, f64)>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum AzureError {
    /// The app registration can't sign in (tenant, client id, secret).
    Auth(String),
    /// A missing role assignment; `action` when Azure names it (`Microsoft.Insights/metrics/read`).
    Forbidden { action: Option<String>, message: String },
    Throttled(String),
    Other(String),
}

#[axum::async_trait]
pub trait AzureApi: Send + Sync {
    /// The ids of the enabled subscriptions the app registration can read.
    async fn subscriptions(&self, c: &Credentials) -> Result<Vec<String>, AzureError>;
    async fn resources(&self, c: &Credentials, subscription: &str) -> Result<Vec<Resource>, AzureError>;
    /// The metric names of a resource (one per resource type is enough).
    async fn metric_names(&self, c: &Credentials, resource_id: &str) -> Result<Vec<String>, AzureError>;
    /// `aggregations` of `metrics` for `resource_ids` (one type and region; at most 50 resources and 20 metrics).
    #[allow(clippy::too_many_arguments)]
    async fn metrics(&self, c: &Credentials, subscription: &str, region: &str, resource_type: &str, resource_ids: &[String], metrics: &[String], aggregations: &[&str], start_ms: i64, end_ms: i64) -> Result<Vec<Series>, AzureError>;
}

/// Azure's public cloud endpoints.
pub struct Endpoints {
    pub login: String,
    pub management: String,
    /// `{region}` is replaced by the resources' region.
    pub metrics: String,
}

impl Default for Endpoints {
    fn default() -> Self {
        Endpoints { login: "https://login.microsoftonline.com".into(), management: "https://management.azure.com".into(), metrics: "https://{region}.metrics.monitor.azure.com".into() }
    }
}

const MANAGEMENT_SCOPE: &str = "https://management.azure.com/.default";
const METRICS_SCOPE: &str = "https://metrics.monitor.azure.com/.default";

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
    let message = v["error"]["message"].as_str().or(v["error_description"].as_str()).or(v["error"].as_str()).unwrap_or(body).to_string();
    match status {
        401 => AzureError::Auth(message),
        400 if v["error"].is_string() => AzureError::Auth(message),
        403 => {
            let action = message.split("perform action '").nth(1).and_then(|s| s.split('\'').next()).map(str::to_string);
            AzureError::Forbidden { action, message }
        }
        429 => AzureError::Throttled(message),
        _ => AzureError::Other(format!("HTTP {status}: {message}")),
    }
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
                AzureError::Other(m) => AzureError::Auth(m),
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

#[axum::async_trait]
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

    async fn metric_names(&self, c: &Credentials, resource_id: &str) -> Result<Vec<String>, AzureError> {
        let all = self.pages(c, format!("{}{resource_id}/providers/Microsoft.Insights/metricDefinitions?api-version=2018-01-01", self.endpoints.management)).await?;
        Ok(all.iter().filter_map(|d| d["name"]["value"].as_str().map(str::to_string)).collect())
    }

    async fn metrics(&self, c: &Credentials, subscription: &str, region: &str, resource_type: &str, resource_ids: &[String], metrics: &[String], aggregations: &[&str], start_ms: i64, end_ms: i64) -> Result<Vec<Series>, AzureError> {
        let token = self.token(c, METRICS_SCOPE).await?;
        let url = format!("{}/subscriptions/{subscription}/metrics:getBatch", self.endpoints.metrics.replace("{region}", region));
        let names = metrics.iter().map(|m| m.replace(',', "%2")).collect::<Vec<_>>().join(",");
        let query = [
            ("starttime", iso(start_ms)),
            ("endtime", iso(end_ms)),
            ("interval", "PT1M".into()),
            ("metricnamespace", resource_type.to_string()),
            ("metricnames", names),
            ("aggregation", aggregations.join(",")),
            ("api-version", "2024-02-01".into()),
        ];
        let body = serde_json::json!({ "resourceids": resource_ids });
        let v = self.send(self.http.post(url).query(&query).bearer_auth(token).json(&body)).await?;
        let mut out = vec![];
        for resource in v["values"].as_array().map(Vec::as_slice).unwrap_or_default() {
            let resource_id = resource["resourceid"].as_str().unwrap_or_default();
            for metric in resource["value"].as_array().map(Vec::as_slice).unwrap_or_default() {
                let name = metric["name"]["value"].as_str().unwrap_or_default();
                for aggregation in aggregations {
                    let points: Vec<(i64, f64)> = metric["timeseries"]
                        .as_array()
                        .map(Vec::as_slice)
                        .unwrap_or_default()
                        .iter()
                        .flat_map(|ts| ts["data"].as_array().cloned().unwrap_or_default())
                        .filter_map(|d| Some((chrono::DateTime::parse_from_rfc3339(d["timeStamp"].as_str()?).ok()?.timestamp_millis(), d[*aggregation].as_f64()?)))
                        .collect();
                    if !points.is_empty() {
                        out.push(Series { resource_id: resource_id.to_string(), metric: name.to_string(), aggregation: aggregation.to_string(), points });
                    }
                }
            }
        }
        Ok(out)
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
                get(|| async { Json(json!({ "value": [{ "name": { "value": "Percentage CPU" } }, { "name": { "value": "Network In Total" } }] })) }),
            )
            .route(
                "/subscriptions/sub-a/metrics:getBatch",
                post(move |h: HeaderMap, Query(q): Query<HashMap<String, String>>, Json(body): Json<Value>| async move {
                    assert_eq!(bearer(&h), "Bearer tok-https://metrics.monitor.azure.com/.default");
                    assert_eq!(q["metricnamespace"], "Microsoft.Compute/virtualMachines");
                    assert_eq!(q["aggregation"], "average,maximum");
                    assert_eq!(q["interval"], "PT1M");
                    Json(json!({ "values": [{ "resourceid": body["resourceids"][0], "value": [{ "name": { "value": "Percentage CPU" }, "timeseries": [{ "data": [
                        { "timeStamp": "2023-11-14T22:13:00Z", "average": 12.5, "maximum": 40.0 },
                        { "timeStamp": "2023-11-14T22:14:00Z", "average": 13.0 }
                    ] }] }] }] }))
                }),
            )
            .with_state(seen.clone());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (base, seen)
    }

    #[tokio::test]
    async fn the_rest_client_signs_in_lists_and_reads_batches() {
        let (base, seen) = fake_azure().await;
        let rest = Rest::new(reqwest::Client::new(), Endpoints { login: base.clone(), management: base.clone(), metrics: base.clone() });
        let c = Credentials { tenant_id: "tenant-1".into(), client_id: "client-1".into(), client_secret: "s3cret".into() };
        let resources = rest.resources(&c, "sub-a").await.unwrap();
        assert_eq!(resources.len(), 1);
        assert_eq!((resources[0].kind.as_str(), resources[0].resource_group.as_str(), resources[0].tags.clone()), ("Microsoft.Compute/virtualMachines", "storefront", vec!["env:prod".to_string()]));
        assert_eq!(rest.metric_names(&c, &resources[0].id).await.unwrap(), ["Percentage CPU", "Network In Total"]);
        let series = rest.metrics(&c, "sub-a", "westeurope", "Microsoft.Compute/virtualMachines", &[resources[0].id.clone()], &["Percentage CPU".into()], &["average", "maximum"], 1_700_000_000_000, 1_700_000_600_000).await.unwrap();
        assert_eq!(series.len(), 2);
        assert_eq!(series[0].points, [(1_699_999_980_000, 12.5), (1_700_000_040_000, 13.0)]);
        assert_eq!((series[1].aggregation.as_str(), series[1].points.len()), ("maximum", 1));
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
        let rest = Rest::new(reqwest::Client::new(), Endpoints { login: base.clone(), management: base.clone(), metrics: base });
        let c = Credentials { tenant_id: "tenant-1".into(), client_id: "client-1".into(), client_secret: "s3cret".into() };
        assert_eq!(rest.subscriptions(&c).await.unwrap(), ["sub-a"]);
    }
}
