import { useCallback, useEffect, useMemo, useState } from 'react';
import { createPortal } from 'react-dom';
import { Link, useNavigate, useParams, useSearchParams } from 'react-router-dom';
import type { EChartsOption } from 'echarts';
import { BellRing, CalendarDays, ChevronDown, ChevronUp, ChevronsDown, ChevronsUp, Copy, ExternalLink, List, MapPin, Pause, Play, Radar, Search, Timer, VolumeX, X, Zap } from 'lucide-react';
import type { SyntheticResultSummary, SyntheticTest, SyntheticUptimeSegment } from '../../api/synthetics/client';
import { TYPE_LABEL, assertionText, draftOf, fmtFrequency, fmtUptime } from '../../api/synthetics/logic';
import { Chart } from '../../components/charts/Chart';
import { TimePicker } from '../../components/shell/PageHeader';
import { TimeProvider, useTime } from '../../components/shell/TimeContext';
import { ErrorState, Loading } from '../../components/ui/AsyncState';
import { Menu } from '../../components/ui/Dropdown';
import { timeAgo } from '../../lib/format';
import { token } from '../../theme/chartTheme';
import { TagChips, fmtDateTime, fmtDuration } from '../monitors/common';
import { messageRecipients } from '../monitors/message';
import { RunView, SEGMENT_COLOR, TestStatus, UptimeBar, fmtMs, fmtShortMs } from './common';
import { useDeleteSyntheticTest, useRunSyntheticTest, useSaveSyntheticTest, useSyntheticResult, useSyntheticResults, useSyntheticTest } from './queries';
import '../monitors/monitors.css';
import './synthetics.css';

const LOCATION_NAME = 'ObserveCraft server';
const locationName = (id: string) => (id === 'observecraft-server' ? LOCATION_NAME : id);

type Tab = 'activity' | 'runs' | 'performance' | 'properties';
const TABS: [Tab, string][] = [
  ['activity', 'Activity'],
  ['runs', 'Test Runs'],
  ['performance', 'Performance'],
  ['properties', 'Properties'],
];

/** A test: its activity (uptime, timeline, latest runs), all runs, performance and properties. */
export function SyntheticTestPage() {
  return (
    <TimeProvider initial="1h">
      <TestInner />
    </TimeProvider>
  );
}

function TestInner() {
  const id = useParams().id!;
  const q = useSyntheticTest(id);
  if (q.isPending) return <Loading label="Loading test…" />;
  if (q.isError) return <ErrorState error={q.error} onRetry={() => q.refetch()} />;
  return <TestView test={q.data} />;
}

/** The request a test sends, as one line. */
function target(t: SyntheticTest): string {
  if (t.type === 'http') return t.request.url;
  if (t.type === 'dns') return `${t.request.host}${t.request.dnsServer ? ` @ ${t.request.dnsServer}` : ''}`;
  return `${t.request.host}:${t.request.port ?? (t.type === 'ssl' ? 443 : '')}`;
}

function TestView({ test: t }: { test: SyntheticTest }) {
  const navigate = useNavigate();
  const [params, setParams] = useSearchParams();
  const tab = (TABS.find(([k]) => k === params.get('tab'))?.[0] ?? 'activity') as Tab;
  const setTab = (k: Tab) => {
    const next = new URLSearchParams(params);
    if (k === 'activity') next.delete('tab');
    else next.set('tab', k);
    setParams(next, { replace: true });
  };
  const run = useRunSyntheticTest();
  const save = useSaveSyntheticTest(t.id);
  const remove = useDeleteSyntheticTest();
  const [open, setOpen] = useState<number | null>(null);
  const [copied, setCopied] = useState(false);
  const setPaused = (paused: boolean) => save.mutate({ ...draftOf(t), paused });
  const runNow = () => run.mutate(t.id, { onSuccess: (r) => setOpen(r.id) });
  const recipients = messageRecipients(t.message);
  const copyId = () => navigator.clipboard?.writeText(t.id).then(() => (setCopied(true), setTimeout(() => setCopied(false), 1200)));

  const { range } = useTime();
  const minute = 60_000;
  const r = useMemo(() => ({ from: Math.floor(range.from / minute) * minute, to: Math.ceil(range.to / minute) * minute }), [range.from, range.to]);
  const results = useSyntheticResults(t.id, r);
  const rows = results.data?.results ?? [];

  return (
    <div className="mon-status-page">
      <header className="mon-crumbs syn-test-crumbs">
        <Link to="/synthetics" className="mon-crumb-root" title="Synthetic Monitoring & Testing">
          <Radar size={20} />
        </Link>
        <span className="muted">/</span>
        <Link to="/synthetics/tests" className="mon-crumb-root" title="Tests">
          <List size={20} />
        </Link>
        <span className="muted">/</span>
        <TestStatus status={t.status} paused={t.paused} small />
        <span className="syn-crumb-name">{t.name}</span>
        <span className="mon-crumbs-right">
          <button className="link-btn syn-head-action" disabled={run.isPending} onClick={runNow}>
            <Zap size={15} /> {run.isPending ? 'Running…' : 'Run Test Now'}
          </button>
          <button className="link-btn syn-head-action" disabled={save.isPending} onClick={() => setPaused(!t.paused)}>
            {t.paused ? <Play size={15} /> : <Pause size={15} />} {t.paused ? 'Resume Scheduling' : 'Pause Scheduling'}
          </button>
          <Menu
            triggerClassName="link-btn syn-head-action muted"
            trigger={
              <>
                More <ChevronDown size={14} />
              </>
            }
            items={[
              { label: 'Edit test', onClick: () => navigate(`/synthetics/tests/${t.id}/edit`) },
              { label: 'Clone', onClick: () => navigate(`/synthetics/create?clone=${t.id}`) },
              ...(t.monitorId ? [{ label: 'View monitor', onClick: () => navigate(`/monitors/${t.monitorId}`) }] : []),
              { label: 'View runs in Results Explorer', onClick: () => navigate(`/synthetics/explorer?q=${encodeURIComponent(`@test.id:${t.id}`)}`) },
              {
                label: 'Delete',
                onClick: () => window.confirm(`Delete “${t.name}”, its results and its monitor? This can't be undone.`) && remove.mutate(t.id, { onSuccess: () => navigate('/synthetics/tests') }),
              },
            ]}
          />
        </span>
      </header>
      <div className="mon-status-scroll">
        <section className="syn-test-head">
          <div className="syn-test-meta">
            <span className="syn-meta-type">
              {TYPE_LABEL[t.type]} Test
              <button className="icon-plain" title={copied ? 'Copied' : `Copy the test ID (${t.id})`} onClick={copyId}>
                <Copy size={13} />
              </button>
            </span>
            <span className="syn-meta-sep" />
            {t.monitorId ? (
              <Link className="syn-meta-chip" to={`/monitors/${t.monitorId}`}>
                <BellRing size={13} /> Synthetic Monitor
              </Link>
            ) : (
              <span className="syn-meta-chip muted">No monitor</span>
            )}
            <span className="syn-meta-sep" />
            <span className="muted">
              Created {new Date(t.createdAt).toLocaleDateString(undefined, { month: 'short', day: 'numeric', year: 'numeric' })}
              {t.createdBy ? ` by ${t.createdBy}` : ''}
            </span>
            <span className="syn-meta-sep" />
            {t.env && <span className="syn-chip">{t.env}</span>}
            {t.tags.length > 0 && <TagChips tags={t.tags} max={6} />}
            {t.muted && (
              <span className="syn-muted-tag">
                <VolumeX size={12} /> muted by a downtime
              </span>
            )}
          </div>
          <p className="syn-test-sentence">
            {t.type === 'http' ? (
              <>
                <b>{t.request.method}</b>{' '}
                {/\{\{/.test(t.request.url) ? (
                  <b className="mono">{t.request.url}</b>
                ) : (
                  <a href={t.request.url} target="_blank" rel="noreferrer noopener" className="syn-test-url">
                    {t.request.url} <ExternalLink size={14} />
                  </a>
                )}
              </>
            ) : (
              <>
                <b>{TYPE_LABEL[t.type]}</b> <b className="mono">{target(t)}</b>
              </>
            )}{' '}
            from {LOCATION_NAME} <b>{fmtFrequency(t.frequencySeconds)}</b>
            {t.paused ? ' (paused)' : ''}. On failure,{' '}
            {recipients.length ? (
              <>
                <BellRing size={14} className="muted" /> notify <b>{recipients.map((h) => `@${h}`).join(', ')}</b>
              </>
            ) : (
              <>alert {t.minFailureDurationMinutes ? `after ${t.minFailureDurationMinutes} min` : 'right away'} without notifying anyone</>
            )}
            .
          </p>
          <nav className="syn-test-tabs" role="tablist">
            {TABS.map(([k, label]) => (
              <button key={k} role="tab" aria-selected={tab === k} className={`syn-test-tab${tab === k ? ' active' : ''}`} onClick={() => setTab(k)}>
                {label}
              </button>
            ))}
          </nav>
        </section>
        {run.isError && <p className="form-error syn-pad">{(run.error as Error).message}</p>}
        {save.isError && <p className="form-error syn-pad">{(save.error as Error).message}</p>}
        {tab === 'properties' ? (
          <Properties test={t} />
        ) : results.isPending ? (
          <Loading label="Loading results…" />
        ) : results.isError ? (
          <ErrorState error={results.error} onRetry={() => results.refetch()} />
        ) : tab === 'activity' ? (
          <Activity test={t} range={r} segments={results.data.segments} uptime={results.data.uptime} rows={rows} total={results.data.total} onOpen={setOpen} onRun={runNow} running={run.isPending} />
        ) : tab === 'runs' ? (
          <RunsTab rows={rows} total={results.data.total} onOpen={setOpen} />
        ) : (
          <Performance test={t} range={r} rows={rows} />
        )}
      </div>
      {open !== null && <ResultDrawer test={t} resultId={open} onClose={() => setOpen(null)} siblings={rows.map((x) => x.id)} onMove={setOpen} />}
    </div>
  );
}

// ================================================================ activity

/** Evenly spaced time labels across a range. */
function axisTicks(from: number, to: number): number[] {
  const steps = [60_000, 300_000, 600_000, 900_000, 1_800_000, 3_600_000, 3 * 3_600_000, 6 * 3_600_000, 12 * 3_600_000, 86_400_000, 7 * 86_400_000];
  const step = steps.find((s) => (to - from) / s <= 12) ?? steps[steps.length - 1];
  const out: number[] = [];
  for (let at = Math.ceil(from / step) * step; at <= to; at += step) out.push(at);
  return out;
}

function tickLabel(at: number, span: number): string {
  const d = new Date(at);
  return span > 2 * 86_400_000 ? d.toLocaleDateString(undefined, { month: 'short', day: 'numeric' }) : d.toLocaleTimeString(undefined, { hour: '2-digit', minute: '2-digit', hour12: false });
}

/** The state changes in the range, newest first, for the timeline. */
function stateEvents(segments: SyntheticUptimeSegment[]): { at: number; status: SyntheticUptimeSegment['status']; until: number }[] {
  return segments
    .filter((s, i) => i > 0 && s.status !== segments[i - 1].status)
    .map((s) => ({ at: s.from, status: s.status, until: s.to }))
    .reverse();
}

function Activity({
  test: t,
  range,
  segments,
  uptime,
  rows,
  total,
  onOpen,
  onRun,
  running,
}: {
  test: SyntheticTest;
  range: { from: number; to: number };
  segments: SyntheticUptimeSegment[];
  uptime: number | null | undefined;
  rows: SyntheticResultSummary[];
  total: number;
  onOpen: (id: number) => void;
  onRun: () => void;
  running: boolean;
}) {
  const { relative } = useTime();
  const [location, setLocation] = useState('all');
  const [perLocation, setPerLocation] = useState(false);
  const end = Math.min(range.to, Date.now());
  const ticks = axisTicks(range.from, end);
  const span = end - range.from;
  const events = stateEvents(segments);
  const shown = location === 'all' ? rows : rows.filter((x) => x.location === location);
  const alerts = segments.filter((s) => s.status === 'alert').length;
  const verdict = t.paused ? 'paused' : t.status === 'alert' ? 'failing' : t.status === 'nodata' ? 'nodata' : 'passing';
  const period = relative.label.toLowerCase();

  return (
    <>
      <section className="syn-activity">
        <div className="syn-activity-bar">
          <label className="syn-select-field">
            <span>Locations</span>
            <select value={location} onChange={(e) => setLocation(e.target.value)}>
              <option value="all">All</option>
              {t.locations.map((l) => (
                <option key={l} value={l}>
                  {locationName(l)}
                </option>
              ))}
            </select>
          </label>
          <span className="syn-grow" />
          <TimePicker />
        </div>
        <div className="syn-uptime-grid">
          <span />
          <div className="syn-uptime-ticks">
            <i className="syn-tick-edge">{fmtDateTime(range.from)}</i>
            {ticks
              .filter((at) => at - range.from > span * 0.09 && end - at > span * 0.09)
              .map((at) => (
                <i key={at} style={{ left: `${((at - range.from) / Math.max(1, span)) * 100}%` }}>
                  {tickLabel(at, span)}
                </i>
              ))}
            <i className="syn-tick-edge right">{fmtDateTime(end)}</i>
          </div>
          <span className="syn-uptime-label">
            Global Uptime <b>{fmtUptime(uptime)}</b>
          </span>
          <UptimeBar segments={segments} from={range.from} to={end} />
          {perLocation &&
            t.locations.map((l) => (
              <div key={l} className="syn-uptime-row">
                <span className="syn-uptime-label muted">
                  <MapPin size={13} /> {locationName(l)}
                </span>
                <UptimeBar segments={segments} from={range.from} to={end} />
              </div>
            ))}
        </div>
        <div className="syn-legend syn-activity-legend">
          {(['ok', 'alert', 'nodata'] as const)
            .filter((s) => segments.some((x) => x.status === s))
            .map((s) => (
              <span key={s}>
                <i style={{ background: SEGMENT_COLOR[s] }} /> {s === 'ok' ? 'Ok' : s === 'alert' ? 'Alert' : 'No Data'}
              </span>
            ))}
        </div>
        <button className="syn-divider-btn" onClick={() => setPerLocation((p) => !p)}>
          {perLocation ? <ChevronsUp size={15} /> : <ChevronsDown size={15} />} {perLocation ? 'Hide locations' : 'Show All locations'}
        </button>
      </section>
      <div className="syn-activity-body">
        <aside className="syn-timeline">
          <h4>Timeline</h4>
          <div className="syn-timeline-item live">
            <Zap size={15} />
            <b>Summary</b>
            <span className="syn-live">LIVE</span>
            <span className="muted">
              {total} run{total === 1 ? '' : 's'}
            </span>
          </div>
          {events.length === 0 ? (
            <p className="muted syn-timeline-empty">No events found for the selected timeframe</p>
          ) : (
            events.map((e) => (
              <div key={e.at} className={`syn-timeline-item ${e.status}`}>
                <i style={{ background: SEGMENT_COLOR[e.status] }} />
                <span>
                  <b>{e.status === 'alert' ? 'Test alerted' : e.status === 'ok' ? 'Test recovered' : 'No data'}</b>
                  <span className="muted">
                    {fmtDateTime(e.at)} · for {fmtDuration(e.until - e.at)}
                  </span>
                </span>
              </div>
            ))
          )}
        </aside>
        <div className={`syn-activity-main ${verdict}`}>
          <div className={`syn-verdict ${verdict}`}>
            <span className="syn-verdict-pill">{verdict === 'nodata' ? 'NO DATA' : verdict.toUpperCase()}</span>
            <span>
              {verdict === 'paused' ? (
                <>Scheduling is paused: the test only runs with Run Test Now.</>
              ) : verdict === 'nodata' ? (
                <>The test has no scheduled run in the {period} yet.</>
              ) : (
                <>
                  Your test is <b className={verdict}>{verdict}</b> with {fmtUptime(uptime)} uptime and {alerts ? `${alerts} alert${alerts === 1 ? '' : 's'}` : 'no alerts'} in the {period}
                  {verdict === 'failing' && t.failingSince ? ` (failing for ${fmtDuration(Date.now() - t.failingSince)})` : ''}
                </>
              )}
            </span>
          </div>
          <h4 className="syn-latest-title">
            Latest Test Runs <span className="syn-count">{shown.length}</span>
          </h4>
          <RunsTable rows={shown.slice(0, 50)} onOpen={onOpen} />
          {shown.length > 50 && <p className="muted syn-more-runs">The newest 50 runs. Every run is in Test Runs.</p>}
        </div>
        <aside className="syn-actions-card">
          <h4>Actions</h4>
          <button className="btn syn-wide" disabled={running} onClick={onRun}>
            <Zap size={14} /> {running ? 'Running…' : 'Run Test Now'}
          </button>
          <h5>Investigate</h5>
          <Link className="syn-action-link" to={`/synthetics/explorer?q=${encodeURIComponent(`@test.id:${t.id}`)}`}>
            <Search size={15} /> Results Explorer <ExternalLink size={14} className="muted" />
          </Link>
          {t.monitorId && (
            <Link className="syn-action-link" to={`/monitors/${t.monitorId}`}>
              <BellRing size={15} /> Monitor status <ExternalLink size={14} className="muted" />
            </Link>
          )}
        </aside>
      </div>
    </>
  );
}

const shortDate = (at: number) => new Date(at).toLocaleString(undefined, { month: 'short', day: 'numeric', hour: 'numeric', minute: '2-digit' });

/** Runs with their result, time, trigger, duration and location. */
function RunsTable({ rows, onOpen, failures }: { rows: SyntheticResultSummary[]; onOpen: (id: number) => void; failures?: boolean }) {
  if (!rows.length) return <p className="mon-empty">No runs in this time range.</p>;
  return (
    <div className="mon-table-wrap">
      <table className="mon-table syn-table syn-runs-table">
        <thead>
          <tr>
            <th className="syn-col-result">STATUS</th>
            <th>DATE</th>
            <th>RUN TYPE</th>
            <th className="syn-col-num">DURATION</th>
            <th>LOCATION</th>
            {failures && <th>FAILURE</th>}
            <th className="syn-col-num">ATTEMPTS</th>
          </tr>
        </thead>
        <tbody>
          {rows.map((x) => (
            <tr key={x.id} onClick={() => onOpen(x.id)}>
              <td className="syn-col-result">
                <span className={`syn-run-pill ${x.passed ? 'pass' : 'fail'}`}>{x.passed ? 'PASSED' : 'FAILED'}</span>
              </td>
              <td>
                <span className="syn-ago">{timeAgo(x.startedAt)}</span> <span className="muted">{shortDate(x.startedAt)}</span>
              </td>
              <td>
                {x.trigger === 'manual' ? (
                  <span className="syn-icon-text">
                    <Zap size={14} /> Manually triggered
                  </span>
                ) : (
                  <span className="syn-icon-text">
                    <CalendarDays size={14} /> Scheduled
                  </span>
                )}
              </td>
              <td className="syn-col-num">{fmtMs(x.durationMs)}</td>
              <td>
                <span className="syn-icon-text">
                  <MapPin size={14} /> {locationName(x.location)}
                </span>
              </td>
              {failures && (
                <td className={x.passed ? 'muted' : 'c-alert'}>
                  {x.passed
                    ? ''
                    : (x.error ??
                      (x.failedAssertions ? `${x.statusCode != null ? `HTTP ${x.statusCode} · ` : ''}${x.failedAssertions} assertion${x.failedAssertions > 1 ? 's' : ''} failed` : 'Failed'))}
                </td>
              )}
              <td className="syn-col-num">{x.attempts}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

function RunsTab({ rows, total, onOpen }: { rows: SyntheticResultSummary[]; total: number; onOpen: (id: number) => void }) {
  const [failedOnly, setFailedOnly] = useState(false);
  const shown = rows.filter((x) => !failedOnly || !x.passed);
  return (
    <section className="syn-results">
      <div className="syn-results-head">
        <div className="syn-activity-bar syn-grow">
          <label className="mon-check">
            <input type="checkbox" className="checkbox" checked={failedOnly} onChange={() => setFailedOnly((f) => !f)} />
            <span className="mon-check-label">Failed runs only</span>
          </label>
          <span className="muted">{total > rows.length ? `Newest ${rows.length} of ${total} runs` : `${shown.length} run${shown.length === 1 ? '' : 's'}`}</span>
          <span className="syn-grow" />
          <TimePicker />
        </div>
      </div>
      <RunsTable rows={shown.slice(0, 500)} onOpen={onOpen} failures />
    </section>
  );
}

// ================================================================ performance

const PHASE_SERIES: [keyof NonNullable<SyntheticResultSummary['timings']>, string, string][] = [
  ['dns', 'DNS', '--chart-6'],
  ['tcp', 'Connection', '--chart-1'],
  ['ssl', 'SSL', '--chart-3'],
  ['firstByte', 'Time to first byte', '--chart-4'],
  ['download', 'Download', '--chart-2'],
];

function Performance({ test: t, range, rows }: { test: SyntheticTest; range: { from: number; to: number }; rows: SyntheticResultSummary[] }) {
  const scheduled = useMemo(() => [...rows].reverse().filter((x) => x.trigger === 'scheduled'), [rows]);
  const timed = scheduled.filter((x) => x.timings);
  const durations = scheduled.map((x) => x.durationMs).sort((a, b) => a - b);
  const pct = (p: number) => (durations.length ? durations[Math.min(durations.length - 1, Math.floor((p / 100) * durations.length))] : null);
  const avg = durations.length ? durations.reduce((a, b) => a + b, 0) / durations.length : null;

  const response = useMemo<EChartsOption>(
    () => ({
      grid: { left: 60, right: 16, top: 16, bottom: 28 },
      tooltip: { trigger: 'axis', valueFormatter: (v) => fmtMs(Number(v)) },
      xAxis: { type: 'time', min: range.from, max: range.to },
      yAxis: { type: 'value', axisLabel: { formatter: (v: number) => fmtShortMs(v) } },
      series: [
        {
          name: 'Response time',
          type: 'line',
          showSymbol: false,
          lineStyle: { width: 1.5, color: token('--chart-1') },
          itemStyle: { color: token('--chart-1') },
          data: scheduled.map((x) => [x.startedAt, Math.round(x.durationMs)]),
        },
        {
          name: 'Failed runs',
          type: 'scatter',
          symbolSize: 7,
          itemStyle: { color: token('--status-error') },
          data: scheduled.filter((x) => !x.passed).map((x) => [x.startedAt, Math.round(x.durationMs)]),
        },
      ],
    }),
    [scheduled, range.from, range.to],
  );
  const phases = useMemo<EChartsOption>(
    () => ({
      grid: { left: 60, right: 16, top: 36, bottom: 28 },
      legend: { top: 0, left: 0, icon: 'rect', itemWidth: 10, itemHeight: 10 },
      tooltip: { trigger: 'axis', valueFormatter: (v) => fmtShortMs(Number(v)) },
      xAxis: { type: 'time', min: range.from, max: range.to },
      yAxis: { type: 'value', axisLabel: { formatter: (v: number) => fmtShortMs(v) } },
      series: PHASE_SERIES.filter(([k]) => timed.some((x) => (x.timings?.[k] ?? 0) > 0)).map(([k, name, color]) => ({
        name,
        type: 'line',
        stack: 'phases',
        areaStyle: { opacity: 0.75 },
        showSymbol: false,
        lineStyle: { width: 0 },
        itemStyle: { color: token(color) },
        data: timed.map((x) => [x.startedAt, Math.round((x.timings?.[k] ?? 0) * 10) / 10]),
      })),
    }),
    [timed, range.from, range.to],
  );

  return (
    <section className="syn-perf">
      <div className="syn-activity-bar">
        <h3 className="syn-h3">Response time from {t.locations.length === 1 ? LOCATION_NAME : `${t.locations.length} locations`}</h3>
        <span className="syn-grow" />
        <TimePicker />
      </div>
      <div className="syn-perf-stats">
        {(
          [
            ['Average', avg],
            ['p50', pct(50)],
            ['p90', pct(90)],
            ['p99', pct(99)],
            ['Max', durations.length ? durations[durations.length - 1] : null],
          ] as [string, number | null][]
        ).map(([k, v]) => (
          <div key={k}>
            <span className="muted">{k}</span>
            <b>{fmtMs(v)}</b>
          </div>
        ))}
      </div>
      {scheduled.length ? <Chart option={response} height={220} /> : <p className="mon-empty">No scheduled runs in this time range{t.paused ? ': the test is paused' : ''}.</p>}
      {t.type === 'http' && (
        <>
          <h3 className="syn-h3">
            <Timer size={16} /> Transaction timings
          </h3>
          {timed.length ? <Chart option={phases} height={240} /> : <p className="mon-empty">No timed HTTP runs in this time range.</p>}
        </>
      )}
    </section>
  );
}

// ================================================================ properties

function Properties({ test: t }: { test: SyntheticTest }) {
  const recipients = messageRecipients(t.message);
  const rows: [string, React.ReactNode][] = [
    ['Request', <span className="mono syn-wrap">{t.type === 'http' ? `${t.request.method} ${t.request.url}` : target(t)}</span>],
    ...(t.type === 'http' && t.request.headers.length
      ? ([['Headers', t.request.headers.map((h, i) => <div key={i} className="mono">{`${h.name}: ${h.value}`}</div>)]] as [string, React.ReactNode][])
      : []),
    ...(t.type === 'http' && t.request.body ? ([['Body', <pre className="syn-body">{t.request.body}</pre>]] as [string, React.ReactNode][]) : []),
    ['Assertions', t.assertions.length ? t.assertions.map((a, i) => <div key={i}>{assertionText(a)}</div>) : 'None (the request must succeed)'],
    ['Locations', t.locations.map(locationName).join(', ')],
    ['Frequency', fmtFrequency(t.frequencySeconds)],
    ['Retries', t.retry.count ? `${t.retry.count} after ${t.retry.intervalMs ?? 0} ms` : 'None'],
    ['Alert condition', `Failing for ${t.minFailureDurationMinutes} min from ${t.minLocationFailed} of ${t.locations.length} location${t.locations.length === 1 ? '' : 's'}`],
    ['Timeout', `${t.request.timeoutSeconds} s`],
    ['Notifications', recipients.length ? recipients.map((h) => `@${h}`).join(', ') : 'None'],
    ['Priority', t.priority ? `P${t.priority}` : '—'],
    ['Environment', t.env || '—'],
    ['Team', t.team || '—'],
    ['Tags', t.tags.length ? <TagChips tags={t.tags} max={12} /> : '—'],
    ['Test ID', <span className="mono">{t.id}</span>],
    ['Created', `${fmtDateTime(t.createdAt)}${t.createdBy ? ` by ${t.createdBy}` : ''}`],
    ['Last modified', `${fmtDateTime(t.updatedAt)}${t.updatedBy ? ` by ${t.updatedBy}` : ''}`],
  ];
  return (
    <section className="syn-props">
      <div className="mon-details syn-details">
        {rows.map(([k, v]) => (
          <div key={k}>
            <span className="muted">{k}</span>
            <span>{v}</span>
          </div>
        ))}
      </div>
      {t.message && (
        <>
          <h3 className="syn-h3">Monitor message</h3>
          <pre className="syn-body syn-props-message">{t.message}</pre>
        </>
      )}
    </section>
  );
}

// ================================================================ one run

/** One run in a side panel: result, when, where, how it was started, its duration, then what it sent and got back. */
export function ResultDrawer({
  test,
  resultId,
  onClose,
  siblings,
  onMove,
}: {
  test: Pick<SyntheticTest, 'id' | 'name' | 'type'>;
  resultId: number;
  onClose: () => void;
  /** Ids of the list the run was opened from, to step through it. */
  siblings?: number[];
  onMove?: (id: number) => void;
}) {
  const q = useSyntheticResult(test.id, resultId);
  const at = siblings?.indexOf(resultId) ?? -1;
  const move = useCallback(
    (by: number) => {
      if (!siblings || !onMove || at < 0) return;
      const next = siblings[at + by];
      if (next !== undefined) onMove(next);
    },
    [siblings, onMove, at],
  );
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onClose();
      if (e.key === 'ArrowUp') move(-1);
      if (e.key === 'ArrowDown') move(1);
    };
    globalThis.addEventListener('keydown', onKey);
    return () => globalThis.removeEventListener('keydown', onKey);
  }, [onClose, move]);
  const r = q.data;
  return createPortal(
    <div className="syn-drawer-backdrop" onMouseDown={(e) => e.target === e.currentTarget && onClose()}>
      <aside className="syn-drawer" role="dialog" aria-label="Test result">
        <div className="syn-drawer-head syn-run-head">
          {r ? (
            <>
              <span className={`syn-run-pill ${r.detail.passed ? 'pass' : 'fail'}`}>{r.detail.passed ? 'PASSED' : 'FAILED'}</span>
              <span>
                <b>{timeAgo(r.startedAt)}</b> <span className="muted">({fmtDateTime(r.startedAt)})</span>
              </span>
              <span className="syn-run-head-sep" />
              <span className="syn-icon-text" title={test.name}>
                <span className="syn-run-head-name">{test.name}</span>
              </span>
              <span className="syn-run-head-sep" />
              <span className="syn-icon-text">
                <MapPin size={14} /> {locationName(r.location)}
              </span>
              <span className="syn-run-head-sep" />
              <span className="syn-icon-text">
                {r.trigger === 'manual' ? <Zap size={14} /> : <CalendarDays size={14} />} {r.trigger === 'manual' ? 'Manually triggered' : 'Scheduled'}
              </span>
              <span className="syn-run-head-sep" />
              <span className="syn-icon-text">
                <Timer size={14} /> {fmtMs(r.detail.durationMs)}
              </span>
              {r.attempts > 1 && <span className="muted">· {r.attempts} attempts</span>}
            </>
          ) : (
            <h2>Test result</h2>
          )}
          <div className="syn-drawer-actions">
            <Link className="icon-plain" title="Open the test" to={`/synthetics/tests/${test.id}`} onClick={onClose}>
              <ExternalLink size={16} />
            </Link>
            {siblings && onMove && (
              <>
                <button className="icon-plain" title="Previous run (↑)" disabled={at <= 0} onClick={() => move(-1)}>
                  <ChevronUp size={18} />
                </button>
                <button className="icon-plain" title="Next run (↓)" disabled={at < 0 || at >= siblings.length - 1} onClick={() => move(1)}>
                  <ChevronDown size={18} />
                </button>
              </>
            )}
            <button className="icon-plain" title="Close" onClick={onClose}>
              <X size={18} />
            </button>
          </div>
        </div>
        <div className="syn-drawer-body">
          {q.isPending ? <Loading label="Loading result…" /> : q.isError ? <ErrorState error={q.error} onRetry={() => q.refetch()} /> : <RunView run={q.data.detail} type={test.type} />}
        </div>
      </aside>
    </div>,
    document.body,
  );
}
