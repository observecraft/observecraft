import { useDeferredValue, useEffect, useMemo, useState } from 'react';
import { useSearchParams } from 'react-router-dom';
import type { EChartsOption } from 'echarts';
import { BarChart3, Code2, Download, LineChart, List, PanelLeftClose, PanelLeftOpen, Search } from 'lucide-react';
import type { CiCharts, CiExecution, CiFilters, CiLevel, CiQuery } from '../../api/ci/client';
import { Chart } from '../../components/charts/Chart';
import { TimePicker } from '../../components/shell/PageHeader';
import { useTime } from '../../components/shell/TimeContext';
import { ErrorState, Loading } from '../../components/ui/AsyncState';
import { Pager } from '../../components/ui/Pager';
import { palette } from '../../theme/chartTheme';
import { Facet, ValuesFacet } from '../monitors/common';
import { Branch, Ext, NO_FILTERS, PROVIDER_LABEL, STATUS_COLOR, STATUS_LABEL, StatusPill, fmtAgo, fmtDate, fmtDur } from './common';
import { PipelineDrawer } from './PipelineDrawer';
import { useAggregations, useCharts, useExecutions, useFacets } from './queries';

const PAGE = 50;
const REDS = ['#f28b82', '#e5383b', '#b3122e', '#ff6f61', '#c62828', '#ef9a9a', '#8e0000', '#d32f2f', '#ffab91'];
type View = 'list' | 'timeseries' | 'toplist';

export function ExplorerPage() {
  const t = useTime();
  const [params, setParams] = useSearchParams();
  const [tab, setTab] = useState<'executions' | 'aggregations'>('executions');
  const [level, setLevel] = useState<CiLevel>('pipeline');
  const [text, setText] = useState(params.get('q') ?? '');
  const [filters, setFilters] = useState<CiFilters>(NO_FILTERS);
  const [view, setView] = useState<View>('list');
  const [graphs, setGraphs] = useState(true);
  const [controls, setControls] = useState(true);
  const [sort, setSort] = useState('date');
  const [offset, setOffset] = useState(0);
  const query = useDeferredValue(text);
  const q: CiQuery = { range: t.range, level, query, filters };
  const execs = useExecutions(q, sort, offset, PAGE);
  const facets = useFacets(q);
  const charts = useCharts(q, graphs || view === 'timeseries');
  const aggs = useAggregations(q, tab === 'aggregations' || view === 'toplist');
  const open = params.get('execution');
  useEffect(() => setOffset(0), [query, level, filters, sort]);
  const setOpen = (id: string | null) => {
    const n = new URLSearchParams(params);
    if (id) n.set('execution', id);
    else n.delete('execution');
    setParams(n, { replace: true });
  };
  const set = (k: keyof CiFilters) => (v: string[]) => setFilters((f) => ({ ...f, [k]: v }));
  const f = facets.data;
  const error = execs.error ?? facets.error;

  return (
    <div className="ci-explorer">
      <nav className="ci-subtabs">
        <button className={tab === 'executions' ? 'active' : ''} onClick={() => setTab('executions')}>
          Executions
        </button>
        <button className={tab === 'aggregations' ? 'active' : ''} onClick={() => setTab('aggregations')}>
          Aggregations
        </button>
        <span className="ci-subtabs-right">
          <TimePicker />
        </span>
      </nav>
      <div className="ci-searchbar">
        <label className="ci-level">
          <span>In</span>
          <i />
          <select value={level} onChange={(e) => setLevel(e.target.value as CiLevel)} aria-label="Level">
            <option value="pipeline">CI · Pipeline</option>
            <option value="job">CI · Job</option>
          </select>
        </label>
        <div className="ci-search">
          <Search size={16} />
          <input placeholder={`Filter your ${level} executions, e.g. repo:example-org/billing branch:main status:error`} value={text} onChange={(e) => setText(e.target.value)} />
          <span
            className="ci-search-code"
            title="Words match names and repositories; key:value filters on status, pipeline, repo, branch, provider, team, sha, job, stage or author (prefix with - to exclude)"
          >
            <Code2 size={16} />
          </span>
        </div>
      </div>
      {tab === 'executions' && (
        <div className="ci-viz-row">
          <div className="ci-viz">
            <span>Visualize as</span>
            <button className={view === 'list' ? 'active' : ''} onClick={() => setView('list')}>
              <List size={14} /> List
            </button>
            <button className={view === 'timeseries' ? 'active' : ''} onClick={() => setView('timeseries')}>
              <LineChart size={14} /> Timeseries
            </button>
            <button className={view === 'toplist' ? 'active' : ''} onClick={() => setView('toplist')}>
              <BarChart3 size={14} /> Top List
            </button>
          </div>
          {view === 'list' && (
            <button className="link-btn" onClick={() => setGraphs((g) => !g)}>
              {graphs ? 'Hide Graphs' : 'Show Graphs'}
            </button>
          )}
        </div>
      )}
      {error ? (
        <ErrorState error={error} onRetry={() => execs.refetch()} />
      ) : (
        <>
          {tab === 'executions' && view === 'list' && graphs && <Graphs data={charts.data} level={level} />}
          {tab === 'executions' && view === 'timeseries' && <BigTimeseries data={charts.data} />}
          {tab === 'executions' && view === 'toplist' && <TopList rows={aggs.data ?? []} onPick={(name) => setText(`${level === 'job' ? 'job' : 'pipeline'}:${name.replace(/\s+/g, '*')}`)} />}
          <div className="ci-body">
            {controls && (
              <aside className="ci-facets">
                <Facet title="CI Status">
                  <ValuesFacet
                    values={f?.status ?? []}
                    selected={filters.statuses ?? []}
                    onChange={set('statuses')}
                    label={(v) => STATUS_LABEL[v] ?? v}
                    icon={(v) => <i className="ci-dot" style={{ background: STATUS_COLOR[v] }} />}
                  />
                </Facet>
                <Facet title="Pipeline Name">
                  <ValuesFacet values={f?.pipeline ?? []} selected={filters.pipelines ?? []} onChange={set('pipelines')} searchable />
                </Facet>
                <Facet title="Repository">
                  <ValuesFacet values={f?.repository ?? []} selected={filters.repositories ?? []} onChange={set('repositories')} searchable />
                </Facet>
                <Facet title="Branch">
                  <ValuesFacet values={f?.branch ?? []} selected={filters.branches ?? []} onChange={set('branches')} searchable />
                </Facet>
                <Facet title="Provider" defaultOpen={false}>
                  <ValuesFacet values={f?.provider ?? []} selected={filters.providers ?? []} onChange={set('providers')} label={(v) => PROVIDER_LABEL[v] ?? v} />
                </Facet>
                <Facet title="Team" defaultOpen={false}>
                  <ValuesFacet values={f?.team ?? []} selected={filters.teams ?? []} onChange={set('teams')} />
                </Facet>
              </aside>
            )}
            <main className="ci-main">
              <div className="ci-toolbar">
                <button className="link-btn" onClick={() => setControls((c) => !c)}>
                  {controls ? <PanelLeftClose size={16} /> : <PanelLeftOpen size={16} />} {controls ? 'Hide Controls' : 'Show Controls'}
                </button>
                <span className="mon-sep" />
                {tab === 'executions' ? (
                  <b>{execs.data ? `${execs.data.total.toLocaleString()} ${level} executions found` : '…'}</b>
                ) : (
                  <b>{aggs.data ? `${aggs.data.length} ${level === 'job' ? 'jobs' : 'pipelines'}` : '…'}</b>
                )}
                <span className="ci-toolbar-right">
                  {tab === 'executions' && (
                    <>
                      <select value={sort} onChange={(e) => setSort(e.target.value)} aria-label="Sort">
                        <option value="date">Newest first</option>
                        <option value="duration">Slowest first</option>
                      </select>
                      <button className="btn" disabled={!execs.data} onClick={() => execs.data && exportCsv(execs.data.executions)}>
                        <Download size={14} /> Export
                      </button>
                    </>
                  )}
                </span>
              </div>
              {tab === 'executions' ? (
                execs.isPending ? (
                  <Loading label="Loading executions…" />
                ) : (
                  <ExecutionTable rows={execs.data!.executions} level={level} onOpen={setOpen} total={execs.data!.total} offset={offset} onPage={setOffset} />
                )
              ) : aggs.isPending ? (
                <Loading label="Aggregating…" />
              ) : (
                <AggregationsTable rows={aggs.data ?? []} level={level} onOpen={setOpen} />
              )}
            </main>
          </div>
        </>
      )}
      {open && <PipelineDrawer traceId={open} onClose={() => setOpen(null)} />}
    </div>
  );
}

function ExecutionTable({
  rows,
  level,
  onOpen,
  total,
  offset,
  onPage,
}: {
  rows: CiExecution[];
  level: CiLevel;
  onOpen: (id: string) => void;
  total: number;
  offset: number;
  onPage: (o: number) => void;
}) {
  const max = Math.max(1, ...rows.map((r) => r.durationMs));
  return (
    <div className="ci-table-wrap">
      <table className="ci-table ci-exec-table">
        <thead>
          <tr>
            <th className="ci-col-bar" />
            <th>DATE</th>
            <th>{level === 'job' ? 'JOB NAME' : 'PIPELINE NAME'}</th>
            {level === 'job' && <th>STAGE</th>}
            <th>DURATION</th>
            <th>PIPELINE ID</th>
            <th>REPOSITORY NAME</th>
            <th>BRANCH</th>
            <th>AUTHOR</th>
          </tr>
        </thead>
        <tbody>
          {rows.map((r) => (
            <tr key={r.spanId} onClick={() => onOpen(r.traceId)}>
              <td className="ci-col-bar">
                <i style={{ background: STATUS_COLOR[r.status] }} title={STATUS_LABEL[r.status]} />
              </td>
              <td className="ci-nowrap">{fmtDate(r.start)}</td>
              <td className="ci-strong">
                {r.name}
                {level === 'job' && <span className="muted"> · {r.pipelineName}</span>}
              </td>
              {level === 'job' && <td>{r.stage}</td>}
              <td className="ci-dur">
                <span>{fmtDur(r.durationMs)}</span>
                <i style={{ width: `${(r.durationMs / max) * 60}px` }} />
              </td>
              <td>
                <Ext href={r.runUrl}>#{r.runId}</Ext>
              </td>
              <td>
                <Ext href={r.repositoryUrl}>{r.repository}</Ext>
              </td>
              <td>
                <Branch name={r.branch} />
              </td>
              <td className="ci-nowrap">{r.author}</td>
            </tr>
          ))}
        </tbody>
      </table>
      {rows.length === 0 && <p className="mon-empty">No executions match these filters in the selected timeframe.</p>}
      <div className="mon-pager">
        <Pager offset={offset} limit={PAGE} total={total} onChange={onPage} />
      </div>
    </div>
  );
}

function AggregationsTable({ rows, level, onOpen }: { rows: NonNullable<ReturnType<typeof useAggregations>['data']>; level: CiLevel; onOpen: (id: string) => void }) {
  return (
    <div className="ci-table-wrap">
      <table className="ci-table">
        <thead>
          <tr>
            <th>{level === 'job' ? 'JOB' : 'PIPELINE'}</th>
            <th>REPOSITORY</th>
            <th className="num">EXECUTIONS</th>
            <th className="num">FAILURE RATE</th>
            <th className="num">AVG</th>
            <th className="num">P50</th>
            <th className="num">P95</th>
            <th>LAST EXECUTION</th>
          </tr>
        </thead>
        <tbody>
          {rows.map((a) => {
            const rate = a.executions ? (a.failures * 100) / a.executions : 0;
            return (
              <tr key={`${a.name}|${a.repository}`} onClick={() => onOpen(a.lastTraceId)}>
                <td className="ci-strong">{a.name}</td>
                <td>{a.repository}</td>
                <td className="num">{a.executions.toLocaleString()}</td>
                <td className="num">
                  <span className={rate >= 20 ? 'ci-bad' : rate >= 5 ? 'ci-warn' : ''}>{rate.toFixed(1)}%</span>
                </td>
                <td className="num">{fmtDur(a.avgMs)}</td>
                <td className="num">{fmtDur(a.p50Ms)}</td>
                <td className="num">{fmtDur(a.p95Ms)}</td>
                <td>
                  <StatusPill status={a.lastStatus} /> <span className="muted">{fmtAgo(a.lastStart)}</span>
                </td>
              </tr>
            );
          })}
        </tbody>
      </table>
      {rows.length === 0 && <p className="mon-empty">Nothing ran in the selected timeframe.</p>}
    </div>
  );
}

function baseOption(data: CiCharts): EChartsOption {
  return {
    grid: { left: 36, right: 8, top: 10, bottom: 22 },
    tooltip: { trigger: 'axis' },
    xAxis: {
      type: 'time',
      min: data.times[0],
      max: data.times[data.times.length - 1] + data.stepMs,
    },
    yAxis: { type: 'value', minInterval: 1, splitNumber: 3 },
  };
}

function Graphs({ data, level }: { data?: CiCharts; level: CiLevel }) {
  const options = useMemo(() => {
    if (!data) return null;
    const colors = palette();
    const duration: EChartsOption = {
      ...baseOption(data),
      tooltip: { trigger: 'axis', valueFormatter: (v) => fmtDur(v as number) },
      yAxis: {
        type: 'value',
        splitNumber: 3,
        axisLabel: { formatter: (v: number) => fmtDur(v) },
      },
      grid: { left: 56, right: 8, top: 10, bottom: 22 },
      series: data.series.map((s, i) => ({
        name: s.name,
        type: 'line',
        showSymbol: false,
        connectNulls: true,
        color: colors[i % colors.length],
        data: data.times.map((t, j) => [t, s.duration[j]]),
      })),
    };
    const bars = (pick: 'errored' | 'executions', cols: string[]): EChartsOption => ({
      ...baseOption(data),
      series: data.series.map((s, i) => ({
        name: s.name,
        type: 'bar',
        stack: 'x',
        barMaxWidth: 12,
        color: cols[i % cols.length],
        data: data.times.map((t, j) => [t + data.stepMs / 2, s[pick][j]]),
      })),
    });
    return {
      duration,
      errored: bars('errored', REDS),
      executions: bars('executions', colors),
    };
  }, [data]);
  return (
    <div className="ci-graphs">
      {(['duration', 'errored', 'executions'] as const).map((k) => (
        <section key={k} className="ci-graph">
          <h4>{k === 'duration' ? 'Duration (Avg)' : k === 'errored' ? 'Errored' : 'Executions'}</h4>
          {options ? <Chart option={options[k]} height={150} /> : <div className="ci-graph-empty" />}
        </section>
      ))}
      <span className="ci-graphs-note muted">Grouped by {level === 'job' ? 'job' : 'pipeline'} name</span>
    </div>
  );
}

function BigTimeseries({ data }: { data?: CiCharts }) {
  const option = useMemo<EChartsOption | null>(() => {
    if (!data) return null;
    const colors = palette();
    return {
      ...baseOption(data),
      grid: { left: 40, right: 16, top: 30, bottom: 26 },
      legend: { top: 0, type: 'scroll' },
      series: data.series.map((s, i) => ({
        name: s.name,
        type: 'bar',
        stack: 'x',
        barMaxWidth: 18,
        color: colors[i % colors.length],
        data: data.times.map((t, j) => [t + data.stepMs / 2, s.executions[j]]),
      })),
    };
  }, [data]);
  return <section className="ci-big-chart">{option ? <Chart option={option} height={260} /> : <Loading />}</section>;
}

function TopList({ rows, onPick }: { rows: NonNullable<ReturnType<typeof useAggregations>['data']>; onPick: (name: string) => void }) {
  const top = rows.slice(0, 10);
  const max = Math.max(1, ...top.map((r) => r.executions));
  return (
    <section className="ci-toplist">
      {top.map((r) => (
        <button key={`${r.name}|${r.repository}`} onClick={() => onPick(r.name)} title="Filter on this pipeline">
          <span className="ci-toplist-label">
            {r.name} <span className="muted">{r.repository}</span>
          </span>
          <span className="ci-toplist-bar">
            <i style={{ width: `${(r.executions / max) * 100}%` }} />
          </span>
          <b>{r.executions}</b>
        </button>
      ))}
    </section>
  );
}

function exportCsv(rows: CiExecution[]) {
  const esc = (s: string) => `"${s.replace(/"/g, '""')}"`;
  const head = ['date', 'name', 'pipeline', 'status', 'duration_ms', 'run_id', 'repository', 'branch', 'sha', 'author'];
  const lines = [head.join(',')].concat(
    rows.map((r) => [new Date(r.start).toISOString(), r.name, r.pipelineName, r.status, String(r.durationMs), r.runId, r.repository, r.branch, r.sha, r.author].map(esc).join(',')),
  );
  const a = document.createElement('a');
  a.href = URL.createObjectURL(new Blob([lines.join('\n')], { type: 'text/csv' }));
  a.download = 'pipeline-executions.csv';
  a.click();
  URL.revokeObjectURL(a.href);
}
