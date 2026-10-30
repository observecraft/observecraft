// Mock resource changes: a fictional week of configuration changes of the mock catalog's resources
// (image rollouts, scaling, label edits, created and deleted pods and jobs), classified as the server's
// change job classifies them (routes/resource_changes.rs).
import type { CatalogResource } from '../catalog/client';
import type { ResourceChange, ResourceChangePage, ResourceChangeSearchRequest } from '../resourceChanges/client';
import { parseQuery } from '../../lib/searchQuery';
import { catalogResources } from './catalog';

const HOUR = 3_600_000;
const FACETS = ['team', 'risk', 'cloud', 'region', 'env', 'account', 'service', 'change_type', 'resource_type'];
const EPHEMERAL = ['kube_pod', 'kube_job', 'container', 'ecs_task'];

function rng(seed: number) {
  let s = seed >>> 0 || 1;
  return () => ((s = Math.imul(s ^ (s >>> 15), 2246822519) ^ Math.imul(s ^ (s >>> 13), 3266489917)), (s >>> 0) / 2 ** 32);
}

function typeLabel(t: string): string {
  const [prefix, ...rest] = t.split('_');
  const words = (prefix === 'kube' || prefix === 'ecs' ? rest : [prefix, ...rest]).map((w) => w.charAt(0).toUpperCase() + w.slice(1));
  return `${prefix === 'kube' ? 'Kubernetes ' : prefix === 'ecs' ? 'ECS ' : ''}${words.join(' ')}`;
}

function doc(r: CatalogResource, replicas: number, image: string, labels: Record<string, string>) {
  const out = {
    resource_type: r.resourceType,
    name: r.name,
    cloud: r.cloud ?? null,
    account: r.account ?? null,
    region: r.region ?? null,
    cluster: r.cluster ?? null,
    namespace: r.namespace ?? null,
    env: r.env ?? null,
    service: r.service ?? null,
    team: r.team ?? null,
    tags: [...r.tags].sort(),
    attributes: { ...r.attributes, replicas: String(replicas), images: image },
    manifest: { metadata: { labels }, spec: { replicas, template: { spec: { containers: [{ name: r.service ?? 'app', image, resources: { limits: { memory: '512Mi' } } }] } } } },
  };
  // Fields the resource doesn't have are left out, as the server does.
  return Object.fromEntries(Object.entries(out).filter(([, v]) => v !== null)) as typeof out;
}

let cache: { at: number; rows: ResourceChange[] } | null = null;

/** A week of changes, the same for an hour (so pages and filters agree). */
function week(): ResourceChange[] {
  const now = Math.floor(Date.now() / HOUR) * HOUR;
  if (cache?.at === now) return cache.rows;
  const r = rng(7);
  const rows: ResourceChange[] = [];
  const resources = catalogResources().filter((x) => x.resourceType !== 'host' || r() < 0.3);
  for (const res of resources) {
    const service = res.service ?? res.name.split('-').slice(0, 2).join('-');
    const base = (t: number, risk: string, title: string, description: string, types: string[], fields: string[], old: object | null, next: object | null, noise = false): ResourceChange => ({
      detected: t,
      resourceId: res.id,
      resourceType: res.resourceType,
      resourceName: res.name,
      cloud: res.cloud ?? '',
      account: res.account ?? '',
      region: res.region ?? '',
      cluster: res.cluster ?? '',
      namespace: res.namespace ?? '',
      env: res.env ?? '',
      service: res.service ?? '',
      team: res.team ?? '',
      changeTypes: types,
      changedFields: fields,
      risk,
      title,
      description,
      isNoise: noise,
      resourceChanges: 0,
      oldFields: old as ResourceChange['oldFields'],
      newFields: next as ResourceChange['newFields'],
    });
    const label = typeLabel(res.resourceType);
    const events = EPHEMERAL.includes(res.resourceType) ? (r() < 0.6 ? 2 : 0) : Math.floor(r() * 4);
    for (let i = 0; i < events; i++) {
      const t = now - Math.floor(r() * 7 * 24) * HOUR - Math.floor(r() * HOUR);
      const v = 1 + Math.floor(r() * 9);
      const before = doc(res, 3, `example-org/${service}:2.${v}.0`, { app: service });
      if (EPHEMERAL.includes(res.resourceType)) {
        rows.push(
          i === 0
            ? base(t, 'low', `${label} created`, `A new ${label.toLowerCase()} appeared in your infrastructure.`, ['creation'], ['name', 'attributes.images'], null, before, true)
            : base(t, 'low', `${label} deleted`, `The ${label.toLowerCase()} is gone.`, ['config', 'deletion'], ['name', 'attributes.images'], before, null, true),
        );
        continue;
      }
      // Images and replicas belong to workloads; other resources get tag edits and deletions.
      const workload = ['kube_deployment', 'kube_statefulset', 'kube_daemonset', 'ecs_service'].includes(res.resourceType);
      const kind = workload ? r() : 0.7 + r() * 0.3;
      if (kind < 0.4) {
        const after = doc(res, 3, `example-org/${service}:2.${v + 1}.0`, { app: service });
        rows.push(
          base(
            t,
            'medium',
            `Image changed to example-org/${service}:2.${v + 1}.0`,
            'A new image rolls out new code: if it misbehaves, roll back to the previous image.',
            ['image'],
            ['attributes.images', `manifest.spec.template.spec.containers[${service}].image`],
            before,
            after,
          ),
        );
      } else if (kind < 0.7) {
        const to = r() < 0.15 ? 0 : 2 + Math.floor(r() * 6);
        const after = doc(res, to, before.attributes.images, { app: service });
        rows.push(
          base(
            t,
            to === 0 ? 'high' : 'low',
            `Replicas changed from 3 to ${to}`,
            to === 0 ? 'Scaled to zero: the workload no longer serves anything.' : 'The number of running copies changed: capacity and cost follow.',
            ['scaling'],
            ['attributes.replicas', 'manifest.spec.replicas'],
            before,
            after,
          ),
        );
      } else if (kind < 0.9) {
        const after = doc(res, 3, before.attributes.images, { app: service, 'example.com/owner': res.team ?? 'platform' });
        rows.push(
          base(
            t,
            'low',
            'Tags changed',
            'Labels and tags changed: monitors and dashboards that select on them may pick the resource up differently.',
            ['tags'],
            ['manifest.metadata.labels.example.com/owner'],
            before,
            after,
          ),
        );
      } else {
        rows.push(
          base(
            t,
            'high',
            `${label} deleted`,
            `Deleting a ${label.toLowerCase()} removes it for everything that depends on it: workloads, traffic or data it served stop being available until it's recreated.`,
            ['config', 'deletion'],
            ['name', 'attributes.images', 'manifest.spec.replicas'],
            before,
            null,
          ),
        );
      }
    }
  }
  cache = { at: now, rows };
  return rows;
}

const values = (c: ResourceChange, key: string): string[] => {
  switch (key) {
    case 'risk':
    case 'risk_level':
      return [c.risk];
    case 'change_type':
    case '@change_types':
      return c.changeTypes;
    case '@changed_fields':
    case 'changed_field':
      return c.changedFields;
    case 'resource_type':
      return [c.resourceType];
    case 'resource_id':
      return [c.resourceId];
    case '@is_noise':
    case 'is_noise':
      return [String(c.isNoise)];
    case 'kube_cluster_name':
    case 'cluster':
      return c.cluster ? [c.cluster] : [];
    default: {
      const v = (c as unknown as Record<string, unknown>)[key];
      return typeof v === 'string' && v ? [v] : [];
    }
  }
};

const RISK_RANK: Record<string, number> = { high: 3, medium: 2, low: 1 };

export async function searchResourceChanges(req: ResourceChangeSearchRequest): Promise<ResourceChangePage> {
  const terms = parseQuery(req.query ?? '');
  const inRange = week().filter((c) => c.detected >= req.range.from && c.detected <= req.range.to);
  const glob = (p: string, v: string) =>
    new RegExp(
      `^${p
        .split('*')
        .map((x) => x.replace(/[.+?^${}()|[\]\\]/g, '\\$&'))
        .join('.*')}$`,
      'i',
    ).test(v);
  const match = (c: ResourceChange, skip?: string) =>
    terms.every((t) =>
      t.kind === 'text'
        ? `${c.resourceName} ${c.title}`.toLowerCase().includes(t.text.toLowerCase()) !== t.negate
        : t.key === skip || values(c, t.key).some((v) => t.values.some((p) => glob(p, v))) !== t.negate,
    );
  const facets = FACETS.map((key) => {
    const counts = new Map<string, number>();
    for (const c of inRange.filter((c) => match(c, key))) for (const v of new Set(values(c, key))) counts.set(v, (counts.get(v) ?? 0) + 1);
    return { key, values: [...counts].map(([value, count]) => ({ value, count })).sort((a, b) => b.count - a.count) };
  }).filter((f) => f.values.length);
  const matched = inRange.filter((c) => match(c));
  const per = new Map<string, number>();
  for (const c of matched) per.set(c.resourceId, (per.get(c.resourceId) ?? 0) + 1);
  const span = req.range.to - req.range.from;
  const size = [300e3, 900e3, 1800e3, HOUR, 3 * HOUR, 6 * HOUR, 12 * HOUR, 24 * HOUR].find((s) => span / s <= 60) ?? 24 * HOUR;
  const histogram: ResourceChangePage['histogram'] = [];
  for (let t = Math.floor(req.range.from / size) * size; t <= req.range.to; t += size) {
    const counts: Record<string, number> = {};
    for (const c of matched.filter((c) => c.detected >= t && c.detected < t + size)) for (const v of values(c, req.groupBy || 'resource_type')) counts[v] = (counts[v] ?? 0) + 1;
    histogram.push({ t, counts });
  }
  const sorted = [...matched].sort((a, b) => (RISK_RANK[b.risk] ?? 0) - (RISK_RANK[a.risk] ?? 0) || b.detected - a.detected);
  const offset = req.offset ?? 0;
  return {
    total: matched.length,
    items: sorted.slice(offset, offset + (req.limit ?? 50)).map((c) => ({ ...c, resourceChanges: per.get(c.resourceId) ?? 1 })),
    histogram,
    bucketMs: size,
    facets,
  };
}
