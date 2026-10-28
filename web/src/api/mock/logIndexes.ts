// In-browser log indexes and org data retention for mock mode (Logs → Indexes, Organization
// Settings → Data Retention). State lives in memory for the session; validation follows the
// server (routes/log_indexes.rs): lowercase index names, retention within the org's ceilings,
// metric retention picked from fixed tiers in order (raw ≤ 5m ≤ 1h).
import type { components } from '../generated/schema';

type S = components['schemas'];

const TIERS = [7, 15, 30, 90, 180, 395, 760];
const fail = (message: string): never => {
  throw new Error(message);
};

interface State {
  indexes: S['LogIndex'][];
  settings: S['OrgDataSettings'];
  seq: number;
}

let state: State | null = null;

function db(): State {
  if (state) return state;
  const filter = (name: string, query: string, sampleRate: number, enabled = true): S['ExclusionFilter'] => ({ name, query, sampleRate, enabled });
  state = {
    indexes: [
      { id: 'ix-0001', name: 'billing-audit', filter: 'service:billing', retentionDays: 30, dailyQuota: null, quotaResetHour: 0, exclusionFilters: [], position: 0, indexedToday: 48_210 },
      {
        id: 'ix-0002',
        name: 'checkout',
        filter: 'service:(order-service OR inventory-service)',
        retentionDays: 15,
        dailyQuota: 2_000_000,
        quotaResetHour: 0,
        exclusionFilters: [filter('Health checks', '@http.url_details.path:/healthz', 100)],
        position: 1,
        indexedToday: 412_903,
      },
      {
        id: 'ix-0003',
        name: 'main',
        filter: '',
        retentionDays: 15,
        dailyQuota: 10_000_000,
        quotaResetHour: 0,
        exclusionFilters: [filter('Debug logs', 'status:debug', 90), filter('api-gateway access logs', 'service:api-gateway status:info', 50, false)],
        position: 2,
        indexedToday: 1_874_550,
      },
    ],
    settings: {
      spanRetentionDays: 15,
      maxSpanRetentionDays: 30,
      maxLogRetentionDays: 30,
      maxIndexes: 10,
      metrics: { raw: 15, rollup5m: 90, rollup1h: 395 },
      maxMetrics: { raw: 30, rollup5m: 180, rollup1h: 760 },
      metricRetentionTiers: TIERS,
      ciRetentionDays: 90,
      rawLogsHours: 48,
    },
    seq: 4,
  };
  return state;
}

const copy = <T>(v: T): T => structuredClone(v);
const ordered = () => [...db().indexes].sort((a, b) => a.position - b.position);

export async function listIndexes(): Promise<S['LogIndexes']> {
  const s = db();
  const indexes = ordered();
  return {
    indexes: copy(indexes),
    ingestedToday: Math.round(indexes.reduce((n, i) => n + i.indexedToday, 0) * 1.35),
    maxRetentionDays: s.settings.maxLogRetentionDays,
    maxIndexes: s.settings.maxIndexes,
  };
}

function validate(input: S['LogIndexInput'], exceptId?: string): Omit<S['LogIndex'], 'id' | 'position' | 'indexedToday'> {
  const name = input.name.trim().toLowerCase();
  if (!name || name.length > 64 || !/^[a-z0-9_-]+$/.test(name) || name === 'rehydrated') fail('Index names are 1–64 lowercase letters, digits, dashes or underscores (not "rehydrated")');
  const max = db().settings.maxLogRetentionDays;
  if (!(input.retentionDays >= 1 && input.retentionDays <= max)) fail(`Retention must be between 1 and ${max} days for this organization`);
  if (input.dailyQuota != null && input.dailyQuota <= 0) fail('The daily quota must be positive (or empty for no limit)');
  const hour = input.quotaResetHour ?? 0;
  if (hour < 0 || hour > 23) fail('The quota reset hour is 0–23 (UTC)');
  const filters = input.exclusionFilters ?? [];
  if (filters.length > 20) fail('At most 20 exclusion filters per index');
  if (filters.some((f) => !Number.isFinite(f.sampleRate) || f.sampleRate < 0 || f.sampleRate > 100)) fail('Exclusion rates are 0–100 %');
  if (db().indexes.some((i) => i.name === name && i.id !== exceptId)) fail(`An index named '${name}' already exists`);
  return {
    name,
    filter: (input.filter ?? '').trim(),
    retentionDays: input.retentionDays,
    dailyQuota: input.dailyQuota ?? null,
    quotaResetHour: hour,
    exclusionFilters: filters.map((f) => ({ ...f, name: [...f.name.trim()].slice(0, 100).join('') })),
  };
}

export async function createIndex(input: S['LogIndexInput']): Promise<S['LogIndex']> {
  const s = db();
  const fields = validate(input);
  if (s.indexes.length >= s.settings.maxIndexes) fail(`This organization can have at most ${s.settings.maxIndexes} indexes`);
  const ix: S['LogIndex'] = { id: `ix-${String(s.seq++).padStart(4, '0')}`, ...fields, position: s.indexes.reduce((m, i) => Math.max(m, i.position), -1) + 1, indexedToday: 0 };
  s.indexes.push(ix);
  return copy(ix);
}

export async function updateIndex(id: string, input: S['LogIndexInput']): Promise<S['LogIndex']> {
  const ix = db().indexes.find((i) => i.id === id) ?? fail('Index not found');
  Object.assign(ix, validate(input, id));
  return copy(ix);
}

export async function deleteIndex(id: string): Promise<void> {
  const s = db();
  if (!s.indexes.some((i) => i.id === id)) fail('Index not found');
  s.indexes = s.indexes.filter((i) => i.id !== id);
}

export async function reorderIndexes(ids: string[]): Promise<void> {
  ids.forEach((id, position) => {
    const ix = db().indexes.find((i) => i.id === id);
    if (ix) ix.position = position;
  });
}

export async function dataSettings(): Promise<S['OrgDataSettings']> {
  return copy(db().settings);
}

export async function updateDataSettings(p: { spanRetentionDays?: number; metrics?: S['MetricRetention'] }): Promise<S['OrgDataSettings']> {
  const s = db().settings;
  if (p.spanRetentionDays != null && !(p.spanRetentionDays >= 1 && p.spanRetentionDays <= s.maxSpanRetentionDays))
    fail(`Span retention must be between 1 and ${s.maxSpanRetentionDays} days for this organization`);
  if (p.metrics) {
    const m = p.metrics;
    const lim = s.maxMetrics;
    if ([m.raw, m.rollup5m, m.rollup1h].some((d) => !TIERS.includes(d))) fail(`Metric retention must be one of ${TIERS.join(', ')} days`);
    if (m.raw > m.rollup5m || m.rollup5m > m.rollup1h) fail("Metrics can't be kept longer at a finer resolution: full resolution ≤ 5-minute ≤ 1-hour rollups");
    if (m.raw > lim.raw || m.rollup5m > lim.rollup5m || m.rollup1h > lim.rollup1h) fail(`Metric retention can be up to ${lim.raw} / ${lim.rollup5m} / ${lim.rollup1h} days for this organization`);
  }
  if (p.spanRetentionDays != null) s.spanRetentionDays = p.spanRetentionDays;
  if (p.metrics) s.metrics = { ...p.metrics };
  return copy(s);
}
