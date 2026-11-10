// Mock metrics of the Kubernetes integrations the mock shows installed (ingress-nginx, CoreDNS, KEDA, Karpenter,
// Temporal, vLLM, DCGM, the control plane, Argo Workflows and Rollouts, Istio, Envoy): their tags' values and the values
// that read as theirs (object counts, 0/1 states, versions), on the demo's clusters. Read by coreMetrics.ts.

type Tags = Record<string, string>;
type Rand = () => number;
type Override = { base: (e: Tags, r: Rand) => number; noise: number; integer?: boolean; max?: number };

const HOSTS = ['i-0a1b2c3d4e5f60001', 'i-0a1b2c3d4e5f60002', 'i-0a1b2c3d4e5f60003', 'i-0a1b2c3d4e5f60006'];
const CLUSTER = ['shop-prod-eks', 'shop-prod-eks', 'shop-prod-eks', 'shop-qa-eks'];

/** A value between `lo` and `hi` per instance, varying by `noise`. */
const between = (lo: number, hi: number, noise = 0.15, integer = false): Override => ({ base: (_e, r) => lo + r() * (hi - lo), noise, ...(integer ? { integer } : {}) });
/** Always `v` (versions, info gauges, configured limits). */
const fixed = (v: number): Override => ({ base: () => v, noise: 0, integer: Number.isInteger(v) });
/** A typical value per value of a tag (response codes, phases), 0 for the others. */
const byTag = (tag: string, values: Record<string, number>, noise = 0.2, integer = false): Override => ({ base: (e) => values[e[tag]] ?? 0, noise, ...(integer ? { integer } : {}) });
/** Usually 0, sometimes up to `hi` (errors, retries, restarts). */
const rare = (hi: number, share = 0.25): Override => ({ base: (_e, r) => (r() < share ? 1 + r() * (hi - 1) : 0), noise: 0.6, integer: true });

/** The control plane's tags (kube-apiserver, kube-scheduler, kube-controller-manager). */
const CONTROL_PLANE = {
  kube_cluster_name: CLUSTER,
  pod_name: ['kube-apiserver-ip-10-0-1-10', 'kube-apiserver-ip-10-0-2-10', 'kube-apiserver-ip-10-0-3-10', 'kube-apiserver-ip-10-0-1-10'],
  code: ['200', '201', '200', '404'],
  method: ['GET', 'PUT', 'PATCH', 'POST'],
  verb: ['list', 'get', 'watch', 'patch'],
};

export const INFRA_VALUES: Record<string, Record<string, string[]>> = {
  'nginx-ingress-controller': {
    state: ['active', 'reading', 'writing', 'waiting'],
    controller_pod: ['ingress-nginx-controller-7c9d8b6f5-4qmzx', 'ingress-nginx-controller-7c9d8b6f5-8hjkt', 'ingress-nginx-controller-7c9d8b6f5-r2vnc', 'ingress-nginx-controller-5b8f7d9c4-x7lpw'],
  },
  'kube-apiserver-metrics': {
    ...CONTROL_PLANE,
    verb: ['LIST', 'GET', 'WATCH', 'PATCH'],
    kind: ['Pod', 'ConfigMap', 'Lease', 'EndpointSlice'],
    resource: ['pods', 'configmaps', 'leases', 'events'],
    scope: ['cluster', 'namespace', 'resource', 'namespace'],
    grpc_code: ['OK', 'OK', 'OK', 'Canceled'],
    grpc_service: ['etcdserverpb.KV', 'etcdserverpb.Watch', 'etcdserverpb.Lease', 'etcdserverpb.KV'],
    grpc_method: ['Range', 'Watch', 'LeaseKeepAlive', 'Txn'],
    name: ['SidecarContainers', 'InPlacePodVerticalScaling', 'JobPodReplacementPolicy', 'DynamicResourceAllocation'],
    stage: ['BETA', 'ALPHA', 'BETA', 'ALPHA'],
    kube_namespace: ['kube-system', 'keda', 'kube-system', 'keda'],
    apiservice_name: ['v1beta1.metrics.k8s.io', 'v1beta1.external.metrics.k8s.io', 'v1beta1.metrics.k8s.io', 'v1beta1.external.metrics.k8s.io'],
  },
  'kube-scheduler': {
    ...CONTROL_PLANE,
    pod_name: ['kube-scheduler-ip-10-0-1-10', 'kube-scheduler-ip-10-0-2-10', 'kube-scheduler-ip-10-0-3-10', 'kube-scheduler-ip-10-0-1-10'],
    verb: ['POST', 'GET', 'PATCH', 'PUT'],
    url: [
      'https://172.20.0.1:443/api/v1/namespaces/%7Bnamespace%7D/pods/%7Bname%7D/binding',
      'https://172.20.0.1:443/apis/coordination.k8s.io/v1/namespaces/kube-system/leases/kube-scheduler',
      'https://172.20.0.1:443/api/v1/namespaces/%7Bnamespace%7D/pods/%7Bname%7D/status',
      'https://172.20.0.1:443/api/v1/namespaces/%7Bnamespace%7D/events',
    ],
    result: ['scheduled', 'scheduled', 'unschedulable', 'error'],
    profile: ['default-scheduler', 'default-scheduler', 'default-scheduler', 'default-scheduler'],
    queue: ['active', 'backoff', 'unschedulable', 'gated'],
    event: ['PodAdd', 'ScheduleAttemptFailure', 'NodeAdd', 'AssignedPodDelete'],
  },
  'kube-controller-manager': {
    ...CONTROL_PLANE,
    pod_name: ['kube-controller-manager-ip-10-0-1-10', 'kube-controller-manager-ip-10-0-2-10', 'kube-controller-manager-ip-10-0-3-10', 'kube-controller-manager-ip-10-0-1-10'],
    queue: ['deployment', 'replicaset', 'endpoint_slice', 'horizontalpodautoscaler'],
    limiter: ['node_lifecycle_controller', 'endpoint_slice_controller', 'job_controller', 'cronjob_controller'],
  },
  coredns: {
    kube_cluster_name: CLUSTER,
    kube_deployment: ['coredns', 'coredns', 'coredns', 'coredns'],
    kube_service: ['kube-dns', 'kube-dns', 'kube-dns', 'kube-dns'],
    kube_namespace: ['kube-system', 'kube-system', 'kube-system', 'kube-system'],
    node: ['ip-10-0-1-23.ec2.internal', 'ip-10-0-2-41.ec2.internal', 'ip-10-0-3-17.ec2.internal', 'ip-10-1-1-12.ec2.internal'],
    label: ['kube-dns', 'kube-dns', 'kube-dns', 'kube-dns'],
    type: ['success', 'denial', 'success', 'denial'],
    rcode: ['NOERROR', 'NXDOMAIN', 'NOERROR', 'SERVFAIL'],
  },
  keda: {
    kube_cluster_name: CLUSTER,
    namespace: ['checkout', 'storefront', 'billing', 'catalog'],
    scaler: ['redisScaler', 'prometheusScaler', 'redisScaler', 'cronScaler'],
    scaledobject: ['order-worker', 'storefront-api', 'billing-worker', 'catalog-indexer'],
    type: ['redis', 'prometheus', 'redis', 'cron'],
    triggerindex: ['0', '0', '0', '1'],
    metric: ['s0-redis-orders', 's0-prometheus-http_requests', 's0-redis-invoices', 's1-cron-Europe-Berlin-08xx1-5-18xx1-5'],
    scaledjob: ['image-resize', 'nightly-export', 'image-resize', 'nightly-export'],
    controller: ['scaledobject', 'scaledjob', 'triggerauthentication', 'scaledobject'],
    resource: ['order-worker', 'storefront-api', 'billing-worker', 'catalog-indexer'],
    grpc_code: ['OK', 'OK', 'OK', 'Unavailable'],
    grpc_service: ['api.MetricsService', 'api.MetricsService', 'api.MetricsService', 'api.MetricsService'],
    grpc_method: ['GetMetrics', 'GetMetrics', 'GetMetrics', 'GetMetrics'],
    grpc_type: ['unary', 'unary', 'unary', 'unary'],
  },
};

export const INFRA_OVERRIDES: Record<string, Override> = {
  'nginx_ingress.nginx.connections.current': between(40, 160, 0.2, true),
  'nginx_ingress.nginx.process.count': fixed(5),
  'nginx_ingress.nginx.bytes.read': between(4e6, 40e6, 0.3),
  'nginx_ingress.nginx.bytes.write': between(30e6, 300e6, 0.3),
  'nginx_ingress.nginx.mem.resident': between(90e6, 240e6, 0.05),
  'nginx_ingress.nginx.mem.virtual': between(420e6, 680e6, 0.02),
  'nginx_ingress.nginx.cpu.time': between(0.05, 0.4, 0.3),
  'nginx_ingress.controller.cpu.time': between(0.01, 0.08, 0.3),
  'nginx_ingress.controller.mem.resident': between(60e6, 140e6, 0.05),
  'nginx_ingress.controller.mem.virtual': between(760e6, 1.3e9, 0.02),
  'nginx_ingress.controller.reload.success': rare(2, 0.1),
  'kube_scheduler.pod_preemption.attempts': rare(3, 0.1),
  'kube_scheduler.pod_preemption.victims.sum': rare(2, 0.05),
  'kube_scheduler.client.http.requests': between(2, 60, 0.3),
  'kube_scheduler.schedule_attempts': between(1, 25, 0.4),
  'kube_scheduler.scheduling.attempt_duration.sum': between(0.004, 0.06, 0.3),
  'kube_scheduler.scheduling.algorithm_duration.sum': between(0.001, 0.02, 0.3),
  'kube_scheduler.scheduling.algorithm_duration.count': between(1, 25, 0.4),
  'kube_scheduler.pending_pods': { base: (e, r) => (e.queue === 'unschedulable' ? 1 + r() * 3 : e.queue === 'backoff' && r() < 0.4 ? 1 : 0), noise: 0.3, integer: true },
  'kube_scheduler.queue.incoming_pods': between(0, 12, 0.5, true),
  'kube_scheduler.threads': between(12, 20, 0.05, true),
  'kube_scheduler.max_fds': fixed(1048576),
  'kube_scheduler.open_fds': between(14, 30, 0.05, true),
  'kube_scheduler.goroutines': between(160, 320, 0.05, true),
  'kube_controller_manager.queue.depth': rare(6, 0.3),
  'kube_controller_manager.queue.adds': between(4, 180, 0.4),
  'kube_controller_manager.queue.retries': rare(5, 0.2),
  'kube_controller_manager.queue.work_longest_duration': between(0, 0.4, 0.5),
  'kube_controller_manager.queue.work_unfinished_duration': between(0, 0.6, 0.5),
  'kube_controller_manager.client.http.requests': between(10, 240, 0.3),
  'kube_controller_manager.rate_limiter.use': between(0.001, 0.06, 0.4),
  'kube_controller_manager.threads': between(14, 24, 0.05, true),
  'kube_controller_manager.max_fds': fixed(1048576),
  'kube_controller_manager.open_fds': between(18, 40, 0.05, true),
  'kube_controller_manager.goroutines': between(900, 1800, 0.05, true),
  // Most answers come from the cache; most of the rest are the search domains' NXDOMAINs.
  'coredns.cache_hits_count': { base: (e, r) => (e.type === 'denial' ? 60 : 420) * (0.7 + r() * 0.6), noise: 0.2 },
  'coredns.cache_hits_count.count': { base: (e, r) => (e.type === 'denial' ? 60 : 420) * (0.7 + r() * 0.6), noise: 0.2 },
  'coredns.cache_misses_count': between(20, 70, 0.3),
  'coredns.cache_misses_count.count': between(20, 70, 0.3),
  'coredns.cache_size.count': { base: (e, r) => (e.type === 'denial' ? 40 + r() * 120 : 300 + r() * 1200), noise: 0.05, integer: true },
  'coredns.process.cpu_seconds_total': between(0.05, 0.3, 0.3),
  'coredns.process.max_fds': fixed(1048576),
  'coredns.process.open_fds': between(14, 28, 0.05, true),
  'coredns.go.memstats.alloc_bytes': between(9e6, 22e6, 0.15),
  'coredns.go.memstats.heap_alloc_bytes': between(9e6, 22e6, 0.15),
  'coredns.request_duration.seconds.sum': between(0.6, 2.4, 0.3),
  'coredns.request_duration.seconds.count': between(500, 1400, 0.2),
  'coredns.forward_request_duration.seconds.sum': between(0.6, 2.8, 0.3),
  'coredns.forward_request_duration.seconds.count': between(60, 200, 0.2),
  'coredns.forward_response_rcode_count': byTag('rcode', { NOERROR: 120, NXDOMAIN: 30, SERVFAIL: 1 }, 0.3),
  'coredns.forward_response_rcode_count.count': byTag('rcode', { NOERROR: 120, NXDOMAIN: 30, SERVFAIL: 1 }, 0.3),
  'coredns.response_code_count': byTag('rcode', { NOERROR: 700, NXDOMAIN: 260, SERVFAIL: 2 }, 0.25),
  'coredns.response_code_count.count': byTag('rcode', { NOERROR: 700, NXDOMAIN: 260, SERVFAIL: 2 }, 0.25),
  'keda.go.info': fixed(1),
  'keda.scaler.active': { base: (e) => (e.type === 'cron' ? 0 : 1), noise: 0, integer: true },
  'keda.resource_totals': between(2, 6, 0, true),
  'keda.trigger_totals': between(1, 4, 0, true),
  'keda.scaler.metrics_value': { base: (e, r) => (e.type === 'cron' ? 0 : e.type === 'prometheus' ? 40 + r() * 200 : r() * 90), noise: 0.4 },
  'keda.scaler.metrics_latency_seconds': between(0.002, 0.03, 0.4),
  'keda.scaler.detail_errors.count': rare(3, 0.08),
  'keda.scaled_job.errors.count': rare(2, 0.05),
  'keda.scaled_object.errors.count': rare(2, 0.05),
  'keda.controller.runtime.active_workers': rare(2, 0.3),
  'keda.controller.runtime.reconcile_errors.count': rare(2, 0.05),
  'keda.controller.runtime.reconcile_time.seconds.sum': between(0.01, 0.2, 0.4),
  'keda.controller.runtime.reconcile_time.seconds.count': between(2, 20, 0.4),
  'keda.workqueue.adds.count': between(1, 20, 0.4),
  'keda.workqueue.depth': rare(3, 0.15),
  'keda.workqueue.retries.count': rare(2, 0.1),
  'keda.workqueue.unfinished_work.seconds': fixed(0),
  'keda.workqueue.work.duration.seconds.sum': between(0.005, 0.1, 0.4),
  'keda.workqueue.work.duration.seconds.count': between(1, 20, 0.4),
  'keda.workqueue.queue.duration.seconds.sum': between(0.0002, 0.005, 0.4),
  'keda.workqueue.queue.duration.seconds.count': between(1, 20, 0.4),
  'keda.internal_scale.loop_latency_seconds': between(0.0001, 0.004, 0.5),
  'keda.internal_metricsservice.grpc_server_handled.count': { base: (e, r) => (e.grpc_code === 'OK' ? 60 + r() * 180 : r() < 0.2 ? 1 : 0), noise: 0.3, integer: true },
  'keda.internal_metricsservice.grpc_server_msg_received.count': between(60, 240, 0.3),
  'keda.internal_metricsservice.grpc_server_msg_sent.count': between(60, 240, 0.3),
  'keda.go.goroutines': between(140, 380, 0.05, true),
  'keda.go.memstats.alloc_bytes': between(18e6, 60e6, 0.15),
  'keda.process.cpu.seconds.count': between(0.02, 0.25, 0.3),
  'kube_apiserver.registered_watchers': between(5, 180, 0.05, true),
  'kube_apiserver.process_virtual_memory_bytes': between(1.6e9, 2.1e9, 0.01),
  'kube_apiserver.process_resident_memory_bytes': between(650e6, 1.2e9, 0.03),
  'kube_apiserver.audit_event.count': between(40, 260, 0.25),
  'kube_apiserver.aggregator_unavailable_apiservice': fixed(0),
  'kube_apiserver.rest_client_requests_total.count': between(2, 40, 0.3),
  'kube_apiserver.storage_objects': between(40, 900, 0.01, true),
  'kube_apiserver.go_threads': between(18, 32, 0.05, true),
  'kube_apiserver.go_goroutines': between(2200, 5200, 0.05, true),
  'kube_apiserver.apiserver_request_total.count': between(15, 320, 0.25),
  'kube_apiserver.current_inflight_requests': between(1, 12, 0.5, true),
  'kube_apiserver.longrunning_gauge': between(40, 380, 0.05, true),
  'kube_apiserver.grpc_client_handled_total': between(20, 400, 0.25),
  'kube_apiserver.apiserver_admission_webhook_fail_open_count.count': rare(3, 0.1),
  'kube_apiserver.admission_webhook_admission_latencies_seconds.sum': between(0.02, 0.25, 0.3),
  // On for the beta gates, off for the alpha ones.
  'kube_apiserver.kubernetes_feature_enabled': { base: (e) => (e.stage === 'ALPHA' ? 0 : 1), noise: 0, integer: true },
};

/** Pods of the Kubernetes integrations whose dashboards chart their containers (`short_image:kube-apiserver`,
 * Istio's `short_image:pilot`): the control plane's static pods and Istio's, on the demo's clusters.
 * [short image, container, namespace, pods, CPU request (cores), CPU used (millicores), memory request and used (MiB)]. */
const INFRA_CONTAINERS: [image: string, container: string, namespace: string, pods: string[], cpu: number, used: number, memory: number, rss: number][] = [
  ['kube-apiserver', 'kube-apiserver', 'kube-system', ['kube-apiserver-ip-10-0-1-10', 'kube-apiserver-ip-10-0-2-10', 'kube-apiserver-ip-10-0-3-10'], 0.25, 420, 512, 1400],
  ['kube-scheduler', 'kube-scheduler', 'kube-system', ['kube-scheduler-ip-10-0-1-10', 'kube-scheduler-ip-10-0-2-10', 'kube-scheduler-ip-10-0-3-10'], 0.1, 18, 64, 90],
  [
    'kube-controller-manager',
    'kube-controller-manager',
    'kube-system',
    ['kube-controller-manager-ip-10-0-1-10', 'kube-controller-manager-ip-10-0-2-10', 'kube-controller-manager-ip-10-0-3-10'],
    0.2,
    65,
    128,
    210,
  ],
  ['pilot', 'discovery', 'istio-system', ['istiod-7d9c6b5f4-2xkqp', 'istiod-7d9c6b5f4-9tmwz'], 0.5, 140, 2048, 420],
  ['proxyv2', 'istio-proxy', 'storefront', ['storefront-api-6f8b9c7d5-4kzxq', 'order-service-5c7d8b6f9-h2lmn', 'billing-7b6c5d4f8-p9wvx', 'catalog-service-84f7c6d59-tq3rs'], 0.1, 35, 128, 80],
];
export const INFRA_PODS: Tags[] = INFRA_CONTAINERS.flatMap(([image, container, namespace, pods]) =>
  pods.map((pod, i) => ({
    short_image: image,
    kube_container_name: container,
    container_name: container,
    kube_namespace: namespace,
    pod_name: pod,
    host: HOSTS[i % HOSTS.length],
    kube_cluster_name: CLUSTER[i % 3],
    cluster_name: CLUSTER[i % 3],
  })),
);
const sizes = (e: Tags) => INFRA_CONTAINERS.find(([image]) => image === e.short_image);
/** Their Kubernetes container metrics (the mock engine's `kubernetes.*` on its deployments, these pods added). */
export const INFRA_POD_VALUES: Record<string, (e: Tags, r: () => number) => number | undefined> = {
  'kubernetes.cpu.requests': (e) => sizes(e)?.[4],
  'kubernetes.cpu.usage.total': (e, r) => (sizes(e)?.[5] ?? 0) * (0.8 + r() * 0.4),
  'kubernetes.memory.requests': (e) => sizes(e)?.[6],
  'kubernetes.memory.usage': (e, r) => (sizes(e)?.[7] ?? 0) * (0.9 + r() * 0.2),
  'kubernetes.containers.running': () => 1,
};
