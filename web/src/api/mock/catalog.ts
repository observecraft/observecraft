// In-browser stand-in for Infrastructure → Catalog (see api/catalog/client.ts): the mock hosts and
// Kubernetes inventory as one list, searched, faceted and paged as the server does.
import type { CatalogCategory, CatalogFacet, CatalogGroup, CatalogPage, CatalogResource, CatalogSearchRequest, CatalogType } from '../catalog/client';
import type { KubeKind, KubeResource } from '../kubernetes/client';
import { parseQuery, type Term } from '../../lib/searchQuery';
import { hostInventory, type StoredHost } from './infrastructure';
import { inventory } from './kubernetes';

const FACETS = ['resource_category', 'resource_type', 'cloud', 'account', 'region', 'cluster', 'namespace', 'env', 'service', 'team'];

const KUBE_TYPES: Partial<Record<KubeKind, CatalogType>> = {
  clusters: 'kube_cluster',
  nodes: 'kube_node',
  namespaces: 'kube_namespace',
  deployments: 'kube_deployment',
  statefulsets: 'kube_statefulset',
  daemonsets: 'kube_daemonset',
  cronjobs: 'kube_cronjob',
  jobs: 'kube_job',
  pods: 'kube_pod',
  services: 'kube_service',
  ingresses: 'kube_ingress',
  persistentvolumeclaims: 'kube_persistent_volume_claim',
  persistentvolumes: 'kube_persistent_volume',
  containers: 'container',
  ecs_clusters: 'ecs_cluster',
  ecs_services: 'ecs_service',
  ecs_tasks: 'ecs_task',
};

function category(t: CatalogType): CatalogCategory {
  if (t === 'host') return 'compute';
  if (t === 'kube_pod' || t === 'container') return 'containers';
  if (t.startsWith('ecs_')) return 'ecs';
  return 'kubernetes';
}

const delay = <T>(value: T, ms = 80 + Math.random() * 120) => new Promise<T>((r) => setTimeout(() => r(value), ms));

function fromHost(h: StoredHost): CatalogResource {
  const tags = [
    `host:${h.name}`,
    `os:${h.os}`,
    ...(h.env ? [`env:${h.env}`] : []),
    ...(h.team ? [`team:${h.team}`] : []),
    ...(h.cluster ? [`kube_cluster_name:${h.cluster}`] : []),
    ...(h.region ? [`region:${h.region}`] : []),
  ];
  return {
    id: `host:${h.name}`,
    resourceType: 'host',
    resourceCategory: 'compute',
    name: h.name,
    cloud: h.cloud === 'none' ? null : h.cloud,
    account: h.cloud === 'aws' ? '123456789012' : null,
    region: h.region,
    cluster: h.cluster,
    namespace: null,
    env: h.env,
    service: h.service,
    team: h.team,
    status: 'Active',
    health: 'ok',
    createdAt: null,
    lastSeen: Date.now() - (h.seed % 60) * 1000,
    tags,
    attributes: { os: h.os, collector: h.collectors.join(', '), cpu_pct: String(h.cpuPct), mem_pct: String(h.memPct), ...(h.distro ? { distro: h.distro } : {}) },
    kubeKind: null,
  };
}

function fromKube(r: KubeResource): CatalogResource | null {
  const type = r.kind ? KUBE_TYPES[r.kind] : undefined;
  if (!type || !r.kind) return null;
  const tag = (k: string) => r.tags.find((t) => t.startsWith(`${k}:`))?.slice(k.length + 1) || null;
  const attributes: Record<string, string> = {};
  const put = (k: string, v: string | null | undefined) => {
    if (v) attributes[k] = v;
  };
  put('node', r.node);
  put('ready', r.ready);
  put('owner', r.owner ? `${r.owner.kind}/${r.owner.name}` : null);
  put('images', r.images.join(', '));
  put('version', r.version);
  put('instance_type', r.instanceType);
  put('availability_zone', r.zone);
  return {
    id: r.uid,
    resourceType: type,
    resourceCategory: category(type),
    name: r.name,
    cloud: null,
    account: tag('aws_account'),
    region: tag('region') ?? (r.zone ? r.zone.replace(/[a-z]$/, '') : null),
    cluster: r.cluster || null,
    namespace: r.namespace ?? null,
    env: tag('env'),
    service: tag('service'),
    team: tag('team'),
    status: r.status || null,
    health: r.health,
    createdAt: r.createdAt ?? null,
    lastSeen: r.lastSeen,
    tags: r.tags,
    attributes,
    kubeKind: r.kind,
  };
}

function values(r: CatalogResource, key: string): string[] {
  const one = (v: string | null | undefined) => (v ? [v] : []);
  switch (key) {
    case 'resource_type':
    case 'type':
      return [r.resourceType];
    case 'resource_category':
    case 'category':
      return [r.resourceCategory];
    case 'name':
      return [r.name];
    case 'cloud':
    case 'account':
    case 'region':
    case 'env':
    case 'service':
    case 'team':
    case 'status':
      return one(r[key]);
    case 'cluster':
    case 'kube_cluster_name':
      return one(r.cluster);
    case 'namespace':
    case 'kube_namespace':
      return one(r.namespace);
  }
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

function matches(r: CatalogResource, terms: Term[], skipKey?: string): boolean {
  return terms.every((t) => {
    if (t.kind === 'text') return r.name.toLowerCase().includes(t.text.toLowerCase()) !== t.negate;
    if (t.key === skipKey) return true;
    return values(r, t.key).some((v) => t.values.some((p) => glob(p, v))) !== t.negate;
  });
}

export function catalogResources(): CatalogResource[] {
  return [...hostInventory().map(fromHost), ...inventory().flatMap((r) => fromKube(r) ?? [])];
}

export async function searchCatalog(req: CatalogSearchRequest): Promise<CatalogPage> {
  const all = catalogResources();
  const terms = parseQuery(req.query ?? '');
  const facets: CatalogFacet[] = FACETS.map((key) => {
    const counts = new Map<string, number>();
    for (const r of all.filter((x) => matches(x, terms, key))) for (const v of new Set(values(r, key))) counts.set(v, (counts.get(v) ?? 0) + 1);
    return { key, values: [...counts].map(([value, count]) => ({ value, count })).sort((a, b) => b.count - a.count || a.value.localeCompare(b.value)) };
  }).filter((f) => f.values.length);
  const field = req.sort ?? 'name';
  const key = (r: CatalogResource): string | number =>
    field === 'type' ? r.resourceType : field === 'cluster' ? (r.cluster ?? '') : field === 'region' ? (r.region ?? '') : field === 'last_seen' ? r.lastSeen : r.name.toLowerCase();
  const rows = all
    .filter((r) => matches(r, terms))
    .sort((a, b) => {
      const x = key(a);
      const y = key(b);
      const o = x < y ? -1 : x > y ? 1 : 0;
      return (req.desc ? -o : o) || a.name.localeCompare(b.name) || a.id.localeCompare(b.id);
    });
  const offset = req.offset ?? 0;
  return delay({ items: rows.slice(offset, offset + Math.min(Math.max(req.limit, 1), 500)), total: rows.length, facets, groups: req.groupBy ? groups(rows, req.groupBy) : null });
}

function groups(rows: CatalogResource[], key: string): CatalogGroup[] {
  const by = new Map<string, { count: number; sets: Set<string>[] }>();
  for (const r of rows) {
    const name = values(r, key)[0] ?? `no ${key}`;
    const g = by.get(name) ?? { count: 0, sets: [new Set(), new Set(), new Set(), new Set()] };
    g.count += 1;
    [r.region, r.env, r.team, r.service].forEach((v, i) => v && g.sets[i].add(v));
    by.set(name, g);
  }
  const list = (s: Set<string>) => [...s].sort().slice(0, 10);
  return [...by]
    .map(([k, g]) => ({ key: k, count: g.count, regions: list(g.sets[0]), envs: list(g.sets[1]), teams: list(g.sets[2]), services: list(g.sets[3]) }))
    .sort((a, b) => b.count - a.count || a.key.localeCompare(b.key));
}
