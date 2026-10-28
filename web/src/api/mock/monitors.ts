// In-browser stand-in for the monitor and downtime endpoints (see api/monitors/client.ts), mirroring
// the server (routes/monitors.rs, monitor_engine.rs, monitor_formula.rs, downtimes.rs).
//
// Monitors are seeded from the recommended templates (pages/monitors/templates.ts) and evaluated over
// the mock metric series (metricsEngine.ts): on first use the last two days are replayed every five
// minutes, which gives each group its status and each monitor its history; afterwards every read
// re-evaluates once a minute like the server's evaluator. Formulas between metric queries and log
// monitors (`logs("…") by {…}`, over synthetic log counts) are evaluated too. Creates, updates, mutes,
// resolves and downtimes are kept in memory for the session.
import type { components } from '../generated/schema';
import { parseMetricQuery } from '../../lib/metricQuery';
import { TEMPLATES } from '../../pages/monitors/templates';
import { USERS } from './errorTracking';
import { evaluateGroups, type GroupSeries } from './metricsEngine';

type S = components['schemas'];
type MonitorStatus = S['MonitorStatus'];
type MonitorType = S['MonitorType'];
type MonitorOptions = S['MonitorOptions'];
type MonitorInput = S['MonitorInput'];
type MonitorItem = S['MonitorItem'];
type MonitorDetail = S['MonitorDetail'];
type MonitorEvent = S['MonitorEvent'];
type MemberRef = S['MemberRef'];
type FacetCount = S['FacetCount'];
type TimeRange = S['TimeRange'];
type DowntimeItem = S['DowntimeItem'];
type DowntimeInput = S['DowntimeInput'];
type Recurrence = S['Recurrence'];
type Tags = Record<string, string>;
type Points = [number, number][];

const MIN = 60_000;
const HOUR = 60 * MIN;
const DAY = 24 * HOUR;

const ME: MemberRef = { id: USERS[0].id, name: USERS[0].name };
const member = (i: number): MemberRef => ({ id: USERS[i].id, name: USERS[i].name });
/** Notification channels configured in this mock org (`@slack-…`, `@webhook-…`). */
const CHANNELS = ['slack-platform-oncall', 'slack-checkout-alerts', 'slack-catalog', 'slack-storefront', 'slack-billing-alerts', 'webhook-oncall'];

const bad = (message: string) => new Error(message);

function hash(s: string): number {
  let h = 2166136261;
  for (let i = 0; i < s.length; i++) h = Math.imul(h ^ s.charCodeAt(i), 16777619);
  return h >>> 0;
}

// ---------------------------------------------------------------- statuses and groups

const SEVERITY: Record<MonitorStatus, number> = { alert: 3, warn: 2, nodata: 1, ok: 0 };

function worst(statuses: MonitorStatus[]): MonitorStatus | null {
  return statuses.reduce<MonitorStatus | null>((w, s) => (w === null || SEVERITY[s] > SEVERITY[w] ? s : w), null);
}

/** `host:a,service:b` (keys sorted), or `*` for an ungrouped query. */
function groupKey(tags: Tags): string {
  const keys = Object.keys(tags).sort();
  return keys.length ? keys.map((k) => `${k}:${tags[k]}`).join(',') : '*';
}

function parseGroupKey(key: string): Tags {
  if (key === '*') return {};
  const out: Tags = {};
  for (const part of key.split(',')) {
    const i = part.indexOf(':');
    if (i > 0) out[part.slice(0, i)] = part.slice(i + 1);
  }
  return out;
}

// ---------------------------------------------------------------- options (monitor_engine.rs)

/** The options with every field present, in a fixed order (so definitions compare). */
function normOptions(o: MonitorOptions): MonitorOptions {
  return {
    detection: o.detection,
    timeAggregation: o.timeAggregation,
    windowMinutes: o.windowMinutes,
    changeMode: o.changeMode ?? null,
    changeMinutes: o.changeMinutes ?? null,
    comparator: o.comparator,
    thresholds: {
      critical: o.thresholds.critical,
      warning: o.thresholds.warning ?? null,
      criticalRecovery: o.thresholds.criticalRecovery ?? null,
      warningRecovery: o.thresholds.warningRecovery ?? null,
    },
    notifyNoData: o.notifyNoData ?? false,
    noDataMinutes: o.noDataMinutes ?? 10,
  };
}

const upward = (o: MonitorOptions) => o.comparator === 'above' || o.comparator === 'above_or_equal';

function crosses(o: MonitorOptions, value: number, threshold: number): boolean {
  switch (o.comparator) {
    case 'above':
      return value > threshold;
    case 'above_or_equal':
      return value >= threshold;
    case 'below':
      return value < threshold;
    default:
      return value <= threshold;
  }
}

function validateOptions(o: MonitorOptions): void {
  if (!(o.windowMinutes >= 1 && o.windowMinutes <= 1440)) throw bad('the evaluation window must be between 1 minute and 1 day');
  if (o.detection === 'change' && !(o.changeMinutes != null && o.changeMinutes >= 1 && o.changeMinutes <= 10_080)) throw bad('change monitors need a comparison period between 1 minute and 1 week');
  const noData = o.noDataMinutes ?? 10;
  if (!(noData >= 1 && noData <= 1440)) throw bad('the no-data timeframe must be between 1 minute and 1 day');
  const t = o.thresholds;
  if ([t.critical, t.warning, t.criticalRecovery, t.warningRecovery].some((v) => v != null && !Number.isFinite(v))) throw bad('thresholds must be numbers');
  const up = upward(o);
  const before = (a: number, b: number) => (up ? a < b : a > b);
  const side = up ? 'below' : 'above';
  if (t.warning != null && !before(t.warning, t.critical)) throw bad(`the warning threshold must be ${side} the alert threshold`);
  if (t.criticalRecovery != null && !before(t.criticalRecovery, t.critical)) throw bad(`the alert recovery threshold must be ${side} the alert threshold`);
  if (t.warningRecovery != null && t.warning != null && !before(t.warningRecovery, t.warning)) throw bad(`the warning recovery threshold must be ${side} the warning threshold`);
  if (t.warningRecovery != null && t.warning == null) throw bad('a warning recovery threshold needs a warning threshold');
}

const lookbackMs = (o: MonitorOptions) => (o.windowMinutes + (o.detection === 'change' ? (o.changeMinutes ?? 0) : 0)) * MIN;

const minutesLabel = (m: number) => (m % 1440 === 0 ? `${m / 1440}d` : m % 60 === 0 ? `${m / 60}h` : `${m}m`);

function fmtNum(v: number): string {
  if (Number.isInteger(v) && Math.abs(v) < 1e15) return String(v);
  return v.toFixed(4).replace(/0+$/, '').replace(/\.$/, '');
}

const SYMBOL = { above: '>', above_or_equal: '>=', below: '<', below_or_equal: '<=' } as const;

/** Datadog-style full query: `avg(last_5m):avg:system.load.1{*} by {host} > 2`. */
function fullQuery(o: MonitorOptions, metricQuery: string): string {
  const reducer = `${o.timeAggregation}(last_${minutesLabel(o.windowMinutes)})`;
  const head = o.detection === 'change' ? `${o.changeMode === 'pct_change' ? 'pct_change' : 'change'}(${reducer},last_${minutesLabel(o.changeMinutes ?? 60)})` : reducer;
  return `${head}:${metricQuery.trim()} ${SYMBOL[o.comparator]} ${fmtNum(o.thresholds.critical)}`;
}

/** Next status for one group given its evaluated value (null = no data in the window). */
function classify(o: MonitorOptions, value: number | null, prev: MonitorStatus): MonitorStatus {
  if (value === null) return o.notifyNoData ? 'nodata' : prev;
  const t = o.thresholds;
  if (crosses(o, value, t.critical)) return 'alert';
  if (prev === 'alert' && t.criticalRecovery != null && crosses(o, value, t.criticalRecovery)) return 'alert';
  if (t.warning != null) {
    if (crosses(o, value, t.warning)) return 'warn';
    if (prev === 'warn' && t.warningRecovery != null && crosses(o, value, t.warningRecovery)) return 'warn';
  }
  return 'ok';
}

/** Index of the first point after `t` (points are sorted by time). */
function after(points: Points, t: number): number {
  let lo = 0;
  let hi = points.length;
  while (lo < hi) {
    const mid = (lo + hi) >> 1;
    if (points[mid][0] <= t) lo = mid + 1;
    else hi = mid;
  }
  return lo;
}

/** Points in `(from, to]` reduced with the time aggregator. */
function reduceWindow(points: Points, from: number, to: number, agg: MonitorOptions['timeAggregation']): number | null {
  const vs = points.slice(after(points, from), after(points, to)).map((p) => p[1]);
  if (vs.length === 0) return null;
  switch (agg) {
    case 'last':
      return vs[vs.length - 1];
    case 'sum':
      return vs.reduce((a, b) => a + b, 0);
    case 'min':
      return Math.min(...vs);
    case 'max':
      return Math.max(...vs);
    default:
      return vs.reduce((a, b) => a + b, 0) / vs.length;
  }
}

function valueAt(o: MonitorOptions, points: Points, at: number): number | null {
  const w = o.windowMinutes * MIN;
  const now = reduceWindow(points, at - w, at, o.timeAggregation);
  if (now === null || o.detection !== 'change') return now;
  const shift = (o.changeMinutes ?? 60) * MIN;
  const before = reduceWindow(points, at - shift - w, at - shift, o.timeAggregation);
  if (before === null) return null;
  if (o.changeMode === 'pct_change') return before === 0 ? null : ((now - before) / Math.abs(before)) * 100;
  return now - before;
}

// ---------------------------------------------------------------- formulas (monitor_formula.rs)

type Expr = { q: number } | { n: number } | { zero: Expr } | { op: '+' | '-' | '*' | '/'; a: Expr; b: Expr };

const QUERY_AT = /(?:avg|sum|min|max):[\w.]+\{[^}]*\}(?:\s+by\s+\{[^}]*\})?/y;
const NUMBER_AT = /[\d.]+/y;

/** One metric query, or arithmetic between metric queries and numbers (`default_zero(…)` included). */
function parseFormula(text: string): { expr: Expr; queries: string[] } {
  const s = text.trim();
  const queries: string[] = [];
  let pos = 0;
  const ws = () => {
    while (pos < s.length && /\s/.test(s[pos])) pos++;
  };
  const eat = (c: string) => {
    ws();
    if (s[pos] !== c) return false;
    pos++;
    return true;
  };
  const expr = (): Expr => {
    let left = term();
    for (;;) {
      if (eat('+')) left = { op: '+', a: left, b: term() };
      else if (eat('-')) left = { op: '-', a: left, b: term() };
      else return left;
    }
  };
  const term = (): Expr => {
    let left = factor();
    for (;;) {
      if (eat('*')) left = { op: '*', a: left, b: factor() };
      else if (eat('/')) left = { op: '/', a: left, b: factor() };
      else return left;
    }
  };
  const factor = (): Expr => {
    ws();
    if (eat('(')) {
      const e = expr();
      if (!eat(')')) throw bad("missing ')'");
      return e;
    }
    if (s.startsWith('default_zero(', pos)) {
      pos += 'default_zero('.length;
      const e = expr();
      if (!eat(')')) throw bad("missing ')' after default_zero(");
      return { zero: e };
    }
    NUMBER_AT.lastIndex = pos;
    const n = NUMBER_AT.exec(s);
    if (n) {
      pos += n[0].length;
      const v = Number(n[0]);
      if (Number.isNaN(v)) throw bad(`invalid number '${n[0]}'`);
      return { n: v };
    }
    if (pos >= s.length) throw bad('expected a metric query or a number at the end');
    QUERY_AT.lastIndex = pos;
    const q = QUERY_AT.exec(s);
    if (!q || !parseMetricQuery(q[0])) throw bad('expected <agg>:<metric>{…}');
    pos += q[0].length;
    queries.push(q[0].trim());
    return { q: queries.length - 1 };
  };
  const e = expr();
  ws();
  if (pos < s.length) throw bad(`unexpected '${s.slice(pos)}' at ${pos + 1}`);
  return { expr: e, queries };
}

function evalExpr(e: Expr, value: (i: number) => number | undefined): number | undefined {
  if ('q' in e) return value(e.q);
  if ('n' in e) return e.n;
  if ('zero' in e) {
    const v = evalExpr(e.zero, (i) => value(i) ?? 0);
    return v !== undefined && Number.isFinite(v) ? v : 0;
  }
  const a = evalExpr(e.a, value);
  const b = evalExpr(e.b, value);
  if (a === undefined || b === undefined) return undefined;
  switch (e.op) {
    case '+':
      return a + b;
    case '-':
      return a - b;
    case '*':
      return a * b;
    default:
      return b !== 0 ? a / b : undefined;
  }
}

/** The formula's series: groups joined by tags (a query without `by` applies to every group) and time. */
function combine(expr: Expr, results: GroupSeries[][]): GroupSeries[] {
  const lookup = results.map((groups) => new Map(groups.map((g) => [groupKey(g.tags), new Map(g.points)])));
  const keyed = new Map<string, Tags>();
  for (const g of results.flat()) if (Object.keys(g.tags).length) keyed.set(groupKey(g.tags), g.tags);
  const keys = keyed.size
    ? [...keyed.entries()].sort((a, b) => (a[0] < b[0] ? -1 : 1))
    : results
        .flat()
        .slice(0, 1)
        .map((g) => [groupKey(g.tags), g.tags] as [string, Tags]);
  const times = [...new Set(results.flat().flatMap((g) => g.points.map((p) => p[0])))].sort((a, b) => a - b);
  const out: GroupSeries[] = [];
  for (const [key, tags] of keys) {
    const points: Points = [];
    for (const t of times) {
      const v = evalExpr(expr, (i) => (lookup[i].get(key) ?? lookup[i].get('*'))?.get(t));
      if (v !== undefined && Number.isFinite(v)) points.push([t, Math.round(v * 1e4) / 1e4]);
    }
    if (points.length) out.push({ tags, points });
  }
  return out;
}

// ---------------------------------------------------------------- log monitors

/** `logs("<search>") by {a,b}` → parts (the server's parse_log_monitor). */
function parseLogQuery(q: string): { search: string; by: string[] } {
  const m = /^\s*logs\("((?:[^"\\]|\\.)*)"\)\s*(?:by\s*\{([^}]*)\})?\s*$/.exec(q);
  if (!m) throw bad('log monitor queries look like logs("<search>") by {<tags>}');
  const by = (m[2] ?? '')
    .split(',')
    .map((s) => s.trim())
    .filter(Boolean);
  if (by.length > 4) throw bad('group by at most 4 tags or @attributes');
  return { search: m[1].replace(/\\(.)/g, '$1'), by };
}

const LOG_SERVICES: [service: string, host: string, errorsPerMin: number][] = [
  ['api-gateway', 'i-0a1b2c3d4e5f60001', 9],
  ['storefront-api', 'i-0a1b2c3d4e5f60002', 6],
  ['order-service', 'i-0a1b2c3d4e5f60001', 180],
  ['billing', 'i-0a1b2c3d4e5f60002', 4],
  ['inventory-service', 'i-0a1b2c3d4e5f60003', 12],
  ['catalog-service', 'i-0a1b2c3d4e5f60004', 35],
  ['notification-worker', 'i-0a1b2c3d4e5f60005', 7],
];
const LOG_SOURCES: { tags: Tags; perMin: number }[] = LOG_SERVICES.flatMap(([service, host, errors]) =>
  (
    [
      ['error', errors],
      ['warn', errors * 3 + 20],
      ['info', 600 + (hash(service) % 900)],
    ] as const
  ).map(([status, perMin]) => ({ tags: { service, host, env: 'prod', status }, perMin })),
);

function wobble(seed: number, t: number): number {
  const knot = (i: number) => {
    const x = Math.sin(seed * 0.0001 + i * 12.9898) * 43758.5453;
    return x - Math.floor(x) - 0.5;
  };
  const p = t / (5 * MIN);
  const i = Math.floor(p);
  const u = (p - i) * (p - i) * (3 - 2 * (p - i));
  return knot(i) * (1 - u) + knot(i + 1) * u;
}

/** Matching logs per interval and group: `key:value` terms filter on tags, free text matches a share. */
function logSeries(query: string, range: TimeRange, step: number): GroupSeries[] {
  const { search, by } = parseLogQuery(query);
  let share = 1;
  const terms: { key: string; value: string; negate: boolean }[] = [];
  for (const word of search.split(/\s+/).filter(Boolean)) {
    const negate = word.startsWith('-');
    const body = negate ? word.slice(1) : word;
    const i = body.indexOf(':');
    if (i > 0) terms.push({ key: body.slice(0, i), value: body.slice(i + 1).toLowerCase(), negate });
    else if (body !== '*') share *= 0.2;
  }
  const sources = LOG_SOURCES.filter((s) => terms.every((t) => (s.tags[t.key]?.toLowerCase() === t.value) !== t.negate));
  const groups = new Map<string, { tags: Tags; members: typeof sources }>();
  for (const s of sources) {
    const tags = Object.fromEntries(by.map((k) => [k, s.tags[k.replace(/^@/, '')] ?? 'N/A']));
    const key = groupKey(tags);
    const g = groups.get(key) ?? { tags, members: [] };
    g.members.push(s);
    groups.set(key, g);
  }
  const start = Math.floor(range.from / step) * step;
  return [...groups.values()].map((g) => {
    const points: Points = [];
    for (let t = start; t <= range.to; t += step)
      points.push([t, Math.round(g.members.reduce((n, s) => n + s.perMin * share * (step / MIN) * Math.max(0, 1 + 0.8 * wobble(hash(groupKey(s.tags)), t)), 0))]);
    return { tags: g.tags, points };
  });
}

// ---------------------------------------------------------------- series and backtests

/**
 * Resolution of a monitor's series: one point per evaluation window at most (5 minutes at most), so
 * every evaluation sees data. The server lets ClickHouse pick the rollup; the mock picks it here.
 */
function resolutionMs(type: MonitorType, o: MonitorOptions): number {
  return type === 'log' ? MIN : Math.min(o.windowMinutes, 5) * MIN;
}

function monitorSeries(type: MonitorType, query: string, o: MonitorOptions, range: TimeRange, points?: number): GroupSeries[] {
  const res = resolutionMs(type, o);
  if (type === 'log') return logSeries(query, range, Math.max(res, points ? Math.floor((range.to - range.from) / points) : res));
  let f: ReturnType<typeof parseFormula>;
  try {
    f = parseFormula(query);
  } catch (e) {
    throw bad(`invalid query: ${(e as Error).message}`);
  }
  const n = Math.min(5000, Math.max(points ?? 0, Math.ceil((range.to - range.from) / res)));
  const results = f.queries.map((q) => evaluateGroups(q, range, n));
  if ('q' in f.expr && f.expr.q === 0) return results[0];
  return combine(f.expr, results);
}

interface Transition {
  at: number;
  group: string;
  from: MonitorStatus;
  to: MonitorStatus;
  value: number | null;
}

/** Replays a monitor over `[from, to]` every `step` like the live evaluator; every group starts OK. */
function backtest(o: MonitorOptions, groups: GroupSeries[], from: number, to: number, step: number) {
  step = Math.max(step, 10_000);
  const transitions: Transition[] = [];
  const evaluated: [string, [number, number | null][]][] = [];
  const first = Math.min(from + o.windowMinutes * MIN, to);
  for (const g of groups) {
    const key = groupKey(g.tags);
    let status: MonitorStatus = 'ok';
    const values: [number, number | null][] = [];
    const born = g.points[0]?.[0] ?? Infinity;
    for (let at = first - ((first - from) % step); at <= to; at += step) {
      if (at < born) continue;
      const value = valueAt(o, g.points, at);
      const next = classify(o, value, status);
      if (next !== status) transitions.push({ at, group: key, from: status, to: next, value });
      status = next;
      values.push([at, value]);
    }
    evaluated.push([key, values]);
  }
  transitions.sort((a, b) => a.at - b.at);
  return { transitions, evaluated };
}

// ---------------------------------------------------------------- messages and recipients

/** `@handles` in a message: emails, `@slack-…`, `@webhook-…`, user handles. */
function recipients(message: string): string[] {
  const out: string[] = [];
  const re = /(^|[^\p{L}\p{N}._+-])@([\p{L}\p{N}._+@-]+)/gu;
  for (const m of message.matchAll(re)) {
    const handle = m[2].replace(/[.\-@]+$/, '');
    if (handle && !out.includes(handle)) out.push(handle);
  }
  return out;
}

const FLAGS = ['is_alert', 'is_warning', 'is_no_data', 'is_recovery', 'is_alert_recovery', 'is_warning_recovery'];

/** `{{#is_alert}}…{{/is_alert}}` blocks and `{{value}}`, `{{threshold}}`, `{{<tag>.name}}` variables. */
function renderMessage(template: string, status: MonitorStatus, previous: MonitorStatus, value: number | null, o: MonitorOptions, name: string, group: Tags): string {
  const recovery = status === 'ok' && previous !== 'ok';
  const flag: Record<string, boolean> = {
    is_alert: status === 'alert',
    is_warning: status === 'warn',
    is_no_data: status === 'nodata',
    is_recovery: recovery,
    is_alert_recovery: recovery && previous === 'alert',
    is_warning_recovery: recovery && previous === 'warn',
  };
  const shown: boolean[] = [];
  const visible = () => shown.every(Boolean);
  let out = '';
  let rest = template;
  for (let open = rest.indexOf('{{'); open >= 0; open = rest.indexOf('{{')) {
    if (visible()) out += rest.slice(0, open);
    const close = rest.indexOf('}}', open);
    if (close < 0) {
      rest = rest.slice(open);
      break;
    }
    const tag = rest.slice(open + 2, close).trim();
    const raw = rest.slice(open, close + 2);
    rest = rest.slice(close + 2);
    if (tag.startsWith('#') || tag.startsWith('^')) {
      const f = tag.slice(1).trim();
      if (FLAGS.includes(f)) shown.push(tag.startsWith('#') ? flag[f] : !flag[f]);
      else if (visible()) out += raw;
      continue;
    }
    if (tag.startsWith('/')) {
      if (FLAGS.includes(tag.slice(1).trim())) shown.pop();
      else if (visible()) out += raw;
      continue;
    }
    if (!visible()) continue;
    const v =
      tag === 'value'
        ? value === null
          ? undefined
          : fmtNum(value)
        : tag === 'threshold'
          ? fmtNum(o.thresholds.critical)
          : tag === 'warn_threshold'
            ? o.thresholds.warning == null
              ? undefined
              : fmtNum(o.thresholds.warning)
            : tag === 'monitor.name'
              ? name
              : tag.endsWith('.name')
                ? group[tag.slice(0, -'.name'.length)]
                : undefined;
    out += v ?? raw;
  }
  if (visible()) out += rest;
  return out;
}

// ---------------------------------------------------------------- downtimes (downtimes.rs)

interface Downtime {
  id: number;
  monitorId: number | null;
  monitorTags: string[];
  scope: string[];
  message: string;
  start: number;
  end: number | null;
  recurrence: Recurrence | null;
  recurUntil: number | null;
  creator: MemberRef | null;
  createdAt: number;
  canceledAt: number | null;
}

const periodOf = (d: Downtime) => (d.recurrence === 'daily' ? DAY : d.recurrence === 'weekly' ? 7 * DAY : null);

/** The occurrence active at `now` as [start, end]: undefined when the downtime isn't active. */
function activeAt(d: Downtime, now: number): [number, number | null] | undefined {
  if ((d.canceledAt !== null && d.canceledAt <= now) || now < d.start) return undefined;
  const period = periodOf(d);
  if (d.end === null || period === null) return d.end !== null && now >= d.end ? undefined : [d.start, d.end];
  const k = Math.floor((now - d.start) / period);
  const [s, e] = [d.start + k * period, d.end + k * period];
  if ((d.recurUntil !== null && s > d.recurUntil) || now >= e) return undefined;
  return [s, e];
}

function finishedAt(d: Downtime, now: number): boolean {
  if (d.canceledAt !== null && d.canceledAt <= now) return true;
  const period = periodOf(d);
  if (d.end === null) return false;
  if (period === null) return now >= d.end;
  if (d.recurUntil === null) return false;
  return now >= d.end + Math.floor((d.recurUntil - d.start) / period) * period;
}

const appliesTo = (d: Downtime, m: Row) => (d.monitorId !== null ? d.monitorId === m.id : d.monitorTags.every((t) => m.tags.includes(t)));

/** Whether a scope (`[]`/`['*']` = everything, else every `key:value` must match) covers a group. */
const scopeCovers = (scope: string[], group: Tags) =>
  scope
    .filter((s) => s !== '*')
    .every((s) => {
      const i = s.indexOf(':');
      return i > 0 && group[s.slice(0, i)] === s.slice(i + 1);
    });

/** Latest end among the active downtimes applying to the monitor and passing `covers`: undefined = not muted, null = no end. */
function mutedBy(m: Row, now: number, covers: (d: Downtime) => boolean): number | null | undefined {
  let out: number | null | undefined;
  for (const d of db().downtimes) {
    if (d.canceledAt !== null || !appliesTo(d, m) || !covers(d)) continue;
    const a = activeAt(d, now);
    if (!a) continue;
    out = out === null || a[1] === null ? null : out === undefined ? a[1] : Math.max(out, a[1]);
  }
  return out;
}

const monitorMuted = (m: Row, now: number) => mutedBy(m, now, (d) => d.scope.every((s) => s === '*'));
const groupMuted = (m: Row, group: string, now: number) => mutedBy(m, now, (d) => scopeCovers(d.scope, parseGroupKey(group)));

function downtimeItem(d: Downtime, now: number): DowntimeItem {
  const active = activeAt(d, now);
  const monitor = d.monitorId === null ? undefined : db().monitors.find((m) => m.id === d.monitorId);
  return {
    id: d.id,
    status: d.canceledAt !== null ? 'canceled' : active ? 'active' : finishedAt(d, now) ? 'ended' : 'scheduled',
    monitorId: d.monitorId,
    monitorName: monitor?.name ?? null,
    monitorTags: d.monitorTags,
    scope: d.scope.length ? d.scope : ['*'],
    message: d.message,
    start: d.start,
    end: d.end,
    recurrence: d.recurrence,
    recurUntil: d.recurUntil,
    activeUntil: active?.[1] ?? null,
    creator: d.creator,
    createdAt: d.createdAt,
  };
}

// ---------------------------------------------------------------- monitor state

interface GroupRow {
  group: string;
  status: MonitorStatus;
  value: number | null;
  lastDataAt: number | null;
  stateChangedAt: number;
}

interface Row {
  id: number;
  name: string;
  type: MonitorType;
  query: string;
  options: MonitorOptions;
  message: string;
  priority: number | null;
  tags: string[];
  teams: string[];
  draft: boolean;
  creator: MemberRef | null;
  createdAt: number;
  modifiedAt: number;
  status: MonitorStatus;
  stateChangedAt: number;
  evaluatedAt: number | null;
  groups: GroupRow[];
  /** Oldest first. */
  events: MonitorEvent[];
}

interface State {
  monitors: Row[];
  downtimes: Downtime[];
  nextId: number;
  nextDowntimeId: number;
  nextEventId: number;
  /** Last time every monitor was evaluated. */
  evaluatedAt: number;
}

let state: State | null = null;

/** How far back the first evaluation replays, and how often. */
const REPLAY_MS = 2 * DAY;
const REPLAY_STEP = 5 * MIN;

function addEvent(m: Row, group: string, from: MonitorStatus, to: MonitorStatus, value: number | null, at: number) {
  const s = db();
  const muted = m.draft || groupMuted(m, group, at) !== undefined;
  const message = renderMessage(m.message, to, from, value, m.options, m.name, parseGroupKey(group));
  m.events.push({ id: s.nextEventId++, group, fromStatus: from, toStatus: to, value, muted, recipients: muted ? [] : recipients(message), message, createdAt: at });
}

function setOverall(m: Row, at: number) {
  const next = worst(m.groups.map((g) => g.status)) ?? 'nodata';
  if (next !== m.status) {
    m.status = next;
    m.stateChangedAt = at;
  }
}

/** One evaluation at `now`, as the server's evaluator does it (monitors.rs evaluate_inner). */
function evaluate(m: Row, now: number, series?: GroupSeries[]) {
  const o = m.options;
  if (!series) {
    try {
      series = monitorSeries(m.type, m.query, o, { from: now - lookbackMs(o) - MIN, to: now });
    } catch {
      series = []; // an unparsable query is no data, not an error
    }
  }
  const groups = new Map(m.groups.map((g) => [g.group, g]));
  const seen = new Set<string>();
  for (const g of series) {
    const key = groupKey(g.tags);
    seen.add(key);
    const value = valueAt(o, g.points, now);
    // Series may run past `now` while replaying: the last point at or before it.
    const last = g.points[after(g.points, now) - 1]?.[0] ?? null;
    if (last !== null) step(m, groups, key, value, last, now);
  }
  for (const g of m.groups) if (!seen.has(g.group)) step(m, groups, g.group, null, null, now);
  m.groups = [...groups.values()].sort((a, b) => (a.group < b.group ? -1 : 1));
  setOverall(m, now);
  m.evaluatedAt = now;
}

function step(m: Row, groups: Map<string, GroupRow>, key: string, value: number | null, lastPoint: number | null, now: number) {
  const o = m.options;
  let g = groups.get(key);
  const prev = g?.status ?? 'ok';
  const lastData = lastPoint ?? g?.lastDataAt ?? null;
  // Missing data only counts once it has been missing for the no-data timeframe.
  const fresh = lastData !== null && now - lastData < (o.noDataMinutes ?? 10) * MIN;
  const next = value === null && fresh ? prev : classify(o, value, prev);
  if (!g) {
    g = { group: key, status: next, value, lastDataAt: lastData, stateChangedAt: now };
    groups.set(key, g);
    if (next !== 'ok') addEvent(m, key, prev, next, value, now);
    return;
  }
  if (next !== prev) {
    addEvent(m, key, prev, next, value, now);
    g.stateChangedAt = now;
  }
  g.status = next;
  if (value !== null) g.value = value;
  g.lastDataAt = lastData;
}

/** First evaluation of a seeded monitor: replay the last two days so it has a status and a history. */
function replay(m: Row, now: number) {
  const o = m.options;
  const from = Math.max(now - REPLAY_MS, m.createdAt);
  let series: GroupSeries[];
  try {
    series = monitorSeries(m.type, m.query, o, { from: from - lookbackMs(o) - REPLAY_STEP, to: now });
  } catch {
    series = [];
  }
  const times: number[] = [];
  for (let t = Math.ceil(from / REPLAY_STEP) * REPLAY_STEP; t < now; t += REPLAY_STEP) times.push(t);
  times.push(now);
  // Before the replay, each group has been in its first status for a while (since some time after creation).
  for (const g of series) {
    const key = groupKey(g.tags);
    const value = valueAt(o, g.points, times[0]);
    const since = m.createdAt + Math.floor((from - m.createdAt) * ((hash(`${m.name}|${key}`) % 1000) / 1000));
    m.groups.push({ group: key, status: classify(o, value, 'ok'), value, lastDataAt: times[0], stateChangedAt: since });
  }
  m.groups.sort((a, b) => (a.group < b.group ? -1 : 1));
  m.status = worst(m.groups.map((g) => g.status)) ?? 'nodata';
  m.stateChangedAt = m.groups.filter((g) => g.status === m.status).reduce((t, g) => Math.min(t, g.stateChangedAt), m.groups.length ? Infinity : m.createdAt);
  for (const t of times.slice(1)) evaluate(m, t, series);
}

interface Seed {
  template?: string;
  name?: string;
  type?: MonitorType;
  query?: string;
  options?: Partial<MonitorOptions>;
  thresholds?: Partial<MonitorOptions['thresholds']>;
  message: string;
  priority: number | null;
  tags: string[];
  teams: string[];
  draft?: boolean;
  creator: number;
  ageDays: number;
}

const BASE: MonitorOptions = normOptions({ detection: 'threshold', timeAggregation: 'avg', windowMinutes: 5, comparator: 'above', thresholds: { critical: 0 } });

const SEEDS: Seed[] = [
  {
    template: 'cpu-high',
    options: { windowMinutes: 10 },
    thresholds: { critical: 95, warning: 90, criticalRecovery: 80, warningRecovery: 75 },
    message: '@slack-platform-oncall',
    priority: 2,
    tags: ['env:prod', 'team:platform'],
    teams: ['platform'],
    creator: 0,
    ageDays: 140,
  },
  {
    template: 'mem-high',
    thresholds: { criticalRecovery: 85, warningRecovery: 75 },
    message: '@slack-platform-oncall',
    priority: 3,
    tags: ['env:prod', 'team:platform'],
    teams: ['platform'],
    creator: 0,
    ageDays: 120,
  },
  { template: 'disk-low', message: '@webhook-oncall @slack-platform-oncall', priority: 2, tags: ['env:prod', 'team:platform'], teams: ['platform'], creator: 2, ageDays: 95 },
  // No @recipients: shows up in Monitor Quality.
  {
    template: 'load-high',
    thresholds: { critical: 11, warning: 9, criticalRecovery: 8, warningRecovery: 6.5 },
    message: '',
    priority: 4,
    tags: ['env:prod'],
    teams: ['platform'],
    creator: 2,
    ageDays: 33,
  },
  {
    template: 'net-in',
    thresholds: { criticalRecovery: 60_000_000, warningRecovery: 30_000_000 },
    message: '@slack-platform-oncall',
    priority: null,
    tags: ['env:prod'],
    teams: ['platform'],
    draft: true,
    creator: 4,
    ageDays: 6,
  },
  { template: 'apm-errors', message: '@slack-checkout-alerts @priya.shah@example.com', priority: 1, tags: ['env:prod', 'service:order-service'], teams: ['checkout'], creator: 1, ageDays: 80 },
  { template: 'apm-latency', message: '@slack-catalog', priority: 2, tags: ['env:prod', 'service:catalog-service'], teams: ['catalog'], creator: 3, ageDays: 64 },
  { template: 'apm-traffic-drop', message: '@slack-storefront', priority: 3, tags: ['env:prod', 'service:storefront-api'], teams: ['storefront'], creator: 1, ageDays: 41 },
  { template: 'apm-no-traffic', message: '@webhook-oncall', priority: 1, tags: ['env:prod', 'service:api-gateway'], teams: ['platform'], creator: 0, ageDays: 150 },
  {
    template: 'kubernetes-kubernetes-deployment-replicas-are-failing',
    message: '@slack-platform-oncall',
    priority: 3,
    tags: ['env:prod', 'integration:kubernetes'],
    teams: ['platform'],
    creator: 4,
    ageDays: 22,
  },
  { template: 'kubernetes-nodes-are-unavailable', message: '@slack-platform-oncall', priority: 2, tags: ['integration:kubernetes'], teams: ['platform'], creator: 4, ageDays: 22 },
  {
    name: 'Order service is logging errors',
    type: 'log',
    query: 'logs("service:order-service status:error") by {service}',
    options: { timeAggregation: 'sum' },
    thresholds: { critical: 600, warning: 400, criticalRecovery: 500, warningRecovery: 300 },
    message: '{{#is_alert}}{{service.name}} logged {{value}} errors in 5 minutes. @slack-checkout-alerts{{/is_alert}}{{#is_recovery}}Errors are back to normal. @slack-checkout-alerts{{/is_recovery}}',
    priority: 2,
    tags: ['env:prod', 'service:order-service'],
    teams: ['checkout'],
    creator: 1,
    ageDays: 52,
  },
  {
    name: 'Billing is logging errors',
    type: 'log',
    query: 'logs("service:billing status:error") by {host}',
    options: { timeAggregation: 'sum' },
    thresholds: { critical: 150, warning: 80 },
    message: '{{host.name}} logged {{value}} billing errors. @slack-billing-alerts',
    priority: 2,
    tags: ['env:prod', 'service:billing'],
    teams: ['billing'],
    creator: 3,
    ageDays: 75,
  },
  {
    name: 'Billing hosts are running hot',
    query: 'avg:system.cpu.user{team:billing} by {host}',
    options: { windowMinutes: 15 },
    thresholds: { critical: 85, warning: 70, criticalRecovery: 65, warningRecovery: 55 },
    message: 'CPU at {{value}}% on {{host.name}}. @slack-billing-alerts @sara.levi@example.com',
    priority: 3,
    tags: ['env:prod', 'service:billing'],
    teams: ['billing'],
    creator: 3,
    ageDays: 75,
  },
  {
    // A metric nobody sends any more: stuck in No Data.
    name: 'Billing invoice queue is backing up',
    query: 'max:billing.invoice_queue.depth{env:prod}',
    options: { timeAggregation: 'max', notifyNoData: true, noDataMinutes: 30 },
    thresholds: { critical: 500, warning: 200 },
    message: 'The invoice queue holds {{value}} jobs. @slack-billing-alerts',
    priority: 2,
    tags: ['env:prod', 'service:billing'],
    teams: ['billing'],
    creator: 3,
    ageDays: 48,
  },
  {
    name: 'Storefront API p95 latency above SLO',
    type: 'apm',
    query: 'avg:trace.latency.p95{env:prod,service:storefront-api}',
    options: { windowMinutes: 10 },
    thresholds: { critical: 400, warning: 300 },
    message: 'p95 latency is {{value}} ms. @slack-storefront',
    priority: 2,
    tags: ['env:prod', 'service:storefront-api'],
    teams: ['storefront'],
    creator: 0,
    ageDays: 110,
  },
  {
    // Muted for weeks by a downtime (below): shows up in Monitor Quality.
    name: '[Legacy] Inventory batch hosts are almost full',
    query: 'max:system.disk.in_use{env:qa} by {host}',
    options: { timeAggregation: 'max', windowMinutes: 15 },
    thresholds: { critical: 92 },
    message: '{{host.name}} disk is {{value}}% full. @omar.haddad@example.com',
    priority: 5,
    tags: ['env:qa', 'service:inventory-service'],
    teams: ['checkout'],
    creator: 4,
    ageDays: 200,
  },
  {
    name: 'Inventory service error rate',
    type: 'apm',
    query: 'avg:trace.error_rate{env:prod,service:inventory-service}',
    thresholds: { critical: 5, warning: 3 },
    message: '{{#is_alert}}Inventory is failing {{value}}% of requests. @slack-checkout-alerts{{/is_alert}}',
    priority: 2,
    tags: ['env:prod', 'service:inventory-service'],
    teams: ['checkout'],
    creator: 1,
    ageDays: 66,
  },
];

function seedRow(seed: Seed, id: number, now: number): Row {
  const t = seed.template ? TEMPLATES.find((x) => x.id === seed.template) : undefined;
  if (seed.template && !t) throw bad(`unknown monitor template ${seed.template}`);
  const base = t ? normOptions(t.options) : BASE;
  const options = normOptions({ ...base, ...seed.options, thresholds: { ...base.thresholds, ...seed.thresholds } });
  const createdAt = now - seed.ageDays * DAY - (hash(String(id)) % 600) * MIN;
  const message = t ? `${t.message}${seed.message ? `\n\n${seed.message}` : ''}` : seed.message;
  return {
    id,
    name: seed.name ?? t!.name,
    type: seed.type ?? t?.type ?? 'metric',
    query: seed.query ?? t!.query,
    options,
    message,
    priority: seed.priority,
    tags: seed.tags,
    teams: seed.teams,
    draft: seed.draft ?? false,
    creator: member(seed.creator),
    createdAt,
    modifiedAt: createdAt + Math.min(seed.ageDays, 9) * DAY,
    status: 'nodata',
    stateChangedAt: createdAt,
    evaluatedAt: null,
    groups: [],
    events: [],
  };
}

function seedDowntimes(now: number, monitors: Row[]): Downtime[] {
  const legacy = monitors.find((m) => m.name.startsWith('[Legacy]'))!;
  const dayStart = Math.floor(now / DAY) * DAY;
  const dt = (d: Partial<Downtime> & Pick<Downtime, 'id' | 'start' | 'message'>): Downtime => ({
    monitorId: null,
    monitorTags: [],
    scope: [],
    end: null,
    recurrence: null,
    recurUntil: null,
    creator: ME,
    createdAt: d.start - HOUR,
    canceledAt: null,
    ...d,
  });
  return [
    dt({ id: 1, monitorId: legacy.id, start: now - 45 * DAY, message: 'Batch hosts are being decommissioned', creator: member(4) }),
    // Every QA monitor, nightly while the environment is rebuilt.
    dt({
      id: 2,
      monitorTags: ['env:qa'],
      start: dayStart - 14 * DAY + 2 * HOUR,
      end: dayStart - 14 * DAY + 3 * HOUR,
      recurrence: 'daily',
      message: 'Nightly QA environment rebuild',
      creator: member(2),
    }),
    dt({
      id: 3,
      monitorTags: ['service:billing'],
      start: dayStart + 2 * DAY + 5 * HOUR,
      end: dayStart + 2 * DAY + 7 * HOUR,
      message: 'Billing database upgrade to the next major version',
      creator: member(3),
    }),
    dt({ id: 4, monitorTags: ['service:order-service'], scope: ['env:prod'], start: now - 3 * DAY, end: now - 3 * DAY + 90 * MIN, message: 'order-service 3.2.0 rollout', creator: member(1) }),
    dt({ id: 5, monitorTags: ['team:platform'], start: now - 6 * DAY, end: now - 6 * DAY + 4 * HOUR, message: 'Cluster node pool rotation', creator: member(4), canceledAt: now - 6 * DAY + HOUR }),
  ];
}

function db(): State {
  if (state) return state;
  const now = Date.now();
  const monitors = SEEDS.map((s, i) => seedRow(s, 1001 + i, now));
  state = { monitors, downtimes: [], nextId: 1001 + SEEDS.length, nextDowntimeId: 6, nextEventId: 1, evaluatedAt: now };
  state.downtimes = seedDowntimes(now, monitors);
  for (const m of monitors) replay(m, now);
  return state;
}

/** Re-evaluates every monitor once a minute, like the server's evaluator. */
function fresh(): State {
  const s = db();
  const now = Date.now();
  if (now - s.evaluatedAt >= MIN) {
    for (const m of s.monitors) evaluate(m, now);
    s.evaluatedAt = now;
  }
  return s;
}

function find(id: number): Row {
  const m = fresh().monitors.find((x) => x.id === id);
  if (!m) throw bad(`monitor ${id} not found`);
  return m;
}

// ---------------------------------------------------------------- list

function item(m: Row, now: number): MonitorItem {
  const counts = { alert: 0, warn: 0, nodata: 0, ok: 0 };
  for (const g of m.groups) counts[g.status]++;
  const muted = monitorMuted(m, now);
  return {
    id: m.id,
    name: m.name,
    type: m.type,
    status: m.status,
    priority: m.priority,
    tags: m.tags,
    teams: m.teams,
    draft: m.draft,
    creator: m.creator,
    recipients: recipients(m.message),
    muted: muted !== undefined,
    mutedUntil: muted ?? null,
    query: fullQuery(m.options, m.query),
    groupCounts: counts,
    stateChangedAt: m.stateChangedAt,
    createdAt: m.createdAt,
    modifiedAt: m.modifiedAt,
  };
}

function textMatches(query: string, name: string, full: string, tags: string[]): boolean {
  return query
    .split(/\s+/)
    .filter(Boolean)
    .every((word) => {
      const w = word.toLowerCase();
      if (w.includes(':') && !w.startsWith(':')) return tags.some((t) => t.toLowerCase() === w);
      return name.toLowerCase().includes(w) || full.toLowerCase().includes(w) || tags.some((t) => t.toLowerCase().includes(w));
    });
}

function counts(values: string[]): FacetCount[] {
  const map = new Map<string, number>();
  for (const v of values) map.set(v, (map.get(v) ?? 0) + 1);
  return [...map.entries()].map(([value, count]) => ({ value, count })).sort((a, b) => b.count - a.count || (a.value < b.value ? -1 : a.value > b.value ? 1 : 0));
}

const page = <T>(rows: T[], offset: number, limit: number) => rows.slice(Math.max(0, offset), Math.max(0, offset) + Math.min(200, Math.max(1, limit)));
const byName = (a: string, b: string) => (a.toLowerCase() < b.toLowerCase() ? -1 : a.toLowerCase() > b.toLowerCase() ? 1 : 0);

export function searchMonitors(req: S['MonitorSearchRequest']): S['MonitorSearchPage'] {
  const s = fresh();
  const now = Date.now();
  const items = s.monitors.map((m) => item(m, now)).filter((i) => textMatches(req.query ?? '', i.name, i.query, i.tags));
  type F = 'status' | 'muted' | 'priority' | 'type' | 'draft' | 'recipient' | 'creator' | 'team';
  const passes = (i: MonitorItem, skip?: F) =>
    (skip === 'status' || !req.statuses?.length || req.statuses.includes(i.status)) &&
    (skip === 'muted' || !req.muted?.length || req.muted.includes(i.muted)) &&
    (skip === 'priority' || !req.priorities?.length || req.priorities.includes(i.priority ?? 0)) &&
    (skip === 'type' || !req.types?.length || req.types.includes(i.type)) &&
    (skip === 'draft' || !req.drafts?.length || req.drafts.includes(i.draft)) &&
    (skip === 'recipient' || !req.recipients?.length || i.recipients.some((r) => req.recipients.includes(r))) &&
    (skip === 'creator' || !req.creators?.length || (i.creator != null && req.creators.includes(i.creator.id))) &&
    (skip === 'team' || !req.teams?.length || i.teams.some((t) => req.teams.includes(t)));
  const facet = (f: F, key: (i: MonitorItem) => string[]) => counts(items.filter((i) => passes(i, f)).flatMap(key));
  const names = new Map(items.flatMap((i) => (i.creator ? [[i.creator.id, i.creator.name] as const] : [])));
  const facets: S['MonitorFacets'] = {
    status: facet('status', (i) => [i.status]),
    muted: facet('muted', (i) => [String(i.muted)]),
    priority: facet('priority', (i) => [String(i.priority ?? 0)]),
    type: facet('type', (i) => [i.type]),
    draft: facet('draft', (i) => [String(i.draft)]),
    recipients: facet('recipient', (i) => i.recipients),
    creators: facet('creator', (i) => (i.creator ? [i.creator.id] : [])).map((c) => ({ ...c, value: names.has(c.value) ? `${c.value}|${names.get(c.value)}` : c.value })),
    teams: facet('team', (i) => i.teams),
  };
  const matched = items.filter((i) => passes(i));
  switch (req.sort) {
    case 'name':
      matched.sort((a, b) => byName(a.name, b.name));
      break;
    case 'priority':
      matched.sort((a, b) => (a.priority ?? 9) - (b.priority ?? 9) || byName(a.name, b.name));
      break;
    case 'modified':
      matched.sort((a, b) => b.modifiedAt - a.modifiedAt);
      break;
    default:
      matched.sort((a, b) => SEVERITY[b.status] - SEVERITY[a.status] || byName(a.name, b.name));
  }
  return { monitors: page(matched, req.offset ?? 0, req.limit ?? 0), total: matched.length, facets };
}

// ---------------------------------------------------------------- triggered

export function searchTriggered(req: S['TriggeredSearchRequest']): S['TriggeredPage'] {
  const s = fresh();
  const now = Date.now();
  const rows: S['TriggeredGroup'][] = [];
  for (const m of s.monitors) {
    if (!textMatches(req.query ?? '', m.name, fullQuery(m.options, m.query), m.tags)) continue;
    for (const g of m.groups) {
      if (g.status === 'ok') continue;
      const muted = groupMuted(m, g.group, now);
      rows.push({
        monitorId: m.id,
        name: m.name,
        priority: m.priority,
        draft: m.draft,
        status: g.status,
        group: g.group,
        value: g.value,
        triggeredAt: g.stateChangedAt,
        muted: muted !== undefined,
        mutedUntil: muted ?? null,
      });
    }
  }
  type F = 'status' | 'muted' | 'priority' | 'draft' | 'within';
  const within = req.triggeredWithin ?? null;
  const pass = (r: S['TriggeredGroup'], skip?: F) =>
    (skip === 'status' || !req.statuses?.length || req.statuses.includes(r.status)) &&
    (skip === 'muted' || !req.muted?.length || req.muted.includes(r.muted)) &&
    (skip === 'priority' || !req.priorities?.length || req.priorities.includes(r.priority ?? 0)) &&
    (skip === 'draft' || !req.drafts?.length || req.drafts.includes(r.draft)) &&
    (skip === 'within' || within === null || now - r.triggeredAt <= within * MIN);
  const facet = (f: F, key: (r: S['TriggeredGroup']) => string) => counts(rows.filter((r) => pass(r, f)).map(key));
  const base = rows.filter((r) => pass(r, 'within'));
  const facets: S['TriggeredFacets'] = {
    status: facet('status', (r) => r.status),
    muted: facet('muted', (r) => String(r.muted)),
    priority: facet('priority', (r) => String(r.priority ?? 0)),
    draft: facet('draft', (r) => String(r.draft)),
    triggeredWithin: [30, 60, 720, 1440].map((m) => ({ value: String(m), count: base.filter((r) => now - r.triggeredAt <= m * MIN).length })),
  };
  const matched = rows.filter((r) => pass(r)).sort((a, b) => b.triggeredAt - a.triggeredAt || SEVERITY[b.status] - SEVERITY[a.status]);
  return { groups: page(matched, req.offset ?? 0, req.limit ?? 0), total: matched.length, facets };
}

// ---------------------------------------------------------------- detail

function detail(m: Row): MonitorDetail {
  const now = Date.now();
  return {
    ...item(m, now),
    metricQuery: m.query,
    options: m.options,
    message: m.message,
    groups: m.groups.map((g) => ({ group: g.group, status: g.status, value: g.value, lastDataAt: g.lastDataAt, stateChangedAt: g.stateChangedAt, muted: groupMuted(m, g.group, now) !== undefined })),
    evaluatedAt: m.evaluatedAt,
  };
}

export function getMonitor(id: number): MonitorDetail {
  return detail(find(id));
}

// ---------------------------------------------------------------- create / update / clone / bulk

function validate(input: MonitorInput): MonitorInput & { message: string; tags: string[]; teams: string[] } {
  const name = input.name.trim();
  if (!name || name.length > 500) throw bad('the monitor name must be 1–500 characters');
  const query = input.query.trim();
  if (input.type === 'log') {
    try {
      parseLogQuery(query);
    } catch (e) {
      throw bad(`invalid query: ${(e as Error).message}`);
    }
  } else {
    let f: ReturnType<typeof parseFormula>;
    try {
      f = parseFormula(query);
    } catch (e) {
      throw bad(`invalid query: ${(e as Error).message}`);
    }
    if (input.type === 'apm' && !f.queries.every((q) => parseMetricQuery(q)?.metric.startsWith('trace.'))) throw bad('APM monitors use trace.* metrics');
  }
  validateOptions(input.options);
  if (input.priority != null && !(input.priority >= 1 && input.priority <= 5)) throw bad('priority must be P1–P5');
  const message = input.message ?? '';
  if (message.length > 10_000) throw bad('the message is limited to 10,000 characters');
  const tags = (input.tags ?? []).map((t) => t.trim()).filter((t, i, all) => t !== '' && t !== all[i - 1]);
  if (tags.length > 50 || tags.some((t) => t.length > 200 || t.includes(','))) throw bad('up to 50 tags of at most 200 characters, without commas');
  return { ...input, name, query, message, tags, teams: input.teams ?? [] };
}

export function createMonitor(input: MonitorInput): MonitorDetail {
  const v = validate(input);
  const s = fresh();
  const now = Date.now();
  const m: Row = {
    id: s.nextId++,
    name: v.name,
    type: v.type,
    query: v.query,
    options: normOptions(v.options),
    message: v.message,
    priority: v.priority ?? null,
    tags: v.tags,
    teams: v.teams,
    draft: v.draft ?? false,
    creator: ME,
    createdAt: now,
    modifiedAt: now,
    status: 'nodata',
    stateChangedAt: now,
    evaluatedAt: null,
    groups: [],
    events: [],
  };
  s.monitors.push(m);
  evaluate(m, now);
  return detail(m);
}

export function updateMonitor(id: number, input: MonitorInput): MonitorDetail {
  const v = validate(input);
  const m = find(id);
  const now = Date.now();
  const options = normOptions(v.options);
  const reset = m.query !== v.query || JSON.stringify(m.options) !== JSON.stringify(options);
  Object.assign(m, { name: v.name, type: v.type, query: v.query, options, message: v.message, priority: v.priority ?? null, tags: v.tags, teams: v.teams, draft: v.draft ?? false, modifiedAt: now });
  if (reset) {
    // A new query or thresholds: groups are re-evaluated from scratch.
    m.groups = [];
    m.status = 'nodata';
    m.stateChangedAt = now;
    evaluate(m, now);
  }
  return detail(m);
}

export function cloneMonitor(id: number): MonitorDetail {
  const src = find(id);
  const s = db();
  const now = Date.now();
  const m: Row = {
    ...src,
    id: s.nextId++,
    name: `[Clone] ${src.name}`,
    creator: ME,
    createdAt: now,
    modifiedAt: now,
    status: 'nodata',
    stateChangedAt: now,
    evaluatedAt: null,
    groups: [],
    events: [],
  };
  s.monitors.push(m);
  evaluate(m, now);
  return detail(m);
}

export function bulkMonitors(ids: number[], action: S['BulkAction'], muteMinutes?: number | null): S['MonitorBulkResult'] {
  if (ids.length === 0 || ids.length > 1000) throw bad('select 1–1000 monitors');
  const s = fresh();
  const now = Date.now();
  const rows = s.monitors.filter((m) => ids.includes(m.id));
  switch (action) {
    case 'mute':
      if (muteMinutes != null && !(muteMinutes >= 1 && muteMinutes <= 525_600)) throw bad('mute for 1 minute to 1 year');
      for (const m of rows)
        s.downtimes.push({
          id: s.nextDowntimeId++,
          monitorId: m.id,
          monitorTags: [],
          scope: ['*'],
          message: 'Muted from the monitor',
          start: now,
          end: muteMinutes != null ? now + muteMinutes * MIN : null,
          recurrence: null,
          recurUntil: null,
          creator: ME,
          createdAt: now,
          canceledAt: null,
        });
      break;
    case 'unmute':
      for (const d of s.downtimes) if (d.monitorId !== null && ids.includes(d.monitorId) && d.canceledAt === null && d.start <= now) d.canceledAt = now;
      break;
    case 'resolve':
      for (const m of rows) {
        for (const g of m.groups) {
          if (g.status === 'ok') continue;
          m.events.push({
            id: s.nextEventId++,
            group: g.group,
            fromStatus: g.status,
            toStatus: 'ok',
            value: null,
            muted: true,
            recipients: [],
            message: `Resolved manually by ${ME.name}`,
            createdAt: now,
          });
          g.status = 'ok';
          g.stateChangedAt = now;
        }
        setOverall(m, now);
      }
      break;
    case 'delete':
      s.monitors = s.monitors.filter((m) => !ids.includes(m.id));
      s.downtimes = s.downtimes.filter((d) => d.monitorId === null || !ids.includes(d.monitorId));
      break;
  }
  return { affected: rows.length };
}

// ---------------------------------------------------------------- history

export function monitorHistory(id: number, range: TimeRange): S['MonitorHistory'] {
  if (range.from >= range.to) throw bad('the time range must end after it starts');
  const m = find(id);
  const events = m.events.filter((e) => e.createdAt >= range.from && e.createdAt <= range.to);
  const end = Math.min(range.to, Date.now());
  // Nothing to show before the monitor existed.
  const start = Math.max(range.from, m.createdAt);
  const keys = [...new Set([...m.groups.map((g) => g.group), ...events.map((e) => e.group)])];
  const segments: S['StatusSegment'][] = [];
  for (const key of keys) {
    const mine = events.filter((e) => e.group === key);
    let status = (mine[0]?.fromStatus as MonitorStatus | undefined) ?? m.groups.find((g) => g.group === key)?.status ?? 'ok';
    let from = start;
    for (const e of mine) {
      if (e.createdAt > from) segments.push({ group: key, status, from, to: e.createdAt });
      status = e.toStatus as MonitorStatus;
      from = Math.max(e.createdAt, start);
    }
    if (end > from) segments.push({ group: key, status, from, to: end });
  }
  return { events: [...events].reverse().slice(0, 500), segments };
}

// ---------------------------------------------------------------- preview (backtest)

export function previewMonitor(type: MonitorType | null | undefined, query: string, options: MonitorOptions, range: TimeRange): S['MonitorPreview'] {
  if (range.from >= range.to) throw bad('the time range must end after it starts');
  if (range.to - range.from > 7 * DAY) throw bad('previews cover at most a week');
  validateOptions(options);
  const o = normOptions(options);
  const span = range.to - range.from;
  const fetch = { from: range.from - lookbackMs(o), to: range.to };
  const points = Math.min(1000, Math.max(10, Math.floor((fetch.to - fetch.from) / Math.max(span / 300, 10_000))));
  const kind = type ?? 'metric';
  const groups = monitorSeries(kind, query, o, fetch, points);
  const b = backtest(o, groups, range.from, range.to, Math.max(span / 240, MIN));
  const transitions = { alert: 0, warn: 0, recover: 0, nodata: 0 };
  for (const t of b.transitions) transitions[t.to === 'ok' ? 'recover' : t.to]++;
  // Chart the groups with the largest values.
  const peak = (g: GroupSeries) => g.points.reduce((m, p) => Math.max(m, p[1]), -Infinity);
  const order = groups
    .map((_, i) => i)
    .sort((x, y) => peak(groups[y]) - peak(groups[x]))
    .slice(0, 20);
  return {
    raw: order.map((i) => ({ group: groupKey(groups[i].tags), points: groups[i].points.filter((p) => p[0] >= range.from).map(([t, v]) => [t, v] as [number, number | null]) })),
    evaluated: order.map((i) => ({ group: b.evaluated[i][0], points: b.evaluated[i][1] })),
    transitions,
    groups: groups.length,
  };
}

// ---------------------------------------------------------------- quality

export function monitorQuality(req: S['MonitorQualityRequest']): S['MonitorQuality'] {
  const s = fresh();
  const now = Date.now();
  const tagValues = (tags: string[], key: string) => tags.filter((t) => t.startsWith(`${key}:`)).map((t) => t.slice(key.length + 1));
  const creators = new Map<string, string>();
  const teams = new Set<string>();
  const services = new Set<string>();
  const envs = new Set<string>();
  for (const m of s.monitors) {
    if (m.creator) creators.set(m.creator.id, m.creator.name);
    m.teams.forEach((t) => teams.add(t));
    tagValues(m.tags, 'service').forEach((t) => services.add(t));
    tagValues(m.tags, 'env').forEach((t) => envs.add(t));
  }
  const any = (want: string[] | undefined, have: string[]) => !want?.length || have.some((h) => want.includes(h));
  const scoped = s.monitors.filter(
    (m) => any(req.creators, m.creator ? [m.creator.id] : []) && any(req.teams, m.teams) && any(req.services, tagValues(m.tags, 'service')) && any(req.envs, tagValues(m.tags, 'env')),
  );
  const q = (m: Row, detail: string): S['QualityMonitor'] => ({ id: m.id, name: m.name, type: m.type, detail });
  // Alerts per day over the last week, or over the monitor's history when it's younger than that.
  const alertsPerDay = (m: Row) => {
    const week = m.events.filter((e) => e.createdAt > now - 7 * DAY);
    if (!week.length) return undefined;
    const days = Math.max(1, Math.min(7, (now - Math.min(...week.map((e) => e.createdAt))) / DAY));
    return week.filter((e) => e.toStatus === 'alert').length / days;
  };
  const high = scoped
    .map((m) => [alertsPerDay(m) ?? 0, m] as const)
    .filter(([d]) => d >= 5)
    .sort((a, b) => b[0] - a[0]);
  const missing = scoped.filter((m) => !m.draft && recipients(m.message).length === 0);
  const mutedLong = scoped.flatMap((m) => {
    if (monitorMuted(m, now) === undefined) return [];
    const starts = s.downtimes.filter((d) => d.canceledAt === null && appliesTo(d, m)).flatMap((d) => activeAt(d, now)?.[0] ?? []);
    if (!starts.length) return [];
    const since = Math.min(...starts);
    return now - since > 30 * DAY ? [[now - since, m] as const] : [];
  });
  const stuck = (status: MonitorStatus, days: number) => scoped.filter((m) => m.status === status && now - m.stateChangedAt > days * DAY);
  const days = (ms: number) => Math.floor(ms / DAY);
  const plural = (n: number, one: string, many: string) => `${n} ${n === 1 ? one : many}`;
  const issues: S['QualityIssue'][] = [];
  const push = (kind: string, title: string, description: string, fix: string, monitors: S['QualityMonitor'][]) => {
    if (monitors.length) issues.push({ kind, title, description, fix, monitors });
  };
  push(
    'high_volume',
    `${plural(high.length, 'monitor is', 'monitors are')} generating a high volume of alerts`,
    'Monitors that alerted at least 5 times a day on average over the last week. Try adjusting the thresholds, evaluation window or recovery thresholds to improve accuracy and reduce noise.',
    'Resolve this issue by editing or deleting the monitor',
    high.map(([d, m]) => q(m, `${Math.round(d)} alerts daily`)),
  );
  push(
    'missing_recipients',
    `${plural(missing.length, 'monitor is', 'monitors are')} missing recipients`,
    "These monitors don't @-mention anyone in their message, so nobody is notified when they trigger.",
    'Add @handles to the monitor message',
    missing.map((m) => q(m, 'No recipients')),
  );
  push(
    'muted_long',
    `${plural(mutedLong.length, 'monitor has been', 'monitors have been')} muted for more than 30 days`,
    "Long-muted monitors give a false sense of coverage. Unmute them or delete them if they're no longer needed.",
    'Unmute or delete the monitor',
    mutedLong.map(([ms, m]) => q(m, `Muted for ${days(ms)} days`)),
  );
  const alerting = stuck('alert', 7);
  push(
    'stuck_alert',
    `${plural(alerting.length, 'monitor has been', 'monitors have been')} alerting for more than a week`,
    'A monitor that never recovers is usually misconfigured or ignored.',
    'Review the thresholds or fix the underlying issue',
    alerting.map((m) => q(m, `Alerting for ${days(now - m.stateChangedAt)} days`)),
  );
  const silent = stuck('nodata', 1);
  push(
    'stuck_no_data',
    `${plural(silent.length, 'monitor has been', 'monitors have been')} reporting No Data for more than a day`,
    'The query returns no data: the metric may have been renamed, or its tags no longer match.',
    'Fix the monitor query',
    silent.map((m) => q(m, `No data for ${Math.max(1, days(now - m.stateChangedAt))} days`)),
  );
  const flagged = new Set(issues.flatMap((i) => i.monitors.map((m) => m.id)));
  return {
    healthy: scoped.filter((m) => !flagged.has(m.id)).length,
    toImprove: flagged.size,
    issues,
    creators: [...creators.entries()].sort((a, b) => (a[0] < b[0] ? -1 : 1)).map(([id, name]) => ({ id, name })),
    teams: [...teams].sort(),
    services: [...services].sort(),
    envs: [...envs].sort(),
  };
}

// ---------------------------------------------------------------- test notifications

export function sendTestNotification(req: S['TestNotification']): S['TestDelivery'][] {
  const who = recipients(req.message);
  if (who.length === 0) throw bad('The message mentions no @recipients');
  if (who.length > 20) throw bad('At most 20 recipients per test');
  return who.map((r) => {
    if (r.includes('@')) {
      const known = USERS.some((u) => u.email.toLowerCase() === r.toLowerCase());
      return known ? { recipient: r, status: 'sent', error: null } : { recipient: r, status: 'unknown_recipient', error: `${r} isn't an active member of this organization` };
    }
    return CHANNELS.includes(r)
      ? { recipient: r, status: 'sent', error: null }
      : { recipient: r, status: 'unknown_recipient', error: `No channel named @${r}. Add it under Monitors → Settings → Notification Channels.` };
  });
}

// ---------------------------------------------------------------- downtimes

export function searchDowntimes(req: S['DowntimeSearchRequest']): S['DowntimeSearchPage'] {
  const s = fresh();
  const now = Date.now();
  const words = (req.query ?? '').toLowerCase().split(/\s+/).filter(Boolean);
  const items = s.downtimes
    .filter((d) => d.canceledAt === null || (req.includeCanceled && d.canceledAt > now - 30 * DAY))
    .map((d) => downtimeItem(d, now))
    .filter((i) => {
      const hay = `${i.monitorName ?? ''} ${i.message} ${i.scope.join(' ')} ${i.monitorTags.join(' ')}`.toLowerCase();
      return words.every((w) => hay.includes(w));
    });
  const isActive = (i: DowntimeItem) => i.status === 'active';
  const pass = (i: DowntimeItem, skip: 0 | 1 | 2 | 3) =>
    (skip === 1 || !req.active?.length || req.active.includes(isActive(i))) &&
    (skip === 2 || !req.recurring?.length || req.recurring.includes(i.recurrence != null)) &&
    (skip === 3 || !req.creators?.length || (i.creator != null && req.creators.includes(i.creator.id)));
  const boolFacet = (skip: 1 | 2, f: (i: DowntimeItem) => boolean) =>
    ['true', 'false'].map((v) => ({ value: v, label: v === 'true' ? 'True' : 'False', count: items.filter((i) => pass(i, skip) && String(f(i)) === v).length }));
  const creators: S['DowntimeFacetCount'][] = [];
  for (const i of items.filter((x) => pass(x, 3))) {
    if (!i.creator) continue;
    const f = creators.find((c) => c.value === i.creator!.id);
    if (f) f.count++;
    else creators.push({ value: i.creator.id, label: i.creator.name, count: 1 });
  }
  creators.sort((a, b) => b.count - a.count || byName(a.label, b.label));
  const matched = items.filter((i) => pass(i, 0)).sort((a, b) => Number(isActive(b)) - Number(isActive(a)) || b.start - a.start);
  return {
    downtimes: page(matched, req.offset ?? 0, req.limit ?? 0),
    total: matched.length,
    facets: { active: boolFacet(1, isActive), recurring: boolFacet(2, (i) => i.recurrence != null), creators },
  };
}

export function createDowntime(input: DowntimeInput): DowntimeItem {
  const s = fresh();
  const now = Date.now();
  const start = input.start ?? now;
  if (input.end != null && (input.end <= start || input.end <= now)) throw bad('the downtime must end after it starts, in the future');
  if (input.recurrence) {
    if (input.end == null) throw bad('recurring downtimes need an end time');
    if (input.end - start >= (input.recurrence === 'daily' ? DAY : 7 * DAY)) throw bad('each occurrence must be shorter than the repeat period');
  }
  const scope = (input.scope ?? []).map((x) => x.trim()).filter((x) => x !== '' && x !== '*');
  if (scope.some((x) => !x.includes(':'))) throw bad('scope entries are key:value tags, e.g. host:web-1');
  if (input.monitorId != null && !s.monitors.some((m) => m.id === input.monitorId)) throw bad(`monitor ${input.monitorId} not found`);
  const d: Downtime = {
    id: s.nextDowntimeId++,
    monitorId: input.monitorId ?? null,
    monitorTags: input.monitorId != null ? [] : (input.monitorTags ?? []),
    scope,
    message: (input.message ?? '').trim(),
    start,
    end: input.end ?? null,
    recurrence: input.recurrence ?? null,
    recurUntil: input.recurUntil ?? null,
    creator: ME,
    createdAt: now,
    canceledAt: null,
  };
  s.downtimes.push(d);
  return downtimeItem(d, now);
}

export function cancelDowntime(id: number): void {
  const d = fresh().downtimes.find((x) => x.id === id && x.canceledAt === null);
  if (!d) throw bad(`downtime ${id} not found`);
  d.canceledAt = Date.now();
}
