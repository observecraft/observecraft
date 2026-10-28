import { Fragment, useMemo, useState } from 'react';
import { useSearchParams } from 'react-router-dom';
import { ChevronDown, ChevronRight, Plus, RefreshCw, Search } from 'lucide-react';
import type { ScorecardRule, ScorecardsOverview } from '../../api/scorecards/client';
import { countStates, levelCounts, teamRollup } from '../../api/scorecards/logic';
import { ErrorState, Loading } from '../../components/ui/AsyncState';
import { timeAgo } from '../../lib/format';
import { fmtPct, LevelBadge, LevelBar, PassBar, StateIcon } from './common';
import { RuleModal } from './RuleModal';
import { RulePanel } from './RulePanel';
import { ScorecardsIntro } from './ScorecardsIntro';
import { ServicePanel } from './ServicePanel';
import { useEvaluateScorecards, useScorecards, useUpdateScorecardRule } from './queries';

type View = 'rules' | 'services' | 'teams';
const VIEWS: { id: View; label: string }[] = [
  { id: 'rules', label: 'Rules' },
  { id: 'services', label: 'Services' },
  { id: 'teams', label: 'Teams' },
];

/** Developer Portal → Scorecards: the first-run page until set up, then rules, services and teams. */
export function ScorecardsPage() {
  const q = useScorecards();
  if (q.isPending) return <Loading label="Loading scorecards…" />;
  if (q.isError) return <ErrorState error={q.error} onRetry={() => q.refetch()} />;
  return q.data.setUp ? <Board data={q.data} /> : <ScorecardsIntro recommended={q.data.recommended} />;
}

function Board({ data }: { data: ScorecardsOverview }) {
  const [params, setParams] = useSearchParams();
  const view = (VIEWS.find((v) => v.id === params.get('view'))?.id ?? 'rules') as View;
  const ruleId = params.get('rule');
  const service = params.get('service');
  const [search, setSearch] = useState('');
  const [creating, setCreating] = useState(false);
  const evaluate = useEvaluateScorecards();
  const set = (patch: Record<string, string | null>) =>
    setParams((p) => {
      const next = new URLSearchParams(p);
      for (const [k, v] of Object.entries(patch)) {
        if (v === null) next.delete(k);
        else next.set(k, v);
      }
      return next;
    });
  const rule = data.rules.find((r) => r.id === ruleId);
  const counts = levelCounts(data.services);

  return (
    <div className="sc-board">
      <div className="sc-summary">
        <div className="sc-summary-stat">
          <span className="sc-summary-value">{data.services.length}</span>
          <span className="sc-summary-label">Services</span>
        </div>
        {counts.map((n, l) => (
          <div key={l} className="sc-summary-stat">
            <span className="sc-summary-value">
              <LevelBadge level={l} /> {n}
            </span>
            <span className="sc-summary-label">{l ? `At level ${l}` : 'Below level 1'}</span>
          </div>
        ))}
        <div className="sc-summary-eval">
          <span title={data.evaluatedAt ? new Date(data.evaluatedAt).toLocaleString() : undefined}>
            {data.evaluatedAt ? `Evaluated ${timeAgo(data.evaluatedAt)}` : 'Not evaluated yet'}
            {data.nextEvaluationAt ? ` · next evaluation ${new Date(data.nextEvaluationAt).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })}` : ''}
          </span>
          <button className="btn" disabled={evaluate.isPending} onClick={() => evaluate.mutate()}>
            <RefreshCw size={14} className={evaluate.isPending ? 'sc-spin' : undefined} /> {evaluate.isPending ? 'Evaluating…' : 'Evaluate now'}
          </button>
          <button className="btn btn-primary" onClick={() => setCreating(true)}>
            <Plus size={15} /> New Rule
          </button>
        </div>
      </div>
      {evaluate.isError && <div className="form-error sc-board-error">{(evaluate.error as Error).message}</div>}

      <div className="sc-toolbar">
        <div className="btn-group">
          {VIEWS.map((v) => (
            <button key={v.id} className={`btn btn-sm${v.id === view ? ' btn-selected' : ''}`} onClick={() => set({ view: v.id === 'rules' ? null : v.id })}>
              {v.label}
            </button>
          ))}
        </div>
        <label className="sc-search">
          <Search size={14} />
          <input value={search} onChange={(e) => setSearch(e.target.value)} placeholder={view === 'teams' ? 'Search teams' : view === 'services' ? 'Search services or teams' : 'Search rules'} />
        </label>
      </div>

      {view === 'rules' && <RulesView data={data} search={search} onRule={(id) => set({ rule: id })} onService={(s) => set({ service: s })} />}
      {view === 'services' && <ServicesView data={data} search={search} onService={(s) => set({ service: s })} />}
      {view === 'teams' && <TeamsView data={data} search={search} onTeam={(t) => (set({ view: 'services' }), setSearch(t))} />}

      {rule && <RulePanel data={data} rule={rule} onClose={() => set({ rule: null })} onService={(s) => set({ rule: null, service: s })} />}
      {service && <ServicePanel data={data} service={service} onClose={() => set({ service: null })} onRule={(id) => set({ service: null, rule: id })} />}
      {creating && <RuleModal data={data} onClose={() => setCreating(false)} />}
    </div>
  );
}

function RulesView({ data, search, onRule, onService }: { data: ScorecardsOverview; search: string; onRule: (id: string) => void; onService: (s: string) => void }) {
  const [collapsed, setCollapsed] = useState<Set<string>>(new Set());
  const [expanded, setExpanded] = useState<Set<string>>(new Set());
  const update = useUpdateScorecardRule();
  const needle = search.trim().toLowerCase();
  const byRule = useMemo(() => {
    const m = new Map<string, ScorecardsOverview['outcomes']>();
    for (const o of data.outcomes) m.set(o.ruleId, [...(m.get(o.ruleId) ?? []), o]);
    return m;
  }, [data.outcomes]);
  const flip = (set: Set<string>, key: string) => {
    const next = new Set(set);
    if (next.has(key)) next.delete(key);
    else next.add(key);
    return next;
  };

  return (
    <div className="sc-cards">
      {data.scorecards.map((card) => {
        const rules = data.rules.filter((r) => r.scorecardId === card.id && (!needle || r.name.toLowerCase().includes(needle)));
        if (!rules.length) return null;
        const all = rules.filter((r) => r.enabled).flatMap((r) => byRule.get(r.id) ?? []);
        const open = !collapsed.has(card.id);
        return (
          <section key={card.id} className="sc-card">
            <button className="sc-card-head" onClick={() => setCollapsed(flip(collapsed, card.id))} aria-expanded={open}>
              {open ? <ChevronDown size={16} /> : <ChevronRight size={16} />}
              <span className="sc-card-name">{card.name}</span>
              <span className="sc-card-meta">
                {rules.length} {rules.length === 1 ? 'rule' : 'rules'}
              </span>
              <span className="sc-card-score">
                <PassBar counts={countStates(all)} /> {fmtPct(countStates(all).passPct)} passing
              </span>
            </button>
            {open && (
              <table className="sc-table sc-rules-table">
                <thead>
                  <tr>
                    <th className="sc-col-expand" />
                    <th>Rule</th>
                    <th className="sc-col-level">Level</th>
                    <th className="sc-col-pass">Pass rate</th>
                    <th>Scope</th>
                    <th className="sc-col-enabled">Enabled</th>
                  </tr>
                </thead>
                <tbody>
                  {rules.map((r) => {
                    const outcomes = byRule.get(r.id) ?? [];
                    const c = countStates(outcomes);
                    const isOpen = expanded.has(r.id);
                    return (
                      <Fragment key={r.id}>
                        <tr className={`sc-rule-row${r.enabled ? '' : ' disabled'}`}>
                          <td className="sc-col-expand">
                            <button className="sc-icon-btn" aria-label={isOpen ? 'Collapse' : 'Expand'} onClick={() => setExpanded(flip(expanded, r.id))}>
                              {isOpen ? <ChevronDown size={15} /> : <ChevronRight size={15} />}
                            </button>
                          </td>
                          <td>
                            <button className="sc-link" onClick={() => onRule(r.id)}>
                              {r.name}
                            </button>
                            {!r.builtin && <span className="sc-custom-tag">Custom</span>}
                          </td>
                          <td className="sc-col-level">
                            <LevelBadge level={r.level} />
                          </td>
                          <td className="sc-col-pass">
                            {r.enabled ? (
                              <span className="sc-pass-cell">
                                <PassBar counts={c} />
                                <span className="sc-pass-pct">{fmtPct(c.passPct)}</span>
                                <span className="sc-pass-of">
                                  {c.pass}/{c.total}
                                </span>
                              </span>
                            ) : (
                              <span className="sc-muted">Disabled</span>
                            )}
                          </td>
                          <td>{r.scope ? <code className="sc-scope">{r.scope}</code> : <span className="sc-muted">All services</span>}</td>
                          <td className="sc-col-enabled">
                            <button
                              className={`switch${r.enabled ? ' on' : ''}`}
                              role="switch"
                              aria-checked={r.enabled}
                              aria-label={`${r.enabled ? 'Disable' : 'Enable'} ${r.name}`}
                              disabled={update.isPending}
                              onClick={() => update.mutate({ id: r.id, input: { enabled: !r.enabled } })}
                            />
                          </td>
                        </tr>
                        {isOpen && (
                          <tr className="sc-rule-detail">
                            <td />
                            <td colSpan={5}>
                              <RuleOutcomes rule={r} outcomes={outcomes} onService={onService} />
                            </td>
                          </tr>
                        )}
                      </Fragment>
                    );
                  })}
                </tbody>
              </table>
            )}
          </section>
        );
      })}
    </div>
  );
}

/** A rule's failing (then skipped, then passing) services, inline under its row. */
function RuleOutcomes({ rule, outcomes, onService }: { rule: ScorecardRule; outcomes: ScorecardsOverview['outcomes']; onService: (s: string) => void }) {
  const order = { fail: 0, skip: 1, pass: 2 };
  const shown = [...outcomes].sort((a, b) => order[a.state] - order[b.state] || a.service.localeCompare(b.service));
  if (!rule.enabled) return <div className="sc-muted">Enable the rule to evaluate it.</div>;
  if (!shown.length) return <div className="sc-muted">No service is in this rule's scope.</div>;
  return (
    <>
      <p className="sc-rule-desc">{rule.description}</p>
      <ul className="sc-outcome-list">
        {shown.map((o) => (
          <li key={o.service}>
            <StateIcon state={o.state} />
            <button className="sc-link" onClick={() => onService(o.service)}>
              {o.service}
            </button>
            <span className="sc-outcome-reason">{o.reason}</span>
          </li>
        ))}
      </ul>
    </>
  );
}

function ServicesView({ data, search, onService }: { data: ScorecardsOverview; search: string; onService: (s: string) => void }) {
  const needle = search.trim().toLowerCase();
  const rows = data.services.filter((s) => !needle || s.service.toLowerCase().includes(needle) || s.team.toLowerCase().includes(needle));
  if (!rows.length) return <div className="sc-empty">No service matches.</div>;
  return (
    <table className="sc-table sc-services-table">
      <thead>
        <tr>
          <th>Service</th>
          <th>Team</th>
          <th className="sc-col-level">Level</th>
          {data.scorecards.map((c) => (
            <th key={c.id} className="sc-col-card" title={c.name}>
              {c.name}
            </th>
          ))}
          <th className="sc-col-pass">Score</th>
          <th className="sc-col-num">Pass</th>
          <th className="sc-col-num">Fail</th>
          <th className="sc-col-num">Skip</th>
        </tr>
      </thead>
      <tbody>
        {rows.map((s) => (
          <tr key={s.service} onClick={() => onService(s.service)} className="sc-clickable">
            <td>
              <span className="sc-service-name">{s.service}</span>
              {!s.instrumented && (
                <span className="sc-custom-tag" title="Declared in metadata; no spans in the past 7 days">
                  No spans
                </span>
              )}
            </td>
            <td>{s.team || <span className="sc-muted">—</span>}</td>
            <td className="sc-col-level">
              <LevelBadge level={s.level} />
            </td>
            {data.scorecards.map((c) => (
              <td key={c.id} className="sc-col-card">
                <LevelBadge level={s.levels[c.id] ?? 0} />
              </td>
            ))}
            <td className="sc-col-pass">
              <span className="sc-pass-cell">
                <PassBar counts={{ pass: s.pass, fail: s.fail, skip: s.skip, total: s.pass + s.fail + s.skip, passPct: s.score ?? null }} />
                <span className="sc-pass-pct">{fmtPct(s.score)}</span>
              </span>
            </td>
            <td className="sc-col-num">{s.pass}</td>
            <td className="sc-col-num">{s.fail}</td>
            <td className="sc-col-num">{s.skip}</td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}

function TeamsView({ data, search, onTeam }: { data: ScorecardsOverview; search: string; onTeam: (team: string) => void }) {
  const needle = search.trim().toLowerCase();
  const rows = teamRollup(data.services).filter((t) => !needle || t.team.toLowerCase().includes(needle));
  if (!rows.length) return <div className="sc-empty">No team matches.</div>;
  return (
    <table className="sc-table sc-teams-table">
      <thead>
        <tr>
          <th>Team</th>
          <th className="sc-col-num">Services</th>
          <th className="sc-col-pass">Average score</th>
          <th>Services by level</th>
        </tr>
      </thead>
      <tbody>
        {rows.map((t) => (
          <tr key={t.team || '-'} className="sc-clickable" onClick={() => t.team && onTeam(t.team)}>
            <td>{t.team || <span className="sc-muted">No team</span>}</td>
            <td className="sc-col-num">{t.services}</td>
            <td className="sc-col-pass">
              <span className="sc-pass-cell">
                <span className="sc-meter">
                  <span style={{ width: `${t.avgScore ?? 0}%` }} />
                </span>
                <span className="sc-pass-pct">{fmtPct(t.avgScore)}</span>
              </span>
            </td>
            <td>
              <span className="sc-level-dist">
                <LevelBar levels={t.levels} />
                <span className="sc-level-legend">
                  {t.levels
                    .map((n, l) => (n ? `L${l} ${n}` : null))
                    .filter(Boolean)
                    .join(' · ')}
                </span>
              </span>
            </td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}
