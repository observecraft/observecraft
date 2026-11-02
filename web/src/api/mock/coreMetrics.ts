// Mock metrics of the integrations-core dashboards the mock shows installed (coreDashboards.generated.json) and of
// the demo accounts' AWS, Azure and Google Cloud services (cloudServiceDashboards.generated.json):
// every metric they chart, on a few fictional instances per integration tagged by the dashboards' template
// variables, sized by the metric's unit from metadata.csv.
import core from '../integrations/coreDashboards.generated.json' with { type: 'json' };
import cloudServices from '../integrations/cloudServiceDashboards.generated.json' with { type: 'json' };

type Tags = Record<string, string>;

export interface CoreMetricDef {
  set: string;
  description: string;
  unit?: string;
  base: (e: Tags, r: () => number) => number;
  noise: number;
  integer?: boolean;
  max?: number;
}

const HOSTS = ['i-0a1b2c3d4e5f60001', 'i-0a1b2c3d4e5f60002', 'i-0a1b2c3d4e5f60003', 'i-0a1b2c3d4e5f60006'];
/** Values of common tags, from the e-commerce demo; other tags get `<tag>-<n>`. */
const VALUES: Record<string, string[]> = {
  db: ['orders', 'catalog', 'billing', 'inventory'],
  table: ['orders', 'order_items', 'products', 'invoices'],
  schema: ['public', 'public', 'billing', 'catalog'],
  service: ['order-service', 'catalog-service', 'billing', 'inventory-service'],
  topic: ['orders', 'payments', 'inventory-updates', 'catalog-changes'],
  queue: ['orders', 'emails', 'payments', 'webhooks'],
  env: ['prod', 'prod', 'prod', 'qa'],
};

/** Sized by unit: fractions and percentages in range, bytes in megabytes to gigabytes, times in milliseconds. */
function base(unit: string | undefined): Pick<CoreMetricDef, 'base' | 'max' | 'integer'> {
  const u = unit?.split('/')[0];
  if (u === 'fraction') return { base: (_e, r) => 0.05 + r() * 0.6, max: 1 };
  if (u === 'percent') return { base: (_e, r) => 5 + r() * 60, max: 100 };
  if (u && /byte$/.test(u)) return { base: (_e, r) => 2e6 + Math.pow(r(), 2) * 4e9 };
  if (u && /(second|millisecond|microsecond|nanosecond)$/.test(u)) return { base: (_e, r) => (u === 'second' ? 0.01 : 10) + r() * (u === 'second' ? 2 : 400) };
  return { base: (_e, r) => 1 + Math.pow(r(), 2) * 900, integer: true };
}

type Dashboard = { logo: string; templateVariables: { prefix?: string }[]; widgets: unknown };

export const CORE_SETS: Record<string, Tags[]> = {};
export const CORE_METRICS: Record<string, CoreMetricDef> = {};

const dashboards = [...(core as { dashboards: Dashboard[] }).dashboards, ...(cloudServices as { dashboards: Dashboard[] }).dashboards];
// One set of instances per integration, tagged by every template variable of its dashboards.
const tagsOf = new Map<string, Set<string>>();
for (const d of dashboards) {
  const tags = tagsOf.get(d.logo) ?? new Set(['host']);
  for (const v of d.templateVariables) if (v.prefix) tags.add(v.prefix);
  tagsOf.set(d.logo, tags);
}
for (const [logo, tags] of tagsOf) CORE_SETS[`core_${logo}`] = HOSTS.map((host, i) => Object.fromEntries([...tags].map((t) => [t, t === 'host' ? host : (VALUES[t]?.[i] ?? `${t}-${i + 1}`)])));

for (const d of dashboards)
  for (const [, metric] of JSON.stringify(d.widgets).matchAll(/[a-z]+:([a-z][a-z0-9_]*\.[a-z0-9_.]+)\{/g)) {
    if (CORE_METRICS[metric]) continue;
    const unit = (core.units as Record<string, string>)[metric] ?? (cloudServices.units as Record<string, string>)[metric];
    CORE_METRICS[metric] = { set: `core_${d.logo}`, description: metric, unit, noise: 0.2, ...base(unit) };
  }
