// One run of Assist (an investigation, a security verdict, an agent's run or a coding session): its
// status, conclusion, the steps it took, feedback and, for coding sessions, the change and its pull request.
import { useState, type ReactNode } from 'react';
import { Link, NavLink } from 'react-router-dom';
import ReactMarkdown from 'react-markdown';
import remarkGfm from 'remark-gfm';
import { ArrowLeft, CircleAlert, CircleCheck, ExternalLink, GitPullRequest, LoaderCircle, ThumbsDown, ThumbsUp, X } from 'lucide-react';
import type { CodeChange, RunDetail } from '../../api/assist/client';
import { ErrorState, Loading } from '../../components/ui/AsyncState';
import { lineDiff } from '../../lib/lineDiff';
import { timeAgo } from '../../lib/format';
import { RunTimeline } from './RunTimeline';
import { useCancelRun, useOpenPullRequest, useRun, useRunFeedback } from './runsQueries';

const VERDICT_TONE: Record<string, string> = {
  conclusive: 'ok',
  no_issue: 'ok',
  benign: 'ok',
  done: 'ok',
  change_proposed: 'ok',
  inconclusive: 'muted',
  no_change: 'muted',
  suspicious: 'warn',
  needs_attention: 'warn',
  malicious: 'bad',
  failed: 'bad',
};

export const verdictLabel = (v: string) => v.replace(/_/g, ' ').replace(/^./, (c) => c.toUpperCase());

export function VerdictBadge({ verdict }: { verdict?: string | null }) {
  if (!verdict) return null;
  return <span className={`ai-verdict ${VERDICT_TONE[verdict] ?? 'muted'}`}>{verdictLabel(verdict)}</span>;
}

export function StatusBadge({ status }: { status: string }) {
  const icon =
    status === 'running' || status === 'queued' ? (
      <LoaderCircle size={12} className="ai-spin" />
    ) : status === 'failed' ? (
      <CircleAlert size={12} />
    ) : status === 'concluded' ? (
      <CircleCheck size={12} />
    ) : null;
  return (
    <span className={`ai-status ${status}`}>
      {icon} {verdictLabel(status)}
    </span>
  );
}

export function duration(r: { createdAt: number; finishedAt?: number | null }): string {
  if (!r.finishedAt) return '—';
  const s = Math.max(1, Math.round((r.finishedAt - r.createdAt) / 1000));
  return s < 60 ? `${s}s` : s < 3600 ? `${Math.floor(s / 60)}m ${s % 60}s` : `${Math.floor(s / 3600)}h ${Math.floor((s % 3600) / 60)}m`;
}

const md = (text: string) => (
  <div className="ai-md">
    <ReactMarkdown
      remarkPlugins={[remarkGfm]}
      components={{
        a: ({ href, children }) =>
          href?.startsWith('/') ? (
            <Link to={href}>{children}</Link>
          ) : (
            <a href={href} target="_blank" rel="noreferrer noopener">
              {children}
            </a>
          ),
        img: () => null,
      }}
    >
      {text}
    </ReactMarkdown>
  </div>
);

export function ChangeView({ change }: { change: CodeChange }) {
  return (
    <div className="ai-change">
      {change.files.map((f) => {
        const lines = lineDiff(f.before ?? '', f.after);
        const added = lines.filter((l) => l.kind === 'add').length;
        const removed = lines.filter((l) => l.kind === 'remove').length;
        return (
          <div key={f.path} className="ai-diff">
            <div className="ai-diff-head">
              <code>{f.path}</code>
              {f.before == null && <span className="ai-verdict ok">new file</span>}
              <span className="ai-ok">+{added}</span> <span className="ai-bad">−{removed}</span>
            </div>
            <table>
              <tbody>
                {lines.map((l, i) => (
                  <tr key={i} className={l.kind}>
                    <td className="ai-ln">{l.oldLine ?? ''}</td>
                    <td className="ai-ln">{l.newLine ?? ''}</td>
                    <td className="ai-sign">{l.kind === 'add' ? '+' : l.kind === 'remove' ? '−' : ' '}</td>
                    <td className="ai-code">{l.text}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        );
      })}
    </div>
  );
}

function PullRequestBox({ run }: { run: RunDetail }) {
  const open = useOpenPullRequest();
  const pr = run.pullRequest as { url?: string; number?: number; branch?: string; draft?: boolean; state?: string } | null | undefined;
  if (pr?.url)
    return (
      <a className="btn btn-sm" href={pr.url} target="_blank" rel="noreferrer noopener">
        <GitPullRequest size={14} /> #{pr.number} {pr.state === 'merged' ? 'merged' : pr.draft ? 'draft' : 'open'} <ExternalLink size={12} />
      </a>
    );
  return (
    <span className="ai-row">
      <button className="btn btn-primary btn-sm" disabled={open.isPending} onClick={() => open.mutate(run.id)}>
        <GitPullRequest size={14} /> {open.isPending ? 'Opening…' : 'Create pull request'}
      </button>
      {open.error && <span className="ai-bad">{(open.error as Error).message}</span>}
    </span>
  );
}

function Feedback({ run }: { run: RunDetail }) {
  const fb = useRunFeedback();
  const [remember, setRemember] = useState('');
  const [asking, setAsking] = useState(false);
  if (run.kind !== 'investigation' && run.kind !== 'security') return null;
  return (
    <div className="ai-feedback">
      <span className="muted">Did Assist find the cause?</span>
      <button className={`btn btn-sm${run.feedback === 'helpful' ? ' active' : ''}`} onClick={() => fb.mutate({ id: run.id, feedback: 'helpful' })}>
        <ThumbsUp size={13} /> Yes
      </button>
      <button className={`btn btn-sm${run.feedback === 'not_helpful' ? ' active' : ''}`} onClick={() => (setAsking(true), fb.mutate({ id: run.id, feedback: 'not_helpful' }))}>
        <ThumbsDown size={13} /> Not quite
      </button>
      {asking && (
        <span className="ai-row ai-remember">
          <input className="settings-input" placeholder="What should Assist remember next time? (optional)" value={remember} onChange={(e) => setRemember(e.target.value)} />
          <button
            className="btn btn-sm"
            disabled={!remember.trim() || fb.isPending}
            onClick={() => fb.mutate({ id: run.id, feedback: 'not_helpful', remember }, { onSuccess: () => (setAsking(false), setRemember('')) })}
          >
            Remember
          </button>
        </span>
      )}
    </div>
  );
}

/** The run page's body; `back` links to its list. */
export function RunView({ id, back, backLabel, extra }: { id: string; back: string; backLabel: string; extra?: (r: RunDetail) => ReactNode }) {
  const q = useRun(id);
  const cancel = useCancelRun();
  if (q.isPending) return <Loading label="Loading…" />;
  if (q.isError) return <ErrorState error={q.error} onRetry={() => q.refetch()} />;
  const r = q.data;
  const c = r.conclusion;
  const evidence = c?.evidence ?? [];
  const nextSteps = c?.nextSteps ?? [];
  const busy = r.status === 'queued' || r.status === 'running';
  return (
    <div className="ai-run">
      <Link className="link-muted ai-back" to={back}>
        <ArrowLeft size={14} /> {backLabel}
      </Link>
      <header className="ai-run-head">
        <h1>{r.title}</h1>
        <div className="ai-run-meta">
          <StatusBadge status={r.status} />
          <VerdictBadge verdict={r.verdict} />
          {c && <span className="muted">Confidence: {c.confidence}</span>}
          <span className="muted">
            {r.trigger === 'manual' ? `Started by ${r.createdBy}` : `Triggered by ${r.trigger === 'monitor' ? 'a monitor alert' : r.trigger === 'signal' ? 'a new signal' : 'its schedule'}`} ·{' '}
            {timeAgo(r.createdAt)} · {duration(r)}
          </span>
          {busy && (
            <button className="btn btn-sm" disabled={cancel.isPending} onClick={() => cancel.mutate(r.id)}>
              <X size={13} /> Cancel
            </button>
          )}
          {extra?.(r)}
        </div>
      </header>
      {r.error && (
        <div className="ai-error">
          <CircleAlert size={14} /> {r.error}
        </div>
      )}
      {c && (
        <section className="ai-card ai-conclusion">
          <h2>Conclusion</h2>
          {c.rootCause && (
            <p className="ai-root">
              <b>Root cause:</b> {c.rootCause}
            </p>
          )}
          {md(c.summary)}
          {evidence.length > 0 && (
            <>
              <h3>Evidence</h3>
              <ul className="ai-evidence">
                {evidence.map((e, i) => (
                  <li key={i}>
                    <b>{e.link ? e.link.startsWith('/') ? <Link to={e.link}>{e.title}</Link> : e.title : e.title}</b>
                    {e.detail && <span className="muted"> {e.detail}</span>}
                  </li>
                ))}
              </ul>
            </>
          )}
          {nextSteps.length > 0 && (
            <>
              <h3>Next steps</h3>
              <ol className="ai-next">
                {nextSteps.map((s, i) => (
                  <li key={i}>{s}</li>
                ))}
              </ol>
            </>
          )}
          <Feedback run={r} />
        </section>
      )}
      {r.change && (
        <section className="ai-card">
          <div className="ai-row ai-spread">
            <h2>Proposed change: {r.change.title}</h2>
            <PullRequestBox run={r} />
          </div>
          {r.change.description && md(r.change.description)}
          <ChangeView change={r.change} />
        </section>
      )}
      <RunTimeline run={r} />
    </div>
  );
}

/** A product's header: icon and name, tabs, and actions on the right. */
export function ProductHeader({ icon, name, tabs, actions }: { icon: ReactNode; name: string; tabs?: { to: string; label: string; end?: boolean }[]; actions?: ReactNode }) {
  return (
    <header className="product-header">
      <div className="product-title">
        {icon}
        {name}
      </div>
      {tabs && <Tabs tabs={tabs} />}
      {actions && <div className="ai-header-actions">{actions}</div>}
    </header>
  );
}

function Tabs({ tabs }: { tabs: { to: string; label: string; end?: boolean }[] }) {
  return (
    <nav className="product-tabs">
      {tabs.map((t) => (
        <NavLink key={t.to} to={t.to} end={t.end} className={({ isActive }) => `product-tab${isActive ? ' active' : ''}`}>
          {t.label}
        </NavLink>
      ))}
    </nav>
  );
}
