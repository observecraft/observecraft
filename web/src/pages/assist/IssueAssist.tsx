// Assist on an Error Tracking issue: investigate it (Review tab) and propose a fix in its repository
// (Resolve tab). Both start runs and show their progress in place, with a link to the full run.
import { useEffect, useState } from 'react';
import { Link, useNavigate } from 'react-router-dom';
import { ArrowRight, GitPullRequest, Sparkles } from 'lucide-react';
import type { IssueDetail } from '../../api/errorTracking/types';
import { StatusBadge, VerdictBadge } from './RunView';
import { useRepos, useRun, useRuns, useStartCodeSession, useStartRun } from './runsQueries';
import './assist.css';

/** The latest run's progress and outcome, in a line or two. */
function RunLine({ id, to }: { id: string; to: string }) {
  const run = useRun(id);
  const r = run.data;
  if (!r) return null;
  const summary = r.conclusion?.rootCause ?? r.change?.title ?? r.conclusion?.summary.split('\n')[0];
  return (
    <div className="investigate-note ai-issue-run">
      <span className="ai-row">
        <StatusBadge status={r.status} />
        <VerdictBadge verdict={r.verdict} />
        {r.status === 'running' && <span className="muted">{r.steps.filter((s) => s.type === 'tool').length} steps so far…</span>}
        {r.error && <span className="ai-bad">{r.error}</span>}
        <Link className="ai-push ai-row" to={to}>
          Open <ArrowRight size={13} />
        </Link>
      </span>
      {summary && <span className="ai-issue-summary">{summary}</span>}
    </div>
  );
}

export function IssueInvestigation({ issue }: { issue: IssueDetail }) {
  const runs = useRuns({ kind: 'investigation', issueId: issue.id, limit: 1, monitorId: null, status: null });
  const start = useStartRun();
  const latest = runs.data?.[0];
  const busy = latest && (latest.status === 'queued' || latest.status === 'running');
  return (
    <>
      <div className="investigate-banner">
        <Sparkles size={16} className="brand-icon" />
        <strong>Assist Investigation</strong>
        <span className="muted-text">Investigate this issue, find the root cause, and generate a fix.</span>
        <button
          className="btn btn-brand"
          disabled={start.isPending || busy}
          onClick={() => start.mutate({ kind: 'investigation', issueId: issue.id, monitorId: null, question: null, signalId: null })}
        >
          <Sparkles size={13} /> {start.isPending ? 'Starting…' : latest ? 'Investigate again' : 'Investigate this issue'}
        </button>
      </div>
      {start.error && <div className="investigate-note ai-bad">{(start.error as Error).message}</div>}
      {latest && <RunLine id={latest.id} to={`/assist/investigations/${latest.id}`} />}
    </>
  );
}

/** The repository of the issue's service, if one is added to Assist Code. */
function useIssueRepo(service: string) {
  const repos = useRepos();
  const mapped = repos.data?.find((r) => r.services.includes(service));
  return { repos, mapped: mapped ?? repos.data?.[0] };
}

export function IssueFix({ issue }: { issue: IssueDetail }) {
  const navigate = useNavigate();
  const { repos, mapped } = useIssueRepo(issue.service);
  const [repo, setRepo] = useState('');
  const start = useStartCodeSession();
  const [session, setSession] = useState<string>();
  useEffect(() => {
    if (!repo && mapped) setRepo(mapped.repo);
  }, [mapped, repo]);
  if (repos.data && repos.data.length === 0)
    return (
      <>
        <p className="muted-text">Add the repository of {issue.service} to Assist Code to get a fix proposed as a pull request.</p>
        <button className="btn" onClick={() => navigate('/assist/code/settings/repositories')}>
          <GitPullRequest size={14} /> Connect repository
        </button>
      </>
    );
  const where = issue.location ? ` It is thrown in ${issue.location.function}${issue.location.file ? ` (${issue.location.file})` : ''}.` : '';
  const prompt = `Fix the Error Tracking issue ${issue.errorType} in ${issue.service}: "${issue.message.split('\n')[0]}".${where} It happened ${issue.count} times. Find its cause in the code and propose the smallest fix, with a test if the repository has tests.`;
  return (
    <>
      <p className="muted-text">Assist reads the code of {issue.service}, finds why this error happens and proposes a change you can open as a pull request.</p>
      <div className="ai-row ai-wrap">
        <select className="settings-input" value={repo} onChange={(e) => setRepo(e.target.value)} aria-label="Repository">
          {(repos.data ?? []).map((r) => (
            <option key={r.repo} value={r.repo}>
              {r.repo}
            </option>
          ))}
        </select>
        <button className="btn btn-brand" disabled={!repo || start.isPending} onClick={() => start.mutate({ repo, prompt }, { onSuccess: (r) => setSession(r.id) })}>
          <Sparkles size={13} /> {start.isPending ? 'Starting…' : 'Suggest a fix'}
        </button>
      </div>
      {start.error && <div className="investigate-note ai-bad">{(start.error as Error).message}</div>}
      {session && <RunLine id={session} to={`/assist/code/sessions/${session}`} />}
    </>
  );
}

/** An incident's Investigation section: the latest Assist investigation of it, or the button to start one. */
export function IncidentInvestigation({ incidentId }: { incidentId: number }) {
  const runs = useRuns({ kind: 'investigation', incidentId, limit: 1, monitorId: null, status: null });
  const start = useStartRun();
  const latest = runs.data?.[0];
  const busy = latest && (latest.status === 'queued' || latest.status === 'running');
  return (
    <>
      <div className="assist-card">
        <Sparkles size={18} /> <b>Assist Investigation</b>
        {!latest && !runs.isPending && <span className="muted">Assist hasn't investigated this incident yet</span>}
        <button className="btn" disabled={start.isPending || busy} onClick={() => start.mutate({ kind: 'investigation', incidentId, issueId: null, monitorId: null, question: null, signalId: null })}>
          <Sparkles size={14} /> {start.isPending ? 'Starting…' : latest ? 'Investigate again' : 'Investigate'}
        </button>
      </div>
      {start.error && <div className="investigate-note ai-bad">{(start.error as Error).message}</div>}
      {latest && <RunLine id={latest.id} to={`/assist/investigations/${latest.id}`} />}
    </>
  );
}
