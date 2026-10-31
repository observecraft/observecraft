// Mock metric store + query evaluator. Each metric is backed by an "entity set" (deployments,
// hosts, services, monitors) whose members carry tags; every entity gets a smooth deterministic
// series (value noise), so the same query returns the same shape at any zoom level.
import { evalFormula } from '../../lib/metricFormula';
import { formulaQueries, groupLabel, parseMetricQuery, QUERY_IN_FORMULA, type ParsedMetricQuery } from '../../lib/metricQuery';
import type { DistributionResponse, GroupedScalar, MetricCatalogEntry, ScalarRequest, TimeAggregator, TimeseriesRequest } from '../dashboards/types';
import type { MetricSeries, TimeRange } from '../types';
import { parseQuery } from '../../lib/searchQuery';
import { rng } from './generator';
import { generatedMetrics } from './logMetrics';
import { derivedFormulas } from './derivedMetrics';
import { derivedTags, expandDerived } from '../../lib/derivedMetrics';
import { matchesTerms } from './match';

type Tags = Record<string, string>;

function hash(s: string): number {
  let h = 2166136261;
  for (let i = 0; i < s.length; i++) h = Math.imul(h ^ s.charCodeAt(i), 16777619);
  return h >>> 0;
}

const pick = <T>(r: () => number, a: readonly T[]) => a[Math.floor(r() * a.length)];

// ---------------------------------------------------------------- entity sets

const CLUSTERS = ['shopprod-core', 'platformqa-core', 'checkoutqa-core', 'billingprod-core', 'storefrontprod-core'];
const NAMESPACES = ['ns-mongo-prod', 'ns-platform', 'ns-checkout-qa', 'ns-billing-prod', 'ns-customers-be-qa', 'kube-system', 'ingress-nginx', 'observecraft', 'logging'];
const APPS = ['api', 'worker', 'backend', 'frontend', 'scheduler', 'gateway', 'consumer', 'mongo', 'redis', 'notifier', 'auth', 'billing'];
export const SERVICES = ['storefront-api', 'inventory-service', 'order-service', 'billing', 'billing-watcher', 'workflow-engine', 'pricing-service', 'notificationsrv', 'gateway', 'support-portal'];
const TEAMS = ['storefront', 'billing', 'platform', 'payments', 'checkout'];
/** Services behind the `trace.*` APM metrics (the same services as the APM mock's traces). */
const APM_SERVICES: [service: string, team: string][] = [
  ['api-gateway', 'platform'],
  ['storefront-api', 'storefront'],
  ['order-service', 'checkout'],
  ['billing', 'billing'],
  ['inventory-service', 'checkout'],
  ['catalog-service', 'catalog'],
  ['notification-worker', 'notifications'],
];

function build<T extends Tags>(n: number, seed: number, make: (r: () => number, i: number) => T): T[] {
  const r = rng(seed);
  return Array.from({ length: n }, (_, i) => make(r, i));
}

// The synthetic tests of the synthetics mock (mock/synthetics.ts): [id suffix, subtype, env, typical ms].
const SYN_TESTS: [string, string, string, number][] = [
  ['a001', 'http', 'prod', 180],
  ['a002', 'http', 'prod', 95],
  ['a003', 'http', 'prod', 420],
  ['a004', 'ssl', 'prod', 48],
  ['a005', 'dns', 'prod', 14],
  ['a007', 'http', 'prod', 130],
  ['a008', 'http', 'prod', 160],
];
const SYN_BASE = new Map(SYN_TESTS.map(([id, , , ms]) => [`f1d2c3b4-0${id.slice(1)}-4a00-8000-00000000${id}`, ms]));
const synTest = ([id, subtype, env]: (typeof SYN_TESTS)[number]) => ({ test_id: `f1d2c3b4-0${id.slice(1)}-4a00-8000-00000000${id}`, subtype, env, location: 'observecraft-server', test_type: 'api' });
const SYN_RUNS = SYN_TESTS.flatMap((t) => ['passed', 'failed'].map((result) => ({ ...synTest(t), result })));
const synOf = (subtype: string) => SYN_TESTS.filter((t) => t[1] === subtype).map(synTest);
/** A phase of a synthetic HTTP run: its share of the test's typical response time. */
const synPhase = (share: number, https = true): Pick<MetricDef, 'set' | 'unit' | 'base' | 'noise' | 'entities'> => ({
  set: 'synthetics',
  unit: 'ms',
  entities: synOf('http'),
  base: (e) => (https ? (SYN_BASE.get(e.test_id) ?? 100) * share : 0),
  noise: 0.25,
});

const SETS: Record<string, Tags[]> = {
  synthetics: SYN_RUNS,
  deployments: build(140, 11, (r, i) => {
    const cluster = pick(r, CLUSTERS);
    const ns = pick(r, NAMESPACES);
    return { kube_cluster_name: cluster, kube_namespace: ns, kube_deployment: `${pick(r, APPS)}-${i.toString(36)}`, env: cluster.includes('prod') ? 'prod' : 'qa', team: pick(r, TEAMS) };
  }),
  hosts: build(48, 12, (r, i) => {
    const cluster = pick(r, CLUSTERS);
    return { host: `i-0${(hash(`h${i}`) >>> 0).toString(16).padStart(8, '0')}${i.toString(16)}`, kube_cluster_name: cluster, env: cluster.includes('prod') ? 'prod' : 'qa', team: pick(r, TEAMS) };
  }),
  services: SERVICES.flatMap((service, i) => ['prod', 'qa'].map((env) => ({ service, env, team: TEAMS[i % TEAMS.length] }))),
  apm: APM_SERVICES.flatMap(([service, team]) => ['prod', 'staging'].map((env) => ({ service, env, team }))),
  nodes: build(36, 14, (r, i) => {
    const cluster = CLUSTERS[i % CLUSTERS.length];
    return { kube_cluster_name: cluster, kube_node: `ip-10-0-${(i * 7) % 250}-${10 + i}.ec2.internal`, status: r() < 0.06 ? 'unschedulable' : 'schedulable' };
  }),
  containers: build(90, 15, (r, i) => {
    const cluster = CLUSTERS[i % CLUSTERS.length];
    const image = pick(r, ['agent', 'kube-proxy', 'coredns', 'aws-node', 'ebs-csi-driver', 'storefront-api', 'order-service', 'catalog-service', 'nginx-ingress']);
    return { kube_cluster_name: cluster, kube_namespace: pick(r, NAMESPACES), short_image: image, image_tag: pick(r, ['7.58.0', '7.57.2', 'v1.29.3', 'v1.11.1', '1.4.2']) };
  }),
  agent_containers: build(40, 16, (r, i) => {
    const app = pick(r, ['api-gateway', 'storefront-api', 'order-service', 'billing', 'catalog-service', 'inventory-service', 'datadog-agent']);
    return {
      container_name: app,
      host: `i-0a1b2c3d4e5f6000${i % 8}`,
      short_image: app === 'datadog-agent' ? 'agent' : app,
      kube_cluster_name: pick(r, CLUSTERS),
      pod_name: `${app}-${i.toString(36)}`,
    };
  }),
  monitors: build(320, 13, (r, i) => {
    const service = r() < 0.3 ? 'undefined' : pick(r, SERVICES);
    const name = pick(r, [
      `[Subscriptions] API-Gateway Latency on {{apiid.name}} in {{aws_account.name}}`,
      `[Returns] API Gateway 4xx error rate (%) high on {{apiid.name}} in {{aws_account.name}}`,
      `[Mobile] API Gateway client-side 4xx errors on {{apiid.name}}`,
      `[Usage] - Billing - Distinct profiled hosts over the current month`,
      `[CDC] DMS RAM Utilization on {{replicationinstanceidentifier.name}}`,
      `[K8S] Node CPU utilization is high on {{host.name}}`,
      `[RPA] - Queue Items Errors`,
      `[${service}] Error rate above 5%`,
      `[${service}] p95 latency above SLO`,
    ]);
    const s = r();
    return {
      monitor_id: String(100000 + i),
      monitor_name: name,
      service,
      team: pick(r, TEAMS),
      env: r() < 0.7 ? 'prod' : 'qa',
      host: `i-0${hash(`mh${i % 40}`).toString(16)}`,
      status: s < 0.08 ? 'alert' : s < 0.1 ? 'warn' : s < 0.92 ? 'ok' : 'nodata',
    };
  }),
};

// ---------------------------------------------------------------- metric catalog

interface MetricDef {
  set: keyof typeof SETS;
  description: string;
  unit?: string;
  /** Per-entity base value; return 0 to make an entity mostly silent. */
  base: (e: Tags, r: () => number) => number;
  /** Relative amplitude of variation (0 = constant). */
  noise: number;
  integer?: boolean;
  /** Clamp to [0, max]. */
  max?: number;
  /** The entities when not the whole set (generated metrics: the log streams their query matches). */
  entities?: Tags[];
  /** The catalog's tags when not the set's. */
  tags?: string[];
  /** Seeds the series instead of the metric's name (a distribution's statistics share one shape). */
  seed?: string;
}

// Per deployment (not per metric), so desired, available and running replicas describe the same deployment.
const REPLICAS = (e: Tags, _r?: () => number) => (e.kube_deployment.startsWith('mongo') ? 1300 : 6 + (hash(e.kube_deployment) % 40));

const METRICS: Record<string, MetricDef> = {
  'kubernetes_state.deployment.count': { set: 'deployments', description: 'Deployments', base: () => 1, noise: 0, integer: true },
  'kubernetes_state.deployment.replicas': { set: 'deployments', description: 'Replicas per deployment', base: REPLICAS, noise: 0.02, integer: true },
  'kubernetes_state.deployment.replicas_desired': { set: 'deployments', description: 'Desired replicas', base: REPLICAS, noise: 0.01, integer: true },
  'kubernetes_state.deployment.replicas_available': { set: 'deployments', description: 'Available replicas', base: (e, r) => REPLICAS(e, r) * 0.98, noise: 0.02, integer: true },
  'kubernetes_state.deployment.replicas_updated': { set: 'deployments', description: 'Updated replicas', base: (e, r) => REPLICAS(e, r) * 0.99, noise: 0.01, integer: true },
  'kubernetes_state.deployment.replicas_unavailable': {
    set: 'deployments',
    description: 'Unavailable replicas',
    base: (_e, r) => (r() < 0.15 ? 1 + Math.floor(r() * 4) : 0),
    noise: 0.4,
    integer: true,
  },
  'kubernetes_state.deployment.replicas_desired_not_available': {
    set: 'deployments',
    description: 'Desired but not available replicas',
    base: (_e, r) => (r() < 0.6 ? 2 + r() * 30 : 0),
    noise: 0.5,
    integer: true,
  },
  'kubernetes_state.deployment.outdated': { set: 'deployments', description: 'Deployments with outdated replicas', base: (_e, r) => (r() < 0.01 ? 1 : 0), noise: 0, integer: true },
  'kubernetes_state.pod.running': { set: 'deployments', description: 'Running pods', base: (e, r) => REPLICAS(e, r) * 0.97, noise: 0.02, integer: true },
  'kubernetes_state.pod.pending': { set: 'deployments', description: 'Pending pods', base: (_e, r) => (r() < 0.1 ? 1 + r() * 3 : 0), noise: 0.6, integer: true },
  'kubernetes_state.pod.failed': { set: 'deployments', description: 'Failed pods', base: (_e, r) => (r() < 0.05 ? 1 : 0), noise: 0.5, integer: true },
  'kubernetes.cpu.usage.total': { set: 'deployments', description: 'CPU used by containers', unit: 'millicore', base: (_e, r) => 50 + r() * 900, noise: 0.25 },
  'kubernetes.memory.usage': { set: 'deployments', description: 'Memory used by containers', unit: 'MiB', base: (_e, r) => 120 + r() * 1500, noise: 0.08 },
  'kubernetes.containers.restarts': { set: 'deployments', description: 'Container restarts', base: (_e, r) => (r() < 0.2 ? r() * 8 : 0), noise: 0.8, integer: true },
  'kubernetes.cpu.requests': { set: 'deployments', description: 'CPU requested by containers', unit: 'core', base: (e) => 0.25 * (1 + (hash(e.kube_deployment) % 12)), noise: 0 },
  'kubernetes.cpu.limits': { set: 'deployments', description: 'CPU limit of containers', unit: 'core', base: (e) => 0.5 * (1 + (hash(e.kube_deployment) % 16)), noise: 0 },
  'kubernetes.memory.requests': { set: 'deployments', description: 'Memory requested by containers', unit: 'MiB', base: (e) => 256 * (1 + (hash(e.kube_deployment) % 8)), noise: 0 },
  'kubernetes.memory.limits': { set: 'deployments', description: 'Memory limit of containers', unit: 'MiB', base: (e) => 512 * (1 + (hash(e.kube_deployment) % 8)), noise: 0 },
  'system.cpu.num_cores': { set: 'hosts', description: 'Number of CPU cores', unit: 'core', base: (_e, r) => 2 ** (1 + Math.floor(r() * 4)), noise: 0, integer: true },
  'system.cpu.system': { set: 'hosts', description: 'CPU time in kernel space', unit: '%', base: (_e, r) => 1 + r() * 8, noise: 0.2, max: 100 },
  'system.mem.total': { set: 'hosts', description: 'Total memory', unit: 'mebibyte', base: (_e, r) => 1024 * 2 ** (2 + Math.floor(r() * 4)), noise: 0 },
  'system.mem.free': { set: 'hosts', description: 'Free memory', unit: 'mebibyte', base: (_e, r) => 1024 * (1 + r() * 6), noise: 0.05 },
  'system.cpu.user': { set: 'hosts', description: 'CPU time in user space', unit: '%', base: (_e, r) => 10 + Math.pow(r(), 1.5) * 80, noise: 0.15, max: 100 },
  'system.cpu.idle': { set: 'hosts', description: 'CPU idle', unit: '%', base: (_e, r) => 20 + r() * 70, noise: 0.1, max: 100 },
  'system.mem.used': { set: 'hosts', description: 'Memory used', unit: 'GiB', base: (_e, r) => 2 + r() * 12, noise: 0.05 },
  'system.load.1': { set: 'hosts', description: 'Load average (1m)', base: (_e, r) => 0.5 + r() * 12, noise: 0.3 },
  'system.load.5': { set: 'hosts', description: 'Load average (5m)', base: (_e, r) => 0.5 + r() * 9, noise: 0.2 },
  'system.mem.pct_usable': { set: 'hosts', description: 'Share of memory usable', unit: 'fraction', base: (_e, r) => 0.06 + r() * 0.8, noise: 0.05, max: 1 },
  'system.net.bytes_rcvd': { set: 'hosts', description: 'Bytes received', unit: 'B/s', base: (_e, r) => 2e5 + Math.pow(r(), 3) * 8e7, noise: 0.35 },
  'system.disk.in_use': { set: 'hosts', description: 'Disk usage fraction', unit: '%', base: (_e, r) => 20 + r() * 70, noise: 0.01, max: 100 },
  'trace.http.request.hits': { set: 'services', description: 'HTTP requests', unit: 'req/s', base: (e, r) => (e.env === 'prod' ? 40 + r() * 400 : 5 + r() * 40), noise: 0.2 },
  'trace.http.request.errors': { set: 'services', description: 'HTTP request errors', unit: 'err/s', base: (_e, r) => r() * 6, noise: 0.5 },
  'trace.http.request.error_rate': { set: 'services', description: 'HTTP error rate', unit: '%', base: (_e, r) => 0.2 + r() * 4, noise: 0.4, max: 100 },
  'trace.http.request.duration.p95': { set: 'services', description: 'p95 latency', unit: 'ms', base: (_e, r) => 40 + r() * 400, noise: 0.15 },
  'trace.requests': { set: 'apm', description: 'Requests', unit: 'req/s', base: (e, r) => (e.env === 'prod' ? 30 + r() * 300 : 2 + r() * 20), noise: 0.1 },
  'trace.errors': { set: 'apm', description: 'Errored requests', unit: 'err/s', base: (e, r) => (e.service === 'order-service' ? 14 : r() * 3), noise: 0.4 },
  // order-service fails a growing share of checkouts; the others stay below the recommended thresholds.
  'trace.error_rate': { set: 'apm', description: 'Share of requests in error', unit: '%', base: (e, r) => (e.service === 'order-service' ? 6.6 : 0.2 + r() * 1.6), noise: 0.15, max: 100 },
  'trace.latency.p50': { set: 'apm', description: 'Median latency', unit: 'ms', base: (e, r) => (e.service === 'catalog-service' ? 210 : 12 + r() * 60), noise: 0.12 },
  // catalog-service's slower product query (see the APM mock) keeps its p95 above 300 ms.
  'trace.latency.p95': { set: 'apm', description: 'p95 latency', unit: 'ms', base: (e, r) => (e.service === 'catalog-service' ? 410 : 60 + r() * 200), noise: 0.1 },
  'container.cpu.usage': { set: 'agent_containers', description: 'CPU used by the container', unit: 'core', base: (_e, r) => 0.01 + Math.pow(r(), 2) * 1.2, noise: 0.35 },
  'container.memory.rss': { set: 'agent_containers', description: 'Resident memory of the container', unit: 'byte', base: (_e, r) => (60 + r() * 900) * 2 ** 20, noise: 0.05 },
  'container.net.sent': { set: 'agent_containers', description: 'Bytes sent per second', unit: 'byte/second', base: (_e, r) => 200 + r() * 40_000, noise: 0.5 },
  'container.net.rcvd': { set: 'agent_containers', description: 'Bytes received per second', unit: 'byte/second', base: (_e, r) => 500 + r() * 90_000, noise: 0.5 },
  'kubernetes_state.node.count': { set: 'nodes', description: 'Nodes', unit: 'node', base: () => 1, noise: 0, integer: true },
  'kubernetes.containers.running': { set: 'containers', description: 'Running containers', base: (_e, r) => 1 + Math.floor(r() * 6), noise: 0, integer: true },
  'kubernetes_state.node.status': { set: 'nodes', description: 'Nodes by status', base: () => 1, noise: 0, integer: true },
  // Synthetic API tests: runs by result, response times and the phases of HTTP runs.
  'synthetics.test_runs': {
    set: 'synthetics',
    description: 'Synthetic test runs',
    entities: SYN_RUNS,
    // Checkout API health (a002) fails; the others pass nearly always.
    base: (e) => (e.result === 'passed' ? (e.test_id.endsWith('a002') ? 0.8 : 1) : e.test_id.endsWith('a002') ? 0.25 : 0),
    noise: 0.2,
  },
  'synthetics.http.response.time': { ...synPhase(1), description: 'Synthetic test response time (ms)' },
  'synthetics.http.dns.time': { ...synPhase(0.12), description: 'Synthetic HTTP DNS lookup time (ms)' },
  'synthetics.http.connect.time': { ...synPhase(0.08), description: 'Synthetic HTTP connection time (ms)' },
  'synthetics.http.ssl.time': { ...synPhase(0.1), description: 'Synthetic HTTP TLS handshake time (ms)' },
  'synthetics.http.firstbyte.time': { ...synPhase(0.6), description: 'Synthetic HTTP time to first byte (ms)' },
  'synthetics.http.download.time': { ...synPhase(0.02), description: 'Synthetic HTTP download time (ms)' },
  'synthetics.ssl.handshake.time': { set: 'synthetics', description: 'Synthetic test response time (ms)', unit: 'ms', entities: synOf('ssl'), base: (e) => SYN_BASE.get(e.test_id) ?? 50, noise: 0.2 },
  'synthetics.dns.response.time': { set: 'synthetics', description: 'Synthetic test response time (ms)', unit: 'ms', entities: synOf('dns'), base: (e) => SYN_BASE.get(e.test_id) ?? 15, noise: 0.3 },
  'observecraft.monitor.notifications': {
    set: 'monitors',
    description: 'Monitor notifications sent',
    base: (e, r) => (e.status === 'ok' ? r() * 0.12 : e.status === 'nodata' ? 0 : r() * 0.25),
    noise: 1.2,
    integer: false,
  },
};

// ---------------------------------------------------------------- generated metrics (mock/logMetrics.ts)

const LOG_SERVICES: [service: string, namespace: string][] = [
  ['api-gateway', 'ns-platform'],
  ['storefront-api', 'ns-storefront'],
  ['order-service', 'ns-checkout'],
  ['billing', 'ns-billing'],
  ['catalog-service', 'ns-catalog'],
  ['inventory-service', 'ns-checkout'],
];
const LOG_STATUSES: [status: string, code: string][] = [
  ['info', '200'],
  ['warn', '404'],
  ['error', '502'],
];

/** One entity per (service, status, env): the log streams generated metrics count. */
const LOG_STREAMS: Tags[] = LOG_SERVICES.flatMap(([service, namespace], i) =>
  LOG_STATUSES.flatMap(([status, code]) =>
    ['prod', 'staging'].map((env) => ({
      service,
      status,
      env,
      host: `i-0a1b2c3d4e5f6000${i}`,
      source: service === 'api-gateway' ? 'nginx' : 'app',
      kube_namespace: namespace,
      'http.status_code': code,
    })),
  ),
);

/** Each definition as a metric over the log streams its query matches, tagged by its group-by tag names. */
function generatedMetricDefs(): Record<string, MetricDef> {
  return Object.fromEntries(
    generatedMetrics().flatMap((m) => {
      const terms = parseQuery(m.query);
      const groupBy = m.groupBy.map((g) => ({ field: g.path.replace(/^@/, ''), tag: g.tagName || g.path.replace(/^@/, '') }));
      const entities = LOG_STREAMS.filter((e) => matchesTerms((k) => e[k.replace(/^@/, '')], '', terms)).map((e) => ({
        ...e,
        ...Object.fromEntries(groupBy.map((g) => [g.tag, e[g.field] ?? 'N/A'])),
      }));
      const matched = `logs matching: ${m.query || '*'}`;
      const rate = (e: Tags, r: () => number) => (e.status === 'info' ? 20 + r() * 80 : e.status === 'warn' ? 1 + r() * 6 : 0.2 + r() * 3);
      const common = { set: 'services', entities, tags: groupBy.map((g) => g.tag), noise: 0.3, seed: m.name };
      if (m.aggregation === 'count') return [[m.name, { ...common, description: `Count of ${matched}`, unit: 'logs/s', base: rate }]];
      // A distribution is queried by statistic (`<name>.p95`); the statistics share one series shape.
      return DIST_STAT_SCALES.map(([stat, scale]) => [
        `${m.name}.${stat}`,
        { ...common, description: `${stat} of ${m.path} in ${matched}`, base: stat === 'count' ? rate : (_e: Tags, r: () => number) => (20 + r() * 300) * scale },
      ]);
    }),
  );
}

/** Distribution statistics and their size relative to the average. */
const DIST_STAT_SCALES: [stat: string, scale: number][] = [
  ['count', 1],
  ['avg', 1],
  ['p50', 0.8],
  ['p95', 2.2],
  ['p99', 3.5],
  ['max', 5],
];

/** A metric's definition: a built-in one or a generated metric. */
function lookup(name: string): MetricDef | undefined {
  return METRICS[name] ?? generatedMetricDefs()[name];
}

const entitiesOf = (d: MetricDef) => d.entities ?? SETS[d.set];

export function metricCatalog(): MetricCatalogEntry[] {
  const defs = { ...METRICS, ...generatedMetricDefs() };
  const entries: MetricCatalogEntry[] = Object.entries(defs).map(([name, d]) => ({ name, tags: d.tags ?? Object.keys(SETS[d.set][0]), unit: d.unit, description: d.description }));
  // Derived metrics (mock/derivedMetrics.ts), with the tags of the metrics they read.
  const tagsOf = (metric: string) => entries.find((e) => e.name === metric)?.tags ?? [];
  for (const [name, formula] of Object.entries(derivedFormulas())) entries.push({ name, tags: derivedTags(formula, tagsOf), description: `Derived: ${formula}`, kind: 'derived' });
  return entries;
}

/** Metrics by series: one series per entity of the metric's set. */
export function metricVolume(): { totalSeries: number; metrics: { name: string; kind: string; series: number; share: number }[] } {
  const rows = Object.entries({ ...METRICS, ...generatedMetricDefs() }).map(([name, d]) => ({ name, kind: 'gauge', series: entitiesOf(d).length }));
  const totalSeries = rows.reduce((n, r) => n + r.series, 0);
  const metrics = rows.map((r) => ({ ...r, share: totalSeries ? r.series / totalSeries : 0 })).sort((a, b) => b.series - a.series || a.name.localeCompare(b.name));
  return { totalSeries, metrics };
}

export function tagValues(key: string): string[] {
  const out = new Set<string>();
  for (const set of Object.values(SETS)) for (const e of set) if (e[key] !== undefined) out.add(e[key]);
  return [...out].sort();
}

export function monitorEntities(): Tags[] {
  return SETS.monitors;
}

// ---------------------------------------------------------------- series

const KNOT_MS = 5 * 60e3; // value-noise knot spacing

function knot(seed: number, i: number): number {
  const x = Math.sin(seed * 0.0001 + i * 12.9898) * 43758.5453;
  return x - Math.floor(x) - 0.5; // [-0.5, 0.5)
}

function smooth(seed: number, t: number): number {
  const p = t / KNOT_MS;
  const i = Math.floor(p);
  const f = p - i;
  const u = f * f * (3 - 2 * f);
  return knot(seed, i) * (1 - u) + knot(seed, i + 1) * u + 0.35 * knot(seed + 7, Math.floor(t / 60e3));
}

function entityValue(metric: string, def: MetricDef, e: Tags, t: number): number {
  const key = `${def.seed ?? metric}|${Object.values(e).join('|')}`;
  const seed = hash(key);
  const base = def.base(e, rng(seed));
  if (base === 0 && def.noise < 0.5) return 0;
  let v = base * (1 + def.noise * 2 * smooth(seed, t));
  if (base === 0) v = Math.max(0, smooth(seed, t) - 0.3) * 4; // mostly-silent entities spike occasionally
  v = Math.max(0, def.max !== undefined ? Math.min(def.max, v) : v);
  return def.integer ? Math.round(v) : v;
}

function stepFor(range: TimeRange, points: number): number {
  const raw = (range.to - range.from) / points;
  const steps = [10e3, 20e3, 30e3, 60e3, 120e3, 300e3, 600e3, 1800e3, 3600e3, 3 * 3600e3, 6 * 3600e3];
  return steps.find((s) => s >= raw) ?? 12 * 3600e3;
}

/** `key:value`, with `*` wildcards (`kube_deployment:*` is any entity that has the tag). */
function matches(e: Tags, q: ParsedMetricQuery): boolean {
  return q.filters.every((f) => {
    const v = e[f.key];
    const hit = f.value.includes('*')
      ? v !== undefined &&
        new RegExp(
          `^${f.value
            .split('*')
            .map((p) => p.replace(/[.+?^${}()|[\]\\]/g, '\\$&'))
            .join('.*')}$`,
        ).test(v)
      : v === f.value;
    return hit !== f.negate;
  });
}

function combine(values: number[], agg: ParsedMetricQuery['aggregator']): number {
  if (values.length === 0) return 0;
  switch (agg) {
    case 'sum':
      return values.reduce((a, b) => a + b, 0);
    case 'min':
      return Math.min(...values);
    case 'max':
      return Math.max(...values);
    default:
      return values.reduce((a, b) => a + b, 0) / values.length;
  }
}

export interface GroupSeries {
  tags: Tags;
  points: [number, number][];
}

function evaluate(query: string, range: TimeRange, points: number): GroupSeries[] {
  const q = parseMetricQuery(query);
  if (!q) throw new Error(`Invalid query: ${query}`);
  const def = lookup(q.metric);
  if (!def) throw new Error(`Unknown metric: ${q.metric}`);
  const step = stepFor(range, points);
  const entities = entitiesOf(def).filter((e) => matches(e, q));
  const groups = new Map<string, { tags: Tags; members: Tags[] }>();
  for (const e of entities) {
    const tags = Object.fromEntries(q.by.map((k) => [k, e[k] ?? 'N/A']));
    const key = JSON.stringify(tags);
    const g = groups.get(key) ?? { tags, members: [] };
    g.members.push(e);
    groups.set(key, g);
  }
  const start = Math.floor(range.from / step) * step;
  const out: GroupSeries[] = [];
  for (const g of groups.values()) {
    const pts: [number, number][] = [];
    for (let t = start; t <= range.to; t += step) {
      const v = combine(
        g.members.map((e) => entityValue(q.metric, def, e, t)),
        q.aggregator,
      );
      pts.push([t, Math.round(v * 100) / 100]);
    }
    out.push({ tags: g.tags, points: pts });
  }
  return out;
}

/**
 * A metric query's groups over `range` (`points` ≈ resolution), for monitors: an unknown metric has no
 * series (no data), like a metric that has not been reported yet; an unparsable query throws.
 */
export function evaluateGroups(query: string, range: TimeRange, points: number): GroupSeries[] {
  const q = parseMetricQuery(query);
  if (!q) throw new Error(`Invalid query: ${query}`);
  return lookup(q.metric) ? evaluate(query, range, points) : [];
}

function reduce(points: [number, number][], agg: TimeAggregator): number {
  const vs = points.map((p) => p[1]);
  if (vs.length === 0) return 0;
  switch (agg) {
    case 'last':
      return vs[vs.length - 1];
    case 'sum':
      return vs.reduce((a, b) => a + b, 0);
    case 'min':
      return Math.min(...vs);
    case 'max':
      return Math.max(...vs);
    default:
      return vs.reduce((a, b) => a + b, 0) / vs.length;
  }
}

const delay = <T>(v: T, ms = 60 + Math.random() * 140) => new Promise<T>((r) => setTimeout(() => r(v), ms));
const attempt = <T>(fn: () => T): Promise<T> => {
  try {
    return delay(fn());
  } catch (e) {
    return Promise.reject(e);
  }
};

/** A formula between metric queries: each query's groups joined by tags (a tag-less query applies to all). */
function evaluateFormula(formula: string, range: TimeRange, points: number): GroupSeries[] {
  const queries = formulaQueries(formula);
  if (!queries) throw new Error(`Invalid query: ${formula}`);
  const texts = formula.match(QUERY_IN_FORMULA) ?? [];
  let n = 0;
  const shape = formula.replace(QUERY_IN_FORMULA, () => `Q${n++}`).replace(/default_zero\(/g, '(');
  const results = texts.map((t) => evaluate(t, range, points));
  const keyed = results.map((gs) => new Map(gs.map((g) => [JSON.stringify(g.tags), new Map(g.points)])));
  const keys = [
    ...new Set(
      results
        .flat()
        .filter((g) => Object.keys(g.tags).length)
        .map((g) => JSON.stringify(g.tags)),
    ),
  ];
  const times = [...new Set(results.flat().flatMap((g) => g.points.map((p) => p[0])))].sort((a, b) => a - b);
  return (keys.length ? keys : ['{}']).map((key) => ({
    tags: JSON.parse(key) as Tags,
    points: times.flatMap((t) => {
      const v = evalFormula(
        shape,
        keyed.map((m) => (m.get(key) ?? m.get('{}'))?.get(t) ?? null),
      );
      return v === null ? [] : [[t, Math.round(v * 100) / 100] as [number, number]];
    }),
  }));
}

/** A query's groups; a derived metric's (mock/derivedMetrics.ts) are its formula's. */
function evaluateAny(query: string, range: TimeRange, points: number): GroupSeries[] {
  const resolved = expandDerived(query, derivedFormulas());
  return resolved === query ? evaluate(query, range, points) : evaluateFormula(resolved, range, points);
}

export function queryTimeseries(req: TimeseriesRequest): Promise<MetricSeries[]> {
  return attempt(() =>
    req.queries.flatMap((raw) => {
      // The mock averages every interval: a `.rollup(method)` suffix is accepted and ignored.
      const query = raw.replace(/\.rollup\([^()]*\)$/, '');
      const resolved = expandDerived(query, derivedFormulas());
      const q = resolved === query ? parseMetricQuery(query) : null;
      if (!q) return evaluateFormula(resolved, req.range, req.points ?? 120).map((g) => ({ name: Object.keys(g.tags).length ? groupLabel(g.tags) : query, points: g.points }));
      return evaluate(query, req.range, req.points ?? 120).map((g) => ({
        // Ungrouped series are named by their full query so several queries stay distinct.
        name: q?.by.length ? groupLabel(g.tags) : query,
        points: g.points,
      }));
    }),
  );
}

export function queryScalar(req: ScalarRequest): Promise<number> {
  return attempt(() => {
    const groups = evaluateAny(req.query, req.range, 60);
    // A grouped query reduced to one number: combine groups per timestamp first.
    const q = parseMetricQuery(req.query)!;
    if (groups.length === 0) return 0;
    const merged = groups[0].points.map(
      ([t], i) =>
        [
          t,
          combine(
            groups.map((g) => g.points[i]?.[1] ?? 0),
            q.aggregator,
          ),
        ] as [number, number],
    );
    return reduce(merged, req.aggregator);
  });
}

export function queryGrouped(req: ScalarRequest): Promise<GroupedScalar[]> {
  return attempt(() =>
    evaluateAny(req.query, req.range, 60)
      .map((g) => ({ group: groupLabel(g.tags), tags: g.tags, value: Math.round(reduce(g.points, req.aggregator) * 100) / 100 }))
      .sort((a, b) => b.value - a.value),
  );
}

export function queryDistribution(req: { query: string; range: TimeRange }): Promise<DistributionResponse> {
  return attempt(() => {
    const q = parseMetricQuery(req.query);
    if (!q) throw new Error(`Invalid query: ${req.query}`);
    const def = lookup(q.metric);
    if (!def) throw new Error(`Unknown metric: ${q.metric}`);
    // Distribution across entities: re-run without grouping but keep each entity separate.
    const perEntity = evaluate(
      `${q.aggregator}:${q.metric}{${q.filters.map((f) => `${f.negate ? '!' : ''}${f.key}:${f.value}`).join(',') || '*'}} by {${Object.keys(entitiesOf(def)[0] ?? {}).join(',')}}`,
      req.range,
      40,
    );
    const all = perEntity.flatMap((s) => s.points.map((p) => p[1]));
    const max = Math.max(1, ...all);
    const bucketCount = 10;
    const buckets = Array.from({ length: bucketCount }, (_, i) => Math.round(((i + 1) * max) / bucketCount));
    const times = perEntity[0]?.points.map((p) => p[0]) ?? [];
    const counts = times.map((_, ti) => {
      const row = new Array<number>(bucketCount).fill(0);
      for (const s of perEntity) {
        const v = s.points[ti][1];
        row[Math.min(bucketCount - 1, Math.floor((v / max) * bucketCount))]++;
      }
      return row;
    });
    return { buckets, times, counts };
  });
}

/** A metric's series (entities) and its kind, for the catalog's per-metric details; undefined when unknown. */
export function metricEntities(name: string): Tags[] | undefined {
  const def = lookup(name);
  return def ? entitiesOf(def) : undefined;
}
