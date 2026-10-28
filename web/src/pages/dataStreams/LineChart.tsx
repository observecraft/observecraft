import { useMemo } from 'react';
import type { EChartsOption } from 'echarts';
import { Chart } from '../../components/charts/Chart';
import { hhmm } from '../../lib/format';
import { token } from '../../theme/chartTheme';

export interface LineSeries {
  name: string;
  points: [number, number][];
  /** A chart token (`--chart-5`, `--status-error`) for this line. */
  color?: string;
}

/** A small time series chart with a title; "No data in this range" when every series is empty. */
export function LineChart({ title, series, fmt, height = 170, area = false }: { title: string; series: LineSeries[]; fmt: (v: number) => string; height?: number; area?: boolean }) {
  const option = useMemo<EChartsOption>(
    () => ({
      grid: { left: 64, right: 12, top: 14, bottom: series.length > 1 ? 44 : 24 },
      legend: series.length > 1 ? { show: true, bottom: 0, icon: 'roundRect', itemWidth: 10, itemHeight: 4, textStyle: { fontSize: 11 } } : { show: false },
      tooltip: { trigger: 'axis', valueFormatter: (v) => (v == null ? '—' : fmt(Number(v))) },
      xAxis: { type: 'time', splitNumber: 4, axisLabel: { formatter: (v: number) => hhmm(v) } },
      yAxis: { type: 'value', min: 0, splitNumber: 3, axisLabel: { formatter: (v: number) => fmt(v) } },
      series: series.map((s) => ({
        name: s.name,
        type: 'line',
        showSymbol: false,
        data: s.points,
        lineStyle: { width: 1.6, color: s.color ? token(s.color) : undefined },
        itemStyle: s.color ? { color: token(s.color) } : undefined,
        areaStyle: area || series.length === 1 ? { opacity: 0.1 } : undefined,
      })),
    }),
    [series, fmt, area],
  );
  const bottom = series.length > 1 ? height + 18 : height;
  return (
    <div className="dsm-chart">
      <h4>{title}</h4>
      {series.every((s) => s.points.length === 0) ? <div className="dsm-chart-empty muted">No data in this range</div> : <Chart option={option} height={bottom} />}
    </div>
  );
}
