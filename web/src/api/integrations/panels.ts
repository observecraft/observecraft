import type { PanelDataSource } from './panelQueries';
import { metricCatalog, queryTimeseries } from '../dashboards/client';
import type { MetricSeries, TimeRange } from '../types';
import { aggregateLogs } from '../client';
import { searchSignals } from '../security/client';
import { checkStatusQuery } from './checkStatus';
import { signalFacet, signalSeries, type SignalPanelQuery } from './signals';
import { logRequest, logSeries, throughputs, type LogPanelQuery } from './logs';
import { aggregate, canonicalMetricQuery, formula, formulaUnit, normalizeRequest, preparePanelQuery, serverAggregator, timeShifts, type IntegrationWidgetDefinition, type IntegrationFormula } from './evaluate';

export interface PanelSeries extends MetricSeries {
  column?: string;
  group?: string;
  display: string;
  dashed: boolean;
  /** The request's palette name (dog_classic, cool, warm, purple, …). */
  palette?: string;
}

/** Formula functions of a series over time: a scalar panel evaluates them on the points, then reduces the result. */
const OVER_TIME = /\b(?:derivative|rate|dt|diff|monotonic_diff|cumsum|autosmooth|ewma_\d+|per_second|per_minute|per_hour)\(/;

/** Datadog's `percentile` aggregator of a percentile query (`p95:latency{*}`, read as `latency.p95`) is that percentile of its values. */
function percentileOf(aggregator: string, query = ''): string {
  return aggregator === 'percentile' ? (/\.(p\d+)\{/.exec(canonicalMetricQuery(query))?.[1] ?? 'max') : aggregator;
}

export async function queryIntegrationPanel(
  def: IntegrationWidgetDefinition,
  resolve: (query: string) => string,
  range: TimeRange,
  source: PanelDataSource = { catalog: metricCatalog, timeseries: queryTimeseries },
): Promise<PanelSeries[]> {
  if (def.type === 'check_status') {
    const query = checkStatusQuery(def, resolve);
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
    for (const query of normalizeRequest(request).queries) {
      if (query.data_source === 'logs') logRequest(query as unknown as LogPanelQuery, resolve, range, !scalar);
      else if (query.data_source === 'metrics') preparePanelQuery(query.query, resolve, range);
      else if (query.data_source === 'security_signals') continue;
      else throw new Error(`Data source ${query.data_source} is not supported by this panel yet`);
    }
  }
  const catalog = await source.catalog();
  return (
    await Promise.all(
      def.requests.map(async (request) => {
        // A legacy `q` is query1; a function around a metric query (top(…)) becomes a formula.
        const { queries, formulas: requestFormulas } = normalizeRequest(request);
        const formulas: IntegrationFormula[] = (requestFormulas ?? queries.map((q) => ({ formula: q.name }))).map((f) => ({ ...f, formula: throughputs(timeShifts(f.formula).text).text }));
        const shifts = (requestFormulas ?? []).flatMap((f) => timeShifts(f.formula).shifts);
        const perSecond = [...new Set((requestFormulas ?? []).flatMap((f) => throughputs(f.formula).names))];
        const overTime = scalar && formulas.some((f) => OVER_TIME.test(f.formula));
        const aggregatorOf = (name: string) => queries.find((q) => q.name === name)?.aggregator ?? request.aggregator ?? 'avg';
        const inputs: Record<string, MetricSeries[]> = {};
        // A query over `range` moved back by `seconds`, its points moved onto `range` (time comparisons).
        // The dashboard's shared stream only has the current range, so earlier ranges are fetched directly.
        const load = async (query: (typeof queries)[number], seconds: number): Promise<MetricSeries[]> => {
          const shifted = { from: range.from - seconds * 1000, to: range.to - seconds * 1000 };
          const from = seconds ? { catalog: source.catalog, timeseries: queryTimeseries } : source;
          return (await loadQuery(query, shifted, from)).map((s) => ({ ...s, points: s.points.map(([t, v]) => [t + seconds * 1000, v]) }));
        };
        const loadQuery = async (query: (typeof queries)[number], range: TimeRange, source: PanelDataSource): Promise<MetricSeries[]> => {
          if (query.data_source === 'security_signals') {
            const panelQuery = query as unknown as SignalPanelQuery;
            const facet = signalFacet(panelQuery);
            const page = await searchSignals({ range, query: resolve(panelQuery.search?.query ?? ''), state: '', limit: 1, offset: 0, ascending: false, facets: facet ? [facet] : [] });
            return signalSeries(page, panelQuery, range, !scalar);
          }
          if (query.data_source === 'logs') {
            const panelQuery = query as unknown as LogPanelQuery;
            return logSeries(await aggregateLogs(logRequest(panelQuery, resolve, range, !scalar)), panelQuery, range, !scalar);
          }
          const { plan, text, points } = preparePanelQuery(query.query, resolve, range);
          let series: MetricSeries[];
          try {
            // A scalar panel's query arrives as one value per series from the dashboard stream (the
            // aggregate below leaves it as it is); other sources send the points.
            const aggregator = scalar && !overTime ? serverAggregator(plan.operations, query.aggregator ?? request.aggregator) : undefined;
            series = await source.timeseries({ queries: [text], range, points, ...(aggregator ? { aggregator } : {}) });
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
          // fill(0) is fill(zero).
          if (fill?.args[0] === '0') fill.args[0] = 'zero';
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
          return series;
        };
        await Promise.all([
          ...queries.map(async (query) => (inputs[query.name] = await load(query, 0))),
          ...shifts.map(async (shift) => {
            const query = queries.find((q) => q.name === shift.query);
            if (!query) throw new Error(`Unknown formula query: ${shift.query}`);
            inputs[shift.name] = await load(query, shift.seconds);
          }),
        ]);
        // throughput(query): the query's counts per second over the range.
        for (const name of perSecond) {
          if (!inputs[name]) throw new Error(`Unknown formula query: ${name}`);
          inputs[`${name}__throughput`] = inputs[name].map((s) => ({
            ...s,
            unit: s.unit ? `${s.unit}/second` : '1/second',
            points: s.points.map(([t, v]) => [t, (v * 1000) / (range.to - range.from)]),
          }));
        }
        // Scalar requests aggregate each named query before evaluating its formulas.
        if (scalar && !overTime) {
          for (const [name, series] of Object.entries(inputs)) {
            const query = queries.find((q) => q.name === name.split('__')[0]);
            inputs[name] = series.flatMap((s) => {
              const value = aggregate(
                s.points.map((p) => p[1]),
                percentileOf(query?.aggregator ?? request.aggregator ?? 'avg', query?.query),
              );
              return value === undefined ? [] : [{ ...s, name: s.name, points: [[range.to, value]] }];
            });
          }
        }
        const evaluated = (f: IntegrationFormula) => {
          const result = formula(f.formula, inputs);
          if (!overTime) return result;
          // Reduced with the aggregator of the formula's first query, as its scalar value.
          const first = queries.find((q) => new RegExp(`\\b${q.name}\\b`).test(f.formula));
          const method = percentileOf(aggregatorOf(first?.name ?? ''), first?.query);
          return result.flatMap((s) => {
            const value = aggregate(
              s.points.map((p) => p[1]),
              method,
            );
            return value === undefined ? [] : [{ ...s, points: [[range.to, value]] as [number, number][] }];
          });
        };
        return formulas.flatMap((f) =>
          evaluated(f).map((s) => ({
            ...s,
            unit: formulaUnit(f) ?? s.unit,
            column: f.alias ?? f.formula,
            group: s.name,
            name: f.alias ? `${f.alias}${s.name === '*' ? '' : ` · ${s.name}`}` : s.name === '*' ? f.formula : s.name,
            display: request.display_type ?? 'line',
            dashed: request.style?.line_type === 'dashed',
            palette: request.style?.palette,
          })),
        );
      }),
    )
  ).flat();
}
