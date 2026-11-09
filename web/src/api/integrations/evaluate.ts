// Execution adapter for the original integrations-core request/formula schema.
// No eval/Function: formulas are parsed using a small, closed arithmetic grammar.
import type { MetricSeries } from '../types';
import type { TimeAggregator } from '../dashboards/types';

export interface IntegrationQuery {
  name: string;
  query: string;
  data_source: string;
  aggregator?: string;
  /** Newer table requests name the metric and its filter instead of writing `query`. */
  metric_name?: string;
  filter?: string;
}
interface TableGroupBy {
  group_keys?: { query: string; group_key: string }[];
}
interface TableColumn {
  type?: string;
  compute?: { query: string; aggregation?: string };
}
export interface IntegrationFormula {
  formula: string;
  alias?: string;
}
export interface IntegrationRequest {
  query?: { query_string: string; data_source: string };
  q?: string;
  queries?: IntegrationQuery[];
  formulas?: IntegrationFormula[];
  /** Table requests (`request_type: table`): the rows' groups and the compute columns. */
  rows?: { group_by?: TableGroupBy[] };
  columns?: TableColumn[];
  aggregator?: string;
  display_type?: string;
  style?: { line_type?: string; palette?: string };
  conditional_formats?: { comparator: string; value: number; palette: string }[];
}
export interface IntegrationWidgetDefinition {
  type: string;
  title?: string;
  requests: IntegrationRequest[];
  precision?: number;
  custom_unit?: string;
  yaxis?: { min?: string; max?: string; scale?: string; include_zero?: boolean };
  show_legend?: boolean;
  [key: string]: unknown;
}

/** A metric query (with its `.fill()`-style functions) inside a function such as top(…). */
const WRAPPED_QUERY = /(?:avg|sum|min|max|count):[\w./-]+\{[^{}]*\}(?:\s+by\s+\{[\w, -]+\})?(?:\.[a-z_]+\([^()]*\))*/;

/**
 * A request's metric queries and formulas, with a legacy `q` as query1 and any function wrapped
 * around a metric query (`top(avg:…, 10, 'mean', 'desc')`) moved into the formulas, where it applies
 * to that query's name.
 */
/**
 * Datadog's metric query forms as the engine reads them: a query without an aggregator averages
 * (`system.load.1{*}`), `count(v: …):` counts, and a percentile (`p95:latency{*}`) is the distribution's
 * statistic, the highest across the group's series (`max:latency.p95{*}`). Every query of a formula too.
 */
export function canonicalMetricQuery(text: string): string {
  return text
    .replace(/count\(v:[^)]*\):/g, 'count:')
    .replace(/(^|[^\w.:$])(?:(avg|sum|min|max|count|p\d{1,2}(?:\.\d+)?):)?([A-Za-z_][\w./-]*)\{/g, (all, pre: string, agg: string | undefined, name: string) => {
      if (!agg) return `${pre}avg:${name}{`;
      if (agg.startsWith('p')) return `${pre}max:${name}.${agg}{`;
      return all;
    });
}

/** Metric queries written inside formulas (`query2-sum:m{*}`): queries of their own, `inline1`, `inline2`, … in the formula. */
function inlineQueries(queries: IntegrationQuery[], formulas: IntegrationFormula[] | undefined): { queries: IntegrationQuery[]; formulas?: IntegrationFormula[] } {
  if (!formulas) return { queries, formulas };
  const inline = new RegExp(WRAPPED_QUERY.source, 'g');
  const added: IntegrationQuery[] = [];
  const rewritten = formulas.map((f) => ({
    ...f,
    formula: canonicalMetricQuery(f.formula).replace(inline, (query) => {
      const name = `inline${added.length + 1}`;
      added.push({ name, data_source: 'metrics', query } as IntegrationQuery);
      return name;
    }),
  }));
  return added.length ? { queries: [...queries, ...added], formulas: rewritten } : { queries, formulas };
}

/** A log or security query in the legacy request form (`log_query`, `security_query`). */
interface LegacyQuery {
  index?: string;
  search?: { query?: string };
  group_by?: unknown;
  compute?: { aggregation?: string; metric?: string; facet?: string };
}

/** A legacy `q` listing several queries (`a{*}, b{*}`): each one, split at the commas outside braces and parentheses. */
function splitQueries(q: string): string[] {
  const out: string[] = [];
  let depth = 0;
  let start = 0;
  for (let i = 0; i < q.length; i++) {
    if (q[i] === '{' || q[i] === '(') depth++;
    else if (q[i] === '}' || q[i] === ')') depth--;
    else if (q[i] === ',' && depth === 0) {
      out.push(q.slice(start, i).trim());
      start = i + 1;
    }
  }
  out.push(q.slice(start).trim());
  return out.filter(Boolean);
}

/** `metric_name` + `filter` as a metric query, aggregated as its compute column and grouped by the table's rows. */
function namedMetricQuery(q: IntegrationQuery, request: Pick<IntegrationRequest, 'rows' | 'columns'>): string {
  const aggregation = request.columns?.find((c) => c.compute?.query === q.name)?.compute?.aggregation ?? 'avg';
  const groups = (request.rows?.group_by ?? []).flatMap((g) => (g.group_keys ?? []).filter((k) => k.query === q.name).map((k) => k.group_key));
  return `${aggregation}:${q.metric_name}{${q.filter || '*'}}${groups.length ? ` by {${groups.join(',')}}` : ''}`;
}

export function normalizeRequest(request: Pick<IntegrationRequest, 'q' | 'queries' | 'formulas' | 'rows' | 'columns'>): { queries: IntegrationQuery[]; formulas?: IntegrationFormula[] } {
  // Cost queries (`sum:aws.cost.amortized{*}`) are cost metrics: the metric engine reads them.
  // Legacy requests: `q` (metric queries, comma-separated), `log_query` and `security_query` (one query each).
  const old = request as { log_query?: LegacyQuery; security_query?: LegacyQuery };
  const legacyQuery = (source: string, q: LegacyQuery) => {
    const { index: _index, compute, ...rest } = q;
    const metric = compute?.metric ?? compute?.facet;
    return { name: 'query1', data_source: source, ...rest, compute: { aggregation: compute?.aggregation ?? 'count', ...(metric ? { metric } : {}) } } as unknown as IntegrationQuery;
  };
  const legacy = request.q
    ? splitQueries(request.q).map((query, i) => ({ name: `query${i + 1}`, query, data_source: 'metrics' }))
    : old.log_query
      ? [legacyQuery('logs', old.log_query)]
      : old.security_query
        ? [legacyQuery('security_signals', old.security_query)]
        : [];
  const raw: IntegrationQuery[] = (request.queries ?? (legacy as IntegrationQuery[]))
    .map((q) => (q.data_source === 'cloud_cost' ? { ...q, data_source: 'metrics' } : q))
    .map((q) => (q.data_source === 'metrics' && typeof q.query !== 'string' && q.metric_name ? { ...q, query: namedMetricQuery(q, request) } : q))
    .map((q) => (q.data_source === 'metrics' && typeof q.query === 'string' ? { ...q, query: canonicalMetricQuery(q.query) } : q));
  const wrappers = new Map<string, string>();
  const queries = raw.map((q) => {
    const m = q.data_source === 'metrics' ? WRAPPED_QUERY.exec(q.query) : null;
    if (!m || m[0] === q.query.trim()) return q;
    wrappers.set(q.name, q.query.replace(m[0], q.name));
    return { ...q, query: m[0] };
  });
  if (!wrappers.size) return inlineQueries(queries, request.formulas);
  // A time comparison of a wrapped query (hour_before(query1)) is wrapped whole: top(hour_before(query1), …).
  const wrap = (text: string) =>
    text.replace(
      /\b(?:(?:hour_before|day_before|week_before|month_before)\(\s*(\w+)\s*\)|(?:timeshift|calendar_shift)\(\s*(\w+)\s*,[^()]*\)|(\w+))/g,
      (call: string, a?: string, b?: string, bare?: string) => {
        const name = a ?? b ?? bare ?? '';
        const wrapper = wrappers.get(name);
        if (!wrapper) return call;
        return `(${wrapper.replace(new RegExp(`\\b${name}\\b`), call)})`;
      },
    );
  const formulas = request.formulas?.map((f) => ({ ...f, formula: wrap(f.formula) })) ?? queries.map((q) => ({ formula: wrappers.get(q.name) ?? q.name }));
  return inlineQueries(queries, formulas);
}

export function metricPlan(text: string) {
  const m = /^((?:avg|sum|min|max|count):[\w./-]+\{[^{}]*\}(?:\s+by\s+\{[\w, -]+\})?)(.*)$/.exec(canonicalMetricQuery(text));
  if (!m) throw new Error(`Unsupported metric expression: ${text}`);
  const operations: { name: string; args: string[] }[] = [];
  let rest = m[2];
  while (rest) {
    const op = /^\.([a-z_]+)\(([^()]*)\)/.exec(rest);
    if (!op) throw new Error(`Unsupported metric function: ${rest}`);
    if (!['fill', 'rollup', 'as_rate', 'as_count', 'weighted'].includes(op[1])) throw new Error(`Metric function ${op[1]} is not supported yet`);
    operations.push({ name: op[1], args: op[2].split(',').map((v) => v.trim()) });
    rest = rest.slice(op[0].length);
  }
  return { query: m[1], operations };
}

type Vector = MetricSeries[];
const key = (s: MetricSeries) => (s.name.includes(':') ? s.name : '*');

function binary(a: Vector, b: Vector, fn: (a: number, b: number) => number, op?: string): Vector {
  const out: Vector = [];
  for (const left of a) {
    for (const right of b) {
      if (key(left) !== '*' && key(right) !== '*' && key(left) !== key(right)) continue;
      const rightPoints = new Map(right.points);
      const lc = left.name === '__constant';
      const rc = right.name === '__constant';
      const times = lc ? right.points.map(([t]) => t) : left.points.map(([t]) => t);
      const lp = new Map(left.points);
      const points: [number, number][] = [];
      for (const t of times) {
        const x = lc ? left.points[0][1] : lp.get(t);
        const y = rc ? right.points[0][1] : rightPoints.get(t);
        if (x === undefined || y === undefined) continue;
        const value = fn(x, y);
        if (Number.isFinite(value)) points.push([t, value]);
      }
      let unit: string | undefined;
      if (lc || rc) {
        const quantity = lc ? right : left;
        const constant = (lc ? left : right).points[0][1];
        unit = quantity.unit;
        if (op === '/' && lc) unit = undefined;
        if ((quantity.unit === 'n{core}' || quantity.unit === 'nanocore') && ((op === '/' && rc && constant === 1e9) || (op === '*' && constant === 1e-9))) unit = '{core}';
        if ((quantity.unit === '{core}' || quantity.unit === 'core') && op === '*' && constant === 1e9) unit = 'n{core}';
      } else if (left.unit === right.unit) {
        if (op === '+' || op === '-') unit = left.unit;
        if (op === '/') unit = '1';
      }
      if (points.length) out.push({ ...(lc || key(left) === '*' ? right : left), name: lc || key(left) === '*' ? right.name : left.name, unit, points });
    }
  }
  return out;
}

export function aggregate(values: number[], method = 'avg'): number | undefined {
  if (!values.length) return undefined;
  switch (method) {
    case 'last':
      return values.at(-1);
    case 'sum':
      return values.reduce((a, b) => a + b, 0);
    case 'min':
      return Math.min(...values);
    case 'max':
      return Math.max(...values);
    case 'avg':
    case 'mean':
      return values.reduce((a, b) => a + b, 0) / values.length;
    default:
      throw new Error(`Unsupported aggregation: ${method}`);
  }
}

const POINTWISE: Record<string, (v: number) => number> = { abs: Math.abs, log2: Math.log2, log10: Math.log10 };
const PER_UNIT: Record<string, number> = { per_second: 1, per_minute: 60, per_hour: 3600 };

/** Seconds per calendar_shift unit (a month as 30 days). */
const CALENDAR_UNIT: Record<string, number> = { h: 3600, d: 86_400, w: 604_800, mo: 2_592_000 };

/** Seconds each time-comparison function looks back (month_before is 28 days, as in Datadog). */
const LOOK_BACK: Record<string, number> = { hour_before: 3600, day_before: 86_400, week_before: 604_800, month_before: 2_419_200 };

/**
 * Rewrites time comparisons (`week_before(query1)`, `timeshift(query1, -3600)`, `calendar_shift(query1, '-1w', 'UTC')`, …) into inputs read
 * over an earlier range: `query1__shift604800` is query1 queried `seconds` earlier, its points moved
 * back onto the current range.
 */
export function timeShifts(text: string): { text: string; shifts: { name: string; query: string; seconds: number }[] } {
  const shifts: { name: string; query: string; seconds: number }[] = [];
  const shift = (query: string, seconds: number) => {
    const name = `${query}__shift${seconds}`;
    if (!shifts.some((s) => s.name === name)) shifts.push({ name, query, seconds });
    return name;
  };
  const rewritten = text
    .replace(/\b(hour_before|day_before|week_before|month_before)\(\s*(\w+)\s*\)/g, (_, fn: string, query: string) => shift(query, LOOK_BACK[fn]))
    .replace(/\btimeshift\(\s*(\w+)\s*,\s*-(\d+)\s*\)/g, (_, query: string, seconds: string) => shift(query, Number(seconds)))
    // calendar_shift(query1, '-1w', 'UTC'): the time zone only matters for month boundaries, taken as 30 days.
    .replace(/\bcalendar_shift\(\s*(\w+)\s*,\s*'-(\d+)(mo|h|d|w)'\s*(?:,\s*'[^']*'\s*)?\)/g, (_, query: string, n: string, unit: string) => shift(query, Number(n) * CALENDAR_UNIT[unit]));
  // Report shifts in the order they appear in the formula.
  shifts.sort((a, b) => rewritten.indexOf(a.name) - rewritten.indexOf(b.name));
  return { text: rewritten, shifts };
}

/** The next quarter of a series' span (same step), projected linearly or by repeating its last stretch. */
function forecastPoints(points: [number, number][], algorithm: 'linear' | 'seasonal'): [number, number][] {
  const n = points.length;
  if (n < 2) return [];
  const step = (points[n - 1][0] - points[0][0]) / (n - 1);
  const ahead = Math.max(1, Math.round(n / 4));
  if (algorithm === 'seasonal') return Array.from({ length: ahead }, (_, k) => [points[n - 1][0] + (k + 1) * step, points[n - ahead + k][1]]);
  const meanT = points.reduce((a, [t]) => a + t, 0) / n;
  const meanV = points.reduce((a, [, v]) => a + v, 0) / n;
  const slope = points.reduce((a, [t, v]) => a + (t - meanT) * (v - meanV), 0) / points.reduce((a, [t]) => a + (t - meanT) ** 2, 0);
  return Array.from({ length: ahead }, (_, k) => {
    const t = points[n - 1][0] + (k + 1) * step;
    return [t, meanV + slope * (t - meanT)];
  });
}

export function formula(text: string, inputs: Record<string, Vector>): Vector {
  const tokens = text.match(/'[^']*'|"[^"]*"|\d+(?:\.\d+)?(?:e[+-]?\d+)?|[a-zA-Z_]\w*|[()+*/,\-]/g) ?? [];
  if (tokens.join('') !== text.replace(/\s/g, '')) throw new Error(`Unsupported formula: ${text}`);
  let i = 0;
  function expr(min = 0): Vector {
    let value: Vector;
    const token = tokens[i++];
    if (token === '-') value = expr(3).map((s) => ({ ...s, points: s.points.map(([t, v]) => [t, -v]) }));
    else if (token === '(') {
      value = expr();
      if (tokens[i++] !== ')') throw new Error('Unclosed formula');
    } else if (/^\d/.test(token ?? '')) value = [{ name: '__constant', points: [[0, Number(token)]] }];
    else if (tokens[i] === '(') {
      i++;
      const arg = expr();
      if (token === 'forecast') {
        // forecast(query, 'linear' | 'seasonal', deviations[, …options]): the observed points, then the
        // next quarter of their span projected (a straight-line fit, or the last stretch repeated).
        if (tokens[i++] !== ',') throw new Error('forecast requires an algorithm');
        const algorithm = tokens[i++]?.slice(1, -1);
        while (tokens[i] !== ')' && i < tokens.length) i++;
        i++;
        if (algorithm !== 'linear' && algorithm !== 'seasonal') throw new Error(`Unsupported forecast algorithm: ${algorithm}`);
        value = arg.map((s) => ({ ...s, points: [...s.points, ...forecastPoints(s.points, algorithm)] }));
      } else if (token === 'top') {
        if (tokens[i++] !== ',') throw new Error('top requires a limit');
        const limit = Number(tokens[i++]);
        if (tokens[i++] !== ',') throw new Error('top requires an aggregation');
        const method = tokens[i++]?.slice(1, -1);
        if (tokens[i++] !== ',') throw new Error('top requires a direction');
        const direction = tokens[i++]?.slice(1, -1);
        if (tokens[i++] !== ')' || !Number.isInteger(limit) || limit < 1 || limit > 1000 || !['avg', 'sum', 'min', 'max', 'last', 'mean'].includes(method) || !['asc', 'desc'].includes(direction))
          throw new Error('Invalid top arguments');
        value = [...arg]
          .sort(
            (a, b) =>
              (direction === 'asc' ? 1 : -1) *
              ((aggregate(
                a.points.map((p) => p[1]),
                method,
              ) ?? 0) -
                (aggregate(
                  b.points.map((p) => p[1]),
                  method,
                ) ?? 0)),
          )
          .slice(0, limit);
      } else {
        let second: Vector | undefined;
        if (tokens[i] === ',') {
          i++;
          second = expr();
        }
        if (tokens[i++] !== ')') throw new Error(`Unsupported function: ${token}`);
        if (token === 'exclude_null')
          value = arg.filter((s) => {
            const groups = s.tags ? Object.values(s.tags) : s.name === '*' ? [] : s.name.split(/,\s*/).map((part) => part.slice(part.indexOf(':') + 1));
            return groups.every((v) => v !== '' && v !== 'N/A');
          });
        else if (token === 'default_zero') value = arg.length ? arg : [{ name: '__constant', points: [[0, 0]] }];
        else if ((token === 'max' || token === 'min') && second) value = binary(arg, second, token === 'max' ? Math.max : Math.min);
        else if (token === 'round') value = arg.map((s) => ({ ...s, points: s.points.map(([t, v]) => [t, Math.round(v)]) }));
        else if (token === 'derivative' || token === 'rate')
          // Change per second between consecutive points.
          value = arg.map((s) => ({ ...s, points: s.points.slice(1).map(([t, v], j) => [t, ((v - s.points[j][1]) * 1000) / (t - s.points[j][0])]) }));
        else if (token === 'dt') value = arg.map((s) => ({ ...s, points: s.points.slice(1).map(([t], j) => [t, (t - s.points[j][0]) / 1000]) }));
        else if ((token === 'cutoff_max' || token === 'cutoff_min') && second) {
          const limit = second[0]?.points[0]?.[1] ?? 0;
          value = arg.map((s) => ({ ...s, points: s.points.filter(([, v]) => (token === 'cutoff_max' ? v <= limit : v >= limit)) }));
        } else if (token === 'autosmooth')
          // A trailing moving average over five points.
          value = arg.map((s) => ({ ...s, points: s.points.map(([t], j) => [t, s.points.slice(Math.max(0, j - 4), j + 1).reduce((a, p) => a + p[1], 0) / (Math.min(j, 4) + 1)]) }));
        else if (/^ewma_\d+$/.test(token)) {
          // An exponentially weighted moving average over the given span of points.
          const alpha = 2 / (Number(token.slice(5)) + 1);
          value = arg.map((s) => {
            let avg: number | undefined;
            return { ...s, points: s.points.map(([t, v]) => [t, (avg = avg === undefined ? v : alpha * v + (1 - alpha) * avg)]) };
          });
        } else if (token === 'clamp_min' && second) value = binary(arg, second, Math.max);
        else if (token === 'clamp_max' && second) value = binary(arg, second, Math.min);
        else if (Object.hasOwn(POINTWISE, token)) value = arg.map((s) => ({ ...s, points: s.points.map(([t, v]) => [t, POINTWISE[token](v)]) }));
        else if (token === 'cumsum')
          value = arg.map((s) => {
            let total = 0;
            return { ...s, points: s.points.map(([t, v]) => [t, (total += v)]) };
          });
        else if (Object.hasOwn(PER_UNIT, token))
          // Change per second / minute / hour between consecutive points (counters).
          value = arg.map((s) => ({ ...s, points: s.points.slice(1).map(([t, v], j) => [t, ((v - s.points[j][1]) * PER_UNIT[token] * 1000) / (t - s.points[j][0])]) }));
        else if (token === 'monotonic_diff')
          // A drop is a counter reset: the new value is the increase since it.
          value = arg.map((s) => ({ ...s, points: s.points.slice(1).map(([t, v], j) => [t, v >= s.points[j][1] ? v - s.points[j][1] : v]) }));
        else if (token === 'diff') value = arg.map((s) => ({ ...s, points: s.points.slice(1).map(([t, v], j) => [t, v - s.points[j][1]]) }));
        else if (token === 'count_nonzero' || token === 'count_not_null') {
          const counts = new Map<number, number>();
          for (const s of arg) for (const [t, v] of s.points) counts.set(t, (counts.get(t) ?? 0) + (token === 'count_not_null' || v !== 0 ? 1 : 0));
          value = counts.size ? [{ name: '*', points: [...counts].sort((a, b) => a[0] - b[0]) }] : [];
        } else throw new Error(`Formula function ${token} is not supported yet`);
      }
    } else {
      if (!Object.hasOwn(inputs, token)) throw new Error(`Unknown formula query: ${token}`);
      value = inputs[token];
    }
    while (i < tokens.length) {
      const op = tokens[i];
      const precedence = op === '+' || op === '-' ? 1 : op === '*' || op === '/' ? 2 : 0;
      if (!precedence || precedence <= min) break;
      i++;
      const rhs = expr(precedence);
      value = binary(value, rhs, op === '+' ? (a, b) => a + b : op === '-' ? (a, b) => a - b : op === '*' ? (a, b) => a * b : (a, b) => (b === 0 ? NaN : a / b), op);
    }
    return value;
  }
  const result = expr();
  if (i !== tokens.length) throw new Error(`Unsupported formula: ${text}`);
  return result;
}

/** The collector and renderer share exactly the same resolution and modifier validation. */
/** Time aggregations the dashboard endpoint can apply per series (Datadog's `mean` is `avg`). */
const SERVER_AGGREGATORS: Record<string, TimeAggregator> = { avg: 'avg', mean: 'avg', sum: 'sum', min: 'min', max: 'max', last: 'last' };

/**
 * How a scalar panel's query can be reduced on the server, one value per series: its aggregator, or
 * undefined when the panel needs every point (fill() adds points, as_count() scales by the gap
 * between them, before the panel aggregates) or the aggregation is one the server doesn't have.
 */
export function serverAggregator(operations: { name: string }[], aggregator = 'avg'): TimeAggregator | undefined {
  if (operations.some((o) => o.name === 'fill' || o.name === 'as_count')) return undefined;
  return Object.hasOwn(SERVER_AGGREGATORS, aggregator) ? SERVER_AGGREGATORS[aggregator] : undefined;
}

export function preparePanelQuery(query: string, resolve: (query: string) => string, range: { from: number; to: number }) {
  const plan = metricPlan(query);
  // `.rollup(10)`: only the interval, with the default (average) method.
  const found = plan.operations.find((o) => o.name === 'rollup');
  const rollup = found && /^\d/.test(found.args[0]) ? { ...found, args: ['avg', found.args[0]] } : found;
  if (rollup && !['avg', 'max', 'min', 'sum', 'count'].includes(rollup.args[0])) throw new Error(`Rollup ${rollup.args[0]} is not supported yet`);
  // Methods other than avg (the API's default) travel with the query; the interval sets the points.
  const method = rollup && rollup.args[0] !== 'avg' ? `.rollup(${rollup.args[0]})` : '';
  const named: Record<string, number> = { hourly: 3600, daily: 86_400, weekly: 604_800, monthly: 2_592_000 };
  const interval = rollup?.args[1] ? (named[rollup.args[1].replace(/['"]/g, '')] ?? Number(rollup.args[1])) : undefined;
  if (interval !== undefined && (!Number.isFinite(interval) || interval < 1)) throw new Error('Invalid rollup interval');
  const points = interval ? Math.min(1000, Math.max(10, Math.ceil((range.to - range.from) / (interval * 1000)))) : 120;
  const weighted = plan.operations.find((o) => o.name === 'weighted');
  if (weighted && (weighted.args.some(Boolean) || !plan.query.startsWith('sum:'))) throw new Error('weighted() requires sum and no arguments');
  return { plan, text: resolve(plan.query) + method + (weighted ? '.weighted()' : ''), points };
}
