import { useMemo, useState } from 'react';
import { keepPreviousData, useQuery } from '@tanstack/react-query';
import type { EChartsOption } from 'echarts';
import { NavLink, useNavigate, useSearchParams } from 'react-router-dom';
import { Cpu, ExternalLink, MoreHorizontal, PanelLeft, X } from 'lucide-react';
import { gpuTimeseries, searchGpus, type GpuDevice, type GpuSearchPage, type GpuSeries, type GpuState } from '../../api/gpu/client';
import { Chart } from '../../components/charts/Chart';
import { FacetPanel } from '../../components/facets/FacetPanel';
import { QueryBar } from '../../components/query/QueryBar';
import { TimePicker } from '../../components/shell/PageHeader';
import { useTime } from '../../components/shell/TimeContext';
import { Menu, Select } from '../../components/ui/Dropdown';
import { ErrorState, Loading } from '../../components/ui/AsyncState';
import { Modal } from '../../components/ui/Modal';
import { formatBytes, UsageBar } from '../../components/ui/Pager';
import { hhmm } from '../../lib/format';
import { addTerm, formatQuery, parseQuery, type Term } from '../../lib/searchQuery';
import { SetupOptions } from '../setup/SetupOptions';
import { GPU_SETUP } from './gpuSetup';
import './gpu.css';

const STATE_LABELS: Record<GpuState, string> = { idle: 'Idle', underutilized: 'Underutilized', active: 'Active', no_data: 'No data' };
const FACET_LABELS: Record<string, string> = { cluster: 'Cluster', host: 'Host', model: 'Model', vendor: 'Vendor', state: 'State', kube_namespace: 'Namespace' };
const GROUPS = [
  { value: 'host', label: 'Host' },
  { value: 'cluster', label: 'Cluster' },
  { value: 'model', label: 'Model' },
  { value: 'kube_namespace', label: 'Namespace' },
  { value: 'fleet', label: 'Whole fleet' },
];

const pct = (v: number | null | undefined) => (v == null ? '—' : `${v.toFixed(1)} %`);
const fmtPower = (w: number) => (w >= 1000 ? `${(w / 1000).toFixed(2)} kW` : `${w.toFixed(0)} W`);
const CHARTS: { metric: string; title: string; fmt: (v: number) => string; max?: number }[] = [
  { metric: 'utilization', title: 'GPU utilization', fmt: (v) => `${v.toFixed(0)} %`, max: 100 },
  { metric: 'memory_used', title: 'GPU memory used', fmt: formatBytes },
  { metric: 'temperature', title: 'Temperature', fmt: (v) => `${v.toFixed(0)} °C` },
  { metric: 'power', title: 'Power draw', fmt: fmtPower },
];

/** The page's query: the search plus the vendor picked in the header. */
function useGpuQuery() {
  const [params, setParams] = useSearchParams();
  const query = params.get('q') ?? '';
  const vendor = params.get('vendor') ?? '';
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
  return { query, vendor, full: vendor ? addTerm(query, 'vendor', vendor) : query, set, params };
}

function useGpus(full: string) {
  const { range } = useTime();
  return useQuery({ queryKey: ['gpu', 'search', range, full], queryFn: () => searchGpus({ range, query: full }), placeholderData: keepPreviousData });
}

/** Infrastructure → GPU Monitoring: Summary (fleet cards and charts) and Explorer (every GPU). */
export function GpuMonitoringPage({ view }: { view: 'summary' | 'explorer' }) {
  const { query, vendor, full, set, params } = useGpuQuery();
  const [setup, setSetup] = useState(false);
  const navigate = useNavigate();
  const page = useGpus(full);
  // Facets count every GPU, whatever the query: the vendor list stays complete.
  const vendors = page.data?.facets.find((f) => f.key === 'vendor')?.values.map((v) => v.value) ?? [];
  const search = params.toString() ? `?${params.toString()}` : '';
  return (
    <div className="gpu-page">
      <header className="product-header">
        <div className="product-title">
          <Cpu size={20} /> GPU Monitoring
        </div>
        <div className="gpu-vendor">
          <Select
            value={vendor}
            options={[{ value: '', label: 'All vendors' }, ...vendors.map((v) => ({ value: v, label: v === 'nvidia' ? 'NVIDIA' : v }))]}
            onChange={(v) => set({ vendor: v || null })}
            title="GPU vendor"
          />
        </div>
        <nav className="product-tabs">
          <NavLink to={`/gpu-monitoring${search}`} end className={({ isActive }) => `product-tab${isActive ? ' active' : ''}`}>
            Summary
          </NavLink>
          <NavLink to={`/gpu-monitoring/explorer${search}`} className={({ isActive }) => `product-tab${isActive ? ' active' : ''}`}>
            Explorer
          </NavLink>
        </nav>
        <Menu
          align="left"
          triggerClassName="product-tab gpu-more"
          trigger={<MoreHorizontal size={18} />}
          items={[
            { label: 'Monitors', onClick: () => navigate('/monitors/create/metric') },
            { label: 'Dashboards', onClick: () => navigate('/dashboards') },
            { label: 'GPU Monitoring Setup', onClick: () => setSetup(true) },
          ]}
        />
      </header>
      <div className="gpu-body">
        {page.data?.enabled && (
          <div className="gpu-toolbar">
            <TimePicker />
          </div>
        )}
        {page.isError ? (
          <ErrorState error={page.error} onRetry={() => page.refetch()} />
        ) : !page.data ? (
          <Loading label="Loading GPUs…" />
        ) : !page.data.enabled ? (
          <GpuEmpty onSetup={() => setSetup(true)} />
        ) : view === 'summary' ? (
          <GpuSummaryView page={page.data} query={full} />
        ) : (
          <GpuExplorer page={page.data} query={query} onQuery={(q) => set({ q: q || null })} />
        )}
      </div>
      {setup && (
        <Modal label="GPU Monitoring setup" onClose={() => setSetup(false)} width={860} height="auto">
          <div className="gpu-setup">
            <header>
              <h2>Send GPU metrics to ObserveCraft</h2>
              <button className="icon-btn" onClick={() => setSetup(false)} title="Close">
                <X size={16} />
              </button>
            </header>
            <SetupOptions options={GPU_SETUP} />
          </div>
        </Modal>
      )}
    </div>
  );
}

function GpuEmpty({ onSetup }: { onSetup: () => void }) {
  return (
    <div className="gpu-empty">
      <Cpu size={88} strokeWidth={1.6} />
      <h2>
        No GPU monitoring
        <br />
        data yet
      </h2>
      <p>See how busy every GPU in your fleet is, which ones sit idle, and which workloads use them, from the GPU metrics your hosts already collect.</p>
      <div className="gpu-empty-sep">
        <span>Setup guide</span>
      </div>
      <button className="btn gpu-empty-btn" onClick={onSetup}>
        GPU Monitoring Setup <ExternalLink size={15} />
      </button>
    </div>
  );
}

function Card({ label, value, sub, tone }: { label: string; value: string; sub?: string; tone?: 'warn' | 'idle' }) {
  return (
    <div className={`gpu-card${tone ? ` gpu-card-${tone}` : ''}`}>
      <span className="gpu-card-label">{label}</span>
      <strong>{value}</strong>
      {sub && <span className="muted">{sub}</span>}
    </div>
  );
}

function SummaryCards({ page }: { page: GpuSearchPage }) {
  const s = page.summary;
  return (
    <div className="gpu-cards">
      <Card label="GPUs" value={s.gpus.toLocaleString()} />
      <Card label="Hosts" value={s.hosts.toLocaleString()} />
      <Card label="Avg utilization" value={pct(s.avgUtilPct)} />
      <Card label="Memory used" value={formatBytes(s.memUsedBytes)} sub={s.memTotalBytes ? `of ${formatBytes(s.memTotalBytes)}` : undefined} />
      <Card label="Idle GPUs" value={s.idle.toLocaleString()} sub="under 5 % utilization" tone={s.idle ? 'idle' : undefined} />
      <Card label="Underutilized" value={s.underutilized.toLocaleString()} sub="under 30 % utilization" tone={s.underutilized ? 'warn' : undefined} />
    </div>
  );
}

function SeriesChart({ title, series, fmt, max }: { title: string; series: GpuSeries[]; fmt: (v: number) => string; max?: number }) {
  const option = useMemo<EChartsOption>(
    () => ({
      grid: { left: 64, right: 12, top: 12, bottom: 24 },
      tooltip: { trigger: 'axis', valueFormatter: (v) => (v == null ? '—' : fmt(Number(v))) },
      xAxis: { type: 'time', axisLabel: { formatter: (v: number) => hhmm(v) } },
      yAxis: { type: 'value', max, splitNumber: 3, axisLabel: { formatter: (v: number) => fmt(v) } },
      series: series.map((s) => ({ name: s.group ?? title, type: 'line', showSymbol: false, data: s.points, areaStyle: series.length === 1 ? { opacity: 0.12 } : undefined })),
    }),
    [series, title, fmt, max],
  );
  return (
    <div className="gpu-chart">
      <h4>{title}</h4>
      {series.every((s) => s.points.length === 0) ? <div className="gpu-chart-empty muted">No data in this range</div> : <Chart option={option} height={170} />}
    </div>
  );
}

function GpuSummaryView({ page, query }: { page: GpuSearchPage; query: string }) {
  const { range } = useTime();
  const navigate = useNavigate();
  const [group, setGroup] = useState('host');
  const groupBy = group === 'fleet' ? null : group;
  const series = useQuery({ queryKey: ['gpu', 'series', range, query, groupBy], queryFn: () => gpuTimeseries({ range, query, groupBy }), placeholderData: keepPreviousData });
  const review = page.devices.filter((d) => d.state === 'idle' || d.state === 'underutilized').sort((a, b) => (a.utilPct ?? 0) - (b.utilPct ?? 0));
  return (
    <>
      <SummaryCards page={page} />
      <div className="gpu-toolbar">
        <span className="muted">Graph by</span>
        <Select value={group} options={GROUPS} onChange={setGroup} />
      </div>
      {series.isError ? (
        <ErrorState error={series.error} onRetry={() => series.refetch()} />
      ) : !series.data ? (
        <Loading label="Loading GPU metrics…" />
      ) : (
        <div className="gpu-charts">
          {CHARTS.map((c) => (
            <SeriesChart key={c.metric} title={c.title} fmt={c.fmt} max={c.max} series={series.data.filter((s) => s.metric === c.metric)} />
          ))}
        </div>
      )}
      <section className="gpu-review">
        <h3>GPUs to review</h3>
        {review.length === 0 ? (
          <p className="muted">Every GPU is busy: none is idle or underutilized right now.</p>
        ) : (
          <>
            <p className="muted">Idle and underutilized GPUs over the last minutes: candidates to consolidate, share or release.</p>
            <GpuTable devices={review} onHost={(h) => navigate(`/gpu-monitoring/explorer?q=${encodeURIComponent(`host:${h}`)}`)} />
          </>
        )}
      </section>
    </>
  );
}

type SortKey = 'host' | 'util' | 'mem' | 'temp' | 'power';

function GpuTable({ devices, onHost }: { devices: GpuDevice[]; onHost?: (host: string) => void }) {
  const [sort, setSort] = useState<{ key: SortKey; desc: boolean }>({ key: 'host', desc: false });
  const value = (d: GpuDevice): number | string => {
    switch (sort.key) {
      case 'util':
        return d.utilPct ?? -1;
      case 'mem':
        return d.memUsedBytes ?? -1;
      case 'temp':
        return d.tempC ?? -1;
      case 'power':
        return d.powerW ?? -1;
      default:
        return `${d.host}/${d.index ?? ''}`;
    }
  };
  const rows = [...devices].sort((a, b) => {
    const [x, y] = [value(a), value(b)];
    const o = typeof x === 'number' && typeof y === 'number' ? x - y : String(x).localeCompare(String(y));
    return sort.desc ? -o : o;
  });
  const th = (key: SortKey, label: string, num = false) => (
    <th className={`gpu-sort${num ? ' num' : ''}`} onClick={() => setSort((s) => ({ key, desc: s.key === key ? !s.desc : key !== 'host' }))}>
      {label}
      {sort.key === key ? (sort.desc ? ' ↓' : ' ↑') : ''}
    </th>
  );
  return (
    <div className="gpu-table-wrap">
      <table className="gpu-table">
        <thead>
          <tr>
            <th>STATE</th>
            {th('host', 'HOST')}
            <th>DEVICE</th>
            <th>MODEL</th>
            <th>CLUSTER</th>
            {th('util', 'UTILIZATION', true)}
            {th('mem', 'MEMORY', true)}
            {th('temp', 'TEMP', true)}
            {th('power', 'POWER', true)}
            <th>WORKLOADS</th>
          </tr>
        </thead>
        <tbody>
          {rows.map((d) => (
            <tr key={d.id} className={`gpu-row-${d.state}`}>
              <td>
                <span className={`gpu-state gpu-state-${d.state}`}>{STATE_LABELS[d.state]}</span>
              </td>
              <td>
                {onHost ? (
                  <button className="link-btn" onClick={() => onHost(d.host)}>
                    {d.host}
                  </button>
                ) : (
                  d.host
                )}
              </td>
              <td className="mono" title={d.uuid ?? undefined}>
                {d.index != null ? `#${d.index}` : ''} {d.uuid ? <span className="muted">{d.uuid.slice(0, 12)}…</span> : null}
              </td>
              <td>{d.model ?? '—'}</td>
              <td>{d.cluster ?? '—'}</td>
              <td className="num">
                {d.utilPct != null && <UsageBar pct={d.utilPct} />} {pct(d.utilPct)}
              </td>
              <td className="num">
                {d.memUsedBytes != null && d.memTotalBytes ? <UsageBar pct={(d.memUsedBytes / d.memTotalBytes) * 100} tone="ok" /> : null} {d.memUsedBytes != null ? formatBytes(d.memUsedBytes) : '—'}
                {d.memTotalBytes ? <span className="muted"> / {formatBytes(d.memTotalBytes)}</span> : null}
              </td>
              <td className="num">{d.tempC != null ? `${d.tempC.toFixed(0)} °C` : '—'}</td>
              <td className="num">{d.powerW != null ? fmtPower(d.powerW) : '—'}</td>
              <td className="gpu-workloads">{d.workloads.length ? d.workloads.join(', ') : <span className="muted">none tagged</span>}</td>
            </tr>
          ))}
          {rows.length === 0 && (
            <tr>
              <td colSpan={10} className="gpu-table-empty">
                No GPUs match this query.
              </td>
            </tr>
          )}
        </tbody>
      </table>
    </div>
  );
}

function GpuExplorer({ page, query, onQuery }: { page: GpuSearchPage; query: string; onQuery: (q: string) => void }) {
  const [showFacets, setShowFacets] = useState(true);
  const terms = useMemo(() => parseQuery(query), [query]);
  const facets = useMemo(() => page.facets.map((f) => ({ key: f.key, label: FACET_LABELS[f.key] ?? f.key, group: '', values: f.values })), [page.facets]);
  return (
    <>
      <SummaryCards page={page} />
      <div className="gpu-query">
        <QueryBar value={query} onChange={onQuery} placeholder="Filter GPUs, such as state:idle cluster:shop-ml model:*L4*" />
      </div>
      <div className="gpu-explorer">
        {showFacets && (
          <div className="gpu-facets">
            <FacetPanel facets={facets} terms={terms} onChange={(next: Term[]) => onQuery(formatQuery(next))} valueSearch defaultOpen={['state', 'cluster', 'model', 'host']} verified={[]} />
          </div>
        )}
        <div className="gpu-results">
          <div className="gpu-results-head">
            <button className="icon-btn" onClick={() => setShowFacets((s) => !s)} title={showFacets ? 'Hide facets' : 'Show facets'}>
              <PanelLeft size={16} />
            </button>
            <strong>
              {page.devices.length.toLocaleString()} {page.devices.length === 1 ? 'GPU' : 'GPUs'}
            </strong>
          </div>
          <GpuTable devices={page.devices} onHost={(h) => onQuery(addTerm(query, 'host', h))} />
        </div>
      </div>
    </>
  );
}
