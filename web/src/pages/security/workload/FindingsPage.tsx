import { useMemo, useState } from 'react';
import { useSearchParams } from 'react-router-dom';
import { ArrowDownUp, Bell, BellOff, BarChart3, ChevronDown, ChevronRight, Container, Download, PanelLeftClose, PanelLeftOpen, Search, Server, X } from 'lucide-react';
import type { WpFinding } from '../../../api/workloadProtection/client';
import type { Facet } from '../../../api/types';
import { FacetPanel } from '../../../components/facets/FacetPanel';
import { TimePicker } from '../../../components/shell/PageHeader';
import { TimeProvider, useTime } from '../../../components/shell/TimeContext';
import { ErrorState, Loading } from '../../../components/ui/AsyncState';
import { compact, timeAgo } from '../../../lib/format';
import { formatQuery, parseQuery, type Term } from '../../../lib/searchQuery';
import { SeverityBadge, SEVERITY_LABEL } from '../common';
import { downloadCsv } from '../appsec/common';
import { useTriageWpFinding, useWpFindings } from './exploreQueries';
import '../appsec/findings.css';
import './explore.css';

const MIN = 60_000;
const RANK = ['info', 'low', 'medium', 'high', 'critical'];
const byRank = (a: string, b: string) => RANK.indexOf(b) - RANK.indexOf(a);
const DEFAULT_QUERY = '@status:open';
const NESTED = 5;

const FACETS: [string, string][] = [
  ['@severity', 'Severity'],
  ['@status', 'Status'],
  ['service', 'Service'],
  ['env', 'Environment'],
  ['@resource_type', 'Resource Type'],
  ['@resource', 'Resource Name'],
  ['@evaluation', 'Evaluation'],
  ['@rule.name', 'Rule'],
  ['host', 'Host'],
];

function valueOf(f: WpFinding, key: string): string {
  switch (key.replace(/^@/, '')) {
    case 'severity':
      return f.severity;
    case 'status':
      return f.status;
    case 'service':
      return f.service;
    case 'env':
      return f.env;
    case 'resource_type':
      return f.resourceType;
    case 'resource':
      return f.resource;
    case 'evaluation':
      return 'fail';
    case 'rule.name':
      return f.ruleName;
    case 'rule.id':
      return f.ruleId;
    case 'host':
      return f.host;
    case 'image_name':
      return f.image;
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

function matches(f: WpFinding, terms: Term[]) {
  return terms.every((t) => {
    const hit = t.kind === 'text' ? `${f.ruleName} ${f.resource} ${f.host} ${f.image} ${f.service}`.toLowerCase().includes(t.text.toLowerCase()) : t.values.some((v) => like(v, valueOf(f, t.key)));
    return hit !== t.negate;
  });
}

function ResourceChip({ f }: { f: WpFinding }) {
  return (
    <span className="wpx-host" title={f.resourceType === 'container' ? `${f.image} on ${f.host}` : f.host}>
      {f.resourceType === 'container' ? <Container size={12} /> : <Server size={12} />} {f.resource}
    </span>
  );
}

type Group = 'rule' | 'resource' | 'none';

/** Workload Protection → Findings: the hosts and containers failing the finding rules, from the Agents' events. */
export function WorkloadFindingsPage() {
  return (
    <TimeProvider initial="1w">
      <Findings />
    </TimeProvider>
  );
}

function Findings() {
  const { range } = useTime();
  const [params, setParams] = useSearchParams();
  const r = useMemo(() => ({ from: Math.floor(range.from / MIN) * MIN, to: Math.ceil(range.to / MIN) * MIN }), [range.from, range.to]);
  const q = useWpFindings(r);
  const triage = useTriageWpFinding();
  const query = params.get('q') ?? DEFAULT_QUERY;
  const [draft, setDraft] = useState(query);
  const [group, setGroup] = useState<Group>('rule');
  const [controls, setControls] = useState(true);
  const [graphs, setGraphs] = useState(false);
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
  const rules = useMemo(() => new Map((q.data?.rules ?? []).map((r) => [r.id, r])), [q.data]);
  const rows = useMemo(() => all.filter((f) => matches(f, terms)), [all, terms]);
  const facets = useMemo<Facet[]>(
    () =>
      FACETS.map(([key, label]) => {
        const others = terms.filter((t) => t.kind !== 'tag' || t.key !== key);
        const counts = new Map<string, number>();
        for (const f of all) {
          const v = valueOf(f, key);
          if (v && matches(f, others)) counts.set(v, (counts.get(v) ?? 0) + 1);
        }
        const values = [...counts.entries()].map(([value, count]) => ({ value, count }));
        values.sort((a, b) => (key === '@severity' ? byRank(a.value, b.value) : b.count - a.count || a.value.localeCompare(b.value)));
        return { key, label, group: 'CORE', values };
      }),
    [all, terms],
  );
  const groups = useMemo(() => {
    const m = new Map<string, WpFinding[]>();
    for (const f of rows) {
      const k = group === 'rule' ? f.ruleId : group === 'resource' ? f.resource : f.key;
      m.set(k, [...(m.get(k) ?? []), f]);
    }
    for (const items of m.values()) items.sort((a, b) => byRank(a.severity, b.severity) || a.resource.localeCompare(b.resource));
    return [...m.entries()].sort((a, b) => byRank(a[1][0].severity, b[1][0].severity) || b[1].length - a[1].length);
  }, [rows, group]);
  const bySeverity = useMemo(() => {
    const m = new Map<string, number>();
    for (const f of rows) m.set(f.severity, (m.get(f.severity) ?? 0) + 1);
    return [...m.entries()].sort((a, b) => byRank(a[0], b[0]));
  }, [rows]);
  const byType = useMemo(() => {
    const m = new Map<string, number>();
    for (const f of rows) m.set(f.resourceType, (m.get(f.resourceType) ?? 0) + 1);
    return [...m.entries()].sort((a, b) => b[1] - a[1]);
  }, [rows]);
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
  const triageButtons = (f: WpFinding) => (
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
  const check = (f: WpFinding) => <input type="checkbox" checked={selected.has(f.key)} onChange={() => toggleIn(setSelected, f.key)} aria-label="Select finding" />;
  const statusCell = (items: WpFinding[]) => {
    const openN = items.filter((f) => f.status !== 'muted').length;
    return (
      <>
        {openN > 0 && <span className="wpx-status open">{compact(openN)} OPEN</span>}
        {items.length - openN > 0 && <span className="wpx-status muted">{compact(items.length - openN)} MUTED</span>}
      </>
    );
  };
  const maxSev = Math.max(1, ...bySeverity.map(([, n]) => n));
  return (
    <div className="sec-page fi-page">
      <div className="fi-top">
        <span className="syn-grow" />
        <TimePicker compact />
      </div>
      <form
        className="fi-query"
        onSubmit={(e) => {
          e.preventDefault();
          setQuery(draft.trim());
        }}
      >
        <Search size={15} className="muted" />
        <input value={draft} placeholder="Filter findings: @severity:high host:… @resource_type:container …" onChange={(e) => setDraft(e.target.value)} onBlur={() => setQuery(draft.trim())} />
        {draft && (
          <button type="button" className="btn btn-ghost btn-sm" onClick={() => setQuery('')} aria-label="Clear">
            <X size={14} />
          </button>
        )}
      </form>
      <div className="wpx-groupby-row">
        <div className="fi-groupby">
          <span>Group by</span>
          {(
            [
              ['rule', 'Rule Name'],
              ['resource', 'Resource Name'],
              ['none', 'None'],
            ] as const
          ).map(([v, l]) => (
            <button key={v} className={group === v ? 'active' : ''} onClick={() => (setGroup(v), setOpen(new Set()))}>
              {l}
            </button>
          ))}
        </div>
        <span className="syn-grow" />
        <button className="link-btn" onClick={() => setGraphs((g) => !g)}>
          <BarChart3 size={14} /> {graphs ? 'Hide Graphs' : 'Show Graphs'}
        </button>
      </div>
      {graphs && (
        <div className="wpx-graphs">
          <section>
            <h5>Findings by severity</h5>
            <ul className="inv-toplist">
              {bySeverity.map(([s, n]) => (
                <li key={s}>
                  <b>{compact(n)}</b>
                  <span className="inv-toplist-bar">
                    <i className={`wpx-bar sev-${s}`} style={{ width: `${(n / maxSev) * 100}%` }} />
                    <span>{SEVERITY_LABEL[s] ?? s}</span>
                  </span>
                </li>
              ))}
            </ul>
          </section>
          <section>
            <h5>Findings by resource type</h5>
            <div className="wpx-types">
              {byType.map(([t, n]) => (
                <div key={t}>
                  <b>{compact(n)}</b>
                  <span>
                    {t === 'container' ? <Container size={13} /> : <Server size={13} />} {t}
                  </span>
                </div>
              ))}
            </div>
          </section>
        </div>
      )}
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
              defaultOpen={['@severity', '@status', '@resource_type']}
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
                <b>{compact(group === 'none' ? rows.length : groups.length)}</b> results found
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
                  downloadCsv(
                    'workload-findings.csv',
                    ['severity', 'rule', 'resource_type', 'resource', 'host', 'image', 'service', 'env', 'events', 'first_seen', 'last_seen', 'status'],
                    rows.map((f) => [
                      f.severity,
                      f.ruleName,
                      f.resourceType,
                      f.resource,
                      f.host,
                      f.image,
                      f.service,
                      f.env,
                      f.events,
                      new Date(f.firstSeen).toISOString(),
                      new Date(f.lastSeen).toISOString(),
                      f.status,
                    ]),
                  )
                }
              >
                <Download size={14} /> Export
              </button>
            </div>
            {!rows.length ? (
              <div className="fi-empty">
                <Search size={40} />
                <p>No matching results found</p>
                <span className="muted">Findings come from the events your Agents send; change the search, the facets or the time range.</span>
              </div>
            ) : group === 'none' ? (
              <table className="mon-table syn-table fi-findings wpx-findings">
                <thead>
                  <tr>
                    <th className="sec-check" />
                    <th>SEVERITY</th>
                    <th>RULE</th>
                    <th>RESOURCE</th>
                    <th>RESOURCE TYPE</th>
                    <th className="num">EVENTS</th>
                    <th>LAST SEEN</th>
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
                        <ResourceChip f={f} />
                      </td>
                      <td>{f.resourceType}</td>
                      <td className="num">{compact(f.events)}</td>
                      <td className="nowrap muted">{timeAgo(f.lastSeen)}</td>
                      <td>{triageButtons(f)}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            ) : (
              <table className="mon-table syn-table fi-findings wpx-findings">
                <thead>
                  <tr>
                    <th>{group === 'resource' ? 'RESOURCE' : 'RULE'}</th>
                    <th>RESOURCE TYPE</th>
                    <th>
                      <ArrowDownUp size={11} /> SEVERITY BREAKDOWN
                    </th>
                    <th className="num">{group === 'resource' ? 'RULES' : 'RESOURCES'}</th>
                    <th>STATUS</th>
                    <th>TICKETS</th>
                  </tr>
                </thead>
                <tbody>
                  {groups.map(([k, items]) => {
                    const expanded = open.has(k);
                    const counts = new Map<string, number>();
                    for (const f of items) counts.set(f.severity, (counts.get(f.severity) ?? 0) + 1);
                    const shown = full.has(k) ? items : items.slice(0, NESTED);
                    const f0 = items[0];
                    const rule = rules.get(f0.ruleId);
                    return [
                      <tr key={k} className="fi-group-row" onClick={() => toggleIn(setOpen, k)}>
                        <td>
                          {expanded ? <ChevronDown size={14} /> : <ChevronRight size={14} />}
                          {group === 'resource' ? <ResourceChip f={f0} /> : <span title={rule?.description}>{f0.ruleName}</span>}
                        </td>
                        <td>{group === 'resource' ? f0.resourceType : (rule?.resourceType ?? f0.resourceType)}</td>
                        <td className="nowrap">
                          {[...counts.entries()]
                            .sort((a, b) => byRank(a[0], b[0]))
                            .map(([s, n]) => (
                              <span key={s} className={`fi-sev-n sev-${s}`}>
                                {compact(n)} {s.toUpperCase()}
                              </span>
                            ))}
                        </td>
                        <td className="num">{group === 'resource' ? new Set(items.map((f) => f.ruleId)).size : new Set(items.map((f) => f.resource)).size}</td>
                        <td className="nowrap">{statusCell(items)}</td>
                        <td className="muted">—</td>
                      </tr>,
                      expanded && (
                        <tr key={`${k}-nested`}>
                          <td colSpan={6} className="fi-nested-cell">
                            <div className="fi-nested">
                              <table className="mon-table syn-table wpx-nested">
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
                                    <th>{group === 'resource' ? 'RULE' : 'RESOURCE'}</th>
                                    <th>HOST</th>
                                    <th>ENV</th>
                                    <th className="num">EVENTS</th>
                                    <th>LAST SEEN</th>
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
                                      <td>{group === 'resource' ? f.ruleName : <ResourceChip f={f} />}</td>
                                      <td className="mono wpx-ellipsis" title={f.host}>
                                        {f.host}
                                      </td>
                                      <td>{f.env && <span className="tag-chip">env:{f.env}</span>}</td>
                                      <td className="num">{compact(f.events)}</td>
                                      <td className="nowrap muted">{timeAgo(f.lastSeen)}</td>
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
