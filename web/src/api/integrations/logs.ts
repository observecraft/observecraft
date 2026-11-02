// Log queries of integration panels as Datadog log analytics requests (`/api/v1/logs/aggregate`, the shape of
// Datadog's `/api/v2/logs/analytics/aggregate`): counts, unique counts, sums, averages and percentiles of a
// measure, by none or several facets, as totals or over time.
import type { MetricSeries, TimeRange } from '../types';

export interface LogPanelQuery {
  name: string;
  data_source: string;
  search?: { query?: string };
  /** A list of facets, or Datadog's newer `{fields, limit, sort}`. */
  group_by?: { facet: string; limit?: number; sort?: { order?: string } }[] | { fields?: string[]; limit?: number; sort?: { order?: string } };
  compute?: { aggregation: string; metric?: string; interval?: number | string };
}

export interface LogAggregateRequest {
  compute: { aggregation: string; metric?: string; type: 'total' | 'timeseries'; interval?: string }[];
  filter: { query: string; from: number; to: number };
  group_by: { facet: string; limit?: number; sort?: { order: string } }[];
}

export interface LogAggregateResponse {
  data: { buckets: { by: Record<string, string>; computes: Record<string, number | null | { time: string; value: number | null }[]> }[] };
}

/** The query's group-bys as one entry per facet, whichever form it uses. */
function groupBys(query: LogPanelQuery): { facet: string; limit?: number; sort?: { order?: string } }[] {
  const g = query.group_by;
  if (!g) return [];
  if (Array.isArray(g)) return g;
  return (g.fields ?? []).map((facet) => ({ facet, limit: g.limit, sort: g.sort }));
}

const AGGREGATIONS = new Set(['count', 'cardinality', 'sum', 'min', 'max', 'avg', 'median', 'pc75', 'pc90', 'pc95', 'pc98', 'pc99']);

/** The analytics request of a panel's log query over `range`, as totals or (`timeseries`) over time. */
export function logRequest(query: LogPanelQuery, resolve: (query: string) => string, range: TimeRange, timeseries: boolean): LogAggregateRequest {
  const aggregation = query.compute?.aggregation ?? 'count';
  if (!AGGREGATIONS.has(aggregation)) throw new Error(`Log aggregation ${aggregation} is not supported`);
  const metric = query.compute?.metric;
  const interval = query.compute?.interval;
  return {
    compute: [{ aggregation, ...(metric && aggregation !== 'count' ? { metric } : {}), type: timeseries ? 'timeseries' : 'total', ...(timeseries && interval ? { interval: String(interval) } : {}) }],
    filter: { query: resolve(query.search?.query ?? ''), from: range.from, to: range.to },
    group_by: groupBys(query).map((g) => ({ facet: g.facet, ...(g.limit ? { limit: g.limit } : {}), ...(g.sort?.order ? { sort: { order: g.sort.order } } : {}) })),
  };
}

/** One series per bucket, named by its facets' values (`*` without facets): its total at the end of the range, or its steps. */
export function logSeries(response: LogAggregateResponse, query: LogPanelQuery, range: TimeRange, timeseries: boolean): MetricSeries[] {
  const facets = groupBys(query).map((g) => g.facet);
  return response.data.buckets.map((b) => {
    const name = facets.length ? facets.map((f) => `${f}:${b.by[f] ?? ''}`).join(',') : '*';
    const value = b.computes.c0;
    const points: [number, number][] = timeseries
      ? (Array.isArray(value) ? value : []).filter((p) => p.value !== null).map((p) => [Date.parse(p.time), p.value as number])
      : typeof value === 'number'
        ? [[range.to, value]]
        : [];
    return { name, points };
  });
}

/** `throughput(query1)` → the input `query1__throughput` (per second over the range). */
export function throughputs(text: string): { text: string; names: string[] } {
  const names: string[] = [];
  const rewritten = text.replace(/\bthroughput\(\s*(\w+)\s*\)/g, (_, name: string) => {
    if (!names.includes(name)) names.push(name);
    return `${name}__throughput`;
  });
  return { text: rewritten, names };
}
