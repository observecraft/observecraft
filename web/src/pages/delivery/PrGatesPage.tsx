import { useState } from 'react';
import { Link, NavLink } from 'react-router-dom';
import { Bug, CircleCheck, CircleDashed, CircleMinus, CircleX, GitPullRequest, Gauge, Package, Pencil, Plus, Scale, Trash2, X } from 'lucide-react';
import type { PrGateCheckStatus, PrGateEvaluation, PrGateRule, PrGateRuleInput, PrGateSeverity, PrGateSource } from '../../api/deliveryGates/client';
import { CodeBlock } from '../../components/code/CodeBlock';
import { ErrorState, Loading } from '../../components/ui/AsyncState';
import { Modal } from '../../components/ui/Modal';
import { ChipsInput, Switch } from '../featureFlags/common';
import { useAccess } from '../settings/queries';
import { fmtAgo } from './common';
import { NEW_PR_RULE, PR_SOURCES, PR_SOURCE_ORDER, PR_SUGGESTIONS, prCheckSnippet } from './gatesModel';
import { usePrRuleMutations, usePrRules } from './gatesQueries';
import { useRepositories } from './queries';
import '../featureFlags/featureFlags.css';
import './delivery.css';
import './gates.css';

const SUGGESTION_ICON: Record<string, typeof Bug> = { code_vulnerabilities: Bug, code_quality: Gauge, library_vulnerabilities: Package, license_violations: Scale };
const SEVERITIES: PrGateSeverity[] = ['critical', 'high', 'medium', 'low'];

type Editing = { rule: PrGateRule | null; input: PrGateRuleInput } | null;

const inputOf = (r: PrGateRule): PrGateRuleInput => ({
  name: r.name,
  source: r.source,
  repositories: r.repositories,
  threshold: r.threshold,
  minSeverity: r.minSeverity,
  blocking: r.blocking,
  enabled: r.enabled,
});

/** Software Delivery → PR Gates: the first-run page with suggested rules, or the rules list. */
export function PrGatesPage() {
  const q = usePrRules();
  const { canWrite } = useAccess();
  const [editing, setEditing] = useState<Editing>(null);
  const open = (input: PrGateRuleInput, rule: PrGateRule | null = null) => setEditing({ rule, input });
  return (
    <div className="ci-layout">
      <header className="product-header">
        <div className="product-title">
          <GitPullRequest size={20} /> PR Gates <span className="badge-new">NEW</span>
        </div>
        <nav className="product-tabs">
          <NavLink to="/delivery/pr-gates" end className={({ isActive }) => `product-tab${isActive ? ' active' : ''}`}>
            PR Gates Rules
          </NavLink>
        </nav>
        {canWrite && (q.data?.length ?? 0) > 0 && (
          <div className="ci-header-actions">
            <button className="btn btn-primary" onClick={() => open(NEW_PR_RULE)}>
              <Plus size={14} /> New Rule
            </button>
          </div>
        )}
      </header>
      <div className="ci-section">
        {q.isPending ? (
          <Loading label="Loading PR gate rules…" />
        ) : q.isError ? (
          <ErrorState error={q.error} onRetry={() => q.refetch()} />
        ) : q.data.length === 0 ? (
          <FirstRule canWrite={canWrite} onCreate={open} />
        ) : (
          <RuleList rules={q.data} canWrite={canWrite} onEdit={(r) => open(inputOf(r), r)} />
        )}
      </div>
      {editing && <RuleEditor rule={editing.rule} initial={editing.input} onClose={() => setEditing(null)} />}
    </div>
  );
}

function FirstRule({ canWrite, onCreate }: { canWrite: boolean; onCreate: (input: PrGateRuleInput) => void }) {
  return (
    <div className="prg-empty">
      <h1>
        Catch problems before they merge,
        <br />
        not after they ship
      </h1>
      <p>A PR gate checks every pull request against your rules and reports a pass or fail your repository can require before merging.</p>
      {canWrite && (
        <button className="btn btn-primary prg-cta" onClick={() => onCreate(NEW_PR_RULE)}>
          <Plus size={16} /> Create your first rule
        </button>
      )}
      <div className="prg-suggested">
        <span className="prg-suggested-title">SUGGESTED RULES</span>
        <div className="prg-suggestions">
          {PR_SUGGESTIONS.map((s) => {
            const Icon = SUGGESTION_ICON[s.input.source] ?? Bug;
            return (
              <button key={s.title} className="prg-suggestion" disabled={!canWrite} onClick={() => onCreate(s.input)}>
                <Icon size={20} />
                <b>{s.title}</b>
                <span>{s.description}</span>
              </button>
            );
          })}
        </div>
      </div>
    </div>
  );
}

function condition(r: { source: PrGateSource; threshold?: number; minSeverity?: PrGateSeverity }) {
  const meta = PR_SOURCES[r.source];
  const severity = meta.evaluable ? '' : ` (${r.minSeverity ?? 'high'} or above)`;
  return `More than ${r.threshold ?? 0} ${meta.finding}${severity}`;
}

function RuleList({ rules, canWrite, onEdit }: { rules: PrGateRule[]; canWrite: boolean; onEdit: (r: PrGateRule) => void }) {
  const m = usePrRuleMutations();
  return (
    <div className="dg-page">
      <div className="ci-table-wrap">
        <table className="ci-table dg-table">
          <thead>
            <tr>
              <th>ENABLED</th>
              <th>RULE</th>
              <th>SOURCE</th>
              <th>REPOSITORIES</th>
              <th>FAILS WHEN</th>
              <th>ON FAILURE</th>
              <th>UPDATED</th>
              <th />
            </tr>
          </thead>
          <tbody>
            {rules.map((r) => (
              <tr key={r.id} onClick={() => canWrite && onEdit(r)}>
                <td onClick={(e) => e.stopPropagation()}>
                  <Switch
                    checked={r.enabled}
                    disabled={!canWrite || m.save.isPending}
                    onChange={(enabled) => m.save.mutate({ id: r.id, input: { ...inputOf(r), enabled } })}
                    label={r.enabled ? 'On' : 'Off'}
                  />
                </td>
                <td>
                  <b>{r.name}</b>
                </td>
                <td>
                  {PR_SOURCES[r.source].label}
                  {!r.evaluable && <span className="prg-needs">Needs Code Security</span>}
                </td>
                <td>{r.repositories.length ? r.repositories.join(', ') : <span className="muted">All repositories</span>}</td>
                <td>{condition(r)}</td>
                <td>
                  <span className={`dg-mode${r.blocking ? '' : ' dry'}`}>{r.blocking ? 'Blocking' : 'Non-blocking'}</span>
                </td>
                <td className="muted">
                  {fmtAgo(r.updatedAt)} by {r.updatedBy || '—'}
                </td>
                <td className="dg-actions" onClick={(e) => e.stopPropagation()}>
                  {canWrite && (
                    <>
                      <button className="icon-plain" title="Edit" onClick={() => onEdit(r)}>
                        <Pencil size={15} />
                      </button>
                      <button className="icon-plain" title="Delete" onClick={() => window.confirm(`Delete the rule "${r.name}"?`) && m.remove.mutate(r.id)}>
                        <Trash2 size={15} />
                      </button>
                    </>
                  )}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
      {(m.save.error || m.remove.error) && <p className="form-error">{((m.save.error ?? m.remove.error) as Error).message}</p>}
      <CheckCommit />
    </div>
  );
}

const CHECK_ICON: Record<PrGateCheckStatus, typeof CircleCheck> = { passed: CircleCheck, failed: CircleX, no_data: CircleDashed, not_evaluated: CircleMinus };
const CHECK_LABEL: Record<PrGateCheckStatus, string> = { passed: 'Passed', failed: 'Failed', no_data: 'No CI data', not_evaluated: 'Not evaluated' };

/** Evaluate a commit against the rules, as a pull request check would, and the snippet to run it in CI. */
function CheckCommit() {
  const m = usePrRuleMutations();
  const repos = useRepositories();
  const [repository, setRepository] = useState('');
  const [sha, setSha] = useState('');
  const [result, setResult] = useState<PrGateEvaluation | null>(null);
  return (
    <section className="prg-check">
      <h3>Check a pull request</h3>
      <p className="muted">
        Run this from your CI on every pull request and make the step a required check. Rules are evaluated with the CI Visibility results of the head commit; Code Security rules are reported as not
        evaluated.
      </p>
      <CodeBlock code={prCheckSnippet(window.location.origin)} />
      <form
        className="prg-check-form"
        onSubmit={(e) => {
          e.preventDefault();
          m.evaluate.mutate({ repository, sha }, { onSuccess: setResult });
        }}
      >
        <label className="form-label">
          Repository
          <input list="prg-repos" value={repository} onChange={(e) => setRepository(e.target.value)} placeholder="example-org/billing" required />
          <datalist id="prg-repos">
            {(repos.data ?? []).map((r) => (
              <option key={r.repository} value={r.repository} />
            ))}
          </datalist>
        </label>
        <label className="form-label">
          Commit SHA
          <input className="mono" value={sha} onChange={(e) => setSha(e.target.value)} placeholder="a1b2c3d" required pattern="[0-9a-fA-F]{7,64}" />
        </label>
        <button className="btn" disabled={m.evaluate.isPending}>
          Evaluate
        </button>
      </form>
      {m.evaluate.error && <p className="form-error">{(m.evaluate.error as Error).message}</p>}
      {result && (
        <div className={`prg-result prg-result-${result.status}`}>
          <b>
            {result.pass ? <CircleCheck size={16} /> : <CircleX size={16} />} {result.pass ? 'The pull request passes' : 'The pull request is blocked'}
          </b>
          <span className="muted">
            {result.repository} @ <code>{result.sha.slice(0, 12)}</code> · {result.ci.pipelines} pipelines, {result.ci.tests} tests
          </span>
          {result.rules.length === 0 && <p className="muted">No enabled rule applies to this repository.</p>}
          <ul className="dg-results">
            {result.rules.map((r) => {
              const Icon = CHECK_ICON[r.status];
              return (
                <li key={r.ruleId}>
                  <Icon size={16} className={`prg-check-${r.status}`} />
                  <div>
                    <b>{r.name}</b> <span className="muted">· {CHECK_LABEL[r.status]}</span> {!r.blocking && <span className="dg-mode dry">Non-blocking</span>}
                    <div className="muted">{r.reason}</div>
                  </div>
                </li>
              );
            })}
          </ul>
        </div>
      )}
    </section>
  );
}

function RuleEditor({ rule, initial, onClose }: { rule: PrGateRule | null; initial: PrGateRuleInput; onClose: () => void }) {
  const m = usePrRuleMutations();
  const repos = useRepositories();
  const [input, setInput] = useState<PrGateRuleInput>(initial);
  const [scoped, setScoped] = useState((initial.repositories ?? []).length > 0);
  const set = (p: Partial<PrGateRuleInput>) => setInput((i) => ({ ...i, ...p }));
  const meta = PR_SOURCES[input.source];
  const save = () => m.save.mutate({ id: rule?.id, input: { ...input, repositories: scoped ? input.repositories : [] } }, { onSuccess: onClose });
  return (
    <Modal onClose={onClose} label={rule ? 'Edit PR gate rule' : 'New PR gate rule'} width={760} height="auto">
      <form
        className="small-modal dg-editor"
        onSubmit={(e) => {
          e.preventDefault();
          save();
        }}
      >
        <div className="modal-title-row">
          <h3>
            <GitPullRequest size={18} /> {rule ? `Edit ${rule.name}` : 'New PR Gate Rule'}
          </h3>
          <button type="button" className="icon-plain" onClick={onClose} title="Close">
            <X size={18} />
          </button>
        </div>
        <h4 className="dg-step">
          <span>1</span> Choose what to check
        </h4>
        <label className="form-label">
          Source
          <select value={input.source} onChange={(e) => set({ source: e.target.value as PrGateSource })}>
            <optgroup label="CI Visibility">
              {PR_SOURCE_ORDER.filter((s) => PR_SOURCES[s].evaluable).map((s) => (
                <option key={s} value={s}>
                  {PR_SOURCES[s].label}
                </option>
              ))}
            </optgroup>
            <optgroup label="Code Security (not available yet)">
              {PR_SOURCE_ORDER.filter((s) => !PR_SOURCES[s].evaluable).map((s) => (
                <option key={s} value={s}>
                  {PR_SOURCES[s].label} — needs Code Security
                </option>
              ))}
            </optgroup>
          </select>
        </label>
        <p className="muted">{meta.description}.</p>
        {!meta.evaluable && (
          <p className="prg-notice">
            This rule needs Code Security, which this installation doesn’t have yet. It is saved and listed, and pull request checks report it as <b>not evaluated</b> — never as passed — until
            findings exist.
          </p>
        )}

        <h4 className="dg-step">
          <span>2</span> Repositories
        </h4>
        <div className="dg-mode-pick" role="radiogroup" aria-label="Repositories">
          <button type="button" role="radio" aria-checked={!scoped} className={!scoped ? 'active' : ''} onClick={() => setScoped(false)}>
            <b>All repositories</b>
            <span>Every pull request is checked.</span>
          </button>
          <button type="button" role="radio" aria-checked={scoped} className={scoped ? 'active' : ''} onClick={() => setScoped(true)}>
            <b>Selected repositories</b>
            <span>Only the repositories you list.</span>
          </button>
        </div>
        {scoped && (
          <ChipsInput
            label="Repositories"
            values={input.repositories ?? []}
            onChange={(repositories) => set({ repositories })}
            placeholder="example-org/billing, then Enter"
            suggestions={(repos.data ?? []).map((r) => r.repository)}
          />
        )}

        <h4 className="dg-step">
          <span>3</span> Condition
        </h4>
        <div className="prg-condition">
          Fail when there are more than
          <input type="number" min={0} max={100000} value={input.threshold ?? 0} onChange={(e) => set({ threshold: Number(e.target.value) })} aria-label="Threshold" />
          <span>{meta.finding}</span>
          {!meta.evaluable && (
            <>
              of
              <select value={input.minSeverity ?? 'high'} onChange={(e) => set({ minSeverity: e.target.value as PrGateSeverity })} aria-label="Lowest severity">
                {SEVERITIES.map((s) => (
                  <option key={s} value={s}>
                    {s}
                  </option>
                ))}
              </select>
              severity or above
            </>
          )}
        </div>
        <div className="dg-mode-pick" role="radiogroup" aria-label="On failure">
          <button type="button" role="radio" aria-checked={input.blocking ?? true} className={(input.blocking ?? true) ? 'active' : ''} onClick={() => set({ blocking: true })}>
            <b>Blocking</b>
            <span>A failure fails the pull request check.</span>
          </button>
          <button type="button" role="radio" aria-checked={!(input.blocking ?? true)} className={!(input.blocking ?? true) ? 'active' : ''} onClick={() => set({ blocking: false })}>
            <b>Non-blocking</b>
            <span>Failures are reported; the check still passes.</span>
          </button>
        </div>

        <h4 className="dg-step">
          <span>4</span> Name
        </h4>
        <label className="form-label">
          Rule name
          <input value={input.name} onChange={(e) => set({ name: e.target.value })} placeholder="No failing tests" required maxLength={200} />
        </label>
        <Switch checked={input.enabled ?? true} onChange={(enabled) => set({ enabled })} label="Enabled" />
        <p className="muted">
          Pull request checks call <code>POST /api/v1/pr-gates/evaluate</code> with an application key from <Link to="/organization-settings/api-keys">API Keys</Link>.
        </p>
        {m.save.error && <p className="form-error">{(m.save.error as Error).message}</p>}
        <div className="small-modal-actions">
          <button type="button" className="btn" onClick={onClose}>
            Cancel
          </button>
          <button className="btn btn-primary" disabled={m.save.isPending}>
            {rule ? 'Save' : 'Create Rule'}
          </button>
        </div>
      </form>
    </Modal>
  );
}
