import { useMemo, useState, type ReactNode } from 'react';
import { Link } from 'react-router-dom';
import type { EChartsOption } from 'echarts';
import {
  AlertCircle,
  ArrowRight,
  BookOpen,
  ChevronDown,
  ChevronRight,
  Container,
  ExternalLink,
  FileSearch,
  Filter,
  LibraryBig,
  ListChecks,
  Monitor,
  PanelLeftClose,
  PanelLeftOpen,
  ScrollText,
  Server,
  ShieldCheck,
  SquareFunction,
} from 'lucide-react';
import type { CsmAgent, WpSeries, WpSummary } from '../../../api/workloadProtection/client';
import { Chart } from '../../../components/charts/Chart';
import { TimePicker } from '../../../components/shell/PageHeader';
import { useTime } from '../../../components/shell/TimeContext';
import { ErrorState, Loading } from '../../../components/ui/AsyncState';
import { token } from '../../../theme/chartTheme';
import { useWpAgentsList, useWpSummary } from './overviewQueries';
import '../appsec/overview.css';
import './overview.css';

const MIN = 60_000;
const BASE = '/security/workload-protection';
const DOCS = 'https://github.com/observecraft/observecraft/blob/main/docs/sending-data.md';
const NAV: [string, string][] = [
  ['overview', 'Overview'],
  ['activity', 'Workload Activity'],
  ['coverage', 'Coverage'],
  ['packs', 'Content Packs'],
  ['deeper', 'Dive deeper'],
];
const CATEGORIES: [string, string][] = [
  ['file', 'File'],
  ['process', 'Process'],
  ['network', 'Network'],
  ['kernel', 'Kernel'],
];
/** Archive reasons of triaged signals, in the order the triage list shows them. */
const TRIAGE: [string, string][] = [
  ['remediated', 'Remediated'],
  ['true_positive_malicious', 'True Positive: Malicious'],
  ['true_positive_benign', 'True Positive: Benign'],
  ['investigated_case_opened', 'Investigated - Case Opened'],
  ['testing_or_maintenance', 'Testing/Maintenance'],
  ['false_positive', 'False Positive'],
  ['other', 'Other'],
];
const SEVERITIES: [string, string][] = [
  ['critical', '--sev-critical'],
  ['high', '--sev-high'],
  ['medium', '--sev-medium'],
  ['low', '--sev-low'],
  ['info', '--sev-info'],
];
const CATEGORY_COLORS = ['--chart-1', '--chart-2', '--chart-3', '--chart-4'];

/** 4300000000 → "4.3B": the event counts run into the billions. */
function big(n: number): string {
  if (n < 1000) return String(n);
  if (n < 1e6) return `${+(n / 1e3).toFixed(1)}K`;
  if (n < 1e9) return `${+(n / 1e6).toFixed(1)}M`;
  return `${+(n / 1e9).toFixed(1)}B`;
}

/** Workload Protection → Overview: what the Agents watch, what the rules catch, coverage and content packs. */
export function WorkloadOverviewPage() {
  const { range } = useTime();
  const r = useMemo(() => ({ from: Math.floor(range.from / MIN) * MIN, to: Math.ceil(range.to / MIN) * MIN }), [range.from, range.to]);
  const q = useWpSummary(r);
  const agentsQ = useWpAgentsList();
  const [active, setActive] = useState('overview');
  const [collapsed, setCollapsed] = useState(false);
  const [env, setEnv] = useState('');
  const all = useMemo(() => [...(agentsQ.data?.hosts ?? []), ...(agentsQ.data?.serverless ?? [])], [agentsQ.data]);
  const envs = [...new Set(all.flatMap((a) => a.envs ?? []))].sort();
  const keep = (a: CsmAgent) => !env || (a.envs ?? []).includes(env);
  const go = (id: string) => {
    setActive(id);
    document.getElementById(`wp-${id}`)?.scrollIntoView({ behavior: 'smooth', block: 'start' });
  };
  const s = q.data;
  // With an env chosen, the Agent counts come from the Agents of that env; otherwise from the summary.
  const agents = env
    ? (() => {
        const hosts = (agentsQ.data?.hosts ?? []).filter(keep);
        const tasks = (agentsQ.data?.serverless ?? []).filter(keep);
        return { hosts: hosts.length, hostsEnabled: hosts.filter((a) => a.isCwsEnabled).length, serverless: tasks.length, serverlessEnabled: tasks.filter((a) => a.isCwsEnabled).length };
      })()
    : s?.agents;
  return (
    <div className={`ov-layout${collapsed ? ' collapsed' : ''}`}>
      <aside className="ov-nav">
        <div className="ov-nav-groups">
          <div className="ov-nav-group">
            <button className="ov-nav-head on" onClick={() => go('overview')} title="Summary">
              <Monitor size={15} /> {!collapsed && 'Summary'}
            </button>
            {!collapsed &&
              NAV.map(([id, label]) => (
                <button key={id} className={`ov-nav-item${active === id ? ' active' : ''}`} onClick={() => go(id)}>
                  {label}
                </button>
              ))}
          </div>
        </div>
        <div className="ov-nav-foot">
          <a href={DOCS} target="_blank" rel="noreferrer" title="Documentation">
            <LibraryBig size={15} /> {!collapsed && 'Documentation'} {!collapsed && <ExternalLink size={13} className="ov-push" />}
          </a>
          <button onClick={() => setCollapsed((c) => !c)} title={collapsed ? 'Expand' : 'Collapse'}>
            {collapsed ? <PanelLeftOpen size={15} /> : <PanelLeftClose size={15} />} {!collapsed && 'Collapse'}
          </button>
        </div>
      </aside>
      <div className="ov-main">
        <div className="ov-filters">
          <span className="ov-filter-by">
            <Filter size={14} /> Filter this page by
          </span>
          <label className="ov-select">
            <span>Env</span>
            <select value={env} onChange={(e) => setEnv(e.target.value)} aria-label="Env">
              <option value="">All</option>
              {envs.map((e) => (
                <option key={e}>{e}</option>
              ))}
            </select>
          </label>
          <span className="syn-grow" />
          <TimePicker compact />
        </div>
        {q.isLoading ? (
          <Loading />
        ) : q.isError ? (
          <ErrorState error={q.error} onRetry={() => q.refetch()} />
        ) : (
          <>
            <Flow s={s!} agents={agents ?? s!.agents} />
            <Activity s={s!} range={r} />
            <Coverage agents={agents ?? s!.agents} />
            <Packs s={s!} />
            <DiveDeeper />
          </>
        )}
      </div>
    </div>
  );
}

type Agents = WpSummary['agents'];

// ---------------------------------------------------------------- the flow diagram

/** "Protect your infrastructure": from the Agents' events, through the agent rules, to signals and findings. */
function Flow({ s, agents }: { s: WpSummary; agents: Agents }) {
  const monitored = sum(s.monitored);
  const captured = sum(s.captured);
  const sev = (o: Record<string, number>, keys: string[]) => keys.reduce((a, k) => a + (o[k] ?? 0), 0);
  return (
    <section id="wp-overview" className="wp-flow-wrap">
      <h1 className="wp-hero">Protect your infrastructure</h1>
      <div className="wp-flow">
        <div className="wp-infra">
          <h3>Your Infrastructure</h3>
          <div className="wp-card">
            <div className="wp-card-head">
              <b>AGENTS</b>
              <Link to={`${BASE}/inventory`} className="wp-count-link">
                {agents.hostsEnabled + agents.serverlessEnabled} <ExternalLink size={13} />
              </Link>
            </div>
            <div className="wp-chips">
              <Link to={`${BASE}/inventory`} className="wp-chip">
                <Server size={13} /> <b className={agents.hostsEnabled ? '' : 'wp-red'}>{agents.hostsEnabled}</b> / {agents.hosts} hosts and containers
              </Link>
              <Link to={`${BASE}/inventory/serverless`} className="wp-chip">
                <SquareFunction size={13} /> <b className={agents.serverlessEnabled ? '' : 'wp-red'}>{agents.serverlessEnabled}</b> / {agents.serverless} serverless tasks
              </Link>
            </div>
          </div>
          <div className="wp-card wp-events-card">
            <div className="wp-events">
              <b>WORKLOAD EVENTS</b>
              {CATEGORIES.map(([k, label]) => (
                <div key={k} className="wp-event-row">
                  <span>{label.toUpperCase()}</span>
                  <b>{big(s.monitored[k] ?? 0)}</b>
                </div>
              ))}
            </div>
            <svg className="wp-funnel" viewBox="0 0 40 100" preserveAspectRatio="none" aria-hidden>
              {[18, 36, 54, 72].map((y) => (
                <path key={y} d={`M0 ${y} C 22 ${y}, 18 62, 40 62`} />
              ))}
            </svg>
            <div className="wp-policies">
              <div className="wp-card-head">
                <b>POLICIES</b>
                <Link to={`${BASE}/policies`} className="wp-count-link">
                  {s.policies} <ExternalLink size={13} />
                </Link>
              </div>
              <div className="wp-card wp-rules">
                <div className="wp-card-head">
                  <b>AGENT RULES</b>
                  <Link to={`${BASE}/policies`} className="wp-count-link">
                    {s.agentRules} <ExternalLink size={13} />
                  </Link>
                </div>
                <Pipe left={big(monitored)} right={<Mark n={captured} />} leftLabel="Monitored Events" rightLabel="Captured Events" />
              </div>
            </div>
          </div>
        </div>
        <svg className="wp-ribbon wp-ribbon-1" viewBox="0 0 40 100" preserveAspectRatio="none" aria-hidden>
          <path d="M0 66 C 20 66, 20 30, 40 30" />
          <path d="M0 66 C 20 66, 20 76, 40 76" />
        </svg>
        <div className="wp-env">
          <h3>ObserveCraft Environment</h3>
          <div className="wp-env-grid">
            <div className="wp-stage">
              <h4>Threat Detection</h4>
              <div className="wp-card">
                <div className="wp-card-head">
                  <b>DETECTION RULES</b>
                  <Link to={`${BASE}/policies/detection-rules`} className="wp-count-link">
                    {s.detectionRules} <ExternalLink size={13} />
                  </Link>
                </div>
                <p className="muted wp-small">Real-time detection based on agent events</p>
                <Pipe left={<Mark n={captured} />} right={sev(s.signals, ['critical', 'high', 'medium', 'low', 'info'])} leftLabel="Captured Events" rightLabel="Signals" />
              </div>
            </div>
            <svg className="wp-ribbon" viewBox="0 0 30 100" preserveAspectRatio="none" aria-hidden>
              <path d="M0 50 C 15 50, 15 25, 30 25" />
              <path d="M0 50 C 15 50, 15 75, 30 75" />
            </svg>
            <div className="wp-outcome">
              <h4>Signals</h4>
              <Outcome n={sev(s.signals, ['critical', 'high'])} label="CRITICAL & HIGH SEVERITY" to={`${BASE}/signals?severity=critical,high`} />
              <Outcome n={sev(s.signals, ['medium', 'low', 'info'])} label="MEDIUM, LOW & INFO" to={`${BASE}/signals?severity=medium,low,info`} />
            </div>
            <div className="wp-stage">
              <h4>Posture Assessment</h4>
              <div className="wp-card">
                <div className="wp-card-head">
                  <b>FINDING RULES</b>
                  <Link to={`${BASE}/policies/finding-rules`} className="wp-count-link">
                    {s.findingRules} <ExternalLink size={13} />
                  </Link>
                </div>
                <Pipe left={<Mark n={captured} />} right={sev(s.findings, ['critical', 'high', 'medium', 'low', 'info'])} leftLabel="Events" rightLabel="Findings" />
              </div>
            </div>
            <svg className="wp-ribbon" viewBox="0 0 30 100" preserveAspectRatio="none" aria-hidden>
              <path d="M0 50 C 15 50, 15 25, 30 25" />
              <path d="M0 50 C 15 50, 15 75, 30 75" />
            </svg>
            <div className="wp-outcome">
              <h4>Findings</h4>
              <Outcome n={sev(s.findings, ['critical', 'high'])} label="CRITICAL & HIGH SEVERITY" to={`${BASE}/findings`} />
              <Outcome n={sev(s.findings, ['medium', 'low', 'info'])} label="MEDIUM, LOW & INFO" to={`${BASE}/findings`} />
            </div>
          </div>
        </div>
      </div>
    </section>
  );
}

/** A count, or a red mark when nothing went through (as the reference shows for empty stages). */
function Mark({ n }: { n: number }) {
  return n ? <>{big(n)}</> : <AlertCircle size={16} className="wp-mark" aria-label="No events" />;
}

function Pipe({ left, right, leftLabel, rightLabel }: { left: ReactNode; right: ReactNode; leftLabel: string; rightLabel: string }) {
  return (
    <div className="wp-pipe">
      <div className="wp-pipe-row">
        <ChevronRight size={16} className="wp-chev" />
        <b>{left}</b>
        <span className="wp-arrow" />
        <b>{right}</b>
      </div>
      <div className="wp-pipe-labels">
        <span>{leftLabel}</span>
        <span>{rightLabel}</span>
      </div>
    </div>
  );
}

function Outcome({ n, label, to }: { n: number; label: string; to: string }) {
  return (
    <Link to={to} className="wp-outcome-card">
      <ChevronRight size={18} className="wp-chev" />
      <b>{n}</b>
      <span>{label}</span>
    </Link>
  );
}

// ---------------------------------------------------------------- workload activity

function Activity({ s, range }: { s: WpSummary; range: { from: number; to: number } }) {
  const counts = new Map(s.triage.map((t) => [t.name, t.count]));
  const total = s.triage.reduce((a, t) => a + t.count, 0);
  const triaged = TRIAGE.reduce((a, [k]) => a + (counts.get(k) ?? 0), 0);
  const [open, setOpen] = useState<string | null>(null);
  // Signals are counted per day, so their axis follows the buckets rather than the picked range.
  const signalChart = useMemo(() => bars(s.signalSeries, null, SEVERITIES), [s.signalSeries]);
  const capturedChart = useMemo(
    () =>
      bars(
        s.capturedSeries,
        range,
        CATEGORIES.map(([k], i) => [k, CATEGORY_COLORS[i]]),
      ),
    [s.capturedSeries, range],
  );
  return (
    <section id="wp-activity" className="wp-section">
      <h2>Workload Activity</h2>
      <div className="wp-grid two">
        <div className="wp-panel">
          <header>
            <b>SIGNAL TRIAGE</b>
            <span className="syn-grow" />
            <span className="muted wp-small">
              Triaged {triaged} of {total} signals
            </span>
            <Link className="btn btn-sm" to={`${BASE}/signals?state=archived`}>
              Go to Triaged Signals <ExternalLink size={13} />
            </Link>
          </header>
          <ul className="wp-triage">
            {TRIAGE.map(([k, label]) => (
              <li key={k}>
                <button onClick={() => setOpen(open === k ? null : k)}>
                  {open === k ? <ChevronDown size={14} /> : <ChevronRight size={14} />} {label}
                  <span className="wp-badge">{counts.get(k) ?? 0}</span>
                </button>
                {open === k && (
                  <p className="muted wp-small">
                    {counts.get(k) ? (
                      <Link to={`${BASE}/signals?state=archived&reason=${k}`}>
                        View the {counts.get(k)} signals archived as {label} <ArrowRight size={12} />
                      </Link>
                    ) : (
                      `No signal was archived as ${label} in this time range.`
                    )}
                  </p>
                )}
              </li>
            ))}
          </ul>
        </div>
        <div className="wp-panel">
          <header>
            <b>SIGNAL TRENDS</b>
            <span className="syn-grow" />
            <Link className="btn btn-sm" to={`${BASE}/signals`}>
              Go to Signals <ExternalLink size={13} />
            </Link>
          </header>
          <Chart option={signalChart} height={250} />
        </div>
      </div>
      <div className="wp-grid narrow-wide">
        <div className="wp-panel">
          <header>
            <b>MONITORED EVENTS</b>
            <span className="muted wp-small" title="Events the Agents' probes saw, before the agent rules filter them">
              (?)
            </span>
          </header>
          <div className="wp-values">
            {CATEGORIES.map(([k, label]) => (
              <QueryValue key={k} label={`${label} Events`} value={s.monitored[k] ?? 0} points={groupValues(s.monitoredSeries, k)} />
            ))}
          </div>
        </div>
        <div className="wp-panel">
          <header>
            <b>CAPTURED EVENTS</b>
            <span className="muted wp-small" title="Events that matched an agent rule and were sent">
              (?)
            </span>
            <span className="syn-grow" />
            <Link className="btn btn-sm" to={`${BASE}/investigate`}>
              Go to Explorer <ExternalLink size={13} />
            </Link>
          </header>
          <Chart option={capturedChart} height={260} />
        </div>
      </div>
    </section>
  );
}

/** A query value with its series drawn faintly behind it. */
function QueryValue({ label, value, points }: { label: string; value: number; points: number[] }) {
  const max = Math.max(1, ...points);
  const d = points.length > 1 ? points.map((p, i) => `${(i / (points.length - 1)) * 100},${40 - (p / max) * 36}`).join(' ') : '';
  return (
    <div className="wp-value">
      {value > 0 ? (
        <>
          {d && (
            <svg viewBox="0 0 100 40" preserveAspectRatio="none" aria-hidden>
              <polygon points={`0,40 ${d} 100,40`} />
            </svg>
          )}
          <b>{big(value)}</b>
        </>
      ) : (
        <span className="muted">(No data)</span>
      )}
      <span className="wp-value-label">{label}</span>
    </div>
  );
}

// ---------------------------------------------------------------- coverage

function Coverage({ agents }: { agents: Agents }) {
  return (
    <section id="wp-coverage" className="wp-section">
      <h2>Coverage</h2>
      <div className="wp-grid two">
        <CoverageCard title="HOSTS AND CONTAINERS" icon={<Server size={16} />} on={agents.hostsEnabled} of={agents.hosts} unit="hosts" to={`${BASE}/inventory`} />
        <CoverageCard title="SERVERLESS RESOURCES" icon={<SquareFunction size={16} />} on={agents.serverlessEnabled} of={agents.serverless} unit="tasks" to={`${BASE}/inventory/serverless`} />
      </div>
    </section>
  );
}

function CoverageCard({ title, icon, on, of, unit, to }: { title: string; icon: ReactNode; on: number; of: number; unit: string; to: string }) {
  const pct = of ? Math.round((on / of) * 100) : 0;
  const level = !of ? '' : pct >= 80 ? 'good' : pct >= 40 ? 'fair' : 'low';
  return (
    <div className="wp-panel">
      <header>
        <b>{title}</b>
        <span className="syn-grow" />
        <Link className="btn btn-sm" to={to}>
          View inventory <ExternalLink size={13} />
        </Link>
      </header>
      <div className="wp-cov">
        <span className="wp-cov-icon">{icon}</span>
        <b className="wp-cov-num">{on}</b>
        <span className="muted">
          of {of} {unit} with Workload Protection
        </span>
        {level && <span className={`wp-cov-level ${level}`}>{level === 'good' ? 'Good coverage' : level === 'fair' ? 'Partial coverage' : 'Very low coverage'}</span>}
      </div>
      <div className="wp-cov-bar">
        <i style={{ width: `${pct}%` }} />
      </div>
    </div>
  );
}

// ---------------------------------------------------------------- policies, MITRE and content packs

function Packs({ s }: { s: WpSummary }) {
  const maxTactic = Math.max(1, ...s.tactics.map((t) => t.rules));
  const tacticRules = s.tactics.reduce((a, t) => a + t.rules, 0);
  return (
    <section id="wp-packs" className="wp-section">
      <div className="wp-panel">
        <header>
          <b>AGENT POLICIES</b>
          <span className="syn-grow" />
          <Link className="btn btn-sm" to={`${BASE}/policies`}>
            Go to Agent Policies <ExternalLink size={13} />
          </Link>
        </header>
        <div className="wp-grid three">
          {s.agentPolicies.map((p) => (
            <Link key={p.id} to={`${BASE}/policies?policy=${p.id}`} className="wp-policy">
              <div>
                <b>{p.name}</b>
                <span className="muted wp-small">{p.description}</span>
              </div>
              <div className="wp-policy-foot">
                <span>Rules</span>
                <b>{p.rules}</b>
              </div>
            </Link>
          ))}
        </div>
      </div>
      <div className="wp-panel">
        <header>
          <b>MITRE TACTICS COVERAGE</b>
          <span className="wp-small">
            across{' '}
            <Link to={`${BASE}/policies/detection-rules`}>
              {tacticRules} rules <ExternalLink size={12} />
            </Link>
          </span>
          <span className="syn-grow" />
          <Link className="btn btn-sm" to={`${BASE}/policies/detection-rules?view=mitre`}>
            Go to MITRE ATT&amp;CK Map <ExternalLink size={13} />
          </Link>
        </header>
        <div className="wp-tactics">
          {s.tactics.map((t) => (
            <div key={t.id} className="wp-tactic" title={`${t.name}: ${t.rules} rules`}>
              <b>{t.name}</b>
              <span className="wp-tactic-bar">
                <i style={{ width: `${(t.rules / maxTactic) * 100}%` }} />
              </span>
            </div>
          ))}
        </div>
      </div>
      <h2>Content Packs</h2>
      <div className="wp-packs">
        {s.packs.map((p) => (
          <Link key={p.id} to={`${BASE}/policies/detection-rules?pack=${p.id}`} className="wp-pack">
            <div className="wp-pack-head">{p.name}</div>
            <p>{p.description}</p>
            <div className="wp-pack-rules">
              <b>{p.rules}</b> {p.rules === 1 ? 'rule' : 'rules'} in this content pack
              {p.status === 'active' && <span className="muted"> · {p.enabledRules} enabled</span>}
            </div>
            <span className={`wp-pack-status ${p.status}`}>{p.status === 'active' ? 'ENABLED' : 'AVAILABLE'}</span>
          </Link>
        ))}
      </div>
    </section>
  );
}

// ---------------------------------------------------------------- dive deeper

function DiveDeeper() {
  const links: [typeof Server, string, string, string][] = [
    [FileSearch, 'Agent Events Explorer', 'Search every event the agent rules captured, by process, file, network or kernel activity.', `${BASE}/investigate`],
    [ShieldCheck, 'Signals', 'Triage the threats the detection rules raised on your hosts and containers.', `${BASE}/signals`],
    [ListChecks, 'Findings', 'Review the posture findings of your workloads and mute the accepted ones.', `${BASE}/findings`],
    [ScrollText, 'Detection rules', 'Tune the rules that turn captured events into signals.', `${BASE}/policies/detection-rules`],
    [Container, 'Hosts & Containers', 'See which Agents run Workload Protection and which still need it.', `${BASE}/inventory`],
    [BookOpen, 'Set up the Agent', 'Turn on runtime security in the Datadog Agent and send its events to ObserveCraft.', DOCS],
  ];
  return (
    <section id="wp-deeper" className="wp-section">
      <h2>Dive deeper</h2>
      <div className="wp-grid three">
        {links.map(([Icon, title, text, to]) =>
          to.startsWith('http') ? (
            <a key={title} href={to} target="_blank" rel="noreferrer" className="wp-deeper">
              <Icon size={18} />
              <b>
                {title} <ExternalLink size={12} />
              </b>
              <span className="muted wp-small">{text}</span>
            </a>
          ) : (
            <Link key={title} to={to} className="wp-deeper">
              <Icon size={18} />
              <b>{title}</b>
              <span className="muted wp-small">{text}</span>
            </Link>
          ),
        )}
      </div>
    </section>
  );
}

// ---------------------------------------------------------------- helpers

function sum(o: Record<string, number>): number {
  return Object.values(o).reduce((a, n) => a + n, 0);
}

function groupValues(series: WpSeries, name: string): number[] {
  return series.groups.find((g) => g.name === name)?.values ?? [];
}

/** Stacked bars of a summary series, one stack per group, in the given order and colours; the x axis spans `range` when given. */
function bars(series: WpSeries, range: { from: number; to: number } | null, order: [string, string][]): EChartsOption {
  return {
    grid: { left: 40, right: 12, top: 12, bottom: 44 },
    tooltip: { trigger: 'axis' },
    legend: { bottom: 0, type: 'scroll' },
    xAxis: range ? { type: 'time', min: range.from, max: range.to } : { type: 'time' },
    yAxis: { type: 'value', minInterval: 1, axisLabel: { formatter: (v: number) => big(v) } },
    series: order.map(([name, color]) => ({
      name,
      type: 'bar',
      stack: 'all',
      barMaxWidth: 14,
      itemStyle: { color: token(color) },
      data: series.times.map((t, i) => [t, series.groups.find((g) => g.name === name)?.values[i] ?? 0]),
    })),
  };
}
