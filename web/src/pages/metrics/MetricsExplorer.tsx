import { useEffect, useMemo, useState } from 'react';
import { Link, useSearchParams } from 'react-router-dom';
import { BarChart3, Check, Grid3x3, Hash, LineChart, PanelLeftClose, PanelLeftOpen, Plus, Search, Sigma, Table2, Upload, X } from 'lucide-react';
import type { MetricCatalogEntry } from '../../api/dashboards/types';
import { TimePicker } from '../../components/shell/PageHeader';
import { useTime } from '../../components/shell/TimeContext';
import { expandFormula } from '../../lib/metricFormula';
import { usePopover } from '../../components/ui/Dropdown';
import { useMetricCatalog, useTimeseriesData } from '../dashboards/queries';
import { Chart } from '../../components/charts/Chart';
import { splitSeries } from './split';
import { DashboardContext, formatValue, type DashboardContextValue } from '../dashboards/widgets/context';
import { HeatmapWidget, QueryValueWidget, TableWidget, TimeseriesWidget, ToplistWidget } from '../dashboards/widgets/MetricWidgets';
import { timeAgo } from '../../lib/format';
import type { TimeseriesStyle } from '../../api/dashboards/types';
import { ColorOverride, PalettePicker, SourcePicker } from './ExplorerControls';
import { KIND_LABEL } from './MetricsLayout';

export const AGGS = ['avg', 'sum', 'min', 'max'] as const;
export type Agg = (typeof AGGS)[number];

export interface Row {
  agg: Agg;
  metric: string;
  /** `env:prod, service:api-gateway` */
  from: string;
  /** `service, host` */
  by: string;
}

export const LETTERS = 'abcdefghij';

/** Row → query text: `avg:metric{env:prod,service:x} by {host}`. */
export function toQuery(r: Row): string {
  const from = r.from
    .split(',')
    .map((s) => s.trim())
    .filter(Boolean)
    .join(',');
  const by = r.by
    .split(',')
    .map((s) => s.trim())
    .filter(Boolean)
    .join(',');
  return `${r.agg}:${r.metric}{${from || '*'}}${by ? ` by {${by}}` : ''}`;
}

/** Metric name picker with search over the catalog. */
export function MetricPicker({ value, catalog, onChange }: { value: string; catalog: MetricCatalogEntry[]; onChange: (m: string) => void }) {
  const { open, setOpen, ref } = usePopover();
  const [text, setText] = useState('');
  const t = text.trim().toLowerCase();
  const shown = catalog.filter((m) => !t || m.name.toLowerCase().includes(t)).slice(0, 200);
  return (
    <div className="dd mx-picker" ref={ref}>
      <button type="button" className="mx-picker-btn" onClick={() => setOpen((o) => !o)}>
        {value || <span className="muted">Select a metric</span>}
      </button>
      {open && (
        <div className="dd-menu dd-left mx-picker-menu">
          <label className="mx-picker-search">
            <Search size={14} />
            <input autoFocus placeholder="Search metrics" value={text} onChange={(e) => setText(e.target.value)} />
          </label>
          <ul>
            {shown.map((m) => (
              <li key={m.name}>
                <button
                  type="button"
                  onClick={() => {
                    onChange(m.name);
                    setOpen(false);
                  }}
                >
                  <span className="mx-picker-name">{m.name}</span>
                  <span className="mx-kind">{KIND_LABEL[m.kind ?? 'gauge'] ?? m.kind}</span>
                </button>
              </li>
            ))}
            {shown.length === 0 && <li className="muted mx-picker-empty">No metric matches.</li>}
          </ul>
        </div>
      )}
    </div>
  );
}

type Visualization = 'timeseries' | 'toplist' | 'query_value' | 'table' | 'heatmap';
const VISUALIZATIONS: { value: Visualization; label: string; icon: typeof LineChart }[] = [
  { value: 'timeseries', label: 'Timeseries', icon: LineChart },
  { value: 'toplist', label: 'Top List', icon: BarChart3 },
  { value: 'query_value', label: 'Query Value', icon: Hash },
  { value: 'table', label: 'Table', icon: Table2 },
  { value: 'heatmap', label: 'Heatmap', icon: Grid3x3 },
];
type Display = 'line' | 'area' | 'bars';

/** Past queries of this browser, newest first (the Explorer's "sessions"). */
const SESSIONS_KEY = 'oc.metrics.sessions';
const SESSIONS_MAX = 30;
interface Session {
  query: string;
  at: number;
}
function readSessions(): Session[] {
  try {
    return JSON.parse(localStorage.getItem(SESSIONS_KEY) ?? '[]') as Session[];
  } catch {
    return [];
  }
}
function saveSession(query: string): Session[] {
  const next = [{ query, at: Date.now() }, ...readSessions().filter((x) => x.query !== query)].slice(0, SESSIONS_MAX);
  try {
    localStorage.setItem(SESSIONS_KEY, JSON.stringify(next));
  } catch {
    // Private windows: sessions just aren't kept.
  }
  return next;
}

/** `avg:metric{from} by {by}` back into a query row; null when it isn't a plain metric query. */
function rowFromQuery(q: string): Row | null {
  const m = /^(avg|sum|min|max):([\w.\-:/]+)\{([^}]*)\}(?:\s+by\s+\{([^}]*)\})?$/.exec(q.trim());
  if (!m) return null;
  return { agg: m[1] as Agg, metric: m[2], from: m[3] === '*' ? '' : m[3], by: m[4] ?? '' };
}

/** Metrics → Explorer: build queries row by row and graph them over the page's time range. */
export function MetricsExplorer() {
  const [params, setParams] = useSearchParams();
  const { range } = useTime();
  const catalog = useMetricCatalog();
  // Starts empty, as Datadog's does; the first query lives in the URL so a graph can be shared or reloaded.
  const [rows, setRows] = useState<Row[]>(() => [{ agg: (params.get('agg') as Agg | null) ?? 'avg', metric: params.get('metric') ?? '', from: params.get('from') ?? '', by: params.get('by') ?? '' }]);
  const [viz, setViz] = useState<Visualization>((params.get('viz') as Visualization | null) ?? 'timeseries');
  const [display, setDisplay] = useState<Display>('line');
  const [style, setStyle] = useState<TimeseriesStyle>({ palette: 'classic', lineType: 'solid', stroke: 'normal', order: 'values', reverse: false, showLabels: true, legend: true });
  const setStyleKey = <K extends keyof TimeseriesStyle>(k: K, v: TimeseriesStyle[K]) => setStyle((x) => ({ ...x, [k]: v }));
  const [overrides, setOverrides] = useState<Record<number, string | undefined>>({});
  const [splitBy, setSplitBy] = useState<'none' | 'group'>('none');
  const [sessions, setSessions] = useState<Session[]>(readSessions);
  const [sessionSearch, setSessionSearch] = useState('');
  const [sidebar, setSidebar] = useState(true);
  const [copied, setCopied] = useState(false);
  useEffect(() => {
    const first = rows[0];
    setParams(
      (p) => {
        const next = new URLSearchParams(p);
        for (const [k, v] of Object.entries({ metric: first.metric, from: first.from, by: first.by, agg: first.agg === 'avg' ? '' : first.agg, viz: viz === 'timeseries' ? '' : viz })) {
          if (v) next.set(k, v);
          else next.delete(k);
        }
        return next;
      },
      { replace: true },
    );
  }, [rows, viz, setParams]);
  const update = (i: number, patch: Partial<Row>) => setRows((rs) => rs.map((r, j) => (j === i ? { ...r, ...patch } : r)));
  const queries = rows.filter((r) => r.metric).map(toQuery);
  // A formula over the query letters ("a / b * 100") is graphed in place of the queries.
  const [formula, setFormula] = useState<string | null>(null);
  const byLetter = Object.fromEntries(rows.flatMap((r, i) => (r.metric ? [[LETTERS[i], toQuery(r)]] : [])));
  const expanded = formula?.trim() ? expandFormula(formula, byLetter) : null;
  const graphed = expanded ? [expanded] : queries;
  // Every graphed query is remembered as a session once it settles.
  const key = graphed.join(' ; ');
  useEffect(() => {
    if (!key) return;
    const timer = setTimeout(() => setSessions(saveSession(key)), 1500);
    return () => clearTimeout(timer);
  }, [key]);
  const grouped = Boolean(expanded) || Boolean(rows.find((r) => r.metric)?.by.trim());
  const split = splitBy === 'group' && viz === 'timeseries' && graphed.length === 1 && grouped;
  const ctx = useMemo<DashboardContextValue>(
    () => ({ range, resolve: (q) => q, resolveSearch: (q) => q, canEdit: false, onEdit: () => {}, onClone: () => {}, onDelete: () => {}, onFullscreen: () => {}, onGroupLayout: () => {} }),
    [range],
  );
  const entry = (name: string) => catalog.data?.find((m) => m.name === name);
  const openSession = (query: string) => {
    const parts = query.split(' ; ');
    const restored = parts.map(rowFromQuery).filter((r): r is Row => r !== null);
    if (restored.length) {
      setRows(restored);
      setFormula(null);
    }
  };
  const vizInfo = VISUALIZATIONS.find((v) => v.value === viz)!;
  const shownSessions = sessions.filter((x) => !sessionSearch || x.query.toLowerCase().includes(sessionSearch.toLowerCase()));
  const widget = () => {
    const request = { query: graphed[0] };
    switch (viz) {
      case 'toplist':
        return <ToplistWidget def={{ type: 'toplist', request, aggregator: 'avg', limit: 25 }} />;
      case 'query_value':
        return <QueryValueWidget def={{ type: 'query_value', request, aggregator: 'avg', precision: 2, formats: [] }} />;
      case 'table':
        return <TableWidget def={{ type: 'table', requests: graphed.map((query) => ({ query })), aggregator: 'avg', limit: 100, searchable: true }} />;
      case 'heatmap':
        return <HeatmapWidget def={{ type: 'heatmap', request }} />;
      default:
        return (
          <TimeseriesWidget
            def={{
              type: 'timeseries',
              display,
              stacked: display !== 'line',
              style,
              requests: graphed.map((query) => ({ query, palette: expanded ? undefined : overrides[rows.findIndex((r) => r.metric && toQuery(r) === query)] })),
            }}
          />
        );
    }
  };
  return (
    <div className={`mx-explorer${sidebar ? '' : ' mx-no-sessions'}`}>
      {sidebar && (
        <aside className="mx-sessions">
          <div className="mx-sessions-head">
            <h3>Sessions</h3>
            <button className="icon-plain" title="Hide sessions" onClick={() => setSidebar(false)}>
              <PanelLeftClose size={16} />
            </button>
          </div>
          <button
            className="btn btn-primary mx-new-session"
            onClick={() => {
              setRows([{ agg: 'avg', metric: '', from: '', by: '' }]);
              setFormula(null);
            }}
          >
            <Plus size={14} /> New Session
          </button>
          <label className="tv-filter">
            <Search size={14} />
            <input placeholder="Search sessions" value={sessionSearch} onChange={(e) => setSessionSearch(e.target.value)} />
          </label>
          <h4>RECENT</h4>
          <ul>
            {shownSessions.map((x) => (
              <li key={x.query}>
                <button className={x.query === key ? 'active' : undefined} onClick={() => openSession(x.query)} title={x.query}>
                  <span className="mono">{x.query}</span>
                  <span className="muted">{timeAgo(x.at)}</span>
                </button>
              </li>
            ))}
            {!shownSessions.length && <li className="muted">Queries you graph show up here.</li>}
          </ul>
        </aside>
      )}
      <div className="mx-page mx-main">
        <div className="section-header">
          <div className="section-header-left">
            {!sidebar && (
              <button className="icon-plain" title="Show sessions" onClick={() => setSidebar(true)}>
                <PanelLeftOpen size={16} />
              </button>
            )}
            <label className="mx-viz">
              <span className="muted">Visualization:</span>
              <vizInfo.icon size={15} />
              <select value={viz} onChange={(e) => setViz(e.target.value as Visualization)}>
                {VISUALIZATIONS.map((v) => (
                  <option key={v.value} value={v.value}>
                    {v.label}
                  </option>
                ))}
              </select>
            </label>
          </div>
          <TimePicker />
        </div>
        <div className="mx-rows">
          {rows.map((r, i) => {
            const m = entry(r.metric);
            return (
              <div key={i} className="mx-row">
                <span className="mx-letter">{LETTERS[i]}</span>
                <SourcePicker />
                <MetricPicker value={r.metric} catalog={catalog.data ?? []} onChange={(metric) => update(i, { metric })} />
                <label className="mx-field">
                  <span className="mx-chip">from</span>
                  <input placeholder="(everywhere)" value={r.from} onChange={(e) => update(i, { from: e.target.value })} list={`mx-tags-${i}`} />
                </label>
                <label className="mx-field">
                  <select className="mx-agg" value={r.agg} onChange={(e) => update(i, { agg: e.target.value as Agg })} title="Space aggregation: how series are combined">
                    {AGGS.map((a) => (
                      <option key={a} value={a}>
                        {a} by
                      </option>
                    ))}
                  </select>
                  <input className="mx-by" placeholder="(everything)" value={r.by} onChange={(e) => update(i, { by: e.target.value })} list={`mx-tags-${i}`} />
                </label>
                <datalist id={`mx-tags-${i}`}>
                  {(m?.tags ?? []).map((t) => (
                    <option key={t} value={t} />
                  ))}
                </datalist>
                <span className="mx-row-tools">
                  <ColorOverride value={overrides[i]} onChange={(p) => setOverrides((o) => ({ ...o, [i]: p }))} />
                </span>
                {rows.length > 1 && (
                  <button className="icon-plain" title="Remove query" onClick={() => setRows((rs) => rs.filter((_, j) => j !== i))}>
                    <X size={16} />
                  </button>
                )}
              </div>
            );
          })}
          {formula !== null && (
            <div className="mx-row">
              <span className="mx-letter mx-formula-letter">Σ</span>
              <input className="mx-formula" value={formula} onChange={(e) => setFormula(e.target.value)} placeholder="a / b * 100" aria-label="Formula" spellCheck={false} />
              {formula.trim() && !expanded && <span className="form-error">Use the query letters, numbers, + - * / and parentheses</span>}
              <button className="icon-plain" title="Remove formula" onClick={() => setFormula(null)}>
                <X size={16} />
              </button>
            </div>
          )}
          <div className="mx-row-actions">
            {rows.length < LETTERS.length && (
              <button className="btn btn-sm" onClick={() => setRows((rs) => [...rs, { agg: 'avg', metric: '', from: '', by: '' }])}>
                <Plus size={14} /> Add Query
              </button>
            )}
            {formula === null && (
              <button className="btn btn-sm" onClick={() => setFormula(rows.length > 1 ? 'a / b' : 'a * 1')}>
                <Sigma size={14} /> Add Formula
              </button>
            )}
          </div>
        </div>
        {viz === 'timeseries' && (
          <div className="mx-display">
            <label>
              <span className="muted">Display:</span>
              <select value={display} onChange={(e) => setDisplay(e.target.value as Display)}>
                <option value="line">Lines</option>
                <option value="area">Areas</option>
                <option value="bars">Bars</option>
              </select>
            </label>
            <label>
              <button role="switch" aria-checked={style.showLabels} className={`switch${style.showLabels ? ' on' : ''}`} onClick={() => setStyleKey('showLabels', !style.showLabels)} />
              Show labels
            </label>
            <label>
              <span className="muted">Style:</span>
              <select value={style.lineType} onChange={(e) => setStyleKey('lineType', e.target.value as TimeseriesStyle['lineType'])}>
                <option value="solid">Solid</option>
                <option value="dashed">Dashed</option>
                <option value="dotted">Dotted</option>
              </select>
            </label>
            <label>
              <span className="muted">Stroke:</span>
              <select value={style.stroke} onChange={(e) => setStyleKey('stroke', e.target.value as TimeseriesStyle['stroke'])}>
                <option value="thin">Thin</option>
                <option value="normal">Normal</option>
                <option value="thick">Thick</option>
              </select>
            </label>
            <span className="mx-display-sep" />
            <label>
              <span className="muted">Color:</span>
              <PalettePicker value={style.palette ?? 'classic'} onChange={(p) => setStyleKey('palette', p)} />
            </label>
            <span className="mx-display-sep" />
            <label>
              <span className="muted">Order by:</span>
              <select value={style.order} onChange={(e) => setStyleKey('order', e.target.value as TimeseriesStyle['order'])}>
                <option value="values">Values</option>
                <option value="tags">Tags</option>
              </select>
            </label>
            <label>
              <button role="switch" aria-checked={style.reverse} className={`switch${style.reverse ? ' on' : ''}`} onClick={() => setStyleKey('reverse', !style.reverse)} />
              Reverse
            </label>
          </div>
        )}
        <div className="mx-share-row">
          {viz === 'timeseries' && (
            <label className="mx-split">
              <select
                value={splitBy}
                onChange={(e) => setSplitBy(e.target.value as 'none' | 'group')}
                disabled={!grouped || graphed.length !== 1}
                title={grouped ? 'One graph per group' : 'Group the query (avg by …) to split it'}
              >
                <option value="none">Split by: None</option>
                <option value="group">Split by: Group</option>
              </select>
            </label>
          )}
          <button
            className="btn btn-sm"
            onClick={() => {
              navigator.clipboard?.writeText(window.location.href);
              setCopied(true);
              setTimeout(() => setCopied(false), 1500);
            }}
          >
            {copied ? <Check size={14} /> : <Upload size={14} />} {copied ? 'Link copied' : 'Share'}
          </button>
        </div>
        <div className={`mx-chart${split ? ' mx-chart-split' : ''}`}>
          {graphed.length > 0 && <div className="mx-chart-title mono">{graphed.join(', ')}</div>}
          {graphed.length === 0 ? (
            <div className="wd-state">Pick a metric to graph it.</div>
          ) : (
            <DashboardContext.Provider value={ctx}>{split ? <SplitGraph query={graphed[0]} /> : <div className="mx-chart-body">{widget()}</div>}</DashboardContext.Provider>
          )}
        </div>
        {rows.map((r) => entry(r.metric)).filter(Boolean).length > 0 && (
          <div className="mx-about">
            {rows.map((r, i) => {
              const m = entry(r.metric);
              return m ? (
                <p key={i}>
                  <Link to={`/metrics/catalog?metric=${encodeURIComponent(m.name)}`}>
                    <b>{m.name}</b>
                  </Link>{' '}
                  <span className="mx-kind">{KIND_LABEL[m.kind ?? 'gauge'] ?? m.kind}</span> {m.unit && <span className="muted">({m.unit})</span>} — {m.description || 'No description.'}
                </p>
              ) : null;
            })}
          </div>
        )}
      </div>
    </div>
  );
}

/** Groups shown as small graphs at most. */
const SPLIT_LIMIT = 24;

/** One small graph per group of the query, the busiest first. */
function SplitGraph({ query }: { query: string }) {
  const { range } = useTime();
  const q = useTimeseriesData([query], range);
  if (q.isPending) return <div className="wd-state">Loading…</div>;
  if (q.isError) return <div className="wd-state wd-error">{q.error.message}</div>;
  const { shown, hidden } = splitSeries(q.data, SPLIT_LIMIT);
  return (
    <>
      <div className="mx-split">
        {shown.map((s) => (
          <div key={s.name} className="mx-split-cell">
            <div className="mx-split-title" title={s.name}>
              {s.name}
            </div>
            <Chart
              height={120}
              option={{
                grid: { left: 40, right: 8, top: 8, bottom: 20 },
                tooltip: { trigger: 'axis', confine: true },
                xAxis: { type: 'time', axisLabel: { fontSize: 10, hideOverlap: true } },
                yAxis: { type: 'value', axisLabel: { fontSize: 10, formatter: (v: number) => formatValue(v, Math.abs(v) < 1 ? 2 : 0) } },
                series: [{ name: s.name, type: 'line', showSymbol: false, lineStyle: { width: 1.2 }, data: s.points }],
              }}
            />
          </div>
        ))}
      </div>
      {hidden > 0 && <p className="muted">{hidden} more groups not shown.</p>}
    </>
  );
}
