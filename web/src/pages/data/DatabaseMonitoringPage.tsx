import { useMemo, useState } from 'react';
import { keepPreviousData, useQuery } from '@tanstack/react-query';
import type { EChartsOption } from 'echarts';
import { NavLink, useNavigate, useSearchParams } from 'react-router-dom';
import { Check, Database, Info, PanelLeft, X } from 'lucide-react';
import { databaseInstance, searchDatabases, type DbEngine, type DbSearchPage } from '../../api/databases/client';
import { Chart } from '../../components/charts/Chart';
import { FacetPanel } from '../../components/facets/FacetPanel';
import { QueryBar } from '../../components/query/QueryBar';
import { TimePicker } from '../../components/shell/PageHeader';
import { useTime } from '../../components/shell/TimeContext';
import { Empty, ErrorState, Loading } from '../../components/ui/AsyncState';
import { formatBytes, UsageBar } from '../../components/ui/Pager';
import { hhmm, timeAgo } from '../../lib/format';
import { formatQuery, parseQuery, type Term } from '../../lib/searchQuery';
import { SetupOptions } from '../setup/SetupOptions';
import { ENGINES, engineOptions, engineSetup } from './dataSetup';
import './data.css';

const FACET_LABELS: Record<string, string> = { engine: 'Engine', env: 'Env', cluster: 'Cluster', host: 'Host', service: 'Service' };
const num = (v: number | null | undefined, digits = 0) => (v == null ? '—' : v.toLocaleString(undefined, { maximumFractionDigits: digits }));

function useDatabases(query: string) {
  const { range } = useTime();
  return useQuery({ queryKey: ['databases', 'search', range, query], queryFn: () => searchDatabases({ range, query }), placeholderData: keepPreviousData });
}

/** Data Observability → Database Monitoring: the instances (overview) and how to add more (setup). */
export function DatabaseMonitoringPage({ view }: { view: 'overview' | 'setup' }) {
  const [params, setParams] = useSearchParams();
  const query = params.get('q') ?? '';
  const page = useDatabases(query);
  const setQuery = (q: string) => setParams(q ? { q } : {}, { replace: true });
  // With no database metrics at all, the overview is the setup.
  const showSetup = view === 'setup' || page.data?.enabled === false;
  return (
    <div className="dbm-page">
      <header className="product-header">
        <div className="product-title">
          <Database size={20} />
          <span className="dbm-crumb">Data Observability /</span> Database Monitoring
        </div>
        <nav className="product-tabs">
          <NavLink to="/data" end className={({ isActive }) => `product-tab${isActive && !showSetup ? ' active' : ''}`}>
            Databases
          </NavLink>
          <NavLink to="/data/setup" className={() => `product-tab${showSetup ? ' active' : ''}`}>
            Setup
          </NavLink>
        </nav>
        <div className="dbm-header-right">
          <TimePicker />
        </div>
      </header>
      <div className="dbm-body">
        {page.isError ? (
          <ErrorState error={page.error} onRetry={() => page.refetch()} />
        ) : !page.data ? (
          <Loading label="Loading databases…" />
        ) : showSetup ? (
          <DatabaseSetup page={page.data} />
        ) : (
          <DatabaseOverview page={page.data} query={query} onQuery={setQuery} />
        )}
      </div>
    </div>
  );
}

// ---------------------------------------------------------------- overview

function DatabaseOverview({ page, query, onQuery }: { page: DbSearchPage; query: string; onQuery: (q: string) => void }) {
  const [showFacets, setShowFacets] = useState(true);
  const [selected, setSelected] = useState<string | null>(null);
  const navigate = useNavigate();
  const terms = useMemo(() => parseQuery(query), [query]);
  const facets = useMemo(() => page.facets.map((f) => ({ key: f.key, label: FACET_LABELS[f.key] ?? f.key, group: '', values: f.values })), [page.facets]);
  return (
    <>
      <div className="dbm-query">
        <QueryBar value={query} onChange={onQuery} placeholder="Filter instances, such as engine:postgres env:prod service:orders-db" />
      </div>
      {page.unconfigured.length > 0 && (
        <div className="dbm-notice">
          <Info size={15} />
          {page.unconfigured.length} database {page.unconfigured.length === 1 ? 'server runs' : 'servers run'} on hosts without their integration.
          <button className="link-btn" onClick={() => navigate('/data/setup?tab=unconfigured')}>
            Review unconfigured instances
          </button>
        </div>
      )}
      <div className="dbm-explorer">
        {showFacets && (
          <div className="dbm-facets">
            <FacetPanel facets={facets} terms={terms} onChange={(next: Term[]) => onQuery(formatQuery(next))} valueSearch defaultOpen={['engine', 'env', 'cluster']} verified={[]} />
          </div>
        )}
        <div className="dbm-results">
          <div className="dbm-results-head">
            <button className="icon-btn" onClick={() => setShowFacets((s) => !s)} title={showFacets ? 'Hide facets' : 'Show facets'}>
              <PanelLeft size={16} />
            </button>
            <strong>
              {page.instances.length.toLocaleString()} {page.instances.length === 1 ? 'instance' : 'instances'}
            </strong>
          </div>
          <div className="dbm-table-wrap">
            <table className="dbm-table">
              <thead>
                <tr>
                  <th>ENGINE</th>
                  <th>INSTANCE</th>
                  <th>SERVICE</th>
                  <th>ENV</th>
                  <th className="num">CONNECTIONS</th>
                  <th className="num">THROUGHPUT</th>
                  <th className="num">CACHE HIT</th>
                  <th className="num">REPLICATION LAG</th>
                  <th className="num">SIZE</th>
                  <th>DATABASES</th>
                </tr>
              </thead>
              <tbody>
                {page.instances.map((i) => (
                  <tr key={i.id} className={selected === i.id ? 'selected' : ''} onClick={() => setSelected(i.id)}>
                    <td>
                      <EngineBadge engine={i.engine} version={i.version} />
                    </td>
                    <td className="mono">
                      {i.host}
                      {i.port ? `:${i.port}` : ''}
                    </td>
                    <td>{i.service ?? '—'}</td>
                    <td>{i.env ?? '—'}</td>
                    <td className="num">
                      {i.connections != null && i.maxConnections ? <UsageBar pct={(i.connections / i.maxConnections) * 100} /> : null} {num(i.connections)}
                      {i.maxConnections ? <span className="muted"> / {num(i.maxConnections)}</span> : null}
                    </td>
                    <td className="num">
                      {num(i.throughput, 1)} <span className="muted">{engineSetup(i.engine).throughput}</span>
                    </td>
                    <td className={`num${i.cacheHitPct != null && i.cacheHitPct < 90 ? ' dbm-warn' : ''}`}>{i.cacheHitPct == null ? '—' : `${i.cacheHitPct.toFixed(1)} %`}</td>
                    <td className={`num${i.replicationLagSec != null && i.replicationLagSec > 5 ? ' dbm-warn' : ''}`}>{i.replicationLagSec == null ? '—' : `${i.replicationLagSec.toFixed(1)} s`}</td>
                    <td className="num">{i.sizeBytes == null ? '—' : formatBytes(i.sizeBytes)}</td>
                    <td className="dbm-ellipsis">{i.databases.join(', ') || '—'}</td>
                  </tr>
                ))}
                {page.instances.length === 0 && (
                  <tr>
                    <td colSpan={10} className="dbm-empty-row">
                      No instances match this query.
                    </td>
                  </tr>
                )}
              </tbody>
            </table>
          </div>
        </div>
      </div>
      {selected && <InstancePanel id={selected} onClose={() => setSelected(null)} />}
    </>
  );
}

function EngineBadge({ engine, version }: { engine: DbEngine; version?: string | null }) {
  return (
    <span className={`dbm-engine dbm-engine-${engine}`}>
      {engineSetup(engine).name}
      {version ? <span className="muted"> {version}</span> : null}
    </span>
  );
}

const PANEL_CHARTS: { name: string; title: string; fmt: (v: number) => string; max?: number }[] = [
  { name: 'connections', title: 'Connections', fmt: (v) => num(v) },
  { name: 'throughput', title: 'Throughput', fmt: (v) => num(v, 1) },
  { name: 'cache_hit', title: 'Cache hit ratio', fmt: (v) => `${v.toFixed(1)} %`, max: 100 },
  { name: 'replication_lag', title: 'Replication lag', fmt: (v) => `${v.toFixed(1)} s` },
  { name: 'size', title: 'Size', fmt: formatBytes },
];

function PanelChart({ title, points, fmt, max }: { title: string; points: [number, number][]; fmt: (v: number) => string; max?: number }) {
  const option = useMemo<EChartsOption>(
    () => ({
      grid: { left: 64, right: 10, top: 10, bottom: 22 },
      tooltip: { trigger: 'axis', valueFormatter: (v) => (v == null ? '—' : fmt(Number(v))) },
      xAxis: { type: 'time', splitNumber: 3, axisLabel: { formatter: (v: number) => hhmm(v), hideOverlap: true } },
      yAxis: { type: 'value', max, splitNumber: 3, axisLabel: { formatter: (v: number) => fmt(v) } },
      series: [{ type: 'line', name: title, showSymbol: false, areaStyle: { opacity: 0.12 }, data: points }],
    }),
    [points, title, fmt, max],
  );
  return (
    <div className="dbm-chart">
      <h4>{title}</h4>
      <Chart option={option} height={130} />
    </div>
  );
}

/** Side panel of an instance: its tags, key series, and what isn't collected. */
function InstancePanel({ id, onClose }: { id: string; onClose: () => void }) {
  const { range } = useTime();
  const detail = useQuery({ queryKey: ['databases', 'instance', range, id], queryFn: () => databaseInstance({ range, id }) });
  const i = detail.data?.instance;
  return (
    <aside className="dbm-panel" role="dialog" aria-label="Database instance">
      <header className="dbm-panel-head">
        <h3>
          <Database size={18} /> {id.split(':').slice(1).join(':')}
        </h3>
        <button className="icon-btn" onClick={onClose} title="Close">
          <X size={16} />
        </button>
      </header>
      <div className="dbm-panel-body">
        {detail.isError ? (
          <ErrorState error={detail.error} onRetry={() => detail.refetch()} />
        ) : !detail.data ? (
          <Loading label="Loading instance…" />
        ) : !i ? (
          <Empty>This instance didn't report in the selected time range.</Empty>
        ) : (
          <>
            <div className="dbm-tags">
              <EngineBadge engine={i.engine} version={i.version} />
              {[
                ['service', i.service],
                ['env', i.env],
                ['cluster', i.cluster],
                ['host', i.host],
              ]
                .filter(([, v]) => v)
                .map(([k, v]) => (
                  <span key={k} className="tag-chip">
                    {k}:{v}
                  </span>
                ))}
              <span className="muted">Last report {timeAgo(i.lastSeen)}</span>
            </div>
            <div className="dbm-panel-charts">
              {PANEL_CHARTS.map((c) => {
                const s = detail.data.series.find((x) => x.name === c.name);
                if (!s || s.points.length === 0) return null;
                return <PanelChart key={c.name} title={c.name === 'throughput' ? `Throughput (${engineSetup(i.engine).throughput})` : c.title} points={s.points} fmt={c.fmt} max={c.max} />;
              })}
            </div>
            <section className="dbm-unavailable">
              <h4>Query samples and explain plans</h4>
              <p className="muted">
                Not available: they need the Agent's Database Monitoring payloads (sampled queries and their plans), which ObserveCraft doesn't collect yet. The metrics above come from the integration
                check or receiver.
              </p>
            </section>
          </>
        )}
      </div>
    </aside>
  );
}

// ---------------------------------------------------------------- setup

const FEATURES: { name: string; available: boolean; text: string }[] = [
  { name: 'Database metrics', available: true, text: 'Connections, throughput, cache hit ratio, replication lag and size for every instance, from the integration check or receiver.' },
  { name: 'Enriched dashboards', available: true, text: 'The integration dashboards in Integrations read the same metrics, so they work as soon as an instance reports.' },
  { name: 'Query samples', available: false, text: 'Samples of running queries need the Agent’s Database Monitoring payloads, which are not collected yet.' },
  { name: 'Explain plans', available: false, text: 'Execution plans come with query samples and are not available yet either.' },
];

function DatabaseSetup({ page }: { page: DbSearchPage }) {
  const [params, setParams] = useSearchParams();
  const tab = params.get('tab') === 'unconfigured' ? 'unconfigured' : 'add';
  const [engine, setEngine] = useState<DbEngine | null>(null);
  const [feature, setFeature] = useState(0);
  const chosen = engine ? engineSetup(engine) : null;
  return (
    <div className="dbm-setup">
      <div className="btn-group">
        <button className={`btn${tab === 'add' ? ' btn-active' : ''}`} onClick={() => setParams({}, { replace: true })}>
          Add Database Instance
        </button>
        <button className={`btn${tab === 'unconfigured' ? ' btn-active' : ''}`} onClick={() => setParams({ tab: 'unconfigured' }, { replace: true })}>
          Unconfigured Instances {page.unconfigured.length > 0 && <span className="dbm-count">{page.unconfigured.length}</span>}
        </button>
      </div>
      {tab === 'unconfigured' ? (
        <section className="dbm-card">
          <h3>Database servers without their integration</h3>
          <p className="muted">Hosts that report a database server process but no metrics for that engine. Set up its check or receiver to see the instance here.</p>
          {page.unconfigured.length === 0 ? (
            <Empty>Every database server seen in host processes is configured.</Empty>
          ) : (
            <table className="dbm-table">
              <thead>
                <tr>
                  <th>HOST</th>
                  <th>ENGINE</th>
                  <th>PROCESS</th>
                  <th />
                </tr>
              </thead>
              <tbody>
                {page.unconfigured.map((u) => (
                  <tr key={`${u.host}-${u.engine}`}>
                    <td className="mono">{u.host}</td>
                    <td>
                      <EngineBadge engine={u.engine} />
                    </td>
                    <td className="mono">{u.process}</td>
                    <td className="num">
                      <button
                        className="btn btn-sm"
                        onClick={() => {
                          setEngine(u.engine);
                          setParams({}, { replace: true });
                        }}
                      >
                        Set up
                      </button>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          )}
        </section>
      ) : (
        <>
          <p className="muted">Monitor your databases from the metrics their Datadog Agent check or OpenTelemetry receiver sends: no extra agent, no query collection.</p>
          <section className="dbm-card dbm-tour">
            <ul>
              {FEATURES.map((f, i) => (
                <li key={f.name}>
                  <button className={i === feature ? 'active' : ''} onClick={() => setFeature(i)}>
                    {f.available ? <Check size={14} /> : <X size={14} />} {f.name}
                  </button>
                </li>
              ))}
            </ul>
            <div>
              <h4>
                {FEATURES[feature].name} {!FEATURES[feature].available && <span className="dbm-na">Not available</span>}
              </h4>
              <p>{FEATURES[feature].text}</p>
            </div>
          </section>
          <section className="dbm-card dbm-guided">
            <ol className="dbm-rail">
              <li className={chosen ? 'done' : 'current'}>Database</li>
              <li className={chosen ? 'current' : ''}>Collector</li>
              <li className={chosen ? 'current' : ''}>Configure and verify</li>
            </ol>
            <div className="dbm-guided-body">
              <h3>Select database</h3>
              <div className="dbm-engines">
                {ENGINES.map((e) => (
                  <button key={e.engine} className={`setup-tile${engine === e.engine ? ' selected' : ''}`} onClick={() => setEngine(e.engine)}>
                    <strong>{e.name}</strong>
                  </button>
                ))}
              </div>
              {chosen && (
                <>
                  <h3>Set up {chosen.name}</h3>
                  <SetupOptions key={chosen.engine} options={engineOptions(chosen)} />
                </>
              )}
            </div>
          </section>
        </>
      )}
    </div>
  );
}
