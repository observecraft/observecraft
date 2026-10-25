import { useMemo } from 'react';
import type { EChartsOption } from 'echarts';
import type { SpanMeasure, SpanQuery } from '../../../api/apm/client';
import { Chart } from '../../../components/charts/Chart';
import { ErrorState, Loading } from '../../../components/ui/AsyncState';
import { palette } from '../../../theme/chartTheme';
import { useSpanAnalytics } from '../queries';
import { fmtLatency } from '../visuals';

type Viz = 'timeseries' | 'toplist' | 'bars' | 'table' | 'treemap' | 'pie' | 'list';

/** Analytics views over the same span query: series over time or one value per group. */
export function Visualization({ viz, q, groupBy, measure, onPick }: { viz: Viz; q: SpanQuery; groupBy: string | null; measure: SpanMeasure; onPick: (dim: string, value: string) => void }) {
  const a = useSpanAnalytics(q, groupBy, measure, viz === 'table' || viz === 'treemap' ? 25 : 10, true);
  const isDuration = !['count', 'errors'].includes(measure);
  const fmt = (v: number) => (isDuration ? fmtLatency(v) : v.toLocaleString(undefined, { maximumFractionDigits: 2 }));
  const option = useMemo<EChartsOption | null>(() => {
    const d = a.data;
    if (!d) return null;
    const colors = palette();
    if (viz === 'timeseries' || viz === 'bars') {
      return {
        grid: { left: 60, right: 16, top: 34, bottom: 26 },
        legend: { top: 0, type: 'scroll' },
        tooltip: { trigger: 'axis', valueFormatter: (v) => fmt(v as number) },
        xAxis: { type: 'time' },
        yAxis: { type: 'value', axisLabel: { formatter: (v: number) => fmt(v) } },
        series: d.series.map((s, i) => ({
          name: s.group,
          type: viz === 'bars' ? 'bar' : 'line',
          stack: viz === 'bars' && !isDuration ? 'x' : undefined,
          showSymbol: false,
          barMaxWidth: 14,
          color: colors[i % colors.length],
          data: d.times.map((t, j) => [t + d.stepMs / 2, s.values[j] ?? null]),
        })),
      };
    }
    if (viz === 'pie') {
      return {
        tooltip: { trigger: 'item', valueFormatter: (v) => fmt(v as number) },
        legend: { type: 'scroll', orient: 'vertical', right: 10, top: 20 },
        series: [{ type: 'pie', radius: ['35%', '70%'], center: ['40%', '50%'], data: d.series.map((s, i) => ({ name: s.group, value: s.total, itemStyle: { color: colors[i % colors.length] } })) }],
      };
    }
    if (viz === 'treemap') {
      return {
        tooltip: { valueFormatter: (v) => fmt(v as number) },
        series: [
          {
            type: 'treemap',
            roam: false,
            nodeClick: false,
            breadcrumb: { show: false },
            data: d.series.map((s, i) => ({ name: s.group, value: s.total, itemStyle: { color: colors[i % colors.length] } })),
          },
        ],
      };
    }
    return null;
  }, [a.data, viz, groupBy, measure]); // eslint-disable-line react-hooks/exhaustive-deps

  if (a.isPending) return <Loading label="Computing…" />;
  if (a.isError) return <ErrorState error={a.error} onRetry={() => a.refetch()} />;
  const d = a.data;
  if (d.series.length === 0) return <p className="mon-empty">No spans match this search in the selected timeframe.</p>;
  if (viz === 'toplist' || viz === 'table') {
    const max = Math.max(...d.series.map((s) => s.total), 1);
    return (
      <section className="sx-results">
        <table className="sx-table">
          <thead>
            <tr>
              <th>{groupBy ? groupBy.toUpperCase().replace('_', ' ') : 'ALL SPANS'}</th>
              <th className="num">{measure.toUpperCase()}</th>
              {viz === 'toplist' && <th />}
            </tr>
          </thead>
          <tbody>
            {[...d.series]
              .sort((x, y) => y.total - x.total)
              .map((s) => (
                <tr key={s.group} onClick={() => groupBy && onPick(groupBy, s.group)} title={groupBy ? 'Filter on this value' : undefined}>
                  <td>{s.group}</td>
                  <td className="num">{fmt(s.total)}</td>
                  {viz === 'toplist' && (
                    <td className="sx-toplist-cell">
                      <i style={{ width: `${(s.total / max) * 100}%` }} />
                    </td>
                  )}
                </tr>
              ))}
          </tbody>
        </table>
      </section>
    );
  }
  return (
    <section className="sx-results sx-chart">
      {option && <Chart option={option} height={420} onClick={(p) => groupBy && onPick(groupBy, (p.seriesName && viz !== 'pie' && viz !== 'treemap' ? p.seriesName : p.name) ?? '')} />}
    </section>
  );
}
