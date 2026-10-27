import type { GroupedScalar, MetricSeries, TimeAggregator } from './types';

export function reduceMetricValues(values: number[], aggregator: TimeAggregator, rounded = true): number {
  if (!values.length) return 0;
  const v =
    aggregator === 'last'
      ? values[values.length - 1]
      : aggregator === 'sum'
        ? values.reduce((a, b) => a + b, 0)
        : aggregator === 'min'
          ? Math.min(...values)
          : aggregator === 'max'
            ? Math.max(...values)
            : values.reduce((a, b) => a + b, 0) / values.length;
  return rounded ? Math.round(v * 10000) / 10000 : v;
}

export function groupedMetricValues(series: MetricSeries[], query: string, aggregator: TimeAggregator): GroupedScalar[] {
  return series
    .map((s) => {
      const tags =
        s.tags ??
        Object.fromEntries(
          s.name === query || s.name === '*'
            ? []
            : s.name.split(', ').map((part) => {
                const i = part.indexOf(':');
                return [part.slice(0, i), part.slice(i + 1)];
              }),
        );
      return {
        group: Object.keys(tags).length ? s.name : '*',
        tags,
        value: reduceMetricValues(
          s.points.map((p) => p[1]),
          aggregator,
        ),
      };
    })
    .sort((a, b) => b.value - a.value);
}

export function scalarMetricValue(series: MetricSeries[], query: string, aggregator: TimeAggregator): number {
  const byTime = new Map<number, number[]>();
  for (const s of series)
    for (const [t, v] of s.points) {
      const values = byTime.get(t) ?? [];
      values.push(v);
      byTime.set(t, values);
    }
  const space = query.slice(0, query.indexOf(':')) as TimeAggregator;
  return reduceMetricValues(
    [...byTime].sort(([a], [b]) => a - b).map(([, vs]) => reduceMetricValues(vs, space, false)),
    aggregator,
  );
}
