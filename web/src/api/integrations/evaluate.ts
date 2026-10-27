// Execution adapter for the original integrations-core request/formula schema.
// No eval/Function: formulas are parsed using a small, closed arithmetic grammar.
import type { MetricSeries } from '../types';

export interface IntegrationQuery {
  name: string;
  query: string;
  data_source: string;
  aggregator?: string;
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

export function metricPlan(text: string) {
  const m = /^((?:avg|sum|min|max):[\w./-]+\{[^{}]*\}(?:\s+by\s+\{[\w, -]+\})?)(.*)$/.exec(text);
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

/** Seconds each time-comparison function looks back (month_before is 28 days, as in Datadog). */
const LOOK_BACK: Record<string, number> = { hour_before: 3600, day_before: 86_400, week_before: 604_800, month_before: 2_419_200 };

/**
 * Rewrites time comparisons (`week_before(query1)`, `timeshift(query1, -3600)`, …) into inputs read
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
    .replace(/\btimeshift\(\s*(\w+)\s*,\s*-(\d+)\s*\)/g, (_, query: string, seconds: string) => shift(query, Number(seconds)));
  // Report shifts in the order they appear in the formula.
  shifts.sort((a, b) => rewritten.indexOf(a.name) - rewritten.indexOf(b.name));
  return { text: rewritten, shifts };
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
      if (token === 'top') {
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
        else if (token === 'clamp_min' && second) value = binary(arg, second, Math.max);
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
export function preparePanelQuery(query: string, resolve: (query: string) => string, range: { from: number; to: number }) {
  const plan = metricPlan(query);
  const rollup = plan.operations.find((o) => o.name === 'rollup');
  if (rollup && rollup.args[0] !== 'avg') throw new Error(`Rollup ${rollup.args[0]} is not supported yet`);
  const interval = rollup?.args[1] ? Number(rollup.args[1]) : undefined;
  if (interval !== undefined && (!Number.isFinite(interval) || interval < 1)) throw new Error('Invalid rollup interval');
  const points = interval ? Math.min(1000, Math.max(10, Math.ceil((range.to - range.from) / (interval * 1000)))) : 120;
  const weighted = plan.operations.find((o) => o.name === 'weighted');
  if (weighted && (weighted.args.some(Boolean) || !plan.query.startsWith('sum:'))) throw new Error('weighted() requires sum and no arguments');
  return { plan, text: resolve(plan.query) + (weighted ? '.weighted()' : ''), points };
}
