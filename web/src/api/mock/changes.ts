// In-browser stand-in for Infrastructure → Changes (see api/changes/client.ts): a week of changes of
// the e-commerce demo (rollouts of the mock Kubernetes deployments, scaling, config edits, crashes,
// version deployments, CI deploys, feature flags and monitor edits), searched like the server does.
import type { Change, ChangeBucket, ChangePage, ChangeSearchRequest, ChangeType } from '../changes/client';
import type { CatalogFacet } from '../catalog/client';
import { parseQuery, type Term } from '../../lib/searchQuery';
import { rng } from './generator';

const HOUR = 3600e3;
const DAY = 24 * HOUR;
const FACETS = ['change_type', 'source', 'resource_type', 'service', 'env', 'cluster', 'namespace', 'author'];
const SERVICES = [
  { name: 'api-gateway', ns: 'shop' },
  { name: 'storefront-api', ns: 'shop' },
  { name: 'order-service', ns: 'shop' },
  { name: 'catalog-service', ns: 'shop' },
  { name: 'inventory-service', ns: 'shop' },
  { name: 'billing', ns: 'payments' },
];
const CLUSTERS = [
  { cluster: 'prod-eu', env: 'prod' },
  { cluster: 'staging-eu', env: 'staging' },
];
const AUTHORS = ['Alex Morgan', 'Priya Shah', 'dev@example.com', 'release-bot@example.com'];

const delay = <T>(value: T, ms = 80 + Math.random() * 120) => new Promise<T>((r) => setTimeout(() => r(value), ms));

function change(id: string, timestamp: number, changeType: ChangeType, source: Change['source'], resourceType: string, resource: string, summary: string, extra: Partial<Change> = {}): Change {
  return {
    id,
    timestamp,
    changeType,
    source,
    resourceType,
    resource,
    summary,
    service: null,
    env: null,
    cluster: null,
    namespace: null,
    author: null,
    previous: null,
    current: null,
    details: {},
    kubeKind: null,
    kubeUid: null,
    ...extra,
  };
}

let cache: { at: number; items: Change[] } | null = null;

/** A week of demo changes ending now (regenerated every few minutes so "now" moves). */
function changes(): Change[] {
  const now = Date.now();
  if (cache && now - cache.at < 5 * 60e3) return cache.items;
  const r = rng(7);
  const out: Change[] = [];
  const pick = <T>(arr: T[]) => arr[Math.floor(r() * arr.length)];
  for (const { cluster, env } of CLUSTERS) {
    for (const s of SERVICES) {
      const k8s = { service: s.name, env, cluster, namespace: s.ns, kubeKind: 'deployments' as const, kubeUid: `${cluster}-dep-${s.name}` };
      // A rollout every day or two, newest last, each against the previous image.
      let minor = 2 + Math.floor(r() * 3);
      for (let t = now - 7 * DAY + r() * DAY; t < now; t += DAY * (0.8 + r() * 1.5)) {
        const prev = `registry.example.com/${s.name}:2.${minor}.0`;
        minor += 1;
        const cur = `registry.example.com/${s.name}:2.${minor}.0`;
        const ts = Math.round(t);
        out.push(
          change(`rs:${cluster}:${s.name}:${minor}`, ts, 'rollout', 'kubernetes', 'kube_deployment', s.name, `Rolled out ${s.name}: 2.${minor - 1}.0 → 2.${minor}.0`, {
            ...k8s,
            previous: prev,
            current: cur,
            details: { revision: String(minor), replica_set: `${s.name}-${(ts % 0xfffff).toString(16)}` },
          }),
        );
        if (env === 'prod') {
          out.push(
            change(`apm:${s.name}:${env}:2.${minor}.0`, ts + 90e3, 'deployment', 'apm', 'service', s.name, `Deployed version 2.${minor}.0 (was 2.${minor - 1}.0)`, {
              service: s.name,
              env,
              previous: `2.${minor - 1}.0`,
              current: `2.${minor}.0`,
              details: { version: `2.${minor}.0` },
            }),
          );
          const sha = Math.floor(r() * 0xffffffff)
            .toString(16)
            .padStart(8, '0');
          out.push(
            change(`ci:${s.name}:${ts}`, ts - 4 * 60e3, 'deployment', 'ci', 'service', s.name, `deploy deployed ${s.name} at ${sha} to production`, {
              service: s.name,
              env: 'production',
              author: pick(AUTHORS.filter((a) => a.includes('@'))),
              current: sha,
              details: { repository: `example-org/${s.name}`, pipeline: 'deploy', job: 'deploy-production', status: 'success' },
            }),
          );
        }
      }
      // Scaling now and then.
      const scaledAt = Math.round(now - r() * 6 * DAY);
      const replicas = 2 + Math.floor(r() * 3);
      out.push(
        change(`gen:${cluster}:${s.name}:scale`, scaledAt, 'scale', 'kubernetes', 'kube_deployment', s.name, `Scaled from ${replicas} to ${replicas + 1} replicas`, {
          ...k8s,
          previous: String(replicas),
          current: String(replicas + 1),
          details: { generation: `${minor} → ${minor + 1}` },
        }),
      );
    }
    // A strategy edit and the crash looping inventory-service.
    out.push(
      change(`gen:${cluster}:catalog-service:spec`, Math.round(now - 2.5 * DAY), 'config', 'kubernetes', 'kube_deployment', 'catalog-service', 'Spec changed: strategy', {
        service: 'catalog-service',
        env,
        cluster,
        namespace: 'shop',
        kubeKind: 'deployments',
        kubeUid: `${cluster}-dep-catalog-service`,
        details: { generation: '7 → 8' },
      }),
    );
    for (let i = 0; i < 4; i++) {
      out.push(
        change(
          `k8sev:${cluster}:inventory:${i}`,
          Math.round(now - (i * 7 + 1) * HOUR - r() * HOUR),
          'crash',
          'kubernetes',
          'kube_pod',
          'inventory-service-7d9f8c6b5-xkqzt',
          `Crash looping (${3 + i}×): Back-off restarting failed container inventory-service`,
          {
            service: 'inventory-service',
            env,
            cluster,
            namespace: 'shop',
            details: { reason: 'BackOff' },
          },
        ),
      );
    }
  }
  out.push(
    change('event:flag-1', Math.round(now - 30 * HOUR), 'feature_flag', 'events', 'service', 'storefront-api', 'Feature flag checkout-v2 enabled for 10% of users', {
      service: 'storefront-api',
      env: 'prod',
      author: 'Priya Shah',
      details: { event_source: 'api', text: 'Gradual rollout of the new checkout flow.' },
    }),
    change('event:flag-2', Math.round(now - 5 * HOUR), 'feature_flag', 'events', 'service', 'storefront-api', 'Feature flag checkout-v2 enabled for 50% of users', {
      service: 'storefront-api',
      env: 'prod',
      author: 'Priya Shah',
      details: { event_source: 'api' },
    }),
    change('audit:1', Math.round(now - 3 * HOUR), 'config', 'audit', 'monitor', 'Checkout error rate', 'Monitor updated Checkout error rate', {
      author: 'Alex Morgan',
      details: { action: 'monitor.update', target: 'monitor:1' },
    }),
    change('audit:2', Math.round(now - 50 * HOUR), 'config', 'audit', 'log_pipeline', 'nginx', 'Log pipeline updated nginx', { author: 'Alex Morgan', details: { action: 'log_pipeline.updated' } }),
  );
  cache = { at: now, items: out.filter((c) => c.timestamp <= now) };
  return cache.items;
}

function values(c: Change, key: string): string[] {
  const one = (v: string | null | undefined) => (v ? [v] : []);
  switch (key) {
    case 'change_type':
    case 'type':
      return [c.changeType];
    case 'source':
      return [c.source];
    case 'resource_type':
      return [c.resourceType];
    case 'resource':
      return [c.resource];
    case 'service':
    case 'env':
    case 'cluster':
    case 'namespace':
    case 'author':
      return one(c[key]);
    case 'kube_cluster_name':
      return one(c.cluster);
    case 'kube_namespace':
      return one(c.namespace);
  }
  return one(c.details[key]);
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

function matches(c: Change, terms: Term[], skipKey?: string): boolean {
  return terms.every((t) => {
    if (t.kind === 'text') return `${c.resource} ${c.summary}`.toLowerCase().includes(t.text.toLowerCase()) !== t.negate;
    if (t.key === skipKey) return true;
    return values(c, t.key).some((v) => t.values.some((p) => glob(p, v))) !== t.negate;
  });
}

/** About 60 bars over the range, on a round step (as the server buckets). */
export function bucketStep(range: { from: number; to: number }): number {
  const span = Math.max(range.to - range.from, 60e3);
  return [60e3, 300e3, 600e3, 1800e3, HOUR, 3 * HOUR, 6 * HOUR, 12 * HOUR, DAY].find((s) => span / s <= 90) ?? DAY;
}

export async function searchChanges(req: ChangeSearchRequest): Promise<ChangePage> {
  const terms = parseQuery(req.query ?? '');
  const inRange = changes().filter((c) => c.timestamp >= req.range.from && c.timestamp <= req.range.to);
  const facets: CatalogFacet[] = FACETS.map((key) => {
    const counts = new Map<string, number>();
    for (const c of inRange.filter((x) => matches(x, terms, key))) for (const v of new Set(values(c, key))) counts.set(v, (counts.get(v) ?? 0) + 1);
    return { key, values: [...counts].map(([value, count]) => ({ value, count })).sort((a, b) => b.count - a.count || a.value.localeCompare(b.value)) };
  }).filter((f) => f.values.length);
  const rows = inRange.filter((c) => matches(c, terms)).sort((a, b) => b.timestamp - a.timestamp || a.id.localeCompare(b.id));
  const step = bucketStep(req.range);
  const start = Math.floor(req.range.from / step) * step;
  const histogram: ChangeBucket[] = [];
  for (let t = start; t <= req.range.to; t += step) histogram.push({ t, counts: {} });
  for (const c of rows) {
    const b = histogram[Math.floor((c.timestamp - start) / step)];
    if (b) b.counts[c.changeType] = (b.counts[c.changeType] ?? 0) + 1;
  }
  const offset = req.offset ?? 0;
  return delay({ items: rows.slice(offset, offset + Math.min(Math.max(req.limit, 1), 500)), total: rows.length, facets, histogram });
}
