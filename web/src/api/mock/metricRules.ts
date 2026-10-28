// In-browser metrics rules for mock mode (Metrics → Settings → Rules). State lives in memory for the
// session; validation follows the server (routes/metric_rules.rs). Cardinality comes from the mock
// metric engine's volume, as the server's comes from the series table.
import type { MetricCardinality, MetricRule, MetricRuleInput, MetricRulesOverview } from '../metricRules/client';
import { globMatch, validPattern, validTagKey } from '../../lib/metricRules';
import { metricCatalog, metricVolume } from './metricsEngine';

const fail = (message: string): never => {
  throw new Error(message);
};

let state: { rules: Omit<MetricRule, 'metricsImpacted'>[]; seq: number } | null = null;

function db() {
  if (state) return state;
  state = {
    rules: [
      {
        id: 'mr-0001',
        name: 'Kubernetes workload tags',
        kind: 'indexing',
        metricPattern: 'kubernetes.*',
        action: 'allow_tags',
        tags: ['kube_cluster_name', 'kube_namespace', 'kube_deployment', 'env'],
        enabled: true,
        updatedBy: 'Alex Doe',
        updatedAt: Date.now() - 2 * 86_400_000,
      },
    ],
    seq: 2,
  };
  return state;
}

const names = () => metricVolume().metrics.map((m) => m.name);
const withImpact = (r: Omit<MetricRule, 'metricsImpacted'>): MetricRule => ({ ...r, tags: [...r.tags], metricsImpacted: names().filter((n) => globMatch(r.metricPattern, n)).length });

function validate(input: MetricRuleInput): Pick<MetricRule, 'name' | 'kind' | 'metricPattern' | 'action' | 'tags' | 'enabled'> {
  const name = input.name.trim();
  const metricPattern = input.metricPattern.trim();
  const tags = [...new Set((input.tags ?? []).map((t) => t.trim()).filter(Boolean))];
  const bad = tags.find((t) => !validTagKey(t));
  if (bad) fail(`'${bad}' isn't a valid tag key`);
  if (!name || name.length > 200) fail('The rule name is 1 to 200 characters');
  if (!validPattern(metricPattern)) fail('The metric pattern is a metric name, with * for any characters (e.g. kubernetes.*)');
  const ok = (input.kind === 'ingestion' && (input.action === 'drop' || input.action === 'strip_tags')) || (input.kind === 'indexing' && input.action === 'allow_tags');
  if (!ok) fail('Ingestion controls drop metrics or strip tags; indexing controls allow tags');
  if (input.action !== 'drop' && tags.length === 0) fail('List at least one tag');
  return { name, kind: input.kind, metricPattern, action: input.action, tags: input.action === 'drop' ? [] : tags, enabled: input.enabled ?? true };
}

const unique = (name: string, self?: string) => db().rules.some((r) => r.id !== self && r.name.toLowerCase() === name.toLowerCase()) && fail(`A rule named '${name}' already exists`);

export async function listMetricRules(): Promise<MetricRule[]> {
  return [...db().rules].sort((a, b) => a.kind.localeCompare(b.kind) || a.name.localeCompare(b.name)).map(withImpact);
}

export async function createMetricRule(input: MetricRuleInput): Promise<MetricRule> {
  const v = validate(input);
  unique(v.name);
  const s = db();
  const r = { id: `mr-${String(s.seq++).padStart(4, '0')}`, ...v, updatedBy: 'You', updatedAt: Date.now() };
  s.rules.push(r);
  return withImpact(r);
}

export async function updateMetricRule(id: string, input: MetricRuleInput): Promise<MetricRule> {
  const v = validate(input);
  unique(v.name, id);
  const r = db().rules.find((x) => x.id === id) ?? fail('Rule not found');
  Object.assign(r, v, { updatedBy: 'You', updatedAt: Date.now() });
  return withImpact(r);
}

export async function deleteMetricRule(id: string): Promise<void> {
  const s = db();
  if (!s.rules.some((r) => r.id === id)) fail('Rule not found');
  s.rules = s.rules.filter((r) => r.id !== id);
}

/** Ingested series from the volume; indexed ones scale with the share of the metric's tags a control keeps. */
export async function metricRulesOverview(): Promise<MetricRulesOverview> {
  const rules = db().rules.filter((r) => r.enabled);
  const catalog = metricCatalog();
  const entry = (m: { name: string; series: number }): MetricCardinality => {
    const matching = rules.filter((r) => globMatch(r.metricPattern, m.name));
    const allowed = new Set(matching.filter((r) => r.kind === 'indexing').flatMap((r) => r.tags));
    const tags = catalog.find((c) => c.name === m.name)?.tags ?? [];
    const kept = tags.filter((t) => allowed.has(t)).length;
    const indexed = allowed.size === 0 ? m.series : Math.max(1, Math.round(m.series * (tags.length ? kept / tags.length : 1)));
    return { name: m.name, ingested: m.series, indexed, rules: matching.map((r) => r.name) };
  };
  const volume = metricVolume().metrics;
  return { filtered: volume.map(entry).filter((m) => m.rules.length > 0), top: volume.slice(0, 10).map(entry) };
}
