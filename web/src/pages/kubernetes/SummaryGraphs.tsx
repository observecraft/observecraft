import { useMemo, useState } from 'react';
import { useQuery } from '@tanstack/react-query';
import type { EChartsOption } from 'echarts';
import { BarChart3, ChevronDown, ChevronRight } from 'lucide-react';
import { queryTimeseries } from '../../api/dashboards/client';
import type { KubeResource } from '../../api/kubernetes/client';
import { Chart } from '../../components/charts/Chart';
import { Select } from '../../components/ui/Dropdown';
import { formatBytes } from '../../components/ui/Pager';
import { parseQuery } from '../../lib/searchQuery';
import { palette } from '../../theme/chartTheme';
import { formatCores, normalizedCpu } from './columns';

/** The live container metrics a summary graph can show (sent by the Datadog Agent). */
const GRAPHS = {
  cpu: { label: 'Total CPU', metric: 'container.cpu.usage', format: (v: number) => formatCores(v), axis: 'Cores' },
  rss: { label: 'RSS Memory', metric: 'container.memory.rss', format: (v: number) => formatBytes(v), axis: 'Bytes' },
  sent: { label: 'Net Bytes Sent (TX)', metric: 'container.net.sent', format: (v: number) => `${formatBytes(v)}/s`, axis: 'Bytes/s' },
  received: { label: 'Net Bytes Received (RX)', metric: 'container.net.rcvd', format: (v: number) => `${formatBytes(v)}/s`, axis: 'Bytes/s' },
} as const;
type GraphKey = keyof typeof GRAPHS;
/** Filter keys that are also metric tags, so the graphs follow the list's filters. */
const METRIC_TAGS = ['container_name', 'host', 'kube_cluster_name', 'kube_namespace', 'pod_name', 'short_image', 'image_name', 'kube_deployment', 'service', 'env'];
const TOP = 10;

/** The list's filter as a metric scope: its plain `key:value` terms on metric tags. */
function scope(query: string): string {
  const parts = parseQuery(query).flatMap((t) => (t.kind === 'tag' && !t.negate && t.values.length === 1 && METRIC_TAGS.includes(t.key) ? [`${t.key}:${t.values[0]}`] : []));
  return parts.length ? parts.join(',') : '*';
}

function Timeseries({ graph, query, range }: { graph: GraphKey; query: string; range: { from: number; to: number } }) {
  const g = GRAPHS[graph];
  const q = useQuery({
    queryKey: ['containers', 'summary', graph, query, range],
    queryFn: () => queryTimeseries({ queries: [`avg:${g.metric}{${scope(query)}} by {container_name,host}`], range, points: 90 }),
    retry: false,
  });
  // The busiest containers, by their average.
  const series = (q.data ?? [])
    .map((s) => ({ ...s, avg: s.points.reduce((n, p) => n + p[1], 0) / Math.max(1, s.points.length) }))
    .sort((a, b) => b.avg - a.avg)
    .slice(0, TOP);
  const colors = palette();
  const option: EChartsOption = {
    grid: { left: 8, right: 12, top: 12, bottom: 44, containLabel: true },
    tooltip: { trigger: 'axis', confine: true, valueFormatter: (v) => g.format(Number(v)) },
    legend: { bottom: 0, type: 'scroll', formatter: (n: string) => n.replace(/(^|, )\w+:/g, '$1') },
    xAxis: { type: 'time' },
    yAxis: { type: 'value', axisLabel: { formatter: (v: number) => g.format(v) } },
    series: series.map((s, i) => ({ name: s.name, type: 'line', showSymbol: false, color: colors[i % colors.length], lineStyle: { width: 1.2 }, data: s.points })),
  };
  if (q.isError || (q.isSuccess && !series.length)) return <p className="muted-text sg-empty">No {g.label.toLowerCase()} reported by the Datadog Agent for these containers.</p>;
  return <Chart option={option} height="100%" />;
}

/** CPU against memory of the listed containers. */
function Scatter({ rows }: { rows: KubeResource[] }) {
  const points = rows.filter((r) => r.memory?.usage != null && normalizedCpu(r) != null);
  const option: EChartsOption = {
    grid: { left: 8, right: 16, top: 12, bottom: 30, containLabel: true },
    tooltip: {
      trigger: 'item',
      formatter: (p) => {
        const r = points[(p as { dataIndex: number }).dataIndex];
        return `${r.name}<br/>CPU ${normalizedCpu(r)!.toFixed(1)}% · RSS ${formatBytes(r.memory!.usage!)}`;
      },
    },
    xAxis: { type: 'value', name: 'Total CPU - Normalized (%)', nameLocation: 'middle', nameGap: 24 },
    yAxis: { type: 'value', name: 'RSS Memory', axisLabel: { formatter: (v: number) => formatBytes(v) } },
    series: [{ type: 'scatter', symbolSize: 9, color: palette()[0], data: points.map((r) => [normalizedCpu(r)!, r.memory!.usage!]) }],
  };
  return <Chart option={option} height="100%" />;
}

/** Above the containers list: how the busiest containers' CPU and memory evolve, or CPU against memory. */
export function SummaryGraphs({ query, range, rows }: { query: string; range: { from: number; to: number }; rows: KubeResource[] }) {
  const [open, setOpen] = useState(true);
  const [mode, setMode] = useState<'timeseries' | 'scatter'>('timeseries');
  const [left, setLeft] = useState<GraphKey>('cpu');
  const [right, setRight] = useState<GraphKey>('rss');
  const options = useMemo(() => (Object.keys(GRAPHS) as GraphKey[]).map((k) => ({ value: k, label: GRAPHS[k].label })), []);
  return (
    <section className="sg">
      <header className="sg-head">
        <button className="icon-plain" onClick={() => setOpen((o) => !o)} title={open ? 'Collapse' : 'Expand'}>
          {open ? <ChevronDown size={16} /> : <ChevronRight size={16} />}
        </button>
        <BarChart3 size={15} />
        <b>Summary Graphs</b>
        <span className="btn-group">
          <button className={`btn btn-sm${mode === 'timeseries' ? ' btn-active' : ''}`} onClick={() => setMode('timeseries')}>
            Timeseries
          </button>
          <button className={`btn btn-sm${mode === 'scatter' ? ' btn-active' : ''}`} onClick={() => setMode('scatter')}>
            Scatter Plot
          </button>
        </span>
      </header>
      {open &&
        (mode === 'timeseries' ? (
          <div className="sg-graphs">
            {(
              [
                [left, setLeft],
                [right, setRight],
              ] as const
            ).map(([g, set], i) => (
              <div key={i} className="sg-graph">
                <Select value={g} options={options} onChange={(v) => set(v as GraphKey)} />
                <div className="sg-chart">
                  <Timeseries graph={g} query={query} range={range} />
                </div>
              </div>
            ))}
          </div>
        ) : (
          <div className="sg-chart sg-scatter">
            <Scatter rows={rows} />
          </div>
        ))}
    </section>
  );
}
