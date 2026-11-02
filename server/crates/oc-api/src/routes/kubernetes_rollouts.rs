//! Kubernetes → Live Rollouts: each Deployment's latest rollout, from its status (`updatedReplicas`,
//! `availableReplicas`, the `Progressing` condition) and its newest ReplicaSet (highest
//! `deployment.kubernetes.io/revision`, created when the rollout started).

use axum::extract::State;
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;

use super::kubernetes_autoscaling::{custom_objects, CustomObject};
use super::metrics::now_ms;
use crate::error::ApiResult;
use crate::state::{AppState, OrgId};

const REVISION: &str = "deployment.kubernetes.io/revision";

#[derive(Serialize, Deserialize, ToSchema, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RolloutStatus {
    InProgress,
    Complete,
    /// The Progressing condition reports `ProgressDeadlineExceeded`.
    Stalled,
    ScaledDown,
}

#[derive(Serialize, Deserialize, ToSchema, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Rollout {
    pub uid: String,
    pub cluster: String,
    pub namespace: String,
    pub name: String,
    pub revision: Option<u64>,
    pub status: RolloutStatus,
    pub desired: u32,
    pub updated: u32,
    pub ready: u32,
    pub available: u32,
    /// The image of the first container.
    pub image: Option<String>,
    /// Epoch ms: the newest ReplicaSet's creation, and the last progress of a completed rollout.
    pub started: Option<i64>,
    pub finished: Option<i64>,
    pub duration_seconds: Option<f64>,
    /// The Progressing condition's message.
    pub message: Option<String>,
}

/// The latest rollout of every Deployment, the most recent first.
pub fn rollouts(objects: &[CustomObject], now: i64) -> Vec<Rollout> {
    let num = |v: &Value, path: &str| v.pointer(path).and_then(Value::as_u64).and_then(|n| u32::try_from(n).ok());
    let time = |v: Option<&str>| v.and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok()).map(|t| t.timestamp_millis());
    let mut out: Vec<Rollout> = objects
        .iter()
        .filter(|o| o.kind == "Deployment")
        .map(|d| {
            let newest = objects
                .iter()
                .filter(|rs| rs.kind == "ReplicaSet" && rs.cluster == d.cluster && rs.namespace == d.namespace && rs.owner_kind == "Deployment" && rs.owner_name == d.name)
                .filter_map(|rs| rs.annotations.get(REVISION).and_then(|r| r.parse::<u64>().ok()).map(|r| (r, rs)))
                .max_by_key(|(r, _)| *r);
            let desired = num(&d.doc, "/spec/replicas").unwrap_or(1);
            let (updated, available, ready) = (num(&d.doc, "/status/updatedReplicas").unwrap_or(0), num(&d.doc, "/status/availableReplicas").unwrap_or(0), num(&d.doc, "/status/readyReplicas").unwrap_or(0));
            let total = num(&d.doc, "/status/replicas").unwrap_or(updated);
            let progressing = d.doc.pointer("/status/conditions").and_then(Value::as_array).and_then(|cs| cs.iter().find(|c| c.get("type").and_then(Value::as_str) == Some("Progressing")));
            let reason = progressing.and_then(|c| c.get("reason")).and_then(Value::as_str);
            let status = if desired == 0 {
                RolloutStatus::ScaledDown
            } else if reason == Some("ProgressDeadlineExceeded") {
                RolloutStatus::Stalled
            } else if updated >= desired && available >= desired && total <= updated {
                RolloutStatus::Complete
            } else {
                RolloutStatus::InProgress
            };
            let started = if status == RolloutStatus::ScaledDown { None } else { newest.and_then(|(_, rs)| rs.created) };
            let finished = if status == RolloutStatus::Complete { time(progressing.and_then(|c| c.get("lastUpdateTime")).and_then(Value::as_str)) } else { None };
            Rollout {
                uid: d.uid.clone(),
                cluster: d.cluster.clone(),
                namespace: d.namespace.clone(),
                name: d.name.clone(),
                revision: newest.map(|(r, _)| r),
                status,
                desired,
                updated,
                ready,
                available,
                image: d.doc.pointer("/spec/template/spec/containers/0/image").and_then(Value::as_str).map(str::to_string),
                started,
                finished,
                duration_seconds: started.map(|s| (finished.unwrap_or(now) - s).max(0) as f64 / 1000.0),
                message: progressing.and_then(|c| c.get("message")).and_then(Value::as_str).map(str::to_string),
            }
        })
        .collect();
    out.sort_by(|a, b| b.started.cmp(&a.started).then(a.name.cmp(&b.name)));
    out
}

#[utoipa::path(get, path = "/api/v1/kubernetes/rollouts", operation_id = "list_kubernetes_rollouts", responses((status = 200, body = [Rollout])), tag = "orchestration")]
pub async fn list(State(st): State<AppState>, OrgId(org): OrgId) -> ApiResult<Vec<Rollout>> {
    Ok(Json(rollouts(&custom_objects(&st, &org, &["Deployment", "ReplicaSet"]).await?, now_ms())))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::collections::BTreeMap;

    const T: i64 = 1_793_550_000_000;

    fn rfc(ms: i64) -> String {
        chrono::DateTime::from_timestamp_millis(ms).unwrap().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
    }

    fn deployment(name: &str, desired: u32, updated: u32, available: u32, progressing: Option<(&str, i64)>) -> CustomObject {
        let conditions: Vec<Value> = progressing.map(|(reason, at)| json!({ "type": "Progressing", "status": "True", "reason": reason, "lastUpdateTime": rfc(at), "message": format!("ReplicaSet \"{name}\" {reason}") })).into_iter().collect();
        CustomObject {
            cluster: "shop-prod".into(),
            kind: "Deployment".into(),
            uid: format!("dep-{name}"),
            namespace: "checkout".into(),
            name: name.into(),
            doc: json!({ "spec": { "replicas": desired, "template": { "spec": { "containers": [{ "image": format!("registry.example.com/{name}:2.5.0") }] } } },
                         "status": { "replicas": desired, "updatedReplicas": updated, "availableReplicas": available, "readyReplicas": available, "conditions": conditions } }),
            ..Default::default()
        }
    }

    fn replica_set(deployment: &str, revision: u64, created: i64) -> CustomObject {
        CustomObject {
            cluster: "shop-prod".into(),
            kind: "ReplicaSet".into(),
            uid: format!("rs-{deployment}-{revision}"),
            namespace: "checkout".into(),
            name: format!("{deployment}-{revision}abc"),
            created: Some(created),
            annotations: BTreeMap::from([(REVISION.to_string(), revision.to_string())]),
            owner_kind: "Deployment".into(),
            owner_name: deployment.into(),
            ..Default::default()
        }
    }

    #[test]
    fn rollouts_take_their_status_from_the_deployment_and_start_from_the_newest_replica_set() {
        let objects = vec![
            deployment("storefront-api", 3, 3, 3, Some(("NewReplicaSetAvailable", T - 60_000))),
            replica_set("storefront-api", 1, T - 86_400_000),
            replica_set("storefront-api", 2, T - 150_000),
            deployment("order-service", 4, 1, 3, Some(("ReplicaSetUpdated", T - 10_000))),
            replica_set("order-service", 7, T - 30_000),
            deployment("billing", 2, 1, 1, Some(("ProgressDeadlineExceeded", T - 5_000))),
            replica_set("billing", 3, T - 700_000),
            deployment("batch-reports", 0, 0, 0, None),
        ];
        let got = rollouts(&objects, T);
        assert_eq!(got.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(), ["order-service", "storefront-api", "billing", "batch-reports"]);
        let by = |n: &str| got.iter().find(|r| r.name == n).unwrap();

        let done = by("storefront-api");
        assert_eq!((done.status, done.revision, done.started, done.finished), (RolloutStatus::Complete, Some(2), Some(T - 150_000), Some(T - 60_000)));
        assert_eq!(done.duration_seconds, Some(90.0));
        assert_eq!((done.desired, done.updated, done.available), (3, 3, 3));
        assert_eq!(done.image.as_deref(), Some("registry.example.com/storefront-api:2.5.0"));

        let running = by("order-service");
        assert_eq!((running.status, running.finished, running.duration_seconds), (RolloutStatus::InProgress, None, Some(30.0)));

        let stalled = by("billing");
        assert_eq!(stalled.status, RolloutStatus::Stalled);
        assert_eq!(stalled.message.as_deref(), Some("ReplicaSet \"billing\" ProgressDeadlineExceeded"));

        let idle = by("batch-reports");
        assert_eq!((idle.status, idle.started, idle.duration_seconds), (RolloutStatus::ScaledDown, None, None));
    }
}
