import type { Widget, MetricSeries, TimeAggregator } from './types';
import { normalizeRequest, preparePanelQuery, serverAggregator } from '../integrations/evaluate';
import type { TimeRange } from '../types';

export interface DashboardMetricQuery {
  id: string;
  query: string;
  points: number;
  /** One value per series, reduced on the server (tables, treemaps, toplists). */
  aggregator?: TimeAggregator;
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
/** A series as the dashboard endpoints send it: `points`, or `values` every `step_ms` from `start` (null where empty). */
interface WireSeries extends Omit<MetricSeries, 'points'> {
  points?: [number, number][];
  start?: number | null;
  values?: (number | null)[] | null;
}
type WireFrame = Omit<DashboardQueryFrame, 'results'> & { results: (Omit<DashboardQueryResult, 'series'> & { series: WireSeries[] })[] };

/** A frame as received, with every series as points. */
export function decodeDashboardFrame(frame: WireFrame): DashboardQueryFrame {
  return {
    ...frame,
    results: (frame.results ?? []).map((r) => ({
      ...r,
      series: r.series.map(({ start, values, points, ...s }) => ({
        ...s,
        points: start != null && values ? values.flatMap((v, i): [number, number][] => (v === null ? [] : [[start + i * r.step_ms, v]])) : (points ?? []),
      })),
    })),
  };
}

export const metricQueryKey = (query: string, points = 120, aggregator?: TimeAggregator) => JSON.stringify(aggregator ? [query, points, aggregator] : [query, points]);

/** Build the complete metric plan before mounting panels, including nested/collapsed groups. */
export function collectDashboardQueries(widgets: Widget[], resolve: (query: string) => string, range: TimeRange): DashboardMetricQuery[] {
  const queries = new Map<string, { query: string; points: number; aggregator?: TimeAggregator }>();
  const add = (query: string, points = 120, aggregator?: TimeAggregator) => queries.set(metricQueryKey(query, points, aggregator), { query, points, ...(aggregator ? { aggregator } : {}) });
  for (const { definition: d } of widgets) {
    if (d.type === 'group') {
      for (const q of collectDashboardQueries(d.widgets, resolve, range)) add(q.query, q.points, q.aggregator);
    } else if (d.type === 'integration') {
      const def = d.original;
      try {
        if (def.type === 'check_status') {
          const tags = Array.isArray(def.tags) ? def.tags.filter((t): t is string => typeof t === 'string') : [];
          add(resolve(`max:service_check.status{check:${typeof def.check === 'string' ? def.check : ''},${tags.join(',')}} by {host}`));
        } else if (def.type !== 'list_stream') {
          // Panels other than timeseries show one value per series (panels.ts aggregates them).
          const scalar = def.type !== 'timeseries';
          const planned = (def.requests ?? []).flatMap((r) =>
            normalizeRequest(r).queries.map((q) => {
              if (q.data_source !== 'metrics') throw new Error('Unsupported data source');
              const prepared = preparePanelQuery(q.query, resolve, range);
              return { ...prepared, aggregator: scalar ? serverAggregator(prepared.plan.operations, q.aggregator ?? r.aggregator) : undefined };
            }),
          );
          for (const q of planned) {
            add(q.text, q.points, q.aggregator);
            // A query value's timeseries background draws the same queries over time.
            if (def.type === 'query_value' && def.timeseries_background) add(q.text, q.points);
          }
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
  return [...queries.values()].sort((a, b) => metricQueryKey(a.query, a.points, a.aggregator).localeCompare(metricQueryKey(b.query, b.points, b.aggregator))).map((q, i) => ({ id: `q${i}`, ...q }));
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
