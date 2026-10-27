// Log queries of integration panels: log counts per facet value, as toplists and sunbursts show them.
import type { FacetValue, MetricSeries, TimeRange } from '../types';

export interface LogPanelQuery {
  name: string;
  data_source: string;
  search?: { query?: string };
  group_by?: { facet: string; limit?: number }[];
  compute?: { aggregation: string; metric?: string };
}

/** The top-values request for a log query: count logs matching `query` per value of `key`. */
export function logRequest(query: LogPanelQuery, resolve: (query: string) => string): { query: string; key: string; limit: number } {
  if ((query.compute?.aggregation ?? 'count') !== 'count') throw new Error(`Log aggregation ${query.compute?.aggregation} is not supported yet (only count)`);
  const group = query.group_by?.[0];
  if (!group || (query.group_by?.length ?? 0) > 1) throw new Error('Log queries need exactly one group-by facet');
  return { query: resolve(query.search?.query ?? ''), key: group.facet, limit: group.limit ?? 10 };
}

/** One series per value, its count at the end of the range (scalar panels read the last point). */
export function logSeries(values: FacetValue[], key: string, range: TimeRange): MetricSeries[] {
  return values.map((v) => ({ name: `${key}:${v.value}`, points: [[range.to, v.count]] }));
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
