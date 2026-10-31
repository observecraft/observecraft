import { useMemo, useState } from 'react';
import { Link, useSearchParams } from 'react-router-dom';
import { ArrowDownUp, ArrowLeft, Bell, BellOff, Check, ChevronDown, ChevronRight, Download, Globe, Info, PanelLeftClose, PanelLeftOpen, Search, ShieldCheck, X } from 'lucide-react';
import type { ApiFinding, FindingRuleStatus } from '../../../api/appsec/client';
import { OWASP } from '../../../api/appsec/client';
import type { Facet } from '../../../api/types';
import { FacetPanel } from '../../../components/facets/FacetPanel';
import { ErrorState, Loading } from '../../../components/ui/AsyncState';
import { compact } from '../../../lib/format';
import type { Term } from '../../../lib/searchQuery';
import { formatQuery, parseQuery } from '../../../lib/searchQuery';
import { SeverityBadge, SEVERITY_LABEL } from '../common';
import { AapTabs, FINDINGS_TABS, MethodBadge } from './AppsecLayout';
import { useApiFindings, useTriageFinding } from './queries';
import './findings.css';

const RANK = ['info', 'low', 'medium', 'high', 'critical'];
const byRank = (a: string, b: string) => RANK.indexOf(b) - RANK.indexOf(a);
const DEFAULT_QUERY = '@status:open';
const NESTED = 5;

/** Saves rows as a CSV file. */
function downloadCsv(name: string, rows: (string | number)[][]) {
  const csv = rows.map((r) => r.map((c) => (/[",\n]/.test(String(c)) ? `"${String(c).replace(/"/g, '""')}"` : String(c))).join(',')).join('\n');
  const a = document.createElement('a');
  a.href = URL.createObjectURL(new Blob([csv], { type: 'text/csv' }));
  a.download = name;
  a.click();
  URL.revokeObjectURL(a.href);
}

// ---------------------------------------------------------------- API Findings

const FACETS: [string, string][] = [
  ['@severity', 'Severity'],
  ['@status', 'Status'],
  ['@owasp', 'OWASP Category'],
  ['@rule.name', 'Rule'],
  ['service', 'Service'],
  ['env', 'Env'],
];

function valueOf(f: ApiFinding, key: string): string {
  switch (key.replace(/^@/, '')) {
    case 'severity':
      return f.severity;
    case 'status':
      return f.status;
    case 'owasp':
      return f.owasp;
    case 'rule.name':
      return f.ruleName;
    case 'rule.id':
      return f.ruleId;
    case 'service':
      return f.service;
    case 'env':
      return f.env;
    case 'http.route':
      return f.route;
    case 'http.method':
      return f.method;
    default:
      return '';
  }
}

const like = (pattern: string, v: string) =>
  new RegExp(
    `^${pattern
      .toLowerCase()
      .split('*')
      .map((x) => x.replace(/[.+?^${}()|[\]\\]/g, '\\$&'))
      .join('.*')}$`,
  ).test(v.toLowerCase());

function matches(f: ApiFinding, terms: Term[]) {
  return terms.every((t) => {
    const hit = t.kind === 'text' ? `${f.ruleName} ${f.service} ${f.method} ${f.route}`.toLowerCase().includes(t.text.toLowerCase()) : t.values.some((v) => like(v, valueOf(f, t.key)));
    return hit !== t.negate;
  });
}

type Group = 'rule' | 'endpoint' | 'none';

/** Findings → API Findings: endpoint findings, grouped by rule or by endpoint, with facets and triage. */
export function ApiFindingsPage() {
  const [params, setParams] = useSearchParams();
  const q = useApiFindings();
  const triage = useTriageFinding();
  const query = params.get('q') ?? DEFAULT_QUERY;
  const [draft, setDraft] = useState(query);
  const [group, setGroup] = useState<Group>('rule');
  const [controls, setControls] = useState(true);
  const [open, setOpen] = useState<Set<string>>(new Set());
  const [full, setFull] = useState<Set<string>>(new Set());
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const setQuery = (v: string) => {
    setDraft(v);
    setParams(
      (p) => {
        const n = new URLSearchParams(p);
        n.set('q', v);
        return n;
      },
      { replace: true },
    );
  };
  const terms = useMemo(() => parseQuery(query), [query]);
  const all = useMemo(() => q.data?.findings ?? [], [q.data]);
  const rows = useMemo(() => all.filter((f) => matches(f, terms)), [all, terms]);
  const facets = useMemo<Facet[]>(
    () =>
      FACETS.map(([key, label]) => {
        // Each facet counts what the other filters let through.
        const others = terms.filter((t) => t.kind !== 'tag' || t.key !== key);
        const counts = new Map<string, number>();
        for (const f of all) if (matches(f, others)) counts.set(valueOf(f, key), (counts.get(valueOf(f, key)) ?? 0) + 1);
        const values = [...counts.entries()].map(([value, count]) => ({ value, count }));
        values.sort((a, b) => (key === '@severity' ? byRank(a.value, b.value) : b.count - a.count || a.value.localeCompare(b.value)));
        return { key, label, group: 'CORE', values };
      }),
    [all, terms],
  );
  const groups = useMemo(() => {
    const m = new Map<string, ApiFinding[]>();
    for (const f of rows) {
      const k = group === 'rule' ? f.ruleId : group === 'endpoint' ? `${f.service}|${f.method}|${f.route}` : f.key;
      m.set(k, [...(m.get(k) ?? []), f]);
    }
    for (const items of m.values()) items.sort((a, b) => byRank(a.severity, b.severity) || a.service.localeCompare(b.service));
    return [...m.entries()].sort((a, b) => byRank(a[1][0].severity, b[1][0].severity) || b[1].length - a[1].length);
  }, [rows, group]);
  const toggleIn = (set: (f: (s: Set<string>) => Set<string>) => void, k: string) =>
    set((s) => {
      const n = new Set(s);
      if (n.has(k)) n.delete(k);
      else n.add(k);
      return n;
    });
  const bulk = (status: 'open' | 'muted') => {
    for (const key of selected) triage.mutate({ key, status });
    setSelected(new Set());
  };
  const triageButtons = (f: ApiFinding) => (
    <span className="fi-triage">
      {f.status === 'muted' && <span className="fi-state">MUTED</span>}
      <button
        className="fi-icon-btn"
        title={f.status === 'muted' ? 'Reopen' : 'Mute'}
        aria-label={f.status === 'muted' ? 'Reopen' : 'Mute'}
        onClick={() => triage.mutate({ key: f.key, status: f.status === 'muted' ? 'open' : 'muted' })}
      >
        {f.status === 'muted' ? <Bell size={13} /> : <BellOff size={13} />}
      </button>
    </span>
  );
  const check = (f: ApiFinding) => <input type="checkbox" checked={selected.has(f.key)} onChange={() => toggleIn(setSelected, f.key)} aria-label="Select finding" />;
  return (
    <div className="sec-page fi-page">
      <AapTabs tabs={FINDINGS_TABS} />
      <form
        className="fi-query"
        onSubmit={(e) => {
          e.preventDefault();
          setQuery(draft.trim());
        }}
      >
        <Search size={15} className="muted" />
        <input value={draft} placeholder="Filter findings: @severity:high service:api-gateway @owasp:API2 …" onChange={(e) => setDraft(e.target.value)} onBlur={() => setQuery(draft.trim())} />
        {draft && (
          <button type="button" className="btn btn-ghost btn-sm" onClick={() => setQuery('')} aria-label="Clear">
            <X size={14} />
          </button>
        )}
      </form>
      <div className="fi-groupby">
        <span>Group by</span>
        {(
          [
            ['rule', 'API Security Findings'],
            ['endpoint', 'API Endpoints'],
            ['none', 'None'],
          ] as const
        ).map(([v, l]) => (
          <button key={v} className={group === v ? 'active' : ''} onClick={() => (setGroup(v), setOpen(new Set()))}>
            {l}
          </button>
        ))}
      </div>
      {q.isLoading ? (
        <Loading />
      ) : q.isError ? (
        <ErrorState error={q.error} onRetry={() => q.refetch()} />
      ) : (
        <div className={`fi-body${controls ? '' : ' no-controls'}`}>
          {controls && (
            <FacetPanel
              facets={facets}
              terms={terms}
              onChange={(t) => setQuery(formatQuery(t))}
              mode="exclude"
              defaultOpen={['@severity', '@status', 'service']}
              verified={[]}
              noun="facets"
              valueSearch
              renderValueIcon={(k, v) => (k === '@severity' ? <i className={`fi-sev-bar sev-${v}`} /> : null)}
              valueLabel={(k, v) => (k === '@severity' ? (SEVERITY_LABEL[v] ?? v) : k === '@status' ? v[0].toUpperCase() + v.slice(1) : v)}
            />
          )}
          <div className="fi-results">
            <div className="syn-toolbar">
              <button className="link-btn" onClick={() => setControls((c) => !c)}>
                {controls ? <PanelLeftClose size={15} /> : <PanelLeftOpen size={15} />} {controls ? 'Hide Controls' : 'Show Controls'}
              </button>
              <span className="syn-toolbar-sep" />
              <span>
                <b>{compact(groups.length)}</b> results found
              </span>
              {selected.size > 0 && (
                <>
                  <span className="syn-toolbar-sep" />
                  <span className="muted">{selected.size} selected</span>
                  <button className="btn btn-sm" onClick={() => bulk('muted')}>
                    <BellOff size={13} /> Mute
                  </button>
                  <button className="btn btn-sm" onClick={() => bulk('open')}>
                    <Bell size={13} /> Reopen
                  </button>
                </>
              )}
              <span className="syn-grow" />
              <button
                className="btn"
                onClick={() =>
                  downloadCsv('api-findings.csv', [
                    ['severity', 'rule', 'owasp', 'service', 'env', 'method', 'route', 'status'],
                    ...rows.map((f) => [f.severity, f.ruleName, f.owasp, f.service, f.env, f.method, f.route, f.status]),
                  ])
                }
              >
                <Download size={14} /> Export
              </button>
            </div>
            {!groups.length ? (
              <div className="fi-empty">
                <Search size={40} />
                <p>No findings match.</p>
                <span className="muted">Findings come from the endpoints of your traced services; change the search or the facets.</span>
              </div>
            ) : group === 'none' ? (
              <table className="mon-table syn-table fi-findings">
                <thead>
                  <tr>
                    <th className="sec-check" />
                    <th>SEVERITY</th>
                    <th>API SECURITY FINDING</th>
                    <th>SERVICE</th>
                    <th>ENDPOINT</th>
                    <th>TRIAGE</th>
                  </tr>
                </thead>
                <tbody>
                  {rows.map((f) => (
                    <tr key={f.key}>
                      <td className="sec-check">{check(f)}</td>
                      <td>
                        <SeverityBadge severity={f.severity} />
                      </td>
                      <td>{f.ruleName}</td>
                      <td>
                        <span className="fi-service">
                          <Globe size={12} /> {f.service}
                        </span>
                      </td>
                      <td className="nowrap">
                        <MethodBadge method={f.method} /> <span className="mono">{f.route}</span>
                      </td>
                      <td>{triageButtons(f)}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            ) : (
              <table className="mon-table syn-table fi-findings">
                <thead>
                  <tr>
                    <th>{group === 'endpoint' ? 'API ENDPOINT' : 'API SECURITY FINDING'}</th>
                    <th>
                      <ArrowDownUp size={11} /> SEVERITY BREAKDOWN
                    </th>
                    <th className="num">{group === 'endpoint' ? 'FINDINGS' : 'ENDPOINTS'}</th>
                  </tr>
                </thead>
                <tbody>
                  {groups.map(([k, items]) => {
                    const expanded = open.has(k);
                    const counts = new Map<string, number>();
                    for (const f of items) counts.set(f.severity, (counts.get(f.severity) ?? 0) + 1);
                    const shown = full.has(k) ? items : items.slice(0, NESTED);
                    const f0 = items[0];
                    return [
                      <tr key={k} className="fi-group-row" onClick={() => toggleIn(setOpen, k)}>
                        <td>
                          {expanded ? <ChevronDown size={14} /> : <ChevronRight size={14} />}
                          {group === 'endpoint' ? (
                            <span>
                              <MethodBadge method={f0.method} /> <b className="mono">{f0.route}</b> <span className="muted">({f0.service})</span>
                            </span>
                          ) : (
                            <span>{f0.ruleName}</span>
                          )}
                        </td>
                        <td className="nowrap">
                          {[...counts.entries()]
                            .sort((a, b) => byRank(a[0], b[0]))
                            .map(([s, n]) => (
                              <span key={s} className={`fi-sev-n sev-${s}`}>
                                {compact(n)} {s.toUpperCase()}
                              </span>
                            ))}
                        </td>
                        <td className="num">{group === 'endpoint' ? items.length : new Set(items.map((f) => `${f.service}${f.method}${f.route}`)).size}</td>
                      </tr>,
                      expanded && (
                        <tr key={`${k}-nested`}>
                          <td colSpan={3} className="fi-nested-cell">
                            <div className="fi-nested">
                              <table className="mon-table syn-table">
                                <thead>
                                  <tr>
                                    <th className="sec-check">
                                      <input
                                        type="checkbox"
                                        aria-label="Select all"
                                        checked={shown.every((f) => selected.has(f.key))}
                                        onChange={(e) =>
                                          setSelected((s) => {
                                            const n = new Set(s);
                                            for (const f of shown) {
                                              if (e.target.checked) n.add(f.key);
                                              else n.delete(f.key);
                                            }
                                            return n;
                                          })
                                        }
                                      />
                                    </th>
                                    <th>SEVERITY</th>
                                    {group === 'endpoint' ? <th>API SECURITY FINDING</th> : <th>SERVICE</th>}
                                    {group === 'endpoint' ? <th>OWASP</th> : <th>ENDPOINT</th>}
                                    <th>TRIAGE</th>
                                  </tr>
                                </thead>
                                <tbody>
                                  {shown.map((f) => (
                                    <tr key={f.key}>
                                      <td className="sec-check">{check(f)}</td>
                                      <td>
                                        <SeverityBadge severity={f.severity} />
                                      </td>
                                      {group === 'endpoint' ? (
                                        <td>{f.ruleName}</td>
                                      ) : (
                                        <td>
                                          <span className="fi-service">
                                            <Globe size={12} /> {f.service}
                                          </span>
                                          {f.env && <span className="tag-chip">env:{f.env}</span>}
                                        </td>
                                      )}
                                      {group === 'endpoint' ? (
                                        <td>
                                          <span className="tag-chip">{f.owasp}:2023</span>
                                        </td>
                                      ) : (
                                        <td className="nowrap">
                                          <MethodBadge method={f.method} /> <span className="mono">{f.route}</span>
                                        </td>
                                      )}
                                      <td>{triageButtons(f)}</td>
                                    </tr>
                                  ))}
                                </tbody>
                              </table>
                              {items.length > NESTED && (
                                <button className="fi-view-all" onClick={() => toggleIn(setFull, k)}>
                                  {full.has(k) ? 'Show less' : `View All (${items.length})…`}
                                </button>
                              )}
                            </div>
                          </td>
                        </tr>
                      ),
                    ];
                  })}
                </tbody>
              </table>
            )}
          </div>
        </div>
      )}
    </div>
  );
}

// ---------------------------------------------------------------- Compliance

const scoreClass = (score: number) => (score >= 80 ? 'good' : score >= 50 ? 'warn' : 'bad');
const pct = (pass: number, total: number) => (total ? Math.round((pass / total) * 1000) / 10 : 100);

function Passing({ pass, total }: { pass: number; total: number }) {
  const p = pct(pass, total);
  return (
    <span className="fi-passing">
      <span className="fi-bar">
        <i style={{ width: `${p}%` }} />
      </span>
      <b>{Math.round(p)}%</b>
      <span className="muted">
        {compact(pass)} of {compact(total)}
      </span>
    </span>
  );
}

/** The framework's posture: endpoints evaluated and failing over its rules. */
function posture(rules: FindingRuleStatus[]) {
  const evaluated = rules.reduce((a, r) => a + r.evaluated, 0);
  const failing = rules.reduce((a, r) => a + r.failing, 0);
  return { evaluated, failing, score: pct(evaluated - failing, evaluated) };
}

/** Findings → Compliance: the frameworks (the OWASP API Security Top 10) and one framework's requirements. */
export function CompliancePage() {
  const q = useApiFindings();
  const [params, setParams] = useSearchParams();
  const framework = params.get('framework');
  if (q.isLoading) return <Loading />;
  if (q.isError) return <ErrorState error={q.error} onRetry={() => q.refetch()} />;
  return framework ? (
    <FrameworkPage rules={q.data!.rules} findings={q.data!.findings} onBack={() => setParams({})} />
  ) : (
    <FrameworksPage rules={q.data!.rules} onOpen={() => setParams({ framework: 'owasp-api' })} />
  );
}

const FRAMEWORK = { name: 'OWASP API Security Top 10', version: '2023', group: 'Industry frameworks' };

function FrameworksPage({ rules, onOpen }: { rules: FindingRuleStatus[]; onOpen: () => void }) {
  const [search, setSearch] = useState('');
  const [grouped, setGrouped] = useState(true);
  const [collapsed, setCollapsed] = useState(false);
  const { evaluated, failing, score } = posture(rules);
  const failingRules = rules.filter((r) => r.failing);
  const topSev = [...failingRules].sort((a, b) => byRank(a.severity, b.severity))[0]?.severity;
  const visible = FRAMEWORK.name.toLowerCase().includes(search.trim().toLowerCase());
  return (
    <div className="sec-page fi-page">
      <AapTabs tabs={FINDINGS_TABS} />
      <div className="fi-frameworks">
        <div className="fi-compliance-head">
          <div>
            <h2>Compliance Frameworks</h2>
            <p className="muted">Measure your API posture against industry security frameworks, from the findings of your endpoints.</p>
          </div>
        </div>
        <label className="sec-search">
          <Search size={14} />
          <input placeholder="Search by name" value={search} onChange={(e) => setSearch(e.target.value)} />
        </label>
        <div className="fi-groupby">
          <span>Group by</span>
          <button className={grouped ? 'active' : ''} onClick={() => setGrouped(true)}>
            Framework type
          </button>
          <button className={!grouped ? 'active' : ''} onClick={() => setGrouped(false)}>
            None
          </button>
        </div>
        <div className="fi-table-box">
          <div className="fi-toolbar">
            <span>
              <b>{grouped ? 1 : visible ? 1 : 0}</b> {grouped ? 'group' : 'framework'}
            </span>
            {grouped && (
              <>
                <span className="syn-toolbar-sep" />
                <button className="link-btn" onClick={() => setCollapsed((c) => !c)}>
                  {collapsed ? 'Expand All' : 'Collapse All'}
                </button>
              </>
            )}
            <span className="syn-grow" />
            <button
              className="link-btn"
              onClick={() =>
                downloadCsv('compliance-frameworks.csv', [
                  ['framework', 'version', 'score', 'resources passing', 'resources evaluated'],
                  [FRAMEWORK.name, FRAMEWORK.version, score, evaluated - failing, evaluated],
                ])
              }
            >
              <Download size={14} /> Download as CSV
            </button>
          </div>
          <table className="mon-table syn-table">
            <thead>
              <tr>
                <th>FRAMEWORK</th>
                <th>SCORE</th>
                <th>TOP FAILING RULES</th>
                <th>RESOURCES PASSING</th>
              </tr>
            </thead>
            <tbody>
              {grouped && (
                <tr className="fi-group-head" onClick={() => setCollapsed((c) => !c)}>
                  <td colSpan={4}>
                    <span>
                      {collapsed ? <ChevronRight size={14} /> : <ChevronDown size={14} />} {FRAMEWORK.group} <span className="fi-count">{visible ? 1 : 0}</span>
                    </span>
                  </td>
                </tr>
              )}
              {visible && !(grouped && collapsed) && (
                <tr onClick={onOpen}>
                  <td>
                    <span className="fi-fw-name">
                      <ShieldCheck size={26} />
                      <span>
                        <b>{FRAMEWORK.name}</b>
                        <div className="muted sec-sub">version {FRAMEWORK.version}</div>
                      </span>
                    </span>
                  </td>
                  <td className={`fi-score ${scoreClass(score)}`}>{score}%</td>
                  <td>
                    {topSev ? (
                      <span className={`fi-sev-soft sev-${topSev}`}>{`${failingRules.filter((r) => r.severity === topSev).length} ${topSev.toUpperCase()}`}</span>
                    ) : (
                      <span className="muted">None</span>
                    )}
                  </td>
                  <td>
                    <Passing pass={evaluated - failing} total={evaluated} />
                  </td>
                </tr>
              )}
            </tbody>
          </table>
        </div>
      </div>
    </div>
  );
}

type RuleGroup = 'requirement' | 'severity' | 'none';

function FrameworkPage({ rules, findings, onBack }: { rules: FindingRuleStatus[]; findings: ApiFinding[]; onBack: () => void }) {
  const [search, setSearch] = useState('');
  const [groupBy, setGroupBy] = useState<RuleGroup>('requirement');
  const [showPassing, setShowPassing] = useState(false);
  const [closed, setClosed] = useState<Set<string>>(new Set());
  const { score } = posture(rules);
  const openOf = (id: string) => findings.filter((f) => f.ruleId === id && f.status === 'open').length;
  const top = [...rules].filter((r) => r.failing).sort((a, b) => byRank(a.severity, b.severity) || b.failing - a.failing);
  const shown = rules.filter((r) => (showPassing || r.failing) && (!search.trim() || `${r.name} ${r.owasp}`.toLowerCase().includes(search.trim().toLowerCase())));
  const owaspName = (id: string) => OWASP.find(([k]) => k === id)?.[1] ?? id;
  const groups: [string, string, FindingRuleStatus[]][] =
    groupBy === 'requirement'
      ? OWASP.map(([id, name]) => [id, name, shown.filter((r) => r.owasp === id)] as [string, string, FindingRuleStatus[]]).filter(([, , rs]) => rs.length)
      : groupBy === 'severity'
        ? ['critical', 'high', 'medium', 'low', 'info']
            .map((s) => [s, SEVERITY_LABEL[s], shown.filter((r) => r.severity === s)] as [string, string, FindingRuleStatus[]])
            .filter(([, , rs]) => rs.length)
        : [['all', '', shown]];
  const toggle = (k: string) =>
    setClosed((s) => {
      const n = new Set(s);
      if (n.has(k)) n.delete(k);
      else n.add(k);
      return n;
    });
  return (
    <div className="sec-page fi-page">
      <AapTabs tabs={FINDINGS_TABS} />
      <div className="fi-detail-head">
        <button className="btn btn-ghost" onClick={onBack} aria-label="Back">
          <ArrowLeft size={16} />
        </button>
        <ShieldCheck size={22} />
        <h2>{FRAMEWORK.name}</h2>
        <span className="muted">version {FRAMEWORK.version}</span>
        <span className="syn-grow" />
        <button
          className="btn"
          onClick={() =>
            downloadCsv('owasp-api-top-10.csv', [
              ['control', 'requirement', 'severity', 'rule', 'evaluation', 'resources passing', 'resources evaluated'],
              ...rules.map((r) => [`${r.owasp}:2023`, owaspName(r.owasp), r.severity, r.name, r.failing ? 'fail' : 'pass', r.evaluated - r.failing, r.evaluated]),
            ])
          }
        >
          <Download size={14} /> Download
        </button>
      </div>
      <div className="fi-detail-cards">
        <div className={`fi-detail-card fi-posture ${scoreClass(score)}`}>
          <h5>
            Posture Score <Info size={13} aria-label="Share of evaluated endpoints that pass the framework's rules" />
          </h5>
          <b>
            {score}
            <small>%</small>
          </b>
        </div>
        <div className="fi-detail-card">
          <h5>
            Top Failing Findings <Link to={`/security/appsec/findings?q=${encodeURIComponent(DEFAULT_QUERY)}`}>View all in Explorer →</Link>
          </h5>
          <table>
            <thead>
              <tr>
                <th>FINDING</th>
                <th className="num">RESOURCES</th>
                <th>TRIAGE</th>
              </tr>
            </thead>
            <tbody>
              {top.slice(0, 5).map((r) => (
                <tr key={r.id}>
                  <td>
                    <i className={`fi-rule-bar sev-${r.severity}`} />
                    {r.name}
                  </td>
                  <td className="num">{compact(r.failing)}</td>
                  <td>
                    <Link className="fi-open-pill" to={`/security/appsec/findings?q=${encodeURIComponent(`@status:open @rule.id:${r.id}`)}`}>
                      {compact(openOf(r.id))} OPEN
                    </Link>
                  </td>
                </tr>
              ))}
              {!top.length && (
                <tr>
                  <td colSpan={3} className="muted">
                    No failing findings.
                  </td>
                </tr>
              )}
            </tbody>
          </table>
        </div>
        <div className="fi-detail-card">
          <h5>Rules Severity Breakdown</h5>
          <table>
            <thead>
              <tr>
                <th>SEVERITY</th>
                <th>RULES PASSING</th>
              </tr>
            </thead>
            <tbody>
              {['critical', 'high', 'medium', 'low', 'info'].map((s) => {
                const rs = rules.filter((r) => r.severity === s);
                const pass = rs.filter((r) => !r.failing).length;
                return (
                  <tr key={s}>
                    <td>
                      <span className={`fi-sev-soft sev-${s}`}>{s.toUpperCase()}</span>
                    </td>
                    <td>{rs.length ? <Passing pass={pass} total={rs.length} /> : <span className="muted">No rules</span>}</td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        </div>
      </div>
      <div className="fi-searchrow">
        <label className="sec-search">
          <Search size={14} />
          <input placeholder="Search" value={search} onChange={(e) => setSearch(e.target.value)} />
        </label>
        <label className="fi-groupsel">
          <span>Group by</span>
          <select value={groupBy} onChange={(e) => setGroupBy(e.target.value as RuleGroup)}>
            <option value="requirement">Requirement</option>
            <option value="severity">Severity</option>
            <option value="none">None</option>
          </select>
        </label>
      </div>
      <div className="fi-top">
        <span className="muted">
          {shown.length} rules across {new Set(shown.map((r) => r.owasp)).size} requirements
        </span>
        <span className="syn-grow" />
        <label className="fi-toggle">
          <input type="checkbox" checked={showPassing} onChange={(e) => setShowPassing(e.target.checked)} /> Show rules without findings
        </label>
      </div>
      <div className="fi-table-box">
        <table className="mon-table syn-table fi-rules">
          <thead>
            <tr>
              <th>CONTROL</th>
              <th>SEVERITY</th>
              <th>RULE</th>
              <th>EVALUATION</th>
              <th>RESOURCES PASSING</th>
            </tr>
          </thead>
          <tbody>
            {groups.map(([k, name, rs]) => {
              const passing = rs.filter((r) => !r.failing).length;
              const isClosed = closed.has(k);
              return [
                groupBy !== 'none' && (
                  <tr key={k} className="fi-group-head" onClick={() => toggle(k)}>
                    <td colSpan={5}>
                      <span>
                        {isClosed ? <ChevronRight size={14} /> : <ChevronDown size={14} />} {name}
                        <span className={`fi-req-pill ${passing === rs.length ? 'good' : 'bad'}`}>{Math.round((passing / rs.length) * 100)}% RULES PASSING</span>
                        <span className="muted">
                          {passing} of {rs.length}
                        </span>
                      </span>
                    </td>
                  </tr>
                ),
                ...(isClosed
                  ? []
                  : rs.map((r) => (
                      <tr key={r.id} title={r.description}>
                        <td className="mono">{r.owasp}:2023</td>
                        <td>
                          <span className={`fi-sev-soft sev-${r.severity}`}>{r.severity.toUpperCase()}</span>
                        </td>
                        <td>
                          <Link to={`/security/appsec/findings?q=${encodeURIComponent(`@rule.id:${r.id}`)}`}>{r.name}</Link>
                        </td>
                        <td>{r.failing ? <X size={16} className="fi-fail" aria-label="Failing" /> : <Check size={16} className="fi-ok" aria-label="Passing" />}</td>
                        <td>
                          <Passing pass={r.evaluated - r.failing} total={r.evaluated} />
                        </td>
                      </tr>
                    ))),
              ];
            })}
            {!shown.length && (
              <tr>
                <td colSpan={5} className="muted">
                  {showPassing ? 'No rules match.' : 'No rule has findings. Turn on “Show rules without findings” to see every rule.'}
                </td>
              </tr>
            )}
          </tbody>
        </table>
      </div>
    </div>
  );
}
