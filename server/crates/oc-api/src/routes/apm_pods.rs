//! The pods a service runs in, for its page's Infrastructure section (as Datadog's service page shows containerized
//! services): the Kubernetes inventory's pods of the service, and their CPU and memory over the range.
//!
//! A pod is the service's when it carries the unified service tag (`service:<name>`, from its
//! `tags.datadoghq.com/service` label, else `app.kubernetes.io/name` / `app`, as the inventory tags pods), when a
//! Kubernetes Service named like it selects the pod (`kube_service`), or when the service's spans name it (the
//! Datadog Agent's container tags, `pod_name`, or OpenTelemetry's `k8s.pod.name` / `k8s.pod.uid`).

use std::collections::BTreeMap;

use axum::extract::State;
use axum::Json;
use oc_query::Param;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use super::hosts::HostPod;
use super::kubernetes::{KubeKind, KubeResource};
use super::kubernetes_autoscaling::{IS_MEMORY, USAGE_METRICS};
use super::logs::{bind, TimeRange};
use super::metrics::{name_list, now_ms, points_subquery, with_window, MetricSeries};
use crate::error::{ApiError, ApiResult};
use crate::state::{AppState, OrgId};

/// At most this many pods are listed and charted.
const MAX_PODS: usize = 200;
const MIB: f64 = 1024.0 * 1024.0;
/// CPU points: the Datadog Agent's kubelet check (nanocores) and the OpenTelemetry Collector's kubeletstats (cores).
/// The Agent's `container.cpu.usage` is left out: its container check (nanocores) and its live containers (cores)
/// both send that name.
/// A Kubernetes Service's cluster, namespace and selector labels.
type Selector<'a> = (&'a str, Option<&'a str>, Vec<(&'a str, &'a str)>);
/// Series points by pod name.
type ByPod = BTreeMap<String, Vec<(i64, f64)>>;

const POD_CPU: &str = "(MetricName = 'kubernetes.cpu.usage.total' OR (MetricName IN ('container.cpu.usage', 'container.cpu.utilization') AND ResourceAttributes['observecraft.collector'] != 'datadog-agent'))";

#[derive(Deserialize, ToSchema)]
pub struct ServicePodsRequest {
    pub range: TimeRange,
    pub service: String,
    pub env: Option<String>,
}

#[derive(Serialize, ToSchema)]
pub struct ServicePods {
    /// The service's current pods, by name.
    pub pods: Vec<HostPod>,
    /// CPU used per pod (`name`), in millicores.
    pub cpu: Vec<MetricSeries>,
    /// Memory working set per pod (`name`), in MiB.
    pub memory: Vec<MetricSeries>,
}

/// A pod the service's spans name (empty fields: not reported).
#[derive(clickhouse::Row, Deserialize, Clone, Debug, Default)]
pub(crate) struct SpanPod {
    pub cluster: String,
    pub namespace: String,
    pub name: String,
    pub uid: String,
}

/// One pod's usage in one bucket: CPU in cores and memory in bytes, -1 when not measured.
#[derive(clickhouse::Row, Deserialize, Clone, Debug)]
pub(crate) struct PodPoint {
    pub pod: String,
    pub t: i64,
    pub cpu: f64,
    pub mem: f64,
}

/// The inventory's pods that are `service`'s (in `env`, when given).
pub(crate) fn service_pods<'a>(rows: &'a [KubeResource], service: &str, env: Option<&str>, seen: &[SpanPod]) -> Vec<&'a KubeResource> {
    let tag = format!("service:{service}");
    let env = env.filter(|e| !e.is_empty() && *e != "*");
    // Pods tagged with another env aren't this env's; untagged ones may be.
    let in_env = |p: &KubeResource| {
        env.is_none_or(|e| {
            let envs: Vec<&str> = p.tags.iter().filter_map(|t| t.strip_prefix("env:")).collect();
            envs.is_empty() || envs.contains(&e)
        })
    };
    // Kubernetes Services named like the service: (cluster, namespace, selector labels).
    let selectors: Vec<Selector> = rows
        .iter()
        .filter(|r| r.kind == Some(KubeKind::Services) && r.name == service)
        .filter_map(|r| {
            let sel: Vec<(&str, &str)> = r.selector.as_deref()?.split(',').filter_map(|kv| kv.split_once('=')).collect();
            (!sel.is_empty()).then_some((r.cluster.as_str(), r.namespace.as_deref(), sel))
        })
        .collect();
    let selected = |p: &KubeResource| selectors.iter().any(|(c, ns, sel)| *c == p.cluster && *ns == p.namespace.as_deref() && sel.iter().all(|(k, v)| p.labels.get(*k).is_some_and(|x| x == v)));
    let spanned = |p: &KubeResource| {
        seen.iter().any(|s| {
            (!s.uid.is_empty() && s.uid == p.uid)
                || (!s.name.is_empty() && s.name == p.name && (s.namespace.is_empty() || Some(s.namespace.as_str()) == p.namespace.as_deref()) && (s.cluster.is_empty() || s.cluster == p.cluster))
        })
    };
    rows.iter().filter(|r| r.kind == Some(KubeKind::Pods)).filter(|p| ((p.tags.contains(&tag) || selected(p)) && in_env(p)) || spanned(p)).collect()
}

/// Per-pod CPU (millicores) and memory (MiB) series from the bucketed points.
pub(crate) fn pod_series(points: Vec<PodPoint>) -> (Vec<MetricSeries>, Vec<MetricSeries>) {
    let round1 = |v: f64| (v * 10.0).round() / 10.0;
    let (mut cpu, mut mem): (ByPod, ByPod) = Default::default();
    for p in points {
        if p.cpu.is_finite() && p.cpu >= 0.0 {
            cpu.entry(p.pod.clone()).or_default().push((p.t, round1(p.cpu * 1000.0)));
        }
        if p.mem.is_finite() && p.mem >= 0.0 {
            mem.entry(p.pod).or_default().push((p.t, round1(p.mem / MIB)));
        }
    }
    let series = |m: ByPod, unit: &str| {
        m.into_iter()
            .map(|(name, mut points)| {
                points.sort_by_key(|p| p.0);
                MetricSeries { name, points, unit: Some(unit.into()) }
            })
            .collect()
    };
    (series(cpu, "millicore"), series(mem, "mebibyte"))
}

/// The pods of a service and their CPU and memory over the range; none for a service outside Kubernetes.
#[utoipa::path(post, path = "/api/v1/apm/pods", operation_id = "apm_service_pods", request_body = ServicePodsRequest, responses((status = 200, body = ServicePods)), tag = "apm")]
pub async fn pods(State(st): State<AppState>, OrgId(org): OrgId, Json(req): Json<ServicePodsRequest>) -> ApiResult<ServicePods> {
    req.range.validate()?;
    let (clause, mut params) = super::apm::base(req.range, &req.env, "");
    params.push(Param::Str(req.service.clone()));
    let seen = bind(
        st.ch(&org).query(&format!(
            "SELECT DISTINCT ResourceAttributes['k8s.cluster.name'] AS cluster, ResourceAttributes['k8s.namespace.name'] AS namespace, \
             ResourceAttributes['k8s.pod.name'] AS name, ResourceAttributes['k8s.pod.uid'] AS uid FROM otel.otel_traces \
             WHERE OrgId = {{oc_org:String}} AND {clause} AND ServiceName = ? AND (ResourceAttributes['k8s.pod.name'] != '' OR ResourceAttributes['k8s.pod.uid'] != '') LIMIT 1000"
        )),
        &params,
    );
    let (seen, rows) = tokio::try_join!(async { seen.fetch_all::<SpanPod>().await.map_err(ApiError::from) }, super::kubernetes::resources(&st, &org))?;
    let mut matched: Vec<&KubeResource> = service_pods(&rows, &req.service, req.env.as_deref(), &seen);
    matched.sort_by(|a, b| a.name.cmp(&b.name).then(a.cluster.cmp(&b.cluster)));
    matched.truncate(MAX_PODS);
    let names: Vec<String> = matched.iter().map(|p| p.name.clone()).collect();
    let now = now_ms();
    let pods: Vec<HostPod> = matched.into_iter().map(|p| super::hosts::host_pod(p.clone(), now)).collect();
    if pods.is_empty() {
        return Ok(Json(ServicePods { pods, cpu: vec![], memory: vec![] }));
    }

    // Usage per container and bucket (the collectors' metrics averaged), summed per pod: the service's own (its
    // `service` tag on the Datadog Agent's metrics) or its pods' (by name).
    let src = points_subquery("gauge", USAGE_METRICS);
    let env = req.env.as_deref().filter(|e| !e.is_empty() && *e != "*");
    let env_clause = if env.is_some() { " AND ResourceAttributes['deployment.environment'] IN ('', ?)" } else { "" };
    let mut q = with_window(st.ch(&org), req.range.from, req.range.to)
        .query(&format!(
            "SELECT pod, t, if(countIf(cpu >= 0) > 0, sumIf(cpu, cpu >= 0), -1) AS cpu, if(countIf(mem >= 0) > 0, sumIf(mem, mem >= 0), -1) AS mem FROM (\
               SELECT ResourceAttributes['k8s.pod.name'] AS pod, ResourceAttributes['k8s.container.name'] AS container, \
                      toInt64(toUnixTimestamp(toStartOfInterval(TimeUnix, toIntervalSecond(?)))) * 1000 AS t, \
                      ifNotFinite(avgIf(if(MetricName = 'kubernetes.cpu.usage.total', Value / 1e9, Value), {POD_CPU}), -1) AS cpu, ifNotFinite(avgIf(Value, {IS_MEMORY}), -1) AS mem \
               FROM {src} WHERE OrgId = {{oc_org:String}} AND MetricName IN ({names}) AND pod != '' AND container != '' \
                 AND ((ServiceName = ?{env_clause}) OR pod IN ?) \
               GROUP BY pod, container, t) \
             GROUP BY pod, t ORDER BY pod, t LIMIT 100000",
            names = name_list(USAGE_METRICS),
        ))
        .bind(super::apm::bucket_ms(req.range, 60) / 1000)
        .bind(&req.service);
    if let Some(e) = env {
        q = q.bind(e);
    }
    let points = q.bind(&names).fetch_all::<PodPoint>().await?;
    let (cpu, memory) = pod_series(points);
    Ok(Json(ServicePods { pods, cpu, memory }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pod(cluster: &str, ns: &str, name: &str, tags: &[&str], labels: &[(&str, &str)]) -> KubeResource {
        KubeResource {
            uid: format!("{cluster}/{ns}/{name}"),
            kind: Some(KubeKind::Pods),
            cluster: cluster.into(),
            namespace: Some(ns.into()),
            name: name.into(),
            tags: tags.iter().map(|t| t.to_string()).collect(),
            labels: labels.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect::<BTreeMap<_, _>>(),
            ..Default::default()
        }
    }

    fn names(pods: Vec<&KubeResource>) -> Vec<&str> {
        pods.into_iter().map(|p| p.name.as_str()).collect()
    }

    #[test]
    fn pods_carrying_the_unified_service_tag_are_the_services() {
        let rows = vec![
            pod("shop-prod-eu", "checkout", "storefront-api-7d9-a", &["service:storefront-api", "env:prod"], &[]),
            pod("shop-staging", "checkout", "storefront-api-7d9-b", &["service:storefront-api", "env:staging"], &[]),
            pod("shop-prod-eu", "checkout", "order-service-5c7-a", &["service:order-service"], &[]),
            // A deployment tagged like its pods isn't a pod.
            KubeResource { kind: Some(KubeKind::Deployments), name: "storefront-api".into(), tags: vec!["service:storefront-api".into()], ..Default::default() },
        ];
        assert_eq!(names(service_pods(&rows, "storefront-api", None, &[])), vec!["storefront-api-7d9-a", "storefront-api-7d9-b"]);
        // An env narrows to the pods tagged with it (or with none).
        assert_eq!(names(service_pods(&rows, "storefront-api", Some("prod"), &[])), vec!["storefront-api-7d9-a"]);
        assert_eq!(names(service_pods(&rows, "order-service", Some("prod"), &[])), vec!["order-service-5c7-a"]);
        assert!(service_pods(&rows, "billing", None, &[]).is_empty());
    }

    #[test]
    fn pods_a_kubernetes_service_named_like_it_selects_are_the_services() {
        let svc = KubeResource {
            kind: Some(KubeKind::Services),
            cluster: "shop-prod-eu".into(),
            namespace: Some("checkout".into()),
            name: "storefront-api".into(),
            selector: Some("app=storefront,tier=web".into()),
            ..Default::default()
        };
        let rows = vec![
            svc,
            pod("shop-prod-eu", "checkout", "storefront-6f8-a", &[], &[("app", "storefront"), ("tier", "web"), ("pod-template-hash", "6f8")]),
            // Only some of the selector's labels.
            pod("shop-prod-eu", "checkout", "storefront-worker-a", &[], &[("app", "storefront")]),
            // Another namespace.
            pod("shop-prod-eu", "catalog", "storefront-6f8-b", &[], &[("app", "storefront"), ("tier", "web")]),
        ];
        assert_eq!(names(service_pods(&rows, "storefront-api", None, &[])), vec!["storefront-6f8-a"]);
    }

    #[test]
    fn pods_the_services_spans_name_are_the_services() {
        let rows = vec![
            pod("shop-prod-eu", "checkout", "storefront-6f8-a", &[], &[]),
            pod("shop-staging", "checkout", "storefront-6f8-a", &[], &[]),
            pod("shop-prod-eu", "checkout", "storefront-6f8-b", &[], &[]),
            pod("shop-prod-eu", "checkout", "billing-7b6-a", &[], &[]),
        ];
        // The Datadog Agent's container tags: cluster, namespace and pod name.
        let dd = SpanPod { cluster: "shop-prod-eu".into(), namespace: "checkout".into(), name: "storefront-6f8-a".into(), ..Default::default() };
        // OpenTelemetry's k8sattributes: the pod's uid.
        let otel = SpanPod { uid: "shop-prod-eu/checkout/storefront-6f8-b".into(), ..Default::default() };
        let mut found = names(service_pods(&rows, "storefront-api", None, &[dd, otel]));
        found.sort();
        assert_eq!(found, vec!["storefront-6f8-a", "storefront-6f8-b"]);
        // Without a cluster, a pod name matches in every cluster.
        let bare = SpanPod { name: "storefront-6f8-a".into(), ..Default::default() };
        assert_eq!(service_pods(&rows, "storefront-api", None, &[bare]).len(), 2);
    }

    #[test]
    fn pod_usage_series_are_millicores_and_mebibytes_per_pod() {
        let p = |pod: &str, t: i64, cpu: f64, mem: f64| PodPoint { pod: pod.into(), t, cpu, mem };
        let (cpu, mem) = pod_series(vec![
            p("storefront-6f8-a", 60_000, 0.2504, 268_435_456.0),
            p("storefront-6f8-a", 120_000, 0.3, -1.0),
            p("storefront-6f8-b", 60_000, -1.0, 104_857_600.0),
        ]);
        assert_eq!(cpu.len(), 1);
        assert_eq!((cpu[0].name.as_str(), cpu[0].points.clone(), cpu[0].unit.as_deref()), ("storefront-6f8-a", vec![(60_000, 250.4), (120_000, 300.0)], Some("millicore")));
        assert_eq!(mem.iter().map(|s| (s.name.as_str(), s.points.clone())).collect::<Vec<_>>(), vec![("storefront-6f8-a", vec![(60_000, 256.0)]), ("storefront-6f8-b", vec![(60_000, 100.0)])]);
        assert_eq!(mem[0].unit.as_deref(), Some("mebibyte"));
    }
}
