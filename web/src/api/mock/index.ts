// In-browser stand-in for the query service. Implements the same contract (types.ts)
// so pages don't change when client.ts switches to fetch()/SSE.
import { parseQuery } from '../../lib/searchQuery';
import type { Facet, FacetValue, HistogramBucket, JsonValue, LiveTailBatch, LogEvent, LogInsight, LogQuery, LogSearchResult, MetricSeries, QuerySuggestion, TimeRange } from '../types';
import { generateLog, rng } from './generator';
import { logField, matchLog } from './match';
import { standardPaths } from './standardAttributes';

const HOUR = 3600_000;
const MAX_STORED = 150_000;

/** Newest-first store of every generated log; the live tail appends to it. */
let store: LogEvent[] | null = null;

function logs(): LogEvent[] {
  if (store) return store;
  const r = rng(42);
  const now = Date.now();
  const out: LogEvent[] = [];
  // Dense recent history so 15m/1h ranges look alive; sparser for 1d/7d.
  const tiers: [count: number, spanMs: number][] = [
    [6000, HOUR],
    [5000, 24 * HOUR],
    [4000, 7 * 24 * HOUR],
  ];
  for (const [count, span] of tiers) {
    for (let i = 0; i < count; i++) out.push(generateLog(r, Math.round(now - r() * span)));
  }
  out.sort((a, b) => b.timestamp - a.timestamp);
  store = out;
  return out;
}

function inRange(range: TimeRange): LogEvent[] {
  return logs().filter((l) => l.timestamp >= range.from && l.timestamp <= range.to);
}

const FACET_DEFS: { key: string; label: string; group: string }[] = [
  { key: 'index', label: 'Index', group: 'CORE' },
  { key: 'source', label: 'Source', group: 'CORE' },
  { key: 'host', label: 'Host', group: 'CORE' },
  { key: 'service', label: 'Service', group: 'CORE' },
  { key: 'status', label: 'Status', group: 'CORE' },
  { key: 'env', label: 'Env', group: 'ENVIRONMENT' },
  { key: 'kube_cluster_name', label: 'Cluster', group: 'ENVIRONMENT' },
  { key: 'region', label: 'Region', group: 'ENVIRONMENT' },
  { key: 'kube_namespace', label: 'Namespace', group: 'KUBERNETES' },
  { key: 'kube_deployment', label: 'Deployment', group: 'KUBERNETES' },
  { key: 'pod_name', label: 'Pod Name', group: 'KUBERNETES' },
  { key: 'container_name', label: 'Container Name', group: 'KUBERNETES' },
  { key: '@http.method', label: 'Method', group: 'HTTP' },
  { key: '@http.status_code', label: 'Status Code', group: 'HTTP' },
];

function bucketSize(range: TimeRange): number {
  const target = (range.to - range.from) / 60; // ~60 bars
  const steps = [5e3, 10e3, 15e3, 30e3, 60e3, 5 * 60e3, 15 * 60e3, 30 * 60e3, HOUR, 3 * HOUR, 6 * HOUR];
  return steps.find((s) => s >= target) ?? 12 * HOUR;
}

export function histogramFor(events: LogEvent[], range: TimeRange): HistogramBucket[] {
  const size = bucketSize(range);
  const start = Math.floor(range.from / size) * size;
  const buckets: HistogramBucket[] = [];
  for (let t = start; t <= range.to; t += size) buckets.push({ t, counts: { error: 0, warn: 0, info: 0, debug: 0 } });
  for (const l of events) {
    const b = buckets[Math.floor((l.timestamp - start) / size)];
    if (b) b.counts[l.status]++;
  }
  return buckets;
}

/** Facets the server counts exactly from its per-minute aggregate (when the search filters only on them). */
const AGGREGATED_FACETS = new Set(['service', 'status', 'host', 'source', 'index', 'env', 'kube_cluster_name', 'region', 'kube_namespace', 'kube_deployment', 'container_name']);
/** Like the server, other facet counts read only the newest logs of the range (scaled, marked approximate). */
const MOCK_FACET_SCAN = 5000;

export function mockSearchLogs(q: LogQuery): LogSearchResult {
  const terms = parseQuery(q.query);
  const scope = inRange(q.range);
  const hits = scope.filter((l) => matchLog(l, terms));
  // `scope` is newest first.
  const scanned = scope.slice(0, MOCK_FACET_SCAN);
  const factor = scope.length / Math.max(scanned.length, 1);
  const exact = (key: string) => AGGREGATED_FACETS.has(key) && terms.every((t) => t.kind === 'tag' && (t.key === key || AGGREGATED_FACETS.has(t.key)));

  // Facet counts ignore the facet's own filter so unchecked values still show counts.
  const facets: Facet[] = FACET_DEFS.map(({ key, label, group }) => {
    const approximate = !exact(key) && scanned.length < scope.length;
    const counts = new Map<string, number>();
    if (key === 'status') for (const s of ['error', 'warn', 'info', 'debug']) counts.set(s, 0);
    for (const l of approximate ? scanned : scope) {
      if (!matchLog(l, terms, key)) continue;
      const v = logField(l, key);
      if (v !== undefined) counts.set(v, (counts.get(v) ?? 0) + 1);
    }
    const order = ['error', 'warn', 'info', 'debug'];
    const values = [...counts.entries()]
      .map(([v, count]) => ({ value: v, count: approximate ? Math.round(count * factor) : count }))
      .sort((a, b) => (key === 'status' ? order.indexOf(a.value) - order.indexOf(b.value) : b.count - a.count));
    return { key, label, group, values, approximate };
  });

  const standard = standardPaths();
  for (const key of q.facets ?? []) {
    if (facets.some((f) => f.key === key)) continue;
    const approximate = scanned.length < scope.length;
    const counts = new Map<string, number>();
    for (const l of approximate ? scanned : scope) {
      const v = matchLog(l, terms, key) ? logField(l, key) : undefined;
      if (v !== undefined) counts.set(v, (counts.get(v) ?? 0) + 1);
    }
    const values = [...counts]
      .map(([value, count]) => ({ value, count: approximate ? Math.round(count * factor) : count }))
      .sort((a, b) => b.count - a.count)
      .slice(0, 10);
    facets.push({ key, label: key.replace(/^@/, ''), group: isStandard(key, standard) ? 'STANDARD ATTRIBUTES' : 'ADDED', values, approximate });
  }
  return { logs: hits, total: hits.length, facets, histogram: histogramFor(hits, q.range), insights: insights(scope) };
}

/** The most frequent values of `key` among the matching logs, every filter applied. */
export function mockTopLogValues(q: LogQuery & { key: string; limit?: number }): FacetValue[] {
  const terms = parseQuery(q.query);
  const counts = new Map<string, number>();
  for (const l of inRange(q.range)) {
    const v = matchLog(l, terms) ? logField(l, q.key) : undefined;
    if (v) counts.set(v, (counts.get(v) ?? 0) + 1);
  }
  return [...counts]
    .map(([value, count]) => ({ value, count }))
    .sort((a, b) => b.count - a.count)
    .slice(0, q.limit ?? 10);
}

/** Logs grouped by message pattern (numbers and ids as `*`), newest 10,000 matching logs. */
export function mockLogPatterns(q: LogQuery): { pattern: string; count: number; status: string; service: string; sample: string }[] {
  const terms = parseQuery(q.query);
  const groups = new Map<string, { count: number; status: string; service: string; sample: string }>();
  for (const l of inRange(q.range)
    .filter((l) => matchLog(l, terms))
    .slice(0, 10_000)) {
    const pattern = l.message
      .split(/\s+/)
      .map((w) => (/^\d|[0-9a-f]{8,}/.test(w) ? '*' : w))
      .join(' ');
    const g = groups.get(pattern) ?? { count: 0, status: l.status, service: l.service, sample: l.message };
    g.count += 1;
    groups.set(pattern, g);
  }
  return [...groups]
    .map(([pattern, g]) => ({ pattern, ...g }))
    .sort((a, b) => b.count - a.count)
    .slice(0, 50);
}

/** Tags and `@attributes` of the newest 10,000 matching logs, most common first. */
/** A facet key that is one of the org's standard attributes (`@http.url`; tags aren't). */
const isStandard = (key: string, standard: Set<string>) => key.startsWith('@') && standard.has(key.slice(1));

/** Attribute paths of nested attributes, as the server flattens them (`http.method`). */
function attributePaths(attrs: Record<string, JsonValue>, prefix = ''): string[] {
  return Object.entries(attrs).flatMap(([k, v]) => {
    const path = prefix ? `${prefix}.${k}` : k;
    return v && typeof v === 'object' && !Array.isArray(v) && Object.keys(v).length ? attributePaths(v, path) : [path];
  });
}

export function mockLogFields(q: LogQuery): { key: string; count: number; standard: boolean }[] {
  const terms = parseQuery(q.query);
  const counts = new Map<string, number>();
  for (const l of inRange(q.range)
    .filter((l) => matchLog(l, terms))
    .slice(0, 10_000)) {
    const keys = [...Object.keys(l.tags ?? {}), ...attributePaths(l.attributes ?? {}).map((k) => `@${k}`)];
    for (const k of keys) counts.set(k, (counts.get(k) ?? 0) + 1);
  }
  const standard = standardPaths();
  return [...counts].map(([key, count]) => ({ key, count, standard: isStandard(key, standard) })).sort((a, b) => b.count - a.count || a.key.localeCompare(b.key));
}

export function mockHistogram(q: LogQuery): HistogramBucket[] {
  const terms = parseQuery(q.query);
  return histogramFor(
    inRange(q.range).filter((l) => matchLog(l, terms)),
    q.range,
  );
}

/** Services whose share of errors is far above their share of traffic. */
function insights(scope: LogEvent[]): LogInsight[] {
  const errors = scope.filter((l) => l.status === 'error');
  if (errors.length < 3) return [];
  const all = new Map<string, number>();
  const err = new Map<string, number>();
  for (const l of scope) all.set(l.service, (all.get(l.service) ?? 0) + 1);
  for (const l of errors) err.set(l.service, (err.get(l.service) ?? 0) + 1);
  return [...err.entries()]
    .map(([service, n]) => ({
      kind: 'error_outlier' as const,
      attribute: 'service',
      value: service,
      shareOfErrors: n / errors.length,
      shareOfLogs: (all.get(service) ?? 0) / scope.length,
    }))
    .filter((i) => i.shareOfErrors > 2 * i.shareOfLogs)
    .sort((a, b) => b.shareOfErrors - a.shareOfErrors)
    .slice(0, 3);
}

const SUGGEST_KEYS = ['service', 'status', 'host', 'source', 'env', 'kube_namespace', 'pod_name', 'container_name', 'workloadname'];

export function mockSuggest(range: TimeRange, partial: string): QuerySuggestion[] {
  const scope = inRange(range);
  const neg = partial.startsWith('-') ? '-' : '';
  const p = partial.slice(neg.length).toLowerCase();
  const colon = p.indexOf(':');

  const countValues = (key: string) => {
    const m = new Map<string, number>();
    for (const l of scope) {
      const v = logField(l, key);
      if (v !== undefined) m.set(v, (m.get(v) ?? 0) + 1);
    }
    return m;
  };

  if (colon > 0) {
    const key = partial.slice(neg.length, neg.length + colon);
    const v = p.slice(colon + 1);
    const values = [...countValues(key).entries()].filter(([val]) => val.toLowerCase().includes(v));
    const total = values.reduce((a, [, c]) => a + c, 0);
    return [
      ...(v === '' ? [{ text: `${neg}${key}:`, count: total }] : []),
      ...values
        .sort((a, b) => b[1] - a[1])
        .slice(0, 6)
        .map(([val, count]) => ({ text: `${neg}${key}:${val}`, count })),
    ];
  }

  const keys = SUGGEST_KEYS.filter((k) => k.startsWith(p))
    .slice(0, p ? 3 : 5)
    .map((k) => ({ text: `${neg}${k}:`, count: scope.filter((l) => logField(l, k) !== undefined).length }));
  if (!p) return keys;

  const pairs: QuerySuggestion[] = [];
  for (const k of SUGGEST_KEYS) {
    for (const [val, count] of countValues(k)) if (val.toLowerCase().includes(p)) pairs.push({ text: `${neg}${k}:${val}`, count });
  }
  return [...keys, ...pairs.sort((a, b) => b.count - a.count).slice(0, 6 - Math.min(keys.length, 2))];
}

const TICK_MS = 250;
const INGEST_PER_SEC = 30;
const DISPLAY_CAP_PER_SEC = 40;

/**
 * Simulated live-tail stream: ingests ~30 events/s (with 6× bursts), keeps the store
 * fresh, and pushes the matching events — sampled down to the display cap — every 250ms.
 */
export function mockLiveTail(query: string, onBatch: (b: LiveTailBatch) => void): () => void {
  const terms = parseQuery(query);
  const r = rng(Date.now() % 100000);
  let matchedRate = 0;
  let last = Date.now();
  let burstUntil = 0;

  const id = setInterval(() => {
    const now = Date.now();
    if (now > burstUntil && r() < 0.01) burstUntil = now + 4000 + r() * 6000;
    const rate = INGEST_PER_SEC * (now < burstUntil ? 6 : 0.6 + r() * 0.8);
    const n = Math.round((rate * (now - last)) / 1000);
    const fresh: LogEvent[] = [];
    for (let i = 0; i < n; i++) fresh.push(generateLog(r, Math.round(last + ((now - last) * (i + r())) / n)));
    fresh.sort((a, b) => a.timestamp - b.timestamp);

    const s = logs();
    s.unshift(...[...fresh].reverse());
    if (s.length > MAX_STORED) s.length = MAX_STORED;

    const matched = fresh.filter((l) => matchLog(l, terms));
    const cap = Math.ceil((DISPLAY_CAP_PER_SEC * (now - last)) / 1000);
    const shown = matched.length > cap ? matched.filter(() => r() < cap / matched.length) : matched;
    matchedRate = matchedRate * 0.8 + ((matched.length * 1000) / (now - last)) * 0.2;
    last = now;
    onBatch({
      events: shown,
      matchedPerSec: matchedRate,
      displayedRatio: matched.length ? shown.length / matched.length : 1,
    });
  }, TICK_MS);
  return () => clearInterval(id);
}

export function mockMetric(name: string, range: TimeRange, groups: string[], base: number, noise: number): MetricSeries[] {
  const step = Math.max(10e3, Math.round((range.to - range.from) / 150));
  return groups.map((g, gi) => {
    const rand = rng(name.length * 97 + gi * 13);
    const points: [number, number][] = [];
    let v = base * (0.6 + gi * 0.25);
    for (let t = range.from; t <= range.to; t += step) {
      v = Math.max(0, v + (rand() - 0.5) * noise + Math.sin(t / 3.6e6 + gi) * noise * 0.15);
      points.push([t, Math.round(v * 100) / 100]);
    }
    return { name: g, points };
  });
}
