// Mock metric store + query evaluator. Each metric is backed by an "entity set" (deployments,
// hosts, services, monitors) whose members carry tags; every entity gets a smooth deterministic
// series (value noise), so the same query returns the same shape at any zoom level.
import { groupLabel, parseMetricQuery, type ParsedMetricQuery } from '../../lib/metricQuery';
import type { DistributionResponse, GroupedScalar, MetricCatalogEntry, ScalarRequest, TimeAggregator, TimeseriesRequest } from '../dashboards/types';
import type { MetricSeries, TimeRange } from '../types';
import { rng } from './generator';

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

function build<T extends Tags>(n: number, seed: number, make: (r: () => number, i: number) => T): T[] {
  const r = rng(seed);
  return Array.from({ length: n }, (_, i) => make(r, i));
}

const SETS: Record<string, Tags[]> = {
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
}

const REPLICAS = (e: Tags, r: () => number) => (e.kube_deployment.startsWith('mongo') ? 1300 : 6 + Math.floor(r() * 40));

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
  'system.cpu.user': { set: 'hosts', description: 'CPU time in user space', unit: '%', base: (_e, r) => 10 + Math.pow(r(), 1.5) * 80, noise: 0.15, max: 100 },
  'system.cpu.idle': { set: 'hosts', description: 'CPU idle', unit: '%', base: (_e, r) => 20 + r() * 70, noise: 0.1, max: 100 },
  'system.mem.used': { set: 'hosts', description: 'Memory used', unit: 'GiB', base: (_e, r) => 2 + r() * 12, noise: 0.05 },
  'system.load.1': { set: 'hosts', description: 'Load average (1m)', base: (_e, r) => 0.5 + r() * 12, noise: 0.3 },
  'system.disk.in_use': { set: 'hosts', description: 'Disk usage fraction', unit: '%', base: (_e, r) => 20 + r() * 70, noise: 0.01, max: 100 },
  'trace.http.request.hits': { set: 'services', description: 'HTTP requests', unit: 'req/s', base: (e, r) => (e.env === 'prod' ? 40 + r() * 400 : 5 + r() * 40), noise: 0.2 },
  'trace.http.request.errors': { set: 'services', description: 'HTTP request errors', unit: 'err/s', base: (_e, r) => r() * 6, noise: 0.5 },
  'trace.http.request.error_rate': { set: 'services', description: 'HTTP error rate', unit: '%', base: (_e, r) => 0.2 + r() * 4, noise: 0.4, max: 100 },
  'trace.http.request.duration.p95': { set: 'services', description: 'p95 latency', unit: 'ms', base: (_e, r) => 40 + r() * 400, noise: 0.15 },
  'observecraft.monitor.notifications': {
    set: 'monitors',
    description: 'Monitor notifications sent',
    base: (e, r) => (e.status === 'ok' ? r() * 0.12 : e.status === 'nodata' ? 0 : r() * 0.25),
    noise: 1.2,
    integer: false,
  },
};

export function metricCatalog(): MetricCatalogEntry[] {
  return Object.entries(METRICS).map(([name, d]) => ({ name, tags: Object.keys(SETS[d.set][0]), unit: d.unit, description: d.description }));
}

/** Metrics by series: one series per entity of the metric's set. */
export function metricVolume(): { totalSeries: number; metrics: { name: string; kind: string; series: number; share: number }[] } {
  const rows = Object.entries(METRICS).map(([name, d]) => ({ name, kind: 'gauge', series: SETS[d.set].length }));
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
  const key = `${metric}|${Object.values(e).join('|')}`;
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

function matches(e: Tags, q: ParsedMetricQuery): boolean {
  return q.filters.every((f) => (e[f.key] === f.value) !== f.negate);
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

interface GroupSeries {
  tags: Tags;
  points: [number, number][];
}

function evaluate(query: string, range: TimeRange, points: number): GroupSeries[] {
  const q = parseMetricQuery(query);
  if (!q) throw new Error(`Invalid query: ${query}`);
  const def = METRICS[q.metric];
  if (!def) throw new Error(`Unknown metric: ${q.metric}`);
  const step = stepFor(range, points);
  const entities = SETS[def.set].filter((e) => matches(e, q));
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

export function queryTimeseries(req: TimeseriesRequest): Promise<MetricSeries[]> {
  return attempt(() =>
    req.queries.flatMap((query) => {
      const q = parseMetricQuery(query)!;
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
    const groups = evaluate(req.query, req.range, 60);
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
    evaluate(req.query, req.range, 60)
      .map((g) => ({ group: groupLabel(g.tags), tags: g.tags, value: Math.round(reduce(g.points, req.aggregator) * 100) / 100 }))
      .sort((a, b) => b.value - a.value),
  );
}

export function queryDistribution(req: { query: string; range: TimeRange }): Promise<DistributionResponse> {
  return attempt(() => {
    const q = parseMetricQuery(req.query);
    if (!q) throw new Error(`Invalid query: ${req.query}`);
    // Distribution across entities: re-run without grouping but keep each entity separate.
    const perEntity = evaluate(
      `${q.aggregator}:${q.metric}{${q.filters.map((f) => `${f.negate ? '!' : ''}${f.key}:${f.value}`).join(',') || '*'}} by {${Object.keys(SETS[METRICS[q.metric].set][0]).join(',')}}`,
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
