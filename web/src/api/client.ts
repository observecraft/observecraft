// Logs data access. In "live" mode (VITE_API_MODE=live) every call goes to the Rust API;
// otherwise the in-browser mock answers, so the UI still runs without a backend.
import { API_MODE, post } from './http';
import type { components } from './generated/schema';
import { mockHistogram, mockLiveTail, mockLogFields, mockLogPatterns, mockTopLogValues, mockMetric, mockSearchLogs, mockSuggest } from './mock';
import type { Facet, FacetValue, HistogramBucket, JsonValue, LiveTailBatch, LogEvent, LogInsight, LogQuery, LogStatus, MetricSeries, QuerySuggestion, TimeRange } from './types';

/** The UI loads at most this many rows per search; aggregate views work on this sample. */
const LIVE_PAGE = 1000;

type WireEvent = components['schemas']['LogEvent'];

function toLogEvent(e: WireEvent): LogEvent {
  return { ...e, status: e.status as LogStatus, attributes: (e.attributes ?? {}) as Record<string, JsonValue> };
}

/** A page of matching logs, newest first, and how many match in all. */
export async function searchLogPage(q: LogQuery, limit = LIVE_PAGE): Promise<{ logs: LogEvent[]; total: number }> {
  if (API_MODE !== 'live') {
    const r = mockSearchLogs(q);
    return { logs: r.logs.slice(0, limit), total: r.total };
  }
  const page = await post('/api/v1/logs/search', { range: q.range, query: q.query, limit });
  return { logs: page.logs.map(toLogEvent), total: page.total };
}

/** Top values per facet (the built-in ones and `q.facets`). */
export async function logFacets(q: LogQuery): Promise<Facet[]> {
  if (API_MODE !== 'live') return mockSearchLogs(q).facets;
  return post('/api/v1/logs/facets', { range: q.range, query: q.query, extra: q.facets ?? [] });
}

/** Services with far more than their share of the errors. */
export async function logInsights(q: LogQuery): Promise<LogInsight[]> {
  if (API_MODE !== 'live') return mockSearchLogs(q).insights;
  const insights = await post('/api/v1/logs/insights', { range: q.range, query: q.query });
  return insights.map((i) => ({ ...i, kind: 'error_outlier' }) satisfies LogInsight);
}

/** The most frequent values of a key among the matching logs (dashboard toplists and sunbursts). */
export async function topLogValues(q: LogQuery & { key: string; limit?: number }): Promise<FacetValue[]> {
  if (API_MODE !== 'live') return mockTopLogValues(q);
  return post('/api/v1/logs/top', { range: q.range, query: q.query, key: q.key, limit: q.limit });
}

export type LogPattern = components['schemas']['LogPattern'];

/** The matching logs grouped by pattern (Log Explorer → Group into Patterns). */
export async function logPatterns(q: LogQuery): Promise<LogPattern[]> {
  if (API_MODE !== 'live') return mockLogPatterns(q);
  return post('/api/v1/logs/patterns', { range: q.range, query: q.query });
}

export type LogField = components['schemas']['LogField'];

/** The tags and attributes of the matching logs, to add as facets. */
export async function logFields(q: LogQuery): Promise<LogField[]> {
  if (API_MODE !== 'live') return mockLogFields(q);
  return post('/api/v1/logs/fields', { range: q.range, query: q.query });
}

/** Status-stacked counts only — cheaper than a full search, used by Live Tail. */
export async function logHistogram(q: LogQuery): Promise<HistogramBucket[]> {
  if (API_MODE !== 'live') return mockHistogram(q);
  return post('/api/v1/logs/histogram', { range: q.range, query: q.query });
}

/** Autocomplete for the query bar: keys ("service:") and key:value pairs with counts. */
export async function suggestLogQuery(range: TimeRange, partial: string): Promise<QuerySuggestion[]> {
  if (API_MODE !== 'live') return mockSuggest(range, partial);
  return post('/api/v1/logs/suggest', { range, partial });
}

/** Opens a live-tail stream (Server-Sent Events) for a query. Returns an unsubscribe function. */
export function subscribeLiveTail(query: string, onBatch: (b: LiveTailBatch) => void): () => void {
  if (API_MODE !== 'live') return mockLiveTail(query, onBatch);
  const es = new EventSource(`/api/v1/logs/live?query=${encodeURIComponent(query)}`);
  es.addEventListener('batch', (e) => {
    const b = JSON.parse((e as MessageEvent<string>).data) as components['schemas']['LiveTailBatch'];
    onBatch({ events: b.events.map(toLogEvent), matchedPerSec: b.matchedPerSec, displayedRatio: b.displayedRatio });
  });
  // EventSource reconnects on its own after network errors.
  return () => es.close();
}

export async function queryMetric(name: string, range: TimeRange, groups: string[], opts: { base: number; noise: number }): Promise<MetricSeries[]> {
  return mockMetric(name, range, groups, opts.base, opts.noise);
}
