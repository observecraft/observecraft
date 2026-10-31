// In-browser stand-in for APM → Settings (routes/apm_settings.rs): span flow and volumes derived from
// the mock APM services, and the retention filters, span-based metrics, span pipelines and defaults
// kept in memory.
import type {
  ApmDefaults,
  ApmDefaultsInput,
  ApmFlow,
  ApmPoint,
  Ingestion,
  IngestionService,
  RetentionFilter,
  RetentionFilterInput,
  RetentionFilters,
  SpanPipeline,
  SpanPipelineInput,
  SpansMetric,
  SpansMetricInput,
  SpanVolume,
  TimeRange,
} from '../apmSettings/types';
import { SERVICES } from './apmTraces';

const MIN = 60_000;
const HOUR = 60 * MIN;
const DAY = 24 * HOUR;
const delay = <T>(v: T, ms = 120): Promise<T> => new Promise((r) => setTimeout(() => r(structuredClone(v)), ms));
const fail = (msg: string) => Promise.reject(new Error(msg));

/** Spans per second each mock service produces, and how (language, sampling, root of its traces). */
const TRAFFIC: Record<string, { sps: number; root: boolean; configs: string[]; share: number; kind: string }> = {
  'api-gateway': { sps: 410, root: true, configs: ['APM AGENT'], share: 1, kind: 'web' },
  'storefront-api': { sps: 320, root: true, configs: ['APM AGENT', 'RUM'], share: 0.86, kind: 'web' },
  'order-service': { sps: 180, root: false, configs: ['APM AGENT'], share: 1, kind: 'web' },
  billing: { sps: 96, root: false, configs: ['APM LOCAL'], share: 1, kind: 'web' },
  'inventory-service': { sps: 140, root: false, configs: ['APM AGENT'], share: 1, kind: 'web' },
  'catalog-service': { sps: 210, root: true, configs: ['APM AGENT'], share: 0.94, kind: 'web' },
  'workflow-engine': { sps: 44, root: true, configs: ['APM AGENT', 'APM LOCAL'], share: 0.78, kind: 'queue' },
  'notification-worker': { sps: 38, root: true, configs: ['OTEL'], share: 1, kind: 'queue' },
};
const BYTES_PER_SPAN = 820;
/** Generated spans per ingested span (head sampling before ingestion). */
const SAMPLED_FACTOR = 1.36;

function step(r: TimeRange): number {
  const target = Math.max(MIN, (r.to - r.from) / 40);
  return [MIN, 5 * MIN, 15 * MIN, 30 * MIN, HOUR, 3 * HOUR, 6 * HOUR, 12 * HOUR, DAY].find((s) => s >= target) ?? DAY;
}

/** Daily and hourly rhythm around 1, so charts have shape. */
function rhythm(t: number): number {
  const h = new Date(t).getHours();
  return 0.65 + 0.35 * Math.sin(((h - 6) / 24) * Math.PI * 2) * 0.8 + 0.08 * Math.sin(t / (7 * MIN));
}

function series(r: TimeRange, perSecond: number): ApmPoint[] {
  const s = step(r);
  const out: ApmPoint[] = [];
  for (let t = Math.floor(r.from / s) * s; t <= r.to; t += s) out.push({ t, v: Math.round(perSecond * rhythm(t) * (s / 1000)) });
  return out;
}

const totalSps = Object.values(TRAFFIC).reduce((a, b) => a + b.sps, 0);
const secs = (r: TimeRange) => Math.max(1, (r.to - r.from) / 1000);

// ================================================================ retention filters

let filters: RetentionFilter[] = [
  filter('errors-default', 'Error Default', 'status:error', 'spans-errors-sampling-processor', 1, true, 0.021),
  filter('appsec-default', 'App and API Protection Default', '@_dd.appsec.enabled:1', 'spans-appsec-sampling-processor', 1, true, 0.008),
  filter('Hq2xQ0mT3l8bU1cW4yRrAa', 'Checkout requests', 'service:storefront-api resource_name:"POST /checkout"', 'spans-sampling-processor', 1, true, 0.012, 'priya.shah@example.com', 9),
  filter('Zp4rN7sKq2Lw9eXc1vBmTg', 'Slow catalog queries', 'service:catalog-service @duration:>500000000', 'spans-sampling-processor', 1, true, 0.006, 'alex.morgan@example.com', 4),
  filter('Mf8dT1aYw5Uo3iPs6hJkLq', 'Payment provider calls', 'service:billing @peer.service:payments-provider', 'spans-sampling-processor', 0.5, true, 0.018, 'dev@example.com', 2),
  filter('Rb6nC9xVz2Qe7tYu4oIpAs', 'Synthetic checks', '@_dd.origin:synthetics*', 'spans-sampling-processor', 1, false, 0.004, 'alex.morgan@example.com', 30),
];
/** Share of all spans each filter would match first. */
const SHARE: Record<string, number> = Object.fromEntries(filters.map((f) => [f.id, f.spansMatched]));

function filter(id: string, name: string, query: string, filterType: string, rate: number, enabled: boolean, share: number, by = '', daysAgo = 0): RetentionFilter {
  const at = daysAgo ? Math.round((Date.now() - daysAgo * DAY) / 1000) : 0;
  return {
    id,
    name,
    query,
    filterType,
    rate,
    traceRate: 0,
    enabled,
    isDefault: id.endsWith('-default'),
    executionOrder: 0,
    createdBy: by,
    createdAt: at,
    modifiedBy: by,
    modifiedAt: at,
    spansMatched: share,
    spansIndexed: 0,
  };
}

export function retentionFilters(r: TimeRange): Promise<RetentionFilters> {
  const ingested = Math.round(totalSps * secs(r));
  const out = filters.map((f, i) => {
    const matched = Math.round(ingested * (SHARE[f.id] ?? 0.01));
    return { ...f, executionOrder: i, spansMatched: matched, spansIndexed: f.enabled ? Math.round(matched * f.rate) : 0 };
  });
  return delay({ filters: out, spansIngested: ingested, spansIndexed: out.reduce((a, f) => a + f.spansIndexed, 0) });
}

function checkFilter(i: RetentionFilterInput): string | null {
  if (!i.name.trim()) return 'A retention filter needs a name';
  if (!(i.rate >= 0 && i.rate <= 1)) return "'rate' must be between 0 and 1";
  if (i.traceRate != null && !(i.traceRate >= 0 && i.traceRate <= 1)) return "'trace_rate' must be between 0 and 1";
  return null;
}

export function createFilter(i: RetentionFilterInput): Promise<RetentionFilter> {
  const err = checkFilter(i);
  if (err) return fail(err);
  const id = Math.random().toString(36).slice(2, 13) + Math.random().toString(36).slice(2, 13);
  const f = filter(id, i.name.trim(), (i.query ?? '').trim() || '*', 'spans-sampling-processor', i.rate, i.enabled, 0.01, 'you@example.com', 0);
  f.createdAt = f.modifiedAt = Math.round(Date.now() / 1000);
  f.traceRate = i.traceRate ?? 0;
  SHARE[id] = 0.01;
  filters.push(f);
  return delay(f);
}

export function updateFilter(id: string, i: RetentionFilterInput): Promise<RetentionFilter> {
  const err = checkFilter(i);
  if (err) return fail(err);
  const f = filters.find((x) => x.id === id);
  if (!f) return fail('retention filter not found');
  Object.assign(f, {
    name: i.name.trim(),
    query: (i.query ?? '').trim() || '*',
    rate: i.rate,
    traceRate: i.traceRate ?? 0,
    enabled: i.enabled,
    modifiedBy: 'you@example.com',
    modifiedAt: Math.round(Date.now() / 1000),
  });
  return delay(f);
}

export function deleteFilter(id: string): Promise<void> {
  const f = filters.find((x) => x.id === id);
  if (!f) return fail('retention filter not found');
  if (f.isDefault) return fail("Default retention filters can't be deleted; disable them instead");
  filters = filters.filter((x) => x.id !== id);
  return delay(undefined);
}

export function reorderFilters(ids: string[]): Promise<void> {
  if (ids.length !== filters.length || !filters.every((f) => ids.includes(f.id))) return fail('list every retention filter once');
  filters = ids.map((id) => filters.find((f) => f.id === id)!);
  return delay(undefined);
}

// ================================================================ span-based metrics

let spanMetrics: SpansMetric[] = [
  {
    id: 'storefront.checkout.requests',
    query: 'service:storefront-api @span.kind:server @http.status_code:2*',
    aggregationType: 'count',
    path: null,
    includePercentiles: false,
    groupBy: [
      { path: '@http.route', tagName: 'http.route' },
      { path: 'env', tagName: 'env' },
    ],
  },
  {
    id: 'storefront.checkout.requests_all',
    query: 'service:storefront-api @span.kind:server',
    aggregationType: 'count',
    path: null,
    includePercentiles: false,
    groupBy: [{ path: '@http.route', tagName: 'http.route' }],
  },
  {
    id: 'billing.payment.duration',
    query: 'service:billing resource_name:"POST /payments"',
    aggregationType: 'distribution',
    path: '@duration',
    includePercentiles: true,
    groupBy: [
      { path: 'resource_name', tagName: 'resource_name' },
      { path: 'status', tagName: 'status' },
    ],
  },
  {
    id: 'api_gateway.external.requests',
    query: '@http.client_ip:* -@network.client.internal:true',
    aggregationType: 'count',
    path: null,
    includePercentiles: false,
    groupBy: [{ path: 'service', tagName: 'service' }],
  },
];

export const metrics = (): Promise<SpansMetric[]> => delay([...spanMetrics].sort((a, b) => a.id.localeCompare(b.id)));

export function createMetric(i: SpansMetricInput): Promise<SpansMetric> {
  const id = (i.id ?? '')
    .trim()
    .replace(/"/g, '')
    .replace(/[^A-Za-z0-9_.]/g, '_');
  if (!/^[A-Za-z]/.test(id)) return fail('Metric name must start with a letter');
  if (spanMetrics.some((m) => m.id === id)) return fail(`Metric with name '${id}' already exists`);
  if (i.aggregationType === 'distribution' && !i.path?.trim()) return fail('a distribution needs the path of a numeric attribute');
  const m: SpansMetric = {
    id,
    query: (i.query ?? '').trim() || '*',
    aggregationType: i.aggregationType ?? 'count',
    path: i.aggregationType === 'distribution' ? (i.path ?? '@duration').trim() : null,
    includePercentiles: i.aggregationType === 'distribution' && !!i.includePercentiles,
    groupBy: (i.groupBy ?? []).filter((g) => g.path.trim()),
  };
  spanMetrics.push(m);
  return delay(m);
}

export function updateMetric(id: string, i: SpansMetricInput): Promise<SpansMetric> {
  const m = spanMetrics.find((x) => x.id === id);
  if (!m) return fail(`Metric with name '${id}' not found`);
  Object.assign(m, {
    query: (i.query ?? '').trim() || '*',
    includePercentiles: m.aggregationType === 'distribution' && !!i.includePercentiles,
    groupBy: (i.groupBy ?? []).filter((g) => g.path.trim()),
  });
  return delay(m);
}

export function deleteMetric(id: string): Promise<void> {
  spanMetrics = spanMetrics.filter((m) => m.id !== id);
  return delay(undefined);
}

// ================================================================ span pipelines

let spanPipelines: SpanPipeline[] = [
  {
    id: 'b6f1a0c2-6a2e-4d7b-9a51-1f2e3d4c5b6a',
    name: 'Normalize HTTP routes',
    query: 'service:(api-gateway OR storefront-api)',
    enabled: true,
    processors: [{ type: 'remapper', name: 'Route from target', sources: ['http.target'], target: 'http.route', preserveSource: true, categories: [] }],
    position: 0,
    updatedBy: 'Priya Shah',
    updatedAt: Date.now() - 6 * DAY,
  },
];

export const pipelines = (): Promise<SpanPipeline[]> => delay(spanPipelines);

function checkPipeline(i: SpanPipelineInput): string | null {
  if (!i.name.trim()) return 'a pipeline needs a name';
  for (const p of i.processors ?? []) {
    if (!p.target?.trim() || (p.type === 'remapper' ? !p.sources?.length : !p.categories?.length)) return `processor '${p.name}' needs sources or categories and a target`;
  }
  return null;
}

export function createPipeline(i: SpanPipelineInput): Promise<SpanPipeline> {
  const err = checkPipeline(i);
  if (err) return fail(err);
  const p: SpanPipeline = {
    id: crypto.randomUUID(),
    name: i.name.trim(),
    query: i.query?.trim() ?? '',
    enabled: i.enabled,
    processors: i.processors ?? [],
    position: spanPipelines.length,
    updatedBy: 'You',
    updatedAt: Date.now(),
  };
  spanPipelines.push(p);
  return delay(p);
}

export function updatePipeline(id: string, i: SpanPipelineInput): Promise<SpanPipeline> {
  const err = checkPipeline(i);
  if (err) return fail(err);
  const p = spanPipelines.find((x) => x.id === id);
  if (!p) return fail('pipeline not found');
  Object.assign(p, { name: i.name.trim(), query: i.query?.trim() ?? '', enabled: i.enabled, processors: i.processors ?? [], updatedBy: 'You', updatedAt: Date.now() });
  return delay(p);
}

export function deletePipeline(id: string): Promise<void> {
  spanPipelines = spanPipelines.filter((p) => p.id !== id);
  return delay(undefined);
}

// ================================================================ defaults

let settings: { defaultEnv: string | null; primaryTags: string[]; defaultPrimaryTag: string | null } = { defaultEnv: null, primaryTags: [], defaultPrimaryTag: null };

export function defaults(): Promise<ApmDefaults> {
  return delay({
    ...settings,
    detectedEnv: 'prod',
    envs: [
      { env: 'prod', services: 8 },
      { env: 'staging', services: 6 },
    ],
    tagCandidates: ['availability-zone', 'cloud.region', 'datacenter', 'k8s.cluster.name', 'k8s.namespace.name', 'team'],
  });
}

export function updateDefaults(i: ApmDefaultsInput): Promise<ApmDefaults> {
  const tags = [...new Set((i.primaryTags ?? []).map((t) => t.trim().toLowerCase()).filter(Boolean))];
  if (tags.length > 3) return fail('at most 3 primary tags');
  if (i.defaultPrimaryTag && !tags.includes(i.defaultPrimaryTag)) return fail('the default primary tag must be one of the primary tags');
  settings = { defaultEnv: i.defaultEnv?.replace(/^env:/, '') || null, primaryTags: tags, defaultPrimaryTag: i.defaultPrimaryTag || null };
  return defaults();
}

// ================================================================ flow, volume, ingestion

function indexedShare(): number {
  return filters.filter((f) => f.enabled).reduce((a, f) => a + (SHARE[f.id] ?? 0) * f.rate, 0);
}

export function overview(r: TimeRange): Promise<ApmFlow> {
  const ingested = series(r, totalSps);
  const share = indexedShare();
  const sum = (s: ApmPoint[]) => s.reduce((a, p) => a + p.v, 0);
  const generated = ingested.map((p) => ({ t: p.t, v: Math.round(p.v * SAMPLED_FACTOR) }));
  const indexed = ingested.map((p) => ({ t: p.t, v: Math.round(p.v * share) }));
  return delay({
    sdks: 14,
    agents: 5,
    otelSources: 1,
    services: Object.keys(SERVICES).length,
    spansGenerated: sum(generated),
    spansIngested: sum(ingested),
    spansIndexed: sum(indexed),
    generated,
    ingested,
    indexed,
    pipelines: spanPipelines.filter((p) => p.enabled).length,
    serviceRemappings: 0,
    metrics: spanMetrics.length,
    retentionFilters: filters.filter((f) => f.enabled).length,
    retentionDays: 15,
  });
}

export function volume(r: TimeRange): Promise<SpanVolume> {
  const n = Math.round(totalSps * secs(r));
  return delay({ spansIngested: n, bytesIngested: n * BYTES_PER_SPAN, spansLive: Math.round(totalSps * 900), spansIndexed: Math.round(n * indexedShare()), retentionDays: 15 });
}

export function ingestion(r: TimeRange): Promise<Ingestion> {
  const services: IngestionService[] = Object.entries(TRAFFIC).map(([service, t]) => {
    const own = t.sps * BYTES_PER_SPAN;
    const otel = t.configs.includes('OTEL');
    const other = !otel && t.configs.every((c) => c === 'RUM' || c === 'APP & API PROTECTION' || c === 'SYNTHETICS');
    return {
      service,
      kind: t.kind,
      env: 'prod',
      language: SERVICES[service].lang,
      downstreamBytesPerSecond: t.root ? own / t.share + (service === 'api-gateway' ? 300 * BYTES_PER_SPAN : 0) : 0,
      ingestedBytesPerSecond: own,
      trafficShare: t.share,
      ingestedTracesPerSecond: t.root ? t.sps / 6.5 : 0,
      spansPerSecond: t.sps,
      root: t.root,
      configurations: t.configs,
      managedBy: otel ? 'otel' : other ? 'other' : 'apm',
    };
  });
  const by = (pred: (c: string[]) => boolean) =>
    series(
      r,
      Object.values(TRAFFIC)
        .filter((t) => pred(t.configs))
        .reduce((a, t) => a + t.sps, 0) * BYTES_PER_SPAN,
    );
  const apm = by((c) => !c.includes('OTEL'));
  const otel = by((c) => c.includes('OTEL'));
  const other = apm.map((p) => ({ t: p.t, v: Math.round(p.v * 0.04) }));
  return delay({
    services: services.sort((a, b) => b.downstreamBytesPerSecond - a.downstreamBytesPerSecond || b.ingestedBytesPerSecond - a.ingestedBytesPerSecond),
    bytesIngested: Math.round(totalSps * BYTES_PER_SPAN * secs(r)),
    spansIngested: Math.round(totalSps * secs(r)),
    overTime: { apm, other, otel },
  });
}
