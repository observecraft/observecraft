import { useDeferredValue, useEffect, useMemo, useState } from 'react';
import { createPortal } from 'react-dom';
import { Link, NavLink, Outlet, useOutletContext } from 'react-router-dom';
import { Activity, BellRing, BookOpen, CircleCheck, CircleX, FlaskConical, Pencil, Play, Plus, Search, ShieldCheck, Terminal, Trash2, X } from 'lucide-react';
import type { DeploymentGate, DeploymentGateInput, GateEvaluation, GateEvaluationStatus, GateRule, GateRuleResult } from '../../api/deliveryGates/client';
import { CodeBlock } from '../../components/code/CodeBlock';
import { ErrorState, Loading } from '../../components/ui/AsyncState';
import { Modal } from '../../components/ui/Modal';
import { useServices } from '../apm/queries';
import { Switch } from '../featureFlags/common';
import { IntegrationLogo } from '../integrations/IntegrationLogo';
import { useAccess } from '../settings/queries';
import { fmtAgo, fmtDate, fmtDur } from './common';
import { CI_TOOLS, MONITOR_STATUS_LABEL, STATUS_LABEL, ciSnippet, filterEvaluations, type CiToolId } from './gatesModel';
import { useEvaluation, useEvaluations, useGateMutations, useGates } from './gatesQueries';
import '../featureFlags/featureFlags.css';
import './delivery.css';
import './gates.css';

const apiUrl = () => window.location.origin;

type Editing = DeploymentGate | 'new' | null;
interface GatesContext {
  gates: DeploymentGate[];
  edit: (g: Editing) => void;
}

/** Deployment Gates frame: the intro until a gate exists, then the Gates and Evaluations tabs. */
export function DeploymentGatesLayout() {
  const q = useGates();
  const [editing, setEditing] = useState<Editing>(null);
  const { canWrite } = useAccess();
  const editor = editing && <GateEditor gate={editing === 'new' ? null : editing} gates={q.data ?? []} onClose={() => setEditing(null)} />;
  if (q.isPending) return <Loading label="Loading deployment gates…" />;
  if (q.isError) return <ErrorState error={q.error} onRetry={() => q.refetch()} />;
  if (!q.data.length) {
    return (
      <div className="ci-layout">
        <div className="ci-section">
          <GatesIntro onCreate={() => setEditing('new')} canWrite={canWrite} />
        </div>
        {editor}
      </div>
    );
  }
  return (
    <div className="ci-layout">
      <header className="product-header">
        <div className="product-title">
          <ShieldCheck size={20} /> Deployment Gates
        </div>
        <nav className="product-tabs">
          <NavLink to="/delivery/deployment-gates" end className={({ isActive }) => `product-tab${isActive ? ' active' : ''}`}>
            Gates
          </NavLink>
          <NavLink to="/delivery/deployment-gates/evaluations" className={({ isActive }) => `product-tab${isActive ? ' active' : ''}`}>
            Evaluations
          </NavLink>
        </nav>
        {canWrite && (
          <div className="ci-header-actions">
            <button className="btn btn-primary" onClick={() => setEditing('new')}>
              <Plus size={14} /> New Gate
            </button>
          </div>
        )}
      </header>
      <div className="ci-section">
        <Outlet context={{ gates: q.data, edit: setEditing } satisfies GatesContext} />
      </div>
      {editor}
    </div>
  );
}

// ================================================================ intro

function GatesIntro({ onCreate, canWrite }: { onCreate: () => void; canWrite: boolean }) {
  const [tool, setTool] = useState<CiToolId | null>(null);
  const learnMore = () => document.getElementById('dg-learn-more')?.scrollIntoView({ behavior: 'smooth' });
  const chosen = CI_TOOLS.find((t) => t.id === tool);
  return (
    <div className="dg-intro">
      <section className="dg-hero">
        <div className="dg-hero-kicker">
          <span className="dg-hero-icon">
            <ShieldCheck size={18} />
          </span>
          Deployment Gates
        </div>
        <h1>Let only healthy releases reach production</h1>
        <p>Your pipeline asks a gate before it deploys: the gate checks your monitors and compares the new version’s errors with the one it replaces, then answers pass or fail.</p>
        <div className="dg-hero-actions">
          {canWrite && (
            <button className="btn dg-btn-white" onClick={onCreate}>
              <Plus size={16} /> Create a gate
            </button>
          )}
          <button className="btn dg-btn-outline" onClick={learnMore}>
            <BookOpen size={16} /> Learn more
          </button>
        </div>
        <GatePreview />
      </section>

      <h3 className="dg-section-title" id="dg-learn-more">
        LEARN MORE
      </h3>
      <div className="dg-features">
        <div className="dg-feature">
          <BellRing size={22} />
          <b>Gate on your monitors</b>
          <p>Pick monitors by tag. A deployment waits while any of them is in alert or has stopped reporting data.</p>
        </div>
        <div className="dg-feature">
          <Activity size={22} />
          <b>Catch faulty deployments</b>
          <p>The new version’s error rate is compared with the previous version’s from APM spans, using the version tag your services send.</p>
        </div>
        <div className="dg-feature">
          <FlaskConical size={22} />
          <b>Try rules in dry run</b>
          <p>Run a gate or a single rule in dry run first: every evaluation is recorded, but nothing is blocked until you switch it on.</p>
        </div>
      </div>

      <h3 className="dg-section-title">GET STARTED</h3>
      <div className="dg-tools">
        {CI_TOOLS.map((t) => (
          <button key={t.id} className={`dg-tool${tool === t.id ? ' active' : ''}`} onClick={() => setTool(tool === t.id ? null : t.id)} aria-expanded={tool === t.id}>
            <span className="dg-tool-logo">{t.id === 'custom' ? <Terminal size={30} /> : <IntegrationLogo logo={t.logo} name={t.label} size={30} />}</span>
            {t.label}
          </button>
        ))}
      </div>
      {chosen && (
        <div className="dg-tool-snippet">
          <p className="muted">{chosen.note}</p>
          <CodeBlock code={ciSnippet(chosen.id, { apiUrl: apiUrl(), service: 'storefront-api', env: 'prod', identifier: 'default' })} filename={chosen.label} />
          <p className="muted">
            Use an application key from <Link to="/organization-settings/api-keys">Organization Settings → API Keys</Link> of a user with the Standard or Admin role. The request answers once every
            rule is evaluated; <code>pass</code> is false only when a rule outside dry run fails.
          </p>
        </div>
      )}
    </div>
  );
}

/** The hero's product preview: our own evaluation list and failure panel with fictional data. */
function GatePreview() {
  const rows: [string, string, string, GateEvaluationStatus, string][] = [
    ['2 min ago', 'storefront-api', 'prod', 'failed', '1.4s'],
    ['26 min ago', 'order-service', 'prod', 'passed', '0.9s'],
    ['1 hour ago', 'billing', 'staging', 'dry_run', '1.1s'],
    ['3 hours ago', 'catalog-service', 'prod', 'passed', '0.7s'],
  ];
  return (
    <div className="dg-preview" aria-hidden>
      <div className="dg-preview-list">
        <table className="ci-table">
          <thead>
            <tr>
              <th>DATE</th>
              <th>SERVICE</th>
              <th>ENV</th>
              <th>STATUS</th>
              <th className="num">DURATION</th>
            </tr>
          </thead>
          <tbody>
            {rows.map(([when, service, env, status, dur]) => (
              <tr key={service}>
                <td>{when}</td>
                <td>{service}</td>
                <td>{env}</td>
                <td>
                  <EvalStatus status={status} />
                </td>
                <td className="num">{dur}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
      <div className="dg-preview-panel">
        <b>Why did the evaluation fail?</b>
        <div className="dg-why-rule">
          <CircleX size={15} className="dg-bad" /> Monitors service:storefront-api env:prod
        </div>
        <div className="dg-preview-monitor">
          <span>p95 latency of storefront-api checkout</span>
          <MonitorStatus status="alert" />
        </div>
        <div className="dg-why-rule">
          <CircleCheck size={15} className="dg-good" /> Faulty deployment detection
        </div>
        <p className="muted">Error rate 0.7% on 2.13.0 vs 0.6% on 2.12.3</p>
      </div>
    </div>
  );
}

// ================================================================ gates

export function GatesTab() {
  const { gates, edit } = useOutletContext<GatesContext>();
  const { canWrite } = useAccess();
  const m = useGateMutations();
  const [text, setText] = useState('');
  const query = useDeferredValue(text).toLowerCase();
  const [evaluated, setEvaluated] = useState<GateEvaluation | null>(null);
  const shown = gates.filter((g) => `${g.service} ${g.env} ${g.identifier}`.toLowerCase().includes(query));
  return (
    <div className="dg-page">
      <div className="ci-search">
        <Search size={16} />
        <input placeholder="Filter by service, environment or identifier" value={text} onChange={(e) => setText(e.target.value)} aria-label="Filter gates" />
      </div>
      <div className="ci-table-wrap">
        <table className="ci-table dg-table">
          <thead>
            <tr>
              <th>SERVICE</th>
              <th>ENVIRONMENT</th>
              <th>IDENTIFIER</th>
              <th>MODE</th>
              <th>RULES</th>
              <th>LAST EVALUATION</th>
              <th />
            </tr>
          </thead>
          <tbody>
            {shown.map((g) => (
              <tr key={g.id} onClick={() => canWrite && edit(g)}>
                <td>
                  <b>{g.service}</b>
                </td>
                <td>{g.env}</td>
                <td className="mono">{g.identifier}</td>
                <td>
                  <span className={`dg-mode${g.dryRun ? ' dry' : ''}`}>{g.dryRun ? 'Dry run' : 'Active'}</span>
                </td>
                <td>
                  <RuleSummary rules={g.rules} />
                </td>
                <td>
                  {g.lastStatus && g.lastEvaluatedAt ? (
                    <span className="dg-last">
                      <EvalStatus status={g.lastStatus} /> {fmtAgo(g.lastEvaluatedAt)}
                    </span>
                  ) : (
                    <span className="muted">Never</span>
                  )}
                </td>
                <td className="dg-actions" onClick={(e) => e.stopPropagation()}>
                  {canWrite && (
                    <>
                      <button
                        className="icon-plain"
                        title="Evaluate now"
                        disabled={m.evaluate.isPending}
                        onClick={() => m.evaluate.mutate({ service: g.service, env: g.env, identifier: g.identifier }, { onSuccess: setEvaluated })}
                      >
                        <Play size={15} />
                      </button>
                      <button className="icon-plain" title="Edit" onClick={() => edit(g)}>
                        <Pencil size={15} />
                      </button>
                      <button
                        className="icon-plain"
                        title="Delete"
                        onClick={() => window.confirm(`Delete the gate of ${g.service} in ${g.env} (${g.identifier})? Pipelines that call it get a 404.`) && m.remove.mutate(g.id)}
                      >
                        <Trash2 size={15} />
                      </button>
                    </>
                  )}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
        {shown.length === 0 && <p className="mon-empty">No gate matches “{text}”.</p>}
      </div>
      {(m.evaluate.error || m.remove.error) && <p className="form-error">{((m.evaluate.error ?? m.remove.error) as Error).message}</p>}
      {evaluated && <EvaluationPanel id={evaluated.id} onClose={() => setEvaluated(null)} />}
    </div>
  );
}

function RuleSummary({ rules }: { rules: GateRule[] }) {
  const monitors = rules.filter((r) => r.type === 'monitor').length;
  const faulty = rules.length - monitors;
  return (
    <span className="dg-rule-summary">
      {rules.length} {rules.length === 1 ? 'rule' : 'rules'}
      {monitors > 0 && (
        <span className="dg-rule-kind" title={`${monitors} monitor ${monitors === 1 ? 'rule' : 'rules'}`}>
          <BellRing size={13} /> {monitors}
        </span>
      )}
      {faulty > 0 && (
        <span className="dg-rule-kind" title="Faulty deployment detection">
          <Activity size={13} /> {faulty}
        </span>
      )}
    </span>
  );
}

export function EvalStatus({ status }: { status: GateEvaluationStatus }) {
  return <span className={`dg-status dg-status-${status}`}>{STATUS_LABEL[status]}</span>;
}

function MonitorStatus({ status }: { status: string | null | undefined }) {
  if (!status) return <span className="muted">Deleted</span>;
  return <span className={`dg-mon dg-mon-${status}`}>{MONITOR_STATUS_LABEL[status] ?? status}</span>;
}

// ================================================================ editor

const newMonitorRule = (service: string, env: string): GateRule => ({
  type: 'monitor',
  name: '',
  query: [service && `service:${service}`, env && `env:${env}`].filter(Boolean).join(' '),
  dryRun: false,
});
const newFaultyRule = (): GateRule => ({ type: 'faulty_deployment', name: '', dryRun: false, windowMinutes: 30, maxErrorRateIncrease: 1, minRequests: 50 });

function GateEditor({ gate, gates, onClose }: { gate: DeploymentGate | null; gates: DeploymentGate[]; onClose: () => void }) {
  const m = useGateMutations();
  const [service, setService] = useState(gate?.service ?? '');
  const [env, setEnv] = useState(gate?.env ?? '');
  const [identifier, setIdentifier] = useState(gate?.identifier ?? 'default');
  const [dryRun, setDryRun] = useState(gate?.dryRun ?? false);
  const [rules, setRules] = useState<GateRule[]>(gate?.rules ?? [newMonitorRule('', ''), newFaultyRule()]);
  const [tool, setTool] = useState<CiToolId>('github');
  const range = useMemo(() => ({ from: Date.now() - 86_400_000, to: Date.now() }), []);
  const services = useServices({ range });
  const serviceNames = [...new Set([...(services.data ?? []).map((s) => s.service), ...gates.map((g) => g.service)])].sort();
  const envs = [...new Set(['prod', 'staging', ...gates.map((g) => g.env)])];
  const patch = (i: number, p: Partial<GateRule>) => setRules((rs) => rs.map((r, j) => (j === i ? { ...r, ...p } : r)));
  const save = () => {
    const input: DeploymentGateInput = { service, env, identifier, dryRun, rules };
    m.save.mutate({ id: gate?.id, input }, { onSuccess: onClose });
  };
  return (
    <Modal onClose={onClose} label={gate ? 'Edit deployment gate' : 'New deployment gate'} width={960} height="auto">
      <form
        className="small-modal dg-editor"
        onSubmit={(e) => {
          e.preventDefault();
          save();
        }}
      >
        <div className="modal-title-row">
          <h3>
            <ShieldCheck size={18} /> {gate ? `Edit gate of ${gate.service}` : 'New Deployment Gate'}
          </h3>
          <button type="button" className="icon-plain" onClick={onClose} title="Close">
            <X size={18} />
          </button>
        </div>

        <h4 className="dg-step">
          <span>1</span> Choose what the gate protects
        </h4>
        <div className="dg-scope">
          <label className="form-label">
            Service
            <input list="dg-services" value={service} onChange={(e) => setService(e.target.value)} placeholder="storefront-api" required maxLength={200} />
            <datalist id="dg-services">
              {serviceNames.map((s) => (
                <option key={s} value={s} />
              ))}
            </datalist>
          </label>
          <label className="form-label">
            Environment
            <input list="dg-envs" value={env} onChange={(e) => setEnv(e.target.value)} placeholder="prod" required maxLength={200} />
            <datalist id="dg-envs">
              {envs.map((s) => (
                <option key={s} value={s} />
              ))}
            </datalist>
          </label>
          <label className="form-label">
            Identifier
            <input
              className="mono"
              value={identifier}
              onChange={(e) => setIdentifier(e.target.value)}
              placeholder="default"
              pattern="[A-Za-z0-9._\-]{1,100}"
              title="Letters, digits, '-', '_' and '.'"
            />
          </label>
        </div>
        <div className="dg-mode-pick" role="radiogroup" aria-label="Mode">
          <button type="button" role="radio" aria-checked={!dryRun} className={!dryRun ? 'active' : ''} onClick={() => setDryRun(false)}>
            <b>Active</b>
            <span>A failing rule stops the deployment.</span>
          </button>
          <button type="button" role="radio" aria-checked={dryRun} className={dryRun ? 'active' : ''} onClick={() => setDryRun(true)}>
            <b>Dry run</b>
            <span>Evaluations are recorded; deployments always go on.</span>
          </button>
        </div>

        <h4 className="dg-step">
          <span>2</span> Add rules
        </h4>
        <div className="dg-rules">
          {rules.map((r, i) => (
            <div key={i} className="dg-rule">
              <div className="dg-rule-head">
                {r.type === 'monitor' ? <BellRing size={16} /> : <Activity size={16} />}
                <b>{r.type === 'monitor' ? 'Monitors' : 'Faulty Deployment Detection'}</b>
                <input className="dg-rule-name" value={r.name ?? ''} onChange={(e) => patch(i, { name: e.target.value })} placeholder="Rule name (optional)" maxLength={200} aria-label="Rule name" />
                <Switch checked={r.dryRun ?? false} onChange={(v) => patch(i, { dryRun: v })} label="Dry run" />
                <button type="button" className="icon-plain" title="Remove rule" onClick={() => setRules((rs) => rs.filter((_, j) => j !== i))}>
                  <Trash2 size={15} />
                </button>
              </div>
              {r.type === 'monitor' ? (
                <label className="form-label">
                  Monitors with these tags (and name words) — fails when any of them is in ALERT or NO DATA
                  <input className="mono" value={r.query ?? ''} onChange={(e) => patch(i, { query: e.target.value })} placeholder="service:storefront-api env:prod" required maxLength={1000} />
                </label>
              ) : (
                <div className="dg-faulty">
                  <label className="form-label">
                    Compare the last
                    <span className="dg-unit">
                      <input type="number" min={1} max={1440} value={r.windowMinutes ?? 30} onChange={(e) => patch(i, { windowMinutes: Number(e.target.value) })} /> minutes
                    </span>
                  </label>
                  <label className="form-label">
                    Fail when the error rate rises by more than
                    <span className="dg-unit">
                      <input type="number" min={0} max={100} step={0.1} value={r.maxErrorRateIncrease ?? 1} onChange={(e) => patch(i, { maxErrorRateIncrease: Number(e.target.value) })} /> points
                    </span>
                  </label>
                  <label className="form-label">
                    Judge the version after
                    <span className="dg-unit">
                      <input type="number" min={0} value={r.minRequests ?? 50} onChange={(e) => patch(i, { minRequests: Number(e.target.value) })} /> requests
                    </span>
                  </label>
                  <p className="muted dg-faulty-hint">
                    The new version’s error rate (entry spans of <code>{service || 'the service'}</code> in <code>{env || 'the environment'}</code> with its <code>version</code>) is compared with the
                    previous version’s.
                  </p>
                </div>
              )}
            </div>
          ))}
          <div className="dg-add-rule">
            <button type="button" className="btn btn-sm" onClick={() => setRules((rs) => [...rs, newMonitorRule(service, env)])}>
              <Plus size={14} /> Monitor rule
            </button>
            <button type="button" className="btn btn-sm" onClick={() => setRules((rs) => [...rs, newFaultyRule()])}>
              <Plus size={14} /> Faulty deployment detection
            </button>
          </div>
        </div>

        <h4 className="dg-step">
          <span>3</span> Call it from your pipeline
        </h4>
        <div className="ff-sdk-seg" role="tablist" aria-label="CI/CD tool">
          {CI_TOOLS.map((t) => (
            <button type="button" key={t.id} role="tab" aria-selected={t.id === tool} className={t.id === tool ? 'active' : ''} onClick={() => setTool(t.id)}>
              {t.label}
            </button>
          ))}
        </div>
        <CodeBlock code={ciSnippet(tool, { apiUrl: apiUrl(), service: service || '<service>', env: env || '<env>', identifier: identifier || 'default' })} />

        {m.save.error && <p className="form-error">{(m.save.error as Error).message}</p>}
        <div className="small-modal-actions">
          <button type="button" className="btn" onClick={onClose}>
            Cancel
          </button>
          <button className="btn btn-primary" disabled={m.save.isPending || rules.length === 0}>
            {gate ? 'Save' : 'Create Gate'}
          </button>
        </div>
      </form>
    </Modal>
  );
}

// ================================================================ evaluations

const STATUS_FILTERS: (GateEvaluationStatus | null)[] = [null, 'failed', 'dry_run', 'passed'];

export function EvaluationsTab() {
  const q = useEvaluations();
  const [text, setText] = useState('');
  const [status, setStatus] = useState<GateEvaluationStatus | null>(null);
  const [open, setOpen] = useState<string | null>(null);
  const query = useDeferredValue(text);
  if (q.isPending) return <Loading label="Loading evaluations…" />;
  if (q.isError) return <ErrorState error={q.error} onRetry={() => q.refetch()} />;
  const shown = filterEvaluations(q.data, query, status);
  return (
    <div className="dg-page">
      <div className="ci-state-tabs">
        {STATUS_FILTERS.map((s) => (
          <button key={s ?? 'all'} className={status === s ? 'active' : ''} onClick={() => setStatus(s)}>
            {s ? STATUS_LABEL[s] : 'All'} <span>{s ? q.data.filter((e) => e.status === s).length : q.data.length}</span>
          </button>
        ))}
      </div>
      <div className="ci-search">
        <Search size={16} />
        <input placeholder="Filter by service, environment, identifier or version" value={text} onChange={(e) => setText(e.target.value)} aria-label="Filter evaluations" />
      </div>
      <div className="ci-table-wrap">
        <table className="ci-table dg-table">
          <thead>
            <tr>
              <th>DATE</th>
              <th>SERVICE</th>
              <th>ENVIRONMENT</th>
              <th>IDENTIFIER</th>
              <th>VERSION</th>
              <th>STATUS</th>
              <th className="num">DURATION</th>
            </tr>
          </thead>
          <tbody>
            {shown.map((e) => (
              <tr key={e.id} onClick={() => setOpen(e.id)} className={open === e.id ? 'selected' : ''}>
                <td title={new Date(e.startedAt).toLocaleString()}>{fmtDate(e.startedAt)}</td>
                <td>
                  <b>{e.service}</b>
                </td>
                <td>{e.env}</td>
                <td className="mono">{e.identifier}</td>
                <td className="mono">{e.version || '—'}</td>
                <td>
                  <EvalStatus status={e.status} />
                </td>
                <td className="num">{fmtDur(e.durationMs)}</td>
              </tr>
            ))}
          </tbody>
        </table>
        {shown.length === 0 && <p className="mon-empty">{q.data.length ? 'No evaluation matches these filters.' : 'No evaluation yet: pipelines evaluate a gate before they deploy.'}</p>}
      </div>
      {open && <EvaluationPanel id={open} onClose={() => setOpen(null)} />}
    </div>
  );
}

/** Side panel of one evaluation: why it failed (monitors then and now), and every rule's outcome. */
function EvaluationPanel({ id, onClose }: { id: string; onClose: () => void }) {
  const q = useEvaluation(id);
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === 'Escape' && onClose();
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [onClose]);
  const e = q.data;
  const failed = (e?.rules ?? []).filter((r) => r.status === 'failed');
  return createPortal(
    <div className="ci-drawer-backdrop" onMouseDown={(ev) => ev.target === ev.currentTarget && onClose()}>
      <aside className="ci-drawer dg-drawer" role="dialog" aria-label="Gate evaluation">
        {q.isPending ? (
          <Loading label="Loading evaluation…" />
        ) : q.isError ? (
          <ErrorState error={q.error} onRetry={() => q.refetch()} />
        ) : (
          e && (
            <>
              <header className={`ci-drawer-head dg-drawer-head dg-head-${e.status}`}>
                <div className="ci-drawer-title">
                  <EvalStatus status={e.status} />
                  <h2>
                    {e.service} · {e.env}
                  </h2>
                  <span className="mono muted">{e.identifier}</span>
                  <button className="icon-plain ci-drawer-close" onClick={onClose} title="Close">
                    <X size={20} />
                  </button>
                </div>
                <div className="ci-drawer-meta">
                  <span>Version {e.version || 'not sent'}</span>
                  <span>{fmtDate(e.startedAt)}</span>
                  <span>Took {fmtDur(e.durationMs)}</span>
                  <span>Requested by {e.requestedBy || '—'}</span>
                  <span>{e.pass ? 'The deployment could go on' : 'The deployment was stopped'}</span>
                </div>
              </header>
              <div className="ci-drawer-body dg-drawer-body">
                {failed.length > 0 && (
                  <section className="dg-why">
                    <h3>Why did the evaluation {e.status === 'failed' ? 'fail' : 'fail in dry run'}?</h3>
                    {failed.map((r, i) => (
                      <FailedRule key={i} rule={r} />
                    ))}
                  </section>
                )}
                <section>
                  <h3>Rules</h3>
                  <ul className="dg-results">
                    {e.rules.map((r, i) => (
                      <li key={i}>
                        {r.status === 'passed' ? <CircleCheck size={16} className="dg-good" /> : <CircleX size={16} className="dg-bad" />}
                        <div>
                          <b>{r.name}</b> {r.dryRun && <span className="dg-mode dry">Dry run</span>}
                          <div className="muted">{r.reason}</div>
                        </div>
                      </li>
                    ))}
                  </ul>
                </section>
              </div>
            </>
          )
        )}
      </aside>
    </div>,
    document.body,
  );
}

function FailedRule({ rule }: { rule: GateRuleResult }) {
  return (
    <div className="dg-why-card">
      <div className="dg-why-rule">
        <CircleX size={15} className="dg-bad" /> <b>{rule.name}</b> {rule.dryRun && <span className="dg-mode dry">Dry run</span>}
      </div>
      <p className="muted">{rule.reason}</p>
      {rule.type === 'monitor' && (rule.monitors ?? []).length > 0 && (
        <table className="ci-table">
          <thead>
            <tr>
              <th>MONITOR</th>
              <th>AT EVALUATION</th>
              <th>NOW</th>
            </tr>
          </thead>
          <tbody>
            {(rule.monitors ?? []).map((m) => (
              <tr key={m.id}>
                <td>
                  <Link to={`/monitors/${m.id}`}>{m.name}</Link>
                </td>
                <td>
                  <MonitorStatus status={m.status} />
                </td>
                <td>
                  <MonitorStatus status={m.currentStatus} />
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
      {rule.type === 'faulty_deployment' && rule.errorRate != null && (
        <div className="dg-compare">
          <div>
            <span className="muted">{rule.version}</span>
            <b className="dg-bad">{((rule.errorRate ?? 0) * 100).toFixed(1)}%</b>
            <span className="muted">{rule.requests ?? 0} requests</span>
          </div>
          {rule.previousVersion && (
            <div>
              <span className="muted">{rule.previousVersion} (previous)</span>
              <b>{((rule.previousErrorRate ?? 0) * 100).toFixed(1)}%</b>
            </div>
          )}
        </div>
      )}
    </div>
  );
}
