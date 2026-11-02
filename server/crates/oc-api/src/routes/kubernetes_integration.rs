//! Integrations → Kubernetes: the clusters sending data, where their objects come from (the Datadog
//! Cluster Agent's orchestrator manifests or the OpenTelemetry Collector's k8sobjects receiver) and
//! whether the Cluster Agent's workload autoscaling is installed (its `DatadogPodAutoscaler` CRD).
//! Installation itself is the official Datadog chart or the observecraft-agent chart; the page
//! generates their values.

use std::collections::{BTreeMap, BTreeSet};

use axum::extract::State;
use axum::Json;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use super::kubernetes::{resources, KubeKind, KubeResource};
use crate::error::{ApiError, ApiResult};
use crate::state::{AppState, OrgId};

/// The CRD the Cluster Agent's workload autoscaling installs.
pub const DPA_CRD: &str = "datadogpodautoscalers.datadoghq.com";
/// The scope name of objects sent through the intake's orchestrator endpoint (`datadog_orch.rs`).
const DATADOG_SCOPE: &str = "datadog.orchestrator";

/// Who sends a cluster's objects.
#[derive(Serialize, Deserialize, ToSchema, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
#[serde(rename_all = "snake_case")]
pub enum KubeObjectSource {
    /// The Datadog Cluster Agent's orchestrator explorer.
    DatadogClusterAgent,
    /// The OpenTelemetry Collector's k8sobjects receiver (observecraft-agent chart).
    Opentelemetry,
}

#[derive(Serialize, Deserialize, ToSchema, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct KubeClusterSummary {
    pub name: String,
    pub nodes: u32,
    pub pods: u32,
    pub version: Option<String>,
    /// Epoch ms.
    pub last_seen: i64,
    /// Allocatable cores and bytes.
    pub cpu_capacity: Option<f64>,
    pub memory_capacity: Option<f64>,
    pub status: String,
    pub sources: Vec<KubeObjectSource>,
    /// The Cluster Agent's workload autoscaling CRD is installed.
    pub autoscaling: bool,
}

/// One cluster summary per cluster row of the inventory. `scopes` are the (cluster, scope name)
/// pairs of the objects received recently.
pub fn summaries(rows: &[KubeResource], scopes: &[(String, String)]) -> Vec<KubeClusterSummary> {
    let mut sources: BTreeMap<String, BTreeSet<KubeObjectSource>> = BTreeMap::new();
    for (cluster, scope) in scopes {
        sources.entry(cluster.clone()).or_default().insert(if scope == DATADOG_SCOPE { KubeObjectSource::DatadogClusterAgent } else { KubeObjectSource::Opentelemetry });
    }
    let mut out: BTreeMap<String, KubeClusterSummary> = BTreeMap::new();
    let blank = |name: &str| KubeClusterSummary {
        name: name.to_string(),
        nodes: 0,
        pods: 0,
        version: None,
        last_seen: 0,
        cpu_capacity: None,
        memory_capacity: None,
        status: "No Nodes".into(),
        sources: Vec::new(),
        autoscaling: false,
    };
    for r in rows.iter().filter(|r| r.kind == Some(KubeKind::Clusters)) {
        out.insert(
            r.name.clone(),
            KubeClusterSummary {
                nodes: r.nodes.unwrap_or(0),
                pods: r.pods.unwrap_or(0),
                version: r.version.clone(),
                last_seen: r.last_seen,
                cpu_capacity: r.cpu.as_ref().and_then(|u| u.capacity),
                memory_capacity: r.memory.as_ref().and_then(|u| u.capacity),
                status: r.status.clone(),
                ..blank(&r.name)
            },
        );
    }
    for r in rows.iter().filter(|r| r.kind == Some(KubeKind::CustomResourceDefinitions) && r.name == DPA_CRD) {
        out.entry(r.cluster.clone()).or_insert_with(|| blank(&r.cluster)).autoscaling = true;
    }
    for (cluster, set) in sources {
        out.entry(cluster.clone()).or_insert_with(|| blank(&cluster)).sources = set.into_iter().collect();
    }
    out.into_values().collect()
}

#[derive(clickhouse::Row, Deserialize)]
struct ScopeRow {
    cluster: String,
    scope: String,
}

#[utoipa::path(get, path = "/api/v1/kubernetes/clusters", operation_id = "list_kubernetes_clusters", responses((status = 200, body = [KubeClusterSummary])), tag = "orchestration")]
pub async fn clusters(State(st): State<AppState>, OrgId(org): OrgId) -> ApiResult<Vec<KubeClusterSummary>> {
    let rows = resources(&st, &org).await?;
    let scopes = st
        .ch(&org)
        .query(
            "SELECT ResourceAttributes['k8s.cluster.name'] AS cluster, ScopeName AS scope FROM otel.otel_k8s_objects \
             WHERE OrgId = {oc_org:String} AND TimestampTime > now() - INTERVAL 15 MINUTE AND cluster != '' GROUP BY cluster, scope LIMIT 1000",
        )
        .fetch_all::<ScopeRow>()
        .await
        .map_err(ApiError::from)?;
    Ok(Json(summaries(&rows, &scopes.into_iter().map(|r| (r.cluster, r.scope)).collect::<Vec<_>>())))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::routes::kubernetes::KubeUtilization;

    fn cluster(name: &str, nodes: u32) -> KubeResource {
        KubeResource {
            uid: format!("cluster:{name}"),
            kind: Some(KubeKind::Clusters),
            cluster: name.into(),
            name: name.into(),
            nodes: Some(nodes),
            pods: Some(12),
            version: Some("v1.31.2".into()),
            last_seen: 1_792_953_998_000,
            status: "Healthy".into(),
            cpu: Some(KubeUtilization { capacity: Some(8.0), ..Default::default() }),
            memory: Some(KubeUtilization { capacity: Some(32e9), ..Default::default() }),
            ..Default::default()
        }
    }

    fn crd(cluster: &str, name: &str) -> KubeResource {
        KubeResource { uid: format!("{cluster}/{name}"), kind: Some(KubeKind::CustomResourceDefinitions), cluster: cluster.into(), name: name.into(), ..Default::default() }
    }

    #[test]
    fn clusters_carry_their_object_sources_and_autoscaling_crd() {
        let rows = vec![cluster("shop-prod", 3), cluster("shop-staging", 1), crd("shop-prod", DPA_CRD), crd("shop-staging", "rollouts.argoproj.io")];
        let scopes = vec![
            ("shop-prod".to_string(), DATADOG_SCOPE.to_string()),
            ("shop-staging".to_string(), "github.com/open-telemetry/opentelemetry-collector-contrib/receiver/k8sobjectsreceiver".to_string()),
            ("shop-staging".to_string(), DATADOG_SCOPE.to_string()),
        ];
        let got = summaries(&rows, &scopes);
        assert_eq!(got.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), ["shop-prod", "shop-staging"]);
        let prod = &got[0];
        assert_eq!((prod.nodes, prod.pods, prod.version.as_deref(), prod.cpu_capacity, prod.memory_capacity), (3, 12, Some("v1.31.2"), Some(8.0), Some(32e9)));
        assert_eq!((prod.sources.clone(), prod.autoscaling), (vec![KubeObjectSource::DatadogClusterAgent], true));
        assert_eq!((got[1].sources.clone(), got[1].autoscaling), (vec![KubeObjectSource::DatadogClusterAgent, KubeObjectSource::Opentelemetry], false));
    }

    /// Objects received through the intake's orchestrator endpoint list the cluster with the Cluster
    /// Agent as source, for their org only.
    #[tokio::test]
    async fn the_route_reads_the_clusters_of_the_org_from_its_objects() {
        let Some((_, states)) = crate::test_support::replicas(1).await else { return };
        let st = &states[0];
        let org = format!("k8s-integration-{}-{}", std::process::id(), super::super::metrics::now_ms());
        let node = serde_json::json!({ "kind": "Node", "metadata": { "uid": format!("{org}-node"), "name": "i-0a1b2c3d4e5f60001" }, "status": { "allocatable": { "cpu": "4", "memory": "16Gi" } } });
        st.ch(&org)
            .query("INSERT INTO otel.otel_k8s_objects (Timestamp, Body, ResourceAttributes, ScopeName) VALUES (now64(9), ?, map('oc.org_id', ?, 'k8s.cluster.name', 'shop-prod', 'oc.signal', 'k8s.objects'), ?)")
            .bind(node.to_string())
            .bind(&org)
            .bind(DATADOG_SCOPE)
            .execute()
            .await
            .expect("insert node");
        let Json(got) = clusters(State(st.clone()), OrgId(org.clone())).await.unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!((got[0].name.as_str(), got[0].nodes, got[0].sources.clone()), ("shop-prod", 1, vec![KubeObjectSource::DatadogClusterAgent]));
        let Json(other) = clusters(State(st.clone()), OrgId(format!("{org}-other"))).await.unwrap();
        assert!(other.is_empty());
    }

    #[test]
    fn a_cluster_seen_only_through_its_objects_is_listed_without_nodes() {
        let got = summaries(&[], &[("shop-dev".to_string(), DATADOG_SCOPE.to_string())]);
        assert_eq!((got.len(), got[0].name.as_str(), got[0].nodes, got[0].status.as_str()), (1, "shop-dev", 0, "No Nodes"));
    }
}
