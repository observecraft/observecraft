// Mock logs of the Kubernetes integrations whose dashboards show logs (the control plane, Temporal, vLLM, Argo
// Workflows and Rollouts, Istio, Envoy), as the Datadog Agent collects them from their pods and the integrations'
// pipelines parse them: the source the pipeline reads, the status and attributes it sets. Over the last day, on the
// demo's clusters and hosts.
import type { JsonValue, LogEvent, LogStatus } from '../types';
import type { Rand } from './generator';

const HOUR = 3_600_000;
const HOSTS = ['i-0a1b2c3d4e5f60001', 'i-0a1b2c3d4e5f60002', 'i-0a1b2c3d4e5f60003', 'i-0a1b2c3d4e5f60006'];
const CLUSTERS = ['shop-prod-eks', 'shop-prod-eks', 'shop-prod-eks', 'shop-qa-eks'];

const pick = <T>(r: Rand, a: readonly T[]): T => a[Math.floor(r() * a.length)];

/** One kind of line: its status, message and parsed attributes, from a random pick of its values. */
interface Line {
  status: LogStatus;
  message: (r: Rand) => string;
  attributes?: (r: Rand, message: string) => Record<string, JsonValue>;
  /** How often it appears, relative to the integration's other lines (default 1). */
  weight?: number;
}

/** An integration's logs: `count` lines of `lines` from its pods. */
interface Source {
  source: string;
  service: string | ((r: Rand) => string);
  namespace: string;
  pod: (i: number) => string;
  count: number;
  lines: Line[];
}

const KLOG = { I: 'info', W: 'warn', E: 'error' } as const;
/** A klog line (`I1110 09:15:02.123456 1 file.go:42] text`) as the pipelines leave it: the text as the message,
 * the level letter, the file and line. */
const klogLine = (level: keyof typeof KLOG, at: string, text: (r: Rand) => string, weight = 1): Line => ({
  status: KLOG[level],
  message: text,
  attributes: () => ({ level, logger: { name: at.split(':')[0], thread_id: 1 }, lineno: Number(at.split(':')[1]) }),
  weight,
});

const RESOURCES = ['pods', 'deployments', 'configmaps', 'leases', 'endpointslices'];
const APPS = ['storefront-api', 'order-service', 'billing', 'catalog-service', 'inventory-service'];
const NAMESPACES = ['storefront', 'checkout', 'billing', 'catalog'];

const SOURCES: Source[] = [
  {
    source: 'kube-apiserver',
    service: 'kube-apiserver',
    namespace: 'kube-system',
    pod: (i) => `kube-apiserver-ip-10-0-${(i % 3) + 1}-10`,
    count: 300,
    lines: [
      klogLine('I', 'controller.go:615', () => 'quota admission added evaluator for: leases.coordination.k8s.io', 3),
      klogLine('I', 'alloc.go:330', (r) => `"allocated clusterIPs" service="${pick(r, NAMESPACES)}/${pick(r, APPS)}" clusterIPs={"IPv4":"172.20.${Math.floor(r() * 200)}.${Math.floor(r() * 250)}"}`),
      klogLine('W', 'dispatcher.go:217', () => 'Failed calling webhook, failing open validation.gatekeeper.sh: failed calling webhook "validation.gatekeeper.sh": context deadline exceeded'),
      klogLine('E', 'status.go:71', (r) => `apiserver received an error that is not an metav1.Status: context canceled: context canceled verb="GET" resource="${pick(r, RESOURCES)}"`),
      klogLine(
        'I',
        'trace.go:236',
        (r) =>
          `Trace[${Math.floor(r() * 1e9)}]: "List" accept:application/json,audit-id:${Math.floor(r() * 1e8).toString(16)} (10-Nov-2026 09:15:01.880) (total time: ${600 + Math.floor(r() * 900)}ms)`,
      ),
    ],
  },
  {
    source: 'kube-scheduler',
    service: 'kube-scheduler',
    namespace: 'kube-system',
    pod: (i) => `kube-scheduler-ip-10-0-${(i % 3) + 1}-10`,
    count: 200,
    lines: [
      klogLine(
        'I',
        'schedule_one.go:286',
        (r) =>
          `"Successfully bound pod to node" pod="${pick(r, NAMESPACES)}/${pick(r, APPS)}-7d9c6b5f4-${Math.floor(r() * 46656).toString(36)}" node="ip-10-0-${Math.floor(r() * 3) + 1}-${10 + Math.floor(r() * 40)}.ec2.internal" evaluatedNodes=6 feasibleNodes=4`,
        6,
      ),
      klogLine(
        'I',
        'schedule_one.go:1050',
        (r) =>
          `"Unable to schedule pod; no fit; waiting" pod="${pick(r, NAMESPACES)}/${pick(r, APPS)}-batch" err="0/6 nodes are available: 2 Insufficient cpu, 4 node(s) didn't match Pod's node affinity/selector."`,
      ),
      klogLine('I', 'leaderelection.go:260', () => 'successfully acquired lease kube-system/kube-scheduler'),
      klogLine('E', 'reflector.go:150', () => 'k8s.io/client-go/informers/factory.go:160: Failed to watch *v1.PersistentVolumeClaim: the server has asked for the client to provide credentials'),
    ],
  },
  {
    source: 'kube-controller-manager',
    service: 'kube-controller-manager',
    namespace: 'kube-system',
    pod: (i) => `kube-controller-manager-ip-10-0-${(i % 3) + 1}-10`,
    count: 200,
    lines: [
      klogLine(
        'I',
        'event.go:389',
        (r) =>
          `"Event occurred" object="${pick(r, NAMESPACES)}/${pick(r, APPS)}" fieldPath="" kind="Deployment" apiVersion="apps/v1" type="Normal" reason="ScalingReplicaSet" message="Scaled up replica set ${pick(r, APPS)}-7d9c6b5f4 to ${2 + Math.floor(r() * 4)}"`,
        5,
      ),
      klogLine(
        'I',
        'horizontal.go:842',
        (r) =>
          `"Successful rescale" HPA="${pick(r, NAMESPACES)}/${pick(r, APPS)}" currentReplicas=3 desiredReplicas=${4 + Math.floor(r() * 3)} reason="cpu resource utilization (percentage of request) above target"`,
        2,
      ),
      klogLine('W', 'garbagecollector.go:818', () => 'failed to discover some groups: map[external.metrics.k8s.io/v1beta1:stale GroupVersion discovery: external.metrics.k8s.io/v1beta1]'),
      klogLine('E', 'job_controller.go:597', (r) => `"Unhandled Error" err="syncing job: failed to delete pod ${pick(r, NAMESPACES)}/${pick(r, APPS)}-migrate: pods not found"`),
    ],
  },
];

/** Every integration's logs over the last day. */
export function kubernetesIntegrationLogs(r: Rand, now: number): LogEvent[] {
  const out: LogEvent[] = [];
  for (const s of SOURCES) {
    const total = s.lines.reduce((n, l) => n + (l.weight ?? 1), 0);
    for (let i = 0; i < s.count; i++) {
      let x = r() * total;
      const line = s.lines.find((l) => (x -= l.weight ?? 1) < 0) ?? s.lines[0];
      const ts = Math.round(now - Math.pow(r(), 1.2) * 24 * HOUR);
      const n = Math.floor(r() * HOSTS.length);
      const message = line.message(r);
      const service = typeof s.service === 'string' ? s.service : s.service(r);
      out.push({
        id: `${s.source}-${ts}-${i}`,
        timestamp: ts,
        status: line.status,
        service,
        host: HOSTS[n],
        source: s.source,
        message,
        tags: { env: CLUSTERS[n].includes('prod') ? 'prod' : 'qa', kube_cluster_name: CLUSTERS[n], kube_namespace: s.namespace, pod_name: s.pod(n) },
        attributes: line.attributes?.(r, message) ?? {},
      });
    }
  }
  return out;
}
