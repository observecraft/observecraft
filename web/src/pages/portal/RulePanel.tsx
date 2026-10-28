import { useEffect, useMemo, useState } from 'react';
import { createPortal } from 'react-dom';
import { ChevronDown, ChevronRight, Trash2, X } from 'lucide-react';
import type { ScorecardOutcomeState, ScorecardRule, ScorecardsOverview } from '../../api/scorecards/client';
import { countStates, LEVELS, summarize, type SummaryBy } from '../../api/scorecards/logic';
import { CodeBlock } from '../../components/code/CodeBlock';
import { timeAgo } from '../../lib/format';
import { fmtPct, LevelBadge, PassBar, StateIcon } from './common';
import { useDeleteScorecardRule, useSetScorecardOutcomes, useUpdateScorecardRule } from './queries';

const SUMMARY_BY: { id: SummaryBy; label: string }[] = [
  { id: 'entity', label: 'By Entity' },
  { id: 'kind', label: 'By Kind' },
  { id: 'team', label: 'By Team' },
];

/** A rule's side panel: definition (editable), score summary by entity, kind or team, and for custom rules, reported outcomes. */
export function RulePanel({ data, rule, onClose, onService }: { data: ScorecardsOverview; rule: ScorecardRule; onClose: () => void; onService: (s: string) => void }) {
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === 'Escape' && onClose();
    globalThis.addEventListener('keydown', onKey);
    return () => globalThis.removeEventListener('keydown', onKey);
  }, [onClose]);
  const card = data.scorecards.find((c) => c.id === rule.scorecardId);
  const remove = useDeleteScorecardRule();

  return createPortal(
    <div className="sc-drawer-backdrop" onMouseDown={(e) => e.target === e.currentTarget && onClose()}>
      <aside className="sc-drawer" role="dialog" aria-label={`Rule ${rule.name}`}>
        <header className="sc-drawer-head">
          <LevelBadge level={rule.level} />
          <div className="sc-drawer-title">
            <h2>{rule.name}</h2>
            <span className="sc-muted">
              {card?.name ?? 'Scorecard'} · {rule.builtin ? 'Built-in rule' : 'Custom rule'}
            </span>
          </div>
          <div className="sc-drawer-actions">
            {!rule.builtin && (
              <button
                className="btn btn-sm"
                disabled={remove.isPending}
                onClick={() => {
                  if (confirm(`Delete the rule "${rule.name}" and its outcomes?`)) remove.mutate(rule.id, { onSuccess: onClose });
                }}
              >
                <Trash2 size={14} /> Delete
              </button>
            )}
            <button className="sc-icon-btn" aria-label="Close" onClick={onClose}>
              <X size={18} />
            </button>
          </div>
        </header>
        <div className="sc-drawer-body">
          <RuleDefinition key={rule.id} rule={rule} />
          <ScoreSummary data={data} rule={rule} onService={onService} />
          {!rule.builtin && <ReportOutcome data={data} rule={rule} />}
        </div>
      </aside>
    </div>,
    document.body,
  );
}

function RuleDefinition({ rule }: { rule: ScorecardRule }) {
  const [name, setName] = useState(rule.name);
  const [description, setDescription] = useState(rule.description);
  const [level, setLevel] = useState(rule.level);
  const [scope, setScope] = useState(rule.scope);
  const update = useUpdateScorecardRule();
  const dirty = level !== rule.level || scope !== rule.scope || (!rule.builtin && (name !== rule.name || description !== rule.description));
  const save = () => update.mutate({ id: rule.id, input: rule.builtin ? { level, scope } : { level, scope, name, description } });

  return (
    <section className="sc-panel-section">
      <h3>Definition</h3>
      {rule.builtin ? <p className="sc-rule-desc">{rule.description}</p> : null}
      <div className="sc-form">
        {!rule.builtin && (
          <>
            <label className="sc-field">
              <span>Name</span>
              <input className="sc-input" value={name} onChange={(e) => setName(e.target.value)} />
            </label>
            <label className="sc-field">
              <span>Description</span>
              <textarea className="sc-input" rows={2} value={description} onChange={(e) => setDescription(e.target.value)} />
            </label>
          </>
        )}
        <div className="sc-field">
          <span>Level</span>
          <div className="btn-group">
            {LEVELS.map((l) => (
              <button key={l} className={`btn btn-sm${l === level ? ' btn-selected' : ''}`} onClick={() => setLevel(l)}>
                Level {l}
              </button>
            ))}
          </div>
        </div>
        <label className="sc-field">
          <span>Scope</span>
          <input className="sc-input mono" value={scope} onChange={(e) => setScope(e.target.value)} placeholder="All services (e.g. team:checkout -service:legacy-*)" />
        </label>
        <div className="sc-field-row">
          <label className="sc-inline">
            <button
              className={`switch${rule.enabled ? ' on' : ''}`}
              role="switch"
              aria-checked={rule.enabled}
              aria-label="Enabled"
              disabled={update.isPending}
              onClick={() => update.mutate({ id: rule.id, input: { enabled: !rule.enabled } })}
            />
            {rule.enabled ? 'Enabled' : 'Disabled'}
          </label>
          <button className="btn btn-primary btn-sm" disabled={!dirty || update.isPending} onClick={save}>
            Save
          </button>
        </div>
        {update.isError && <div className="form-error">{(update.error as Error).message}</div>}
      </div>
    </section>
  );
}

function ScoreSummary({ data, rule, onService }: { data: ScorecardsOverview; rule: ScorecardRule; onService: (s: string) => void }) {
  const [by, setBy] = useState<SummaryBy>('entity');
  const [open, setOpen] = useState<Set<string>>(new Set());
  const outcomes = useMemo(() => data.outcomes.filter((o) => o.ruleId === rule.id), [data.outcomes, rule.id]);
  const groups = useMemo(() => summarize(outcomes, data.services, by), [outcomes, data.services, by]);
  const total = countStates(outcomes);

  return (
    <section className="sc-panel-section">
      <div className="sc-panel-head">
        <h3>Score Summary</h3>
        <div className="btn-group">
          {SUMMARY_BY.map((s) => (
            <button key={s.id} className={`btn btn-sm${s.id === by ? ' btn-selected' : ''}`} onClick={() => setBy(s.id)}>
              {s.label}
            </button>
          ))}
        </div>
      </div>
      <p className="sc-note">
        Outcomes are evaluated every hour and when you click Evaluate now{data.evaluatedAt ? `; last evaluated ${timeAgo(data.evaluatedAt)}` : ''}.
        {rule.builtin ? '' : ' Custom rule outcomes come from what users and scripts report.'}
      </p>
      <div className="sc-summary-total">
        <PassBar counts={total} wide />
        <span>
          <b>{fmtPct(total.passPct)}</b> passing · {total.pass} pass · {total.fail} fail · {total.skip} skip
        </span>
      </div>
      {!rule.enabled ? (
        <div className="sc-muted">The rule is disabled.</div>
      ) : !groups.length ? (
        <div className="sc-muted">No service is in this rule's scope.</div>
      ) : by === 'entity' ? (
        <ul className="sc-outcome-list">
          {groups
            .flatMap((g) => g.outcomes)
            .map((o) => (
              <li key={o.service}>
                <StateIcon state={o.state} />
                <button className="sc-link" onClick={() => onService(o.service)}>
                  {o.service}
                </button>
                <span className="sc-outcome-reason">{o.reason}</span>
              </li>
            ))}
        </ul>
      ) : (
        <ul className="sc-group-list">
          {groups.map((g) => {
            const isOpen = open.has(g.key);
            return (
              <li key={g.key}>
                <button
                  className="sc-group-head"
                  onClick={() =>
                    setOpen((p) => {
                      const next = new Set(p);
                      if (next.has(g.key)) next.delete(g.key);
                      else next.add(g.key);
                      return next;
                    })
                  }
                >
                  {isOpen ? <ChevronDown size={15} /> : <ChevronRight size={15} />}
                  <span className="sc-group-name">{g.label}</span>
                  <PassBar counts={g} />
                  <span className="sc-pass-pct">{fmtPct(g.passPct)}</span>
                  <span className="sc-pass-of">
                    {g.pass}/{g.total}
                  </span>
                </button>
                {isOpen && (
                  <ul className="sc-outcome-list nested">
                    {g.outcomes.map((o) => (
                      <li key={o.service}>
                        <StateIcon state={o.state} />
                        <button className="sc-link" onClick={() => onService(o.service)}>
                          {o.service}
                        </button>
                        <span className="sc-outcome-reason">{o.reason}</span>
                      </li>
                    ))}
                  </ul>
                )}
              </li>
            );
          })}
        </ul>
      )}
    </section>
  );
}

function ReportOutcome({ data, rule }: { data: ScorecardsOverview; rule: ScorecardRule }) {
  const inScope = data.outcomes.filter((o) => o.ruleId === rule.id).map((o) => o.service);
  const [service, setService] = useState(inScope[0] ?? '');
  const [state, setState] = useState<ScorecardOutcomeState>('pass');
  const [remarks, setRemarks] = useState('');
  const report = useSetScorecardOutcomes();
  const snippet = `curl -X POST "$OBSERVECRAFT_URL/api/v1/scorecards/rules/${rule.id}/outcomes" \\
  -H "Authorization: Bearer $OBSERVECRAFT_APP_KEY" -H "Content-Type: application/json" \\
  -d '{"outcomes": [{"service": "${service || 'order-service'}", "state": "pass", "remarks": "Load test passed"}]}'`;

  return (
    <section className="sc-panel-section">
      <h3>Report an outcome</h3>
      {!inScope.length ? (
        <p className="sc-muted">No service is in this rule's scope.</p>
      ) : (
        <div className="sc-form sc-report">
          <select className="sc-input" value={service} onChange={(e) => setService(e.target.value)} aria-label="Service">
            {inScope.map((s) => (
              <option key={s}>{s}</option>
            ))}
          </select>
          <div className="btn-group">
            {(['pass', 'fail', 'skip'] as const).map((s) => (
              <button key={s} className={`btn btn-sm${s === state ? ' btn-selected' : ''}`} onClick={() => setState(s)}>
                <StateIcon state={s} size={13} /> {s[0].toUpperCase() + s.slice(1)}
              </button>
            ))}
          </div>
          <input className="sc-input" value={remarks} onChange={(e) => setRemarks(e.target.value)} placeholder="Remarks (shown as the reason)" />
          <button
            className="btn btn-primary btn-sm"
            disabled={!service || report.isPending}
            onClick={() => report.mutate({ id: rule.id, input: { outcomes: [{ service, state, remarks }] } }, { onSuccess: () => setRemarks('') })}
          >
            Report
          </button>
        </div>
      )}
      {report.isError && <div className="form-error">{(report.error as Error).message}</div>}
      <p className="sc-note">Scripts and CI jobs report outcomes for many services at once with an application key:</p>
      <CodeBlock code={snippet} />
    </section>
  );
}
