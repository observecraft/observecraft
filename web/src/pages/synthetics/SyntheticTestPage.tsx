import { useEffect, useMemo, useState } from 'react';
import { createPortal } from 'react-dom';
import { Link, useNavigate, useParams } from 'react-router-dom';
import type { EChartsOption } from 'echarts';
import { ChevronRight, Gauge, MoreVertical, Pause, Pencil, Play, Radar, RotateCw, VolumeX, X } from 'lucide-react';
import type { SyntheticResultSummary, SyntheticTest } from '../../api/synthetics/client';
import { assertionText, draftOf, fmtFrequency, fmtUptime } from '../../api/synthetics/logic';
import { Chart } from '../../components/charts/Chart';
import { TimePicker } from '../../components/shell/PageHeader';
import { TimeProvider, useTime } from '../../components/shell/TimeContext';
import { ErrorState, Loading } from '../../components/ui/AsyncState';
import { Menu } from '../../components/ui/Dropdown';
import { token } from '../../theme/chartTheme';
import { TagChips, fmtDateTime, fmtDuration } from '../monitors/common';
import { PassFail, RunView, SEGMENT_COLOR, TestStatus, TypeBadge, UptimeBar, fmtMs } from './common';
import { useDeleteSyntheticTest, useRunSyntheticTest, useSaveSyntheticTest, useSyntheticResult, useSyntheticResults, useSyntheticTest } from './queries';
import '../monitors/monitors.css';
import './synthetics.css';

/** A test: its alert state and uptime, response times, properties and recent results. */
export function SyntheticTestPage() {
  return (
    <TimeProvider initial="1d">
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

function TestView({ test: t }: { test: SyntheticTest }) {
  const navigate = useNavigate();
  const run = useRunSyntheticTest();
  const save = useSaveSyntheticTest(t.id);
  const remove = useDeleteSyntheticTest();
  const [open, setOpen] = useState<number | null>(null);
  const setPaused = (paused: boolean) => save.mutate({ ...draftOf(t), paused });

  return (
    <div className="mon-status-page">
      <header className="mon-crumbs">
        <Link to="/synthetics/tests" className="mon-crumb-root">
          <Radar size={22} /> Synthetic Tests
        </Link>
        <ChevronRight size={18} className="muted" />
        <span className="mon-crumb-link">{t.name}</span>
        <span className="mon-crumbs-right">
          <button className="btn" disabled={run.isPending} onClick={() => run.mutate(t.id, { onSuccess: (r) => setOpen(r.id) })}>
            <RotateCw size={14} /> {run.isPending ? 'Running…' : 'Run Test Now'}
          </button>
          <button className="btn" disabled={save.isPending} onClick={() => setPaused(!t.paused)}>
            {t.paused ? <Play size={14} /> : <Pause size={14} />} {t.paused ? 'Resume' : 'Pause'}
          </button>
          <button className="btn" onClick={() => navigate(`/synthetics/tests/${t.id}/edit`)}>
            <Pencil size={14} /> Edit
          </button>
          <Menu
            triggerClassName="icon-plain"
            trigger={<MoreVertical size={18} />}
            items={[
              ...(t.monitorId ? [{ label: 'View monitor', onClick: () => navigate(`/monitors/${t.monitorId}`) }] : []),
              {
                label: 'Delete',
                onClick: () => window.confirm(`Delete “${t.name}”, its results and its monitor? This can't be undone.`) && remove.mutate(t.id, { onSuccess: () => navigate('/synthetics/tests') }),
              },
            ]}
          />
        </span>
      </header>
      <div className="mon-status-scroll">
        <section className="mon-status-head">
          <div className="mon-status-title">
            <span className="syn-title-row">
              <TestStatus status={t.status} paused={t.paused} />
              <TypeBadge type={t.type} />
              {t.muted && (
                <span className="syn-muted-tag">
                  <VolumeX size={12} /> muted by a downtime
                </span>
              )}
            </span>
            <h1>{t.name}</h1>
            <p className="muted mono syn-wrap">
              {t.type === 'http'
                ? `${t.request.method} ${t.request.url}`
                : t.type === 'dns'
                  ? `${t.request.host}${t.request.dnsServer ? ` @ ${t.request.dnsServer}` : ''}`
                  : `${t.request.host}:${t.request.port ?? ''}`}
            </p>
          </div>
          <div className="syn-head-facts">
            <span>
              <span className="muted">Runs</span> {fmtFrequency(t.frequencySeconds)}
            </span>
            <span>
              <span className="muted">Last run</span> {t.lastRunAt ? fmtDateTime(t.lastRunAt) : 'not yet'}
            </span>
            {t.failingSince && !t.paused && (
              <span className="c-alert">
                <span className="muted">Failing for</span> {fmtDuration(Date.now() - t.failingSince)}
              </span>
            )}
            {t.monitorId && <Link to={`/monitors/${t.monitorId}`}>Monitor and notifications</Link>}
          </div>
        </section>
        {run.isError && <p className="form-error syn-pad">{(run.error as Error).message}</p>}
        {save.isError && <p className="form-error syn-pad">{(save.error as Error).message}</p>}
        <History test={t} onOpen={setOpen} />
        <Properties test={t} />
      </div>
      {open !== null && <ResultDrawer test={t} resultId={open} onClose={() => setOpen(null)} />}
    </div>
  );
}

function History({ test: t, onOpen }: { test: SyntheticTest; onOpen: (id: number) => void }) {
  const { range } = useTime();
  const minute = 60_000;
  const r = useMemo(() => ({ from: Math.floor(range.from / minute) * minute, to: Math.ceil(range.to / minute) * minute }), [range.from, range.to]);
  const q = useSyntheticResults(t.id, r);
  const [failedOnly, setFailedOnly] = useState(false);
  const data = q.data;
  const results = (data?.results ?? []).filter((x) => !failedOnly || !x.passed);
  const option = useMemo<EChartsOption>(() => {
    const rows = [...(data?.results ?? [])].reverse();
    return {
      grid: { left: 54, right: 16, top: 16, bottom: 28 },
      tooltip: { trigger: 'item', valueFormatter: (v) => fmtMs(Number(v)) },
      xAxis: { type: 'time', min: r.from, max: r.to },
      yAxis: { type: 'value', name: 'ms', axisLabel: { formatter: (v: number) => fmtMs(v) } },
      series: [
        {
          name: 'Response time',
          type: 'line',
          showSymbol: false,
          lineStyle: { width: 1.5, color: token('--chart-1') },
          itemStyle: { color: token('--chart-1') },
          data: rows.filter((x) => x.trigger === 'scheduled').map((x) => [x.startedAt, Math.round(x.durationMs)]),
        },
        {
          name: 'Failed runs',
          type: 'scatter',
          symbolSize: 7,
          itemStyle: { color: token('--status-error') },
          data: rows.filter((x) => !x.passed).map((x) => [x.startedAt, Math.round(x.durationMs)]),
        },
      ],
    };
  }, [data, r.from, r.to]);
  const passed = data?.results.filter((x) => x.passed).length ?? 0;

  return (
    <>
      <section className="mon-history">
        <div className="mon-history-bar">
          <TimePicker />
        </div>
        {q.isPending ? (
          <Loading label="Loading results…" />
        ) : q.isError ? (
          <ErrorState error={q.error} onRetry={() => q.refetch()} />
        ) : (
          <>
            <div className="syn-uptime">
              <div className="syn-uptime-value">
                <span className="muted">Uptime</span>
                <b>{fmtUptime(data!.uptime)}</b>
              </div>
              <div className="syn-grow">
                <UptimeBar segments={data!.segments} from={r.from} to={Math.min(r.to, Date.now())} />
                <div className="syn-uptime-axis muted">
                  <span>{fmtDateTime(r.from)}</span>
                  <span className="syn-legend">
                    {(['ok', 'alert', 'nodata'] as const).map((s) => (
                      <span key={s}>
                        <i style={{ background: SEGMENT_COLOR[s] }} /> {s === 'ok' ? 'OK' : s === 'alert' ? 'Alert' : 'No Data'}
                      </span>
                    ))}
                  </span>
                  <span>{fmtDateTime(Math.min(r.to, Date.now()))}</span>
                </div>
              </div>
              <div className="syn-uptime-value">
                <span className="muted">Runs</span>
                <b>{data!.total}</b>
                <span className="muted">
                  {passed} passed · {(data?.results.length ?? 0) - passed} failed
                </span>
              </div>
            </div>
            <h3 className="syn-h3">Response time from {t.locations.length === 1 ? 'the ObserveCraft server' : `${t.locations.length} locations`}</h3>
            {data!.results.length ? <Chart option={option} height={200} /> : <p className="mon-empty">No runs in this time range{t.paused ? ': the test is paused' : ''}.</p>}
          </>
        )}
      </section>
      <section className="syn-results">
        <div className="syn-results-head">
          <h3 className="syn-h3">Test runs</h3>
          <label className="mon-check">
            <input type="checkbox" className="checkbox" checked={failedOnly} onChange={() => setFailedOnly((f) => !f)} />
            <span className="mon-check-label">Failed runs only</span>
          </label>
          {data && data.total > data.results.length && (
            <span className="muted">
              Newest {data.results.length} of {data.total}
            </span>
          )}
        </div>
        {data && (
          <table className="mon-table syn-table">
            <thead>
              <tr>
                <th className="syn-col-icon">RESULT</th>
                <th>DATE</th>
                <th>LOCATION</th>
                <th className="syn-col-num">DURATION</th>
                <th>RESPONSE</th>
                <th className="syn-col-num">ATTEMPTS</th>
                <th>RUN</th>
              </tr>
            </thead>
            <tbody>
              {results.slice(0, 500).map((x) => (
                <ResultRow key={x.id} x={x} onOpen={() => onOpen(x.id)} />
              ))}
            </tbody>
          </table>
        )}
        {data && results.length === 0 && <p className="mon-empty">{failedOnly ? 'No failed runs in this time range.' : 'No runs in this time range.'}</p>}
      </section>
    </>
  );
}

function ResultRow({ x, onOpen }: { x: SyntheticResultSummary; onOpen: () => void }) {
  const response = x.error ?? (x.statusCode != null ? `HTTP ${x.statusCode}` : x.passed ? 'OK' : '');
  return (
    <tr onClick={onOpen}>
      <td className="syn-col-icon">
        <PassFail passed={x.passed} />
      </td>
      <td>{fmtDateTime(x.startedAt)}</td>
      <td>{x.location === 'observecraft-server' ? 'ObserveCraft server' : x.location}</td>
      <td className="syn-col-num">{fmtMs(x.durationMs)}</td>
      <td className={x.passed ? '' : 'c-alert'}>
        {response}
        {x.failedAssertions > 0 && (
          <span className="muted">
            {' '}
            · {x.failedAssertions} assertion{x.failedAssertions > 1 ? 's' : ''} failed
          </span>
        )}
      </td>
      <td className="syn-col-num">{x.attempts}</td>
      <td className="muted">{x.trigger === 'manual' ? 'by hand' : 'scheduled'}</td>
    </tr>
  );
}

function Properties({ test: t }: { test: SyntheticTest }) {
  const rows: [string, React.ReactNode][] = [
    ['Assertions', t.assertions.length ? t.assertions.map((a, i) => <div key={i}>{assertionText(a)}</div>) : 'None (the request must succeed)'],
    ['Location', 'ObserveCraft server'],
    ['Retries', t.retry.count ? `${t.retry.count} after ${t.retry.intervalMs ?? 0} ms` : 'None'],
    ['Alert condition', `Failing for ${t.minFailureDurationMinutes} min from ${t.minLocationFailed} of ${t.locations.length} location`],
    ['Timeout', `${t.request.timeoutSeconds} s`],
    ['Environment', t.env || '—'],
    ['Team', t.team || '—'],
    ['Tags', t.tags.length ? <TagChips tags={t.tags} max={8} /> : '—'],
    ['Created', `${fmtDateTime(t.createdAt)}${t.createdBy ? ` by ${t.createdBy}` : ''}`],
    ['Last modified', `${fmtDateTime(t.updatedAt)}${t.updatedBy ? ` by ${t.updatedBy}` : ''}`],
  ];
  return (
    <section className="syn-props">
      <h3 className="syn-h3">
        <Gauge size={16} /> Properties
      </h3>
      <div className="mon-details syn-details">
        {rows.map(([k, v]) => (
          <div key={k}>
            <span className="muted">{k}</span>
            <span>{v}</span>
          </div>
        ))}
      </div>
    </section>
  );
}

function ResultDrawer({ test, resultId, onClose }: { test: SyntheticTest; resultId: number; onClose: () => void }) {
  const q = useSyntheticResult(test.id, resultId);
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === 'Escape' && onClose();
    globalThis.addEventListener('keydown', onKey);
    return () => globalThis.removeEventListener('keydown', onKey);
  }, [onClose]);
  return createPortal(
    <div className="syn-drawer-backdrop" onMouseDown={(e) => e.target === e.currentTarget && onClose()}>
      <aside className="syn-drawer" role="dialog" aria-label="Test result">
        <div className="syn-drawer-head">
          <h2>{q.data ? `${q.data.detail.passed ? 'Passed' : 'Failed'} · ${fmtDateTime(q.data.startedAt)}` : 'Test result'}</h2>
          <div className="syn-drawer-actions">
            <button className="icon-plain" title="Close" onClick={onClose}>
              <X size={18} />
            </button>
          </div>
        </div>
        <div className="syn-drawer-body">
          {q.isPending ? (
            <Loading label="Loading result…" />
          ) : q.isError ? (
            <ErrorState error={q.error} onRetry={() => q.refetch()} />
          ) : (
            <>
              <p className="muted">
                {test.name} · from ObserveCraft server · {q.data.trigger === 'manual' ? 'run by hand' : 'scheduled run'}
                {q.data.attempts > 1 ? ` · ${q.data.attempts} attempts` : ''}
              </p>
              <RunView run={q.data.detail} type={test.type} />
              {q.data.detail.passed && q.data.trigger === 'scheduled' && <p className="muted">Bodies of passing scheduled runs aren't kept.</p>}
            </>
          )}
        </div>
      </aside>
    </div>,
    document.body,
  );
}
