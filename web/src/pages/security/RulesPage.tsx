import { useState } from 'react';
import { Link, useNavigate, useParams, useSearchParams } from 'react-router-dom';
import { ArrowLeft, Copy, Plus, Search, Trash2, X } from 'lucide-react';
import type { RuleCase, RuleQuery, SecurityRule, SecurityRuleInput } from '../../api/security/client';
import { Select } from '../../components/ui/Dropdown';
import { Empty, ErrorState, Loading } from '../../components/ui/AsyncState';
import { timeAgo } from '../../lib/format';
import { SEVERITY_LABEL, SeverityDot } from './common';
import { useDeleteRule, useRuleEnabled, useSaveRule, useSecurityRule, useSecurityRules } from './queries';

const SEV_ORDER = ['info', 'low', 'medium', 'high', 'critical'];
const maxSeverity = (cases: RuleCase[]) => [...cases.map((c) => c.status)].sort((a, b) => SEV_ORDER.indexOf(b) - SEV_ORDER.indexOf(a))[0] ?? 'info';

/** Cloud SIEM → Detection Rules: the content packs' default rules and your own. */
export function RulesPage({ ruleType }: { ruleType?: string } = {}) {
  const q = useSecurityRules();
  const toggle = useRuleEnabled();
  const [search, setSearch] = useState('');
  const [kind, setKind] = useState('');
  const [state, setState] = useState('');
  if (q.isLoading) return <Loading />;
  if (q.isError) return <ErrorState error={q.error} onRetry={() => q.refetch()} />;
  const rules = q.data!.filter(
    (r) =>
      (!ruleType || r.type === ruleType) &&
      (!kind || (kind === 'custom' ? !r.isDefault : r.pack === kind)) &&
      (!state || (state === 'on' ? r.isEnabled : !r.isEnabled)) &&
      (!search.trim() || `${r.name} ${r.tags.join(' ')} ${r.queries.map((x) => x.query).join(' ')}`.toLowerCase().includes(search.trim().toLowerCase())),
  );
  const packs = [...new Map(q.data!.filter((r) => r.pack).map((r) => [r.pack!, r.packName ?? r.pack!])).entries()];
  return (
    <div className="sec-page">
      <div className="sec-explorer-top">
        <h2 className="sec-h">Detection Rules</h2>
        <span className="syn-grow" />
        <Link className="btn btn-primary" to="/security/siem/rules/new">
          <Plus size={14} /> New Rule
        </Link>
      </div>
      <div className="sec-filters">
        <label className="sec-search">
          <Search size={14} />
          <input placeholder="Search rules by name, tag or query" value={search} onChange={(e) => setSearch(e.target.value)} />
        </label>
        <Select
          label="Source"
          value={kind}
          onChange={setKind}
          options={[{ value: '', label: 'All' }, { value: 'custom', label: 'Custom rules' }, ...packs.map(([v, l]) => ({ value: v, label: l }))]}
        />
        <Select
          label="State"
          value={state}
          onChange={setState}
          options={[
            { value: '', label: 'All' },
            { value: 'on', label: 'Enabled' },
            { value: 'off', label: 'Disabled' },
          ]}
        />
      </div>
      {!rules.length ? (
        <Empty>
          <b>No rules match.</b>
        </Empty>
      ) : (
        <div className="mon-table-wrap">
          <table className="mon-table syn-table sec-rules">
            <thead>
              <tr>
                <th>ENABLED</th>
                <th>RULE</th>
                <th>SEVERITY</th>
                <th>SOURCE</th>
                <th className="num">SIGNALS (7D)</th>
                <th>LAST SIGNAL</th>
              </tr>
            </thead>
            <tbody>
              {rules.map((r) => (
                <tr key={r.id}>
                  <td>
                    <input type="checkbox" className="sec-switch" checked={r.isEnabled} onChange={(e) => toggle.mutate({ id: r.id, enabled: e.target.checked })} aria-label={`Enable ${r.name}`} />
                  </td>
                  <td>
                    <Link to={`/security/siem/rules/${r.id}`}>
                      <b>{r.name}</b>
                    </Link>
                    <div className="sec-tags">
                      <span className="tag-chip">{r.type.replace(/_/g, ' ')}</span>
                      {r.notEvaluated && (
                        <span className="tag-chip sec-warn" title={r.notEvaluated}>
                          not evaluated
                        </span>
                      )}
                      {r.tags
                        .filter((t) => t.startsWith('tactic:'))
                        .map((t) => (
                          <span key={t} className="tag-chip">
                            {t}
                          </span>
                        ))}
                    </div>
                  </td>
                  <td className="nowrap">
                    <SeverityDot severity={maxSeverity(r.cases)} /> {SEVERITY_LABEL[maxSeverity(r.cases)]}
                  </td>
                  <td>{r.packName ?? <span className="muted">Custom</span>}</td>
                  <td className="num">{r.signals7d ? <Link to={`/security/siem/signals?q=${encodeURIComponent(`@workflow.rule.id:${r.id}`)}`}>{r.signals7d}</Link> : 0}</td>
                  <td className="nowrap muted">{r.lastSignalAt ? timeAgo(r.lastSignalAt) : '—'}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
    </div>
  );
}

const WINDOWS = [60, 300, 600, 900, 1800, 3600, 7200, 10800, 21600, 43200, 86400];
const KEEP_ALIVES = [0, 60, 300, 600, 900, 1800, 3600, 7200, 10800, 21600];
const MAX_DURATIONS = [0, 60, 300, 600, 900, 1800, 3600, 7200, 10800, 21600, 43200, 86400];
const label = (s: number) => (s === 0 ? 'none' : s < 3600 ? `${s / 60} minutes` : `${s / 3600} hour${s === 3600 ? '' : 's'}`);

interface Draft {
  name: string;
  message: string;
  isEnabled: boolean;
  queries: RuleQuery[];
  cases: RuleCase[];
  window: number;
  keepAlive: number;
  maxDuration: number;
  tags: string;
  hasExtendedTitle: boolean;
}

function toDraft(r?: SecurityRule, clone = false): Draft {
  const o = (r?.options ?? {}) as unknown as Record<string, number>;
  return {
    name: r ? (clone ? `${r.name} (copy)` : r.name) : '',
    message: r?.message ?? '',
    isEnabled: r?.isEnabled ?? true,
    queries: r?.queries.map((q) => ({ ...q })) ?? [{ query: '', groupByFields: [], distinctFields: [], aggregation: 'count', name: '', hasOptionalGroupByFields: false }],
    cases: r?.cases.map((c) => ({ ...c })) ?? [{ name: '', status: 'medium', condition: 'a > 0', notifications: [] }],
    window: o.evaluationWindow ?? 300,
    keepAlive: o.keepAlive ?? 3600,
    maxDuration: o.maxSignalDuration ?? 86400,
    tags: (r?.tags ?? []).join(', '),
    hasExtendedTitle: r?.hasExtendedTitle ?? true,
  };
}

function toInput(d: Draft): SecurityRuleInput {
  return {
    name: d.name,
    message: d.message,
    type: 'log_detection',
    isEnabled: d.isEnabled,
    queries: d.queries,
    cases: d.cases,
    options: { detectionMethod: 'threshold', evaluationWindow: d.window, keepAlive: d.keepAlive, maxSignalDuration: d.maxDuration } as unknown as SecurityRuleInput['options'],
    filters: [],
    tags: d.tags
      .split(',')
      .map((t) => t.trim())
      .filter(Boolean),
    hasExtendedTitle: d.hasExtendedTitle,
  };
}

/** A detection rule: create, change, clone (`?clone=`), or look at a default rule. */
export function RuleEditorPage() {
  const { id } = useParams();
  const isNew = !id || id === 'new';
  const [params] = useSearchParams();
  const cloneOf = params.get('clone') ?? undefined;
  const rule = useSecurityRule(isNew ? cloneOf : id);
  if (!isNew && rule.isLoading) return <Loading />;
  if (rule.isError) return <ErrorState error={rule.error} />;
  if (cloneOf && rule.isLoading) return <Loading />;
  return <Editor key={`${id}:${cloneOf}`} rule={isNew ? undefined : rule.data} draft={toDraft(rule.data, isNew && !!cloneOf)} />;
}

function Editor({ rule, draft: initial }: { rule?: SecurityRule; draft: Draft }) {
  const navigate = useNavigate();
  const [d, setD] = useState<Draft>(initial);
  const save = useSaveRule(rule?.id);
  const remove = useDeleteRule();
  const readOnly = !!rule?.isDefault;
  const upd = (p: Partial<Draft>) => setD((x) => ({ ...x, ...p }));
  const setQuery = (i: number, p: Partial<RuleQuery>) => upd({ queries: d.queries.map((q, j) => (j === i ? { ...q, ...p } : q)) });
  const setCase = (i: number, p: Partial<RuleCase>) => upd({ cases: d.cases.map((c, j) => (j === i ? { ...c, ...p } : c)) });
  const names = d.queries.map((q, i) => q.name || String.fromCharCode(97 + i));
  return (
    <div className="sec-page sec-editor">
      <div className="sec-explorer-top">
        <Link to="/security/siem/rules" className="btn btn-ghost" aria-label="Back to the rules">
          <ArrowLeft size={16} />
        </Link>
        <h2 className="sec-h">{rule ? rule.name : 'New Rule'}</h2>
        {rule?.isDefault && <span className="tag-chip">Default rule</span>}
        <span className="syn-grow" />
        {rule && (
          <Link className="btn" to={`/security/siem/rules/new?clone=${rule.id}`}>
            <Copy size={14} /> Clone
          </Link>
        )}
        {rule && !rule.isDefault && (
          <button className="btn" onClick={() => confirm(`Delete the rule "${rule.name}"?`) && remove.mutate(rule.id, { onSuccess: () => navigate('/security/siem/rules') })}>
            <Trash2 size={14} /> Delete
          </button>
        )}
      </div>
      {rule?.notEvaluated && <p className="sec-banner">This rule is stored but not evaluated: {rule.notEvaluated}.</p>}
      {readOnly && <p className="sec-banner info">Default rules come from their content pack: turn them on or off, or clone one to change it.</p>}
      <fieldset disabled={readOnly} className="sec-form">
        <section>
          <h4>1. Define search queries</h4>
          <p className="muted">Each query counts the logs matching it over the evaluation window, per group. Conditions refer to queries by name.</p>
          {d.queries.map((q, i) => (
            <div key={i} className="sec-query">
              <span className="sec-qname">{names[i]}</span>
              <input className="mono grow" placeholder="source:cloudtrail @evt.name:ConsoleLogin @evt.outcome:failure" value={q.query} onChange={(e) => setQuery(i, { query: e.target.value })} />
              <Select
                label="Aggregation"
                value={q.aggregation ?? 'count'}
                onChange={(v) => setQuery(i, { aggregation: v })}
                options={[
                  { value: 'count', label: 'Count' },
                  { value: 'cardinality', label: 'Unique count' },
                ]}
              />
              {q.aggregation === 'cardinality' && (
                <input className="mono" placeholder="of @field" value={q.distinctFields?.[0] ?? ''} onChange={(e) => setQuery(i, { distinctFields: e.target.value ? [e.target.value] : [] })} />
              )}
              <span className="sec-groupby">group by</span>
              <input
                className="mono"
                placeholder="@usr.email, service"
                value={(q.groupByFields ?? []).join(', ')}
                onChange={(e) =>
                  setQuery(i, {
                    groupByFields: e.target.value
                      .split(',')
                      .map((s) => s.trim())
                      .filter(Boolean),
                  })
                }
              />
              {d.queries.length > 1 && (
                <button type="button" className="btn btn-ghost" onClick={() => upd({ queries: d.queries.filter((_, j) => j !== i) })} aria-label="Remove the query">
                  <X size={14} />
                </button>
              )}
            </div>
          ))}
          {d.queries.length < 10 && (
            <button type="button" className="link-btn" onClick={() => upd({ queries: [...d.queries, { ...d.queries[0], query: '', name: '' }] })}>
              + Add query
            </button>
          )}
        </section>
        <section>
          <h4>2. Set rule cases</h4>
          <p className="muted">The first case whose condition holds sets the signal&rsquo;s severity.</p>
          {d.cases.map((c, i) => (
            <div key={i} className="sec-case">
              <input className="mono" placeholder="a > 5" value={c.condition ?? ''} onChange={(e) => setCase(i, { condition: e.target.value })} />
              <Select label="Severity" value={c.status} onChange={(v) => setCase(i, { status: v })} options={SEV_ORDER.map((s) => ({ value: s, label: SEVERITY_LABEL[s] }))} />
              <input placeholder="Case name (optional)" value={c.name ?? ''} onChange={(e) => setCase(i, { name: e.target.value })} />
              <input
                placeholder="Notify: @slack-security, @ana@example.com"
                value={(c.notifications ?? []).join(', ')}
                onChange={(e) =>
                  setCase(i, {
                    notifications: e.target.value
                      .split(',')
                      .map((s) => s.trim())
                      .filter(Boolean),
                  })
                }
              />
              {d.cases.length > 1 && (
                <button type="button" className="btn btn-ghost" onClick={() => upd({ cases: d.cases.filter((_, j) => j !== i) })} aria-label="Remove the case">
                  <X size={14} />
                </button>
              )}
            </div>
          ))}
          <button type="button" className="link-btn" onClick={() => upd({ cases: [...d.cases, { name: '', status: 'low', condition: 'a > 0', notifications: [] }] })}>
            + Add case
          </button>
          <div className="sec-options">
            <Select label="Evaluation window" value={String(d.window)} onChange={(v) => upd({ window: Number(v) })} options={WINDOWS.map((s) => ({ value: String(s), label: label(s) }))} />
            <Select label="Keep alive" value={String(d.keepAlive)} onChange={(v) => upd({ keepAlive: Number(v) })} options={KEEP_ALIVES.map((s) => ({ value: String(s), label: label(s) }))} />
            <Select
              label="Maximum signal duration"
              value={String(d.maxDuration)}
              onChange={(v) => upd({ maxDuration: Number(v) })}
              options={MAX_DURATIONS.map((s) => ({ value: String(s), label: label(s) }))}
            />
          </div>
        </section>
        <section>
          <h4>3. Say what&rsquo;s happening</h4>
          <label>
            Rule name
            <input value={d.name} onChange={(e) => upd({ name: e.target.value })} />
          </label>
          <label>
            Message
            <textarea rows={4} value={d.message} onChange={(e) => upd({ message: e.target.value })} />
          </label>
          <label>
            Tags
            <input placeholder="tactic:TA0006-credential-access, team:security" value={d.tags} onChange={(e) => upd({ tags: e.target.value })} />
          </label>
          <label className="sec-check-label">
            <input type="checkbox" checked={d.hasExtendedTitle} onChange={(e) => upd({ hasExtendedTitle: e.target.checked })} /> Add the group values to the signal&rsquo;s title
          </label>
        </section>
      </fieldset>
      <footer className="sec-editor-foot">
        {save.isError && <span className="sec-error">{(save.error as Error).message}</span>}
        <label className="sec-check-label">
          <input type="checkbox" checked={d.isEnabled} onChange={(e) => upd({ isEnabled: e.target.checked })} /> Enabled
        </label>
        <button className="btn btn-primary" disabled={save.isPending} onClick={() => save.mutate(toInput(d), { onSuccess: (r) => navigate(`/security/siem/rules/${r.id}`) })}>
          {rule ? 'Save Rule' : 'Create Rule'}
        </button>
      </footer>
    </div>
  );
}
