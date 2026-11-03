// Assist Investigation: investigations of monitor alerts (by hand or automatically) and of questions,
// the monitors that can be investigated, and reports.
import { useMemo, useState } from 'react';
import { Link, Outlet, useNavigate, useParams, useSearchParams } from 'react-router-dom';
import type { EChartsOption } from 'echarts';
import { BellRing, BookOpen, Brain, ChevronRight, Lightbulb, Network, Play, Plus, Search, Settings, Sparkles } from 'lucide-react';
import type { InvestigationMonitor, RunCount } from '../../api/assist/client';
import { Chart } from '../../components/charts/Chart';
import { TimePicker } from '../../components/shell/PageHeader';
import { TimeProvider, useTime } from '../../components/shell/TimeContext';
import { ErrorState, Loading } from '../../components/ui/AsyncState';
import { Select } from '../../components/ui/Dropdown';
import { timeAgo } from '../../lib/format';
import { palette } from '../../theme/chartTheme';
import { useProfile } from '../settings/queries';
import { duration, ProductHeader, RunView, StatusBadge, VerdictBadge } from './RunView';
import { useInvestigationMonitors, useRunReport, useRuns, useRunSettings, useSetMonitorAuto, useStartRun } from './runsQueries';
import './assist.css';

const TABS = [
  { to: '/assist/investigations', label: 'List', end: true },
  { to: '/assist/investigations/monitors', label: 'Monitors' },
  { to: '/assist/investigations/reports', label: 'Reports' },
];

export function InvestigationLayout() {
  return (
    <div className="ai-product">
      <ProductHeader
        icon={<Sparkles size={20} />}
        name="Assist Investigation"
        tabs={TABS}
        actions={
          <>
            <Link className="link-muted ai-header-link" to="/assist/investigations/settings">
              <Settings size={15} /> Settings
            </Link>
            <Link className="btn btn-primary" to="/assist/investigations/new">
              <Plus size={15} /> New Investigation
            </Link>
          </>
        }
      />
      <div className="ai-product-body">
        <Outlet />
      </div>
    </div>
  );
}

// ---------------------------------------------------------------- list

function GetStarted({ monitors }: { monitors: InvestigationMonitor[] }) {
  const navigate = useNavigate();
  const start = useStartRun();
  const settings = useRunSettings();
  const supported = monitors.filter((m) => m.supported);
  const alerts = supported.flatMap((m) => m.recentAlerts.map((a) => ({ m, a }))).sort((x, y) => y.a.at - x.a.at);
  const steps = [
    { done: false, title: 'Investigate a recent alert', to: '/assist/investigations/monitors' },
    { done: (settings.data?.knowledge ?? '').trim().length > 0, title: 'Write the knowledge base', to: '/assist/investigations/settings' },
    { done: (settings.data?.notify ?? []).length > 0, title: 'Set up notifications', to: '/assist/investigations/settings/notifications' },
    { done: monitors.some((m) => m.autoInvestigate), title: 'Automate monitor investigations', to: '/assist/investigations/monitors' },
  ];
  return (
    <section className="ai-card ai-start">
      <h2>Get started with AI-powered investigations</h2>
      <p className="muted">Assist reads your logs, traces, metrics, error issues and Kubernetes to find why an alert fired, and concludes with evidence and next steps.</p>
      <div className="ai-alerts">
        <div className="ai-row ai-spread">
          <b>
            <BellRing size={14} /> Latest alerts
          </b>
          <Link to="/assist/investigations/monitors">View all supported monitors ({supported.length})</Link>
        </div>
        {alerts.length === 0 && <p className="muted">No recent alerts. When a supported monitor fires, its alerts appear here so you can investigate them.</p>}
        {alerts.slice(0, 4).map(({ m, a }) => (
          <div key={`${m.id}-${a.at}`} className="ai-alert-row">
            <span className="ai-alert-dot" />
            <span className="ellipsis">{m.name}</span>
            <span className="muted">
              {a.group} · {timeAgo(a.at)}
            </span>
            <button
              className="btn btn-sm"
              disabled={start.isPending}
              onClick={() => start.mutate({ kind: 'investigation', monitorId: m.id, question: null, signalId: null }, { onSuccess: (r) => navigate(`/assist/investigations/${r.id}`) })}
            >
              <Play size={12} /> Investigate
            </button>
          </div>
        ))}
      </div>
      <ol className="ai-checklist">
        {steps.map((s) => (
          <li key={s.title} className={s.done ? 'done' : ''}>
            <Link to={s.to}>
              {s.title} <ChevronRight size={14} />
            </Link>
          </li>
        ))}
      </ol>
    </section>
  );
}

export function InvestigationList() {
  const me = useProfile();
  const [params] = useSearchParams();
  const monitorId = params.get('monitor') ? Number(params.get('monitor')) : null;
  const runs = useRuns({ kind: 'investigation', limit: 200, monitorId, status: null });
  const monitors = useInvestigationMonitors();
  const [text, setText] = useState('');
  if (runs.isPending || monitors.isPending) return <Loading />;
  if (runs.isError) return <ErrorState error={runs.error} onRetry={() => runs.refetch()} />;
  const list = runs.data.filter((r) => !text.trim() || r.title.toLowerCase().includes(text.toLowerCase()));
  const first = (me.data?.name ?? '').split(/\s+/)[0];
  return (
    <div className="ai-page-pad">
      {runs.data.length < 3 && monitorId == null && (
        <>
          <h1 className="ai-welcome">Welcome to Assist Investigation{first && `, ${first}`}.</h1>
          <GetStarted monitors={monitors.data ?? []} />
        </>
      )}
      <div className="ai-row ai-spread ai-list-head">
        <h2>Investigations</h2>
        <div className="ai-filter">
          <Search size={15} />
          <input placeholder="Search investigations" value={text} onChange={(e) => setText(e.target.value)} />
        </div>
      </div>
      <table className="ai-table">
        <thead>
          <tr>
            <th>STATUS</th>
            <th>INVESTIGATION</th>
            <th>VERDICT</th>
            <th>TRIGGER</th>
            <th>STARTED</th>
            <th>DURATION</th>
          </tr>
        </thead>
        <tbody>
          {list.map((r) => (
            <tr key={r.id}>
              <td>
                <StatusBadge status={r.status} />
              </td>
              <td>
                <Link to={`/assist/investigations/${r.id}`}>{r.title}</Link>
                {r.rootCause && <div className="muted ai-sub">{r.rootCause}</div>}
              </td>
              <td>
                <VerdictBadge verdict={r.verdict} />
              </td>
              <td>{r.trigger === 'monitor' ? 'Auto-investigate' : r.trigger === 'manual' ? r.createdBy : r.trigger}</td>
              <td className="muted">{timeAgo(r.createdAt)}</td>
              <td className="muted">{duration(r)}</td>
            </tr>
          ))}
          {list.length === 0 && (
            <tr>
              <td colSpan={6} className="muted ai-empty-cell">
                No investigations yet.
              </td>
            </tr>
          )}
        </tbody>
      </table>
    </div>
  );
}

// ---------------------------------------------------------------- monitors

export function InvestigationMonitors() {
  const navigate = useNavigate();
  const q = useInvestigationMonitors();
  const start = useStartRun();
  const auto = useSetMonitorAuto();
  const [text, setText] = useState('');
  const [status, setStatus] = useState('*');
  const [priority, setPriority] = useState('*');
  const [team, setTeam] = useState('*');
  const [supported, setSupported] = useState(true);
  const [autoOnly, setAutoOnly] = useState(false);
  if (q.isPending) return <Loading />;
  if (q.isError) return <ErrorState error={q.error} onRetry={() => q.refetch()} />;
  const teams = [...new Set(q.data.flatMap((m) => m.teams))].sort();
  const t = text.trim().toLowerCase();
  const shown = q.data.filter(
    (m) =>
      m.supported === supported &&
      (!autoOnly || m.autoInvestigate) &&
      (status === '*' || m.status === status) &&
      (priority === '*' || String(m.priority ?? '') === priority) &&
      (team === '*' || m.teams.includes(team)) &&
      (!t || m.name.toLowerCase().includes(t) || m.tags.some((g) => g.includes(t))),
  );
  const investigate = (m: InvestigationMonitor) =>
    start.mutate({ kind: 'investigation', monitorId: m.id, question: null, signalId: null }, { onSuccess: (r) => navigate(`/assist/investigations/${r.id}`) });
  return (
    <div className="ai-page-pad">
      <h1 className="ai-page-title">Monitor Management</h1>
      <div className="ai-filter ai-filter-wide">
        <Search size={15} />
        <input placeholder="Filter monitors" value={text} onChange={(e) => setText(e.target.value)} />
      </div>
      <div className="ai-row ai-filters">
        <Select
          label="Status"
          value={status}
          onChange={setStatus}
          options={[{ value: '*', label: 'All' }, ...['alert', 'warn', 'nodata', 'ok'].map((s) => ({ value: s, label: s === 'nodata' ? 'No Data' : s[0].toUpperCase() + s.slice(1) }))]}
        />
        <Select label="Priority" value={priority} onChange={setPriority} options={[{ value: '*', label: 'All' }, ...[1, 2, 3, 4, 5].map((p) => ({ value: String(p), label: `P${p}` }))]} />
        <Select label="Team" value={team} onChange={setTeam} options={[{ value: '*', label: 'All' }, ...teams.map((x) => ({ value: x, label: x }))]} />
        <label className="ai-auto ai-push">
          <button type="button" role="switch" aria-checked={autoOnly} className={`switch${autoOnly ? ' on' : ''}`} onClick={() => setAutoOnly(!autoOnly)} />
          Auto-Investigate Only
        </label>
      </div>
      <div className="ai-row ai-segment">
        <button className={supported ? 'active' : ''} onClick={() => setSupported(true)}>
          <Sparkles size={14} /> Supported <span className="ai-count">{q.data.filter((m) => m.supported).length}</span>
        </button>
        <button className={!supported ? 'active' : ''} onClick={() => setSupported(false)}>
          Unsupported <span className="ai-count">{q.data.filter((m) => !m.supported).length}</span>
        </button>
      </div>
      <p className="muted">
        {supported
          ? 'Assist can investigate these monitors. Start an investigation for a recent alert, or turn on automatic investigation of every alert.'
          : 'Synthetic tests can’t be investigated yet: their data comes from test runs, not from your services.'}
      </p>
      <table className="ai-table">
        <thead>
          <tr>
            <th>MONITOR</th>
            <th>INVESTIGATE RECENT ALERT</th>
            <th>AUTO-INVESTIGATE</th>
            <th>TOTAL INVESTIGATIONS</th>
            <th>MEMORIES</th>
          </tr>
        </thead>
        <tbody>
          {shown.map((m) => (
            <tr key={m.id}>
              <td>
                <Link to={`/monitors/${m.id}`}>{m.name}</Link>
                <div className="ai-tags">
                  {m.tags.slice(0, 4).map((g) => (
                    <span key={g} className="ai-tag">
                      {g}
                    </span>
                  ))}
                </div>
              </td>
              <td>
                {m.recentAlerts.length ? (
                  <button className="btn btn-sm" disabled={!m.supported || start.isPending} onClick={() => investigate(m)} title={`${m.recentAlerts[0].group}, ${timeAgo(m.recentAlerts[0].at)}`}>
                    <Play size={12} /> Investigate ({timeAgo(m.recentAlerts[0].at)})
                  </button>
                ) : (
                  <button className="btn btn-sm" disabled={!m.supported || start.isPending} onClick={() => investigate(m)}>
                    No recent alerts · Investigate now
                  </button>
                )}
              </td>
              <td>
                <button
                  type="button"
                  role="switch"
                  aria-label={`Auto-investigate ${m.name}`}
                  aria-checked={m.autoInvestigate}
                  disabled={!m.supported || auto.isPending}
                  className={`switch${m.autoInvestigate ? ' on' : ''}`}
                  onClick={() => auto.mutate({ id: m.id, on: !m.autoInvestigate })}
                />
              </td>
              <td>
                <Link className="ai-pill" to={`/assist/investigations?monitor=${m.id}`}>
                  <Network size={13} /> {m.investigations}
                </Link>
              </td>
              <td>
                <Link className="ai-pill" to="/assist/investigations/settings/memories">
                  <Lightbulb size={13} /> {m.memories}
                </Link>
              </td>
            </tr>
          ))}
          {shown.length === 0 && (
            <tr>
              <td colSpan={5} className="muted ai-empty-cell">
                No monitors match.
              </td>
            </tr>
          )}
        </tbody>
      </table>
      {auto.error && <p className="ai-bad">{(auto.error as Error).message}</p>}
      {start.error && <p className="ai-bad">{(start.error as Error).message}</p>}
    </div>
  );
}

// ---------------------------------------------------------------- reports

export function bars(points: { at: number; count: number }[], name: string, color: string): EChartsOption {
  return {
    color: [color],
    grid: { left: 36, right: 12, top: 12, bottom: 24 },
    tooltip: { trigger: 'axis' },
    xAxis: {
      type: 'category',
      data: points.map((p) => new Date(p.at).toLocaleString('en-US', { month: 'short', day: 'numeric', hour: points.length > 1 && points[1].at - points[0].at < 86_400_000 ? '2-digit' : undefined })),
    },
    yAxis: { type: 'value', minInterval: 1 },
    series: [{ name, type: 'bar', data: points.map((p) => p.count), barMaxWidth: 18 }],
  };
}

export function CountList({ rows, label }: { rows: RunCount[]; label?: (n: string) => string }) {
  const max = Math.max(1, ...rows.map((r) => r.count));
  if (!rows.length) return <p className="muted ai-nodata">(No data)</p>;
  return (
    <ul className="ai-surfaces">
      {rows.map((r) => (
        <li key={r.name}>
          <span className="ellipsis">{label ? label(r.name) : r.name}</span>
          <span className="ai-surface-bar">
            <span style={{ width: `${(r.count / max) * 100}%` }} />
          </span>
          <b>{r.count}</b>
        </li>
      ))}
    </ul>
  );
}

function Reports() {
  const { range } = useTime();
  const monitors = useInvestigationMonitors();
  const [monitor, setMonitor] = useState('*');
  const req = useMemo(
    () => ({ kind: 'investigation', range: { from: Math.floor(range.from / 60_000) * 60_000, to: Math.ceil(range.to / 60_000) * 60_000 }, monitorId: monitor === '*' ? null : Number(monitor) }),
    [range.from, range.to, monitor],
  );
  const q = useRunReport(req);
  const r = q.data;
  const colors = palette();
  const mttc = r?.meanSecondsToConclusion;
  return (
    <div className="ai-page-pad">
      <div className="ai-row ai-spread">
        <h1 className="ai-page-title">Reports</h1>
        <div className="ai-row">
          <Select label="Monitor" value={monitor} onChange={setMonitor} options={[{ value: '*', label: 'All' }, ...(monitors.data ?? []).map((m) => ({ value: String(m.id), label: m.name }))]} />
          <TimePicker compact />
        </div>
      </div>
      {q.isError ? (
        <ErrorState error={q.error} onRetry={() => q.refetch()} />
      ) : (
        <div className="ai-report-grid">
          <section className="ai-card ai-kpi">
            <h3>Total Investigations</h3>
            <b>{r?.total ?? '–'}</b>
          </section>
          <section className="ai-card ai-kpi">
            <h3>Mean Time to Conclusion</h3>
            <b>{mttc == null ? '–' : mttc < 60 ? `${Math.round(mttc)}s` : `${Math.round(mttc / 60)}m`}</b>
          </section>
          <section className="ai-card ai-kpi">
            <h3>Concluded</h3>
            <b>{r ? `${r.concluded} / ${r.total}` : '–'}</b>
          </section>
          <section className="ai-card ai-kpi">
            <h3>Helpful</h3>
            <b>{r ? `${r.helpful} of ${r.helpful + r.notHelpful}` : '–'}</b>
          </section>
          <section className="ai-card ai-panel-wide">
            <h3>Investigations Over Time</h3>
            {r?.overTime.length ? <Chart option={bars(r.overTime, 'Investigations', colors[0])} height={200} /> : <p className="muted ai-nodata">(No data)</p>}
          </section>
          <section className="ai-card ai-panel-wide">
            <h3>Verdicts</h3>
            <CountList rows={r?.byVerdict ?? []} label={(n) => n.replace(/_/g, ' ')} />
          </section>
          <section className="ai-card ai-panel-wide">
            <h3>Trigger Method</h3>
            <CountList rows={r?.byTrigger ?? []} label={(n) => (n === 'monitor' ? 'Auto-investigate' : n === 'manual' ? 'Manual' : n)} />
          </section>
          <section className="ai-card ai-panel-wide">
            <h3>Investigations by Monitor</h3>
            <CountList rows={r?.byMonitor ?? []} />
          </section>
        </div>
      )}
    </div>
  );
}

export function InvestigationReports() {
  return (
    <TimeProvider initial="1w">
      <Reports />
    </TimeProvider>
  );
}

// ---------------------------------------------------------------- new and one

export function NewInvestigation() {
  const navigate = useNavigate();
  const monitors = useInvestigationMonitors();
  const start = useStartRun();
  const [question, setQuestion] = useState('');
  const [monitor, setMonitor] = useState('');
  const go = () =>
    start.mutate(
      { kind: 'investigation', monitorId: monitor ? Number(monitor) : null, question: monitor ? null : question, signalId: null },
      { onSuccess: (r) => navigate(`/assist/investigations/${r.id}`) },
    );
  return (
    <div className="ai-page-pad ai-narrow">
      <h1 className="ai-page-title">New Investigation</h1>
      <section className="ai-card">
        <label className="form-label">
          Investigate a monitor’s latest alert
          <select className="settings-input" value={monitor} onChange={(e) => setMonitor(e.target.value)}>
            <option value="">—</option>
            {(monitors.data ?? [])
              .filter((m) => m.supported)
              .map((m) => (
                <option key={m.id} value={m.id}>
                  {m.name}
                </option>
              ))}
          </select>
        </label>
        <p className="muted ai-or">or describe the problem</p>
        <textarea
          className="ai-context"
          rows={5}
          disabled={!!monitor}
          value={question}
          placeholder="Checkout requests have been slow since 09:00 and some fail with 502"
          onChange={(e) => setQuestion(e.target.value)}
        />
        <div className="ai-row">
          <button className="btn btn-primary" disabled={start.isPending || (!monitor && !question.trim())} onClick={go}>
            <Brain size={15} /> {start.isPending ? 'Starting…' : 'Investigate'}
          </button>
          {start.error && <span className="ai-bad">{(start.error as Error).message}</span>}
        </div>
      </section>
      <p className="muted">
        <BookOpen size={13} /> Investigations read your knowledge base and memories (Settings), and use your access: what you can’t see, Assist can’t either.
      </p>
    </div>
  );
}

export function InvestigationRun() {
  const { id } = useParams();
  return <RunView id={id!} back="/assist/investigations" backLabel="Investigations" />;
}
