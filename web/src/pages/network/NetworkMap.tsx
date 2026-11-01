import { useMemo, useState } from 'react';
import { useQuery } from '@tanstack/react-query';
import type { EChartsOption } from 'echarts';
import { useSearchParams } from 'react-router-dom';
import { Search, X } from 'lucide-react';
import { networkFlows, type Flow } from '../../api/network/client';
import { Chart } from '../../components/charts/Chart';
import { useTime } from '../../components/shell/TimeContext';
import { ErrorState, Loading } from '../../components/ui/AsyncState';
import { formatBytes } from '../../components/ui/Pager';
import { token } from '../../theme/chartTheme';
import { GROUP_KEYS, fmtLatency, fmtThroughput } from './NetworkAnalytics';
import { useTraffic } from './NetworkLayout';
import { NetworkIntro } from './NetworkIntro';

/** What edges show (Datadog's list). */
const EDGE_METRICS: { key: string; label: string; value: (f: Flow) => number; fmt: (v: number) => string }[] = [
  { key: 'throughput_sent', label: 'Throughput sent', value: (f) => f.throughputSent, fmt: fmtThroughput },
  { key: 'throughput_received', label: 'Throughput received', value: (f) => f.throughputReceived, fmt: fmtThroughput },
  { key: 'retransmits', label: 'TCP Retransmits', value: (f) => f.retransmits, fmt: (v) => v.toLocaleString() },
  { key: 'latency', label: 'TCP Latency', value: (f) => f.rttUs ?? 0, fmt: fmtLatency },
  { key: 'jitter', label: 'TCP Jitter', value: (f) => f.rttVarUs ?? 0, fmt: fmtLatency },
  { key: 'established', label: 'Established Connections', value: (f) => f.established, fmt: (v) => v.toLocaleString() },
  { key: 'closed', label: 'Closed Connections', value: (f) => f.closed, fmt: (v) => v.toLocaleString() },
];

/** Cloud Network Monitoring → Map: nodes are the values of a tag, edges the traffic between them. */
export function NetworkMap() {
  const { range } = useTime();
  const traffic = useTraffic();
  const [params, setParams] = useSearchParams();
  const tag = params.get('tag') ?? 'service';
  const metricKey = params.get('edge') ?? 'throughput_sent';
  const [filter, setFilter] = useState('');
  const [inspect, setInspect] = useState<string | null>(null);
  const [menu, setMenu] = useState<{ node: string; x: number; y: number } | null>(null);
  const metric = EDGE_METRICS.find((m) => m.key === metricKey) ?? EDGE_METRICS[0];
  const set = (k: string, v: string | null) =>
    setParams(
      (p) => {
        const n = new URLSearchParams(p);
        if (v) n.set(k, v);
        else n.delete(k);
        return n;
      },
      { replace: true },
    );
  const q = useQuery({
    queryKey: ['network', 'map', range, tag, traffic],
    queryFn: () => networkFlows({ range, query: '', clientGroupBy: tag, serverGroupBy: tag, ...traffic, limit: 1000 }),
  });

  const option = useMemo<EChartsOption | null>(() => {
    if (!q.data) return null;
    const needle = filter.trim().toLowerCase();
    const flows = q.data.flows.filter(
      (f) => (!inspect || f.client === inspect || f.server === inspect) && (!needle || f.client.toLowerCase().includes(needle) || f.server.toLowerCase().includes(needle)),
    );
    const volume = new Map<string, number>();
    for (const f of flows) {
      for (const n of [f.client, f.server]) volume.set(n, (volume.get(n) ?? 0) + f.bytesSent + f.bytesReceived);
    }
    const maxVol = Math.max(1, ...volume.values());
    const maxEdge = Math.max(1e-9, ...flows.map(metric.value));
    const primary = token('--chart-1');
    return {
      tooltip: {
        formatter: (p: unknown) => {
          const d = (p as { dataType: string; data: { name?: string; source?: string; target?: string; value?: number } }).data;
          const dt = (p as { dataType: string }).dataType;
          return dt === 'edge' ? `${d.source} → ${d.target}<br/>${metric.label}: ${metric.fmt(d.value ?? 0)}` : `${d.name}<br/>${formatBytes(volume.get(d.name ?? '') ?? 0)} sent and received`;
        },
      },
      series: [
        {
          type: 'graph',
          layout: 'force',
          roam: true,
          draggable: true,
          force: { repulsion: 420, edgeLength: [90, 220], gravity: 0.08 },
          edgeSymbol: ['none', 'arrow'],
          edgeSymbolSize: 7,
          label: { show: true, position: 'right', fontSize: 12 },
          emphasis: { focus: 'adjacency', lineStyle: { width: 4 } },
          data: [...volume].map(([name, v]) => ({
            name,
            value: v,
            symbolSize: 14 + 26 * Math.sqrt(v / maxVol),
            itemStyle: { color: name === 'N/A' ? token('--status-nodata') : primary, borderColor: name === inspect ? token('--text') : undefined, borderWidth: name === inspect ? 3 : 0 },
          })),
          links: flows.map((f) => ({
            source: f.client,
            target: f.server,
            value: metric.value(f),
            lineStyle: { width: 1 + 6 * (metric.value(f) / maxEdge), opacity: 0.55, curveness: 0.15, color: token('--text-faint') },
          })),
        },
      ],
    };
  }, [q.data, filter, inspect, metric]);

  return (
    <div className="net-page net-map-page">
      <div className="net-map-controls">
        <label className="net-chipset">
          <span>View</span>
          <select value={tag} onChange={(e) => (set('tag', e.target.value === 'service' ? null : e.target.value), setInspect(null))}>
            {GROUP_KEYS.map((g) => (
              <option key={g.key} value={g.key}>
                {g.label}
              </option>
            ))}
          </select>
        </label>
        <label className="net-chipset">
          <span>Edges show</span>
          <select value={metric.key} onChange={(e) => set('edge', e.target.value === 'throughput_sent' ? null : e.target.value)}>
            {EDGE_METRICS.map((m) => (
              <option key={m.key} value={m.key}>
                {m.label}
              </option>
            ))}
          </select>
        </label>
        <label className="net-map-filter">
          <Search size={14} />
          <input value={filter} onChange={(e) => setFilter(e.target.value)} placeholder="Filter nodes" />
        </label>
        {inspect && (
          <span className="net-inspecting">
            Inspecting <b>{inspect}</b>
            <button className="net-icon-btn" onClick={() => setInspect(null)} title="Stop inspecting">
              <X size={14} />
            </button>
          </span>
        )}
      </div>
      {q.isError ? (
        <ErrorState error={q.error} onRetry={() => q.refetch()} />
      ) : !q.data || !option ? (
        <Loading label="Loading network map…" />
      ) : !q.data.enabled ? (
        <NetworkIntro />
      ) : (
        <div className="net-map" onClick={() => setMenu(null)}>
          <Chart
            option={option}
            height={640}
            onClick={(p: unknown) => {
              const e = p as { dataType?: string; name?: string; event?: { offsetX: number; offsetY: number } };
              if (e.dataType === 'node' && e.name) setTimeout(() => setMenu({ node: e.name!, x: e.event?.offsetX ?? 0, y: e.event?.offsetY ?? 0 }));
            }}
          />
          {menu && (
            <div className="dd-menu net-node-menu" style={{ left: menu.x + 12, top: menu.y }}>
              <button onClick={() => (setInspect(menu.node), setMenu(null))}>Inspect</button>
            </div>
          )}
        </div>
      )}
    </div>
  );
}
