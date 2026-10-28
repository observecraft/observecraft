// In-browser service level objectives for mock mode. State lives in memory for the session; every SLO
// gets a deterministic hourly history of good and total events (or minutes) over 90 days, and the
// SLI, error budget, burn rate and state follow the server's math (routes/slo_math.rs).
import type { components } from '../generated/schema';
import type {
  BudgetUnit,
  BurnLevel,
  CorrectionCategory,
  SloCorrection,
  SloCorrectionInput,
  SloDetail,
  SloFacetCount,
  SloInput,
  SloItem,
  SloMonitor,
  SloSearchPage,
  SloSearchRequest,
  SloState,
  SloStatus,
  SloTarget,
  SloType,
  SloWindow,
} from '../slos/types';
import { searchMonitors } from './monitors';
import { formulaQueries } from '../../lib/metricQuery';
import { USERS } from './errorTracking';

type MemberRef = components['schemas']['MemberRef'];

const MIN = 60_000;
const HOUR = 60 * MIN;
const DAY = 24 * HOUR;
const HOURS = 90 * 24;
const MAX_SLOS = 500;
const WINDOW_MS: Record<SloWindow, number> = { '1d': DAY, '7d': 7 * DAY, '30d': 30 * DAY, '90d': 90 * DAY };
const HISTORY_BUCKET: Record<SloWindow, number> = { '1d': HOUR, '7d': 2 * HOUR, '30d': 6 * HOUR, '90d': DAY };
const WINDOWS: SloWindow[] = ['1d', '7d', '30d', '90d'];

const fail = (message: string): never => {
  throw new Error(message);
};
const copy = <T>(v: T): T => structuredClone(v);

interface Measure {
  good: number;
  total: number;
}

/** A bad period: from `ago` hours before now, for `hours`, with this share of bad events (or down time). */
interface Incident {
  ago: number;
  hours: number;
  bad: number;
}

interface Profile {
  /** Usual share of bad events (metric) or of bad time (monitor, time slice). */
  baseline: number;
  incidents: Incident[];
  /** Events per hour at the daily peak (metric SLOs). */
  volume: number;
  noData?: boolean;
}

interface Row {
  slo: Omit<SloItem, 'status'>;
  profile: Profile;
}

interface State {
  slos: Row[];
  corrections: SloCorrection[];
  seq: number;
  /** Start of the newest hourly bucket when the store was seeded. */
  anchor: number;
}

let state: State | null = null;

const member = (i: number): MemberRef => ({ id: USERS[i].id, name: USERS[i].name });

/** Deterministic pseudo-random numbers in [0, 1) from a string. */
function rng(seed: string): () => number {
  let h = 2166136261;
  for (const c of seed) h = Math.imul(h ^ c.charCodeAt(0), 16777619);
  return () => {
    h = Math.imul(h ^ (h >>> 15), 2246822507);
    h = Math.imul(h ^ (h >>> 13), 3266489909);
    h ^= h >>> 16;
    return (h >>> 0) / 4294967296;
  };
}

/** Good and total amounts per hour, oldest first: events for metric SLOs, milliseconds otherwise. */
function hourly(row: Row, anchor: number): [number, Measure][] {
  const { profile, slo } = row;
  if (profile.noData) return [];
  const r = rng(slo.id);
  const created = Math.max(slo.createdAt, anchor - HOURS * HOUR);
  const out: [number, Measure][] = [];
  for (let i = HOURS - 1; i >= 0; i--) {
    const t = anchor - i * HOUR;
    if (t + HOUR <= created) continue;
    const incident = profile.incidents.find((x) => i < x.ago && i >= x.ago - x.hours);
    const bad = incident ? incident.bad : profile.baseline * (0.4 + 1.2 * r());
    if (slo.type === 'metric') {
      const hourOfDay = new Date(t).getUTCHours();
      const total = Math.round(profile.volume * (0.45 + 0.55 * Math.sin(((hourOfDay - 6) / 24) * Math.PI * 2) ** 2) * (0.9 + 0.2 * r()));
      out.push([t, { good: total - Math.round(total * bad), total }]);
    } else {
      out.push([t, { good: HOUR * (1 - bad), total: HOUR }]);
    }
  }
  return out;
}

const SEEDS: {
  name: string;
  type: SloType;
  description: string;
  def: Partial<SloInput>;
  targets: SloTarget[];
  primary: SloWindow;
  tags: string[];
  teams: string[];
  creator: number;
  ageDays: number;
  profile: Profile;
}[] = [
  {
    name: 'Checkout API availability',
    type: 'metric',
    description: 'Share of checkout requests that do not fail with a server error.',
    def: { goodQuery: 'sum:trace.http.request.hits{service:order-service,!http.status_code:5*}', totalQuery: 'sum:trace.http.request.hits{service:order-service}' },
    targets: [
      { window: '7d', target: 99.9, warning: 99.95 },
      { window: '30d', target: 99.9, warning: 99.95 },
    ],
    primary: '30d',
    tags: ['service:order-service', 'env:prod', 'journey:checkout'],
    teams: ['checkout'],
    creator: 1,
    ageDays: 120,
    profile: { baseline: 0.0004, volume: 42_000, incidents: [{ ago: 200, hours: 3, bad: 0.012 }] },
  },
  {
    name: 'Billing payment success',
    type: 'metric',
    description: 'Payment authorizations that succeed.',
    def: { goodQuery: 'sum:billing.payments.authorized{env:prod}', totalQuery: 'sum:billing.payments.attempted{env:prod}' },
    targets: [{ window: '30d', target: 99.95, warning: 99.97 }],
    primary: '30d',
    tags: ['service:billing', 'env:prod', 'journey:checkout'],
    teams: ['payments'],
    creator: 2,
    ageDays: 200,
    profile: {
      baseline: 0.0002,
      volume: 9_000,
      incidents: [
        { ago: 70, hours: 5, bad: 0.03 },
        { ago: 1, hours: 1, bad: 0.006 },
      ],
    },
  },
  {
    name: 'Storefront p95 latency under 500 ms',
    type: 'time_slice',
    description: 'Five-minute slices in which the storefront answers fast enough.',
    def: { sliceQuery: 'avg:trace.latency.p95{service:storefront-api,env:prod}', comparator: '<', threshold: 0.5, sliceMinutes: 5 },
    targets: [
      { window: '7d', target: 99, warning: 99.5 },
      { window: '30d', target: 99, warning: 99.5 },
    ],
    primary: '7d',
    tags: ['service:storefront-api', 'env:prod'],
    teams: ['checkout'],
    creator: 0,
    ageDays: 60,
    profile: { baseline: 0.001, volume: 0, incidents: [{ ago: 30, hours: 1, bad: 0.5 }] },
  },
  {
    name: 'Platform host health',
    type: 'monitor',
    description: 'Time the host CPU and memory monitors are not alerting.',
    def: { monitorIds: [1001, 1002] },
    targets: [
      { window: '30d', target: 99.9, warning: 99.95 },
      { window: '90d', target: 99.9 },
    ],
    primary: '30d',
    tags: ['env:prod', 'tier:platform'],
    teams: ['platform'],
    creator: 4,
    ageDays: 300,
    profile: { baseline: 0, volume: 0, incidents: [{ ago: 500, hours: 2, bad: 1 }] },
  },
  {
    name: 'Catalog search availability',
    type: 'metric',
    description: 'Catalog searches answered without an error.',
    def: { goodQuery: 'sum:trace.http.request.hits{service:catalog-service,!http.status_code:5*}', totalQuery: 'sum:trace.http.request.hits{service:catalog-service}' },
    targets: [{ window: '7d', target: 99.5, warning: 99.7 }],
    primary: '7d',
    tags: ['service:catalog-service', 'env:prod'],
    teams: ['catalog'],
    creator: 3,
    ageDays: 90,
    profile: { baseline: 0.0006, volume: 18_000, incidents: [] },
  },
  {
    name: 'Inventory sync freshness',
    type: 'time_slice',
    description: 'Minutes in which stock levels are less than two minutes old.',
    def: { sliceQuery: 'max:inventory.sync.lag_seconds{env:prod}', comparator: '<=', threshold: 120, sliceMinutes: 1 },
    targets: [{ window: '30d', target: 99 }],
    primary: '30d',
    tags: ['service:inventory-service', 'env:prod'],
    teams: ['catalog'],
    creator: 3,
    ageDays: 45,
    profile: { baseline: 0.002, volume: 0, incidents: [{ ago: 400, hours: 2, bad: 0.8 }] },
  },
  {
    name: 'Order processing success',
    type: 'metric',
    description: 'Orders placed that reach the confirmed state.',
    def: { goodQuery: 'sum:orders.confirmed{env:prod}', totalQuery: 'sum:orders.placed{env:prod}' },
    targets: [
      { window: '30d', target: 99.5, warning: 99.8 },
      { window: '90d', target: 99.5 },
    ],
    primary: '30d',
    tags: ['service:order-service', 'env:prod', 'journey:checkout'],
    teams: ['checkout', 'payments'],
    creator: 1,
    ageDays: 150,
    profile: { baseline: 0.0015, volume: 3_000, incidents: [] },
  },
  {
    name: 'Storefront API availability (staging)',
    type: 'metric',
    description: 'Created before staging reports trace metrics.',
    def: { goodQuery: 'sum:trace.http.request.hits{service:storefront-api,env:staging,!http.status_code:5*}', totalQuery: 'sum:trace.http.request.hits{service:storefront-api,env:staging}' },
    targets: [{ window: '7d', target: 99 }],
    primary: '7d',
    tags: ['service:storefront-api', 'env:staging'],
    teams: ['checkout'],
    creator: 0,
    ageDays: 2,
    profile: { baseline: 0, volume: 0, incidents: [], noData: true },
  },
];

function emptyDef(type: SloType, def: Partial<SloInput>): Pick<SloItem, 'goodQuery' | 'totalQuery' | 'monitorIds' | 'sliceQuery' | 'comparator' | 'threshold' | 'sliceMinutes'> {
  return {
    goodQuery: type === 'metric' ? (def.goodQuery ?? null) : null,
    totalQuery: type === 'metric' ? (def.totalQuery ?? null) : null,
    monitorIds: type === 'monitor' ? (def.monitorIds ?? []) : [],
    sliceQuery: type === 'time_slice' ? (def.sliceQuery ?? null) : null,
    comparator: type === 'time_slice' ? (def.comparator ?? null) : null,
    threshold: type === 'time_slice' ? (def.threshold ?? null) : null,
    sliceMinutes: type === 'time_slice' ? (def.sliceMinutes ?? null) : null,
  };
}

function db(): State {
  if (state) return state;
  const anchor = Math.floor(Date.now() / HOUR) * HOUR;
  const slos: Row[] = SEEDS.map((s, i) => {
    const at = anchor - s.ageDays * DAY;
    return {
      slo: {
        id: `slo-${String(i + 1).padStart(4, '0')}`,
        name: s.name,
        description: s.description,
        type: s.type,
        ...emptyDef(s.type, s.def),
        targets: s.targets,
        primaryWindow: s.primary,
        tags: s.tags,
        teams: s.teams,
        creator: member(s.creator),
        createdAt: at,
        updatedAt: at + DAY,
      },
      profile: s.profile,
    };
  });
  const correction = (id: string, sloId: string | null, category: CorrectionCategory, description: string, start: number, hours: number, creator: number): SloCorrection => ({
    id,
    sloId,
    sloName: slos.find((s) => s.slo.id === sloId)?.slo.name ?? null,
    category,
    description,
    start,
    end: start + hours * HOUR,
    creator: member(creator),
    createdAt: start - DAY,
  });
  state = {
    slos,
    corrections: [
      correction('corr-0001', null, 'scheduled_maintenance', 'Database failover drill', anchor - 12 * DAY, 2, 4),
      correction('corr-0002', 'slo-0001', 'deployment', 'order-service 3.2.0 rollout', anchor - 5 * DAY, 1, 1),
      correction('corr-0003', 'slo-0006', 'outside_business_hours', 'Warehouse feed paused overnight', anchor - 20 * DAY, 8, 3),
    ],
    seq: slos.length + 1,
    anchor,
  };
  return state;
}

// ---------------------------------------------------------------- math (routes/slo_math.rs)

function correctionSpans(sloId: string): [number, number][] {
  const spans = db()
    .corrections.filter((c) => !c.sloId || c.sloId === sloId)
    .map((c): [number, number] => [c.start, c.end])
    .sort((a, b) => a[0] - b[0]);
  const out: [number, number][] = [];
  for (const s of spans) {
    const last = out[out.length - 1];
    if (last && s[0] <= last[1]) last[1] = Math.max(last[1], s[1]);
    else out.push([...s]);
  }
  return out;
}

const covered = (from: number, to: number, spans: [number, number][]) => spans.reduce((n, s) => n + Math.max(0, Math.min(s[1], to) - Math.max(s[0], from)), 0);

/** Hourly measures within the window ending now, each scaled by its share outside the corrections. */
function windowMeasures(row: Row, window: SloWindow): [number, Measure][] {
  const s = db();
  const to = s.anchor + HOUR;
  const from = to - WINDOW_MS[window];
  const spans = correctionSpans(row.slo.id);
  return hourly(row, s.anchor)
    .filter(([t]) => t >= from)
    .map(([t, m]): [number, Measure] => {
      const share = (HOUR - covered(t, t + HOUR, spans)) / HOUR;
      return [t, { good: m.good * share, total: m.total * share }];
    })
    .filter(([, m]) => m.total > 0);
}

const sum = (ms: Measure[]): Measure => ms.reduce((a, m) => ({ good: a.good + m.good, total: a.total + m.total }), { good: 0, total: 0 });
const sli = (m: Measure) => (m.total > 0 ? Math.min(100, Math.max(0, (m.good / m.total) * 100)) : null);
const burn = (m: Measure, target: number) => (m.total > 0 ? (m.total - m.good) / m.total / (1 - target / 100) : null);

function stateOf(value: number | null, target: number, warning?: number | null): SloState {
  if (value === null) return 'no_data';
  if (value < target) return 'breached';
  if (warning !== null && warning !== undefined && value < warning) return 'warning';
  return 'ok';
}

function targetFor(slo: Omit<SloItem, 'status'>, window: SloWindow): SloTarget {
  const t = slo.targets.find((x) => x.window === window) ?? slo.targets.find((x) => x.window === slo.primaryWindow) ?? slo.targets[0];
  return { ...t, window };
}

function statusOf(row: Row, window: SloWindow): SloStatus {
  const target = targetFor(row.slo, window);
  const measures = windowMeasures(row, window);
  const m = sum(measures.map(([, x]) => x));
  const unit: BudgetUnit = row.slo.type === 'metric' ? 'events' : 'minutes';
  const scale = unit === 'minutes' ? 1 / MIN : 1;
  const value = sli(m);
  const allowed = m.total * (1 - target.target / 100);
  const used = Math.max(0, m.total - m.good);
  const last = measures.length ? measures[measures.length - 1][1] : { good: 0, total: 0 };
  return {
    window,
    target: target.target,
    warning: target.warning ?? null,
    state: stateOf(value, target.target, target.warning),
    sli: value,
    good: m.good * scale,
    total: m.total * scale,
    unit,
    budgetRemaining: m.total > 0 ? ((allowed - used) / allowed) * 100 : null,
    budgetAllowed: m.total > 0 ? allowed * scale : null,
    budgetUsed: m.total > 0 ? used * scale : null,
    burnRate1h: burn(last, target.target),
    burnRate: burn(m, target.target),
  };
}

function historyOf(row: Row, window: SloWindow): SloDetail['history'] {
  const target = targetFor(row.slo, window).target;
  const measures = windowMeasures(row, window);
  const to = db().anchor + HOUR;
  const from = to - WINDOW_MS[window];
  const size = HISTORY_BUCKET[window];
  const buckets = new Map<number, Measure>();
  for (const [t, m] of measures) {
    const start = from + Math.floor((t - from) / size) * size;
    const b = buckets.get(start) ?? { good: 0, total: 0 };
    buckets.set(start, { good: b.good + m.good, total: b.total + m.total });
  }
  const sorted = [...buckets].sort((a, b) => a[0] - b[0]);
  const all = sum(sorted.map(([, m]) => m));
  const allowed = all.total * (1 - target / 100);
  let sofar: Measure = { good: 0, total: 0 };
  return sorted.map(([t, m]) => {
    sofar = { good: sofar.good + m.good, total: sofar.total + m.total };
    return { t, bucketSli: sli(m), sli: sli(sofar), budgetRemaining: allowed > 0 ? ((allowed - (sofar.total - sofar.good)) / allowed) * 100 : null };
  });
}

// ---------------------------------------------------------------- list

const burnLevel = (rate: number | null | undefined): BurnLevel => (rate === null || rate === undefined ? 'unknown' : rate >= 10 ? 'high' : rate >= 1 ? 'elevated' : 'normal');
const tagValues = (tags: string[], key: string) => tags.filter((t) => t.startsWith(`${key}:`)).map((t) => t.slice(key.length + 1));
const otherTags = (tags: string[]) => tags.filter((t) => !t.startsWith('service:') && !t.startsWith('env:'));
const SEVERITY: Record<SloState, number> = { breached: 3, warning: 2, no_data: 1, ok: 0 };

type FacetKey = 'type' | 'state' | 'burn' | 'creator' | 'team' | 'service' | 'env' | 'tag';

function textMatches(query: string, i: SloItem): boolean {
  return query
    .split(/\s+/)
    .filter(Boolean)
    .every((word) => {
      const w = word.toLowerCase();
      if (w.includes(':') && !w.startsWith(':')) return i.tags.some((t) => t.toLowerCase() === w);
      return i.name.toLowerCase().includes(w) || i.description.toLowerCase().includes(w) || i.tags.some((t) => t.toLowerCase().includes(w));
    });
}

function passes(i: SloItem, req: SloSearchRequest, skip?: FacetKey): boolean {
  const any = (want: string[], have: string[]) => want.length === 0 || have.some((h) => want.includes(h));
  return (
    (skip === 'type' || req.types.length === 0 || req.types.includes(i.type)) &&
    (skip === 'state' || req.states.length === 0 || req.states.includes(i.status.state)) &&
    (skip === 'burn' || req.burnRates.length === 0 || req.burnRates.includes(burnLevel(i.status.burnRate1h))) &&
    (skip === 'creator' || req.creators.length === 0 || (!!i.creator && req.creators.includes(i.creator.id))) &&
    (skip === 'team' || any(req.teams, i.teams)) &&
    (skip === 'service' || any(req.services, tagValues(i.tags, 'service'))) &&
    (skip === 'env' || any(req.envs, tagValues(i.tags, 'env'))) &&
    (skip === 'tag' || any(req.tags, otherTags(i.tags)))
  );
}

function counts(values: string[]): SloFacetCount[] {
  const map = new Map<string, number>();
  for (const v of values) map.set(v, (map.get(v) ?? 0) + 1);
  return [...map].map(([value, count]) => ({ value, count })).sort((a, b) => b.count - a.count || a.value.localeCompare(b.value));
}

const item = (row: Row, window?: SloWindow | null): SloItem => ({ ...copy(row.slo), status: statusOf(row, window ?? row.slo.primaryWindow) });

export async function searchSlos(req: SloSearchRequest): Promise<SloSearchPage> {
  const items = db()
    .slos.map((r) => item(r, req.window))
    .filter((i) => textMatches(req.query, i));
  const facet = (key: FacetKey, values: (i: SloItem) => string[]) => counts(items.filter((i) => passes(i, req, key)).flatMap(values));
  const facets = {
    type: facet('type', (i) => [i.type]),
    state: facet('state', (i) => [i.status.state]),
    burnRate: facet('burn', (i) => [burnLevel(i.status.burnRate1h)]),
    creators: facet('creator', (i) => (i.creator ? [`${i.creator.id}|${i.creator.name}`] : [])),
    teams: facet('team', (i) => i.teams),
    services: facet('service', (i) => tagValues(i.tags, 'service')),
    envs: facet('env', (i) => tagValues(i.tags, 'env')),
    tags: facet('tag', (i) => otherTags(i.tags)),
  };
  const summary = { breached: 0, warning: 0, ok: 0, noData: 0, total: 0 };
  for (const i of items.filter((x) => passes(x, req, 'state'))) {
    summary.total++;
    if (i.status.state === 'no_data') summary.noData++;
    else summary[i.status.state]++;
  }
  const matched = items.filter((i) => passes(i, req));
  const name = (i: SloItem) => i.name.toLowerCase();
  const num = (v: number | null | undefined) => v ?? Infinity;
  const sorters: Record<string, (a: SloItem, b: SloItem) => number> = {
    name: (a, b) => name(a).localeCompare(name(b)),
    sli: (a, b) => num(a.status.sli) - num(b.status.sli) || name(a).localeCompare(name(b)),
    budget: (a, b) => num(a.status.budgetRemaining) - num(b.status.budgetRemaining) || name(a).localeCompare(name(b)),
    modified: (a, b) => b.updatedAt - a.updatedAt,
    state: (a, b) => SEVERITY[b.status.state] - SEVERITY[a.status.state] || name(a).localeCompare(name(b)),
  };
  matched.sort(sorters[req.sort ?? 'state'] ?? sorters.state);
  const limit = req.limit > 0 ? Math.min(req.limit, 200) : 50;
  return { slos: matched.slice(Math.max(0, req.offset), Math.max(0, req.offset) + limit), total: matched.length, summary, facets };
}

function find(id: string): Row {
  return db().slos.find((r) => r.slo.id === id) ?? fail(`SLO ${id} not found`);
}

async function monitorsOf(row: Row): Promise<SloMonitor[]> {
  const page = searchMonitors({ query: '', statuses: [], muted: [], priorities: [], types: [], drafts: [], recipients: [], creators: [], teams: [], sort: 'name', offset: 0, limit: 200 });
  return page.monitors.filter((m) => row.slo.monitorIds.includes(m.id) || m.tags.includes(`slo:${row.slo.id}`)).map((m) => ({ id: m.id, name: m.name, status: m.status }));
}

export async function getSlo(id: string, window?: SloWindow | null): Promise<SloDetail> {
  const row = find(id);
  const w = window ?? row.slo.primaryWindow;
  const since = db().anchor + HOUR - WINDOW_MS[w];
  return {
    slo: item(row, w),
    statuses: row.slo.targets.map((t) => statusOf(row, t.window)),
    history: historyOf(row, w),
    corrections: copy(db().corrections.filter((c) => (!c.sloId || c.sloId === id) && c.end > since)),
    monitors: await monitorsOf(row),
  };
}

// ---------------------------------------------------------------- writes

const MAX_NAME_CHARS = 200;
const MAX_DESCRIPTION_CHARS = 2000;
const MAX_QUERY_CHARS = 2000;

function cleanList(values: string[] | undefined): string[] {
  return [...new Set((values ?? []).map((v) => v.trim()).filter(Boolean))];
}

function checkQuery(label: string, q: string | null | undefined): string {
  const text = (q ?? '').trim();
  if (!text) fail(`The ${label} query is required`);
  if (text.length > MAX_QUERY_CHARS) fail(`The ${label} query is at most ${MAX_QUERY_CHARS} characters`);
  if (!formulaQueries(text)) fail(`Invalid ${label} query: ${text}`);
  return text;
}

/** Trimmed and checked as the server does; throws with the server's messages. */
function validate(input: SloInput): Omit<SloItem, 'id' | 'status' | 'creator' | 'createdAt' | 'updatedAt'> {
  const name = input.name.trim();
  const description = (input.description ?? '').trim();
  if (!name || name.length > MAX_NAME_CHARS) fail(`Name must be 1–${MAX_NAME_CHARS} characters`);
  if (description.length > MAX_DESCRIPTION_CHARS) fail(`The description is at most ${MAX_DESCRIPTION_CHARS} characters`);
  const def = emptyDef(input.type, input);
  if (input.type === 'metric') {
    def.goodQuery = checkQuery('good events', input.goodQuery);
    def.totalQuery = checkQuery('total events', input.totalQuery);
  }
  if (input.type === 'monitor') {
    def.monitorIds = [...new Set(input.monitorIds ?? [])].sort((a, b) => a - b);
    if (def.monitorIds.length === 0 || def.monitorIds.length > 20) fail('Pick 1–20 monitors');
  }
  if (input.type === 'time_slice') {
    def.sliceQuery = checkQuery('time slice', input.sliceQuery);
    if (!input.comparator || input.threshold === null || input.threshold === undefined || !Number.isFinite(input.threshold)) fail('A time slice needs a comparator and a numeric threshold');
    if (input.sliceMinutes !== 1 && input.sliceMinutes !== 5) fail('Time slices are 1 or 5 minutes');
  }
  const targets = input.targets;
  if (targets.length === 0 || targets.length > 4) fail('Set a target for 1–4 time windows');
  if (new Set(targets.map((t) => t.window)).size !== targets.length) fail('Set one target per time window');
  for (const t of targets) {
    if (!(Number.isFinite(t.target) && t.target > 0 && t.target < 100)) fail('Targets are percentages above 0 and below 100');
    if (t.warning !== null && t.warning !== undefined && !(t.warning > t.target && t.warning < 100)) fail('A warning threshold is above its target and below 100');
  }
  const primaryWindow = input.primaryWindow ?? targets[0].window;
  if (!targets.some((t) => t.window === primaryWindow)) fail('The primary time window needs a target');
  const tags = cleanList(input.tags);
  const teams = cleanList(input.teams);
  if (tags.length > 50 || teams.length > 50) fail('At most 50 tags and 50 teams');
  return {
    name,
    description,
    type: input.type,
    ...def,
    targets: [...targets].sort((a, b) => WINDOWS.indexOf(a.window) - WINDOWS.indexOf(b.window)).map((t) => ({ window: t.window, target: t.target, warning: t.warning ?? null })),
    primaryWindow,
    tags,
    teams,
  };
}

/** A plausible history for a new SLO, derived from its name. */
function profileFor(name: string): Profile {
  const r = rng(name);
  return { baseline: 0.0002 + r() * 0.0008, volume: 2_000 + Math.round(r() * 20_000), incidents: [{ ago: 10 + Math.round(r() * 300), hours: 1, bad: 0.01 + r() * 0.02 }] };
}

export async function createSlo(input: SloInput): Promise<SloItem> {
  const s = db();
  const v = validate(input);
  if (s.slos.length >= MAX_SLOS) fail(`An organization has at most ${MAX_SLOS} SLOs`);
  const now = Date.now();
  // New SLOs measure from the start of the history, like an SLO over metrics that already exist.
  const row: Row = { slo: { id: `slo-${String(s.seq++).padStart(4, '0')}`, ...v, creator: member(0), createdAt: s.anchor - 90 * DAY, updatedAt: now }, profile: profileFor(v.name) };
  s.slos.push(row);
  return item(row);
}

export async function updateSlo(id: string, input: SloInput): Promise<SloItem> {
  const row = find(id);
  row.slo = { ...row.slo, ...validate(input), updatedAt: Date.now() };
  return item(row);
}

export async function deleteSlo(id: string): Promise<void> {
  const s = db();
  find(id);
  s.slos = s.slos.filter((r) => r.slo.id !== id);
  s.corrections = s.corrections.filter((c) => c.sloId !== id);
}

// ---------------------------------------------------------------- corrections

export async function listCorrections(sloId?: string | null): Promise<SloCorrection[]> {
  return copy(
    db()
      .corrections.filter((c) => !sloId || !c.sloId || c.sloId === sloId)
      .sort((a, b) => b.start - a.start),
  );
}

function validateCorrection(input: SloCorrectionInput): SloCorrectionInput {
  const sloId = input.sloId?.trim() || null;
  const description = (input.description ?? '').trim();
  if (input.end <= input.start) fail('A correction ends after it starts');
  if (input.end - input.start > 90 * DAY) fail('A correction covers at most 90 days');
  if (description.length > MAX_DESCRIPTION_CHARS) fail(`The description is at most ${MAX_DESCRIPTION_CHARS} characters`);
  if (sloId) find(sloId);
  return { ...input, sloId, description };
}

export async function createCorrection(input: SloCorrectionInput): Promise<SloCorrection> {
  const s = db();
  const v = validateCorrection(input);
  const c: SloCorrection = {
    id: `corr-${String(s.corrections.length + 100 + s.seq++).padStart(4, '0')}`,
    sloId: v.sloId ?? null,
    sloName: v.sloId ? find(v.sloId).slo.name : null,
    category: v.category,
    description: v.description ?? '',
    start: v.start,
    end: v.end,
    creator: member(0),
    createdAt: Date.now(),
  };
  s.corrections.push(c);
  return copy(c);
}

export async function updateCorrection(id: string, input: SloCorrectionInput): Promise<SloCorrection> {
  const c = db().corrections.find((x) => x.id === id) ?? fail(`correction ${id} not found`);
  const v = validateCorrection(input);
  Object.assign(c, { sloId: v.sloId ?? null, sloName: v.sloId ? find(v.sloId).slo.name : null, category: v.category, description: v.description ?? '', start: v.start, end: v.end });
  return copy(c);
}

export async function deleteCorrection(id: string): Promise<void> {
  const s = db();
  if (!s.corrections.some((c) => c.id === id)) fail(`correction ${id} not found`);
  s.corrections = s.corrections.filter((c) => c.id !== id);
}
