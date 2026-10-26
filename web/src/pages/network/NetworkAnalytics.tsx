import { useMemo, useState } from 'react';
import { useQuery } from '@tanstack/react-query';
import type { EChartsOption } from 'echarts';
import { useNavigate, useSearchParams } from 'react-router-dom';
import { ArrowRight, Columns3, ExternalLink, Globe, PanelLeft, Plus, X } from 'lucide-react';
import { searchLogs } from '../../api/client';
import { searchSpans } from '../../api/apm/client';
import { API_MODE } from '../../api/http';
import { searchKubernetes } from '../../api/kubernetes/client';
import { networkFlows, type Flow, type FlowPage, type NetworkPoint } from '../../api/network/client';
import { Chart } from '../../components/charts/Chart';
import { FacetPanel } from '../../components/facets/FacetPanel';
import { QueryBar } from '../../components/query/QueryBar';
import { useTime } from '../../components/shell/TimeContext';
import { Empty, ErrorState, Loading } from '../../components/ui/AsyncState';
import { formatBytes } from '../../components/ui/Pager';
import { hhmm, timeAgo } from '../../lib/format';
import { addTerm, formatQuery, parseQuery, type Term } from '../../lib/searchQuery';
import { LogTable } from '../logs/LogTable';
import { useTraffic } from './NetworkLayout';

/** Tags a side can be grouped by. */
export const GROUP_KEYS: { key: string; label: string }[] = [
  { key: 'service', label: 'service' },
  { key: 'env', label: 'env' },
  { key: 'availability-zone', label: 'availability-zone' },
  { key: 'kube_namespace', label: 'kube_namespace' },
  { key: 'kube_service', label: 'kube_service' },
  { key: 'pod_name', label: 'pod_name' },
  { key: 'container_name', label: 'container_name' },
  { key: 'short_image', label: 'short_image' },
  { key: 'host', label: 'host' },
  { key: 'ip', label: 'ip' },
  { key: 'port', label: 'port' },
  { key: 'domain', label: 'domain' },
];

const FACET_LABELS: Record<string, string> = {
  service: 'Service',
  env: 'Env',
  'availability-zone': 'Availability Zone',
  kube_namespace: 'Namespace',
  pod_name: 'Pod',
  host: 'Host',
  ip: 'IP',
  port: 'Port',
  domain: 'Domain',
};

export const fmtThroughput = (bps: number) => `${formatBytes(bps)}/s`;
export const fmtLatency = (us: number | null | undefined) => (us == null ? '—' : us >= 1000 ? `${(us / 1000).toFixed(us >= 100_000 ? 0 : 2)} ms` : `${Math.round(us)} µs`);

/** Summary graph metrics (Datadog's list). */
const GRAPH_METRICS: { key: string; label: string; value: (p: NetworkPoint, stepS: number) => number | null; fmt: (v: number) => string }[] = [
  { key: 'volume_sent', label: 'Volume Sent', value: (p) => p.bytesSent, fmt: formatBytes },
  { key: 'throughput_sent', label: 'Throughput Sent', value: (p, s) => p.bytesSent / s, fmt: fmtThroughput },
  { key: 'volume_received', label: 'Volume Received', value: (p) => p.bytesReceived, fmt: formatBytes },
  { key: 'throughput_received', label: 'Throughput Received', value: (p, s) => p.bytesReceived / s, fmt: fmtThroughput },
  { key: 'established', label: 'Established Connections', value: (p) => p.established, fmt: (v) => v.toLocaleString() },
  { key: 'closed', label: 'Closed Connections', value: (p) => p.closed, fmt: (v) => v.toLocaleString() },
  { key: 'established_rate', label: 'Established Connections / Second', value: (p, s) => p.established / s, fmt: (v) => v.toFixed(2) },
  { key: 'closed_rate', label: 'Closed Connections / Second', value: (p, s) => p.closed / s, fmt: (v) => v.toFixed(2) },
  { key: 'tcp_latency', label: 'TCP Latency', value: (p) => (p.rttUs == null ? null : p.rttUs), fmt: (v) => fmtLatency(v) },
];

interface Column {
  key: string;
  label: string;
  num?: boolean;
  render: (f: Flow) => React.ReactNode;
}

const COLUMNS: Column[] = [
  { key: 'volume_sent', label: 'VOLUME SENT', num: true, render: (f) => formatBytes(f.bytesSent) },
  { key: 'volume_received', label: 'VOLUME RECEIVED', num: true, render: (f) => formatBytes(f.bytesReceived) },
  { key: 'throughput_sent', label: 'THROUGHPUT SENT', num: true, render: (f) => fmtThroughput(f.throughputSent) },
  { key: 'throughput_received', label: 'THROUGHPUT RECEIVED', num: true, render: (f) => fmtThroughput(f.throughputReceived) },
  { key: 'retransmits', label: 'TCP RETRANSMITS', num: true, render: (f) => f.retransmits.toLocaleString() },
  { key: 'latency', label: 'TCP LATENCY', num: true, render: (f) => fmtLatency(f.rttUs) },
  { key: 'jitter', label: 'TCP JITTER', num: true, render: (f) => fmtLatency(f.rttVarUs) },
  { key: 'established', label: 'ESTABLISHED CONNECTIONS', num: true, render: (f) => f.established.toLocaleString() },
  { key: 'closed', label: 'CLOSED CONNECTIONS', num: true, render: (f) => f.closed.toLocaleString() },
];

function SummaryGraph({ page, metric, onMetric, onRemove }: { page: FlowPage; metric: string; onMetric: (m: string) => void; onRemove: () => void }) {
  const m = GRAPH_METRICS.find((x) => x.key === metric) ?? GRAPH_METRICS[0];
  const stepS = page.stepMs / 1000;
  const option = useMemo<EChartsOption>(
    () => ({
      grid: { left: 64, right: 10, top: 10, bottom: 22 },
      tooltip: { trigger: 'axis', valueFormatter: (v) => (v == null ? '—' : m.fmt(Number(v))) },
      xAxis: { type: 'time', axisLabel: { formatter: (v: number) => hhmm(v) } },
      yAxis: { type: 'value', splitNumber: 3, axisLabel: { formatter: (v: number) => m.fmt(v) } },
      series: [{ type: 'line', showSymbol: false, areaStyle: { opacity: 0.15 }, data: page.series.map((p) => [p.t, m.value(p, stepS)]) }],
    }),
    [page, m, stepS],
  );
  return (
    <div className="net-graph">
      <div className="net-graph-head">
        <select value={m.key} onChange={(e) => onMetric(e.target.value)}>
          {GRAPH_METRICS.map((x) => (
            <option key={x.key} value={x.key}>
              {x.label}
            </option>
          ))}
        </select>
        <button className="net-icon-btn" title="Hide graph" onClick={onRemove}>
          <X size={14} />
        </button>
      </div>
      <Chart option={option} height={140} />
    </div>
  );
}

/** Tags of one side, as chips. */
function TagChips({ tags }: { tags: string[] }) {
  return (
    <div className="net-chips">
      {tags.map((t) => (
        <span key={t} className="net-chip">
          {t}
        </span>
      ))}
    </div>
  );
}

const tagValue = (tags: string[], key: string) => tags.find((t) => t.startsWith(`${key}:`))?.slice(key.length + 1);

type PanelTab = 'flows' | 'logs' | 'traces' | 'processes';

/** Side panel of a flow: shared tags, then the connections behind it, logs, traces and processes. */
function FlowPanel({ flow, base, onClose }: { flow: Flow; base: { query: string; clientGroupBy: string; serverGroupBy: string }; onClose: () => void }) {
  const navigate = useNavigate();
  const { range } = useTime();
  const traffic = useTraffic();
  const [tab, setTab] = useState<PanelTab>('flows');
  let scoped = base.query;
  if (flow.client !== 'N/A') scoped = addTerm(scoped, `client_${base.clientGroupBy}`, flow.client);
  if (flow.server !== 'N/A') scoped = addTerm(scoped, `server_${base.serverGroupBy}`, flow.server);
  const service = tagValue(flow.clientTags, 'service');
  const host = tagValue(flow.clientTags, 'host');
  const detail = useQuery({
    queryKey: ['network', 'detail', range, scoped, traffic],
    queryFn: () => networkFlows({ range, query: scoped, clientGroupBy: 'ip', serverGroupBy: 'port', ...traffic, showUnresolved: true, limit: 200 }),
    enabled: tab === 'flows',
  });
  const logs = useQuery({ queryKey: ['network', 'logs', range, service], queryFn: () => searchLogs({ range, query: `service:${service}` }), enabled: tab === 'logs' && !!service });
  const spans = useQuery({
    queryKey: ['network', 'spans', range, service],
    queryFn: () => searchSpans({ range, query: `service:${service}`, limit: 50 }),
    enabled: tab === 'traces' && !!service && API_MODE === 'live',
  });
  const procs = useQuery({
    queryKey: ['network', 'processes', host],
    queryFn: () => searchKubernetes({ kind: 'processes', query: `host:${host}`, groupBy: null, sort: 'cpu', desc: true, offset: 0, limit: 50 }),
    enabled: tab === 'processes' && !!host,
  });
  return (
    <aside className="net-panel" role="dialog" aria-label="Flow details">
      <header className="net-panel-head">
        <h3>
          {flow.client} <ArrowRight size={16} /> {flow.server}
        </h3>
        <button className="net-icon-btn" onClick={onClose} title="Close">
          <X size={16} />
        </button>
      </header>
      <div className="net-panel-sides">
        <div>
          <h5>CLIENT</h5>
          <TagChips tags={flow.clientTags} />
        </div>
        <div>
          <h5>SERVER</h5>
          <TagChips tags={flow.serverTags} />
        </div>
      </div>
      <nav className="net-panel-tabs">
        {(['flows', 'logs', 'traces', 'processes'] as PanelTab[]).map((t) => (
          <button key={t} className={`net-panel-tab${tab === t ? ' active' : ''}`} onClick={() => setTab(t)}>
            {t[0].toUpperCase() + t.slice(1)}
          </button>
        ))}
      </nav>
      <div className="net-panel-body">
        {tab === 'flows' &&
          (detail.isPending ? (
            <Loading label="Loading flows…" />
          ) : detail.isError ? (
            <ErrorState error={detail.error} onRetry={() => detail.refetch()} />
          ) : (
            <table className="net-mini">
              <thead>
                <tr>
                  <th>CLIENT IP</th>
                  <th>SERVER PORT</th>
                  <th className="num">VOLUME SENT</th>
                  <th className="num">VOLUME RECEIVED</th>
                  <th className="num">TCP LATENCY</th>
                  <th className="num">RETRANSMITS</th>
                </tr>
              </thead>
              <tbody>
                {detail.data.flows.map((f) => (
                  <tr key={`${f.client}-${f.server}`}>
                    <td className="mono">{f.client}</td>
                    <td className="mono">{f.server}</td>
                    <td className="num">{formatBytes(f.bytesSent)}</td>
                    <td className="num">{formatBytes(f.bytesReceived)}</td>
                    <td className="num">{fmtLatency(f.rttUs)}</td>
                    <td className="num">{f.retransmits}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          ))}
        {tab === 'logs' &&
          (!service ? (
            <Empty>The client has no service tag to find its logs by.</Empty>
          ) : logs.isPending ? (
            <Loading label="Loading logs…" />
          ) : logs.isError ? (
            <ErrorState error={logs.error} onRetry={() => logs.refetch()} />
          ) : (
            <>
              <div className="net-panel-link">
                <code>service:{service}</code>
                <button className="btn btn-sm" onClick={() => navigate(`/logs?query=${encodeURIComponent(`service:${service}`)}`)}>
                  View in Log Explorer <ExternalLink size={13} />
                </button>
              </div>
              <LogTable logs={logs.data.logs.slice(0, 50)} selectedId={null} onSelect={() => undefined} options={{ wrap: false, showHost: true, showService: false }} />
            </>
          ))}
        {tab === 'traces' &&
          (!service || API_MODE !== 'live' ? (
            <Empty>No traces are linked to this client.</Empty>
          ) : spans.isPending ? (
            <Loading label="Loading traces…" />
          ) : spans.isError ? (
            <ErrorState error={spans.error} onRetry={() => spans.refetch()} />
          ) : (
            <table className="net-mini">
              <thead>
                <tr>
                  <th>DATE</th>
                  <th>RESOURCE</th>
                  <th className="num">DURATION</th>
                </tr>
              </thead>
              <tbody>
                {spans.data.spans.map((sp) => (
                  <tr key={sp.spanId} className="net-click" onClick={() => navigate(`/apm/traces?trace=${sp.traceId}`)}>
                    <td>{timeAgo(sp.timestamp)}</td>
                    <td className="net-ellipsis">{sp.resource}</td>
                    <td className="num">{sp.durationMs.toFixed(1)} ms</td>
                  </tr>
                ))}
              </tbody>
            </table>
          ))}
        {tab === 'processes' &&
          (!host ? (
            <Empty>The client has no host tag.</Empty>
          ) : procs.isPending ? (
            <Loading label="Loading processes…" />
          ) : procs.isError ? (
            <ErrorState error={procs.error} onRetry={() => procs.refetch()} />
          ) : procs.data.items.length === 0 ? (
            <Empty>{`No processes reported for host:${host}.`}</Empty>
          ) : (
            <table className="net-mini">
              <thead>
                <tr>
                  <th>PROCESS</th>
                  <th>USER</th>
                  <th className="num">RSS MEMORY</th>
                </tr>
              </thead>
              <tbody>
                {procs.data.items.map((p) => (
                  <tr key={p.uid}>
                    <td className="mono net-ellipsis">{p.name}</td>
                    <td>{p.user ?? '—'}</td>
                    <td className="num">{p.memory?.usage != null ? formatBytes(p.memory.usage) : '—'}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          ))}
      </div>
    </aside>
  );
}

/** Cloud Network Monitoring → Analytics: traffic between client and server groups. */
export function NetworkAnalytics() {
  const { range } = useTime();
  const traffic = useTraffic();
  const [params, setParams] = useSearchParams();
  const query = params.get('q') ?? '';
  const clientGroupBy = params.get('client') ?? 'service';
  const serverGroupBy = params.get('server') ?? 'service';
  const [graphs, setGraphs] = useState(['volume_sent', 'throughput_sent', 'tcp_latency']);
  const [side, setSide] = useState<'client' | 'server'>('client');
  const [showFacets, setShowFacets] = useState(true);
  const [hidden, setHidden] = useState<Set<string>>(new Set());
  const [customize, setCustomize] = useState(false);
  const [selected, setSelected] = useState<Flow | null>(null);
  const set = (patch: Record<string, string | null>) =>
    setParams(
      (p) => {
        const n = new URLSearchParams(p);
        for (const [k, v] of Object.entries(patch)) {
          if (v) n.set(k, v);
          else n.delete(k);
        }
        return n;
      },
      { replace: true },
    );
  const q = useQuery({
    queryKey: ['network', 'flows', range, query, clientGroupBy, serverGroupBy, traffic],
    queryFn: () => networkFlows({ range, query, clientGroupBy, serverGroupBy, ...traffic, limit: 500 }),
  });
  const terms = useMemo(() => parseQuery(query), [query]);
  const facets = useMemo(
    () => (q.data?.facets ?? []).filter((f) => f.key.startsWith(`${side}_`)).map((f) => ({ key: f.key, label: FACET_LABELS[f.key.slice(side.length + 1)] ?? f.key, group: 'CORE', values: f.values })),
    [q.data, side],
  );
  const cols = COLUMNS.filter((c) => !hidden.has(c.key));

  return (
    <div className="net-page">
      <div className="net-query">
        <QueryBar value={query} onChange={(v) => set({ q: v })} placeholder="Search for traffic by tags, such as client_service:checkout or server_port:443" />
      </div>
      <div className="net-groupby">
        <span className="net-groupby-label">Group by</span>
        <label className="net-chipset">
          <span>Client</span>
          <select value={clientGroupBy} onChange={(e) => set({ client: e.target.value === 'service' ? null : e.target.value })}>
            {GROUP_KEYS.map((g) => (
              <option key={g.key} value={g.key}>
                {g.label}
              </option>
            ))}
          </select>
        </label>
        <label className="net-chipset">
          <span>Server</span>
          <select value={serverGroupBy} onChange={(e) => set({ server: e.target.value === 'service' ? null : e.target.value })}>
            {GROUP_KEYS.map((g) => (
              <option key={g.key} value={g.key}>
                {g.label}
              </option>
            ))}
          </select>
        </label>
      </div>

      {q.isError ? (
        <ErrorState error={q.error} onRetry={() => q.refetch()} />
      ) : !q.data ? (
        <Loading label="Loading network traffic…" />
      ) : !q.data.enabled ? (
        <Empty>No network data yet. Enable Cloud Network Monitoring in the Datadog Agent (system-probe with network_config enabled) and point it at this intake.</Empty>
      ) : (
        <>
          <div className="net-graphs">
            {graphs.map((g, i) => (
              <SummaryGraph
                key={i}
                page={q.data}
                metric={g}
                onMetric={(m) => setGraphs((gs) => gs.map((x, j) => (j === i ? m : x)))}
                onRemove={() => setGraphs((gs) => gs.filter((_, j) => j !== i))}
              />
            ))}
            {graphs.length < 3 && (
              <button className="net-add-graph" onClick={() => setGraphs((gs) => [...gs, 'volume_received'])} title="Add graph">
                <Plus size={18} />
              </button>
            )}
          </div>

          <div className="net-body">
            {showFacets && (
              <div className="net-facets">
                <div className="net-side-tabs">
                  {(['client', 'server'] as const).map((s) => (
                    <button key={s} className={`net-side-tab${side === s ? ' active' : ''}`} onClick={() => setSide(s)}>
                      {s === 'client' ? 'Client' : 'Server'}
                    </button>
                  ))}
                </div>
                <FacetPanel key={side} facets={facets} terms={terms} onChange={(next: Term[]) => set({ q: formatQuery(next) })} valueSearch defaultOpen={[`${side}_service`, `${side}_env`]} />
              </div>
            )}
            <div className="net-results">
              <div className="net-toolbar">
                <button className="net-icon-btn net-facet-toggle" onClick={() => setShowFacets((s) => !s)} title={showFacets ? 'Hide facets' : 'Show facets'}>
                  <PanelLeft size={16} />
                </button>
                <span className="net-count">
                  {q.data.total.toLocaleString()} {q.data.total === 1 ? 'dependency' : 'dependencies'}
                </span>
                <span className="muted">{formatBytes(q.data.totalBytes)} sent and received</span>
                <div className="net-customize dd">
                  <button className="btn btn-sm" onClick={() => setCustomize((c) => !c)}>
                    <Columns3 size={14} /> Customize
                  </button>
                  {customize && (
                    <div className="dd-menu net-columns-menu">
                      {COLUMNS.map((c) => (
                        <label key={c.key} className="net-toggle">
                          <input
                            type="checkbox"
                            checked={!hidden.has(c.key)}
                            onChange={() =>
                              setHidden((h) => {
                                const n = new Set(h);
                                if (!n.delete(c.key)) n.add(c.key);
                                return n;
                              })
                            }
                          />
                          <span>{c.label}</span>
                        </label>
                      ))}
                    </div>
                  )}
                </div>
              </div>
              <div className="net-table-wrap">
                <table className="net-table">
                  <thead>
                    <tr>
                      <th>CLIENT</th>
                      <th>SERVER</th>
                      {cols.map((c) => (
                        <th key={c.key} className={c.num ? 'num' : undefined}>
                          {c.label}
                        </th>
                      ))}
                    </tr>
                  </thead>
                  <tbody>
                    {q.data.flows.map((f) => (
                      <tr key={`${f.client}\u0000${f.server}`} className={selected && selected.client === f.client && selected.server === f.server ? 'selected' : ''} onClick={() => setSelected(f)}>
                        <td className={f.client === 'N/A' ? 'muted' : ''}>{f.client}</td>
                        <td className={f.server === 'N/A' ? 'muted' : ''}>
                          {f.external && <Globe size={13} className="net-ext" aria-label="External" />} {f.server}
                        </td>
                        {cols.map((c) => (
                          <td key={c.key} className={c.num ? 'num' : undefined}>
                            {c.render(f)}
                          </td>
                        ))}
                      </tr>
                    ))}
                    {q.data.flows.length === 0 && (
                      <tr>
                        <td colSpan={cols.length + 2} className="net-empty">
                          No traffic matches this query in the selected time range.
                        </td>
                      </tr>
                    )}
                  </tbody>
                </table>
              </div>
            </div>
          </div>
        </>
      )}
      {selected && <FlowPanel flow={selected} base={{ query, clientGroupBy, serverGroupBy }} onClose={() => setSelected(null)} />}
    </div>
  );
}
