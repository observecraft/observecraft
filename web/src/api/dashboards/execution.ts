import type { Widget, MetricSeries } from './types';
import { preparePanelQuery } from '../integrations/evaluate';
import type { TimeRange } from '../types';

export interface DashboardMetricQuery {
  id: string;
  query: string;
  points: number;
}
export interface DashboardQueryResult {
  id: string;
  series: MetricSeries[];
  kind: string;
  unit?: string | null;
  step_ms: number;
  replace_from: number;
  error?: string | null;
}
export interface DashboardQueryFrame {
  results: DashboardQueryResult[];
  range: TimeRange;
  data_scans: number;
  unique_queries: number;
  error?: string | null;
}
export const metricQueryKey = (query: string, points = 120) => JSON.stringify([query, points]);

/** Build the complete metric plan before mounting panels, including nested/collapsed groups. */
export function collectDashboardQueries(widgets: Widget[], resolve: (query: string) => string, range: TimeRange): DashboardMetricQuery[] {
  const queries = new Map<string, { query: string; points: number }>();
  const add = (query: string, points = 120) => queries.set(metricQueryKey(query, points), { query, points });
  for (const { definition: d } of widgets) {
    if (d.type === 'group') {
      for (const q of collectDashboardQueries(d.widgets, resolve, range)) add(q.query, q.points);
    } else if (d.type === 'integration') {
      const def = d.original;
      try {
        if (def.type === 'check_status') {
          const tags = Array.isArray(def.tags) ? def.tags.filter((t): t is string => typeof t === 'string') : [];
          add(resolve(`max:service_check.status{check:${typeof def.check === 'string' ? def.check : ''},${tags.join(',')}} by {host}`));
        } else if (def.type !== 'list_stream') {
          const planned = (def.requests ?? [])
            .flatMap((r) => r.queries ?? (r.q ? [{ query: r.q, data_source: 'metrics' }] : []))
            .map((q) => {
              if (q.data_source !== 'metrics') throw new Error('Unsupported data source');
              return preparePanelQuery(q.query, resolve, range);
            });
          for (const q of planned) add(q.text, q.points);
        }
      } catch {
        /* The renderer displays the unsupported-expression error without network traffic. */
      }
    } else if (d.type === 'timeseries') {
      d.requests.forEach((r) => add(resolve(r.query)));
    } else if (d.type === 'table') {
      d.requests.forEach((r) => add(resolve(r.query), 60));
    } else if (['query_value', 'toplist', 'bar_chart', 'pie', 'treemap', 'change'].includes(d.type) && 'request' in d) {
      add(resolve(d.request.query), 60);
    }
  }
  return [...queries.values()].sort((a, b) => metricQueryKey(a.query, a.points).localeCompare(metricQueryKey(b.query, b.points))).map((q, i) => ({ id: `q${i}`, ...q }));
}

/** Replace whole aligned tail buckets; addition alone would retain vanished groups/stale points. */
export function mergeDashboardFrame(previous: DashboardQueryFrame | undefined, incoming: DashboardQueryFrame, snapshot: boolean): DashboardQueryFrame {
  if (snapshot || !previous) return incoming;
  const old = new Map(previous.results.map((r) => [r.id, r]));
  return {
    ...incoming,
    results: incoming.results.map((update) => {
      const before = old.get(update.id);
      if (!before) return update;
      if (update.error) return { ...update, series: before.series };
      const series = new Map(
        before.series.map((s) => [s.name, { ...s, points: s.points.filter(([t]) => t >= Math.floor(incoming.range.from / update.step_ms) * update.step_ms && t < update.replace_from) }]),
      );
      for (const s of update.series) {
        const existing = series.get(s.name);
        series.set(s.name, { ...s, points: [...(existing?.points ?? []), ...s.points] });
      }
      return { ...update, series: [...series.values()].filter((s) => s.points.length) };
    }),
  };
}
