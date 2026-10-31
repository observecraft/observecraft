// In-browser generated metrics for mock mode (Logs → Configuration → Generate Metrics). State lives
// in memory for the session; validation follows the server (routes/log_metrics.rs). The mock metric
// engine (metricsEngine.ts) graphs every definition, so "Open in Metrics Explorer" shows a series.
import type { LogMetric, LogMetricGroupBy, LogMetricInput } from '../logMetrics/types';

const MAX_METRICS = 100;
const MAX_GROUP_BYS = 10;
const MAX_NAME_CHARS = 200;
const MAX_QUERY_CHARS = 2000;
const MAX_SERIES = 1000;
const RESERVED_PREFIXES = ['trace.', 'datadog.', 'observecraft.'];

const fail = (message: string): never => {
  throw new Error(message);
};
const copy = <T>(v: T): T => structuredClone(v);

interface State {
  metrics: LogMetric[];
  seq: number;
}

let state: State | null = null;

const by = (path: string, tagName = path.replace(/^@/, '')): LogMetricGroupBy => ({ path, tagName });

function db(): State {
  if (state) return state;
  const metric = (id: string, name: string, query: string, groupBy: LogMetricGroupBy[], path: string | null = null): LogMetric => ({
    id,
    name,
    query,
    aggregation: path ? 'distribution' : 'count',
    path,
    groupBy,
    includePercentiles: !!path,
    maxSeries: MAX_SERIES,
  });
  state = {
    metrics: [
      metric('lm-0001', 'logs.checkout.errors', 'service:(order-service OR billing) status:error', [by('service'), by('@http.status_code')]),
      metric('lm-0002', 'logs.gateway.requests', 'service:api-gateway source:nginx', [by('status'), by('env')]),
      metric('lm-0003', 'logs.storefront.latency', 'service:storefront-api', [by('env')], '@duration'),
      metric('lm-0004', 'logs.inventory.stock_warnings', 'service:inventory-service status:warn', [by('kube_namespace')]),
    ],
    seq: 5,
  };
  return state;
}

const sorted = () => [...db().metrics].sort((a, b) => a.name.localeCompare(b.name));

const validName = (n: string) => /^[A-Za-z][A-Za-z0-9_.]*$/.test(n) && !n.endsWith('.') && !n.includes('..');
const validPath = (p: string) => /^@?[A-Za-z0-9_.-]+$/.test(p);
const validTag = (t: string) => /^[A-Za-z0-9_./-]+$/.test(t) && !t.startsWith('oc.');

/** Trimmed and checked as the server does; throws with the server's messages. */
function validate(input: LogMetricInput): Omit<LogMetric, 'id' | 'maxSeries'> {
  const name = input.name.trim();
  const query = (input.query ?? '').trim();
  if (name.length > MAX_NAME_CHARS || !validName(name)) fail(`Metric names are up to ${MAX_NAME_CHARS} letters, digits, underscores and dots, starting with a letter (e.g. logs.checkout.errors)`);
  const reserved = RESERVED_PREFIXES.find((p) => name.startsWith(p));
  if (reserved) fail(`Metric names can't start with '${reserved}'`);
  if (query.length > MAX_QUERY_CHARS) fail(`The query is at most ${MAX_QUERY_CHARS} characters`);
  let path: string | null = null;
  if (input.aggregation === 'distribution') {
    const p = (input.path ?? '').trim();
    if (!p.startsWith('@') || !validPath(p)) fail('A distribution measures a numeric attribute, e.g. @duration');
    path = p;
  }
  const groupBy = input.groupBy ?? [];
  if (groupBy.length > MAX_GROUP_BYS) fail(`At most ${MAX_GROUP_BYS} group-bys`);
  const tags = new Set<string>();
  const normalized = groupBy.map((g) => {
    const p = g.path.trim();
    if (!validPath(p) || p === '@') fail(`Can't group by '${p}': use a tag (env), a reserved field (service) or an attribute (@http.status_code)`);
    const tagName = (g.tagName ?? '').trim() || p.replace(/^@/, '');
    if (!validTag(tagName)) fail(`'${tagName}' isn't a valid tag name`);
    if (tags.has(tagName)) fail(`Two group-bys are tagged '${tagName}'`);
    tags.add(tagName);
    return { path: p, tagName };
  });
  return { name, query, aggregation: input.aggregation, path, groupBy: normalized, includePercentiles: input.aggregation === 'distribution' && !!input.includePercentiles };
}

const find = (id: string) => db().metrics.find((m) => m.id === id) ?? fail('Generated metric not found');

export async function listLogMetrics(): Promise<LogMetric[]> {
  return copy(sorted());
}

export async function createLogMetric(input: LogMetricInput): Promise<LogMetric> {
  const s = db();
  const v = validate(input);
  if (s.metrics.length >= MAX_METRICS) fail(`An organization can have at most ${MAX_METRICS} generated metrics`);
  if (s.metrics.some((m) => m.name === v.name)) fail(`A metric named '${v.name}' already exists`);
  const metric: LogMetric = { id: `lm-${String(s.seq++).padStart(4, '0')}`, ...v, maxSeries: MAX_SERIES };
  s.metrics.push(metric);
  return copy(metric);
}

export async function updateLogMetric(id: string, input: LogMetricInput): Promise<LogMetric> {
  const v = validate(input);
  const metric = find(id);
  if (metric.name !== v.name) fail("A generated metric can't be renamed: create a new one");
  Object.assign(metric, v);
  return copy(metric);
}

export async function deleteLogMetric(id: string): Promise<void> {
  const s = db();
  find(id);
  s.metrics = s.metrics.filter((m) => m.id !== id);
}

/** The definitions, for the mock metric engine's catalog (synchronous). */
export function generatedMetrics(): LogMetric[] {
  return sorted();
}
