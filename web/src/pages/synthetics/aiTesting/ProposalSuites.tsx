import { useState } from 'react';
import { Link } from 'react-router-dom';
import { Check, ChevronDown, ChevronRight, CircleAlert, ExternalLink, Layers, Play, X } from 'lucide-react';
import type { AiTestingProposal } from '../../../api/aiTesting/client';
import { STEP_LABEL, stepName } from '../../../api/synthetics/browserSteps';
import { useDecide } from './queries';

/** Proposals grouped into their suites (one per job and journey), to start testing or ignore. */
export function ProposalSuites({ proposals, showJob }: { proposals: AiTestingProposal[]; showJob?: boolean }) {
  const groups = new Map<string, AiTestingProposal[]>();
  for (const p of proposals) groups.set(`${p.runId}\n${p.suite}`, [...(groups.get(`${p.runId}\n${p.suite}`) ?? []), p]);
  return (
    <div className="ait-suites">
      {[...groups.entries()].map(([key, tests]) => (
        <SuiteCard key={key} suite={tests[0].suite} runId={tests[0].runId} tests={tests} showJob={showJob} />
      ))}
    </div>
  );
}

function SuiteCard({ suite, runId, tests, showJob }: { suite: string; runId: string; tests: AiTestingProposal[]; showJob?: boolean }) {
  const { accept, reject } = useDecide();
  const [ignoring, setIgnoring] = useState<string[] | null>(null);
  const [feedback, setFeedback] = useState('');
  const pending = tests.filter((t) => t.status === 'pending').map((t) => t.id);
  const busy = accept.isPending || reject.isPending;
  const error = (accept.error ?? reject.error) as Error | null;
  return (
    <section className="ait-suite">
      <header className="ait-suite-head">
        <Layers size={16} />
        <h3>{suite}</h3>
        <span className="muted">
          {tests.length} browser test{tests.length === 1 ? '' : 's'}
        </span>
        {showJob && (
          <Link className="link-muted" to={`/synthetics/ai-testing/jobs/${runId}`}>
            from this job <ExternalLink size={12} />
          </Link>
        )}
        <span className="ait-suite-actions">
          {pending.length > 0 && (
            <>
              <button className="btn btn-sm" disabled={busy} onClick={() => setIgnoring(pending)}>
                Ignore
              </button>
              <button className="btn btn-sm btn-primary" disabled={busy} onClick={() => accept.mutate(pending)}>
                <Play size={13} /> Start Testing{pending.length < tests.length ? ` (${pending.length})` : ''}
              </button>
            </>
          )}
        </span>
      </header>
      {ignoring && (
        <div className="ait-ignore">
          <textarea className="syn-textarea" rows={2} value={feedback} onChange={(e) => setFeedback(e.target.value)} placeholder="Why aren't these useful? (optional)" aria-label="Feedback" />
          <span className="ait-row">
            <button className="btn btn-sm" onClick={() => setIgnoring(null)}>
              Cancel
            </button>
            <button className="btn btn-sm btn-danger" disabled={busy} onClick={() => reject.mutate({ ids: ignoring, feedback }, { onSuccess: () => setIgnoring(null) })}>
              Ignore {ignoring.length} test{ignoring.length === 1 ? '' : 's'}
            </button>
          </span>
        </div>
      )}
      {error && <p className="form-error">{error.message}</p>}
      <ul className="ait-tests">
        {tests.map((t) => (
          <ProposalRow key={t.id} p={t} busy={busy} onAccept={() => accept.mutate([t.id])} onIgnore={() => setIgnoring([t.id])} />
        ))}
      </ul>
    </section>
  );
}

function ProposalRow({ p, busy, onAccept, onIgnore }: { p: AiTestingProposal; busy: boolean; onAccept: () => void; onIgnore: () => void }) {
  const [open, setOpen] = useState(false);
  const v = p.verification;
  return (
    <li className={`ait-test ${p.status}`}>
      <div className="ait-test-head">
        <button className="icon-plain" onClick={() => setOpen((o) => !o)} aria-expanded={open} title={open ? 'Hide the steps' : 'Show the steps'}>
          {open ? <ChevronDown size={15} /> : <ChevronRight size={15} />}
        </button>
        <span className="ait-test-name">
          <b>{p.name}</b>
          {p.description && <span className="muted">{p.description}</span>}
        </span>
        {v ? (
          <span className={`ait-sample ${v.passed ? 'pass' : 'fail'}`} title={v.error ?? 'The sample run passed'}>
            {v.passed ? <Check size={12} /> : <CircleAlert size={12} />} Sample run {v.passed ? 'passed' : 'failed'}
          </span>
        ) : (
          <span className="ait-sample">No sample run</span>
        )}
        <span className="muted">{p.steps.length} steps</span>
        {p.status === 'pending' ? (
          <span className="ait-row">
            <button className="icon-plain" disabled={busy} title="Ignore" onClick={onIgnore}>
              <X size={15} />
            </button>
            <button className="btn btn-sm" disabled={busy} onClick={onAccept}>
              Start Testing
            </button>
          </span>
        ) : p.status === 'accepted' && p.testId ? (
          <Link className="ait-status accepted" to={`/synthetics/tests/${p.testId}`}>
            <Check size={13} /> Testing
          </Link>
        ) : (
          <span className={`ait-status ${p.status}`} title={p.feedback || undefined}>
            {p.status === 'rejected' ? 'Ignored' : p.status}
          </span>
        )}
      </div>
      {open && (
        <div className="ait-test-body">
          {v && !v.passed && v.error && <p className="syn-run-error">{v.error}</p>}
          <ol className="ait-steps">
            <li>
              <span className="syn-chip">Navigate to URL</span> <span className="mono syn-wrap">{p.startUrl}</span>
            </li>
            {p.steps.map((s, i) => (
              <li key={i}>
                <span className="syn-chip">{STEP_LABEL[s.type]}</span> {stepName(s)}
                {s.params?.element?.userLocator?.values?.[0] && <span className="mono muted ait-locator">{s.params.element.userLocator.values[0].value}</span>}
              </li>
            ))}
          </ol>
        </div>
      )}
    </li>
  );
}
