// "Fix with Assist" on a security finding's side panel (ADR-0008): what can be done about the finding (explain, fix in
// code, fix in infrastructure, needs provenance), an Assist explanation with remediation steps, and a fix reviewed as a
// diff before its merge request is opened. Infrastructure fixes show the resolved source (an inferred one is confirmed
// first); findings without one list the tags the resource lacks and the snippet that adds them. When the connection
// can't write, the diff is copied instead.
import { useState } from 'react';
import { Link } from 'react-router-dom';
import ReactMarkdown from 'react-markdown';
import remarkGfm from 'remark-gfm';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { CircleAlert, Copy, ExternalLink, FolderGit2, GitPullRequest, LoaderCircle, Sparkles, Tags, Wrench } from 'lucide-react';
import {
  explainFinding,
  fixFinding,
  openMergeRequest,
  remediationOf,
  type FindingRef,
  type FixSource,
  type FixView,
  type MissingTag,
  type ProvenanceSnippet,
  type Remediation,
} from '../../api/remediation/client';
import type { Provenance } from '../../api/tagging/types';
import { VIA_LABELS } from '../provenance/SourceCell';
import { ChangeView } from '../../pages/assist/RunView';
import { useRun } from '../../pages/assist/runsQueries';
import { unifiedDiff } from '../../lib/unifiedDiff';
import '../../pages/assist/assist.css';
import './remediation.css';

const busy = (s?: string | null) => s === 'queued' || s === 'running';
const LEVEL: Record<Remediation['level'], string> = { explain: 'Explain', fix_in_code: 'Fix in code', fix_in_infra: 'Fix in infrastructure', needs_provenance: 'Needs provenance' };
const KIND: Record<Provenance['kind'], string> = { terraform: 'Terraform', helm: 'Helm values', kustomize: 'Kustomize', manifest: 'Manifests', dockerfile: 'Dockerfile', code: 'Code' };

const md = (text: string) => (
  <div className="ai-md">
    <ReactMarkdown remarkPlugins={[remarkGfm]} components={{ img: () => null }}>
      {text}
    </ReactMarkdown>
  </div>
);

export function RemediationCard({ finding }: { finding: FindingRef }) {
  const qc = useQueryClient();
  const key = ['remediation', finding.kind, finding.id];
  const q = useQuery({
    queryKey: key,
    queryFn: ({ signal }) => remediationOf(finding, signal),
    refetchInterval: (query) => (busy(query.state.data?.explainRun?.status) || busy(query.state.data?.fix?.status) ? 2000 : false),
  });
  const refresh = () => qc.invalidateQueries({ queryKey: key });
  const explain = useMutation({ mutationFn: () => explainFinding(finding), onSuccess: refresh });
  const [repository, setRepository] = useState('');
  const [confirmed, setConfirmed] = useState(false);
  const fix = useMutation({ mutationFn: () => fixFinding({ ...finding, repository: repository || null, confirmed }), onSuccess: refresh });
  if (q.isPending) return null;
  if (q.isError) return <div className="rem-card rem-error">{(q.error as Error).message}</div>;
  const r = q.data;
  const error = (explain.error ?? fix.error) as Error | null;
  return (
    <section className="rem-card" aria-label="Fix with Assist">
      <header className="rem-head">
        <h4>
          <Sparkles size={13} /> FIX WITH ASSIST
        </h4>
        <span className={`rem-level ${r.level}`}>{LEVEL[r.level]}</span>
      </header>
      <p className="muted">{r.reason}</p>
      {r.source && <SourceBlock p={r.source} />}
      {r.level === 'fix_in_infra' && r.confirm && r.source && (
        <label className="rem-confirm">
          <input type="checkbox" checked={confirmed} onChange={(e) => setConfirmed(e.target.checked)} />
          <span>
            I checked that <code>{[r.source.repo, r.source.path].filter(Boolean).join('/')}</code> defines this resource: Assist may edit it.
          </span>
        </label>
      )}
      {r.level === 'needs_provenance' && (r.missingTags.length > 0 || r.snippet) && <MissingTags tags={r.missingTags} snippet={r.snippet ?? null} />}
      {r.level === 'needs_provenance' && r.taggingPolicies && (
        <p className="rem-provenance">
          <Tags size={13} /> Tag the resource with the repository and file that create it, then Assist can propose the fix there. <Link to={r.taggingPolicies}>Set up Tagging Policies</Link>
        </p>
      )}
      {r.connection && (
        <p className="rem-warn">
          <CircleAlert size={13} /> {r.connection}
        </p>
      )}
      <div className="rem-actions">
        {r.targets.length > 1 && (
          <select className="settings-input rem-repo" aria-label="Repository to fix" value={repository || r.targets[0].repository} onChange={(e) => setRepository(e.target.value)}>
            {r.targets.map((t) => (
              <option key={t.repository} value={t.repository}>
                {t.repository}
              </option>
            ))}
          </select>
        )}
        {(r.level === 'fix_in_code' || r.level === 'fix_in_infra') && (
          <button className="btn btn-primary btn-sm" disabled={!!r.connection || fix.isPending || busy(r.fix?.status) || (r.confirm && !confirmed)} onClick={() => fix.mutate()}>
            <Wrench size={13} /> {fix.isPending ? 'Preparing…' : r.fix ? 'Fix again' : 'Fix'}
          </button>
        )}
        <button className="btn btn-sm" disabled={explain.isPending || busy(r.explainRun?.status)} onClick={() => explain.mutate()}>
          <Sparkles size={13} /> {explain.isPending ? 'Starting…' : r.explainRun ? 'Explain again' : 'Explain'}
        </button>
      </div>
      {error && <p className="rem-warn">{error.message}</p>}
      {r.explainRun && <Explanation runId={r.explainRun.id} />}
      {r.fix && <FixPanel fix={r.fix} onChange={refresh} />}
    </section>
  );
}

/** Where the resource is defined: the repository, path and address or Application, how it was found and how sure. */
export function SourceBlock({ p, title = 'Source' }: { p: Provenance; title?: string }) {
  return (
    <div className="rem-source" aria-label="Resolved source">
      <div className="rem-row">
        <h5>
          <FolderGit2 size={13} /> {title}: {KIND[p.kind]}
        </h5>
        <span className="rem-badges">
          <span className={`rem-conf ${p.confidence}`}>{p.confidence}</span>
          <span className="tag-chip">via {VIA_LABELS[p.via] ?? p.via}</span>
        </span>
      </div>
      <dl>
        {p.repo && (
          <>
            <dt>Repository</dt>
            <dd className="mono">{p.repo}</dd>
          </>
        )}
        {p.path && (
          <>
            <dt>Path</dt>
            <dd className="mono">{p.path}</dd>
          </>
        )}
        {p.ref && (
          <>
            <dt>Ref</dt>
            <dd className="mono">{p.ref}</dd>
          </>
        )}
        {p.application && (
          <>
            <dt>Argo CD Application</dt>
            <dd className="mono">{p.application}</dd>
          </>
        )}
        {p.address && (
          <>
            <dt>{p.kind === 'terraform' ? 'Address' : 'Object'}</dt>
            <dd className="mono">{p.address}</dd>
          </>
        )}
      </dl>
    </div>
  );
}

/** The tags a resource lacks (per the policy requiring them) and the snippet that adds them. */
function MissingTags({ tags, snippet }: { tags: MissingTag[]; snippet: ProvenanceSnippet | null }) {
  const [copied, setCopied] = useState(false);
  const copy = async () => {
    if (!snippet) return;
    try {
      await navigator.clipboard.writeText(snippet.code);
      setCopied(true);
    } catch {
      setCopied(false);
    }
  };
  return (
    <div className="rem-block">
      {tags.length > 0 && (
        <>
          <h5>Missing tags</h5>
          <div className="rem-tags">
            {tags.map((t) => (
              <span key={t.key} className="tag-chip mono" title={t.policy}>
                {t.key.split('|').join(' | ')}
                {t.value != null && ` = ${t.value} (not allowed)`}
              </span>
            ))}
          </div>
        </>
      )}
      {snippet && (
        <>
          <div className="rem-row">
            <h5>{snippet.title}</h5>
            <button className="btn btn-sm" onClick={copy}>
              <Copy size={13} /> {copied ? 'Copied' : 'Copy'}
            </button>
          </div>
          <pre className="rem-snippet">
            <code>{snippet.code}</code>
          </pre>
        </>
      )}
    </div>
  );
}

function FixSourceLine({ s }: { s: FixSource }) {
  return (
    <p className="muted rem-fine">
      {s.path != null ? (
        <>
          Tags for <b>{s.resource}</b> under <code>{s.path || '(repository root)'}</code>
        </>
      ) : (
        <>
          <b>{s.resource}</b>
          {s.located ? (
            <>
              {' '}
              · <code>{s.located.address}</code> in <code>{s.located.file}</code> (lines {s.located.startLine}–{s.located.endLine})
            </>
          ) : (
            s.provenance && (
              <>
                {' '}
                · {s.provenance.repo}/{s.provenance.path ?? ''}
              </>
            )
          )}
          {s.confirmed && ' · inferred source confirmed'}
        </>
      )}
    </p>
  );
}

function Progress({ runId }: { runId: string }) {
  const run = useRun(runId);
  const steps = run.data?.steps ?? [];
  const last = [...steps].reverse().find((s) => s.type === 'text');
  return (
    <div className="rem-progress">
      <LoaderCircle size={13} className="ai-spin" /> {run.data?.status === 'queued' ? 'Queued…' : 'Assist is working…'}
      <ul>{steps.filter((s) => s.type === 'tool').map((s) => s.type === 'tool' && <li key={s.id}>{`${s.name.replace(/_/g, ' ')}${s.summary ? `: ${s.summary}` : ''}`}</li>)}</ul>
      {last && last.type === 'text' && <div className="muted">{last.text}</div>}
    </div>
  );
}

function Explanation({ runId }: { runId: string }) {
  const run = useRun(runId);
  if (!run.data) return null;
  if (busy(run.data.status)) return <Progress runId={runId} />;
  const c = run.data.conclusion;
  if (!c) return <p className="rem-warn">{run.data.error ?? 'The explanation stopped before it concluded.'}</p>;
  return (
    <div className="rem-block">
      <h5>Explanation</h5>
      {md(c.summary)}
      {(c.nextSteps ?? []).length > 0 && (
        <>
          <h5>Remediation steps</h5>
          <ol className="rem-steps">
            {(c.nextSteps ?? []).map((s, i) => (
              <li key={i}>{s}</li>
            ))}
          </ol>
        </>
      )}
    </div>
  );
}

/** A fix: its progress, the proposed change as a diff, and its merge request. */
export function FixPanel({ fix, onChange }: { fix: FixView; onChange: () => void }) {
  const open = useMutation({ mutationFn: () => openMergeRequest(fix.id), onSuccess: onChange });
  const [copied, setCopied] = useState(false);
  if (busy(fix.status) && fix.runId) return <Progress runId={fix.runId} />;
  const mr = fix.mergeRequest;
  const copy = async () => {
    if (!fix.change) return;
    try {
      await navigator.clipboard.writeText(unifiedDiff(fix.change));
      setCopied(true);
    } catch {
      setCopied(false);
    }
  };
  return (
    <div className="rem-block">
      <div className="rem-row">
        <h5>{fix.change ? `Proposed change: ${fix.change.title}` : 'No change proposed'}</h5>
        {mr ? (
          <a className={`btn btn-sm rem-mr ${mr.state}`} href={mr.url} target="_blank" rel="noreferrer noopener">
            <GitPullRequest size={13} /> {fix.repository.startsWith('gitlab') ? '!' : '#'}
            {mr.number} {mr.state === 'open' && mr.draft ? 'draft' : mr.state} <ExternalLink size={11} />
          </a>
        ) : (
          fix.change &&
          !fix.blocked && (
            <button className="btn btn-primary btn-sm" disabled={open.isPending} onClick={() => open.mutate()}>
              <GitPullRequest size={13} /> {open.isPending ? 'Opening…' : 'Open merge request'}
            </button>
          )
        )}
      </div>
      {fix.source && <FixSourceLine s={fix.source} />}
      {fix.explanation && md(fix.explanation)}
      {fix.change && (
        <>
          {fix.change.description && md(fix.change.description)}
          <ChangeView change={fix.change} />
        </>
      )}
      {fix.notes.length > 0 && (
        <ul className="rem-notes">
          {fix.notes.map((n) => (
            <li key={n}>{n}</li>
          ))}
        </ul>
      )}
      {mr?.state === 'merged' && <p className="muted">Merged: the next scan of the repository resolves the finding.</p>}
      {fix.change && !mr && fix.blocked && (
        <div className="rem-warn">
          <CircleAlert size={13} /> {fix.blocked} Copy the diff and apply it yourself (`git apply`).
          <button className="btn btn-sm" onClick={copy}>
            <Copy size={13} /> {copied ? 'Copied' : 'Copy diff'}
          </button>
        </div>
      )}
      {open.error && <p className="rem-warn">{(open.error as Error).message}</p>}
      {fix.change && !mr && <p className="muted rem-fine">Nothing is merged automatically: the merge request names the finding and the change, and the repository’s CI must pass.</p>}
    </div>
  );
}
