import { useMemo } from 'react';
import type { EChartsOption } from 'echarts';
import type { HistogramBucket, LogStatus } from '../../api/types';
import { Chart } from '../../components/charts/Chart';
import { compact, hhmm } from '../../lib/format';
import { statusColor } from '../../theme/chartTheme';

const STACK_ORDER: LogStatus[] = ['error', 'warn', 'info', 'debug'];

/** Status-stacked log volume bars above the log list. */
export function LogHistogram({ buckets, height = 74 }: { buckets: HistogramBucket[]; height?: number }) {
  const option = useMemo<EChartsOption>(
    () => ({
      grid: { left: 36, right: 12, top: 8, bottom: 20 },
      tooltip: { trigger: 'axis', axisPointer: { type: 'shadow' } },
      xAxis: {
        type: 'category',
        data: buckets.map((b) => b.t),
        axisLabel: {
          formatter: (v: string) => hhmm(Number(v)),
          interval: Math.max(0, Math.floor(buckets.length / 14) - 1),
        },
      },
      yAxis: {
        type: 'value',
        minInterval: 1,
        splitNumber: 1,
        axisLabel: { formatter: (v: number) => compact(v).replace('K', 'k') },
      },
      series: STACK_ORDER.map((s) => ({
        name: s,
        type: 'bar',
        stack: 'status',
        barCategoryGap: '25%',
        barMaxWidth: 12,
        color: statusColor(s),
        data: buckets.map((b) => b.counts[s]),
      })),
    }),
    [buckets],
  );
  return <Chart option={option} height={height} />;
}
