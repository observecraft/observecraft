//! The monitors of an APM service, for the service page's monitor bar: the org's monitors whose query or tags
//! name the service (`service:order-service`), as the Software Catalog counts them, and the service's resources
//! each one targets (`resource_name:get_/orders`), for the Resources table.

use axum::extract::State;
use axum::Json;
use serde::Serialize;
use utoipa::ToSchema;

use super::apm::{ResourcesRequest, ServiceRequest};
use super::monitor_engine::MonitorStatus;
use super::scorecard_rules::references_service;
use crate::error::ApiResult;
use crate::state::{AppState, OrgId};

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ServiceMonitor {
    pub id: i64,
    pub name: String,
    pub status: MonitorStatus,
    /// The service's resources (in the range) its query or tags name.
    pub resources: Vec<String>,
}

/// A resource name as Datadog tags it: lowercase, characters other than letters, digits and `_-./:` as `_`, one `_`
/// for a run of them and none at the end (`GET /orders/{id}` → `get_/orders/_id`).
fn tag_form(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.to_lowercase().chars() {
        let c = if c.is_alphanumeric() || matches!(c, '_' | '-' | '.' | '/' | ':') { c } else { '_' };
        if !(c == '_' && out.ends_with('_')) {
            out.push(c);
        }
    }
    out.trim_end_matches('_').to_string()
}

/// Values of `resource_name:` (Datadog's tag) and `resource:` (span searches) in a text, in tag form: bare, quoted
/// (also `\"…\"` inside a quoted search) or a parenthesized `OR` list. Negated (`-resource:`) and longer keys
/// (`aws_resource_name:`) aren't.
fn resource_values(text: &str) -> Vec<String> {
    let text = text.to_lowercase();
    let mut out = Vec::new();
    for key in ["resource_name:", "resource:"] {
        let mut from = 0;
        while let Some(found) = text[from..].find(key) {
            let at = from + found;
            from = at + key.len();
            if text[..at].chars().next_back().is_some_and(|c| c.is_alphanumeric() || matches!(c, '_' | '.' | '-')) {
                continue;
            }
            let rest = &text[from..];
            if let Some(list) = rest.strip_prefix('(') {
                let list = &list[..list.find(')').unwrap_or(list.len())];
                out.extend(list.split(|c: char| c.is_whitespace() || c == ',').map(|v| v.trim_matches(|c| c == '"' || c == '\\')).filter(|v| !v.is_empty() && *v != "or").map(tag_form));
            } else if let Some(quoted) = rest.strip_prefix("\\\"").or_else(|| rest.strip_prefix('"')) {
                out.push(tag_form(&quoted[..quoted.find(['"', '\\']).unwrap_or(quoted.len())]));
            } else {
                out.push(tag_form(&rest[..rest.find(|c: char| c.is_whitespace() || matches!(c, ',' | '}' | ')' | '"')).unwrap_or(rest.len())]));
            }
        }
    }
    out
}

/// The resources a monitor's texts (query, tags) target, in the order of `resources`.
fn monitor_resources(texts: &[String], resources: &[String]) -> Vec<String> {
    let named: Vec<String> = texts.iter().flat_map(|t| resource_values(t)).collect();
    resources.iter().filter(|r| named.contains(&tag_form(r))).cloned().collect()
}

/// The monitors naming the service in their query or tags (drafts aside), the worst status first, with the
/// service's resources each targets.
#[utoipa::path(post, path = "/api/v1/apm/service/monitors", request_body = ServiceRequest, responses((status = 200, body = Vec<ServiceMonitor>)), tag = "apm")]
pub async fn service_monitors(State(st): State<AppState>, OrgId(org): OrgId, Json(req): Json<ServiceRequest>) -> ApiResult<Vec<ServiceMonitor>> {
    let resources_req = ResourcesRequest { range: req.range, env: req.env.clone(), service: Some(req.service.clone()), operation: req.operation.clone(), query: req.query.clone() };
    let Json(resources) = super::apm::resources(State(st.clone()), OrgId(org.clone()), Json(resources_req)).await?;
    let resources: Vec<String> = resources.into_iter().map(|r| r.resource).collect();
    let rows: Vec<(i64, String, String, Vec<String>, String)> =
        sqlx::query_as("SELECT id, name, query, tags, status FROM monitors WHERE org_id = $1 AND NOT draft").bind(&org).fetch_all(&st.postgres).await?;
    let mut out: Vec<ServiceMonitor> = rows
        .into_iter()
        .filter(|(_, _, query, tags, _)| std::iter::once(query).chain(tags).any(|t| references_service(t, &req.service)))
        .map(|(id, name, query, tags, status)| {
            let texts: Vec<String> = std::iter::once(query).chain(tags).collect();
            ServiceMonitor { id, name, status: MonitorStatus::parse(&status), resources: monitor_resources(&texts, &resources) }
        })
        .collect();
    out.sort_by(|a, b| b.status.severity().cmp(&a.status.severity()).then_with(|| a.name.cmp(&b.name)));
    Ok(Json(out))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::routes::apm::ServiceRequest;
    use crate::routes::logs::TimeRange;

    async fn monitor(st: &AppState, org: &str, name: &str, query: &str, tags: &[&str], status: &str, draft: bool) {
        let tags: Vec<String> = tags.iter().map(|t| t.to_string()).collect();
        sqlx::query("INSERT INTO monitors (org_id, name, type, query, definition, status, tags, draft) VALUES ($1, $2, 'apm', $3, '{}'::jsonb, $4, $5, $6)")
            .bind(org)
            .bind(name)
            .bind(query)
            .bind(status)
            .bind(&tags)
            .bind(draft)
            .execute(&st.postgres)
            .await
            .expect("monitor");
    }

    #[test]
    fn monitors_target_resources_by_resource_name() {
        let resources: Vec<String> = ["GET /orders/{id}", "POST /orders", "GET /cart"].iter().map(|r| r.to_string()).collect();
        let of = |texts: &[&str]| monitor_resources(&texts.iter().map(|t| t.to_string()).collect::<Vec<_>>(), &resources);
        // Datadog's tag form of the resource (lowercase, `_` for other characters).
        assert_eq!(of(&["avg(last_5m):avg:trace.http.request.duration{service:order-service,resource_name:get_/orders/_id} > 1"]), ["GET /orders/{id}"]);
        assert_eq!(of(&["sum:trace.http.request.errors{service:order-service}", "resource_name:post_/orders"]), ["POST /orders"]);
        // Span searches quote it, under either key.
        assert_eq!(of(&["trace-analytics(\"service:order-service resource_name:\"POST /orders\"\").rollup(\"count\") > 5"]), ["POST /orders"]);
        assert_eq!(of(&["service:order-service @http.status_code:500 resource:\"GET /cart\""]), ["GET /cart"]);
        assert_eq!(of(&["resource_name:(get_/cart OR post_/orders)"]), ["POST /orders", "GET /cart"]);
        // Other keys ending in `resource_name` and unknown resources don't.
        assert!(of(&["avg:aws.elb.latency{service:order-service,aws_resource_name:get_/cart}", "resource_name:get_/payments"]).is_empty());
        assert!(of(&["avg:trace.http.request.duration{service:order-service} > 1"]).is_empty());
    }

    fn req() -> ServiceRequest {
        let now = chrono::Utc::now().timestamp_millis();
        ServiceRequest { range: TimeRange { from: now - 600_000, to: now }, service: "order-service".into(), env: None, resource: None, version: None, operation: None, query: String::new() }
    }

    /// Monitors naming the service in their query or tags, the worst first; drafts and other services' aren't.
    #[tokio::test]
    async fn a_services_monitors_worst_first() {
        let Some((_, states)) = crate::test_support::replicas(1).await else { return };
        let st = &states[0];
        let org = crate::test_support::new_org(st, "apm-monitors").await;
        monitor(st, &org, "Order latency", "avg(last_5m):avg:trace.http.request.duration{service:order-service} > 1", &[], "ok", false).await;
        monitor(st, &org, "Order errors", "sum(last_5m):sum:trace.http.request.errors{env:prod} > 5", &["service:order-service"], "alert", false).await;
        monitor(st, &org, "Order hits", "sum(last_5m):sum:trace.http.request.hits{service:order-service} < 1", &[], "warn", false).await;
        monitor(st, &org, "Draft", "avg(last_5m):avg:trace.http.request.duration{service:order-service} > 1", &[], "alert", true).await;
        monitor(st, &org, "Billing errors", "sum(last_5m):sum:trace.http.request.errors{service:billing} > 5", &[], "alert", false).await;
        let other = crate::test_support::new_org(st, "apm-monitors-other").await;
        monitor(st, &other, "Other org", "avg(last_5m):avg:trace.http.request.duration{service:order-service} > 1", &[], "alert", false).await;

        monitor(st, &org, "Cart latency", "avg(last_5m):avg:trace.http.request.duration{service:order-service,resource_name:get_/cart} > 1", &[], "ok", false).await;
        let now = chrono::Utc::now().timestamp_millis();
        st.ch(&org)
            .query(&format!(
                "INSERT INTO otel.otel_traces (Timestamp, TraceId, SpanId, SpanName, SpanKind, ServiceName, ResourceAttributes, SpanAttributes, Duration, StatusCode) VALUES \
                 (fromUnixTimestamp64Milli(toInt64({})), '{:032x}', '{:016x}', 'GET /cart', 'Server', 'order-service', map('oc.org_id', '{org}'), map('operation.name', 'http.server.request'), 1000000, 'Ok')",
                now - 60_000, 1, 1
            ))
            .execute()
            .await
            .expect("insert span");

        let Json(got) = service_monitors(State(st.clone()), OrgId(org), Json(req())).await.unwrap();
        let got: Vec<(&str, MonitorStatus, Vec<String>)> = got.iter().map(|m| (m.name.as_str(), m.status, m.resources.clone())).collect();
        assert_eq!(
            got,
            [
                ("Order errors", MonitorStatus::Alert, vec![]),
                ("Order hits", MonitorStatus::Warn, vec![]),
                ("Cart latency", MonitorStatus::Ok, vec!["GET /cart".to_string()]),
                ("Order latency", MonitorStatus::Ok, vec![]),
            ]
        );
    }
}
