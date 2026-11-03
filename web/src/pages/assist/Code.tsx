// Assist Code: coding sessions on the org's repositories (read the code, propose a change, open a pull
// request), automations that start sessions on a schedule, usage, and settings.
import { useEffect, useMemo, useState } from 'react';
import { Link, NavLink, Outlet, useNavigate, useParams } from 'react-router-dom';
import { ArrowUp, BarChart3, Code2, FolderGit2, GitPullRequest, Plus, Search, Trash2, Wrench } from 'lucide-react';
import type { AgentInput, CodeSettings } from '../../api/assist/client';
import { Chart } from '../../components/charts/Chart';
import { TimePicker } from '../../components/shell/PageHeader';
import { TimeProvider, useTime } from '../../components/shell/TimeContext';
import { ErrorState, Loading } from '../../components/ui/AsyncState';
import { timeAgo } from '../../lib/format';
import { palette } from '../../theme/chartTheme';
import { AgentTable } from './Agents';
import { bars } from './Investigation';
import { ProductHeader, RunView, StatusBadge, VerdictBadge, duration } from './RunView';
import { useAddRepo, useAgents, useCodeConnections, useCodeSettings, useCodeUsage, useRemoveRepo, useRepos, useRuns, useSaveAgent, useSaveCodeSettings, useStartCodeSession } from './runsQueries';
import './assist.css';

const TABS = [
  { to: '/assist/code', label: 'Sessions', end: true },
  { to: '/assist/code/automations', label: 'Automations' },
  { to: '/assist/code/usage', label: 'Usage' },
  { to: '/assist/code/settings', label: 'Settings' },
];

export function CodeLayout() {
  return (
    <div className="ai-product">
      <ProductHeader icon={<Code2 size={20} />} name="Assist Code" tabs={TABS} />
      <div className="ai-product-body">
        <Outlet />
      </div>
    </div>
  );
}

function ConnectIntro() {
  return (
    <section className="ai-hero ai-hero-center">
      <div className="ai-hero-icon">
        <Code2 size={24} />
      </div>
      <h1>A coding agent to fix what your telemetry finds</h1>
      <p>
        Assist reads your repository, finds the code behind an error, a slow endpoint or a vulnerability, and proposes the smallest change that fixes it. You review the diff and open the pull request.
      </p>
      <Link className="btn btn-primary" to="/assist/code/settings/repositories">
        <FolderGit2 size={15} /> Connect source code
      </Link>
      <div className="ai-intro-grid">
        <div>
          <b>Close the loop on investigations</b>
          <span className="muted">Turn a root cause into a pull request.</span>
        </div>
        <div>
          <b>Triage and resolve errors</b>
          <span className="muted">From an Error Tracking issue to a fix and its tests.</span>
        </div>
        <div>
          <b>Fix performance issues</b>
          <span className="muted">Slow queries and N+1 calls found in traces.</span>
        </div>
      </div>
    </section>
  );
}

export function CodeSessions() {
  const navigate = useNavigate();
  const repos = useRepos();
  const runs = useRuns({ kind: 'code', limit: 200, monitorId: null, status: null });
  const start = useStartCodeSession();
  const [repo, setRepo] = useState('');
  const [prompt, setPrompt] = useState('');
  const [text, setText] = useState('');
  useEffect(() => {
    if (!repo && repos.data?.length) setRepo(repos.data[0].repo);
  }, [repos.data, repo]);
  if (repos.isPending) return <Loading />;
  if (repos.isError) return <ErrorState error={repos.error} onRetry={() => repos.refetch()} />;
  if (!repos.data.length) return <ConnectIntro />;
  const go = () => start.mutate({ repo, prompt }, { onSuccess: (r) => navigate(`/assist/code/sessions/${r.id}`) });
  const list = (runs.data ?? []).filter((r) => !text.trim() || r.title.toLowerCase().includes(text.toLowerCase()));
  return (
    <div className="ai-page-pad">
      <div className="ai-composer ai-code-composer">
        <textarea
          rows={3}
          value={prompt}
          placeholder="Ask Assist to fix an issue or change the code"
          onChange={(e) => setPrompt(e.target.value)}
          onKeyDown={(e) => e.key === 'Enter' && (e.metaKey || e.ctrlKey) && prompt.trim() && go()}
        />
        <div className="ai-composer-bar">
          <select className="settings-input ai-repo-select" value={repo} onChange={(e) => setRepo(e.target.value)} aria-label="Repository">
            {repos.data.map((r) => (
              <option key={r.repo} value={r.repo}>
                {r.repo}
              </option>
            ))}
          </select>
          {start.error && <span className="ai-bad">{(start.error as Error).message}</span>}
          <button className="ai-send" title="Start session" disabled={!prompt.trim() || start.isPending} onClick={go}>
            <ArrowUp size={16} />
          </button>
        </div>
      </div>
      <div className="ai-row ai-spread ai-list-head">
        <h2>Sessions</h2>
        <div className="ai-filter">
          <Search size={15} />
          <input placeholder="Search sessions" value={text} onChange={(e) => setText(e.target.value)} />
        </div>
      </div>
      <SessionTable runs={list} />
    </div>
  );
}

function SessionTable({ runs }: { runs: ReturnType<typeof useRuns>['data'] }) {
  return (
    <table className="ai-table">
      <thead>
        <tr>
          <th>STATUS</th>
          <th>TITLE</th>
          <th>REPOSITORY</th>
          <th>RESULT</th>
          <th>CREATED</th>
          <th>DURATION</th>
        </tr>
      </thead>
      <tbody>
        {(runs ?? []).map((r) => (
          <tr key={r.id}>
            <td>
              <StatusBadge status={r.status} />
            </td>
            <td>
              <Link to={`/assist/code/sessions/${r.id}`}>{r.title}</Link>
            </td>
            <td className="muted">{(r.subject as { repo?: string }).repo}</td>
            <td>
              <VerdictBadge verdict={r.verdict} />
            </td>
            <td className="muted">{timeAgo(r.createdAt)}</td>
            <td className="muted">{duration(r)}</td>
          </tr>
        ))}
        {(runs ?? []).length === 0 && (
          <tr>
            <td colSpan={6} className="muted ai-empty-cell">
              No sessions yet.
            </td>
          </tr>
        )}
      </tbody>
    </table>
  );
}

export function CodeSessionPage() {
  const { id } = useParams();
  return (
    <div className="ai-product">
      <ProductHeader icon={<Code2 size={20} />} name="Assist Code" tabs={TABS} />
      <RunView id={id!} back="/assist/code" backLabel="Sessions" />
    </div>
  );
}

// ---------------------------------------------------------------- automations

const TEMPLATES = [
  {
    icon: BarChart3,
    name: 'Fix the slowest endpoint',
    instructions: 'Find the slowest endpoint of the services in this repository over the last day (p95 from APM), find its code, and propose a change that makes it faster.',
  },
  {
    icon: Wrench,
    name: 'Fix frequent errors for a repo',
    instructions: 'Scan the last 24 hours of error logs and error issues of the services in this repository, find the most frequent error, and propose a fix with a test.',
  },
  { icon: GitPullRequest, name: 'Clean up dead feature flags', instructions: 'Find feature flag checks in the code that always take the same branch, and propose removing them.' },
];

export function CodeAutomations() {
  const agents = useAgents();
  const repos = useRepos();
  const save = useSaveAgent();
  const [form, setForm] = useState<AgentInput | null>(null);
  const [text, setText] = useState('');
  const list = (agents.data ?? []).filter((a) => a.kind === 'code' && (!text || `${a.name} ${a.repo}`.toLowerCase().includes(text.toLowerCase())));
  const startFrom = (name = '', instructions = '') =>
    setForm({
      name,
      instructions,
      description: '',
      skills: ['code', 'logs', 'errors', 'apm'],
      allowActions: false,
      tags: [],
      schedule: '0 6 * * 1-5',
      timezone: 'UTC',
      enabled: true,
      kind: 'code',
      repo: repos.data?.[0]?.repo ?? null,
    });
  return (
    <div className="ai-page-pad">
      <div className="ai-row ai-spread">
        <h1 className="ai-page-title">Automate with Assist</h1>
        <button className="btn btn-primary" onClick={() => startFrom()}>
          <Plus size={15} /> New Automation
        </button>
      </div>
      <section className="ai-card ai-blueprints">
        <div>
          <h2>Automation templates</h2>
          <p className="muted">Sessions that start on a schedule and propose changes for review.</p>
        </div>
        {TEMPLATES.map((t) => (
          <button key={t.name} className="ai-blueprint" onClick={() => startFrom(t.name, t.instructions)}>
            <b>
              <t.icon size={15} /> {t.name}
            </b>
            <span className="muted">{t.instructions}</span>
          </button>
        ))}
      </section>
      {form && (
        <section className="ai-card ai-editor-inline">
          <div className="ai-row">
            <label className="form-label">
              Name
              <input className="settings-input" value={form.name} onChange={(e) => setForm({ ...form, name: e.target.value })} />
            </label>
            <label className="form-label">
              Repository
              <select className="settings-input" value={form.repo ?? ''} onChange={(e) => setForm({ ...form, repo: e.target.value })}>
                {(repos.data ?? []).map((r) => (
                  <option key={r.repo}>{r.repo}</option>
                ))}
              </select>
            </label>
            <label className="form-label">
              Schedule (cron)
              <input className="settings-input ai-mono" value={form.schedule ?? ''} onChange={(e) => setForm({ ...form, schedule: e.target.value || null })} />
            </label>
          </div>
          <label className="form-label">
            What each session does
            <textarea className="ai-context" rows={4} value={form.instructions} onChange={(e) => setForm({ ...form, instructions: e.target.value })} />
          </label>
          <div className="ai-row">
            <button className="btn btn-primary btn-sm" disabled={save.isPending} onClick={() => save.mutate({ input: form }, { onSuccess: () => setForm(null) })}>
              Save automation
            </button>
            <button className="btn btn-sm" onClick={() => setForm(null)}>
              Cancel
            </button>
            {save.error && <span className="ai-bad">{(save.error as Error).message}</span>}
          </div>
        </section>
      )}
      <div className="ai-filter ai-filter-wide">
        <Search size={15} />
        <input placeholder="Search by name or repository" value={text} onChange={(e) => setText(e.target.value)} />
      </div>
      {agents.isPending ? (
        <Loading />
      ) : list.length ? (
        <AgentTable agents={list} />
      ) : (
        <div className="ai-empty">
          <Wrench size={40} />
          <h3>No automations yet</h3>
          <p className="muted">Create one, or start from a template above.</p>
        </div>
      )}
    </div>
  );
}

// ---------------------------------------------------------------- usage

function Usage() {
  const { range } = useTime();
  const r = useMemo(() => ({ from: Math.floor(range.from / 60_000) * 60_000, to: Math.ceil(range.to / 60_000) * 60_000 }), [range.from, range.to]);
  const q = useCodeUsage(r);
  const runs = useRuns({ kind: 'code', limit: 200, monitorId: null, status: null });
  const colors = palette();
  const u = q.data;
  return (
    <div className="ai-page-pad">
      <div className="ai-row ai-spread">
        <h1 className="ai-page-title">Usage</h1>
        <TimePicker compact />
      </div>
      {q.isError ? (
        <ErrorState error={q.error} onRetry={() => q.refetch()} />
      ) : (
        <div className="ai-report-grid">
          <section className="ai-card ai-kpi">
            <h3>Pull Requests Opened</h3>
            <b>{u?.pullRequestsOpened ?? '–'}</b>
          </section>
          <section className="ai-card ai-kpi">
            <h3>Pull Requests Merged</h3>
            <b>{u?.pullRequestsMerged ?? '–'}</b>
          </section>
          <section className="ai-card ai-kpi">
            <h3>Sessions</h3>
            <b>{u?.sessions ?? '–'}</b>
          </section>
          <section className="ai-card ai-kpi">
            <h3>Merge Rate</h3>
            <b>{u && u.pullRequestsOpened ? `${Math.round((u.pullRequestsMerged / u.pullRequestsOpened) * 100)}%` : '–'}</b>
          </section>
          <section className="ai-card ai-panel-wide">
            <h3>Sessions</h3>
            {u?.sessionsPerDay.length ? <Chart option={bars(u.sessionsPerDay, 'Sessions', colors[0])} height={200} /> : <p className="muted ai-nodata">(No data)</p>}
          </section>
          <section className="ai-card ai-panel-wide">
            <h3>Pull Requests</h3>
            {u?.pullRequestsPerDay.length ? <Chart option={bars(u.pullRequestsPerDay, 'Pull requests', colors[1])} height={200} /> : <p className="muted ai-nodata">(No data)</p>}
          </section>
        </div>
      )}
      <SessionTable runs={(runs.data ?? []).filter((x) => x.createdAt >= r.from && x.createdAt < r.to)} />
    </div>
  );
}

export function CodeUsagePage() {
  return (
    <TimeProvider initial="1w">
      <Usage />
    </TimeProvider>
  );
}

// ---------------------------------------------------------------- settings

export function CodeSettingsLayout() {
  return (
    <div className="ai-settings-frame">
      <nav className="ai-side-nav">
        <NavLink to="/assist/code/settings" end className={({ isActive }) => `ai-side-link${isActive ? ' active' : ''}`}>
          Admin
        </NavLink>
        <NavLink to="/assist/code/settings/repositories" className={({ isActive }) => `ai-side-link${isActive ? ' active' : ''}`}>
          Repositories
        </NavLink>
      </nav>
      <div className="ai-settings-main">
        <Outlet />
      </div>
    </div>
  );
}

export function CodeAdminSettings() {
  const q = useCodeSettings();
  const save = useSaveCodeSettings();
  const [form, setForm] = useState<CodeSettings | null>(null);
  useEffect(() => {
    if (q.data) setForm(q.data);
  }, [q.data]);
  if (!form) return q.isError ? <ErrorState error={q.error} onRetry={() => q.refetch()} /> : <Loading />;
  return (
    <section>
      <h2>
        <GitPullRequest size={18} /> Pull requests
      </h2>
      <p className="muted">Assist never pushes to an existing branch and never merges: each change goes on a new branch and is reviewed in a pull request.</p>
      <div className="ai-row ai-setting">
        <span>
          <b>Default pull request state</b>
          <span className="muted">Draft or ready for review.</span>
        </span>
        <div className="ai-row ai-segment">
          <button className={form.draft ? 'active' : ''} onClick={() => setForm({ ...form, draft: true })}>
            Draft
          </button>
          <button className={!form.draft ? 'active' : ''} onClick={() => setForm({ ...form, draft: false })}>
            Open
          </button>
        </div>
      </div>
      <div className="ai-row ai-setting">
        <span>
          <b>Create a pull request when done</b>
          <span className="muted">Open it as soon as a session proposes a change, instead of on the reviewer’s click.</span>
        </span>
        <button
          type="button"
          role="switch"
          aria-checked={form.autoPullRequest}
          className={`switch${form.autoPullRequest ? ' on' : ''}`}
          onClick={() => setForm({ ...form, autoPullRequest: !form.autoPullRequest })}
        />
      </div>
      <div className="ai-row ai-setting">
        <span>
          <b>Branch prefix</b>
          <span className="muted">Branches are named &lt;prefix&gt;&lt;title&gt;-&lt;session&gt;.</span>
        </span>
        <input className="settings-input ai-mono" value={form.branchPrefix} onChange={(e) => setForm({ ...form, branchPrefix: e.target.value })} />
      </div>
      <div className="ai-row">
        <button className="btn btn-primary btn-sm" disabled={save.isPending || JSON.stringify(form) === JSON.stringify(q.data)} onClick={() => save.mutate(form)}>
          Save
        </button>
        {save.isSuccess && <span className="ai-ok">Saved</span>}
        {save.error && <span className="ai-bad">{(save.error as Error).message}</span>}
      </div>
    </section>
  );
}

export function CodeRepositories() {
  const repos = useRepos();
  const conns = useCodeConnections();
  const add = useAddRepo();
  const remove = useRemoveRepo();
  const [repo, setRepo] = useState('');
  const [provider, setProvider] = useState('github');
  const [services, setServices] = useState('');
  return (
    <section>
      <h2>Repositories</h2>
      <p className="muted">The repositories Assist may read and propose changes to, through your organization’s connections (Developer Portal → Settings).</p>
      <div className="ai-conn-grid">
        {(['github', 'gitlab'] as const).map((p) => (
          <div key={p} className="ai-card ai-conn">
            <b>{p === 'github' ? 'GitHub' : 'GitLab'}</b>
            {conns.data?.[p] ? <span className="ai-ok">Connected as {conns.data[p]}</span> : <Link to="/portal/settings">Connect</Link>}
          </div>
        ))}
      </div>
      <div className="ai-card ai-row ai-wrap">
        <select className="settings-input" value={provider} onChange={(e) => setProvider(e.target.value)} aria-label="Provider">
          <option value="github">GitHub</option>
          <option value="gitlab">GitLab</option>
        </select>
        <input className="settings-input" placeholder="example-org/storefront" value={repo} onChange={(e) => setRepo(e.target.value)} aria-label="Repository" />
        <input className="settings-input" placeholder="Services (storefront-api, order-service)" value={services} onChange={(e) => setServices(e.target.value)} aria-label="Services" />
        <button
          className="btn btn-primary btn-sm"
          disabled={!repo.trim() || add.isPending}
          onClick={() =>
            add.mutate(
              {
                repo,
                provider,
                services: services
                  .split(',')
                  .map((s) => s.trim())
                  .filter(Boolean),
              },
              { onSuccess: () => (setRepo(''), setServices('')) },
            )
          }
        >
          {add.isPending ? 'Checking…' : 'Add repository'}
        </button>
        {add.error && <span className="ai-bad">{(add.error as Error).message}</span>}
      </div>
      <table className="ai-table">
        <thead>
          <tr>
            <th>REPOSITORY</th>
            <th>PROVIDER</th>
            <th>SERVICES</th>
            <th>ADDED</th>
            <th />
          </tr>
        </thead>
        <tbody>
          {(repos.data ?? []).map((r) => (
            <tr key={r.repo}>
              <td>
                <code>{r.repo}</code>
              </td>
              <td>{r.provider === 'gitlab' ? 'GitLab' : 'GitHub'}</td>
              <td className="muted">{r.services.join(', ') || '—'}</td>
              <td className="muted">
                {r.addedBy}, {timeAgo(r.addedAt)}
              </td>
              <td>
                <button className="icon-plain" title="Remove" onClick={() => remove.mutate(r.repo)}>
                  <Trash2 size={14} />
                </button>
              </td>
            </tr>
          ))}
          {repos.data?.length === 0 && (
            <tr>
              <td colSpan={5} className="muted ai-empty-cell">
                No repositories yet.
              </td>
            </tr>
          )}
        </tbody>
      </table>
    </section>
  );
}
