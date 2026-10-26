import { useMemo, useState } from 'react';
import { useQuery } from '@tanstack/react-query';
import type { EChartsOption } from 'echarts';
import { useSearchParams } from 'react-router-dom';
import { Columns3, PanelLeft, Plus, X } from 'lucide-react';
import { networkDns, type DnsFlow, type DnsPage, type DnsPoint } from '../../api/network/client';
import { Chart } from '../../components/charts/Chart';
import { FacetPanel } from '../../components/facets/FacetPanel';
import { QueryBar } from '../../components/query/QueryBar';
import { useTime } from '../../components/shell/TimeContext';
import { Empty, ErrorState, Loading } from '../../components/ui/AsyncState';
import { hhmm } from '../../lib/format';
import { formatQuery, parseQuery, type Term } from '../../lib/searchQuery';
import { GROUP_KEYS, TrafficPanel, scopedQuery } from './NetworkAnalytics';
import { useTraffic } from './NetworkLayout';

/** How DNS servers can be shown (Datadog's options). */
const SERVER_GROUPS = [
  { key: 'dns_server', label: 'dns_server' },
  { key: 'host', label: 'host' },
  { key: 'service', label: 'service' },
  { key: 'ip', label: 'ip' },
  { key: 'dns_query', label: 'dns_query' },
];

const FACET_LABELS: Record<string, string> = {
  service: 'Service',
  env: 'Env',
  'availability-zone': 'Availability Zone',
  kube_namespace: 'Namespace',
  pod_name: 'Pod',
  host: 'Host',
  ip: 'IP',
  dns_server: 'DNS Server',
  dns_query: 'DNS Query',
  dns_query_type: 'Query Type',
};

const fmtTime = (us: number | null | undefined) => (us == null ? '—' : us >= 1000 ? `${(us / 1000).toFixed(us >= 100_000 ? 0 : 2)} ms` : `${Math.round(us)} µs`);
const pct = (n: number, of: number) => (of > 0 ? (n / of) * 100 : 0);
const fmtPct = (v: number) => (v === 0 ? '0%' : v < 0.01 ? '<0.01%' : `${v.toFixed(v < 10 ? 2 : 1)}%`);

const GRAPHS: { key: string; label: string; value: (p: DnsPoint, stepS: number) => number | null; fmt: (v: number) => string }[] = [
  { key: 'requests', label: 'DNS Requests', value: (p) => p.requests, fmt: (v) => Math.round(v).toLocaleString() },
  { key: 'rate', label: 'DNS Requests / Second', value: (p, s) => p.requests / s, fmt: (v) => v.toFixed(2) },
  { key: 'response_time', label: 'DNS Response Time', value: (p) => p.responseTimeUs ?? null, fmt: fmtTime },
  { key: 'timeouts', label: 'Timeouts', value: (p) => pct(p.timeouts, p.requests), fmt: fmtPct },
  { key: 'errors', label: 'Errors', value: (p) => pct(p.errors, p.requests), fmt: fmtPct },
  { key: 'failures', label: 'Failures', value: (p) => pct(p.errors + p.timeouts, p.requests), fmt: fmtPct },
];

interface Column {
  key: string;
  label: string;
  render: (f: DnsFlow) => string;
}

/** Datadog's DNS columns; error columns are shares of all DNS responses. */
const COLUMNS: Column[] = [
  { key: 'requests', label: 'DNS REQUESTS', render: (f) => f.requests.toLocaleString() },
  { key: 'rate', label: 'DNS REQUESTS / SECOND', render: (f) => f.requestsPerSec.toFixed(2) },
  { key: 'response_time', label: 'DNS RESPONSE TIME', render: (f) => fmtTime(f.responseTimeUs) },
  { key: 'timeouts', label: 'TIMEOUTS', render: (f) => fmtPct(pct(f.timeouts, f.requests)) },
  { key: 'errors', label: 'ERRORS', render: (f) => fmtPct(pct(f.servfail + f.nxdomain + f.otherErrors, f.requests)) },
  { key: 'servfail', label: 'SERVFAIL', render: (f) => fmtPct(pct(f.servfail, f.requests)) },
  { key: 'nxdomain', label: 'NXDOMAIN', render: (f) => fmtPct(pct(f.nxdomain, f.requests)) },
  { key: 'other', label: 'OTHER', render: (f) => fmtPct(pct(f.otherErrors, f.requests)) },
  { key: 'failures', label: 'FAILURES', render: (f) => fmtPct(pct(f.servfail + f.nxdomain + f.otherErrors + f.timeouts, f.requests)) },
];

function DnsGraph({ page, metric, onMetric, onRemove }: { page: DnsPage; metric: string; onMetric: (m: string) => void; onRemove: () => void }) {
  const m = GRAPHS.find((x) => x.key === metric) ?? GRAPHS[0];
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
          {GRAPHS.map((x) => (
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

/** Flows tab of a DNS row: the domains and query types behind it. */
function DnsQueriesTab({ query }: { query: string }) {
  const { range } = useTime();
  const q = useQuery({
    queryKey: ['network', 'dns', 'detail', range, query],
    queryFn: () => networkDns({ range, query, clientGroupBy: 'ip', serverGroupBy: 'dns_query', showUnresolved: true, limit: 200 }),
  });
  if (q.isPending) return <Loading label="Loading DNS queries…" />;
  if (q.isError) return <ErrorState error={q.error} onRetry={() => q.refetch()} />;
  return (
    <table className="net-mini">
      <thead>
        <tr>
          <th>CLIENT IP</th>
          <th>DNS QUERY</th>
          <th className="num">DNS REQUESTS</th>
          <th className="num">DNS RESPONSE TIME</th>
          <th className="num">NXDOMAIN</th>
          <th className="num">FAILURES</th>
        </tr>
      </thead>
      <tbody>
        {q.data.flows.map((f) => (
          <tr key={`${f.client}-${f.server}`}>
            <td className="mono">{f.client}</td>
            <td className="mono net-ellipsis" title={f.server}>
              {f.server}
            </td>
            <td className="num">{f.requests.toLocaleString()}</td>
            <td className="num">{fmtTime(f.responseTimeUs)}</td>
            <td className="num">{fmtPct(pct(f.nxdomain, f.requests))}</td>
            <td className="num">{fmtPct(pct(f.requests - f.noerror, f.requests))}</td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}

/** Cloud Network Monitoring → Analytics → DNS: requests from clients to the DNS servers they query. */
export function DnsAnalytics() {
  const { range } = useTime();
  const { showUnresolved } = useTraffic();
  const [params, setParams] = useSearchParams();
  const query = params.get('q') ?? '';
  const clientGroupBy = params.get('client') ?? 'service';
  const serverGroupBy = params.get('server') ?? 'dns_server';
  const [graphs, setGraphs] = useState(['requests', 'response_time', 'failures']);
  const [side, setSide] = useState<'client' | 'server'>('client');
  const [showFacets, setShowFacets] = useState(true);
  const [hidden, setHidden] = useState<Set<string>>(new Set());
  const [customize, setCustomize] = useState(false);
  const [selected, setSelected] = useState<DnsFlow | null>(null);
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
    queryKey: ['network', 'dns', range, query, clientGroupBy, serverGroupBy, showUnresolved],
    queryFn: () => networkDns({ range, query, clientGroupBy, serverGroupBy, showUnresolved, limit: 500 }),
  });
  const terms = useMemo(() => parseQuery(query), [query]);
  const facets = useMemo(
    () => (q.data?.facets ?? []).filter((f) => f.key.startsWith(`${side}_`)).map((f) => ({ key: f.key, label: FACET_LABELS[f.key.slice(side.length + 1)] ?? f.key, group: 'CORE', values: f.values })),
    [q.data, side],
  );
  const cols = COLUMNS.filter((c) => !hidden.has(c.key));

  return (
    <>
      <div className="net-query">
        <QueryBar value={query} onChange={(v) => set({ q: v })} placeholder="Search DNS traffic by tags, such as client_service:checkout or server_dns_query:*.example.net" />
      </div>
      <div className="net-groupby">
        <label className="net-chipset">
          <span>View clients as</span>
          <select value={clientGroupBy} onChange={(e) => set({ client: e.target.value === 'service' ? null : e.target.value })}>
            {GROUP_KEYS.filter((g) => g.key !== 'port' && g.key !== 'domain').map((g) => (
              <option key={g.key} value={g.key}>
                {g.label}
              </option>
            ))}
          </select>
        </label>
        <label className="net-chipset">
          <span>View servers as</span>
          <select value={serverGroupBy} onChange={(e) => set({ server: e.target.value === 'dns_server' ? null : e.target.value })}>
            {SERVER_GROUPS.map((g) => (
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
        <Loading label="Loading DNS traffic…" />
      ) : !q.data.enabled ? (
        <Empty>No DNS data yet. DNS monitoring needs the Datadog Agent 7.33+ with Cloud Network Monitoring (system-probe) enabled.</Empty>
      ) : (
        <>
          <div className="net-graphs">
            {graphs.map((g, i) => (
              <DnsGraph key={i} page={q.data} metric={g} onMetric={(m) => setGraphs((gs) => gs.map((x, j) => (j === i ? m : x)))} onRemove={() => setGraphs((gs) => gs.filter((_, j) => j !== i))} />
            ))}
            {graphs.length < 3 && (
              <button className="net-add-graph" onClick={() => setGraphs((gs) => [...gs, 'rate'])} title="Add graph">
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
                      {s === 'client' ? 'Client' : 'DNS Server'}
                    </button>
                  ))}
                </div>
                <FacetPanel
                  key={side}
                  facets={facets}
                  terms={terms}
                  onChange={(next: Term[]) => set({ q: formatQuery(next) })}
                  valueSearch
                  defaultOpen={side === 'client' ? ['client_service'] : ['server_dns_server', 'server_dns_query']}
                />
              </div>
            )}
            <div className="net-results">
              <div className="net-toolbar">
                <button className="net-icon-btn" onClick={() => setShowFacets((s) => !s)} title={showFacets ? 'Hide facets' : 'Show facets'}>
                  <PanelLeft size={16} />
                </button>
                <span className="net-count">
                  {q.data.total.toLocaleString()} {q.data.total === 1 ? 'dependency' : 'dependencies'}
                </span>
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
                      <th>DNS SERVER</th>
                      {cols.map((c) => (
                        <th key={c.key} className="num">
                          {c.label}
                        </th>
                      ))}
                    </tr>
                  </thead>
                  <tbody>
                    {q.data.flows.map((f) => (
                      <tr key={`${f.client}\u0000${f.server}`} className={selected && selected.client === f.client && selected.server === f.server ? 'selected' : ''} onClick={() => setSelected(f)}>
                        <td className={f.client === 'N/A' ? 'muted' : ''}>{f.client}</td>
                        <td className={f.server === 'N/A' ? 'muted' : ''}>{f.server}</td>
                        {cols.map((c) => (
                          <td key={c.key} className="num">
                            {c.render(f)}
                          </td>
                        ))}
                      </tr>
                    ))}
                    {q.data.flows.length === 0 && (
                      <tr>
                        <td colSpan={cols.length + 2} className="net-empty">
                          No DNS traffic matches this query in the selected time range.
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
      {selected && <TrafficPanel row={selected} flows={<DnsQueriesTab query={scopedQuery(query, selected, clientGroupBy, serverGroupBy)} />} onClose={() => setSelected(null)} />}
    </>
  );
}
