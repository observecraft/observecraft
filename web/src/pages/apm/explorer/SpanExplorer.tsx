import { useEffect, useMemo, useState } from 'react';
import { Link, useSearchParams } from 'react-router-dom';
import type { EChartsOption } from 'echarts';
import { ArrowRight, BarChart3, ChevronDown, ChevronRight, Gauge, LayoutGrid, LineChart, List, PanelLeftClose, PanelLeftOpen, PieChart, Search, Table2, X } from 'lucide-react';
import type { ExplorerSpan, SpanMeasure, SpanQuery, SpanScope } from '../../../api/apm/client';
import { Chart } from '../../../components/charts/Chart';
import { QueryBar } from '../../../components/query/QueryBar';
import { TimePicker } from '../../../components/shell/PageHeader';
import { useTime } from '../../../components/shell/TimeContext';
import { ErrorState, Loading } from '../../../components/ui/AsyncState';
import { Pager } from '../../../components/ui/Pager';
import { palette } from '../../../theme/chartTheme';
import { Facet, ValuesFacet } from '../../monitors/common';
import { useExploreSpans, useSpanFacets, useSpanSummary } from '../queries';
import { fmtLatency } from '../visuals';
import { TraceView } from '../trace/TraceView';
import { HttpStatus, SpanService } from './SpanBits';
import { Visualization } from './Visualizations';
import './explorer.css';

const PAGE = 50;

type Viz = 'list' | 'timeseries' | 'toplist' | 'bars' | 'table' | 'treemap' | 'pie';
const VIZ: { id: Viz; label: string; icon: typeof List }[] = [
  { id: 'list', label: 'List', icon: List },
  { id: 'timeseries', label: 'Timeseries', icon: LineChart },
  { id: 'toplist', label: 'Top List', icon: BarChart3 },
  { id: 'bars', label: 'Bar Chart', icon: BarChart3 },
  { id: 'table', label: 'Table', icon: Table2 },
  { id: 'treemap', label: 'Tree Map', icon: LayoutGrid },
  { id: 'pie', label: 'Pie Chart', icon: PieChart },
];

export const DIMENSION_LABEL: Record<string, string> = {
  service: 'Service',
  resource: 'Resource',
  env: 'Env',
  status: 'Status',
  http_status: 'HTTP Status Code',
  method: 'HTTP Method',
  kind: 'Span Kind',
  host: 'Host',
  db_system: 'Database',
  version: 'Version',
};
const FACETS = ['status', 'env', 'service', 'resource', 'http_status', 'method', 'kind', 'host', 'db_system', 'version'];
const MEASURES: { id: SpanMeasure; label: string }[] = [
  { id: 'count', label: 'Count of spans' },
  { id: 'errors', label: 'Count of errors' },
  { id: 'avg', label: 'Avg duration' },
  { id: 'p50', label: 'p50 duration' },
  { id: 'p95', label: 'p95 duration' },
  { id: 'p99', label: 'p99 duration' },
  { id: 'max', label: 'Max duration' },
];

/** Span / trace explorer: query, facets, visualizations and the trace side panel. */
export function SpanExplorer() {
  const t = useTime();
  const [params, setParams] = useSearchParams();
  const query = params.get('query') ?? '';
  const [mode, setMode] = useState<'spans' | 'traces'>('spans');
  const [scope, setScope] = useState<SpanScope>('all');
  const [filters, setFilters] = useState<Record<string, string[]>>({});
  const [minMs, setMinMs] = useState('');
  const [maxMs, setMaxMs] = useState('');
  const [viz, setViz] = useState<Viz>('list');
  const [groupBy, setGroupBy] = useState<string>('');
  const [measure, setMeasure] = useState<SpanMeasure>('count');
  const [controls, setControls] = useState(true);
  const [rel, setRel] = useState(false);
  const [sort, setSort] = useState('date');
  const [offset, setOffset] = useState(0);
  const [facetSearch, setFacetSearch] = useState('');

  const q: SpanQuery = {
    range: t.range,
    scope: mode === 'traces' ? 'traces' : scope,
    query,
    filters,
    minDurationMs: Number(minMs) || null,
    maxDurationMs: Number(maxMs) || null,
  };
  const list = useExploreSpans(q, sort, offset, PAGE);
  const facets = useSpanFacets(q, FACETS);
  const summary = useSpanSummary(q, rel);
  useEffect(() => setOffset(0), [query, mode, scope, filters, minMs, maxMs, sort]);

  const openTrace = params.get('trace');
  const openSpan = params.get('span');
  const setParam = (k: string, v: string | null) => {
    const n = new URLSearchParams(params);
    if (v) n.set(k, v);
    else n.delete(k);
    setParams(n, { replace: true });
  };
  const open = (s: ExplorerSpan) => {
    const n = new URLSearchParams(params);
    n.set('trace', s.traceId);
    n.set('span', s.spanId);
    setParams(n, { replace: true });
  };
  const closeTrace = () => {
    const n = new URLSearchParams(params);
    n.delete('trace');
    n.delete('span');
    setParams(n, { replace: true });
  };
  const setFacet = (dim: string) => (v: string[]) => setFilters((f) => ({ ...f, [dim]: v }));
  const fs = facetSearch.trim().toLowerCase();

  return (
    <div className="sx">
      <div className="sx-top">
        <span className="sx-label">Search for</span>
        <div className="sx-seg">
          <button className={mode === 'spans' ? 'active' : ''} onClick={() => setMode('spans')}>
            Spans
          </button>
          <button className={mode === 'traces' ? 'active' : ''} onClick={() => setMode('traces')}>
            Traces
          </button>
        </div>
        <span className="sx-top-right">
          <span className={`sx-live${t.live ? ' on' : ''}`} title={t.live ? 'Results refresh as new spans arrive' : 'Paused'}>
            Live Search
          </span>
          <TimePicker />
        </span>
      </div>

      <div className="sx-query">
        <span className="sx-letter">a</span>
        {mode === 'traces' ? (
          <span className="sx-scope fixed">Root Spans</span>
        ) : (
          <label className="sx-scope">
            <select value={scope} onChange={(e) => setScope(e.target.value as SpanScope)} aria-label="Span scope">
              <option value="all">All Spans</option>
              <option value="service_entry">Service Entry Spans</option>
            </select>
            <ChevronDown size={14} />
          </label>
        )}
        <div className="sx-input">
          <QueryBar value={query} onChange={(v) => setParam('query', v || null)} placeholder="Search spans: service:billing env:prod -status:ok @http.route:/orders/*" />
        </div>
        {query && (
          <button className="icon-plain sx-clear" title="Clear" onClick={() => setParam('query', null)}>
            <X size={18} />
          </button>
        )}
      </div>
      <div className="sx-viz-row">
        <div className="sx-tree">
          <div className="sx-group">
            <span>Group by</span>
            <select value={groupBy} onChange={(e) => setGroupBy(e.target.value)} disabled={viz === 'list'} aria-label="Group by">
              <option value="">{viz === 'list' ? 'Select value' : 'Everything'}</option>
              {FACETS.map((d) => (
                <option key={d} value={d}>
                  {DIMENSION_LABEL[d]}
                </option>
              ))}
            </select>
            {viz !== 'list' && (
              <select value={measure} onChange={(e) => setMeasure(e.target.value as SpanMeasure)} aria-label="Measure">
                {MEASURES.map((m) => (
                  <option key={m.id} value={m.id}>
                    {m.label}
                  </option>
                ))}
              </select>
            )}
          </div>
          <div className="sx-viz">
            <span>Visualize as</span>
            {VIZ.map((v) => (
              <button
                key={v.id}
                className={viz === v.id ? 'active' : ''}
                onClick={() => {
                  setViz(v.id);
                  if (v.id !== 'list' && !groupBy && v.id !== 'timeseries' && v.id !== 'bars') setGroupBy('service');
                }}
              >
                <v.icon size={14} /> {v.label}
              </button>
            ))}
          </div>
        </div>
      </div>

      <div className="sx-body">
        {controls && (
          <aside className="sx-facets">
            <label className="mon-facet-search sx-facet-search">
              <Search size={14} />
              <input placeholder="Search facets" value={facetSearch} onChange={(e) => setFacetSearch(e.target.value)} />
            </label>
            <div className="sx-core">CORE</div>
            {(!fs || 'duration'.includes(fs)) && (
              <Facet title="Duration" defaultOpen={!!(minMs || maxMs)}>
                <div className="sx-duration">
                  <label>
                    min
                    <input type="number" min={0} placeholder={facets.data ? fmtLatency(facets.data.minDurationMs) : ''} value={minMs} onChange={(e) => setMinMs(e.target.value)} />
                    ms
                  </label>
                  <label>
                    max
                    <input type="number" min={0} placeholder={facets.data ? fmtLatency(facets.data.maxDurationMs) : ''} value={maxMs} onChange={(e) => setMaxMs(e.target.value)} />
                    ms
                  </label>
                </div>
              </Facet>
            )}
            {FACETS.filter((d) => !fs || DIMENSION_LABEL[d].toLowerCase().includes(fs)).map((d) => (
              <Facet key={d} title={DIMENSION_LABEL[d]} defaultOpen={['status', 'env', 'service'].includes(d)}>
                <ValuesFacet
                  values={facets.data?.facets[d] ?? []}
                  selected={filters[d] ?? []}
                  onChange={setFacet(d)}
                  searchable={['service', 'resource', 'host'].includes(d)}
                  label={d === 'status' ? (v) => (v === 'error' ? 'Error' : 'Ok') : undefined}
                  icon={d === 'status' ? (v) => <i className={`sx-dot ${v}`} /> : undefined}
                />
              </Facet>
            ))}
          </aside>
        )}
        <main className="sx-main">
          <div className="sx-toolbar">
            <button className="link-btn" onClick={() => setControls((c) => !c)}>
              {controls ? <PanelLeftClose size={16} /> : <PanelLeftOpen size={16} />} {controls ? 'Hide Controls' : 'Show Controls'}
            </button>
          </div>
          <section className="sx-rel">
            <header>
              <button onClick={() => setRel((r) => !r)}>{rel ? <ChevronDown size={16} /> : <ChevronRight size={16} />} Requests, Errors, and Latency</button>
              <span>
                <button className="link-btn" onClick={() => setFilters((f) => ({ ...f, status: ['error'] }))}>
                  Analyze Error Spans <ArrowRight size={14} />
                </button>
                <button
                  className="link-btn"
                  onClick={() => {
                    setRel(true);
                    if (summary.data) setMinMs(String(Math.round(summary.data.overallP95Ms)));
                  }}
                  title="Show spans slower than the p95 of the current results"
                >
                  Analyze High Latency Spans <ArrowRight size={14} />
                </button>
              </span>
            </header>
            {rel && (summary.data ? <RelCharts data={summary.data} /> : <Loading />)}
          </section>
          {viz === 'list' ? (
            <section className="sx-results">
              <div className="sx-results-head">
                {list.data ? (
                  mode === 'traces' ? (
                    <b>{list.data.total.toLocaleString()} traces found</b>
                  ) : (
                    <b>
                      {list.data.perSecond.toLocaleString()} spans/s <span className="muted">· {list.data.total.toLocaleString()} spans</span>
                    </b>
                  )
                ) : (
                  <b>…</b>
                )}
                <span className="sx-results-right">
                  <Link className="btn" to="/monitors/create/apm" title="Alert on these spans with an APM monitor">
                    <Gauge size={14} /> Create Monitor
                  </Link>
                  <select value={sort} onChange={(e) => setSort(e.target.value)} aria-label="Sort">
                    <option value="date">Newest first</option>
                    <option value="duration">Slowest first</option>
                  </select>
                </span>
              </div>
              {list.isPending ? (
                <Loading label="Searching spans…" />
              ) : list.isError ? (
                <ErrorState error={list.error} onRetry={() => list.refetch()} />
              ) : (
                <>
                  <SpanList rows={list.data.spans} traces={mode === 'traces' ? list.data.traces : null} selected={openSpan} onOpen={open} />
                  <div className="mon-pager">
                    <Pager offset={offset} limit={PAGE} total={Math.min(list.data.total, 10_000 + PAGE)} onChange={setOffset} />
                  </div>
                </>
              )}
            </section>
          ) : (
            <Visualization viz={viz} q={q} groupBy={groupBy || null} measure={measure} onPick={(dim, value) => setFilters((f) => ({ ...f, [dim]: [value] }))} />
          )}
        </main>
      </div>
      {openTrace && <TraceView traceId={openTrace} focusSpanId={openSpan} onClose={closeTrace} onSelectSpan={(id) => setParam('span', id)} />}
    </div>
  );
}

function SpanList({
  rows,
  traces,
  selected,
  onOpen,
}: {
  rows: ExplorerSpan[];
  traces: Record<string, { spans: number; services: number; errors: number }> | null;
  selected: string | null;
  onOpen: (s: ExplorerSpan) => void;
}) {
  if (rows.length === 0) return <p className="mon-empty">No spans match this search in the selected timeframe.</p>;
  return (
    <table className="sx-table">
      <thead>
        <tr>
          <th className="sx-bar-col" />
          <th>DATE</th>
          <th>SERVICE</th>
          <th>RESOURCE</th>
          <th className="num">DURATION</th>
          {traces && (
            <>
              <th className="num">SPANS</th>
              <th className="num">SERVICES</th>
              <th className="num">ERRORS</th>
            </>
          )}
          <th>METHOD</th>
          <th>STATUS CODE</th>
        </tr>
      </thead>
      <tbody>
        {rows.map((s) => {
          const tr = traces?.[s.traceId];
          return (
            <tr key={s.spanId} className={s.spanId === selected ? 'selected' : ''} onClick={() => onOpen(s)}>
              <td className="sx-bar-col">
                <i className={s.status} />
              </td>
              <td className="sx-date">{fmtStamp(s.timestamp)}</td>
              <td>
                <SpanService service={s.service} dbSystem={s.dbSystem} kind={s.kind} />
              </td>
              <td className="sx-resource" title={s.resource}>
                {s.resource}
              </td>
              <td className="num">{fmtLatency(s.durationMs)}</td>
              {traces && (
                <>
                  <td className="num">{tr?.spans ?? '—'}</td>
                  <td className="num">{tr?.services ?? '—'}</td>
                  <td className="num">{tr?.errors ? <span className="sx-err">{tr.errors}</span> : 0}</td>
                </>
              )}
              <td>{s.method}</td>
              <td>{s.httpStatus && <HttpStatus code={s.httpStatus} />}</td>
            </tr>
          );
        })}
      </tbody>
    </table>
  );
}

export function fmtStamp(ms: number): string {
  const d = new Date(ms);
  return `${d.toLocaleString(undefined, { month: 'short', day: 'numeric' })} ${d.toLocaleTimeString(undefined, { hour12: false })}.${String(d.getMilliseconds()).padStart(3, '0')}`;
}

function RelCharts({ data }: { data: NonNullable<ReturnType<typeof useSpanSummary>['data']> }) {
  const opts = useMemo(() => {
    const colors = palette();
    const base = (series: EChartsOption['series'], fmt?: (v: number) => string): EChartsOption => ({
      grid: { left: 48, right: 8, top: 20, bottom: 22 },
      legend: { top: 0, itemWidth: 10, itemHeight: 8, textStyle: { fontSize: 11 } },
      tooltip: { trigger: 'axis', valueFormatter: fmt ? (v) => fmt(v as number) : undefined },
      xAxis: { type: 'time', splitNumber: 4 },
      yAxis: { type: 'value', splitNumber: 3, axisLabel: fmt ? { formatter: (v: number) => fmt(v) } : undefined },
      series,
    });
    const at = (vals: (number | null | undefined)[]) => data.times.map((t, i) => [t + data.stepMs / 2, vals[i] ?? null]);
    return {
      requests: base([{ name: 'Requests', type: 'bar', barMaxWidth: 10, color: colors[0], data: at(data.hits) }]),
      errors: base([{ name: 'Errors', type: 'bar', barMaxWidth: 10, color: '#e5383b', data: at(data.errors) }]),
      latency: base(
        [
          { name: 'p50', type: 'line', showSymbol: false, color: colors[2], data: at(data.p50Ms) },
          { name: 'p95', type: 'line', showSymbol: false, color: colors[3], data: at(data.p95Ms) },
          { name: 'p99', type: 'line', showSymbol: false, color: colors[4], data: at(data.p99Ms) },
        ],
        fmtLatency,
      ),
    };
  }, [data]);
  return (
    <div className="sx-rel-charts">
      <div>
        <h4>Requests</h4>
        <Chart option={opts.requests} height={140} />
      </div>
      <div>
        <h4>Errors</h4>
        <Chart option={opts.errors} height={140} />
      </div>
      <div>
        <h4>Latency</h4>
        <Chart option={opts.latency} height={140} />
      </div>
    </div>
  );
}
