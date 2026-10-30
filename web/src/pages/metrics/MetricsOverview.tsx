import { useMemo, useRef, useState, type ReactNode } from 'react';
import { Link, useNavigate } from 'react-router-dom';
import { useQuery } from '@tanstack/react-query';
import { ArrowRight, CalendarClock, FileSearch, ListTree, Network, Pencil, Search, Sigma } from 'lucide-react';
import type { EChartsOption } from 'echarts';
import { catalogStats } from '../../api/metricCatalog/client';
import { listMetricRules, metricRulesOverview } from '../../api/metricRules/client';
import { listLogMetrics } from '../../api/logMetrics/client';
import { listDerivedMetrics } from '../../api/derivedMetrics/client';
import { Chart } from '../../components/charts/Chart';
import { palette, seriesPalette, token } from '../../theme/chartTheme';
import { useMetricCatalog } from '../dashboards/queries';
import { compactCount, distributionBase, isStandard, metricOrigin, ORIGINS, typeGroup, type Origin } from './catalog';
import { searchFleet } from '../../api/fleet/client';
import { squarify } from './squarify';

const MONTH = 30 * 86_400_000;
const SECTIONS = [
  ['overview', 'Overview'],
  ['sources', 'Metric Sources'],
  ['generate', 'Generate Metrics'],
  ['processing', 'Configurable Processing'],
  ['available', 'Available Metrics'],
] as const;

/** Bars of series per bucket, for the "available metrics" column. */
function bars(points: [number, number][], color: string): EChartsOption {
  return {
    grid: { left: 2, right: 2, top: 14, bottom: 2 },
    tooltip: { trigger: 'axis', confine: true, valueFormatter: (v) => Number(v).toLocaleString('en-US') },
    xAxis: { type: 'time', show: false },
    yAxis: { type: 'value', show: false },
    series: [{ type: 'bar', data: points, color, barMaxWidth: 10 }],
  };
}

function FlowRow({ label, value }: { label: string; value: number | undefined }) {
  return (
    <div className="mo-row">
      <span>{label}</span>
      <strong>{value === undefined ? '…' : compactCount(value)}</strong>
      <i className="mo-arrow mo-arrow-out" />
    </div>
  );
}

function Processing({ title, text, to, from, value }: { title: string; text: string; to: string; from?: number; value?: number }) {
  return (
    <div className="mo-card mo-proc">
      <div className="mo-card-head">
        <h4>{title}</h4>
        <Link to={to} title={`Configure ${title.toLowerCase()}`} className="icon-plain">
          <Pencil size={15} />
        </Link>
      </div>
      <p className="muted">{text}</p>
      <div className="mo-flow-line">
        <i className="mo-arrow mo-arrow-in" />
        <span>{from === undefined ? '' : compactCount(from)}</span>
        <i className="mo-line" />
        <strong>{value === undefined ? '…' : compactCount(value)}</strong>
        <i className="mo-arrow mo-arrow-out" />
      </div>
    </div>
  );
}

function Available({ value, label, span, children }: { value: number | undefined; label: string; span: string; children?: ReactNode }) {
  return (
    <div className="mo-avail">
      <i className="mo-arrow mo-arrow-in" />
      <div className="mo-big">
        <strong>{value === undefined ? '…' : compactCount(value)}</strong>
        <span>{label}</span>
        <em>{span}</em>
      </div>
      {children}
    </div>
  );
}

/** Series per origin as a treemap: each box sized by its share, its label scaled to fit it (percent of 1000 × 480 units). */
function OriginTreemap({ origins, value, colors }: { origins: Origin[]; value: (o: Origin) => number; colors: string[] }) {
  const [W, H] = [1000, 480];
  return (
    <>
      {squarify(origins, value, W, H).map(({ item, x, y, w, h }) => {
        const size = Math.max(11, Math.min(80, Math.sqrt(w * h) / 4.5, w / 5.5));
        return (
          <div
            key={item}
            className="mo-tile"
            title={`${item}: ${value(item).toLocaleString('en-US')} series`}
            style={{ left: `${(x / W) * 100}%`, top: `${(y / H) * 100}%`, width: `${(w / W) * 100}%`, height: `${(h / H) * 100}%`, background: colors[ORIGINS.indexOf(item) % colors.length] }}
          >
            <span style={{ fontSize: size * 0.5 }}>{item}</span>
            <strong style={{ fontSize: size }}>{compactCount(value(item))}</strong>
          </div>
        );
      })}
    </>
  );
}

/** Metrics → Overview: where the metrics come from, what processes them and what can be queried. */
export function MetricsOverview() {
  const navigate = useNavigate();
  const range = useMemo(() => ({ from: Date.now() - MONTH, to: Date.now() }), []);
  const hour = useMemo(() => ({ from: Date.now() - 3_600_000, to: Date.now() }), []);
  const catalog = useMetricCatalog();
  const stats = useQuery({ queryKey: ['metrics', 'catalog-stats', 'month'], queryFn: ({ signal }) => catalogStats(range, signal) });
  const recent = useQuery({ queryKey: ['metrics', 'catalog-stats', 'hour'], queryFn: ({ signal }) => catalogStats(hour, signal) });
  const rules = useQuery({ queryKey: ['metrics', 'rules'], queryFn: listMetricRules });
  const overview = useQuery({ queryKey: ['metrics', 'rules-overview', 'hour'], queryFn: ({ signal }) => metricRulesOverview(hour, signal) });
  const logMetrics = useQuery({ queryKey: ['logs', 'metrics'], queryFn: listLogMetrics });
  const derived = useQuery({ queryKey: ['metrics', 'derived'], queryFn: listDerivedMetrics });
  const fleet = useQuery({ queryKey: ['fleet', 'agents', 'metrics-overview'], queryFn: ({ signal }) => searchFleet({ range: hour, query: 'type:datadog_agent' }, signal) });
  const [active, setActive] = useState('overview');
  const [showAgent, setShowAgent] = useState(false);
  const [sourceFilter, setSourceFilter] = useState<'all' | 'standard' | 'custom'>('all');
  const [search, setSearch] = useState('');
  const refs = useRef<Record<string, HTMLElement | null>>({});

  // Series per origin: a distribution's statistics share one set of series, counted once.
  const byOrigin = useMemo(() => {
    const logs = new Set((logMetrics.data ?? []).map((m) => m.name));
    const series = new Map((stats.data?.metrics ?? []).map((m) => [m.name, m.series]));
    const counted = new Set<string>();
    const out = Object.fromEntries(ORIGINS.map((o) => [o, 0])) as Record<Origin, number>;
    for (const m of catalog.data ?? []) {
      const base = distributionBase(m);
      if (counted.has(base)) continue;
      counted.add(base);
      out[metricOrigin(m, logs)] += series.get(base) ?? 0;
    }
    return out;
  }, [catalog.data, stats.data, logMetrics.data]);
  const loaded = stats.isSuccess && catalog.isSuccess;
  const total = (stats.data?.metrics ?? []).reduce((n, m) => n + m.series, 0);
  const distributions = (catalog.data ?? []).filter((m) => typeGroup(m.kind) === 'Distributions');
  const distributionNames = new Set(distributions.map(distributionBase));
  const distributionSeries = (stats.data?.metrics ?? []).filter((m) => distributionNames.has(m.name)).reduce((n, m) => n + m.series, 0);
  const indexing = (rules.data ?? []).filter((r) => r.enabled && r.kind === 'indexing');
  const configured = overview.data?.filtered ?? [];
  const generated = byOrigin.Logs + byOrigin.APM + byOrigin.Events;
  // One bar per day: the most series any hour of it had.
  const timeline = useMemo(() => {
    const days = new Map<number, number>();
    for (const p of stats.data?.timeline ?? []) {
      const day = Math.floor(p.t / 86_400_000) * 86_400_000;
      days.set(day, Math.max(days.get(day) ?? 0, p.series));
    }
    return [...days].sort((a, b) => a[0] - b[0]) as [number, number][];
  }, [stats.data]);
  const indexedNow = (recent.data?.metrics ?? []).reduce((n, m) => n + m.series, 0);
  const jump = (id: string) => refs.current[id]?.scrollIntoView({ behavior: 'smooth', block: 'start' });
  const standard = ORIGINS.filter(isStandard).reduce((n, o) => n + byOrigin[o], 0);
  const custom = total - standard;
  const agentOrigins: Origin[] = ['Agent', 'Agent integrations'];
  const treemap = ORIGINS.filter((o) => byOrigin[o] > 0 && (showAgent || !agentOrigins.includes(o)) && (sourceFilter === 'all' || (sourceFilter === 'standard') === isStandard(o)));
  // One color per origin (ten of them): the chart palette, then two more shades.
  const colors = [...palette(), ...seriesPalette('green').slice(3, 4), ...seriesPalette('orange').slice(3, 4)];
  const integrations = fleet.data?.facets.find((f) => f.key === 'integration')?.values.length ?? 0;
  // The tab of the section at the top of the scrolled page.
  const onScroll = (e: React.UIEvent<HTMLDivElement>) => {
    const top = e.currentTarget.getBoundingClientRect().top + 80;
    let current = 'overview';
    for (const [id] of SECTIONS) if ((refs.current[id]?.getBoundingClientRect().top ?? Infinity) <= top) current = id;
    setActive(current);
  };

  return (
    <div className="mo-page" onScroll={onScroll}>
      <section className="mo-hero" ref={(el) => (refs.current.overview = el)}>
        <div className="mo-hero-head">
          <h1>How your metrics flow through ObserveCraft</h1>
          <span>PAST MONTH</span>
        </div>
        <div className="mo-flow">
          <div className="mo-col">
            <h3>Metric Sources</h3>
            <div className="mo-card">
              <h4>Datadog Agents</h4>
              <p className="muted">{fleet.isSuccess ? `${fleet.data.agents.length.toLocaleString()} active agents` : '…'}</p>
              <p className="muted">{integrations ? `${integrations} active agent integrations` : 'No active agent integrations'}</p>
              <FlowRow label="STANDARD METRICS" value={loaded ? byOrigin.Agent + byOrigin['Agent integrations'] : undefined} />
              <FlowRow label="CUSTOM METRICS" value={loaded ? byOrigin.Custom : undefined} />
            </div>
            <div className="mo-card">
              <h4>Cloud Integrations</h4>
              <p className="muted">AWS, Azure, Google Cloud and other crawled integrations</p>
              <FlowRow label="STANDARD METRICS" value={loaded ? byOrigin['Cloud integrations'] : undefined} />
            </div>
            <div className="mo-card">
              <h4>OpenTelemetry</h4>
              <FlowRow label="CUSTOM METRICS" value={loaded ? byOrigin.OpenTelemetry : undefined} />
            </div>
            <div className="mo-card mo-card-arrow">
              <h4>Logs and APM</h4>
              <p className="muted">Events and traces</p>
              <i className="mo-arrow mo-arrow-out" />
            </div>
          </div>
          <div className="mo-core">
            <img src="/logo.svg" alt="" width={72} height={72} />
            <span>OBSERVECRAFT</span>
          </div>
          <div className="mo-col mo-proc-col">
            <h3>Configurable Processing</h3>
            <Processing
              title="Selected Metrics"
              text="Configure tags to manage data volume"
              to="/metrics/settings/rules"
              from={configured.reduce((n, m) => n + m.ingested, 0)}
              value={overview.isSuccess ? configured.reduce((n, m) => n + m.indexed, 0) : undefined}
            />
            <Processing title="Distribution Metrics" text="Percentiles computed over every host" to="/metrics/catalog" from={distributionNames.size} value={loaded ? distributionSeries : undefined} />
            <Processing title="Logs and APM Metrics" text="Generate metrics from events and traces" to="/logs/generate-metrics" value={loaded ? generated : undefined} />
            <Processing title="Derived Metrics" text="Formulas over existing metrics" to="/metrics/derived" value={derived.data?.length} />
            <div className="mo-card mo-proc mo-other">
              <i className="mo-arrow mo-arrow-in" />
              <h4>All other metrics</h4>
              <strong>{loaded ? compactCount(Math.max(0, total - distributionSeries - configured.reduce((n, m) => n + m.ingested, 0) - generated)) : '…'}</strong>
              <i className="mo-arrow mo-arrow-out" />
            </div>
          </div>
          <div className="mo-col mo-available">
            <h3>Available Metrics</h3>
            <div className="mo-card">
              <Available value={loaded ? total : undefined} label="TOTAL METRICS" span="1mo">
                <div className="mo-bars">{timeline.length > 0 && <Chart option={bars(timeline, token('--chart-4'))} height="100%" />}</div>
              </Available>
              <Available value={loaded ? standard : undefined} label="STANDARD METRICS" span="1mo">
                <div className="mo-bars">
                  {timeline.length > 0 && total > 0 && (
                    <Chart
                      option={bars(
                        timeline.map(([at, v]) => [at, Math.round((v * standard) / total)]),
                        token('--chart-2'),
                      )}
                      height="100%"
                    />
                  )}
                </div>
              </Available>
              <Available value={loaded ? custom : undefined} label="CUSTOM METRICS" span="1mo" />
            </div>
          </div>
        </div>
      </section>

      <nav className="mo-tabs">
        {SECTIONS.map(([id, label], i) => (
          <button key={id} onClick={() => jump(id)} className={`${active === id ? 'active' : ''}${SECTIONS.findIndex(([x]) => x === active) >= i ? ' passed' : ''}`}>
            {label}
          </button>
        ))}
      </nav>

      <section className="mo-section" ref={(el) => (refs.current.sources = el)}>
        <h2>Your metrics by source</h2>
        <div className="mo-treemap-bar">
          <div className="btn-group" role="group" aria-label="Metrics">
            {(
              [
                ['all', 'All Metrics', total],
                ['standard', 'Standard', standard],
                ['custom', 'Custom', custom],
              ] as const
            ).map(([key, label, n]) => (
              <button key={key} type="button" className={`btn btn-sm${sourceFilter === key ? ' btn-active' : ''}`} onClick={() => setSourceFilter(key)}>
                {label} <span className="mo-count">{loaded ? compactCount(n) : ''}</span>
              </button>
            ))}
          </div>
          <label className="mo-agent-toggle">
            <button role="switch" aria-checked={showAgent} className={`switch${showAgent ? ' on' : ''}`} onClick={() => setShowAgent((v) => !v)} />
            Show Agent metrics
          </label>
        </div>
        <div className="mo-treemap">{loaded && <OriginTreemap origins={treemap} value={(o) => byOrigin[o]} colors={colors} />}</div>
      </section>

      <section className="mo-section" ref={(el) => (refs.current.generate = el)}>
        <h2>Generate metrics from any source</h2>
        <p className="muted mo-lead">Create metrics from Logs, APM and Events, or from formulas over the metrics you have.</p>
        <div className="mo-generate">
          <Link className="mo-gen" to="/logs/generate-metrics">
            <FileSearch size={20} /> Logs
          </Link>
          <Link className="mo-gen" to="/apm/traces">
            <Network size={20} /> APM
          </Link>
          <Link className="mo-gen" to="/process/metrics">
            <ListTree size={20} /> Processes
          </Link>
          <Link className="mo-gen" to="/event/settings/generate-metrics">
            <CalendarClock size={20} /> Events
          </Link>
          <Link className="mo-gen" to="/metrics/derived">
            <Sigma size={20} /> Derived
          </Link>
        </div>
      </section>

      <section className="mo-section mo-processing" ref={(el) => (refs.current.processing = el)}>
        <div className="mo-feature">
          <div>
            <h2>Choose the tags that matter the most</h2>
            <p>
              <b>Optimize your analytics.</b> Keep queryable only the tags that are relevant for each metric.
            </p>
            <p>
              <b>Reduce your costs.</b> Strip or drop high-cardinality tags at the intake to cut the number of series stored.
            </p>
            <Link className="btn btn-primary" to="/metrics/settings/rules">
              Configure tags <ArrowRight size={15} />
            </Link>
          </div>
          <div className="mo-feature-card">
            <h4>Tag configurations</h4>
            <p className="muted">
              {indexing.length} indexing {indexing.length === 1 ? 'control' : 'controls'} on {configured.length} {configured.length === 1 ? 'metric' : 'metrics'}
            </p>
            <table className="sx-table">
              <tbody>
                {configured.slice(0, 5).map((m) => (
                  <tr key={m.name}>
                    <td className="mono ellipsis">{m.name}</td>
                    <td className="num">
                      {compactCount(m.ingested)} → <b>{compactCount(m.indexed)}</b>
                    </td>
                  </tr>
                ))}
                {!configured.length && (
                  <tr>
                    <td className="muted">No metric has a tag configuration yet.</td>
                  </tr>
                )}
              </tbody>
            </table>
          </div>
        </div>
        <div className="mo-feature">
          <div>
            <h2>Get more granularity with distribution metrics</h2>
            <p>
              <b>Percentiles.</b> Answer questions like "what is the latency for the worst 1% of requests?"
            </p>
            <p>
              <b>Global accuracy.</b> Histograms and sketches from every host are merged before the percentile is computed.
            </p>
            <Link className="btn btn-primary" to={`/metrics/catalog`}>
              Browse distribution metrics <ArrowRight size={15} />
            </Link>
          </div>
          <div className="mo-feature-card">
            <h4>Distributions</h4>
            <p className="muted">{distributionNames.size} distribution metrics</p>
            <table className="sx-table">
              <tbody>
                {[...distributionNames].slice(0, 5).map((n) => (
                  <tr key={n}>
                    <td className="mono ellipsis">
                      <Link to={`/metrics?metric=${encodeURIComponent(`${n}.p95`)}`}>{n}</Link>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        </div>
      </section>

      <section className="mo-section" ref={(el) => (refs.current.available = el)}>
        <h2>Metrics available for querying</h2>
        <p className="muted mo-lead">Search a metric name to see its recent tags and data</p>
        <form
          className="mo-search"
          onSubmit={(e) => {
            e.preventDefault();
            navigate(`/metrics/catalog?q=${encodeURIComponent(search)}`);
          }}
        >
          <Search size={16} />
          <input placeholder="Search metric" value={search} onChange={(e) => setSearch(e.target.value)} list="mo-metrics" />
          <datalist id="mo-metrics">
            {(catalog.data ?? [])
              .filter((m) => search && m.name.includes(search))
              .slice(0, 50)
              .map((m) => (
                <option key={m.name} value={m.name} />
              ))}
          </datalist>
        </form>
        <div className="mo-indexed">
          Indexed <b>{recent.isSuccess ? indexedNow.toLocaleString('en-US') : '…'}</b> series in the last hour
        </div>
      </section>
    </div>
  );
}
