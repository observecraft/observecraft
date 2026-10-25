// In-browser stand-in for the Infrastructure endpoints (see api/infrastructure/client.ts).
// 443 deterministic hosts; search / grouping / sorting / paging happen here as the backend would.
import type {
  CollectorKind,
  Container,
  HostDetail,
  HostMetrics,
  HostSearchPage,
  HostSearchRequest,
  HostSummary,
  PageRequest,
  Paged,
  Pod,
  Process,
  ProcessPage,
  Software,
  TriggeredMonitor,
} from '../infrastructure/types';
import type { MetricSeries, TimeRange } from '../types';
import { parseQuery } from '../../lib/searchQuery';
import { LOG_HOSTS, rng, type Rand } from './generator';
import { matchesTerms } from './match';

const delay = <T,>(value: T, ms = 100 + Math.random() * 150) => new Promise<T>((r) => setTimeout(() => r(value), ms));
const pick = <T,>(r: Rand, arr: readonly T[]): T => arr[Math.floor(r() * arr.length)];
const hex = (r: Rand, n: number) => Array.from({ length: n }, () => Math.floor(r() * 16).toString(16)).join('');

function hashSeed(s: string): number {
  let h = 2166136261;
  for (let i = 0; i < s.length; i++) h = Math.imul(h ^ s.charCodeAt(i), 16777619);
  return h >>> 0;
}

interface StoredHost extends HostSummary {
  seed: number;
  cluster: string | null;
  region: string | null;
  uptimeSec: number;
}

const COLLECTOR_VERSION = '0.112.0';
const TEAMS = ['storefront', 'billing', 'platform', 'payments'];
const CLUSTERS: Record<string, string> = { storefront: 'storefrontprod-core', billing: 'billingprod-core', platform: 'platform-shared', payments: 'payments-core' };

let store: StoredHost[] | null = null;

function hosts(): StoredHost[] {
  if (store) return store;
  const r = rng(2026);
  const out: StoredHost[] = [];
  const add = (h: Omit<StoredHost, 'seed' | 'monitors' | 'cpuPct' | 'memPct' | 'errorSpans' | 'uptimeSec'> & { cpu?: number }) => {
    const seed = hashSeed(h.name);
    const hr = rng(seed);
    const cpuPct = Math.round(h.cpu ?? Math.pow(hr(), 1.8) * 95 + 2);
    const monitored = h.os !== 'windows' && hr() > 0.08;
    out.push({
      ...h,
      seed,
      cpuPct,
      memPct: Math.round(20 + hr() * 70),
      monitors: monitored ? { alert: cpuPct > 80 ? 1 : 0, warn: cpuPct > 70 && cpuPct <= 80 ? 1 : 0, ok: 1 + Math.floor(hr() * 5) } : { alert: 0, warn: 0, ok: 0 },
      errorSpans: h.software.includes('kubernetes') && hr() < 0.12 ? 1 + Math.floor(hr() * 5) : null,
      uptimeSec: Math.round(3600 * (6 + hr() * 24 * 20)),
    });
  };

  // Hosts that also emit the mock logs, so the host → logs link has data.
  LOG_HOSTS.forEach((name, i) =>
    add({ name, cloud: 'aws', os: 'linux', collectors: ['observecraft'], software: ['kubernetes'], env: 'prod', service: null, team: 'billing', gpu: false, cluster: CLUSTERS.billing, region: 'eu-central-1', cpu: [92, 47, 33, 28, 12][i] }),
  );
  // EKS nodes.
  for (let i = 0; i < 360; i++) {
    const team = pick(r, TEAMS);
    add({
      name: `i-0${hex(r, 16)}`,
      cloud: 'aws',
      os: 'linux',
      collectors: r() < 0.15 ? ['observecraft', 'otel'] : ['observecraft'],
      software: ['kubernetes'],
      env: pick(r, ['prod', 'prod', 'qa', 'staging']),
      service: null,
      team,
      gpu: r() < 0.03,
      cluster: CLUSTERS[team],
      region: pick(r, ['eu-central-1', 'eu-west-1']),
    });
  }
  // Windows domain controllers / app servers (no monitors yet).
  for (let i = 1; i <= 24; i++)
    add({ name: `DCHQ${String(i).padStart(2, '0')}`, cloud: 'none', os: 'windows', collectors: ['observecraft'], software: [], env: 'prod', service: null, team: 'platform', gpu: false, cluster: null, region: null });
  // On-prem Ubuntu servers running Docker.
  for (let i = 1; i <= 18; i++)
    add({ name: `${pick(r, ['edgep', 'billingdb', 'batch', 'gw'])}${String(i).padStart(2, '0')}`, cloud: 'none', os: 'linux', distro: 'ubuntu', collectors: ['observecraft'], software: ['docker'], env: r() < 0.3 ? null : 'prod', service: null, team: 'billing', gpu: false, cluster: null, region: null });
  // Azure + GCP VMs.
  for (let i = 1; i <= 16; i++)
    add({ name: `az-vm-${hex(r, 6)}`, cloud: 'azure', os: r() < 0.5 ? 'windows' : 'linux', collectors: ['otel'], software: [], env: 'staging', service: null, team: 'payments', gpu: false, cluster: null, region: 'westeurope' });
  for (let i = 1; i <= 12; i++)
    add({ name: `gke-node-${hex(r, 8)}`, cloud: 'gcp', os: 'linux', collectors: ['otel'], software: ['kubernetes'], env: 'qa', service: null, team: 'platform', gpu: i <= 3, cluster: 'gke-analytics', region: 'europe-west1' });
  // macOS CI runners + an Oracle Cloud host.
  for (let i = 1; i <= 7; i++)
    add({ name: `mac-ci-${String(i).padStart(2, '0')}`, cloud: 'none', os: 'darwin', collectors: ['observecraft'], software: [], env: null, service: 'ci-runner', team: 'platform', gpu: false, cluster: null, region: null });
  add({ name: 'oci-erp-01', cloud: 'oracle', os: 'linux', collectors: ['otel'], software: [], env: 'prod', service: 'erp', team: 'billing', gpu: false, cluster: null, region: 'eu-frankfurt-1' });

  store = out;
  return out;
}

function find(name: string): StoredHost {
  const h = hosts().find((x) => x.name === name);
  if (!h) throw new Error(`Host ${name} not found`);
  return h;
}

function field(h: StoredHost, key: string): string | undefined {
  switch (key) {
    case 'host':
      return h.name;
    case 'cloud':
      return h.cloud;
    case 'os':
      return h.os;
    case 'env':
      return h.env ?? undefined;
    case 'team':
      return h.team ?? undefined;
    case 'service':
      return h.service ?? undefined;
    case 'gpu':
      return String(h.gpu);
    case 'distro':
      return h.distro;
    case 'cluster':
    case 'kube_cluster_name':
      return h.cluster ?? undefined;
    case 'region':
      return h.region ?? undefined;
  }
  return undefined;
}

/** Multi-valued fields (collector, software) match if any value matches. */
function matches(h: StoredHost, terms: ReturnType<typeof parseQuery>): boolean {
  const multi = terms.filter((t) => t.kind === 'tag' && (t.key === 'collector' || t.key === 'software'));
  for (const t of multi) {
    if (t.kind !== 'tag') continue;
    const vals: string[] = t.key === 'collector' ? h.collectors : h.software;
    const hit = t.values.some((v) => vals.includes(v as CollectorKind & Software));
    if (hit === t.negate) return false;
  }
  return matchesTerms((k) => field(h, k), h.name, terms.filter((t) => !multi.includes(t)));
}

function metricValue(h: StoredHost, metric: string): number {
  switch (metric) {
    case 'system.cpu.user':
      return h.cpuPct;
    case 'system.mem.pct_usable':
      return 100 - h.memPct;
    case 'system.uptime':
      return h.uptimeSec;
    case 'system.load.1':
      return Math.round((h.cpuPct / 100) * 16 * 10) / 10;
  }
  return 0;
}

export function searchHosts(req: HostSearchRequest): Promise<HostSearchPage> {
  const terms = parseQuery(req.query);
  const matched = hosts()
    .filter((h) => matches(h, terms))
    .filter((h) =>
      req.metricFilters.every((f) => {
        const v = metricValue(h, f.metric);
        return (f.min === undefined || v >= f.min) && (f.max === undefined || v <= f.max);
      }),
    );

  const dir = req.sort.dir === 'asc' ? 1 : -1;
  const sortVal = (h: StoredHost): number | string => {
    switch (req.sort.field) {
      case 'cpu':
        return h.cpuPct;
      case 'mem':
        return h.memPct;
      case 'env':
        return h.env ?? '';
      case 'monitors':
        return h.monitors.alert * 100 + h.monitors.warn * 10;
      default:
        return h.name;
    }
  };
  const byField = (a: StoredHost, b: StoredHost) => {
    const x = sortVal(a);
    const y = sortVal(b);
    return (x < y ? -1 : x > y ? 1 : 0) * dir;
  };

  let groups: { key: string; count: number }[] | undefined;
  let ordered: (StoredHost & { group?: string })[];
  if (req.groupBy) {
    const groupOf = (h: StoredHost) => (req.groupBy === 'collector' ? h.collectors.join('+') : field(h, req.groupBy!)) ?? `no ${req.groupBy}`;
    const counts = new Map<string, number>();
    for (const h of matched) counts.set(groupOf(h), (counts.get(groupOf(h)) ?? 0) + 1);
    groups = [...counts.entries()].map(([key, count]) => ({ key, count })).sort((a, b) => b.count - a.count);
    const rank = new Map(groups.map((g, i) => [g.key, i]));
    ordered = matched.map((h) => ({ ...h, group: groupOf(h) })).sort((a, b) => rank.get(a.group)! - rank.get(b.group)! || byField(a, b));
  } else {
    ordered = [...matched].sort(byField);
  }

  const page = ordered.slice(req.offset, req.offset + req.limit).map(({ seed: _s, cluster: _c, region: _r, uptimeSec: _u, ...rest }) => rest);
  return delay({ hosts: page, total: matched.length, groups });
}

// ---------------------------------------------------------------- detail

function tagsFor(h: StoredHost, r: Rand): Record<string, string> {
  const t: Record<string, string> = { host: h.name, os: h.distro ?? h.os };
  if (h.env) t.env = h.env;
  if (h.team) t.team = h.team;
  if (h.cluster) {
    t.cluster_name = h.cluster;
    t.kube_cluster_name = h.cluster;
    t.karpenter_nodepool = pick(r, ['app-workloads-spot', 'general-on-demand', 'batch-spot']);
    t['managed-by'] = 'platform';
  }
  if (h.region) {
    t.region = h.region;
    t['availability-zone'] = `${h.region}${pick(r, ['a', 'b', 'c'])}`;
  }
  if (h.cloud === 'aws') t['instance-type'] = pick(r, ['m6i.xlarge', 'm6i.2xlarge', 'c6i.2xlarge', 'r6i.xlarge']);
  if (h.gpu) t.gpu_model = 'nvidia-a10g';
  return t;
}

function monitorFor(h: StoredHost): TriggeredMonitor[] {
  if (h.monitors.alert === 0 && h.monitors.warn === 0) return [];
  const r = rng(h.seed + 9);
  const now = Date.now();
  const series: [number, number][] = [];
  for (let t = now - 8 * 3600e3; t <= now; t += 5 * 60e3) series.push([t, Math.min(100, h.cpuPct + (r() - 0.5) * 3)]);
  return [
    {
      id: `mon-${h.seed % 100000}`,
      name: `[${h.team ?? 'infra'}] [K8S] Node CPU utilization is high on {{host.name}}`,
      status: h.monitors.alert ? 'alert' : 'warn',
      query: `avg(last_30m):100 - avg:system.cpu.idle{kube_cluster_name:${h.cluster ?? '*'}} by {host} > 80`,
      threshold: 80,
      series,
    },
  ];
}

function collectorConfig(h: StoredHost): string {
  const k8s = h.software.includes('kubernetes');
  return [
    'receivers:',
    '  hostmetrics:',
    '    collection_interval: 15s',
    '    scrapers: { cpu: {}, memory: {}, load: {}, filesystem: {}, network: {}, processes: {} }',
    ...(k8s ? ['  kubeletstats:', '    auth_type: serviceAccount', '    endpoint: ${env:K8S_NODE_NAME}:10250', '  prometheus/nginx_ingress_controller:', '    config:', '      scrape_configs:', "        - job_name: nginx-ingress", "          static_configs: [{ targets: ['ingress-nginx-controller-metrics:10254'] }]"] : []),
    '  otlp:',
    '    protocols: { grpc: { endpoint: 0.0.0.0:4317 }, http: { endpoint: 0.0.0.0:4318 } }',
    '',
    'processors:',
    '  resourcedetection: { detectors: [env, system, ec2] }',
    ...(k8s ? ['  k8sattributes: {}'] : []),
    '  batch: {}',
    '',
    'exporters:',
    '  otlp/observecraft:',
    '    endpoint: ${env:OBSERVECRAFT_OTLP_ENDPOINT}',
    '',
    'service:',
    '  pipelines:',
    '    metrics:',
    `      receivers: [hostmetrics${k8s ? ', kubeletstats, prometheus/nginx_ingress_controller' : ''}, otlp]`,
    `      processors: [resourcedetection${k8s ? ', k8sattributes' : ''}, batch]`,
    '      exporters: [otlp/observecraft]',
    '    traces:',
    '      receivers: [otlp]',
    '      processors: [resourcedetection, batch]',
    '      exporters: [otlp/observecraft]',
  ].join('\n');
}

function detail(h: StoredHost): HostDetail {
  const r = rng(h.seed + 1);
  const k8s = h.software.includes('kubernetes');
  const { seed: _s, cluster, region, uptimeSec, ...summary } = h;
  return {
    ...summary,
    account: h.cloud === 'aws' ? null : null,
    region,
    hostType: h.cloud === 'aws' ? (k8s ? 'Other Host' : 'EC2 Instance') : h.cloud === 'none' ? (h.os === 'darwin' ? 'Mac' : 'Bare metal') : 'VM',
    collectorVersion: `v${COLLECTOR_VERSION}`,
    uptimeSec,
    ntpOffsetMs: Math.round((r() - 0.5) * 120 * 100) / 100,
    tags: tagsFor(h, r),
    products: [...(h.errorSpans !== null || k8s ? (['apm'] as const) : []), 'infra', 'logs', ...(h.os === 'windows' ? (['processes'] as const) : [])],
    triggeredMonitors: monitorFor(h),
    integrations: k8s
      ? [
          { name: 'nginx_ingress_controller', status: h.cpuPct > 80 || r() < 0.2 ? 'broken' : 'ok', metrics: ['nginx_ingress_controller_requests', 'nginx_ingress_controller_request_duration_seconds', 'nginx_ingress_controller_nginx_process_connections'] },
          { name: 'kubelet', status: 'ok', metrics: ['k8s.pod.cpu.utilization', 'k8s.pod.memory.usage', 'k8s.container.restarts'] },
          { name: 'containerd', status: 'ok', metrics: ['container.cpu.utilization', 'container.memory.usage.total'] },
        ]
      : h.software.includes('docker')
        ? [{ name: 'docker', status: 'ok', metrics: ['container.cpu.utilization', 'container.memory.usage.total', 'container.network.io.usage.rx_bytes'] }]
        : [],
    kubernetes: k8s ? { cluster: cluster!, criName: 'containerd', criVersion: '2.2.9+bottlerocket', kubeletVersion: 'v1.35.6-eks-bca9cf6' } : null,
    collectorConfig: collectorConfig(h),
    pipelines: [
      { name: 'metrics', receivers: ['hostmetrics', ...(k8s ? ['kubeletstats', 'prometheus/nginx_ingress_controller'] : []), 'otlp'], processors: ['resourcedetection', ...(k8s ? ['k8sattributes'] : []), 'batch'], exporters: ['otlp/observecraft'] },
      { name: 'traces', receivers: ['otlp'], processors: ['resourcedetection', 'batch'], exporters: ['otlp/observecraft'] },
      { name: 'logs (Vector)', receivers: [k8s ? 'kubernetes_logs' : h.os === 'windows' ? 'windows_event_log' : 'journald'], processors: ['remap'], exporters: ['observecraft_logs'] },
    ],
  };
}

export function getHost(name: string): Promise<HostDetail> {
  return delay(detail(find(name)));
}

function series(name: string, range: TimeRange, r: Rand, base: number, noise: number, step: number): MetricSeries {
  const points: [number, number][] = [];
  let v = base;
  for (let t = range.from; t <= range.to; t += step) {
    v = Math.max(0, base + (v - base) * 0.7 + (r() - 0.5) * noise);
    points.push([t, Math.round(v * 100) / 100]);
  }
  return { name, points };
}

export function getHostMetrics(name: string, range: TimeRange): Promise<HostMetrics> {
  const h = find(name);
  const r = rng(h.seed + 2);
  const step = Math.max(60e3, Math.round((range.to - range.from) / 200));
  const busy = h.cpuPct;
  const totalGiB = pick(r, [8, 16, 32]);
  return delay({
    cpu: [
      series(`system.cpu.user{host:${name}}`, range, r, busy * 0.85, 4, step),
      series(`system.cpu.system{host:${name}}`, range, r, busy * 0.1, 1.5, step),
      series(`system.cpu.iowait{host:${name}}`, range, r, busy * 0.03, 0.6, step),
      series(`system.cpu.stolen{host:${name}}`, range, r, 0.3, 0.2, step),
      series(`system.cpu.idle{host:${name}}`, range, r, Math.max(0, 100 - busy), 3, step),
    ],
    load: [
      series(`system.load.1{host:${name}}`, range, r, (busy / 100) * 14, 3, step),
      series(`system.load.5{host:${name}}`, range, r, (busy / 100) * 13, 1.2, step),
      series(`system.load.15{host:${name}}`, range, r, (busy / 100) * 12.5, 0.5, step),
    ],
    memory: [
      series(`sum:system.mem.usable{host:${name}}`, range, r, totalGiB * (1 - h.memPct / 100), totalGiB * 0.02, step),
      series(`sum:system.mem.total{host:${name}}`, range, r, totalGiB, 0, step),
    ],
    processMemory: [
      { name: h.software.includes('kubernetes') ? 'java' : h.os === 'windows' ? 'sqlservr' : 'postgres', value: 47.98 },
      { name: 'kernel', value: 45.6 },
      { name: 'containerd', value: 3.1 },
      { name: 'otelcol', value: 1.6 },
      { name: 'vector', value: 1.0 },
      { name: 'other', value: 0.72 },
    ],
  });
}

// ---------------------------------------------------------------- pods / containers / processes

const NAMESPACES = ['logging', 'kube-system', 'ns-accounts-be-qa', 'observecraft', 'csi', 'ns-external-api-be-develop', 'ingress-nginx-private', 'ns-customers-be-qa', 'ns-customers-be-development'];
const WORKLOADS = ['aws-for-fluentbit', 'aws-node', 'backend', 'observecraft-collector', 'ebs-csi-node', 'external-api', 'ingress-nginx-private-controller', 'customers-backend', 'kube-proxy', 'processes'];

function podsFor(h: StoredHost): Pod[] {
  if (!h.software.includes('kubernetes')) return [];
  const r = rng(h.seed + 3);
  return Array.from({ length: 18 }, (_, i) => {
    const w = WORKLOADS[i % WORKLOADS.length];
    const system = ['aws-for-fluentbit', 'aws-node', 'ebs-csi-node', 'kube-proxy', 'observecraft-collector'].includes(w);
    const containers = w === 'aws-node' ? 2 : w === 'ebs-csi-node' || w === 'observecraft-collector' ? 3 : 1;
    return {
      name: `${w}-${system ? alnum(r, 5) : `${hex(r, 9)}-${alnum(r, 5)}`}`,
      status: r() < 0.97 ? 'RUNNING' : 'PENDING',
      cluster: h.cluster!,
      namespace: NAMESPACES[(i + (w === 'kube-proxy' ? 1 : 0)) % NAMESPACES.length],
      ageSec: w === 'observecraft-collector' ? 17 * 3600 : 2 * 86400 + Math.round(r() * 3600),
      ready: `${containers}/${containers}`,
      restarts: w === 'aws-for-fluentbit' ? 446 : Math.floor(r() * 2),
      cpuPctOfLimit: system ? null : Math.round(r() * 60) / 100,
      memPctOfLimit: system ? null : Math.round((15 + r() * 20) * 100) / 100,
    } satisfies Pod;
  });
}

function alnum(r: Rand, n: number): string {
  const c = 'bcdfghjklmnpqrstvwxz2456789';
  return Array.from({ length: n }, () => c[Math.floor(r() * c.length)]).join('');
}

function containersFor(h: StoredHost): Container[] {
  if (h.software.length === 0) return [];
  const r = rng(h.seed + 4);
  const now = Date.now();
  return Array.from({ length: h.software.includes('kubernetes') ? 25 : 6 }, (_, i) => {
    const w = i < 2 ? 'processes_processes-56bf944cd4' : `${WORKLOADS[i % WORKLOADS.length]}-${hex(r, 9)}`;
    return {
      name: `${w}-${alnum(r, 5)}`,
      status: 'UP',
      image: `123456789012.dkr.ecr.eu-central-1.amazonaws.com/${WORKLOADS[i % WORKLOADS.length]}:${1 + (i % 4)}.${i}`,
      cpuPct: i < 2 ? 70 + i : Math.round(Math.pow(r(), 2) * 40),
      rssBytes: Math.round((i < 2 ? 386 : 20 + r() * 300) * 1024 * 1024),
      memLimitBytes: 1024 * 1024 * 1024,
      netTxBytes: Math.round(r() < 0.5 ? 0 : r() * 4096),
      netRxBytes: Math.round(r() < 0.5 ? 0 : r() * 4096),
      startedAt: now - Math.round((2 + r() * 2) * 86400e3),
    } satisfies Container;
  }).sort((a, b) => b.rssBytes - a.rssBytes);
}

function processesFor(h: StoredHost): Process[] | null {
  if (h.os !== 'windows' && !h.distro) return null; // process collection only enabled on these hosts
  const r = rng(h.seed + 5);
  const cmds = h.os === 'windows'
    ? ['C:\\Windows\\System32\\lsass.exe', 'C:\\Windows\\System32\\svchost.exe -k netsvcs', 'C:\\Program Files\\ObserveCraft\\otelcol.exe', 'C:\\Windows\\System32\\dns.exe', 'C:\\Windows\\System32\\ntfrs.exe']
    : ['/usr/lib/postgresql/16/bin/postgres -D /var/lib/postgresql', '/usr/bin/dockerd -H fd://', '/opt/observecraft/otelcol --config /etc/observecraft/config.yaml', '/usr/bin/vector --config /etc/vector/vector.yaml', '/usr/sbin/sshd -D'];
  return cmds.map((command, i) => ({
    pid: 400 + Math.floor(r() * 30000),
    command,
    user: h.os === 'windows' ? 'SYSTEM' : i === 0 ? 'postgres' : 'root',
    cpuPct: Math.round(r() * 25 * 10) / 10,
    rssBytes: Math.round((30 + r() * 900) * 1024 * 1024),
  })).sort((a, b) => b.rssBytes - a.rssBytes);
}

function pageOf<T>(items: T[], req: PageRequest, text: (t: T) => string): Paged<T> {
  const q = req.query.trim().toLowerCase();
  const filtered = q ? items.filter((i) => text(i).toLowerCase().includes(q)) : items;
  return { items: filtered.slice(req.offset, req.offset + req.limit), total: filtered.length };
}

export function listPods(req: PageRequest): Promise<Paged<Pod>> {
  return delay(pageOf(podsFor(find(req.host)), req, (p) => `${p.name} ${p.namespace}`));
}

export function listContainers(req: PageRequest): Promise<Paged<Container>> {
  return delay(pageOf(containersFor(find(req.host)), req, (c) => `${c.name} ${c.image}`));
}

export function listProcesses(req: PageRequest): Promise<ProcessPage> {
  const p = processesFor(find(req.host));
  return delay(p ? { ...pageOf(p, req, (x) => x.command), enabled: true } : { items: [], total: 0, enabled: false });
}
