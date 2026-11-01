import { useMemo, useState } from 'react';
import { useQuery } from '@tanstack/react-query';
import type { EChartsOption } from 'echarts';
import { useNavigate, useSearchParams } from 'react-router-dom';
import { ChartLine, ChartNoAxesGantt, ChevronDown, ChevronRight, ChevronsUpDown, Flame, GitFork, ListFilter, PanelLeftClose, PanelLeftOpen, Rows3, Search, Star } from 'lucide-react';
import { flameGraph, profileInsights, profileOverview, searchProfiles, type FlameGraph, type ProfileMetric, type ProfileOverview } from '../../../api/profiles/client';
import { Chart } from '../../../components/charts/Chart';
import { BrandLogo, hasBrandLogo } from '../../../components/brand/BrandLogo';
import { FacetPanel } from '../../../components/facets/FacetPanel';
import { QueryBar } from '../../../components/query/QueryBar';
import { useTime } from '../../../components/shell/TimeContext';
import { Empty, ErrorState, Loading } from '../../../components/ui/AsyncState';
import { formatLongDate, hhmm } from '../../../lib/format';
import { addTerm, formatQuery, parseQuery, type Term } from '../../../lib/searchQuery';
import { ServiceGlyph } from '../explorer/SpanBits';
import { useStars } from '../visuals';
import { FlameGraphView, viewTree, type GroupBy } from './FlameGraph';
import { CATEGORY_ORDER, LANGUAGES, averageLabel, chartLabel, chartValue, fmtCard, fmtChart, fmtValue, frameLabel, isLibrary, selectedService } from './format';
import { ProfilerIntro } from './ProfilerIntro';

type View = 'flame' | 'timeline' | 'metrics' | 'list' | 'callgraph';

const VIEWS: { key: View; label: string; icon: typeof Flame }[] = [
  { key: 'flame', label: 'Flame Graph', icon: Flame },
  { key: 'timeline', label: 'Thread Timeline', icon: ChartNoAxesGantt },
  { key: 'metrics', label: 'Metrics', icon: ChartLine },
  { key: 'list', label: 'Profile List', icon: Rows3 },
  { key: 'callgraph', label: 'Call Graph', icon: GitFork },
];

const FACET_LABELS: Record<string, string> = { env: 'Env', service: 'Service', version: 'Version', host: 'Host' };

export function LanguageMark({ language, size = 16 }: { language: string; size?: number }) {
  const l = LANGUAGES[language];
  if (!l) return null;
  return hasBrandLogo(l.logo) ? <BrandLogo name={l.logo} size={size} title={l.label} /> : <span className="pf-lang">{l.label}</span>;
}

/** The metric over time: one line per service. */
export function MetricChart({ data, height = 150, marks }: { data: ProfileOverview; height?: number; marks?: EChartsOption['series'] }) {
  const option = useMemo<EChartsOption>(() => {
    const m = data.metric;
    return {
      grid: { left: 44, right: 12, top: 10, bottom: 22 },
      tooltip: { trigger: 'axis', valueFormatter: (v) => (m ? fmtChart(Number(v), m) : String(v)) },
      xAxis: { type: 'time', axisLabel: { formatter: (v: number) => hhmm(v) } },
      yAxis: {
        type: 'value',
        splitNumber: 2,
        axisLabel: { formatter: (v: number) => (m && !m.gauge && m.unit !== 'nanoseconds' ? fmtValue(v, m.unit) : m?.gauge ? fmtValue(v, m.unit) : String(Number(v.toFixed(2)))) },
      },
      series: [
        ...data.series.map((s) => ({
          name: s.service,
          type: 'line' as const,
          showSymbol: false,
          lineStyle: { width: 1.5 },
          data: s.points.map(([t, v]) => [t, m ? chartValue(v, m) : v]),
        })),
        ...((marks as never[]) ?? []),
      ],
    };
  }, [data, marks]);
  return <Chart option={option} height={height} />;
}

/** Cards of the sample types, one per category with a type picker; the selected one drives the graph. */
function Cards({ fg, selected, onSelect }: { fg: FlameGraph; selected: string; onSelect: (type: string) => void }) {
  const byCategory = new Map<string, typeof fg.cards>();
  for (const c of fg.cards) byCategory.set(c.metric.category, [...(byCategory.get(c.metric.category) ?? []), c]);
  const cats = [...byCategory.keys()].sort((a, b) => CATEGORY_ORDER.indexOf(a) - CATEGORY_ORDER.indexOf(b));
  return (
    <div className="pf-cards">
      {cats.map((cat) => {
        const cards = byCategory.get(cat)!;
        const current = cards.find((c) => c.metric.type === selected) ?? cards.find((c) => ['cpu-time', 'heap-space', 'wall-time'].includes(c.metric.type)) ?? cards[0];
        return (
          <div key={cat} className={`pf-card${current.metric.type === selected ? ' active' : ''}`} onClick={() => onSelect(current.metric.type)}>
            <span className="pf-card-cat">{cat}</span>
            <div>
              <div className="pf-card-type">
                {cards.length > 1 ? (
                  <select value={current.metric.type} onClick={(e) => e.stopPropagation()} onChange={(e) => onSelect(e.target.value)}>
                    {cards.map((c) => (
                      <option key={c.metric.type} value={c.metric.type}>
                        {c.metric.label}
                      </option>
                    ))}
                  </select>
                ) : (
                  current.metric.label
                )}
              </div>
              <div className="pf-card-value">{fmtCard(current.value, current.metric)}</div>
            </div>
          </div>
        );
      })}
    </div>
  );
}

/** Right of the flame graph: the metric split by endpoint and by function, grouped by package. */
function Breakdown({ fg, endpoint, onEndpoint }: { fg: FlameGraph; endpoint: string | null; onEndpoint: (e: string | null) => void }) {
  const [fnSearch, setFnSearch] = useState('');
  const [epSearch, setEpSearch] = useState('');
  const [open, setOpen] = useState<Set<string>>(new Set());
  const m = fg.metric!;
  const scale = fg.scale;
  const epMax = Math.max(1, ...fg.endpoints.map((e) => e.value));
  const groups = useMemo(() => {
    const out = new Map<string, { value: number; fns: typeof fg.functions }>();
    for (const f of fg.functions) {
      if (fnSearch && !`${f.name} ${f.package}`.toLowerCase().includes(fnSearch.toLowerCase())) continue;
      const key = isLibrary(f.package, f.file) ? f.package : `fn:${f.name}@${f.file}`;
      const g = out.get(key) ?? { value: 0, fns: [] };
      g.value += f.value;
      g.fns.push(f);
      out.set(key, g);
    }
    return [...out].sort((a, b) => b[1].value - a[1].value).slice(0, 60);
  }, [fg, fnSearch]);
  const fnMax = Math.max(1, ...groups.map(([, g]) => g.value));
  const bar = (v: number, max: number) => (
    <span className="pf-bar">
      <i style={{ width: `${(v / max) * 100}%` }} />
    </span>
  );
  return (
    <aside className="pf-breakdown">
      <h4>{averageLabel(m)}</h4>
      <div className="pf-bd-head">
        <input type="checkbox" checked readOnly />
        <span className="pf-bd-select">For Endpoints</span>
        <label className="pf-bd-search">
          <Search size={14} />
          <input value={epSearch} onChange={(e) => setEpSearch(e.target.value)} placeholder="Search endpoints" />
        </label>
      </div>
      {fg.endpoints
        .filter((e) => !epSearch || e.name.toLowerCase().includes(epSearch.toLowerCase()))
        .map((e) => {
          const [verb, ...rest] = e.name.split(' ');
          const isVerb = rest.length > 0 && /^[A-Z]+$/.test(verb);
          return (
            <button key={e.name || '(other)'} className={`pf-bd-row${endpoint === e.name ? ' active' : ''}`} onClick={() => onEndpoint(endpoint === e.name ? null : e.name)}>
              <input type="checkbox" checked={endpoint == null || endpoint === e.name} readOnly />
              <span className="pf-bd-name">
                {e.name === '' ? (
                  'Other activity'
                ) : isVerb ? (
                  <>
                    <span className="pf-verb">{verb}</span> {rest.join(' ')}
                  </>
                ) : (
                  e.name
                )}
              </span>
              <span className="pf-bd-val">{fmtValue(e.value * scale, m.unit)}</span>
              {bar(e.value, epMax)}
            </button>
          );
        })}
      <div className="pf-bd-head pf-bd-sep">
        <input type="checkbox" checked readOnly />
        <span className="pf-bd-select">Function</span>
        <label className="pf-bd-search">
          <Search size={14} />
          <input value={fnSearch} onChange={(e) => setFnSearch(e.target.value)} placeholder="Search function" />
        </label>
      </div>
      {groups.map(([key, g]) => {
        const isPkg = !key.startsWith('fn:');
        const expanded = open.has(key);
        return (
          <div key={key}>
            <button
              className="pf-bd-row"
              onClick={() =>
                isPkg &&
                setOpen((s) => {
                  const n = new Set(s);
                  if (!n.delete(key)) n.add(key);
                  return n;
                })
              }
            >
              <input type="checkbox" checked readOnly />
              <span className="pf-bd-name">
                {isPkg ? (
                  <>
                    <ChevronRight size={13} className={expanded ? 'rot90' : ''} /> {key}
                  </>
                ) : (
                  frameLabel(g.fns[0].name, g.fns[0].file)
                )}
              </span>
              <span className="pf-bd-val">{fmtValue(g.value * scale, m.unit)}</span>
              {bar(g.value, fnMax)}
            </button>
            {isPkg &&
              expanded &&
              g.fns.map((f) => (
                <div key={`${f.name}@${f.file}`} className="pf-bd-row pf-bd-sub" title={f.file}>
                  <span className="pf-bd-name">{frameLabel(f.name, f.file)}</span>
                  <span className="pf-bd-val">{fmtValue(f.value * scale, m.unit)}</span>
                </div>
              ))}
          </div>
        );
      })}
    </aside>
  );
}

/** The flame graph of the selected service: cards, toolbar, graph and breakdown. */
function ServiceFlame({ query, sampleType, setSampleType }: { query: string; sampleType: string; setSampleType: (t: string) => void }) {
  const { range } = useTime();
  const [endpoint, setEndpoint] = useState<string | null>(null);
  const [filter, setFilter] = useState('');
  const [expandAll, setExpandAll] = useState(false);
  const [groupBy, setGroupBy] = useState<GroupBy>('function');
  const q = useQuery({ queryKey: ['profiles', 'flame', range, query, sampleType, endpoint], queryFn: () => flameGraph({ range, query, sampleType, endpoint }) });
  const tree = useMemo(() => (q.data ? viewTree(q.data.root, expandAll, groupBy) : null), [q.data, expandAll, groupBy]);
  if (q.isPending) return <Loading label="Merging profiles…" />;
  if (q.isError) return <ErrorState error={q.error} onRetry={() => q.refetch()} />;
  const fg = q.data;
  if (!fg.metric || fg.profiles === 0) return <Empty>No profiles match this query in the selected time range.</Empty>;
  return (
    <>
      <Cards fg={fg} selected={fg.sampleType} onSelect={setSampleType} />
      <div className="pf-flame-area">
        <div className="pf-flame-main">
          <div className="pf-flame-toolbar">
            <label className="pf-flame-filter">
              <ListFilter size={15} />
              <input value={filter} onChange={(e) => setFilter(e.target.value)} placeholder="Filter flame graph" />
            </label>
            <span className="pf-flame-tools">
              <button className="pf-link-btn" onClick={() => setExpandAll((x) => !x)}>
                <ChevronsUpDown size={15} /> {expandAll ? 'Collapse' : 'Expand All'}
              </button>
              <label className="pf-field pf-field-sm">
                <span>Group by</span>
                <select value={groupBy} onChange={(e) => setGroupBy(e.target.value as GroupBy)}>
                  <option value="function">Function</option>
                  <option value="package">Package</option>
                </select>
              </label>
              <label className="pf-field pf-field-sm">
                <span>Color by</span>
                <select value="total" disabled>
                  <option value="total">Total {fg.metric.label}</option>
                </select>
              </label>
            </span>
          </div>
          {tree && <FlameGraphView tree={tree} scale={fg.scale} unit={fg.metric.unit} title={averageLabel(fg.metric)} filter={filter} />}
        </div>
        <Breakdown fg={fg} endpoint={endpoint} onEndpoint={setEndpoint} />
      </div>
    </>
  );
}

/** "Select service to view flame graph": services by their share of the metric. */
function ServiceTable({ data, onPick }: { data: ProfileOverview; onPick: (s: string) => void }) {
  const [search, setSearch] = useState('');
  const { stars, toggle } = useStars();
  const total = data.services.reduce((a, s) => a + s.value, 0) || 1;
  const m = data.metric;
  const rows = data.services.filter((s) => s.service.toLowerCase().includes(search.toLowerCase())).sort((a, b) => Number(stars.has(b.service)) - Number(stars.has(a.service)));
  return (
    <div className="pf-services">
      <div className="pf-services-head">
        <h3>Select service to view flame graph</h3>
        <label className="pf-bd-search pf-services-search">
          <Search size={15} />
          <input value={search} onChange={(e) => setSearch(e.target.value)} placeholder="Search service" />
        </label>
      </div>
      <table className="pf-table">
        <thead>
          <tr>
            <th className="pf-col-icon" />
            <th>SERVICE</th>
            <th className="num">TOTAL {m ? chartLabel(m).toUpperCase() : ''}</th>
          </tr>
        </thead>
        <tbody>
          {rows.map((s) => {
            const pct = (s.value / total) * 100;
            return (
              <tr key={s.service} className="pf-click" onClick={() => onPick(s.service)}>
                <td className="pf-col-icon">
                  <ChevronRight size={16} className="muted" />
                  <button
                    className={`pf-star${stars.has(s.service) ? ' on' : ''}`}
                    onClick={(e) => {
                      e.stopPropagation();
                      toggle(s.service);
                    }}
                    title="Star service"
                  >
                    <Star size={16} />
                  </button>
                </td>
                <td>
                  <span className="pf-svc">
                    <ServiceGlyph service={s.service} size={20} /> {s.service} <LanguageMark language={s.language} size={18} />
                  </span>
                </td>
                <td className="num">
                  <span className="pf-share">
                    {pct < 0.1 ? '< 0.1%' : `${pct.toFixed(pct < 10 ? 2 : 1)}%`}
                    <span className="pf-bar pf-bar-lg">
                      <i style={{ width: `${pct}%` }} />
                    </span>
                  </span>
                </td>
              </tr>
            );
          })}
          {rows.length === 0 && (
            <tr>
              <td colSpan={3} className="pf-empty">
                No profiled services in the selected time range.
              </td>
            </tr>
          )}
        </tbody>
      </table>
    </div>
  );
}

/** Metrics view: every sample type of the selection over time. */
function MetricsView({ query, metrics }: { query: string; metrics: ProfileMetric[] }) {
  return (
    <div className="pf-metrics">
      {metrics.map((m) => (
        <MetricPanel key={m.type} query={query} metric={m} />
      ))}
    </div>
  );
}

function MetricPanel({ query, metric }: { query: string; metric: ProfileMetric }) {
  const { range } = useTime();
  const q = useQuery({ queryKey: ['profiles', 'overview', range, query, metric.type], queryFn: () => profileOverview({ range, query, sampleType: metric.type }) });
  return (
    <div className="pf-metric-panel">
      <h4>{chartLabel(metric)}</h4>
      {q.data ? <MetricChart data={q.data} height={140} /> : <Loading label="Loading…" />}
    </div>
  );
}

/** Profile List view: the profiles in the range, newest first. */
function ProfileListView({ query }: { query: string }) {
  const { range } = useTime();
  const q = useQuery({ queryKey: ['profiles', 'list', range, query], queryFn: () => searchProfiles({ range, query, limit: 200 }) });
  if (q.isPending) return <Loading label="Loading profiles…" />;
  if (q.isError) return <ErrorState error={q.error} onRetry={() => q.refetch()} />;
  const v = (t: Record<string, number>, ...keys: string[]) => keys.map((k) => t[k]).find((x) => x != null);
  return (
    <table className="pf-table pf-list">
      <thead>
        <tr>
          <th>DATE</th>
          <th>SERVICE</th>
          <th>HOST</th>
          <th>VERSION</th>
          <th>RUNTIME ID</th>
          <th className="num">CPU TIME</th>
          <th className="num">WALL TIME</th>
          <th className="num">HEAP LIVE SIZE</th>
        </tr>
      </thead>
      <tbody>
        {q.data.items.map((p) => {
          const cpu = v(p.totals, 'cpu-time', 'cpu');
          const wall = v(p.totals, 'wall-time', 'wall');
          const heap = v(p.totals, 'heap-space', 'inuse_space');
          return (
            <tr key={`${p.runtimeId}-${p.start}-${p.attachment}`}>
              <td>{formatLongDate(p.start)}</td>
              <td>
                <span className="pf-svc">
                  {p.service} <LanguageMark language={p.language} size={15} />
                </span>
              </td>
              <td className="mono">{p.host || '—'}</td>
              <td>{p.version || '—'}</td>
              <td className="mono pf-ellipsis">{p.runtimeId || '—'}</td>
              <td className="num">{cpu != null ? fmtValue(cpu, 'nanoseconds') : '—'}</td>
              <td className="num">{wall != null ? fmtValue(wall, 'nanoseconds') : '—'}</td>
              <td className="num">{heap != null ? fmtValue(heap, 'bytes') : '—'}</td>
            </tr>
          );
        })}
        {q.data.items.length === 0 && (
          <tr>
            <td colSpan={8} className="pf-empty">
              No profiles match this query in the selected time range.
            </td>
          </tr>
        )}
      </tbody>
    </table>
  );
}

/** APM → Profiles → Explorer. */
export function ProfileExplorer() {
  const navigate = useNavigate();
  const { range } = useTime();
  const [params, setParams] = useSearchParams();
  const query = params.get('q') ?? '';
  const view = (params.get('view') as View | null) ?? 'flame';
  const metricType = params.get('metric') ?? '';
  const sampleType = params.get('type') ?? '';
  const service = selectedService(query);
  const [controls, setControls] = useState<boolean | null>(null);
  const showControls = controls ?? !service;
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
  const setQuery = (q: string) => set({ q });

  const overview = useQuery({ queryKey: ['profiles', 'overview', range, query, metricType], queryFn: () => profileOverview({ range, query, sampleType: metricType }) });
  const insights = useQuery({ queryKey: ['profiles', 'insights', range], queryFn: () => profileInsights({ range }) });
  const shownInsights = (insights.data?.items ?? []).filter((i) => i.status === 'for_review' && (!service || i.service === service));
  const terms = useMemo(() => parseQuery(query), [query]);
  const facets = useMemo(() => (overview.data?.facets ?? []).map((f) => ({ key: f.key, label: FACET_LABELS[f.key] ?? f.key, group: 'CORE', values: f.values })), [overview.data]);
  if (!query && overview.data && overview.data.services.length === 0) return <ProfilerIntro />;

  return (
    <div className="pf-explorer">
      <div className="pf-query">
        <QueryBar value={query} onChange={setQuery} placeholder="Filter your profiles" />
        <button className="btn btn-primary pf-compare" onClick={() => navigate(`/apm/profiles/comparison?q=${encodeURIComponent(query)}`)} disabled={!service}>
          Compare To <ChevronDown size={14} />
        </button>
      </div>
      <div className="pf-views">
        <div className="btn-group">
          <span className="btn btn-label">View</span>
          {VIEWS.map(({ key, label, icon: Icon }) => (
            <button key={key} className={`btn${view === key ? ' btn-active' : ''}`} onClick={() => set({ view: key === 'flame' ? null : key })}>
              <Icon size={14} /> {label}
            </button>
          ))}
        </div>
      </div>

      <div className="pf-top">
        <div className="pf-chart">
          <label className="pf-metric-select">
            <span>Metric</span>
            <select value={overview.data?.metric?.type ?? ''} onChange={(e) => set({ metric: e.target.value })}>
              {(overview.data?.metrics ?? []).map((m) => (
                <option key={m.type} value={m.type}>
                  {chartLabel(m)}
                </option>
              ))}
            </select>
          </label>
          {overview.data ? <MetricChart data={overview.data} /> : <div style={{ height: 150 }} />}
        </div>
        <aside className="pf-insights-side">
          <h4>Insights</h4>
          {shownInsights.map((i) => (
            <button key={i.key} className="pf-insight-chip" onClick={() => navigate(`/apm/profiles/insights?service=${encodeURIComponent(i.service)}`)} title={i.description}>
              <LanguageMark language={i.language} size={15} /> {i.title}
            </button>
          ))}
          {shownInsights.length === 0 && <span className="muted">No insights for review.</span>}
        </aside>
      </div>

      <button className="pf-link-btn pf-controls-toggle" onClick={() => setControls(!showControls)}>
        {showControls ? <PanelLeftClose size={15} /> : <PanelLeftOpen size={15} />} {showControls ? 'Hide Controls' : 'Show Controls'}
      </button>

      <div className="pf-body">
        {showControls && overview.data && (
          <FacetPanel facets={facets} terms={terms} onChange={(next: Term[]) => setQuery(formatQuery(next))} mode="exclude" valueSearch defaultOpen={['env', 'service']} />
        )}
        <div className="pf-main">
          {overview.isError ? (
            <ErrorState error={overview.error} onRetry={() => overview.refetch()} />
          ) : view === 'flame' ? (
            service ? (
              <ServiceFlame key={service} query={query} sampleType={sampleType} setSampleType={(t) => set({ type: t })} />
            ) : overview.data ? (
              <ServiceTable data={overview.data} onPick={(s) => setQuery(addTerm(query, 'service', s))} />
            ) : (
              <Loading label="Loading services…" />
            )
          ) : view === 'metrics' ? (
            <MetricsView query={query} metrics={overview.data?.metrics ?? []} />
          ) : view === 'list' ? (
            <ProfileListView query={query} />
          ) : view === 'timeline' ? (
            <Empty>Thread timelines need per-thread timeline events from the profiler, which ObserveCraft doesn't store yet.</Empty>
          ) : (
            <Empty>The call graph isn't available yet; use the flame graph for the same call paths.</Empty>
          )}
        </div>
      </div>
    </div>
  );
}
