// Assist Agent Builder (Automation → Agent Builder): custom agents with their own instructions and skills,
// run by hand or on a schedule; blueprints to start from, and the skills library.
import { useEffect, useState } from 'react';
import { Link, NavLink, useLocation, useNavigate, useParams } from 'react-router-dom';
import { ArrowLeft, ArrowRight, ArrowUp, Bot, BugPlay, LayoutDashboard, Lock, Play, Plus, ScrollText, Search, Siren, Trash2 } from 'lucide-react';
import type { AgentInput, AgentView } from '../../api/assist/client';
import { ErrorState, Loading } from '../../components/ui/AsyncState';
import { timeAgo } from '../../lib/format';
import { useProfile } from '../settings/queries';
import { useAssistStatus } from './queries';
import { RunView, StatusBadge, VerdictBadge } from './RunView';
import { useAgent, useAgents, useDeleteAgent, useDraftAgent, useRunAgent, useRuns, useSaveAgent } from './runsQueries';
import './assist.css';

export interface Blueprint {
  icon: typeof Bot;
  name: string;
  description: string;
  instructions: string;
  skills: string[];
  schedule: string | null;
}

export const BLUEPRINTS: Blueprint[] = [
  {
    icon: Siren,
    name: 'Incident Responder',
    description: 'Resolve incidents faster, from detection to post-mortem.',
    instructions: 'Look at the active incidents. For each, gather the related monitors, error logs and traces of the affected services, say what is known about the cause, and propose the next steps.',
    skills: ['incidents', 'monitors', 'logs', 'apm'],
    schedule: null,
  },
  {
    icon: ScrollText,
    name: 'Log Analyzer',
    description: 'Find root causes in logs without manual digging.',
    instructions: 'Given a service (from the run input), find the most frequent error patterns in its logs over the last 4 hours, group them by cause, and say which started recently.',
    skills: ['logs', 'errors'],
    schedule: null,
  },
  {
    icon: Lock,
    name: 'Cybersecurity Responder',
    description: 'Identify threats and surface remediation steps.',
    instructions:
      'Review open security signals from the last day, starting with critical and high. For each, decide whether it is a real threat from the logs behind it, and list remediation steps for the real ones.',
    skills: ['security', 'logs'],
    schedule: '0 8 * * *',
  },
  {
    icon: BugPlay,
    name: 'Daily Error Triager',
    description: 'Finds error logs daily and reports non-transient issues.',
    instructions: 'List error issues first seen in the last day. For each, say whether it needs attention (customer impact, growing) or is transient, and which team owns the service.',
    skills: ['errors', 'logs', 'apm'],
    schedule: '0 9 * * 1-5',
  },
  {
    icon: LayoutDashboard,
    name: 'Dashboard Reporter',
    description: 'Watches key metrics for trends that need attention.',
    instructions: 'Compare request rate, error rate and p95 latency of every service over the last day with the week before, and report the services whose numbers changed the most, with links.',
    skills: ['apm', 'metrics'],
    schedule: '0 8 * * 1',
  },
];

function AgentTabs() {
  return (
    <nav className="au-subtabs">
      <NavLink to="/automation/agents" end className={({ isActive }) => `au-subtab${isActive ? ' active' : ''}`}>
        All Agents
      </NavLink>
      <NavLink to="/automation/agents/skills" className={({ isActive }) => `au-subtab${isActive ? ' active' : ''}`}>
        Skills Library
      </NavLink>
    </nav>
  );
}

const fromBlueprint = (b: Blueprint) => ({ name: b.name, description: b.description, instructions: b.instructions, skills: b.skills, schedule: b.schedule });

export function AgentsPage() {
  const navigate = useNavigate();
  const q = useAgents();
  const me = useProfile();
  const [text, setText] = useState('');
  const [mine, setMine] = useState(false);
  const list = (q.data ?? []).filter(
    (a) => a.kind === 'agent' && (!text.trim() || `${a.name} ${a.description}`.toLowerCase().includes(text.toLowerCase())) && (!mine || a.createdBy === me.data?.name),
  );
  return (
    <div className="ai-page-pad">
      <div className="ai-row ai-spread">
        <AgentTabs />
        <Link className="btn btn-primary" to="/automation/agents/new">
          <Plus size={15} /> New Agent
        </Link>
      </div>
      <section className="ai-card ai-blueprints">
        <div>
          <h2>Meet custom agents</h2>
          <p className="muted">Hand work off to AI agents, each with its own instructions and skills, run by hand or on a schedule.</p>
          <Link className="btn btn-sm" to="/automation/agents/new">
            Browse all blueprints <ArrowRight size={13} />
          </Link>
        </div>
        {BLUEPRINTS.slice(0, 4).map((b) => (
          <button key={b.name} className="ai-blueprint" onClick={() => navigate('/automation/agents/edit', { state: { draft: fromBlueprint(b) } })}>
            <b>
              <b.icon size={15} /> {b.name}
            </b>
            <span className="muted">{b.description}</span>
          </button>
        ))}
      </section>
      <div className="ai-filter ai-filter-wide">
        <Search size={15} />
        <input placeholder="Search agents by name or description…" value={text} onChange={(e) => setText(e.target.value)} />
      </div>
      <div className="ai-row">
        <span className="muted">
          Showing {list.length} of {(q.data ?? []).filter((a) => a.kind === 'agent').length} agents
        </span>
        <label className="ai-auto">
          <button type="button" role="switch" aria-checked={mine} className={`switch${mine ? ' on' : ''}`} onClick={() => setMine(!mine)} /> My agents
        </label>
      </div>
      {q.isPending ? (
        <Loading />
      ) : q.isError ? (
        <ErrorState error={q.error} onRetry={() => q.refetch()} />
      ) : list.length === 0 ? (
        <div className="ai-empty">
          <Bot size={44} />
          <h3>No agents found</h3>
          <Link className="btn btn-primary" to="/automation/agents/new">
            <Plus size={15} /> New Agent
          </Link>
        </div>
      ) : (
        <AgentTable agents={list} />
      )}
    </div>
  );
}

export function AgentTable({ agents, base = '/automation/agents' }: { agents: AgentView[]; base?: string }) {
  return (
    <table className="ai-table">
      <thead>
        <tr>
          <th>NAME</th>
          <th>TAGS</th>
          <th>SCHEDULE</th>
          <th>LAST MODIFIED</th>
          <th>LAST RUN</th>
          <th>CREATED BY</th>
        </tr>
      </thead>
      <tbody>
        {agents.map((a) => (
          <tr key={a.id}>
            <td>
              <Link to={`${base}/${a.id}`}>{a.name}</Link>
              <div className="muted ai-sub">{a.repo ?? a.description}</div>
            </td>
            <td>
              <div className="ai-tags">
                {a.tags.map((t) => (
                  <span key={t} className="ai-tag">
                    {t}
                  </span>
                ))}
              </div>
            </td>
            <td className="muted">{a.schedule ? <code>{a.schedule}</code> : 'By hand'}</td>
            <td className="muted">{timeAgo(a.updatedAt)}</td>
            <td>
              {a.lastRunStatus ? (
                <span className="ai-row">
                  {a.lastVerdict ? <VerdictBadge verdict={a.lastVerdict} /> : <StatusBadge status={a.lastRunStatus} />} <span className="muted">{timeAgo(a.lastRunAt!)}</span>
                </span>
              ) : (
                <span className="muted">Never</span>
              )}
            </td>
            <td className="muted">{a.createdBy}</td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}

export function SkillsLibraryPage() {
  const status = useAssistStatus();
  return (
    <div className="ai-page-pad">
      <AgentTabs />
      <p className="muted">Skills are groups of Assist’s tools. An agent uses the skills you give it; tools that change something run only if the agent may take actions.</p>
      <table className="ai-table">
        <thead>
          <tr>
            <th>SKILL</th>
            <th>DESCRIPTION</th>
            <th>TOOLS</th>
          </tr>
        </thead>
        <tbody>
          {(status.data?.skills ?? []).map((s) => (
            <tr key={s.name}>
              <td className="ai-skill-name">
                {s.name}
                {s.writes && <span className="ai-tool-tag">can act</span>}
              </td>
              <td>{s.description}</td>
              <td className="muted ai-skill-tools">{s.tools.join(', ')}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

/** "What should this agent do?": a description turned into a draft, or a blueprint. */
export function NewAgentPage() {
  const navigate = useNavigate();
  const draft = useDraftAgent();
  const [text, setText] = useState('');
  const go = () => draft.mutate(text, { onSuccess: (d) => navigate('/automation/agents/edit', { state: { draft: d } }) });
  return (
    <div className="ai-page-pad ai-narrow ai-new-agent">
      <Link className="link-muted ai-back" to="/automation/agents">
        <ArrowLeft size={14} /> All agents
      </Link>
      <h1>What should this agent do?</h1>
      <div className="ai-composer">
        <textarea
          rows={3}
          value={text}
          autoFocus
          placeholder="Describe what you want this agent to monitor, investigate, or automate…"
          onChange={(e) => setText(e.target.value)}
          onKeyDown={(e) => e.key === 'Enter' && !e.shiftKey && (e.preventDefault(), text.trim() && go())}
        />
        <div className="ai-composer-bar">
          <span className="muted">{draft.isPending ? 'Drafting the agent…' : draft.error ? (draft.error as Error).message : ''}</span>
          <button className="ai-send" title="Draft" disabled={!text.trim() || draft.isPending} onClick={go}>
            <ArrowUp size={16} />
          </button>
        </div>
      </div>
      <h3 className="muted">Start from a blueprint</h3>
      <div className="ai-blueprint-grid">
        {BLUEPRINTS.map((b) => (
          <button key={b.name} className="ai-blueprint" onClick={() => navigate('/automation/agents/edit', { state: { draft: fromBlueprint(b) } })}>
            <b>
              <b.icon size={15} /> {b.name}
            </b>
            <span className="muted">{b.description}</span>
          </button>
        ))}
      </div>
      <Link className="ai-scratch" to="/automation/agents/edit">
        Or start from scratch <ArrowRight size={15} />
      </Link>
    </div>
  );
}

const EMPTY: AgentInput = { name: '', description: '', instructions: '', skills: ['logs'], allowActions: false, tags: [], schedule: null, timezone: 'UTC', enabled: true, kind: 'agent', repo: null };

/** Creates (`/automation/agents/edit`, with an optional draft in the location state) or edits an agent. */
export function AgentEditorPage() {
  const { id } = useParams();
  const navigate = useNavigate();
  const location = useLocation();
  const status = useAssistStatus();
  const agent = useAgent(id);
  const save = useSaveAgent();
  const remove = useDeleteAgent();
  const run = useRunAgent();
  const runs = useRuns({ kind: agent.data?.kind === 'code' ? 'code' : 'agent', limit: 50, monitorId: null, status: null });
  const [form, setForm] = useState<AgentInput>(() => ({ ...EMPTY, ...((location.state as { draft?: Partial<AgentInput> } | null)?.draft ?? {}) }));
  const [input, setInput] = useState('');
  useEffect(() => {
    if (agent.data) {
      const a = agent.data;
      setForm({
        name: a.name,
        description: a.description,
        instructions: a.instructions,
        skills: a.skills,
        allowActions: a.allowActions,
        tags: a.tags,
        schedule: a.schedule,
        timezone: a.timezone,
        enabled: a.enabled,
        kind: a.kind,
        repo: a.repo,
      });
    }
  }, [agent.data]);
  if (id && agent.isPending) return <Loading />;
  if (id && agent.isError) return <ErrorState error={agent.error} onRetry={() => agent.refetch()} />;
  const toggle = (s: string) => setForm({ ...form, skills: form.skills.includes(s) ? form.skills.filter((x) => x !== s) : [...form.skills, s] });
  const mine = (runs.data ?? []).filter((r) => (r.subject as { agentId?: string }).agentId === id);
  return (
    <div className="ai-page-pad ai-editor">
      <Link className="link-muted ai-back" to="/automation/agents">
        <ArrowLeft size={14} /> All agents
      </Link>
      <div className="ai-row ai-spread">
        <h1 className="ai-page-title">{id ? form.name || 'Agent' : 'New Agent'}</h1>
        <div className="ai-row">
          {id && (
            <button className="btn" onClick={() => remove.mutate(id, { onSuccess: () => navigate('/automation/agents') })}>
              <Trash2 size={14} /> Delete
            </button>
          )}
          <button
            className="btn btn-primary"
            disabled={save.isPending}
            onClick={() => save.mutate({ id, input: form }, { onSuccess: (a) => navigate(`/automation/agents/${a.id}`, { replace: true }) })}
          >
            {save.isPending ? 'Saving…' : 'Save'}
          </button>
        </div>
      </div>
      {save.error && <p className="ai-bad">{(save.error as Error).message}</p>}
      <div className="ai-editor-grid">
        <section className="ai-card">
          <label className="form-label">
            Name
            <input className="settings-input" value={form.name} onChange={(e) => setForm({ ...form, name: e.target.value })} />
          </label>
          <label className="form-label">
            Description
            <input className="settings-input" value={form.description} onChange={(e) => setForm({ ...form, description: e.target.value })} />
          </label>
          <label className="form-label">
            Instructions
            <textarea
              className="ai-context"
              rows={10}
              value={form.instructions}
              onChange={(e) => setForm({ ...form, instructions: e.target.value })}
              placeholder="What the agent does on each run, step by step, and what it reports."
            />
          </label>
          <label className="form-label">
            Tags
            <input
              className="settings-input"
              value={(form.tags ?? []).join(', ')}
              placeholder="team:orders, env:prod"
              onChange={(e) =>
                setForm({
                  ...form,
                  tags: e.target.value
                    .split(',')
                    .map((t) => t.trim())
                    .filter(Boolean),
                })
              }
            />
          </label>
        </section>
        <section className="ai-card">
          <h3>Skills</h3>
          {(status.data?.skills ?? []).map((s) => (
            <label key={s.name} className="lm-toggle">
              <input type="checkbox" className="checkbox" checked={form.skills.includes(s.name)} onChange={() => toggle(s.name)} />
              <span>
                <b>{s.name}</b>
                <span className="muted">{s.description}</span>
              </span>
            </label>
          ))}
          <label className="lm-toggle">
            <input type="checkbox" className="checkbox" checked={!!form.allowActions} onChange={(e) => setForm({ ...form, allowActions: e.target.checked })} />
            <span>
              <b>May take actions without asking</b>
              <span className="muted">Create monitors or declare incidents when its instructions say so. Off: it only reads and reports.</span>
            </span>
          </label>
          <h3>Schedule</h3>
          <label className="form-label">
            Cron (minute hour day month weekday); empty: by hand only
            <input className="settings-input ai-mono" value={form.schedule ?? ''} placeholder="0 9 * * 1-5" onChange={(e) => setForm({ ...form, schedule: e.target.value || null })} />
          </label>
          <label className="form-label">
            Time zone
            <input className="settings-input" value={form.timezone ?? 'UTC'} onChange={(e) => setForm({ ...form, timezone: e.target.value })} />
          </label>
          {agent.data?.nextRunAt && <p className="muted">Next run {new Date(agent.data.nextRunAt).toLocaleString()}.</p>}
        </section>
      </div>
      {id && (
        <section className="ai-card">
          <div className="ai-row ai-spread">
            <h3>Runs</h3>
            <div className="ai-row">
              <input className="settings-input" placeholder="Input for this run (optional)" value={input} onChange={(e) => setInput(e.target.value)} />
              <button className="btn btn-primary btn-sm" disabled={run.isPending} onClick={() => run.mutate({ id, input }, { onSuccess: (r) => navigate(`/automation/agents/runs/${r.id}`) })}>
                <Play size={13} /> Run now
              </button>
            </div>
          </div>
          {run.error && <p className="ai-bad">{(run.error as Error).message}</p>}
          <table className="ai-table">
            <tbody>
              {mine.map((r) => (
                <tr key={r.id}>
                  <td>
                    <StatusBadge status={r.status} />
                  </td>
                  <td>
                    <Link to={`/automation/agents/runs/${r.id}`}>{r.trigger === 'schedule' ? 'Scheduled run' : `Run by ${r.createdBy}`}</Link>
                  </td>
                  <td>
                    <VerdictBadge verdict={r.verdict} />
                  </td>
                  <td className="muted">{timeAgo(r.createdAt)}</td>
                </tr>
              ))}
              {mine.length === 0 && (
                <tr>
                  <td className="muted ai-empty-cell">No runs yet.</td>
                </tr>
              )}
            </tbody>
          </table>
        </section>
      )}
    </div>
  );
}

export function AgentRunPage() {
  const { id } = useParams();
  return (
    <div className="ai-page-pad">
      <RunView id={id!} back="/automation/agents" backLabel="All agents" />
    </div>
  );
}
