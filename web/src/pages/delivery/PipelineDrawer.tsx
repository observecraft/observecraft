import { useEffect, useMemo, useState } from 'react';
import { createPortal } from 'react-dom';
import { ChevronDown, ChevronRight, X } from 'lucide-react';
import type { CiSpan } from '../../api/ci/client';
import { ErrorState, Loading } from '../../components/ui/AsyncState';
import { Branch, Ext, PROVIDER_LABEL, STATUS_COLOR, Sha, StatusPill, fmtDate, fmtDur } from './common';
import { usePipeline } from './queries';

type Tab = 'waterfall' | 'tests' | 'commit';

/** Side panel for one pipeline execution (a CI trace): jobs/steps waterfall, test results, commit. */
export function PipelineDrawer({ traceId, onClose }: { traceId: string; onClose: () => void }) {
  const q = usePipeline(traceId);
  const [tab, setTab] = useState<Tab>('waterfall');
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === 'Escape' && onClose();
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [onClose]);
  const p = q.data?.pipeline;
  const tests = (q.data?.spans ?? []).filter((s) => s.kind === 'test');
  const failedTests = tests.filter((t) => t.status === 'error');

  return createPortal(
    <div className="ci-drawer-backdrop" onMouseDown={(e) => e.target === e.currentTarget && onClose()}>
      <aside className="ci-drawer" role="dialog" aria-label="Pipeline execution">
        {q.isPending ? (
          <Loading label="Loading pipeline…" />
        ) : q.isError ? (
          <ErrorState error={q.error} onRetry={() => q.refetch()} />
        ) : (
          p && (
            <>
              <header className="ci-drawer-head" style={{ borderTopColor: STATUS_COLOR[p.status] }}>
                <div className="ci-drawer-title">
                  <StatusPill status={p.status} />
                  <h2>{p.pipelineName}</h2>
                  <Ext href={p.runUrl}>#{p.runId}</Ext>
                  {Number(p.attempt) > 1 && <span className="ci-attempt">attempt {p.attempt}</span>}
                  <button className="icon-plain ci-drawer-close" onClick={onClose} title="Close">
                    <X size={20} />
                  </button>
                </div>
                <div className="ci-drawer-meta">
                  <Ext href={p.repositoryUrl}>{p.repository}</Ext>
                  <Branch name={p.branch} />
                  <Sha sha={p.sha} />
                  <span>{PROVIDER_LABEL[p.provider] ?? p.provider}</span>
                  <span>Started {fmtDate(p.start)}</span>
                  <span>Duration {fmtDur(p.durationMs)}</span>
                </div>
              </header>
              <nav className="ci-drawer-tabs">
                <button className={tab === 'waterfall' ? 'active' : ''} onClick={() => setTab('waterfall')}>
                  Waterfall
                </button>
                <button className={tab === 'tests' ? 'active' : ''} onClick={() => setTab('tests')}>
                  Tests {tests.length > 0 && <span className={`ci-count${failedTests.length ? ' bad' : ''}`}>{failedTests.length ? `${failedTests.length} failed` : tests.length}</span>}
                </button>
                <button className={tab === 'commit' ? 'active' : ''} onClick={() => setTab('commit')}>
                  Commit
                </button>
              </nav>
              <div className="ci-drawer-body">
                {tab === 'waterfall' && <Waterfall spans={q.data.spans} />}
                {tab === 'tests' && <Tests tests={tests} />}
                {tab === 'commit' && (
                  <dl className="ci-commit">
                    <dt>Message</dt>
                    <dd>{p.message || '—'}</dd>
                    <dt>Author</dt>
                    <dd>{p.author || '—'}</dd>
                    <dt>Commit</dt>
                    <dd>
                      <code>{p.sha}</code>
                    </dd>
                    <dt>Branch</dt>
                    <dd>{p.branch}</dd>
                    <dt>Repository</dt>
                    <dd>
                      <Ext href={p.repositoryUrl}>{p.repository}</Ext>
                    </dd>
                    <dt>Team</dt>
                    <dd>{p.team || '—'}</dd>
                  </dl>
                )}
              </div>
            </>
          )
        )}
      </aside>
    </div>,
    document.body,
  );
}

function Waterfall({ spans }: { spans: CiSpan[] }) {
  const root = spans.find((s) => s.kind === 'pipeline');
  const children = useMemo(() => {
    const m = new Map<string, CiSpan[]>();
    for (const s of spans) {
      if (!s.parentSpanId) continue;
      m.set(s.parentSpanId, [...(m.get(s.parentSpanId) ?? []), s]);
    }
    return m;
  }, [spans]);
  // Failed jobs start expanded.
  const [open, setOpen] = useState<Set<string>>(() => new Set(spans.filter((s) => s.kind === 'job' && s.status === 'error').map((s) => s.spanId)));
  if (!root) return null;
  const t0 = root.start;
  const total = Math.max(root.durationMs, 1);
  const bar = (s: CiSpan) => ({
    left: `${((s.start - t0) / total) * 100}%`,
    width: `${Math.max((s.durationMs / total) * 100, 0.4)}%`,
    background: STATUS_COLOR[s.status] ?? STATUS_COLOR.success,
  });
  const rows: { s: CiSpan; depth: number }[] = [{ s: root, depth: 0 }];
  for (const job of children.get(root.spanId) ?? []) {
    rows.push({ s: job, depth: 1 });
    if (open.has(job.spanId)) {
      for (const step of children.get(job.spanId) ?? []) rows.push({ s: step, depth: 2 });
    }
  }
  return (
    <div className="ci-wf">
      {rows.map(({ s, depth }) => {
        const expandable = s.kind === 'job' && (children.get(s.spanId)?.length ?? 0) > 0;
        return (
          <div key={s.spanId} className={`ci-wf-row d${depth}`}>
            <div className="ci-wf-label" style={{ paddingLeft: depth * 18 }}>
              {expandable ? (
                <button
                  className="icon-plain"
                  onClick={() => {
                    const n = new Set(open);
                    if (n.has(s.spanId)) n.delete(s.spanId);
                    else n.add(s.spanId);
                    setOpen(n);
                  }}
                >
                  {open.has(s.spanId) ? <ChevronDown size={14} /> : <ChevronRight size={14} />}
                </button>
              ) : (
                <span className="ci-wf-spacer" />
              )}
              <i style={{ background: STATUS_COLOR[s.status] }} />
              <span className="ci-wf-name" title={s.name}>
                {s.name}
              </span>
              {s.stage && <span className="ci-wf-stage">{s.stage}</span>}
              {s.environment && <span className="ci-wf-env">{s.environment}</span>}
            </div>
            <div className="ci-wf-track">
              <span className="ci-wf-bar" style={bar(s)} />
            </div>
            <span className="ci-wf-dur">{fmtDur(s.durationMs)}</span>
          </div>
        );
      })}
    </div>
  );
}

function Tests({ tests }: { tests: CiSpan[] }) {
  const [onlyFailed, setOnlyFailed] = useState(tests.some((t) => t.status === 'error'));
  if (tests.length === 0) return <p className="muted">This pipeline didn't report test results.</p>;
  const shown = onlyFailed ? tests.filter((t) => t.status === 'error') : tests;
  const failed = tests.filter((t) => t.status === 'error').length;
  return (
    <>
      <div className="ci-tests-bar">
        <b>{tests.length}</b> tests · <span className="ci-bad">{failed} failed</span> · <span className="ci-good">{tests.length - failed} passed</span>
        <label>
          <input type="checkbox" className="checkbox" checked={onlyFailed} onChange={() => setOnlyFailed((v) => !v)} /> Failed only
        </label>
      </div>
      <table className="ci-table">
        <thead>
          <tr>
            <th>STATUS</th>
            <th>SUITE</th>
            <th>TEST</th>
            <th>DURATION</th>
          </tr>
        </thead>
        <tbody>
          {shown.map((t) => (
            <tr key={t.spanId}>
              <td>
                <StatusPill status={t.status === 'error' ? 'error' : 'success'} />
              </td>
              <td>{t.testSuite}</td>
              <td>{t.name.startsWith(`${t.testSuite}.`) ? t.name.slice(t.testSuite.length + 1) : t.name}</td>
              <td>{fmtDur(t.durationMs)}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </>
  );
}
