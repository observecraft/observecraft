// Assist Investigation → Settings: the knowledge base every run reads, memories, notifications, rate
// limits and source code (shared by background runs: investigations, security verdicts and agents).
import { useEffect, useState } from 'react';
import { Link, NavLink, Outlet, useOutletContext } from 'react-router-dom';
import { ChevronRight, Sparkles, Trash2 } from 'lucide-react';
import type { RunSettings } from '../../api/assist/client';
import { ErrorState, Loading } from '../../components/ui/AsyncState';
import { timeAgo } from '../../lib/format';
import { useAddMemory, useDeleteMemory, useInvestigationMonitors, useMemories, useRepos, useRunSettings, useSaveRunSettings } from './runsQueries';
import './assist.css';

const NAV = [
  {
    section: 'KNOWLEDGE BASE',
    items: [
      { to: '/assist/investigations/settings', label: 'ASSIST.md', end: true },
      { to: '/assist/investigations/settings/memories', label: 'Investigation Memories' },
    ],
  },
  {
    section: 'SETTINGS',
    items: [
      { to: '/assist/investigations/settings/notifications', label: 'Notifications' },
      { to: '/assist/investigations/settings/rate-limits', label: 'Rate Limits' },
      { to: '/assist/investigations/settings/source-code', label: 'Source Code Integration' },
    ],
  },
];

type Ctx = { settings: RunSettings };

export function InvestigationSettingsLayout() {
  const q = useRunSettings();
  return (
    <div className="ai-product">
      <header className="product-header">
        <div className="product-title ai-crumbs">
          <Link to="/assist/investigations">
            <Sparkles size={18} /> Assist Investigation
          </Link>
          <ChevronRight size={16} /> Settings
        </div>
      </header>
      <div className="ai-settings-frame">
        <nav className="ai-side-nav">
          {NAV.map((g) => (
            <div key={g.section}>
              <div className="ai-side-section">{g.section}</div>
              {g.items.map((i) => (
                <NavLink key={i.to} to={i.to} end={i.end} className={({ isActive }) => `ai-side-link${isActive ? ' active' : ''}`}>
                  {i.label}
                </NavLink>
              ))}
            </div>
          ))}
        </nav>
        <div className="ai-settings-main">
          {q.isPending ? <Loading /> : q.isError ? <ErrorState error={q.error} onRetry={() => q.refetch()} /> : <Outlet context={{ settings: q.data } satisfies Ctx} />}
        </div>
      </div>
    </div>
  );
}

function useSettingsForm() {
  const { settings } = useOutletContext<Ctx>();
  const save = useSaveRunSettings();
  const [form, setForm] = useState(settings);
  useEffect(() => setForm(settings), [settings]);
  const footer = (
    <div className="ai-row">
      <button className="btn btn-primary btn-sm" disabled={save.isPending || JSON.stringify(form) === JSON.stringify(settings)} onClick={() => save.mutate(form)}>
        {save.isPending ? 'Saving…' : 'Save'}
      </button>
      {save.isSuccess && <span className="ai-ok">Saved</span>}
      {save.error && <span className="ai-bad">{(save.error as Error).message}</span>}
    </div>
  );
  return { form, setForm, footer };
}

export function KnowledgeBasePage() {
  const { form, setForm, footer } = useSettingsForm();
  return (
    <section>
      <h2>ASSIST.md</h2>
      <p className="muted">
        What every investigation, security verdict and agent should know about your systems: which team owns which service, known noisy alerts, runbooks, how deploys work. Markdown, up to 20,000
        characters. Only admins can change it.
      </p>
      <textarea
        className="ai-context ai-mono"
        rows={18}
        maxLength={20000}
        value={form.knowledge}
        placeholder={'# Services\n- order-service: owned by @team-orders, deploys from example-org/storefront\n\n# Known noise\n- Checkout latency alerts at 02:00 UTC are the nightly batch.'}
        onChange={(e) => setForm({ ...form, knowledge: e.target.value })}
      />
      {footer}
    </section>
  );
}

export function NotificationsPage() {
  const { form, setForm, footer } = useSettingsForm();
  return (
    <section>
      <h2>Notifications</h2>
      <p className="muted">
        Who is told when an investigation or a security verdict concludes: notification channels (<code>@slack-ops</code>, <code>@webhook-oncall</code>) set up under Monitors → Settings, or members’
        emails. One per line.
      </p>
      <textarea className="ai-context" rows={5} value={form.notify.join('\n')} placeholder="@slack-incidents" onChange={(e) => setForm({ ...form, notify: e.target.value.split('\n') })} />
      {footer}
    </section>
  );
}

export function RateLimitsPage() {
  const { form, setForm, footer } = useSettingsForm();
  return (
    <section>
      <h2>Rate Limits</h2>
      <p className="muted">Caps on how many runs this organization starts, so an alert storm doesn’t use up its AI credits. Runs over the limit are refused (by hand) or skipped (automatic).</p>
      <label className="form-label ai-num">
        Runs per hour (all kinds)
        <input className="settings-input" type="number" min={1} max={1000} value={form.runsPerHour} onChange={(e) => setForm({ ...form, runsPerHour: Number(e.target.value) })} />
      </label>
      <label className="form-label ai-num">
        Automatic runs per day (monitor alerts, new signals, schedules)
        <input className="settings-input" type="number" min={0} max={10000} value={form.automaticRunsPerDay} onChange={(e) => setForm({ ...form, automaticRunsPerDay: Number(e.target.value) })} />
      </label>
      {footer}
    </section>
  );
}

export function SourceCodePage() {
  const repos = useRepos();
  return (
    <section>
      <h2>Source Code Integration</h2>
      <p className="muted">Give Assist access to your code so investigations can trace issues back to it and Assist Code can propose fixes.</p>
      <div className="ai-card">
        <b>{repos.data?.length ?? 0} repositories</b> added to Assist Code{repos.data?.length ? `: ${repos.data.map((r) => r.repo).join(', ')}` : ''}.
        <div className="ai-row">
          <Link className="btn btn-sm" to="/assist/code/settings/repositories">
            Manage repositories
          </Link>
          <Link className="btn btn-sm" to="/portal/settings">
            GitHub and GitLab connections
          </Link>
        </div>
      </div>
    </section>
  );
}

export function MemoriesPage() {
  const q = useMemories();
  const monitors = useInvestigationMonitors();
  const add = useAddMemory();
  const remove = useDeleteMemory();
  const [text, setText] = useState('');
  const [monitor, setMonitor] = useState('');
  if (q.isPending) return <Loading />;
  if (q.isError) return <ErrorState error={q.error} onRetry={() => q.refetch()} />;
  return (
    <section>
      <h2>Investigation Memories</h2>
      <p className="muted">Facts Assist keeps in mind: for one monitor’s investigations, or for every run. Added here, or from an investigation’s “Not quite” feedback.</p>
      <div className="ai-card ai-memory-new">
        <textarea className="ai-context" rows={3} maxLength={2000} value={text} placeholder="Spikes at 02:00 UTC are the nightly batch, not an incident" onChange={(e) => setText(e.target.value)} />
        <div className="ai-row">
          <select className="settings-input" value={monitor} onChange={(e) => setMonitor(e.target.value)}>
            <option value="">Every run</option>
            {(monitors.data ?? []).map((m) => (
              <option key={m.id} value={m.id}>
                {m.name}
              </option>
            ))}
          </select>
          <button
            className="btn btn-primary btn-sm"
            disabled={!text.trim() || add.isPending}
            onClick={() => add.mutate({ text, monitorId: monitor ? Number(monitor) : null }, { onSuccess: () => setText('') })}
          >
            Add memory
          </button>
          {add.error && <span className="ai-bad">{(add.error as Error).message}</span>}
        </div>
      </div>
      <table className="ai-table">
        <thead>
          <tr>
            <th>MEMORY</th>
            <th>FOR</th>
            <th>ADDED</th>
            <th />
          </tr>
        </thead>
        <tbody>
          {q.data.map((m) => (
            <tr key={m.id}>
              <td>{m.text}</td>
              <td>{m.monitorName ?? (m.monitorId != null ? `Monitor #${m.monitorId}` : 'Every run')}</td>
              <td className="muted">
                {m.createdBy}, {timeAgo(m.createdAt)}
              </td>
              <td>
                <button className="icon-plain" title="Delete memory" onClick={() => remove.mutate(m.id)}>
                  <Trash2 size={14} />
                </button>
              </td>
            </tr>
          ))}
          {q.data.length === 0 && (
            <tr>
              <td colSpan={4} className="muted ai-empty-cell">
                No memories yet.
              </td>
            </tr>
          )}
        </tbody>
      </table>
    </section>
  );
}
