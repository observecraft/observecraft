//! The monitors of an APM service, for the service page's monitor bar: the org's monitors whose query or tags
//! name the service (`service:order-service`), as the Software Catalog counts them.

use axum::extract::State;
use axum::Json;
use serde::Serialize;
use utoipa::ToSchema;

use super::apm::ServiceRequest;
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
}

/// The monitors naming the service in their query or tags (drafts aside), the worst status first.
#[utoipa::path(post, path = "/api/v1/apm/service/monitors", request_body = ServiceRequest, responses((status = 200, body = Vec<ServiceMonitor>)), tag = "apm")]
pub async fn service_monitors(State(st): State<AppState>, OrgId(org): OrgId, Json(req): Json<ServiceRequest>) -> ApiResult<Vec<ServiceMonitor>> {
    let rows: Vec<(i64, String, String, Vec<String>, String)> =
        sqlx::query_as("SELECT id, name, query, tags, status FROM monitors WHERE org_id = $1 AND NOT draft").bind(&org).fetch_all(&st.postgres).await?;
    let mut out: Vec<ServiceMonitor> = rows
        .into_iter()
        .filter(|(_, _, query, tags, _)| std::iter::once(query).chain(tags).any(|t| references_service(t, &req.service)))
        .map(|(id, name, _, _, status)| ServiceMonitor { id, name, status: MonitorStatus::parse(&status) })
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

        let Json(got) = service_monitors(State(st.clone()), OrgId(org), Json(req())).await.unwrap();
        let got: Vec<(&str, MonitorStatus)> = got.iter().map(|m| (m.name.as_str(), m.status)).collect();
        assert_eq!(got, [("Order errors", MonitorStatus::Alert), ("Order hits", MonitorStatus::Warn), ("Order latency", MonitorStatus::Ok)]);
    }
}
