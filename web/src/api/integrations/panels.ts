import type { PanelDataSource } from './panelQueries';
import { metricCatalog, queryTimeseries } from '../dashboards/client';
import type { MetricSeries, TimeRange } from '../types';
import { aggregate, formula, preparePanelQuery, type IntegrationWidgetDefinition, type IntegrationFormula } from './evaluate';

export interface PanelSeries extends MetricSeries {
  column?: string;
  group?: string;
  display: string;
  dashed: boolean;
}

export async function queryIntegrationPanel(
  def: IntegrationWidgetDefinition,
  resolve: (query: string) => string,
  range: TimeRange,
  source: PanelDataSource = { catalog: metricCatalog, timeseries: queryTimeseries },
): Promise<PanelSeries[]> {
  if (def.type === 'check_status') {
    const tags = Array.isArray(def.tags) ? def.tags.filter((t): t is string => typeof t === 'string') : [];
    const check = typeof def.check === 'string' ? def.check : '';
    const query = resolve(`max:service_check.status{check:${check},${tags.join(',')}} by {host}`);
    try {
      return (await source.timeseries({ queries: [query], range, points: 120 })).map((s) => ({ ...s, display: 'line', dashed: false }));
    } catch (error) {
      if (error instanceof Error && error.message.startsWith('Unknown metric:')) return [];
      throw error;
    }
  }
  if (!Array.isArray(def.requests)) throw new Error(`Panel type ${def.type} requires an additional data source`);
  const scalar = def.type !== 'timeseries';
  // Reject unsupported expressions before waiting on any network request.
  for (const request of def.requests) {
    for (const query of request.queries ?? (request.q ? [{ query: request.q }] : [])) preparePanelQuery(query.query, resolve, range);
  }
  const catalog = await source.catalog();
  return (
    await Promise.all(
      def.requests.map(async (request) => {
        const queries = request.queries ?? (request.q ? [{ name: 'query1', query: request.q, data_source: 'metrics' }] : []);
        const inputs: Record<string, MetricSeries[]> = {};
        await Promise.all(
          queries.map(async (query) => {
            if (query.data_source !== 'metrics') throw new Error(`Data source ${query.data_source} is not supported by this panel yet`);
            const { plan, text, points } = preparePanelQuery(query.query, resolve, range);
            let series: MetricSeries[];
            try {
              series = await source.timeseries({ queries: [text], range, points });
            } catch (error) {
              if (error instanceof Error && error.message.startsWith('Unknown metric:')) series = [];
              else throw error;
            }
            if (!text.includes(' by ')) series = series.map((s) => ({ ...s, name: '*' }));
            const name = /:([^{}]+)\{/.exec(plan.query)?.[1];
            const metadata = catalog.find((m) => m.name === name);
            const kind = metadata?.kind;
            series = series.map((s) => ({ ...s, unit: metadata?.unit }));
            const counter =
              kind === 'sum' ||
              kind === 'counter' ||
              kind === 'delta' ||
              kind === 'span' ||
              (['histogram', 'exponential_histogram', 'summary'].includes(kind ?? '') && /\.(count|sum)$/.test(name ?? ''));
            if (plan.operations.some((o) => o.name === 'as_count') && counter) {
              series = series.map((s) => {
                const step = s.points.length > 1 ? (s.points[1][0] - s.points[0][0]) / 1000 : Math.max(30, (range.to - range.from) / points / 1000);
                return { ...s, unit: s.unit?.replace(/\/s$/, ''), points: s.points.map(([t, v]) => [t, v * step]) };
              });
            }
            const fill = plan.operations.find((o) => o.name === 'fill');
            if (fill && !['null', 'zero', 'last'].includes(fill.args[0])) throw new Error(`Fill ${fill.args[0]} is not supported yet`);
            if (fill && fill.args[0] !== 'null') {
              series = series.map((s) => {
                if (s.points.length < 2) return s;
                const step = Math.min(...s.points.slice(1).map(([t], i) => t - s.points[i][0]));
                const limit = Number(fill.args[1] || 300) * 1000;
                const values = new Map(s.points);
                const out: [number, number][] = [];
                let last = s.points[0];
                for (let t = s.points[0][0]; t <= range.to; t += step) {
                  const v = values.get(t);
                  if (v !== undefined) {
                    last = [t, v];
                    out.push(last);
                  } else if (t - last[0] <= limit) out.push([t, fill.args[0] === 'zero' ? 0 : last[1]]);
                }
                return { ...s, points: out };
              });
            }
            // Scalar requests aggregate each named query before evaluating its formulas.
            inputs[query.name] = scalar
              ? series.flatMap((s) => {
                  const value = aggregate(
                    s.points.map((p) => p[1]),
                    query.aggregator ?? request.aggregator ?? 'avg',
                  );
                  return value === undefined ? [] : [{ ...s, name: s.name, points: [[range.to, value]] }];
                })
              : series;
          }),
        );
        const formulas: IntegrationFormula[] = request.formulas ?? queries.map((q) => ({ formula: q.name }));
        return formulas.flatMap((f) =>
          formula(f.formula, inputs).map((s) => ({
            ...s,
            column: f.alias ?? f.formula,
            group: s.name,
            name: f.alias ? `${f.alias}${s.name === '*' ? '' : ` · ${s.name}`}` : s.name === '*' ? f.formula : s.name,
            display: request.display_type ?? 'line',
            dashed: request.style?.line_type === 'dashed',
          })),
        );
      }),
    )
  ).flat();
}
