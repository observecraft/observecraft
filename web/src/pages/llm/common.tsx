import { useMemo, type ReactNode } from 'react';
import type { EChartsOption } from 'echarts';
import type { LlmSeries } from '../../api/llm/client';
import { Chart } from '../../components/charts/Chart';
import { palette } from '../../theme/chartTheme';
import { kindLabel } from './format';

export function StatCard({ label, value, sub, tone }: { label: string; value: ReactNode; sub?: ReactNode; tone?: 'error' | 'estimate' }) {
  return (
    <div className={`llm-stat${tone ? ` ${tone}` : ''}`}>
      <span className="llm-stat-label">{label}</span>
      <span className="llm-stat-value">{value}</span>
      {sub && <span className="llm-stat-sub">{sub}</span>}
    </div>
  );
}

/** A time series chart of grouped series (one line or stacked bar per group). */
export function SeriesChart({
  title,
  times,
  stepMs,
  series,
  kind = 'line',
  format,
  height = 180,
}: {
  title: string;
  times: number[];
  stepMs: number;
  series: LlmSeries[];
  kind?: 'line' | 'bar';
  format?: (v: number) => string;
  height?: number;
}) {
  const option = useMemo((): EChartsOption => {
    const colors = palette();
    return {
      grid: { left: 56, right: 12, top: 34, bottom: 24 },
      legend: { top: 0, type: 'scroll', itemWidth: 10, itemHeight: 8, textStyle: { fontSize: 11 } },
      tooltip: { trigger: 'axis', valueFormatter: format ? (v) => (typeof v === 'number' ? format(v) : '—') : undefined },
      xAxis: { type: 'time', splitNumber: 5 },
      yAxis: { type: 'value', splitNumber: 3, axisLabel: format ? { formatter: (v: number) => format(v) } : undefined },
      series: series.map((s, i) => ({
        name: s.group,
        type: kind,
        stack: kind === 'bar' ? 'total' : undefined,
        barMaxWidth: 14,
        showSymbol: false,
        connectNulls: false,
        color: colors[i % colors.length],
        data: times.map((t, j) => [t + stepMs / 2, s.values[j] ?? null]),
      })),
    };
  }, [times, stepMs, series, kind, format]);
  return (
    <div className="llm-chart">
      <h4>{title}</h4>
      {series.length ? <Chart option={option} height={height} /> : <p className="muted llm-chart-empty">No data in this time range.</p>}
    </div>
  );
}

export function KindBadge({ kind }: { kind: string }) {
  return <span className={`llm-kind k-${kind}`}>{kindLabel(kind)}</span>;
}

export function StatusDot({ status }: { status: string }) {
  return <i className={`llm-dot ${status === 'error' ? 'err' : 'ok'}`} title={status === 'error' ? 'Error' : 'OK'} />;
}

export const COST_NOTE = 'Estimated from list prices per million tokens kept in ObserveCraft, without cached-input, batch or negotiated discounts. Check your provider’s bill for actual spend.';

export function fmtStamp(ms: number): string {
  const d = new Date(ms);
  return `${d.toLocaleString(undefined, { month: 'short', day: 'numeric' })} ${d.toLocaleTimeString(undefined, { hour12: false })}`;
}
