import { useMemo, useState } from 'react';
import { useSearchParams } from 'react-router-dom';
import type { EChartsOption } from 'echarts';
import { BarChart3, Download, Fingerprint, Hand, Lightbulb, List, PanelLeftClose, PanelLeftOpen, RotateCw, Search, Settings2, Table2, TrendingUp, X } from 'lucide-react';
import { searchSyntheticResultsAll, type SyntheticResultRow, type SyntheticResultSearchPage, type SyntheticTestType } from '../../api/synthetics/client';
import { RECOMMENDED_QUERIES, toCsv, type RecommendedQuery, type ResultsVisualization } from '../../api/synthetics/model';
import type { Facet } from '../../api/types';
import { Chart } from '../../components/charts/Chart';
import { FacetPanel } from '../../components/facets/FacetPanel';
import { QueryBar } from '../../components/query/QueryBar';
import { TimePicker } from '../../components/shell/PageHeader';
import { useTime } from '../../components/shell/TimeContext';
import { ErrorState, Loading } from '../../components/ui/AsyncState';
import { Menu } from '../../components/ui/Dropdown';
import { Pager } from '../../components/ui/Pager';
import { formatQuery, parseQuery } from '../../lib/searchQuery';
import { token } from '../../theme/chartTheme';
import { fmtDateTime } from '../monitors/common';
import { PassFail, TypeBadge, fmtMs } from './common';
import { useSyntheticResultSearch } from './queries';
import { ResultDrawer } from './SyntheticTestPage';

const VIZ: { id: ResultsVisualization; label: string; icon: typeof List }[] = [
  { id: 'list', label: 'List', icon: List },
  { id: 'timeseries', label: 'Timeseries', icon: TrendingUp },
  { id: 'toplist', label: 'Top List', icon: BarChart3 },
  { id: 'bars', label: 'Bar Chart', icon: BarChart3 },
  { id: 'table', label: 'Table', icon: Table2 },
];
const QUERY_ICON: Record<RecommendedQuery['icon'], typeof List> = { http: Search, retry: RotateCw, id: Fingerprint, manual: Hand };
const PAGE = 50;
const HIDE_KEY = 'oc.synthetics.explorer.hideRecommended';
const minute = 60_000;

type Column = 'location' | 'httpStatus' | 'retries' | 'trigger';
const COLUMN_LABEL: Record<Column, string> = { location: 'Location', httpStatus: 'HTTP status', retries: 'Retries', trigger: 'Trigger' };

function readHidden(): boolean {
  try {
    return globalThis.localStorage?.getItem(HIDE_KEY) === '1';
  } catch {
    return false;
  }
}

function writeHidden(v: boolean) {
  try {
    if (v) globalThis.localStorage?.setItem(HIDE_KEY, '1');
    else globalThis.localStorage?.removeItem(HIDE_KEY);
  } catch {
    // Storage unavailable: the card shows again next time.
  }
}

/** Synthetic Monitoring & Testing → Results Explorer: every stored run of every test, searched and charted. */
export function ResultsExplorerPage() {
  const [params, setParams] = useSearchParams();
  const query = params.get('q') ?? '';
  const viz = (params.get('viz') as ResultsVisualization | null) ?? 'list';
  const [scope, setScope] = useState<'runs' | 'batches'>('runs');
  const [offset, setOffset] = useState(0);
  const [controls, setControls] = useState(true);
  const [hideRecommended, setHideRecommended] = useState(readHidden);
  const [hidden, setHidden] = useState<Set<Column>>(new Set());
  const [open, setOpen] = useState<SyntheticResultRow | null>(null);
  const [downloading, setDownloading] = useState(false);
  const { range } = useTime();
  const r = useMemo(() => ({ from: Math.floor(range.from / minute) * minute, to: Math.ceil(range.to / minute) * minute }), [range.from, range.to]);
  const q = useSyntheticResultSearch({ query, range: r, offset, limit: PAGE });

  const set = (patch: { q?: string; viz?: ResultsVisualization }) => {
    const next = new URLSearchParams(params);
    if (patch.q !== undefined) {
      if (patch.q) next.set('q', patch.q);
      else next.delete('q');
      setOffset(0);
    }
    if (patch.viz) {
      if (patch.viz === 'list') next.delete('viz');
      else next.set('viz', patch.viz);
    }
    setParams(next, { replace: true });
  };
  const hideCard = (v: boolean) => {
    setHideRecommended(v);
    writeHidden(v);
  };
  const facets = useMemo<Facet[]>(() => (q.data?.facets ?? []).map((f) => ({ key: f.key, label: f.label, group: 'RESULTS', values: f.values })), [q.data]);
  const terms = useMemo(() => parseQuery(query), [query]);
  const download = async () => {
    setDownloading(true);
    try {
      const page = await searchSyntheticResultsAll({ query, range: r, offset: 0, limit: 500 });
      const csv = toCsv(
        ['DATE', 'STATUS', 'TEST', 'TEST ID', 'TYPE', 'LOCATION', 'HTTP STATUS', 'DURATION MS', 'RETRIES', 'TRIGGER', 'ERROR'],
        page.results.map((x) => [
          new Date(x.startedAt).toISOString(),
          x.passed ? 'passed' : 'failed',
          x.testName,
          x.testId,
          x.testType,
          x.location,
          x.statusCode ?? '',
          Math.round(x.durationMs),
          x.attempts - 1,
          x.trigger,
          x.error ?? '',
        ]),
      );
      const a = document.createElement('a');
      a.href = URL.createObjectURL(new Blob([csv], { type: 'text/csv' }));
      a.download = 'synthetic-test-runs.csv';
      a.click();
      URL.revokeObjectURL(a.href);
    } finally {
      setDownloading(false);
    }
  };
  const show = (c: Column) => !hidden.has(c);

  return (
    <div className="syn-page syn-explorer">
      {!hideRecommended && (
        <section className="syn-recommended">
          <div className="syn-recommended-intro">
            <h3>
              <Lightbulb size={16} /> Recommended Queries
            </h3>
            <p className="muted">Start from a common question about your test runs, then refine the query or switch the visualization.</p>
            <details className="syn-syntax">
              <summary>Learn more about the query syntax</summary>
              <ul>
                <li>
                  <code>status:failed</code>, <code>@test.type:(http OR ssl)</code>, <code>-@trigger:manual</code>
                </li>
                <li>
                  <code>@http.status_code:&gt;=500</code>, <code>@http.status_code:4*</code>, <code>@result.duration:&gt;1000</code> (ms), <code>@result.retries:&gt;0</code>
                </li>
                <li>
                  <code>@test.name:Checkout*</code>, <code>@test.id:…</code>, <code>location:observecraft-server</code>, <code>env:prod</code>, <code>team:checkout</code>, test tags such as{' '}
                  <code>service:billing</code>
                </li>
                <li>Free words match the test name and the error.</li>
              </ul>
            </details>
          </div>
          <div className="syn-recommended-cards">
            {RECOMMENDED_QUERIES.map((rq) => {
              const Icon = QUERY_ICON[rq.icon];
              return (
                <div key={rq.id} className="syn-query-card">
                  <Icon size={18} className="syn-query-icon" />
                  <b>{rq.title}</b>
                  <p className="muted">{rq.description}</p>
                  <button className="link-btn" onClick={() => set({ q: rq.query, viz: rq.visualization })}>
                    Run query
                  </button>
                </div>
              );
            })}
          </div>
          <button className="icon-plain syn-recommended-close" title="Hide recommended queries" onClick={() => hideCard(true)}>
            <X size={16} />
          </button>
        </section>
      )}
      <div className="syn-explorer-search">
        <div className="syn-segmented" role="tablist" aria-label="Search in">
          <span className="muted">In:</span>
          <button role="tab" aria-selected={scope === 'runs'} className={scope === 'runs' ? 'active' : ''} onClick={() => setScope('runs')}>
            Test Runs
          </button>
          <button role="tab" aria-selected={scope === 'batches'} className={scope === 'batches' ? 'active' : ''} onClick={() => setScope('batches')}>
            CI Batches
          </button>
        </div>
        <div className="syn-grow">
          <QueryBar value={query} onChange={(v) => set({ q: v })} placeholder="Search test runs, e.g. @type:result status:failed @http.status_code:>=500" />
        </div>
        <TimePicker compact />
      </div>
      {scope === 'batches' ? (
        <section className="syn-unavailable">
          <h2>CI batches aren't available yet</h2>
          <p className="muted">
            ObserveCraft runs tests on their schedule and when you start them by hand; running a batch of tests from a CI pipeline isn't available yet, so there are no CI batches to search. The Test
            Runs view holds every run.
          </p>
          <button className="btn" onClick={() => setScope('runs')}>
            Back to Test Runs
          </button>
        </section>
      ) : (
        <>
          <div className="syn-viz-bar">
            <span className="muted">Visualize as</span>
            <div className="syn-segmented">
              {VIZ.map((v) => (
                <button key={v.id} className={viz === v.id ? 'active' : ''} onClick={() => set({ viz: v.id })}>
                  <v.icon size={14} /> {v.label}
                </button>
              ))}
            </div>
            {hideRecommended && (
              <button className="link-btn syn-viz-right" onClick={() => hideCard(false)}>
                <Lightbulb size={14} /> Recommended queries
              </button>
            )}
          </div>
          <div className={`syn-explorer-body${controls ? '' : ' no-controls'}`}>
            {controls && (
              <FacetPanel
                facets={facets}
                terms={terms}
                onChange={(t) => set({ q: formatQuery(t) })}
                defaultOpen={['status', '@test.type', '@test.name']}
                verified={[]}
                noun="facets"
                valueLabel={(k, v) => (k === 'status' ? (v === 'passed' ? 'Passed' : 'Failed') : v)}
              />
            )}
            <div className="syn-explorer-main">
              <div className="syn-toolbar">
                <button className="link-btn" onClick={() => setControls((c) => !c)}>
                  {controls ? <PanelLeftClose size={15} /> : <PanelLeftOpen size={15} />} {controls ? 'Hide Controls' : 'Show Controls'}
                </button>
                <span className="syn-toolbar-sep" />
                <span>
                  <b>{(q.data?.total ?? 0).toLocaleString()}</b> test run{q.data?.total === 1 ? '' : 's'}
                </span>
                <span className="syn-grow" />
                <button className="btn btn-sm" disabled={downloading || !q.data?.total} onClick={download} title="The newest 500 runs of the search">
                  <Download size={13} /> {downloading ? 'Preparing…' : 'Download as CSV'}
                </button>
                <Menu
                  trigger={
                    <>
                      <Settings2 size={14} /> Options
                    </>
                  }
                  items={(Object.keys(COLUMN_LABEL) as Column[]).map((c) => ({
                    label: `${show(c) ? 'Hide' : 'Show'} ${COLUMN_LABEL[c]} column`,
                    onClick: () =>
                      setHidden((h) => {
                        const next = new Set(h);
                        if (next.has(c)) next.delete(c);
                        else next.add(c);
                        return next;
                      }),
                  }))}
                />
              </div>
              {q.isPending ? (
                <Loading label="Searching test runs…" />
              ) : q.isError ? (
                <ErrorState error={q.error} />
              ) : q.data.total === 0 ? (
                <div className="syn-empty syn-empty-search">
                  <Search size={56} strokeWidth={1.2} />
                  <h3>No Test Runs Found</h3>
                  <p className="muted">{query ? 'No run in this time range matches the query.' : 'No test ran in this time range.'}</p>
                </div>
              ) : viz === 'list' ? (
                <RunsList page={q.data} show={show} onOpen={setOpen} offset={offset} onPage={setOffset} />
              ) : viz === 'table' ? (
                <GroupsTable page={q.data} onTest={(id) => set({ q: `@test.id:${id}`, viz: 'list' })} />
              ) : (
                <ResultsChart page={q.data} viz={viz} range={r} />
              )}
            </div>
          </div>
        </>
      )}
      {open && <ResultDrawer test={{ id: open.testId, name: open.testName, type: open.testType as SyntheticTestType }} resultId={open.id} onClose={() => setOpen(null)} />}
    </div>
  );
}

function RunsList({
  page,
  show,
  onOpen,
  offset,
  onPage,
}: {
  page: SyntheticResultSearchPage;
  show: (c: Column) => boolean;
  onOpen: (r: SyntheticResultRow) => void;
  offset: number;
  onPage: (o: number) => void;
}) {
  return (
    <>
      <div className="mon-table-wrap">
        <table className="mon-table syn-table">
          <thead>
            <tr>
              <th className="syn-col-icon" />
              <th className="syn-col-date">DATE</th>
              <th className="syn-col-type">TYPE</th>
              <th>TEST</th>
              {show('location') && <th className="syn-col-wide">LOCATION</th>}
              {show('httpStatus') && <th className="syn-col-num syn-col-short">HTTP STATUS</th>}
              <th className="syn-col-num syn-col-short">DURATION</th>
              {show('retries') && <th className="syn-col-num syn-col-short">RETRIES</th>}
              {show('trigger') && <th className="syn-col-short">TRIGGER</th>}
            </tr>
          </thead>
          <tbody>
            {page.results.map((x) => (
              <tr key={`${x.testId}-${x.id}`} onClick={() => onOpen(x)}>
                <td className="syn-col-icon">
                  <PassFail passed={x.passed} />
                </td>
                <td className="syn-col-date">{fmtDateTime(x.startedAt)}</td>
                <td className="syn-col-type">
                  <TypeBadge type={x.testType as SyntheticTestType} />
                </td>
                <td className="mon-name">
                  <span className="syn-name">{x.testName}</span>
                  {!x.passed && x.error && <span className="muted syn-sub">{x.error}</span>}
                  {!x.passed && !x.error && x.failedAssertions > 0 && (
                    <span className="muted syn-sub">
                      {x.failedAssertions} assertion{x.failedAssertions === 1 ? '' : 's'} failed
                    </span>
                  )}
                </td>
                {show('location') && <td className="muted syn-col-wide">{x.location}</td>}
                {show('httpStatus') && <td className={`syn-col-num syn-col-short${x.statusCode != null && x.statusCode >= 400 ? ' syn-bad' : ''}`}>{x.statusCode ?? '—'}</td>}
                <td className="syn-col-num syn-col-short">{fmtMs(x.durationMs)}</td>
                {show('retries') && <td className="syn-col-num syn-col-short">{x.attempts - 1}</td>}
                {show('trigger') && <td className="muted syn-col-short">{x.trigger === 'manual' ? 'run by hand' : 'scheduled'}</td>}
              </tr>
            ))}
          </tbody>
        </table>
      </div>
      <Pager offset={offset} limit={PAGE} total={page.total} onChange={onPage} />
    </>
  );
}

function GroupsTable({ page, onTest }: { page: SyntheticResultSearchPage; onTest: (id: string) => void }) {
  return (
    <div className="mon-table-wrap">
      <table className="mon-table syn-table">
        <thead>
          <tr>
            <th className="syn-col-type">TYPE</th>
            <th>TEST</th>
            <th className="syn-col-wide">TEST ID</th>
            <th className="syn-col-num syn-col-short">RUNS</th>
            <th className="syn-col-num syn-col-short">FAILED</th>
            <th className="syn-col-num syn-col-short">AVG DURATION</th>
            <th className="syn-col-num syn-col-short">P95 DURATION</th>
            <th className="syn-col-date">LAST RUN</th>
          </tr>
        </thead>
        <tbody>
          {page.groups.map((g) => (
            <tr key={g.testId} onClick={() => onTest(g.testId)} title="Show this test's runs">
              <td className="syn-col-type">
                <TypeBadge type={g.testType as SyntheticTestType} />
              </td>
              <td className="mon-name">{g.testName}</td>
              <td className="mono muted syn-col-wide" title={g.testId}>
                {g.testId}
              </td>
              <td className="syn-col-num syn-col-short">{g.runs.toLocaleString()}</td>
              <td className={`syn-col-num syn-col-short${g.failed > 0 ? ' syn-bad' : ''}`}>{g.failed.toLocaleString()}</td>
              <td className="syn-col-num syn-col-short">{fmtMs(g.avgDurationMs)}</td>
              <td className="syn-col-num syn-col-short">{fmtMs(g.p95DurationMs)}</td>
              <td className="syn-col-date">{fmtDateTime(g.lastRunAt)}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

function ResultsChart({ page, viz, range }: { page: SyntheticResultSearchPage; viz: ResultsVisualization; range: { from: number; to: number } }) {
  const option = useMemo<EChartsOption>(() => {
    const ok = token('--mon-ok');
    const bad = token('--mon-alert');
    if (viz === 'toplist') {
      const groups = [...page.groups]
        .sort((a, b) => b.failed - a.failed || b.runs - a.runs)
        .slice(0, 20)
        .reverse();
      return {
        grid: { left: 200, right: 40, top: 24, bottom: 28 },
        tooltip: { trigger: 'axis', axisPointer: { type: 'shadow' } },
        legend: { top: 0, right: 0 },
        xAxis: { type: 'value' },
        yAxis: { type: 'category', data: groups.map((g) => g.testName), axisLabel: { width: 180, overflow: 'truncate' } },
        series: [
          { name: 'Failed', type: 'bar', stack: 'runs', itemStyle: { color: bad }, data: groups.map((g) => g.failed) },
          { name: 'Passed', type: 'bar', stack: 'runs', itemStyle: { color: ok }, data: groups.map((g) => g.runs - g.failed) },
        ],
      };
    }
    const asBars = viz === 'bars';
    const series = (name: string, color: string, pick: (p: { passed: number; failed: number }) => number) => ({
      name,
      type: asBars ? ('bar' as const) : ('line' as const),
      stack: 'runs',
      showSymbol: false,
      areaStyle: asBars ? undefined : { opacity: 0.25 },
      itemStyle: { color },
      lineStyle: { color, width: 1.5 },
      data: page.series.map((p) => [p.t, pick(p)]),
    });
    return {
      grid: { left: 48, right: 16, top: 28, bottom: 28 },
      tooltip: { trigger: 'axis' },
      legend: { top: 0, right: 0 },
      xAxis: { type: 'time', min: range.from, max: range.to },
      yAxis: { type: 'value', name: 'runs', minInterval: 1 },
      series: [series('Failed', bad, (p) => p.failed), series('Passed', ok, (p) => p.passed)],
    };
  }, [page, viz, range.from, range.to]);
  return (
    <section className="syn-card syn-chart-card">
      <Chart option={option} height={viz === 'toplist' ? Math.max(240, Math.min(20, page.groups.length) * 26 + 60) : 320} />
      <p className="muted syn-chart-note">{viz === 'toplist' ? 'Tests with the most failed runs.' : `Runs per ${Math.round(page.bucketMs / 60_000)} minutes, failed and passed.`}</p>
    </section>
  );
}
