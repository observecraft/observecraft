import { useMemo, useState } from 'react';
import type { EChartsOption } from 'echarts';
import { DollarSign, Eye, Search, Settings2, Wand2, Wrench } from 'lucide-react';
import type { IngestionService } from '../../../api/apmSettings/client';
import type { ServiceKind } from '../../../api/apm/client';
import { Chart } from '../../../components/charts/Chart';
import { TimePicker } from '../../../components/shell/PageHeader';
import { useTime } from '../../../components/shell/TimeContext';
import { ErrorState, Loading } from '../../../components/ui/AsyncState';
import { ServiceIcon } from '../visuals';
import { bytes, share } from './format';
import { useIngestion } from './queries';

type Tab = 'all' | 'apm' | 'other' | 'otel';
const TABS: [Tab, string][] = [
  ['all', 'All services'],
  ['apm', 'Managed by APM'],
  ['other', 'Managed by other products'],
  ['otel', 'Managed by OTel'],
];

const kindOf = (k: string): ServiceKind => (['web', 'db', 'cache', 'queue', 'http'].includes(k) ? (k as ServiceKind) : 'worker');
const perSecond = (n: number) => `${bytes(n)}/s`;
const traces = (n: number) => `${n >= 100 ? Math.round(n) : n >= 10 ? n.toFixed(1) : n.toFixed(2)} traces/s`;

function ConfigChips({ configs }: { configs: string[] }) {
  if (configs.length > 1) {
    return (
      <span className="aps-chip aps-chip-outline" title={configs.join(', ')}>
        {configs.length} CONFIGURATIONS
      </span>
    );
  }
  const c = configs[0] ?? 'UNKNOWN';
  return <span className={`aps-chip ${c === 'APM LOCAL' ? 'aps-chip-local' : c === 'RUM' ? 'aps-chip-rum' : c === 'OTEL' ? 'aps-chip-otel' : ''}`}>{c}</span>;
}

/** APM → Settings → Ingestion Control: what each service sends, and who decides its sampling. */
export function IngestionControlPage() {
  const { range } = useTime();
  const q = useIngestion(range);
  const [tab, setTab] = useState<Tab>('all');
  const [search, setSearch] = useState('');
  const [env, setEnv] = useState('');
  const [config, setConfig] = useState('');
  const [rootOnly, setRootOnly] = useState(false);
  const all = q.data?.services ?? [];
  const inTab = (s: IngestionService, t: Tab) => t === 'all' || s.managedBy === t;
  const rows = all.filter(
    (s) =>
      inTab(s, tab) &&
      (!search.trim() || s.service.toLowerCase().includes(search.trim().toLowerCase())) &&
      (!env || s.env === env) &&
      (!config || s.configurations.includes(config)) &&
      (!rootOnly || s.root),
  );
  const envs = [...new Set(all.map((s) => s.env).filter(Boolean))].sort();
  const configs = [...new Set(all.flatMap((s) => s.configurations))].sort();
  const secs = Math.max(1, (range.to - range.from) / 1000);
  const option = useMemo<EChartsOption>(() => {
    const o = q.data?.overTime;
    const line = (name: string, data: { t: number; v: number }[], color: string) => ({
      name,
      type: 'line' as const,
      stack: 'bytes',
      areaStyle: { opacity: 0.85 },
      symbol: 'none',
      lineStyle: { width: 1 },
      color,
      data: data.map((p) => [p.t, p.v]),
    });
    return {
      grid: { left: 52, right: 12, top: 14, bottom: 46 },
      tooltip: { trigger: 'axis', valueFormatter: (v) => bytes(Number(v)) },
      legend: { bottom: 0, icon: 'rect', itemWidth: 10, itemHeight: 10 },
      xAxis: { type: 'time', axisLabel: { hideOverlap: true } },
      yAxis: { type: 'value', axisLabel: { formatter: (v: number) => bytes(v) } },
      series: o ? [line('APM', o.apm, '#7b6cf6'), line('Other products', o.other, '#e8457c'), line('OpenTelemetry', o.otel, '#3fb27f')] : [],
    };
  }, [q.data]);
  const downstream = (t: Tab) => all.filter((s) => inTab(s, t)).reduce((a, s) => a + s.downstreamBytesPerSecond, 0);
  return (
    <div className="settings-pad aps-ingestion">
      <div className="aps-title-row">
        <div>
          <h1 className="aps-h1">Ingestion Control</h1>
          <p className="muted">Control how many traces per second your services send.</p>
        </div>
        <div className="aps-title-actions">
          <button className="btn" disabled title="Adaptive sampling needs Remote Configuration, which isn't available yet">
            <Settings2 size={14} /> Manage Adaptive Sampling
          </button>
          <button className="btn" disabled title="Agents receive sampling settings through Remote Configuration, which isn't available yet">
            <Wrench size={14} /> Manage Remote Agent Ingestion
          </button>
          <TimePicker compact />
        </div>
      </div>
      {q.isLoading ? (
        <Loading />
      ) : q.isError || !q.data ? (
        <ErrorState error={q.error} onRetry={() => q.refetch()} />
      ) : (
        <>
          <div className="aps-cards3">
            <section className="aps-box">
              <h4>INGESTION VOLUME</h4>
              <p>
                <b>{bytes(q.data.bytesIngested)}</b> of spans ingested over the selected range.
              </p>
              <div className="aps-volume">
                <div>
                  <span className="muted">Rate</span>
                  <b>{perSecond(q.data.bytesIngested / secs)}</b>
                </div>
                <div>
                  <span className="muted">Projected over 30 days</span>
                  <b>{bytes((q.data.bytesIngested / secs) * 30 * 86_400)}</b>
                </div>
                <div>
                  <span className="muted">Spans</span>
                  <b>{q.data.spansIngested.toLocaleString()}</b>
                </div>
              </div>
              <p className="muted aps-small">Volumes are estimated from the stored span sizes. There is no ingestion allotment: everything sent is kept.</p>
            </section>
            <section className="aps-box">
              <h4>APM ADAPTIVE SAMPLING</h4>
              <p>
                <b>Not available yet.</b> Once tracers receive sampling targets, onboarded services will:
              </p>
              <ul className="aps-benefits">
                <li>
                  <DollarSign size={15} />
                  <span>
                    Stay within a chosen <b>ingestion volume</b>.
                  </span>
                </li>
                <li>
                  <Eye size={15} />
                  <span>
                    Keep visibility into <b>low-traffic services</b>.
                  </span>
                </li>
                <li>
                  <Wand2 size={15} />
                  <span>
                    <b>Adjust sampling automatically</b> as traffic changes.
                  </span>
                </li>
              </ul>
              <button className="btn btn-primary" disabled>
                <Settings2 size={14} /> Manage Adaptive Sampling
              </button>
            </section>
            <section className="aps-box">
              <h4>INGESTION OVER TIME</h4>
              <Chart option={option} height={190} />
            </section>
          </div>
          <h2 className="aps-h2">Ingestion management per service</h2>
          <div className="aps-tabs" role="tablist">
            {TABS.map(([t, label]) => (
              <button key={t} role="tab" aria-selected={tab === t} className={`aps-tab${tab === t ? ' active' : ''}`} onClick={() => setTab(t)}>
                <b>{label}</b>
                <span className="muted">
                  {all.filter((s) => inTab(s, t)).length} services · {perSecond(downstream(t))} downstream
                </span>
              </button>
            ))}
          </div>
          <div className="aps-filters">
            <label className="aps-search">
              <Search size={14} />
              <input placeholder="Filter by service name…" value={search} onChange={(e) => setSearch(e.target.value)} />
            </label>
            <label className="aps-labeled">
              <span>Env</span>
              <select value={env} onChange={(e) => setEnv(e.target.value)}>
                <option value="">All</option>
                {envs.map((e) => (
                  <option key={e}>{e}</option>
                ))}
              </select>
            </label>
            <label className="aps-labeled">
              <span>Configuration</span>
              <select value={config} onChange={(e) => setConfig(e.target.value)}>
                <option value="">All</option>
                {configs.map((c) => (
                  <option key={c}>{c}</option>
                ))}
              </select>
            </label>
            <label className="aps-toggle">
              <input type="checkbox" checked={rootOnly} onChange={(e) => setRootOnly(e.target.checked)} />
              <span className="aps-toggle-track" />
              Root services only
            </label>
          </div>
          <div className="aps-table-wrap">
            <table className="aps-table aps-ingestion-table">
              <thead>
                <tr>
                  <th>TYPE</th>
                  <th>NAME</th>
                  <th className="num">DOWNSTREAM BYTES/S</th>
                  <th>TRAFFIC BREAKDOWN</th>
                  <th className="num">INGESTED BYTES/S</th>
                  <th className="num">INGESTED TRACES/S</th>
                  <th>CONFIGURATION</th>
                  <th>APM ADAPTIVE SAMPLING</th>
                </tr>
              </thead>
              <tbody>
                {rows.map((s) => (
                  <tr key={s.service}>
                    <td>
                      <ServiceIcon name={s.service} kind={kindOf(s.kind)} language={s.language} />
                    </td>
                    <td>
                      <b>{s.service}</b>
                    </td>
                    <td className="num">{s.root ? perSecond(s.downstreamBytesPerSecond) : <span className="muted">not a root</span>}</td>
                    <td>
                      <span className="aps-breakdown">
                        <span>{share(s.trafficShare)}</span>
                        <span className="aps-breakdown-bar">
                          <i style={{ width: `${Math.min(100, s.trafficShare * 100)}%` }} />
                        </span>
                      </span>
                    </td>
                    <td className="num">{perSecond(s.ingestedBytesPerSecond)}</td>
                    <td className="num">{s.root ? traces(s.ingestedTracesPerSecond) : '—'}</td>
                    <td>
                      <ConfigChips configs={s.configurations} />
                    </td>
                    <td>
                      <span className="aps-chip">DISABLED</span>
                    </td>
                  </tr>
                ))}
                {rows.length === 0 && (
                  <tr>
                    <td colSpan={8} className="muted aps-center">
                      No services match.
                    </td>
                  </tr>
                )}
              </tbody>
            </table>
          </div>
        </>
      )}
    </div>
  );
}
