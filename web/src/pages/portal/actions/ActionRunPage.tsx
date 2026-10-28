import { useEffect, useState } from 'react';
import { Link, useParams } from 'react-router-dom';
import { ArrowLeft, ExternalLink, Pencil, Play } from 'lucide-react';
import { blueprint as findBlueprint } from '../../../api/portalActions/blueprints';
import { CATEGORY_LABEL, formatDuration, STEP_LABEL } from '../../../api/portalActions/logic';
import type { ActionValues, PortalActionRun } from '../../../api/portalActions/client';
import { ActionForm, actionLogo, initialValues, RunStatus, ToolLogo } from './common';
import { usePortalAction, usePortalActionRuns, useRunPortalAction } from './queries';
import './actions.css';

/** Links to pages of this app stay in the tab; others open a new one. */
function RunLink({ link }: { link: string }) {
  if (!link) return <span className="psa-muted">—</span>;
  if (link.startsWith('/'))
    return (
      <Link className="sc-link" to={link}>
        {link}
      </Link>
    );
  return (
    <a className="sc-link" href={link} target="_blank" rel="noreferrer noopener">
      {link.replace(/^https?:\/\//, '')} <ExternalLink size={12} />
    </a>
  );
}

/** One action: its form, the Run button, the latest result and the run history. */
export function ActionRunPage() {
  const { id = '' } = useParams();
  const action = usePortalAction(id);
  const runs = usePortalActionRuns(id);
  const run = useRunPortalAction(id);
  const [values, setValues] = useState<ActionValues>({});
  const [latest, setLatest] = useState<PortalActionRun | null>(null);

  useEffect(() => {
    if (action.data) setValues(initialValues(action.data.inputs));
  }, [action.data?.id]);

  if (action.isLoading) return <div className="psa-page psa-muted">Loading…</div>;
  if (action.isError || !action.data) return <div className="psa-page form-error">{(action.error as Error)?.message ?? 'Action not found'}</div>;
  const a = action.data;
  const logo = actionLogo(findBlueprint(a.blueprint)?.logo, a.step.kind, a.step.requests?.[0]?.url);
  const shown = latest ?? runs.data?.[0] ?? null;
  const why = a.canRun ? '' : a.allowedTeams.length ? `Only members of ${a.allowedTeams.map((t) => `@${t}`).join(', ')} can run this action` : 'Your role is read-only';

  const onRun = () =>
    run.mutate(values, {
      onSuccess: (r) => {
        setLatest(r);
        runs.refetch();
      },
    });

  return (
    <div className="psa-page">
      <Link className="psa-back" to="/portal/actions">
        <ArrowLeft size={14} /> Self-Service Actions
      </Link>
      <header className="psa-run-head">
        <span className="psa-run-logo">
          <ToolLogo logo={logo} size={28} />
        </span>
        <div className="psa-grow">
          <h1 className="psa-title">{a.name}</h1>
          {a.description && <p className="psa-muted psa-run-desc">{a.description}</p>}
          <div className="psa-meta">
            <span className="psa-tag">{CATEGORY_LABEL[a.category]}</span>
            <span>{STEP_LABEL[a.step.kind]}</span>
            {a.ownerTeam && <span>Owned by @{a.ownerTeam}</span>}
            {a.allowedTeams.length > 0 && <span>Runs: {a.allowedTeams.map((t) => `@${t}`).join(', ')}</span>}
            <span>Created by {a.createdBy || 'unknown'}</span>
          </div>
        </div>
        <Link className="btn" to={`/portal/actions/${a.id}/edit`}>
          <Pencil size={14} /> Edit
        </Link>
      </header>

      <div className="psa-run-body">
        <section className="psa-panel psa-run-form">
          <h2>Run</h2>
          <ActionForm inputs={a.inputs} values={values} onChange={setValues} />
          {run.isError && <div className="form-error">{(run.error as Error).message}</div>}
          <div className="psa-run-actions">
            <button className="btn btn-primary" disabled={!a.canRun || run.isPending} title={why} onClick={onRun}>
              <Play size={14} /> {run.isPending ? 'Running…' : 'Run'}
            </button>
            {why && <span className="psa-muted">{why}</span>}
          </div>
        </section>

        <section className="psa-panel psa-run-result">
          <h2>{latest ? 'Result' : 'Latest run'}</h2>
          {!shown ? (
            <p className="psa-muted">No runs yet. Fill in the form and run the action.</p>
          ) : (
            <>
              <div className="psa-result-line">
                <RunStatus status={shown.status} />
                {shown.statusCode != null && <span className="psa-code">HTTP {shown.statusCode}</span>}
                <span className="psa-muted">{formatDuration(shown.durationMs)}</span>
                <span className="psa-muted">
                  {shown.userName} · {new Date(shown.createdAt).toLocaleString()}
                </span>
              </div>
              {shown.error && <div className="form-error">{shown.error}</div>}
              {shown.link && (
                <div className="psa-result-link">
                  Created: <RunLink link={shown.link} />
                </div>
              )}
              {shown.responseExcerpt && <pre className="psa-pre">{pretty(shown.responseExcerpt)}</pre>}
            </>
          )}
        </section>
      </div>

      <section className="psa-panel">
        <h2>Run history</h2>
        {runs.data && runs.data.length > 0 ? (
          <table className="psa-table">
            <thead>
              <tr>
                <th>When</th>
                <th>Who</th>
                <th>Status</th>
                <th>Code</th>
                <th>Duration</th>
                <th>Result</th>
              </tr>
            </thead>
            <tbody>
              {runs.data.map((r) => (
                <tr key={r.id} className={shown?.id === r.id ? 'selected' : ''} onClick={() => setLatest(r)}>
                  <td>{new Date(r.createdAt).toLocaleString()}</td>
                  <td>{r.userName}</td>
                  <td>
                    <RunStatus status={r.status} />
                  </td>
                  <td>{r.statusCode ?? '—'}</td>
                  <td>{formatDuration(r.durationMs)}</td>
                  <td>{r.link ? <RunLink link={r.link} /> : <span className="psa-muted psa-clip">{r.error ?? '—'}</span>}</td>
                </tr>
              ))}
            </tbody>
          </table>
        ) : (
          <p className="psa-muted">{runs.isLoading ? 'Loading…' : 'Nobody has run this action yet.'}</p>
        )}
      </section>
    </div>
  );
}

function pretty(s: string): string {
  try {
    return JSON.stringify(JSON.parse(s), null, 2);
  } catch {
    return s;
  }
}
