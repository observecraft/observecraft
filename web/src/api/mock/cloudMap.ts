// In-browser stand-in for the Cloudcraft diagram (see api/cloudMap/client.ts): the mock catalog grouped
// as the server does (cloud_map.rs), with demo monitors, APM error rates and service calls for the overlays.
import type { CatalogResource, CatalogType } from '../catalog/client';
import type { CloudMap, CloudMapEdge, CloudMapGroup, CloudMapHealth, CloudMapNode, CloudMapProvider, CloudMapProviderSummary, CloudMapRequest } from '../cloudMap/client';
import { parseQuery } from '../../lib/searchQuery';
import { catalogResources, matches, values } from './catalog';

export const MAX_NODES_PER_GROUP = 48;
export const MAX_GROUPS_PER_LEVEL = 24;
const OTHER = '__other__';
const GROUP_KEYS = ['cloud', 'account', 'region', 'availability_zone', 'cluster', 'namespace', 'env', 'service', 'team'];
const HEALTH_RANK: Record<CloudMapHealth, number> = { unknown: 0, ok: 1, warning: 2, error: 3 };

const PROVIDER_TYPES: Record<CloudMapProvider, CatalogType[]> = {
  kubernetes: ['kube_node', 'kube_deployment', 'kube_statefulset', 'kube_daemonset', 'kube_cronjob', 'kube_service', 'kube_ingress'],
  hosts: ['host'],
  ecs: ['ecs_service', 'ecs_task'],
};
export const DEFAULT_GROUP_BY: Record<CloudMapProvider, string[]> = { kubernetes: ['cluster', 'namespace'], hosts: ['cloud', 'region', 'availability_zone'], ecs: ['region', 'cluster'] };

/** Demo APM: who calls whom, and error rates over the last hour. */
const CALLS: [string, string, number][] = [
  ['api-gateway', 'storefront-api', 5400],
  ['storefront-api', 'catalog-service', 3100],
  ['storefront-api', 'order-service', 1200],
  ['order-service', 'inventory-service', 1100],
  ['order-service', 'billing', 900],
  ['billing', 'workflow-engine', 300],
  ['order-service', 'notification-worker', 450],
];
const ERROR_RATES: Record<string, number> = { 'inventory-service': 0.083, 'order-service': 0.018, 'api-gateway': 0.002, 'storefront-api': 0.004, billing: 0.001, 'catalog-service': 0.003 };
/** Demo monitors alerting on resources. */
const ALERTING: Record<string, string>[] = [{ kube_deployment: 'inventory-service', env: 'prod' }];

const delay = <T>(value: T, ms = 80 + Math.random() * 120) => new Promise<T>((r) => setTimeout(() => r(value), ms));
const worst = (hs: CloudMapHealth[]): CloudMapHealth => hs.reduce<CloudMapHealth>((a, b) => (HEALTH_RANK[b] > HEALTH_RANK[a] ? b : a), 'unknown');
const drawn = (r: CatalogResource) => r.resourceType !== 'ecs_task' || !values(r, 'ecs_service').length;

function node(r: CatalogResource): CloudMapNode {
  return {
    id: r.id,
    name: r.name,
    resourceType: r.resourceType,
    health: r.health === 'info' || !r.health ? 'unknown' : r.health,
    service: r.service,
    alerts: 0,
    errorRate: null,
    aggregate: null,
    resource: r,
  };
}

function order(nodes: CloudMapNode[]): CloudMapNode[] {
  return [...nodes].sort((a, b) => HEALTH_RANK[b.health] - HEALTH_RANK[a.health] || b.alerts - a.alerts || a.name.localeCompare(b.name) || a.id.localeCompare(b.id));
}

function fold(nodes: CloudMapNode[]): CloudMapNode[] {
  const sorted = order(nodes);
  if (sorted.length <= MAX_NODES_PER_GROUP) return sorted;
  const rest = sorted.slice(MAX_NODES_PER_GROUP - 1);
  return [
    ...sorted.slice(0, MAX_NODES_PER_GROUP - 1),
    {
      id: `more:${rest[0].id}`,
      name: `+${rest.length} more`,
      resourceType: rest[0].resourceType,
      health: worst(rest.map((n) => n.health)),
      service: null,
      alerts: rest.reduce((s, n) => s + n.alerts, 0),
      errorRate: null,
      aggregate: rest.length,
      resource: null,
    },
  ];
}

/** The group tree of `nodes` for `keys`, outermost first (like the server's build_tree). */
export function buildTree(nodes: CloudMapNode[], keys: string[], key = 'root', value = ''): CloudMapGroup {
  const base = { key, value, count: nodes.length, health: worst(nodes.map((n) => n.health)) };
  const [next, ...deeper] = keys;
  if (!next) return { ...base, groups: [], nodes: fold(nodes) };
  const by = new Map<string, CloudMapNode[]>();
  for (const n of nodes) {
    const v = (n.resource && values(n.resource, next)[0]) || '';
    by.set(v, [...(by.get(v) ?? []), n]);
  }
  const parts = [...by].sort((a, b) => Number(a[0] === '') - Number(b[0] === '') || b[1].length - a[1].length || a[0].localeCompare(b[0]));
  if (parts.length <= MAX_GROUPS_PER_LEVEL) return { ...base, groups: parts.map(([v, n]) => buildTree(n, deeper, next, v)), nodes: [] };
  const kept = parts.slice(0, MAX_GROUPS_PER_LEVEL - 1);
  const small = parts.slice(MAX_GROUPS_PER_LEVEL - 1).flatMap(([, n]) => n);
  return { ...base, groups: [...kept.map(([v, n]) => buildTree(n, deeper, next, v)), buildTree(small, [], next, OTHER)], nodes: [] };
}

function providers(all: CatalogResource[]): CloudMapProviderSummary[] {
  return (['kubernetes', 'hosts', 'ecs'] as CloudMapProvider[]).map((provider) => {
    const rows = all.filter((r) => PROVIDER_TYPES[provider].includes(r.resourceType) && drawn(r));
    return { provider, resources: rows.length, clouds: [...new Set(rows.flatMap((r) => (r.cloud ? [r.cloud] : [])))].sort() };
  });
}

export async function getCloudMap(req: CloudMapRequest): Promise<CloudMap> {
  const keys = req.groupBy?.length ? req.groupBy : DEFAULT_GROUP_BY[req.provider];
  const bad = keys.find((k) => !GROUP_KEYS.includes(k));
  if (bad) throw new Error(`Can't group by '${bad}'`);
  if (keys.length > 4) throw new Error('Group by at most 4 keys');
  const all = catalogResources();
  const terms = parseQuery(req.query ?? '');
  const rows = all.filter((r) => PROVIDER_TYPES[req.provider].includes(r.resourceType) && drawn(r) && matches(r, terms));
  const nodes = rows.map((r) => {
    const n = node(r);
    if (req.overlay === 'health') {
      n.alerts = ALERTING.filter((tags) => Object.entries(tags).every(([k, v]) => values(r, k).includes(v))).length;
      if (n.alerts) n.health = 'error';
      const rate = r.service ? ERROR_RATES[r.service] : undefined;
      if (rate != null) {
        n.errorRate = rate;
        n.health = worst([n.health, rate >= 0.05 ? 'error' : rate >= 0.01 ? 'warning' : 'ok']);
      }
    }
    return n;
  });
  const services = new Set(rows.flatMap((r) => (r.service ? [r.service] : [])));
  const edges: CloudMapEdge[] =
    req.overlay === 'dependencies' ? CALLS.filter(([a, b]) => services.has(a) && services.has(b)).map(([from, to, requests]) => ({ from, to, requests, errorRate: ERROR_RATES[to] ?? 0 })) : [];
  return delay({ providers: providers(all), groupBy: keys, root: buildTree(nodes, keys), edges, total: nodes.length });
}
