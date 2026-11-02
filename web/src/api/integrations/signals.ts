// Security signal queries of integration panels (`data_source: security_signals`): signal counts from the
// Cloud SIEM signal search, in total, per facet value, or over time per severity.
import type { MetricSeries, TimeRange } from '../types';

export interface SignalPanelQuery {
  name: string;
  data_source: string;
  search?: { query?: string };
  compute?: { aggregation: string };
  group_by?: { facet: string }[] | { fields?: string[] };
}

/** What the signal search returns that a panel reads. */
export interface SignalCounts {
  total: number;
  facets: { key: string; values: { value: string; count: number }[] }[];
  histogram: { ts: number; info: number; low: number; medium: number; high: number; critical: number }[];
}

const SEVERITIES = ['info', 'low', 'medium', 'high', 'critical'] as const;

/** The panel query's facet (the first group-by), whichever form it uses. */
export function signalFacet(query: SignalPanelQuery): string | undefined {
  const g = query.group_by;
  return Array.isArray(g) ? g[0]?.facet : g?.fields?.[0];
}

/** One series per facet value (`*` without a facet): the count at the end of the range, or per step. */
export function signalSeries(page: SignalCounts, query: SignalPanelQuery, range: TimeRange, timeseries: boolean): MetricSeries[] {
  if ((query.compute?.aggregation ?? 'count') !== 'count') throw new Error(`Signal aggregation ${query.compute?.aggregation} is not supported (only count)`);
  const facet = signalFacet(query);
  if (timeseries) {
    if (!facet) return [{ name: '*', points: page.histogram.map((b) => [b.ts, SEVERITIES.reduce((n, s) => n + b[s], 0)]) }];
    if (facet !== 'status') throw new Error(`Signals over time can be split by status only, not ${facet}`);
    return SEVERITIES.filter((s) => page.histogram.some((b) => b[s] > 0)).map((s) => ({ name: `status:${s}`, points: page.histogram.map((b) => [b.ts, b[s]]) }));
  }
  if (!facet) return [{ name: '*', points: [[range.to, page.total]] }];
  const values = page.facets.find((f) => f.key === facet);
  if (!values) throw new Error(`Signals can't be counted by ${facet} yet`);
  return values.values.map((v) => ({ name: `${facet}:${v.value}`, points: [[range.to, v.count]] }));
}
