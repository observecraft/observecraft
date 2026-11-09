import { useEffect, useState } from 'react';
import { Link, useNavigate, useParams } from 'react-router-dom';
import { Pencil, Plus, Trash2, X } from 'lucide-react';
import type { AgentProfileVariable, AiTestingProfile } from '../../../api/aiTesting/client';
import { ErrorState, Loading } from '../../../components/ui/AsyncState';
import { timeAgo } from '../../../lib/format';
import { useRun, useRuns } from '../../assist/runsQueries';
import { RunView, StatusBadge, duration } from '../../assist/RunView';
import { useSyntheticVariables } from '../queries';
import { ProposalSuites } from './ProposalSuites';
import { useDeleteProfile, useProfiles, useProposals, useSaveProfile } from './queries';

const USAGES = ['username', 'email', 'password', 'otp', 'other'];

/** A job: the agent's timeline and conclusion, then the suites it proposed. */
export function AiTestingJobPage() {
  const id = useParams().id!;
  const run = useRun(id);
  const busy = run.data?.status === 'queued' || run.data?.status === 'running';
  const proposals = useProposals({ runId: id }, busy);
  // Proposals are stored as the job ends: read them once more when it does.
  const { refetch } = proposals;
  useEffect(() => {
    if (run.data && !busy) void refetch();
  }, [busy, run.data, refetch]);
  return (
    <div className="ait-page">
      <RunView id={id} back="/synthetics/ai-testing/jobs" backLabel="All jobs" />
      <section className="ait-proposed">
        <h2>Proposed suites</h2>
        {proposals.isPending ? (
          <Loading label="Loading proposals…" />
        ) : proposals.isError ? (
          <ErrorState error={proposals.error} onRetry={() => proposals.refetch()} />
        ) : proposals.data.length === 0 ? (
          <p className="muted">{busy ? 'The proposed tests appear here once the job finishes.' : 'This job proposed no tests.'}</p>
        ) : (
          <ProposalSuites proposals={proposals.data} />
        )}
      </section>
    </div>
  );
}

/** Every pending proposal, by job and suite. */
export function AiTestingReviewPage() {
  const q = useProposals({ status: 'pending' });
  return (
    <div className="ait-page">
      <h1 className="ait-title">Suites to review</h1>
      <p className="muted">Tests AI Testing proposed. Start testing a suite to create its browser tests (every 15 minutes) and a test suite of them, or ignore it.</p>
      {q.isPending ? (
        <Loading label="Loading…" />
      ) : q.isError ? (
        <ErrorState error={q.error} onRetry={() => q.refetch()} />
      ) : q.data.length === 0 ? (
        <p className="muted ait-empty">
          Nothing to review. <Link to="/synthetics/ai-testing">Start a job</Link> to get tests proposed.
        </p>
      ) : (
        <ProposalSuites proposals={q.data} showJob />
      )}
    </div>
  );
}

/** Every job: status, what it was asked, where it started and what it proposed. */
export function AiTestingJobsPage() {
  const navigate = useNavigate();
  const jobs = useRuns({ kind: 'testing', limit: 200 });
  const proposals = useProposals({});
  const counts = (run: string) => {
    const mine = (proposals.data ?? []).filter((p) => p.runId === run);
    return { all: mine.length, pending: mine.filter((p) => p.status === 'pending').length, accepted: mine.filter((p) => p.status === 'accepted').length };
  };
  return (
    <div className="ait-page">
      <h1 className="ait-title">All jobs</h1>
      {jobs.isPending ? (
        <Loading label="Loading jobs…" />
      ) : jobs.isError ? (
        <ErrorState error={jobs.error} onRetry={() => jobs.refetch()} />
      ) : jobs.data.length === 0 ? (
        <p className="muted ait-empty">
          No jobs yet. <Link to="/synthetics/ai-testing">Start one</Link>.
        </p>
      ) : (
        <table className="mon-table syn-table ait-table">
          <thead>
            <tr>
              <th>Status</th>
              <th>Job</th>
              <th>Starting URL</th>
              <th>Proposed</th>
              <th>Started</th>
              <th>Duration</th>
            </tr>
          </thead>
          <tbody>
            {jobs.data.map((j) => {
              const c = counts(j.id);
              const subject = j.subject as { startUrl?: string };
              return (
                <tr key={j.id} className="clickable" onClick={() => navigate(`/synthetics/ai-testing/jobs/${j.id}`)}>
                  <td>
                    <StatusBadge status={j.status} />
                  </td>
                  <td>
                    <b>{j.title}</b>
                    <div className="muted">by {j.createdBy}</div>
                  </td>
                  <td className="mono syn-wrap">{subject.startUrl}</td>
                  <td>{c.all ? `${c.all} tests (${c.pending} to review, ${c.accepted} testing)` : '—'}</td>
                  <td>{timeAgo(j.createdAt)}</td>
                  <td>{duration(j)}</td>
                </tr>
              );
            })}
          </tbody>
        </table>
      )}
    </div>
  );
}

/** Agent profiles: credentials a job may type, as references to global variables. */
export function AiTestingProfilesPage() {
  const q = useProfiles();
  const [editing, setEditing] = useState<AiTestingProfile | 'new' | null>(null);
  const remove = useDeleteProfile();
  return (
    <div className="ait-page">
      <div className="ait-row ait-spread">
        <h1 className="ait-title">Agent profiles</h1>
        <button className="btn btn-primary" onClick={() => setEditing('new')}>
          <Plus size={14} /> New profile
        </button>
      </div>
      <p className="muted">
        A profile lets a job sign in or fill in saved details. It names <Link to="/synthetics/settings/variables">global variables</Link>; the agent only sees placeholders such as{' '}
        <code>{'{{ SHOP_PASSWORD }}'}</code>, and the values are typed into the page by the browser.
      </p>
      {editing && <ProfileForm profile={editing === 'new' ? undefined : editing} onDone={() => setEditing(null)} />}
      {q.isPending ? (
        <Loading label="Loading profiles…" />
      ) : q.isError ? (
        <ErrorState error={q.error} onRetry={() => q.refetch()} />
      ) : q.data.length === 0 ? (
        <p className="muted ait-empty">No profiles yet: jobs browse as anonymous visitors.</p>
      ) : (
        <table className="mon-table syn-table ait-table">
          <thead>
            <tr>
              <th>Name</th>
              <th>Variables</th>
              <th>Updated</th>
              <th />
            </tr>
          </thead>
          <tbody>
            {q.data.map((p) => (
              <tr key={p.id}>
                <td>
                  <b>{p.name}</b>
                </td>
                <td>
                  {p.variables.map((v) => (
                    <span key={v.variable} className="syn-chip">
                      {v.usage}: <span className="mono">{v.variable}</span>
                    </span>
                  ))}
                </td>
                <td>{timeAgo(p.updatedAt)}</td>
                <td>
                  <span className="ait-row">
                    <button className="icon-plain" title="Edit" onClick={() => setEditing(p)}>
                      <Pencil size={14} />
                    </button>
                    <button className="icon-plain" title="Delete" disabled={remove.isPending} onClick={() => window.confirm(`Delete the profile “${p.name}”?`) && remove.mutate(p.id)}>
                      <Trash2 size={14} />
                    </button>
                  </span>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
    </div>
  );
}

function ProfileForm({ profile, onDone }: { profile?: AiTestingProfile; onDone: () => void }) {
  const [name, setName] = useState(profile?.name ?? '');
  const [rows, setRows] = useState<AgentProfileVariable[]>(profile?.variables ?? [{ usage: 'email', variable: '' }]);
  const variables = useSyntheticVariables();
  const save = useSaveProfile(profile?.id);
  const set = (i: number, patch: Partial<AgentProfileVariable>) => setRows(rows.map((r, j) => (j === i ? { ...r, ...patch } : r)));
  return (
    <section className="ait-card">
      <div className="ait-row ait-spread">
        <h3>{profile ? `Edit ${profile.name}` : 'New profile'}</h3>
        <button className="icon-plain" title="Close" onClick={onDone}>
          <X size={16} />
        </button>
      </div>
      <label className="form-label">
        Name
        <input className="syn-input" value={name} onChange={(e) => setName(e.target.value)} placeholder="QA shopper" />
      </label>
      {rows.map((r, i) => (
        <div key={i} className="ait-row">
          <select className="syn-select" value={r.usage} onChange={(e) => set(i, { usage: e.target.value })} aria-label="Usage">
            {USAGES.map((u) => (
              <option key={u} value={u}>
                {u}
              </option>
            ))}
          </select>
          <select className="syn-select" value={r.variable} onChange={(e) => set(i, { variable: e.target.value })} aria-label="Global variable">
            <option value="">Choose a global variable…</option>
            {(variables.data ?? []).map((v) => (
              <option key={v.id} value={v.name}>
                {v.name}
                {v.secure ? ' (secure)' : ''}
              </option>
            ))}
          </select>
          {rows.length > 1 && (
            <button className="icon-plain" title="Remove" onClick={() => setRows(rows.filter((_, j) => j !== i))}>
              <X size={14} />
            </button>
          )}
        </div>
      ))}
      <button className="link-btn" disabled={rows.length >= 10} onClick={() => setRows([...rows, { usage: 'password', variable: '' }])}>
        + Add a variable
      </button>
      {save.isError && <p className="form-error">{(save.error as Error).message}</p>}
      <div className="ait-row">
        <button className="btn btn-primary" disabled={!name.trim() || rows.some((r) => !r.variable) || save.isPending} onClick={() => save.mutate({ name, variables: rows }, { onSuccess: onDone })}>
          Save profile
        </button>
        <button className="btn" onClick={onDone}>
          Cancel
        </button>
      </div>
    </section>
  );
}
