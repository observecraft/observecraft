import type { MouseEvent, ReactNode } from 'react';
import type { KubeHealth, KubeKind, KubeResource, KubeUtilization } from '../../api/kubernetes/client';
import { formatBytes } from '../../components/ui/Pager';

/** Explorer resources: processes, containers, Kubernetes and ECS kinds. */
export type ExplorerKind = KubeKind;

export interface CellContext {
  /** Adds `key:value` to the explorer's filter. */
  filter: (key: string, value: string) => void;
}

export interface KubeColumn {
  key: string;
  label: string;
  /** Server sort field; unsortable when absent. */
  sort?: string;
  num?: boolean;
  render: (r: KubeResource, ctx: CellContext) => ReactNode;
}

interface KindInfo {
  kind: ExplorerKind;
  /** Plural, as in "Showing 1–50 of 5K Pods". */
  label: string;
  singular: string;
}

export interface TreeNode {
  label: string;
  kind?: ExplorerKind;
  children?: TreeNode[];
}

/** The "Select Resources" tree of Datadog's Kubernetes and ECS Explorers. */
export const RESOURCE_TREE: TreeNode[] = [
  { label: 'Processes', kind: 'processes' },
  { label: 'Containers', kind: 'containers' },
  { label: 'Container Images', kind: 'container_images' },
  {
    label: 'Kubernetes',
    children: [
      { label: 'Pods', kind: 'pods' },
      { label: 'Clusters', kind: 'clusters' },
      { label: 'Namespaces', kind: 'namespaces' },
      { label: 'Nodes', kind: 'nodes' },
      {
        label: 'Workloads',
        children: [
          { label: 'Deployments', kind: 'deployments' },
          { label: 'Replica Sets', kind: 'replicasets' },
          { label: 'Daemon Sets', kind: 'daemonsets' },
          { label: 'Stateful Sets', kind: 'statefulsets' },
          { label: 'Jobs', kind: 'jobs' },
          { label: 'Cron Jobs', kind: 'cronjobs' },
        ],
      },
      {
        label: 'Network',
        children: [
          { label: 'Services', kind: 'services' },
          { label: 'Ingresses', kind: 'ingresses' },
        ],
      },
      {
        label: 'Policies',
        children: [
          { label: 'Network Policies', kind: 'networkpolicies' },
          { label: 'Pod Disruption Budgets', kind: 'poddisruptionbudgets' },
          { label: 'Limit Ranges', kind: 'limitranges' },
          { label: 'Resource Quotas', kind: 'resourcequotas' },
        ],
      },
      {
        label: 'Storage',
        children: [
          { label: 'Persistent Volume Claims', kind: 'persistentvolumeclaims' },
          { label: 'Persistent Volumes', kind: 'persistentvolumes' },
          { label: 'Storage Classes', kind: 'storageclasses' },
        ],
      },
      {
        label: 'Access Control',
        children: [
          { label: 'Service Accounts', kind: 'serviceaccounts' },
          { label: 'Roles', kind: 'roles' },
          { label: 'Role Bindings', kind: 'rolebindings' },
          { label: 'Cluster Roles', kind: 'clusterroles' },
          { label: 'Cluster Role Bindings', kind: 'clusterrolebindings' },
        ],
      },
      { label: 'Autoscaling', children: [{ label: 'Horizontal Pod Autoscalers', kind: 'horizontalpodautoscalers' }] },
      { label: 'Custom Resources', children: [{ label: 'CRDs', kind: 'customresourcedefinitions' }] },
    ],
  },
  {
    label: 'ECS',
    children: [
      { label: 'Tasks', kind: 'ecs_tasks' },
      { label: 'Task Definitions', kind: 'ecs_task_definitions' },
      { label: 'Services', kind: 'ecs_services' },
      { label: 'Clusters', kind: 'ecs_clusters' },
    ],
  },
];

const SINGULAR: Record<ExplorerKind, string> = {
  processes: 'Process',
  containers: 'Container',
  container_images: 'Container Image',
  pods: 'Pod',
  clusters: 'Cluster',
  namespaces: 'Namespace',
  nodes: 'Node',
  deployments: 'Deployment',
  replicasets: 'Replica Set',
  daemonsets: 'Daemon Set',
  statefulsets: 'Stateful Set',
  jobs: 'Job',
  cronjobs: 'Cron Job',
  services: 'Service',
  ingresses: 'Ingress',
  networkpolicies: 'Network Policy',
  poddisruptionbudgets: 'Pod Disruption Budget',
  limitranges: 'Limit Range',
  resourcequotas: 'Resource Quota',
  persistentvolumeclaims: 'Persistent Volume Claim',
  persistentvolumes: 'Persistent Volume',
  storageclasses: 'Storage Class',
  serviceaccounts: 'Service Account',
  roles: 'Role',
  rolebindings: 'Role Binding',
  clusterroles: 'Cluster Role',
  clusterrolebindings: 'Cluster Role Binding',
  horizontalpodautoscalers: 'Horizontal Pod Autoscaler',
  customresourcedefinitions: 'CRD',
  ecs_tasks: 'Task',
  ecs_task_definitions: 'Task Definition',
  ecs_services: 'Service',
  ecs_clusters: 'Cluster',
};

const plural = (s: string) => (s.endsWith('y') ? `${s.slice(0, -1)}ies` : s.endsWith('ss') ? `${s}es` : `${s}s`);

export const KIND_INFO = Object.fromEntries(Object.entries(SINGULAR).map(([k, s]) => [k, { kind: k, singular: s, label: k === 'customresourcedefinitions' ? 'CRDs' : plural(s) }])) as Record<
  ExplorerKind,
  KindInfo
>;

export const isEcs = (k: ExplorerKind) => k.startsWith('ecs_');

/** Display names of tag keys (facets, group-by). */
export const TAG_LABELS: Record<string, string> = {
  kube_cluster_name: 'Cluster',
  kube_namespace: 'Namespace',
  status: 'Status',
  kube_node: 'Node',
  kube_ownerref_kind: 'Owner Kind',
  kube_deployment: 'Deployment',
  kube_daemon_set: 'Daemon Set',
  kube_stateful_set: 'Stateful Set',
  kube_job: 'Job',
  kube_cronjob: 'Cron Job',
  kube_qos: 'QoS Class',
  pod_name: 'Pod',
  service: 'Service',
  env: 'Env',
  short_image: 'Image',
  image_tag: 'Image Tag',
  kube_node_role: 'Node Role',
  kubelet_version: 'Kubelet Version',
  os: 'OS',
  kube_service_type: 'Service Type',
  kube_storage_class: 'Storage Class',
  task_health: 'Task Health',
  task_launch_type: 'Task Launch Type',
  task_family: 'Task Family',
  ecs_cluster: 'ECS Cluster',
  ecs_service: 'ECS Service',
  region: 'Region',
  command: 'Command',
  user: 'User',
  host: 'Host',
  container_id: 'Container',
};

/** Facets about the resource itself; the rest describe what it belongs to. */
const CONDITION_KEYS = new Set([
  'status',
  'kube_qos',
  'kube_ownerref_kind',
  'task_health',
  'task_launch_type',
  'kube_service_type',
  'kube_storage_class',
  'kube_node_role',
  'kubelet_version',
  'os',
  'short_image',
  'image_tag',
  'command',
  'user',
]);
export const facetGroup = (key: string) => (CONDITION_KEYS.has(key) ? 'CONDITIONS' : 'RELATIONS');

/** Cores: `250m`, `1.5`; tiny values stay readable. */
export function formatCores(c: number | null | undefined): string {
  if (c == null) return '—';
  if (c === 0) return '0';
  if (c < 0.001) return '<1m';
  if (c < 1) return `${Math.round(c * 1000)}m`;
  return c.toFixed(c < 10 ? 2 : 1);
}

/** Axis labels keep sub-millicore precision. */
export function formatCoresAxis(c: number): string {
  return c > 0 && c < 0.001 ? `${(c * 1000).toFixed(2)}m` : formatCores(c);
}

export function formatMem(b: number | null | undefined): string {
  return b == null ? '—' : formatBytes(b);
}

/** Datadog's ages: "21 minutes", "6 days", "7 months", "3 years". */
export function age(ts: number | null | undefined): string {
  if (ts == null) return '—';
  const s = Math.max(0, (Date.now() - ts) / 1000);
  const unit = (n: number, u: string) => `${n} ${u}${n === 1 ? '' : 's'}`;
  if (s < 3600) return unit(Math.max(1, Math.round(s / 60)), 'minute');
  if (s < 86400) return unit(Math.round(s / 3600), 'hour');
  if (s < 30 * 86400) return unit(Math.round(s / 86400), 'day');
  if (s < 365 * 86400) return unit(Math.round(s / (30 * 86400)), 'month');
  return unit(Math.round(s / (365 * 86400)), 'year');
}

export function StatusPill({ status, health }: { status: string; health: KubeHealth }) {
  return <span className={`ox-status ox-${health}`}>{(status || '—').toUpperCase()}</span>;
}

/** A tag value shown as a chip; clicking filters the explorer by it. */
export function Chip({ k, v, ctx }: { k: string; v: string | null | undefined; ctx: CellContext }) {
  if (!v) return <span className="muted">—</span>;
  return (
    <button
      className="ox-chip"
      title={`Filter by ${k}:${v}`}
      onClick={(e: MouseEvent) => {
        e.stopPropagation();
        ctx.filter(k, v);
      }}
    >
      {v}
    </button>
  );
}

/** Usage as a share of limits (Datadog's CPU/MEM USAGE/LIMITS), with its bar. */
export function PctOfLimits({ u }: { u: KubeUtilization | null | undefined }) {
  if (u?.usage == null || !u.limits) return <span className="muted">—</span>;
  const pct = (u.usage / u.limits) * 100;
  return (
    <span className="ox-pct" title={`Usage ${u.usage} / limits ${u.limits}`}>
      {pct < 0.01 ? '<0.01' : pct < 1 ? pct.toFixed(pct < 0.1 ? 3 : 2) : pct.toFixed(2)}%
      <span className="ox-pct-bar">
        <i className={pct >= 90 ? 'hot' : pct >= 70 ? 'warm' : ''} style={{ width: `${Math.min(100, pct)}%` }} />
      </span>
    </span>
  );
}

/**
 * Usage against requests and limits with tick marks (the side panel's container table and the
 * Resource Utilization view).
 */
export function UtilCell({ u, kind }: { u: KubeUtilization | null | undefined; kind: 'cpu' | 'memory' }) {
  if (!u) return <span className="muted">—</span>;
  const fmt = kind === 'cpu' ? formatCores : formatMem;
  // Without requests, limits or capacity there is nothing to compare usage with.
  const ref = u.capacity ?? (u.requests != null || u.limits != null ? Math.max(u.limits ?? 0, u.requests ?? 0, u.usage ?? 0) * 1.15 : 0);
  const pct = (v: number | null | undefined) => (v == null || !ref ? null : Math.min(100, (v / ref) * 100));
  const over = u.usage != null && ((u.limits != null && u.usage > u.limits * 0.9) || (u.requests != null && u.usage > u.requests));
  const title = [`Usage (15m avg): ${fmt(u.usage)}`, `Requests: ${fmt(u.requests)}`, `Limits: ${fmt(u.limits)}`, u.capacity != null ? `Allocatable: ${fmt(u.capacity)}` : null]
    .filter(Boolean)
    .join('\n');
  return (
    <span className="kx-util" title={title}>
      <span className="kx-util-text">
        {fmt(u.usage)}
        {u.requests != null && <span className="muted"> / {fmt(u.requests)}</span>}
      </span>
      {ref > 0 && (
        <span className="kx-util-bar">
          {u.usage != null && <i className={over ? 'over' : ''} style={{ width: `${pct(u.usage)}%` }} />}
          {u.requests != null && <b className="req" style={{ left: `${pct(u.requests)}%` }} />}
          {u.limits != null && <b className="lim" style={{ left: `${pct(u.limits)}%` }} />}
        </span>
      )}
    </span>
  );
}

const vcpu = (v: number | null | undefined) => (v == null ? '—' : `${Number(v.toFixed(2))} vCPU`);

const nameCol = (label: string): KubeColumn => ({
  key: 'name',
  label,
  sort: 'name',
  render: (r) => (
    <span className="ox-name" title={r.name}>
      {r.name}
    </span>
  ),
});
const status: KubeColumn = { key: 'status', label: 'STATUS', sort: 'status', render: (r) => <StatusPill status={r.status} health={r.health} /> };
const cluster: KubeColumn = { key: 'cluster', label: 'CLUSTER', sort: 'cluster', render: (r, c) => <Chip k="kube_cluster_name" v={r.cluster} ctx={c} /> };
const ns: KubeColumn = { key: 'namespace', label: 'NAMESPACE', sort: 'namespace', render: (r, c) => <Chip k="kube_namespace" v={r.namespace} ctx={c} /> };
const ageCol: KubeColumn = { key: 'age', label: 'AGE', sort: 'age', num: true, render: (r) => age(r.createdAt) };
const ready: KubeColumn = {
  key: 'ready',
  label: 'READY',
  num: true,
  render: (r) => (r.ready ? <span className={`ox-ready${r.ready.split('/')[0] === r.ready.split('/')[1] ? '' : ' bad'}`}>{r.ready}</span> : '—'),
};
const restarts: KubeColumn = { key: 'restarts', label: 'RESTARTS', sort: 'restarts', num: true, render: (r) => r.restarts ?? 0 };
const cpu: KubeColumn = { key: 'cpu', label: 'CPU USAGE/LIMITS', sort: 'cpu', num: true, render: (r) => <PctOfLimits u={r.cpu} /> };
const mem: KubeColumn = { key: 'memory', label: 'MEM USAGE/LIMITS', sort: 'memory', num: true, render: (r) => <PctOfLimits u={r.memory} /> };
const pods: KubeColumn = { key: 'pods', label: 'PODS', sort: 'pods', num: true, render: (r) => r.pods ?? 0 };
const n = (key: keyof KubeResource, label: string): KubeColumn => ({ key, label, num: true, render: (r) => (r[key] as number | null | undefined) ?? '—' });
const t = (key: keyof KubeResource, label: string, mono = false): KubeColumn => ({
  key,
  label,
  render: (r) => <span className={mono ? 'mono' : undefined}>{(r[key] as string | null | undefined) || '—'}</span>,
});
/** Capacity-based utilization (nodes, clusters): usage / allocatable. */
const alloc = (k: 'cpu' | 'memory', label: string): KubeColumn => ({ key: k, label, sort: k, num: true, render: (r) => <PctOfLimits u={r[k] ? { ...r[k], limits: r[k]!.capacity } : null} /> });

const tagValue = (r: KubeResource, key: string) => r.tags.find((x) => x.startsWith(`${key}:`))?.slice(key.length + 1);

/** Process CPU as a share of all the host's cores (Datadog's "Total CPU - Normalized"). */
function normalizedCpu(r: KubeResource): number | null {
  const used = r.cpu?.usage;
  if (used == null) return null;
  return (used / (r.hostCpus || 1)) * 100;
}

export const COLUMNS: Record<KubeKind, KubeColumn[]> = {
  processes: [
    {
      key: 'name',
      label: 'PROCESS',
      sort: 'name',
      render: (r) => (
        <span className="ox-name mono" title={r.name}>
          {r.name}
        </span>
      ),
    },
    { key: 'user', label: 'USERNAME', sort: 'user', render: (r, c) => <Chip k="user" v={r.user} ctx={c} /> },
    { key: 'host', label: 'HOST', render: (r, c) => <Chip k="host" v={r.node} ctx={c} /> },
    { key: 'pod', label: 'POD', render: (r, c) => <Chip k="pod_name" v={r.owner?.name} ctx={c} /> },
    {
      key: 'container',
      label: 'CONTAINER',
      render: (r, c) => {
        const id = tagValue(r, 'container_id');
        return <Chip k="container_id" v={id ? id.slice(0, 12) : null} ctx={{ filter: (k) => id && c.filter(k, id) }} />;
      },
    },
    {
      key: 'cpu',
      label: 'TOTAL CPU - NORMALIZED',
      sort: 'cpu',
      num: true,
      render: (r) => {
        const pct = normalizedCpu(r);
        return pct == null ? '—' : `${pct < 0.01 && pct > 0 ? '<0.01' : pct.toFixed(2)}%`;
      },
    },
    { key: 'memory', label: 'RSS MEMORY', sort: 'memory', num: true, render: (r) => formatMem(r.memory?.usage) },
    { key: 'age', label: 'STARTED', sort: 'age', num: true, render: (r) => (r.createdAt ? `${age(r.createdAt)} ago` : '—') },
  ],
  pods: [nameCol('POD'), status, cluster, ns, ageCol, ready, restarts, cpu, mem],
  clusters: [nameCol('CLUSTER'), status, t('version', 'VERSION'), n('nodes', 'NODES'), pods, alloc('cpu', 'CPU USAGE/ALLOCATABLE'), alloc('memory', 'MEM USAGE/ALLOCATABLE'), ageCol],
  namespaces: [nameCol('NAMESPACE'), status, cluster, ageCol, pods, cpu, mem],
  nodes: [nameCol('NODE'), status, cluster, t('roles', 'ROLES'), t('version', 'VERSION'), ageCol, pods, alloc('cpu', 'CPU USAGE/ALLOCATABLE'), alloc('memory', 'MEM USAGE/ALLOCATABLE')],
  deployments: [nameCol('DEPLOYMENT'), status, cluster, ns, ageCol, { ...ready, label: 'PODS READY' }, n('upToDate', 'UP-TO-DATE'), n('available', 'AVAILABLE'), cpu, mem],
  replicasets: [nameCol('REPLICA SET'), status, cluster, ns, ageCol, { ...ready, label: 'PODS READY' }, n('desired', 'DESIRED'), n('current', 'CURRENT'), cpu, mem],
  daemonsets: [nameCol('DAEMON SET'), status, cluster, ns, ageCol, n('desired', 'DESIRED'), n('current', 'CURRENT'), ready, n('upToDate', 'UP-TO-DATE'), n('available', 'AVAILABLE'), cpu, mem],
  statefulsets: [nameCol('STATEFUL SET'), status, cluster, ns, ageCol, { ...ready, label: 'PODS READY' }, n('upToDate', 'UP-TO-DATE'), cpu, mem],
  jobs: [
    nameCol('JOB'),
    status,
    cluster,
    ns,
    ageCol,
    t('completions', 'COMPLETIONS'),
    { key: 'duration', label: 'DURATION', num: true, render: (r) => (r.durationSec == null ? '—' : age(Date.now() - r.durationSec * 1000)) },
    n('active', 'ACTIVE'),
  ],
  cronjobs: [
    nameCol('CRON JOB'),
    status,
    cluster,
    ns,
    ageCol,
    t('schedule', 'SCHEDULE', true),
    { key: 'suspend', label: 'SUSPEND', render: (r) => (r.suspend ? 'True' : 'False') },
    n('active', 'ACTIVE'),
    { key: 'lastSchedule', label: 'LAST SCHEDULE', num: true, render: (r) => (r.lastSchedule ? `${age(r.lastSchedule)} ago` : '—') },
  ],
  services: [
    nameCol('SERVICE'),
    { key: 'type', label: 'TYPE', sort: 'type', render: (r) => r.serviceType ?? '—' },
    cluster,
    ns,
    ageCol,
    t('clusterIp', 'CLUSTER IP', true),
    t('external', 'EXTERNAL IP', true),
    t('ports', 'PORTS', true),
  ],
  ingresses: [nameCol('INGRESS'), status, cluster, ns, ageCol, t('ingressClass', 'CLASS'), t('hosts', 'HOSTS', true), t('external', 'ADDRESS', true)],
  networkpolicies: [nameCol('NETWORK POLICY'), cluster, ns, ageCol],
  poddisruptionbudgets: [nameCol('POD DISRUPTION BUDGET'), status, cluster, ns, ageCol, n('desired', 'DESIRED HEALTHY'), n('current', 'CURRENT HEALTHY'), n('available', 'ALLOWED DISRUPTIONS')],
  limitranges: [nameCol('LIMIT RANGE'), cluster, ns, ageCol],
  resourcequotas: [nameCol('RESOURCE QUOTA'), cluster, ns, ageCol],
  persistentvolumeclaims: [
    nameCol('PERSISTENT VOLUME CLAIM'),
    status,
    cluster,
    ns,
    ageCol,
    t('volume', 'VOLUME'),
    t('capacity', 'CAPACITY'),
    t('accessModes', 'ACCESS MODES'),
    t('storageClass', 'STORAGE CLASS'),
  ],
  persistentvolumes: [nameCol('PERSISTENT VOLUME'), status, cluster, ageCol, t('capacity', 'CAPACITY'), t('accessModes', 'ACCESS MODES'), t('volume', 'CLAIM'), t('storageClass', 'STORAGE CLASS')],
  storageclasses: [nameCol('STORAGE CLASS'), cluster, ageCol, t('target', 'PROVISIONER'), t('volume', 'RECLAIM POLICY')],
  serviceaccounts: [nameCol('SERVICE ACCOUNT'), cluster, ns, ageCol],
  roles: [nameCol('ROLE'), cluster, ns, ageCol, n('desired', 'RULES')],
  rolebindings: [nameCol('ROLE BINDING'), cluster, ns, ageCol, t('target', 'ROLE'), t('hosts', 'SUBJECTS')],
  clusterroles: [nameCol('CLUSTER ROLE'), cluster, ageCol, n('desired', 'RULES')],
  clusterrolebindings: [nameCol('CLUSTER ROLE BINDING'), cluster, ageCol, t('target', 'ROLE'), t('hosts', 'SUBJECTS')],
  horizontalpodautoscalers: [
    nameCol('HORIZONTAL POD AUTOSCALER'),
    status,
    cluster,
    ns,
    ageCol,
    t('target', 'REFERENCE'),
    t('replicasRange', 'MIN-MAX'),
    n('current', 'REPLICAS'),
    n('desired', 'DESIRED'),
  ],
  customresourcedefinitions: [nameCol('CRD'), status, cluster, ageCol, t('target', 'GROUP'), t('version', 'VERSION'), t('volume', 'SCOPE')],
  containers: [
    nameCol('CONTAINER'),
    status,
    {
      key: 'image',
      label: 'IMAGE',
      render: (r) => (
        <span className="ox-name mono" title={r.images[0]}>
          {r.images[0] ?? '—'}
        </span>
      ),
    },
    { key: 'cluster', label: 'CLUSTER', sort: 'cluster', render: (r, c) => <Chip k={r.tags.some((x) => x.startsWith('ecs_cluster:')) ? 'ecs_cluster' : 'kube_cluster_name'} v={r.cluster} ctx={c} /> },
    {
      key: 'owner',
      label: 'POD / TASK',
      render: (r) => (
        <span className="ox-name" title={r.owner?.name}>
          {r.owner?.name ?? '—'}
        </span>
      ),
    },
    { key: 'age', label: 'STARTED', sort: 'age', num: true, render: (r) => (r.createdAt ? `${age(r.createdAt)} ago` : '—') },
    restarts,
    cpu,
    mem,
  ],
  container_images: [
    nameCol('IMAGE'),
    { key: 'tag', label: 'TAG', render: (r) => <span className="mono">{r.imageTag ?? '—'}</span> },
    n('runningContainers', 'RUNNING CONTAINERS'),
    {
      key: 'cluster',
      label: 'CLUSTERS',
      render: (r, c) => <span className="ox-chips">{r.cluster ? r.cluster.split(',').map((x) => <Chip key={x} k="kube_cluster_name" v={x} ctx={c} />) : '—'}</span>,
    },
  ],
  ecs_tasks: [
    nameCol('TASK'),
    { key: 'taskDefinition', label: 'TASK DEFINITION', render: (r, c) => <Chip k="task_family" v={r.taskDefinition} ctx={{ filter: (k) => c.filter(k, r.taskDefinition?.split(':')[0] ?? '') }} /> },
    { key: 'cluster', label: 'CLUSTER', sort: 'cluster', render: (r, c) => <Chip k="ecs_cluster" v={r.cluster} ctx={c} /> },
    status,
    {
      key: 'health',
      label: 'HEALTH',
      render: (r) => <span className={`ox-status ox-${r.taskHealth === 'HEALTHY' ? 'ok' : r.taskHealth === 'UNHEALTHY' ? 'error' : 'neutral'}`}>{r.taskHealth ?? 'UNKNOWN'}</span>,
    },
    { key: 'launchType', label: 'LAUNCH TYPE', render: (r, c) => <Chip k="task_launch_type" v={r.launchType} ctx={c} /> },
    { key: 'cpuReservation', label: 'CPU RESERVATION', num: true, render: (r) => vcpu(r.cpuReservation) },
    { key: 'memoryReservation', label: 'MEM RESERVATION', num: true, render: (r) => formatMem(r.memoryReservation) },
    ageCol,
  ],
  ecs_task_definitions: [
    nameCol('TASK DEFINITION'),
    status,
    { key: 'launchType', label: 'LAUNCH TYPE', render: (r, c) => <Chip k="task_launch_type" v={r.launchType} ctx={c} /> },
    { key: 'cpuReservation', label: 'CPU RESERVATION', num: true, render: (r) => vcpu(r.cpuReservation) },
    { key: 'memoryReservation', label: 'MEM RESERVATION', num: true, render: (r) => formatMem(r.memoryReservation) },
    n('tasks', 'TASKS'),
  ],
  ecs_services: [
    nameCol('SERVICE'),
    { key: 'cluster', label: 'CLUSTER', sort: 'cluster', render: (r, c) => <Chip k="ecs_cluster" v={r.cluster} ctx={c} /> },
    status,
    { key: 'launchType', label: 'LAUNCH TYPE', render: (r, c) => <Chip k="task_launch_type" v={r.launchType} ctx={c} /> },
    t('taskDefinition', 'TASK DEFINITION'),
    n('tasks', 'RUNNING TASKS'),
  ],
  ecs_clusters: [nameCol('CLUSTER'), status, n('tasks', 'RUNNING TASKS'), n('services', 'SERVICES')],
};

export function logQuery(r: KubeResource): string | null {
  const nsTerm = r.namespace ? ` kube_namespace:${r.namespace}` : '';
  const c = `kube_cluster_name:${r.cluster}`;
  switch (r.kind) {
    case 'pods':
      return `${c}${nsTerm} pod_name:${r.name}`;
    case 'deployments':
      return `${c}${nsTerm} kube_deployment:${r.name}`;
    case 'replicasets':
      return `${c}${nsTerm} kube_replica_set:${r.name}`;
    case 'daemonsets':
      return `${c}${nsTerm} kube_daemon_set:${r.name}`;
    case 'statefulsets':
      return `${c}${nsTerm} kube_stateful_set:${r.name}`;
    case 'jobs':
      return `${c}${nsTerm} kube_job:${r.name}`;
    case 'cronjobs':
      return `${c}${nsTerm} kube_cronjob:${r.name}`;
    case 'nodes':
      return `${c} kube_node:${r.name}`;
    case 'namespaces':
      return `${c} kube_namespace:${r.name}`;
    case 'clusters':
      return c;
    case 'containers':
      return r.owner?.kind === 'Task' ? `task_arn:${r.owner.uid} container_name:${r.name}` : `${c}${nsTerm} pod_name:${r.owner?.name} kube_container_name:${r.name}`;
    case 'container_images':
      return `image_name:${r.name}`;
    case 'ecs_tasks':
      return `task_arn:${r.uid}`;
    case 'ecs_task_definitions':
      return `task_family:${r.name.split(':')[0]} task_version:${r.name.split(':')[1] ?? '*'}`;
    case 'ecs_services':
      return `ecs_cluster:${r.cluster} task_family:${r.taskDefinition?.split(':')[0] ?? r.name}`;
    case 'ecs_clusters':
      return `ecs_cluster:${r.name}`;
    default:
      return null;
  }
}

/** Metric graphs of the side panel's Metrics tab. */
export function metricQueries(r: KubeResource): { title: string; query: string; unit: 'cores' | 'bytes' | 'count' }[] {
  const c = `kube_cluster_name:${r.cluster}`;
  const ns = r.namespace ? `,kube_namespace:${r.namespace}` : '';
  const podScope = (tag: string) => [
    { title: 'CPU usage by pod (cores)', query: `sum:k8s.pod.cpu.utilization{${c}${ns},${tag}:${r.name}} by {pod_name}`, unit: 'cores' as const },
    { title: 'Memory working set by pod', query: `sum:k8s.pod.memory.working_set{${c}${ns},${tag}:${r.name}} by {pod_name}`, unit: 'bytes' as const },
  ];
  switch (r.kind) {
    case 'pods':
      return [
        { title: 'CPU usage by container (cores)', query: `sum:container.cpu.utilization{${c}${ns},pod_name:${r.name}} by {kube_container_name}`, unit: 'cores' },
        { title: 'Memory working set by container', query: `sum:container.memory.working_set{${c}${ns},pod_name:${r.name}} by {kube_container_name}`, unit: 'bytes' },
        { title: 'Container restarts', query: `max:k8s.container.restarts{${c}${ns},pod_name:${r.name}} by {kube_container_name}`, unit: 'count' },
      ];
    case 'deployments':
      return [...podScope('kube_deployment'), { title: 'Replicas available / desired', query: `max:k8s.deployment.available{${c}${ns},kube_deployment:${r.name}}`, unit: 'count' }];
    case 'daemonsets':
      return [...podScope('kube_daemon_set'), { title: 'Nodes ready', query: `max:k8s.daemonset.ready_nodes{${c}${ns},kube_daemon_set:${r.name}}`, unit: 'count' }];
    case 'statefulsets':
      return podScope('kube_stateful_set');
    case 'replicasets':
      return podScope('kube_replica_set');
    case 'nodes':
      return [
        { title: 'CPU usage (cores)', query: `avg:k8s.node.cpu.utilization{${c},kube_node:${r.name}}`, unit: 'cores' },
        { title: 'Memory working set', query: `avg:k8s.node.memory.working_set{${c},kube_node:${r.name}}`, unit: 'bytes' },
        { title: 'CPU usage by namespace (cores)', query: `sum:k8s.pod.cpu.utilization{${c},kube_node:${r.name}} by {kube_namespace}`, unit: 'cores' },
      ];
    case 'namespaces':
      return [
        { title: 'CPU usage by pod (cores)', query: `sum:k8s.pod.cpu.utilization{${c},kube_namespace:${r.name}} by {pod_name}`, unit: 'cores' },
        { title: 'Memory working set by pod', query: `sum:k8s.pod.memory.working_set{${c},kube_namespace:${r.name}} by {pod_name}`, unit: 'bytes' },
      ];
    case 'containers':
      return r.owner?.kind === 'Task'
        ? [
            { title: 'CPU usage (vCPU)', query: `avg:container.cpu.usage.vcpu{task_arn:${r.owner.uid},container_name:${r.name}}`, unit: 'cores' },
            { title: 'Memory usage', query: `avg:container.memory.usage{task_arn:${r.owner.uid},container_name:${r.name}}`, unit: 'bytes' },
          ]
        : [
            { title: 'CPU usage (cores)', query: `avg:container.cpu.utilization{${c}${ns},pod_name:${r.owner?.name},kube_container_name:${r.name}}`, unit: 'cores' },
            { title: 'Memory working set', query: `avg:container.memory.working_set{${c}${ns},pod_name:${r.owner?.name},kube_container_name:${r.name}}`, unit: 'bytes' },
          ];
    case 'ecs_tasks':
      return [
        { title: 'CPU usage (vCPU)', query: `avg:ecs.task.cpu.usage.vcpu{task_arn:${r.uid}}`, unit: 'cores' },
        { title: 'Memory usage', query: `avg:ecs.task.memory.usage{task_arn:${r.uid}}`, unit: 'bytes' },
        { title: 'CPU usage by container (vCPU)', query: `avg:container.cpu.usage.vcpu{task_arn:${r.uid}} by {container_name}`, unit: 'cores' },
      ];
    case 'ecs_task_definitions':
    case 'ecs_services':
    case 'ecs_clusters': {
      const scope =
        r.kind === 'ecs_clusters'
          ? `ecs_cluster:${r.name}`
          : r.kind === 'ecs_services'
            ? `ecs_cluster:${r.cluster},task_family:${r.taskDefinition?.split(':')[0]}`
            : `task_family:${r.name.split(':')[0]},task_version:${r.name.split(':')[1]}`;
      return [
        { title: 'CPU usage by task (vCPU)', query: `sum:ecs.task.cpu.usage.vcpu{${scope}} by {task_arn}`, unit: 'cores' },
        { title: 'Memory usage by task', query: `sum:ecs.task.memory.usage{${scope}} by {task_arn}`, unit: 'bytes' },
      ];
    }
    case 'clusters':
      return [
        { title: 'CPU usage by node (cores)', query: `avg:k8s.node.cpu.utilization{${c}} by {kube_node}`, unit: 'cores' },
        { title: 'Memory working set by node', query: `avg:k8s.node.memory.working_set{${c}} by {kube_node}`, unit: 'bytes' },
        { title: 'CPU usage by namespace (cores)', query: `sum:k8s.pod.cpu.utilization{${c}} by {kube_namespace}`, unit: 'cores' },
      ];
    default:
      return [];
  }
}
