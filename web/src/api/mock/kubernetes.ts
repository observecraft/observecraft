// In-browser Kubernetes inventory for mock mode: two clusters of the e-commerce demo.
import type { KubeDetail, KubeFacet, KubeGroup, KubeHealth, KubeKind, KubeResource, KubeSearchPage, KubeSearchRequest, KubeUtilization } from '../kubernetes/client';
import { parseQuery } from '../../lib/searchQuery';
import { rng } from './generator';

const MI = 1024 * 1024;
const KINDS: KubeKind[] = [
  'clusters',
  'nodes',
  'namespaces',
  'pods',
  'deployments',
  'replicasets',
  'daemonsets',
  'statefulsets',
  'jobs',
  'cronjobs',
  'services',
  'ingresses',
  'persistentvolumeclaims',
  'persistentvolumes',
  'horizontalpodautoscalers',
  'processes',
];

interface Workload {
  name: string;
  ns: string;
  replicas: number;
  cpu: number;
  mem: number;
  crash?: boolean;
}

const WORKLOADS: Workload[] = [
  { name: 'api-gateway', ns: 'shop', replicas: 3, cpu: 0.25, mem: 256 * MI },
  { name: 'storefront-api', ns: 'shop', replicas: 3, cpu: 0.5, mem: 512 * MI },
  { name: 'order-service', ns: 'shop', replicas: 2, cpu: 0.25, mem: 384 * MI },
  { name: 'catalog-service', ns: 'shop', replicas: 2, cpu: 0.2, mem: 256 * MI },
  { name: 'inventory-service', ns: 'shop', replicas: 2, cpu: 0.2, mem: 256 * MI, crash: true },
  { name: 'billing', ns: 'payments', replicas: 2, cpu: 0.3, mem: 384 * MI },
  { name: 'workflow-engine', ns: 'payments', replicas: 1, cpu: 0.5, mem: 1024 * MI },
  { name: 'notification-worker', ns: 'shop', replicas: 1, cpu: 0.1, mem: 128 * MI },
];

const day = 86400e3;

function base(kind: KubeKind, cluster: string, name: string, ns: string | null, uid: string, age: number): KubeResource {
  return {
    uid,
    kind,
    cluster,
    name,
    namespace: ns,
    createdAt: Date.now() - age,
    lastSeen: Date.now(),
    status: '',
    health: 'ok',
    labels: {},
    annotations: {},
    tags: [`kube_cluster_name:${cluster}`, ...(ns ? [`kube_namespace:${ns}`] : [])],
    images: [],
    containers: [],
  };
}

function util(usage: number | null, requests: number | null, limits: number | null, capacity: number | null = null): KubeUtilization {
  return { usage, requests, limits, capacity };
}

function sum(rows: KubeResource[], pick: (r: KubeResource) => KubeUtilization | null | undefined): KubeUtilization {
  const add = (f: (u: KubeUtilization) => number | null | undefined) => {
    const v = rows
      .map((r) => pick(r))
      .filter((u): u is KubeUtilization => !!u)
      .map(f)
      .filter((x): x is number => x != null);
    return v.length ? v.reduce((a, b) => a + b, 0) : null;
  };
  return util(
    add((u) => u.usage),
    add((u) => u.requests),
    add((u) => u.limits),
    add((u) => u.capacity),
  );
}

let cache: KubeResource[] | null = null;

function inventory(): KubeResource[] {
  if (cache) return cache;
  const r = rng(42);
  const out: KubeResource[] = [];
  for (const [ci, cluster] of ['prod-eu', 'staging-eu'].entries()) {
    const nodes = Array.from({ length: ci === 0 ? 3 : 2 }, (_, i) => {
      const n = base('nodes', cluster, `i-0a1b2c3d4e5f600${ci}${i}`, null, `${cluster}-node-${i}`, 30 * day);
      Object.assign(n, {
        status: 'Ready',
        roles: '<none>',
        version: 'v1.31.2',
        instanceType: 'm6i.xlarge',
        zone: `eu-west-1${'abc'[i]}`,
        runtime: 'containerd://1.7.22',
        osImage: 'Amazon Linux 2023',
      });
      n.tags.push(`kube_node:${n.name}`, 'kubelet_version:v1.31.2', 'kube_node_role:<none>', 'os:linux');
      n.cpu = util(0.4 + r() * 1.5, null, null, 3.92);
      n.memory = util((3 + r() * 6) * 1024 * MI, null, null, 14.8 * 1024 * MI);
      out.push(n);
      return n;
    });
    const pods: KubeResource[] = [];
    for (const w of WORKLOADS) {
      const dep = base('deployments', cluster, w.name, w.ns, `${cluster}-dep-${w.name}`, 12 * day);
      const ready = w.crash ? w.replicas - 1 : w.replicas;
      Object.assign(dep, {
        desired: w.replicas,
        current: w.replicas,
        upToDate: w.replicas,
        available: ready,
        ready: `${ready}/${w.replicas}`,
        status: ready === w.replicas ? 'Available' : 'Progressing',
        health: (ready === w.replicas ? 'ok' : 'warning') as KubeHealth,
      });
      dep.images = [`registry.example.com/${w.name}:2.4.${ci}`];
      dep.labels = { 'app.kubernetes.io/name': w.name, 'tags.datadoghq.com/service': w.name, 'tags.datadoghq.com/env': cluster.startsWith('prod') ? 'prod' : 'staging' };
      dep.tags.push(`kube_deployment:${w.name}`, `service:${w.name}`, `env:${dep.labels['tags.datadoghq.com/env']}`, `short_image:${w.name}`);
      const rs = base('replicasets', cluster, `${w.name}-7d9f8c6b5`, w.ns, `${dep.uid}-rs`, 3 * day);
      Object.assign(rs, { desired: w.replicas, current: w.replicas, ready: dep.ready, status: dep.status, health: dep.health, owner: { kind: 'Deployment', name: w.name, uid: dep.uid } });
      rs.tags.push(`kube_replica_set:${rs.name}`, `kube_deployment:${w.name}`);
      const own: KubeResource[] = [];
      for (let i = 0; i < w.replicas; i++) {
        const crash = w.crash && i === 0;
        const node = nodes[(i + w.name.length) % nodes.length];
        const p = base('pods', cluster, `${rs.name}-${'xkqzt'.slice(i)}${i}w`, w.ns, `${rs.uid}-pod-${i}`, crash ? 2 * 3600e3 : 3 * day);
        const usage = crash ? null : w.cpu * (0.15 + r() * 0.9);
        const mem = crash ? null : w.mem * (0.3 + r() * 0.6);
        Object.assign(p, {
          status: crash ? 'CrashLoopBackOff' : 'Running',
          health: (crash ? 'error' : 'ok') as KubeHealth,
          ready: crash ? '0/1' : '1/1',
          restarts: crash ? 37 : Math.floor(r() * 2),
          node: node.name,
          ip: `10.42.${ci}.${10 + (out.length % 200)}`,
          qos: 'Burstable',
          owner: { kind: 'ReplicaSet', name: rs.name, uid: rs.uid },
          images: dep.images,
          labels: { ...dep.labels, 'pod-template-hash': '7d9f8c6b5' },
          cpu: util(usage, w.cpu, w.cpu * 2),
          memory: util(mem, w.mem, w.mem * 1.5),
        });
        p.containers = [
          {
            name: w.name,
            image: dep.images[0],
            ready: !crash,
            restarts: p.restarts ?? 0,
            state: crash ? 'waiting' : 'running',
            reason: crash ? 'CrashLoopBackOff' : null,
            startedAt: crash ? null : Date.now() - 3 * day,
            init: false,
            cpu: p.cpu!,
            memory: p.memory!,
          },
        ];
        p.tags.push(
          `pod_name:${p.name}`,
          `kube_deployment:${w.name}`,
          `kube_replica_set:${rs.name}`,
          'kube_ownerref_kind:replicaset',
          `kube_node:${node.name}`,
          'kube_qos:Burstable',
          `service:${w.name}`,
          `short_image:${w.name}`,
          `pod_phase:running`,
        );
        p.tags.push(...Object.entries(p.labels).map(([k, v]) => `${k}:${v}`));
        own.push(p);
      }
      for (const x of [dep, rs]) {
        x.cpu = sum(own, (p) => p.cpu);
        x.memory = sum(own, (p) => p.memory);
        x.pods = own.length;
      }
      const svc = base('services', cluster, w.name, w.ns, `${cluster}-svc-${w.name}`, 12 * day);
      Object.assign(svc, { status: 'Active', serviceType: 'ClusterIP', clusterIp: `172.20.${ci}.${out.length % 250}`, ports: '80/TCP', selector: `app.kubernetes.io/name=${w.name}` });
      svc.tags.push(`kube_service:${w.name}`, 'kube_service_type:ClusterIP');
      out.push(dep, rs, svc, ...own);
      pods.push(...own);
    }
    const agentPods = nodes.map((n, i) => {
      const p = base('pods', cluster, `observecraft-agent-${'abcde'[i]}9xk2`, 'observecraft', `${cluster}-agent-${i}`, 5 * day);
      Object.assign(p, { status: 'Running', ready: '1/1', restarts: 0, node: n.name, qos: 'Burstable', owner: { kind: 'DaemonSet', name: 'observecraft-agent', uid: `${cluster}-ds-agent` } });
      p.cpu = util(0.02 + r() * 0.03, 0.1, null);
      p.memory = util((90 + r() * 60) * MI, 200 * MI, 500 * MI);
      p.images = ['otel/opentelemetry-collector-contrib:0.112.0'];
      p.tags.push(`pod_name:${p.name}`, 'kube_daemon_set:observecraft-agent', 'kube_ownerref_kind:daemonset', `kube_node:${n.name}`, 'short_image:opentelemetry-collector-contrib');
      return p;
    });
    const ds = base('daemonsets', cluster, 'observecraft-agent', 'observecraft', `${cluster}-ds-agent`, 5 * day);
    Object.assign(ds, {
      status: 'Available',
      desired: nodes.length,
      current: nodes.length,
      upToDate: nodes.length,
      available: nodes.length,
      ready: `${nodes.length}/${nodes.length}`,
      pods: nodes.length,
      cpu: sum(agentPods, (p) => p.cpu),
      memory: sum(agentPods, (p) => p.memory),
    });
    ds.tags.push('kube_daemon_set:observecraft-agent');
    const cron = base('cronjobs', cluster, 'nightly-report', 'shop', `${cluster}-cron`, 20 * day);
    Object.assign(cron, { status: 'Scheduled', schedule: '0 2 * * *', suspend: false, active: 0, lastSchedule: Date.now() - 9 * 3600e3 });
    cron.tags.push('kube_cronjob:nightly-report');
    out.push(ds, cron, ...agentPods);
    pods.push(...agentPods);
    for (const ns of ['shop', 'payments', 'observecraft', 'kube-system']) {
      const n = base('namespaces', cluster, ns, null, `${cluster}-ns-${ns}`, 40 * day);
      const own = pods.filter((p) => p.namespace === ns);
      Object.assign(n, { status: 'Active', pods: own.length, cpu: sum(own, (p) => p.cpu), memory: sum(own, (p) => p.memory) });
      n.tags.push(`kube_namespace:${ns}`);
      out.push(n);
    }
    for (const n of nodes) {
      const on = pods.filter((p) => p.node === n.name);
      n.pods = on.length;
      n.cpu = { ...n.cpu!, requests: sum(on, (p) => p.cpu).requests, limits: sum(on, (p) => p.cpu).limits };
      n.memory = { ...n.memory!, requests: sum(on, (p) => p.memory).requests, limits: sum(on, (p) => p.memory).limits };
    }
    // Processes: the main process of each running pod, plus the node's own daemons.
    let pid = 1000;
    const proc = (host: string, cmd: string, user: string, cpu: number, rss: number, age: number, pod?: KubeResource) => {
      const p = base('processes', cluster, cmd, pod?.namespace ?? null, `process:${host}:${pid}`, age);
      Object.assign(p, { status: 'Running', node: host, user, hostCpus: 4, ip: String(pid++), cpu: { usage: cpu, capacity: 4 }, memory: { usage: rss } });
      p.tags.push(`host:${host}`, `command:${cmd.split(' ')[0].split('/').pop()}`, `user:${user}`);
      if (pod) {
        p.owner = { kind: 'Pod', name: pod.name, uid: pod.uid };
        p.tags.push(
          `pod_name:${pod.name}`,
          `kube_container_name:${pod.containers[0]?.name ?? pod.name}`,
          `container_id:${pod.uid
            .replace(/[^a-z0-9]/g, '')
            .padEnd(64, '0')
            .slice(0, 64)}`,
        );
      }
      out.push(p);
    };
    for (const p of pods.filter((x) => x.status === 'Running')) {
      const name = p.containers[0]?.name ?? p.name;
      proc(p.node ?? '', `/usr/local/bin/${name} --config /etc/${name}/config.yaml`, 'app', (p.cpu?.usage ?? 0.05) * (0.8 + r() * 0.2), (p.memory?.usage ?? 64 * MI) * 0.9, 3 * day, p);
    }
    for (const n of nodes) {
      proc(n.name, '/usr/bin/kubelet --config=/var/lib/kubelet/config.yaml', 'root', 0.08 + r() * 0.05, (90 + r() * 30) * MI, 20 * day);
      proc(n.name, '/usr/bin/containerd', 'root', 0.03 + r() * 0.02, (50 + r() * 20) * MI, 20 * day);
      proc(n.name, '/usr/sbin/sshd -D', 'root', 0, 6 * MI, 20 * day);
    }
    const c = base('clusters', cluster, cluster, null, `cluster:${cluster}`, 60 * day);
    Object.assign(c, { status: 'Healthy', nodes: nodes.length, pods: pods.length, version: 'v1.31.2', cpu: sum(nodes, (n) => n.cpu), memory: sum(nodes, (n) => n.memory) });
    out.push(c);
  }
  cache = out;
  return out;
}

function values(r: KubeResource, key: string): string[] {
  if (key === 'name') return [r.name];
  if (key === 'status') return [r.status];
  return r.tags.filter((t) => t.startsWith(`${key}:`)).map((t) => t.slice(key.length + 1));
}

function glob(p: string, v: string): boolean {
  const re = new RegExp(
    `^${p
      .toLowerCase()
      .replace(/[.+?^${}()|[\]\\]/g, '\\$&')
      .replace(/\*/g, '.*')}$`,
  );
  return re.test(v.toLowerCase());
}

function matches(r: KubeResource, query: string, skipKey?: string): boolean {
  return parseQuery(query).every((t) => {
    if (t.kind === 'text') return r.name.toLowerCase().includes(t.text.toLowerCase()) !== t.negate;
    if (t.key === skipKey) return true;
    return values(r, t.key).some((v) => t.values.some((p) => glob(p, v))) !== t.negate;
  });
}

const FACETS: Partial<Record<KubeKind, string[]>> = {
  pods: ['kube_node', 'kube_ownerref_kind', 'kube_deployment', 'kube_daemon_set', 'kube_qos', 'service', 'short_image'],
  nodes: ['kube_node_role', 'kubelet_version'],
  deployments: ['service', 'env', 'short_image'],
  services: ['kube_service_type'],
};

export async function searchKubernetes(req: KubeSearchRequest): Promise<KubeSearchPage> {
  const all = inventory();
  const ofKind = all.filter((r) => r.kind === req.kind);
  const keys = ['kube_cluster_name', ...(['clusters', 'nodes', 'namespaces', 'persistentvolumes'].includes(req.kind) ? [] : ['kube_namespace']), 'status', ...(FACETS[req.kind] ?? [])];
  const facets: KubeFacet[] = keys
    .map((key) => {
      const counts = new Map<string, number>();
      for (const r of ofKind.filter((x) => matches(x, req.query ?? '', key))) for (const v of new Set(values(r, key))) counts.set(v, (counts.get(v) ?? 0) + 1);
      return { key, values: [...counts].map(([value, count]) => ({ value, count })).sort((a, b) => b.count - a.count || a.value.localeCompare(b.value)) };
    })
    .filter((f) => f.values.length);
  const rows = ofKind.filter((r) => matches(r, req.query ?? ''));
  const field = req.sort ?? 'name';
  const key = (r: KubeResource): number | string =>
    field === 'cpu'
      ? (r.cpu?.usage ?? -1)
      : field === 'memory'
        ? (r.memory?.usage ?? -1)
        : field === 'restarts'
          ? (r.restarts ?? 0)
          : field === 'age'
            ? -(r.createdAt ?? 0)
            : field === 'status'
              ? r.status
              : field === 'namespace'
                ? (r.namespace ?? '')
                : r.name;
  rows.sort((a, b) => {
    const x = key(a);
    const y = key(b);
    const o = x < y ? -1 : x > y ? 1 : a.name.localeCompare(b.name);
    return req.desc ? -o : o;
  });
  let groups: KubeGroup[] | null = null;
  if (req.groupBy) {
    const keys = req.groupBy
      .split(',')
      .map((k) => k.trim())
      .filter(Boolean);
    const vals = (r: KubeResource) => keys.map((k) => values(r, k)[0] ?? `no ${k}`);
    const of = (r: KubeResource) => vals(r).join(', ');
    const map = new Map<string, KubeResource[]>();
    for (const r of rows) map.set(of(r), [...(map.get(of(r)) ?? []), r]);
    const set = (u: KubeUtilization | null | undefined) => u?.requests != null && u.limits != null;
    groups = [...map]
      .map(([k, rs]) => ({
        key: k,
        values: vals(rs[0]),
        count: rs.length,
        cpu: sum(rs, (r) => r.cpu),
        memory: sum(rs, (r) => r.memory),
        cpuComplete: rs.every((r) => set(r.cpu)),
        memoryComplete: rs.every((r) => set(r.memory)),
      }))
      .sort((a, b) => b.count - a.count || a.key.localeCompare(b.key));
    const rank = new Map(groups.map((x, i) => [x.key, i]));
    rows.sort((a, b) => rank.get(of(a))! - rank.get(of(b))!);
  }
  const tagKeys = [...new Set(ofKind.flatMap((r) => r.tags.map((t) => t.split(':')[0])))].sort();
  return {
    items: rows.slice(req.offset ?? 0, (req.offset ?? 0) + req.limit),
    total: rows.length,
    groups,
    facets,
    kinds: KINDS.map((k) => ({ kind: k, count: all.filter((r) => r.kind === k).length })),
    tagKeys,
  };
}

export async function getKubeResource(uid: string): Promise<KubeDetail> {
  const all = inventory();
  const r = all.find((x) => x.uid === uid);
  if (!r) throw new Error('Kubernetes resource not found');
  const related = all.filter(
    (x) =>
      x.cluster === r.cluster &&
      x.uid !== r.uid &&
      (x.owner?.uid === r.uid || r.owner?.uid === x.uid || all.some((rs) => rs.owner?.uid === r.uid && x.owner?.uid === rs.uid) || (r.kind === 'nodes' && x.node === r.name)),
  );
  const manifest = {
    apiVersion: 'v1',
    kind: r.kind,
    metadata: { name: r.name, namespace: r.namespace, uid: r.uid, labels: r.labels, creationTimestamp: new Date(r.createdAt ?? 0).toISOString() },
    spec: {},
    status: { phase: r.status },
  };
  const events =
    r.health === 'error'
      ? [
          {
            timestamp: Date.now() - 60e3,
            eventType: 'Warning',
            reason: 'BackOff',
            message: `Back-off restarting failed container ${r.containers[0]?.name ?? r.name} in pod ${r.name}`,
            objectKind: 'Pod',
            objectName: r.name,
            count: 37,
          },
        ]
      : [
          {
            timestamp: Date.now() - 3 * day,
            eventType: 'Normal',
            reason: r.kind === 'pods' ? 'Started' : 'ScalingReplicaSet',
            message: r.kind === 'pods' ? `Started container ${r.name}` : `Scaled up replica set to ${r.desired ?? 1}`,
            objectKind: r.kind ?? '',
            objectName: r.name,
            count: 1,
          },
        ];
  return { resource: r, manifest: manifest as unknown as KubeDetail['manifest'], related, events };
}
