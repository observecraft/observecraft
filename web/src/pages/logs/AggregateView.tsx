import { useMemo } from 'react';
import type { EChartsOption } from 'echarts';
import type { HistogramBucket, LogEvent, LogStatus } from '../../api/types';
import { Chart } from '../../components/charts/Chart';
import { palette, statusColor } from '../../theme/chartTheme';

export type VizMode =
  | 'list'
  | 'timeseries'
  | 'toplist'
  | 'bar'
  | 'table'
  | 'treemap'
  | 'pie'
  | 'scatter'
  | 'distribution';

interface Props {
  mode: Exclude<VizMode, 'list'>;
  logs: LogEvent[];
  histogram: HistogramBucket[];
}

const STATUSES: LogStatus[] = ['error', 'warn', 'info', 'debug'];

/** Every non-list "Visualize as" mode: log count grouped by service. */
export function AggregateView({ mode, logs, histogram }: Props) {
  const byService = useMemo(() => {
    const m = new Map<string, { count: number; errors: number; durations: number[] }>();
    for (const l of logs) {
      const e = m.get(l.service) ?? { count: 0, errors: 0, durations: [] };
      e.count++;
      if (l.status === 'error') e.errors++;
      e.durations.push(Number(l.attributes.duration_ms) || 0);
      m.set(l.service, e);
    }
    return [...m.entries()].sort((a, b) => b[1].count - a[1].count);
  }, [logs]);

  if (mode === 'toplist' || mode === 'table') {
    const max = byService[0]?.[1].count ?? 1;
    return (
      <table className="agg-table">
        <thead>
          <tr>
            <th>SERVICE</th>
            <th className="num">COUNT</th>
            {mode === 'table' && <th className="num">ERRORS</th>}
            {mode === 'table' && <th className="num">AVG DURATION</th>}
          </tr>
        </thead>
        <tbody>
          {byService.map(([svc, e]) => (
            <tr key={svc}>
              <td>
                {mode === 'toplist' && (
                  <span className="toplist-bar" style={{ width: `${(e.count / max) * 100}%` }} />
                )}
                <span className="toplist-label">{svc}</span>
              </td>
              <td className="num">{e.count.toLocaleString()}</td>
              {mode === 'table' && <td className="num">{e.errors.toLocaleString()}</td>}
              {mode === 'table' && (
                <td className="num">
                  {Math.round(e.durations.reduce((a, b) => a + b, 0) / e.durations.length)} ms
                </td>
              )}
            </tr>
          ))}
        </tbody>
      </table>
    );
  }

  return <Chart option={option(mode, byService, histogram, logs)} height={360} />;
}

function option(
  mode: Exclude<Props['mode'], 'toplist' | 'table'>,
  byService: [string, { count: number; errors: number; durations: number[] }][],
  histogram: HistogramBucket[],
  logs: LogEvent[],
): EChartsOption {
  const names = byService.map(([s]) => s);
  const counts = byService.map(([, e]) => e.count);
  const grid = { left: 48, right: 16, top: 30, bottom: 30 };

  switch (mode) {
    case 'timeseries':
      return {
        grid,
        tooltip: { trigger: 'axis' },
        legend: { top: 0, right: 0 },
        xAxis: { type: 'time' },
        yAxis: { type: 'value' },
        series: STATUSES.map((s) => ({
          name: s,
          type: 'line',
          stack: 'total',
          areaStyle: { opacity: 0.25 },
          color: statusColor(s),
          data: histogram.map((b) => [b.t, b.counts[s]]),
        })),
      };
    case 'bar':
      return {
        grid,
        tooltip: { trigger: 'axis' },
        xAxis: { type: 'category', data: names },
        yAxis: { type: 'value' },
        series: [{ type: 'bar', data: counts, barMaxWidth: 40, color: palette()[0] }],
      };
    case 'treemap':
      return {
        tooltip: {},
        series: [
          {
            type: 'treemap',
            roam: false,
            nodeClick: false,
            breadcrumb: { show: false },
            label: { fontSize: 12 },
            data: byService.map(([name, e], i) => ({ name, value: e.count, itemStyle: { color: palette()[i % 8] } })),
          },
        ],
      };
    case 'pie':
      return {
        tooltip: { trigger: 'item' },
        legend: { orient: 'vertical', right: 10, top: 'middle' },
        series: [
          {
            type: 'pie',
            radius: ['40%', '70%'],
            center: ['40%', '50%'],
            label: { show: false },
            data: byService.map(([name, e]) => ({ name, value: e.count })),
          },
        ],
      };
    case 'scatter':
      return {
        grid,
        tooltip: { formatter: (p: any) => `${p.data[2]}<br/>logs: ${p.data[0]}<br/>errors: ${p.data[1]}` },
        xAxis: { type: 'value', name: 'logs', nameLocation: 'middle', nameGap: 22 },
        yAxis: { type: 'value', name: 'errors' },
        series: [
          {
            type: 'scatter',
            symbolSize: 12,
            data: byService.map(([name, e]) => [e.count, e.errors, name]),
          },
        ],
      };
    case 'distribution': {
      const edges = [0, 100, 200, 300, 400, 500, 600, 700, 800, 900, 1000];
      const bins = edges.slice(0, -1).map(() => 0);
      for (const l of logs) {
        const d = Number(l.attributes.duration_ms) || 0;
        bins[Math.min(bins.length - 1, Math.floor(d / 100))]++;
      }
      return {
        grid,
        tooltip: { trigger: 'axis' },
        xAxis: { type: 'category', data: edges.slice(0, -1).map((e) => `${e}ms`) },
        yAxis: { type: 'value' },
        series: [{ type: 'bar', data: bins, barCategoryGap: '4%', color: palette()[1] }],
      };
    }
  }
}
