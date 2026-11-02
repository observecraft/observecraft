//! The Datadog Cluster Agent's external recommender, served by ObserveCraft. A DatadogPodAutoscaler
//! annotated `autoscaling.datadoghq.com/custom-recommender: {"endpoint": "<intake>/api/v2/autoscaling/recommend?api_key=…"}`
//! makes the Cluster Agent POST a `WorkloadRecommendationRequest` every 30 seconds (protojson of
//! agent-payload's `autoscaling/kubernetes/recommender.proto`, BSD-3-Clause) and apply the
//! `targetReplicas` of the `WorkloadRecommendationReply`, within the autoscaler's constraints and
//! scaling rules. The intake admits the key and forwards here (internal listener).
//!
//! Replicas follow the HorizontalPodAutoscaler's rule on the last 5 minutes of the workload's pods:
//! `ceil(current × observed / target)` per objective (CPU or memory utilization of requests), the
//! highest wins, unchanged within a 10 % tolerance, clamped to the constraints. Each answer is
//! stored so the Autoscaling pages show what the recommender decided.

use std::collections::HashMap;

use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;

use super::kubernetes::{resources, KubeKind};
use super::kubernetes_autoscaling::{workload_of_pod, CPU_CORES, IS_CPU, IS_MEMORY, USAGE_METRICS};
use super::metrics::{name_list, now_ms, points_subquery, with_window, OPEN_END_MS};
use crate::error::ApiError;
use crate::state::AppState;

/// Ratios this close to 1 keep the current replicas (the HPA's default tolerance).
const TOLERANCE: f64 = 0.1;

#[derive(Deserialize, Serialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct TargetRef {
    pub cluster: String,
    pub namespace: String,
    pub kind: String,
    pub name: String,
    pub api_version: String,
}

#[derive(Deserialize, Serialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Constraints {
    pub min_replicas: i32,
    pub max_replicas: i32,
}

#[derive(Deserialize, Serialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct WorkloadState {
    pub desired_replicas: i32,
    pub current_replicas: Option<i32>,
    pub ready_replicas: Option<i32>,
}

#[derive(Deserialize, Serialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Target {
    /// `cpu` or `memory` (Kubernetes resource names).
    #[serde(rename = "type")]
    pub kind: String,
    /// Utilization of requests, as a fraction (0.7 for 70 %).
    pub target_value: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lower_bound: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub upper_bound: Option<f64>,
}

/// agent-payload `WorkloadRecommendationRequest`.
#[derive(Deserialize, Serialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct RecommendationRequest {
    pub target_ref: TargetRef,
    pub constraints: Option<Constraints>,
    pub state: Option<WorkloadState>,
    pub targets: Vec<Target>,
    pub settings: HashMap<String, Value>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ReplyError {
    pub code: i32,
    pub message: String,
}

/// agent-payload `WorkloadRecommendationReply`.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct RecommendationReply {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<ReplyError>,
    /// RFC 3339.
    pub timestamp: String,
    pub target_replicas: i32,
    pub observed_targets: Vec<Target>,
    pub reason: String,
}

/// The replicas for a request, given the workload's observed utilization per resource (`cpu`,
/// `memory`: usage over requests, as fractions). `now` is RFC 3339.
pub fn decide(req: &RecommendationRequest, observed: &HashMap<String, f64>, now: &str) -> RecommendationReply {
    let fail = |code: i32, message: String| RecommendationReply { error: Some(ReplyError { code, message }), timestamp: now.to_string(), ..Default::default() };
    let t = &req.target_ref;
    let targets: Vec<&Target> = req.targets.iter().filter(|t| t.target_value > 0.0).collect();
    if targets.is_empty() {
        return fail(400, "the autoscaler has no CPU or memory utilization objective".into());
    }
    let state = req.state.clone().unwrap_or_default();
    let current = state.current_replicas.unwrap_or(state.desired_replicas).max(1);
    let mut best: Option<(i32, String)> = None;
    let mut seen = Vec::new();
    for target in targets {
        let Some(&value) = observed.get(&target.kind) else {
            return fail(404, format!("no {} usage or requests for {} {}/{} in the last 5 minutes", target.kind, t.kind, t.namespace, t.name));
        };
        seen.push(Target { kind: target.kind.clone(), target_value: value, lower_bound: None, upper_bound: None });
        let ratio = value / target.target_value;
        let wanted = if (ratio - 1.0).abs() <= TOLERANCE { current } else { (f64::from(current) * ratio - 1e-9).ceil() as i32 };
        if best.as_ref().is_none_or(|(n, _)| wanted > *n) {
            best = Some((wanted, format!("{} utilization {:.0}% for a target of {:.0}%", target.kind, value * 100.0, target.target_value * 100.0)));
        }
    }
    let (wanted, why) = best.unwrap_or((current, String::new()));
    let constraints = req.constraints.clone().unwrap_or_default();
    let mut replicas = wanted.max(constraints.min_replicas).max(1);
    if constraints.max_replicas > 0 {
        replicas = replicas.min(constraints.max_replicas);
    }
    RecommendationReply { error: None, timestamp: now.to_string(), target_replicas: replicas, observed_targets: seen, reason: format!("{why}: {current} → {replicas} replicas") }
}

// ================================================================ serving

/// Utilization of requests (`cpu`, `memory`) of the workload's current pods over the last 5 minutes.
async fn observe(st: &AppState, org: &str, t: &TargetRef) -> Result<HashMap<String, f64>, ApiError> {
    let kind = match t.kind.as_str() {
        "Deployment" => KubeKind::Deployments,
        "StatefulSet" => KubeKind::StatefulSets,
        _ => return Ok(HashMap::new()),
    };
    let rows = resources(st, org).await?;
    let tag = if kind == KubeKind::StatefulSets { "kube_stateful_set:" } else { "kube_deployment:" };
    let wanted = format!("{tag}{}", t.name);
    let pods: Vec<_> = rows
        .iter()
        .filter(|p| p.kind == Some(KubeKind::Pods) && p.cluster == t.cluster && p.namespace.as_deref() == Some(t.namespace.as_str()) && p.status == "Running" && p.tags.contains(&wanted))
        .collect();
    #[derive(clickhouse::Row, Deserialize)]
    struct PodNow {
        pod: String,
        cpu: f64,
        mem: f64,
    }
    let now = now_ms();
    let src = points_subquery("gauge", USAGE_METRICS);
    let usage = with_window(st.ch(org), now - 5 * 60_000, now + OPEN_END_MS)
        .with_setting("param_cluster", &t.cluster)
        .with_setting("param_namespace", &t.namespace)
        .with_setting("param_prefix", format!("{}-", t.name))
        .query(&format!(
            "SELECT pod, sum(cpu) AS cpu, sum(mem) AS mem FROM ( \
               SELECT ResourceAttributes['k8s.pod.name'] AS pod, ResourceAttributes['k8s.container.name'] AS container, \
               ifNotFinite(avgIf({CPU_CORES}, {IS_CPU}), 0) AS cpu, ifNotFinite(avgIf(Value, {IS_MEMORY}), 0) AS mem \
               FROM {src} WHERE OrgId = {{oc_org:String}} AND TimeUnix > now() - INTERVAL 5 MINUTE AND MetricName IN ({names}) \
               AND ResourceAttributes['k8s.cluster.name'] = {{cluster:String}} AND ResourceAttributes['k8s.namespace.name'] = {{namespace:String}} \
               AND startsWith(ResourceAttributes['k8s.pod.name'], {{prefix:String}}) AND container != '' GROUP BY pod, container) \
             GROUP BY pod LIMIT 10000",
            names = name_list(USAGE_METRICS),
        ))
        .fetch_all::<PodNow>()
        .await?;
    let candidates = [(t.kind.clone(), t.name.clone())];
    let by_pod: HashMap<&str, &PodNow> = usage.iter().filter(|u| workload_of_pod(&u.pod, &candidates).is_some()).map(|u| (u.pod.as_str(), u)).collect();
    let (mut used, mut requested) = (HashMap::<&str, f64>::new(), HashMap::<&str, f64>::new());
    for p in pods {
        let Some(u) = by_pod.get(p.name.as_str()) else { continue };
        for (resource, value, request) in [("cpu", u.cpu, p.cpu.as_ref().and_then(|c| c.requests)), ("memory", u.mem, p.memory.as_ref().and_then(|m| m.requests))] {
            if let Some(r) = request.filter(|r| *r > 0.0) {
                *used.entry(resource).or_default() += value;
                *requested.entry(resource).or_default() += r;
            }
        }
    }
    Ok(used.into_iter().filter_map(|(k, v)| requested.get(k).map(|r| (k.to_string(), v / r))).collect())
}

/// The recommender's last answer for an autoscaler's target.
#[derive(Serialize, Deserialize, ToSchema, Clone, Debug, PartialEq, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct StoredRecommendation {
    pub cluster: String,
    pub namespace: String,
    pub kind: String,
    pub name: String,
    pub current_replicas: i32,
    pub target_replicas: i32,
    pub reason: String,
    pub error: Option<String>,
    /// Epoch ms.
    pub updated_at: i64,
}

async fn store(st: &AppState, org: &str, req: &RecommendationRequest, reply: &RecommendationReply) -> Result<(), sqlx::Error> {
    let t = &req.target_ref;
    let state = req.state.clone().unwrap_or_default();
    sqlx::query(
        "INSERT INTO kube_autoscaler_recommendations (org_id, cluster, namespace, kind, name, current_replicas, target_replicas, reason, error, observed) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10) \
         ON CONFLICT (org_id, cluster, namespace, kind, name) DO UPDATE SET current_replicas = EXCLUDED.current_replicas, target_replicas = EXCLUDED.target_replicas, \
         reason = EXCLUDED.reason, error = EXCLUDED.error, observed = EXCLUDED.observed, updated_at = now()",
    )
    .bind(org)
    .bind(&t.cluster)
    .bind(&t.namespace)
    .bind(&t.kind)
    .bind(&t.name)
    .bind(state.current_replicas.unwrap_or(state.desired_replicas))
    .bind(reply.target_replicas)
    .bind(&reply.reason)
    .bind(reply.error.as_ref().map(|e| e.message.clone()))
    .bind(sqlx::types::Json(&reply.observed_targets))
    .execute(&st.postgres)
    .await
    .map(|_| ())
}

pub async fn stored(st: &AppState, org: &str) -> Result<Vec<StoredRecommendation>, sqlx::Error> {
    sqlx::query_as(
        "SELECT cluster, namespace, kind, name, current_replicas, target_replicas, reason, error, (extract(epoch FROM updated_at) * 1000)::bigint AS updated_at \
         FROM kube_autoscaler_recommendations WHERE org_id = $1 AND updated_at > now() - INTERVAL '1 day' ORDER BY cluster, namespace, name",
    )
    .bind(org)
    .fetch_all(&st.postgres)
    .await
}

fn rfc3339_now() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

/// `POST /ingest/autoscaling-recommendation` (internal listener): the intake forwards the Cluster
/// Agent's request with the org of its key.
pub async fn ingest(State(st): State<AppState>, headers: HeaderMap, body: Bytes) -> Response {
    let org = match super::code_security::internal_org(&st, &headers) {
        Ok(o) => o,
        Err(s) => return s.into_response(),
    };
    let Ok(req) = serde_json::from_slice::<RecommendationRequest>(&body) else {
        return (StatusCode::BAD_REQUEST, "not a WorkloadRecommendationRequest").into_response();
    };
    let reply = match observe(&st, &org, &req.target_ref).await {
        Ok(observed) => decide(&req, &observed, &rfc3339_now()),
        Err(e) => {
            tracing::warn!(error = ?e, "autoscaling recommendation: reading usage failed");
            return StatusCode::SERVICE_UNAVAILABLE.into_response();
        }
    };
    if let Err(e) = store(&st, &org, &req, &reply).await {
        tracing::warn!(error = %e, "autoscaling recommendation not stored");
    }
    Json(reply).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const NOW: &str = "2026-11-01T18:00:00Z";

    /// What the Cluster Agent sends (protojson: camelCase, zero values left out).
    const REQUEST: &str = r#"{"targetRef":{"cluster":"shop-prod","namespace":"checkout","kind":"Deployment","name":"storefront-api","apiVersion":"apps/v1"},
        "constraints":{"minReplicas":2,"maxReplicas":10},"state":{"desiredReplicas":3,"currentReplicas":3,"readyReplicas":3},
        "targets":[{"type":"cpu","targetValue":0.7}],"settings":{"team":"checkout"}}"#;

    fn request() -> RecommendationRequest {
        serde_json::from_str(REQUEST).expect("protojson request")
    }

    fn observed(cpu: f64) -> HashMap<String, f64> {
        HashMap::from([("cpu".to_string(), cpu)])
    }

    #[test]
    fn the_agents_request_parses_with_zero_values_left_out() {
        let r = request();
        assert_eq!((r.target_ref.name.as_str(), r.target_ref.cluster.as_str(), r.target_ref.kind.as_str()), ("storefront-api", "shop-prod", "Deployment"));
        assert_eq!(r.constraints, Some(Constraints { min_replicas: 2, max_replicas: 10 }));
        assert_eq!(r.state.as_ref().and_then(|s| s.current_replicas), Some(3));
        assert_eq!(r.targets[0], Target { kind: "cpu".into(), target_value: 0.7, lower_bound: None, upper_bound: None });
        let sparse: RecommendationRequest = serde_json::from_str(r#"{"targetRef":{"namespace":"checkout","kind":"Deployment","name":"billing"},"state":{"currentReplicas":1}}"#).unwrap();
        assert_eq!((sparse.state.unwrap().desired_replicas, sparse.constraints, sparse.targets.len()), (0, None, 0));
    }

    #[test]
    fn replicas_scale_with_the_ratio_of_observed_to_target_utilization() {
        let reply = decide(&request(), &observed(1.4), NOW);
        assert_eq!((reply.target_replicas, reply.error.clone()), (6, None));
        assert_eq!(reply.observed_targets, [Target { kind: "cpu".into(), target_value: 1.4, lower_bound: None, upper_bound: None }]);
        assert_eq!(reply.timestamp, NOW);
        assert_eq!(reply.reason, "cpu utilization 140% for a target of 70%: 3 → 6 replicas");
        // The reply is the protojson the agent reads.
        let wire = serde_json::to_value(&reply).unwrap();
        assert_eq!(wire["targetReplicas"], json!(6));
        assert_eq!(wire["observedTargets"][0], json!({ "type": "cpu", "targetValue": 1.4 }));
        assert!(wire.get("error").is_none());
    }

    #[test]
    fn within_the_tolerance_replicas_stay_and_constraints_clamp() {
        assert_eq!(decide(&request(), &observed(0.74), NOW).target_replicas, 3);
        assert_eq!(decide(&request(), &observed(0.1), NOW).target_replicas, 2, "never below minReplicas");
        assert_eq!(decide(&request(), &observed(5.0), NOW).target_replicas, 10, "never above maxReplicas");
    }

    #[test]
    fn the_objective_needing_most_replicas_wins() {
        let mut req = request();
        req.targets.push(Target { kind: "memory".into(), target_value: 0.8, lower_bound: None, upper_bound: None });
        let reply = decide(&req, &HashMap::from([("cpu".to_string(), 0.35), ("memory".to_string(), 1.6)]), NOW);
        assert_eq!(reply.target_replicas, 6);
        assert_eq!(reply.observed_targets.len(), 2);
    }

    #[test]
    fn missing_objectives_or_usage_are_errors_the_agent_reports() {
        let mut no_targets = request();
        no_targets.targets.clear();
        let reply = decide(&no_targets, &observed(1.0), NOW);
        assert_eq!(reply.error.map(|e| e.code), Some(400));
        let reply = decide(&request(), &HashMap::new(), NOW);
        assert_eq!(reply.error.as_ref().map(|e| e.code), Some(404));
        assert_eq!(reply.error.map(|e| e.message), Some("no cpu usage or requests for Deployment checkout/storefront-api in the last 5 minutes".to_string()));
    }
    /// The Cluster Agent's request, forwarded by the intake with the org of its key: the workload's
    /// pods use 160 % of their CPU requests (the Datadog Agent's `container.cpu.usage`, in nanocores), so 2 replicas become 5 (target 70 %), the answer is
    /// stored for the Autoscaling pages; another org has no such workload.
    #[tokio::test]
    async fn the_internal_endpoint_answers_from_the_workloads_usage_and_stores_it() {
        let Some((_, states)) = crate::test_support::replicas(1).await else { return };
        let st = &states[0];
        let org = format!("k8s-recommender-{}-{}", std::process::id(), now_ms());
        let pods = ["storefront-api-7d9f8c6b5-abcde", "storefront-api-7d9f8c6b5-fghij"];
        let mut bodies = vec![
            json!({ "kind": "Deployment", "metadata": { "uid": format!("{org}-dep"), "name": "storefront-api", "namespace": "checkout" }, "spec": { "replicas": 2 } }),
            json!({ "kind": "ReplicaSet", "metadata": { "uid": format!("{org}-rs"), "name": "storefront-api-7d9f8c6b5", "namespace": "checkout", "ownerReferences": [{ "kind": "Deployment", "name": "storefront-api", "uid": format!("{org}-dep"), "controller": true }] } }),
        ];
        for (i, pod) in pods.iter().enumerate() {
            bodies.push(json!({ "kind": "Pod", "metadata": { "uid": format!("{org}-pod-{i}"), "name": pod, "namespace": "checkout", "ownerReferences": [{ "kind": "ReplicaSet", "name": "storefront-api-7d9f8c6b5", "uid": format!("{org}-rs"), "controller": true }] },
                "spec": { "containers": [{ "name": "app", "image": "registry.example.com/storefront-api:2.4.0", "resources": { "requests": { "cpu": "500m", "memory": "1Gi" } } }] },
                "status": { "phase": "Running", "containerStatuses": [{ "name": "app", "ready": true, "restartCount": 0, "state": { "running": {} } }] } }));
        }
        let ch = st.ch(&org);
        ch.query("INSERT INTO otel.otel_k8s_objects (Timestamp, Body, ResourceAttributes, ScopeName) SELECT now64(9), arrayJoin(?), map('oc.org_id', ?, 'k8s.cluster.name', 'shop-prod', 'oc.signal', 'k8s.objects'), 'datadog.orchestrator'")
            .bind(bodies.iter().map(Value::to_string).collect::<Vec<_>>())
            .bind(&org)
            .execute()
            .await
            .expect("insert objects");
        for pod in pods {
            ch.query(
                "INSERT INTO otel.metrics_ingest_gauge (ResourceAttributes, MetricName, Attributes, TimeUnix, Value) \
                 VALUES (map('oc.org_id', ?, 'k8s.cluster.name', 'shop-prod', 'k8s.namespace.name', 'checkout', 'k8s.pod.name', ?, 'k8s.container.name', 'app', 'observecraft.collector', 'datadog-agent'), 'container.cpu.usage', map(), now64(9) - toIntervalSecond(30), 0.8e9)",
            )
            .bind(&org)
            .bind(pod)
            .execute()
            .await
            .expect("insert usage");
        }
        let headers = |org: &str| {
            let mut h = HeaderMap::new();
            h.insert("x-oc-org", org.parse().unwrap());
            h
        };
        let body = Bytes::from(REQUEST.replace(r#""minReplicas":2,"maxReplicas":10"#, r#""minReplicas":1,"maxReplicas":10"#).replace(r#""currentReplicas":3"#, r#""currentReplicas":2"#));
        let res = ingest(State(st.clone()), headers(&org), body.clone()).await;
        assert_eq!(res.status(), StatusCode::OK);
        let reply: RecommendationReply = serde_json::from_slice(&axum::body::to_bytes(res.into_body(), 1 << 20).await.unwrap()).unwrap();
        assert_eq!((reply.target_replicas, reply.error), (5, None), "{}", reply.reason);
        assert_eq!(reply.observed_targets[0].target_value, 1.6);
        let saved = stored(st, &org).await.unwrap();
        assert_eq!(saved.iter().map(|r| (r.name.as_str(), r.current_replicas, r.target_replicas)).collect::<Vec<_>>(), [("storefront-api", 2, 5)]);

        let other = format!("{org}-other");
        let res = ingest(State(st.clone()), headers(&other), body).await;
        let reply: RecommendationReply = serde_json::from_slice(&axum::body::to_bytes(res.into_body(), 1 << 20).await.unwrap()).unwrap();
        assert_eq!(reply.error.map(|e| e.code), Some(404));
        assert_eq!(stored(st, &other).await.unwrap()[0].error.as_deref().map(|e| e.starts_with("no cpu usage")), Some(true));
        assert_eq!(ingest(State(st.clone()), headers(&org), Bytes::from_static(b"not json")).await.status(), StatusCode::BAD_REQUEST);
    }
}
