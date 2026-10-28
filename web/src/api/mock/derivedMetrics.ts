// In-browser derived metrics for mock mode (Metrics → Derived Metrics). State lives in memory for the
// session; validation follows the server (routes/derived_metrics.rs). The mock metric engine resolves
// the names (metricsEngine.ts), so a derived metric graphs in the explorer and on dashboards.
import type { DerivedMetric, DerivedMetricInput } from '../derivedMetrics/client';
import { expandFormula } from '../../lib/metricFormula';
import { formulaQueries, parseMetricQuery } from '../../lib/metricQuery';
import type { DerivedFormulas } from '../../lib/derivedMetrics';

const LETTERS = 'abcdefghij';
const RESERVED_PREFIXES = ['trace.', 'datadog.', 'observecraft.'];

const fail = (message: string): never => {
  throw new Error(message);
};

let state: { metrics: DerivedMetric[]; seq: number } | null = null;

const DAY = 86_400_000;

function db() {
  if (state) return state;
  const now = Date.now();
  const metric = (id: string, name: string, description: string, queries: string[], expression: string, createdBy: string, ago: number): DerivedMetric => ({
    id,
    name,
    description,
    queries,
    expression,
    formula: compile(queries, expression),
    createdBy,
    createdAt: now - ago,
    updatedAt: now - ago,
  });
  state = {
    metrics: [
      metric(
        'dm-0001',
        'checkout.error_ratio',
        'Share of order-service requests that ended in an error, in percent',
        ['sum:trace.errors{service:order-service} by {env}', 'sum:trace.requests{service:order-service} by {env}'],
        'a / b * 100',
        'Alex Doe',
        3 * DAY,
      ),
      metric('dm-0002', 'storefront.cpu.busy', 'CPU not idle on the storefront hosts', ['avg:system.cpu.idle{*} by {host}'], '100 - a', 'Sam Lee', 9 * DAY),
    ],
    seq: 3,
  };
  return state;
}

function compile(queries: string[], expression: string): string {
  if (queries.length === 0 || queries.length > LETTERS.length) fail(`A derived metric has 1 to ${LETTERS.length} queries`);
  queries.forEach((q, i) => parseMetricQuery(q) ?? fail(`Query ${LETTERS[i]}: expected <agg>:<metric>{…}`));
  if (!expression.trim()) fail('The formula is required, e.g. a / b * 100');
  const formula = expandFormula(expression, Object.fromEntries(queries.map((q, i) => [LETTERS[i], q])));
  if (!formula || !formulaQueries(formula)) fail('Invalid formula: use the query letters, numbers, + - * / and parentheses');
  return formula as string;
}

/** Name → formula, for the mock metric engine. */
export function derivedFormulas(): DerivedFormulas {
  return Object.fromEntries(db().metrics.map((m) => [m.name, m.formula]));
}

const validName = (n: string) => /^[A-Za-z][A-Za-z0-9_.]*$/.test(n) && !n.endsWith('.') && !n.includes('..');

function validate(input: DerivedMetricInput, self?: string): Pick<DerivedMetric, 'name' | 'description' | 'queries' | 'expression' | 'formula'> {
  const name = input.name.trim();
  if (name.length > 200 || !validName(name)) fail('Metric names are up to 200 letters, digits, underscores and dots, starting with a letter (e.g. checkout.error_ratio)');
  const reserved = RESERVED_PREFIXES.find((p) => name.startsWith(p));
  if (reserved) fail(`Metric names can't start with '${reserved}'`);
  const queries = input.queries.map((q) => q.trim());
  const others = db().metrics.filter((m) => m.name !== self);
  for (const q of queries) {
    const metric = parseMetricQuery(q)?.metric;
    if (metric && (metric === name || others.some((m) => m.name === metric))) fail(`'${metric}' is a derived metric: derived metrics can't be nested`);
  }
  const expression = input.expression.trim();
  return { name, description: (input.description ?? '').trim(), queries, expression, formula: compile(queries, expression) };
}

const sorted = () => [...db().metrics].sort((a, b) => a.name.localeCompare(b.name)).map((m) => ({ ...m }));

export async function listDerivedMetrics(): Promise<DerivedMetric[]> {
  return sorted();
}

export async function createDerivedMetric(input: DerivedMetricInput): Promise<DerivedMetric> {
  const v = validate(input);
  const s = db();
  if (s.metrics.some((m) => m.name === v.name)) fail(`A derived metric named '${v.name}' already exists`);
  const now = Date.now();
  const m: DerivedMetric = { id: `dm-${String(s.seq++).padStart(4, '0')}`, ...v, createdBy: 'You', createdAt: now, updatedAt: now };
  s.metrics.push(m);
  return { ...m };
}

export async function updateDerivedMetric(id: string, input: DerivedMetricInput): Promise<DerivedMetric> {
  const s = db();
  const current = s.metrics.find((m) => m.id === id) ?? fail('Derived metric not found');
  const v = validate(input, current.name);
  if (v.name !== current.name) fail("A derived metric can't be renamed: create a new one");
  Object.assign(current, v, { updatedAt: Date.now() });
  return { ...current };
}

export async function deleteDerivedMetric(id: string): Promise<void> {
  const s = db();
  if (!s.metrics.some((m) => m.id === id)) fail('Derived metric not found');
  s.metrics = s.metrics.filter((m) => m.id !== id);
}
