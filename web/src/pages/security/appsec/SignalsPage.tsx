import { useMemo, useState } from 'react';
import { createPortal } from 'react-dom';
import { Link, useSearchParams } from 'react-router-dom';
import type { EChartsOption } from 'echarts';
import { AlertTriangle, ArrowRight, ChevronDown, ChevronRight, Filter, Globe, Hand, Inbox, Pencil, Plus, Search, Siren, UserPlus, Users, X } from 'lucide-react';
import type { SecuritySignal, SignalEntity } from '../../../api/security/client';
import { Chart } from '../../../components/charts/Chart';
import { TimePicker } from '../../../components/shell/PageHeader';
import { TimeProvider, useTime } from '../../../components/shell/TimeContext';
import { Menu } from '../../../components/ui/Dropdown';
import { ErrorState, Loading } from '../../../components/ui/AsyncState';
import { compact, timeAgo } from '../../../lib/format';
import { token } from '../../../theme/chartTheme';
import { useMe } from '../../dashboards/reports/queries';
import { useUsers } from '../../settings/queries';
import { ARCHIVE_REASONS, SeverityBadge, STATE_LABEL } from '../common';
import { useBulkSignals, useSignal, useSignalAssignee, useSignals, useSignalState } from '../queries';
import { MethodBadge } from './AppsecLayout';
import { useBlock, useSecurityTraces } from './queries';
import './findings.css';

const MIN = 60_000;
const SCOPE = '@workflow.rule.type:application_security';
const SEVERITIES = ['critical', 'high', 'medium', 'low', 'info'];

/** The security activity a signal's rule looks for, from its query (`@appsec.type:sql_injection` → sql_injection). */
function activityOf(s: SecuritySignal): string {
  const q = s.queries[0]?.query ?? '';
  const m = /@appsec\.(?:type|category):(\S+)/.exec(q) ?? /@appsec\.events\.([\w.]+)\.track/.exec(q);
  return m ? m[1] : 'other';
}

const entity = (s: SecuritySignal, k: string) => (s.entities as unknown as Record<string, SignalEntity | undefined>)[k];

/** A dropdown with its label above the value. */
function LabeledSelect({ label, value, options, onChange }: { label: string; value: string; options: [string, string][]; onChange: (v: string) => void }) {
  return (
    <label className="fi-select">
      <span>{label}</span>
      <select value={value} onChange={(e) => onChange(e.target.value)}>
        <option value="">All</option>
        {options.map(([v, l]) => (
          <option key={v} value={v}>
            {l}
          </option>
        ))}
      </select>
    </label>
  );
}

/** App & API Protection → Signals: the signals of the application security detection rules, over the past week by default. */
export function AppsecSignalsPage() {
  return (
    <TimeProvider initial="1w">
      <Signals />
    </TimeProvider>
  );
}

function Signals() {
  const [params, setParams] = useSearchParams();
  const { range } = useTime();
  const me = useMe();
  const severity = params.get('severity') ?? '';
  const service = params.get('service') ?? '';
  const activity = params.get('activity') ?? '';
  const card = params.get('card') ?? '';
  const query = params.get('q') ?? '';
  const open = params.get('signal');
  const [searching, setSearching] = useState(!!query);
  const [draft, setDraft] = useState(query);
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const set = (k: string, v: string | null) =>
    setParams(
      (p) => {
        const n = new URLSearchParams(p);
        if (v) n.set(k, v);
        else n.delete(k);
        return n;
      },
      { replace: k !== 'signal' },
    );
  const r = useMemo(() => ({ from: Math.floor(range.from / MIN) * MIN, to: Math.ceil(range.to / MIN) * MIN }), [range.from, range.to]);
  const signals = useSignals({ range: r, query: [SCOPE, severity && `status:${severity}`, service && `service:${service}`, query].filter(Boolean).join(' '), state: '', limit: 200 });
  const all = signals.data?.signals ?? [];
  const bulk = useBulkSignals();
  const mine = (s: SecuritySignal) => !!me.data && s.assignee?.email === me.data.email;
  const counts = {
    critical: all.filter((s) => s.state !== 'archived' && (s.severity === 'critical' || s.severity === 'high')).length,
    open: all.filter((s) => s.state === 'open').length,
    mine: all.filter((s) => s.state !== 'archived' && mine(s)).length,
  };
  const rows = all.filter(
    (s) =>
      (!activity || activityOf(s) === activity) &&
      (card !== 'critical' || (s.state !== 'archived' && (s.severity === 'critical' || s.severity === 'high'))) &&
      (card !== 'open' || s.state === 'open') &&
      (card !== 'mine' || (s.state !== 'archived' && mine(s))),
  );
  const services = (signals.data?.facets.find((f) => f.key === 'service')?.values ?? []).map((v) => [v.value, v.value] as [string, string]);
  const activities = [...new Set(all.map(activityOf))].sort().map((a) => [a, a] as [string, string]);
  const toggle = (id: string) =>
    setSelected((s) => {
      const n = new Set(s);
      if (n.has(id)) n.delete(id);
      else n.add(id);
      return n;
    });
  const bulkState = (state: string, archiveReason?: string) => bulk.mutate({ ids: [...selected], state: { state, archiveReason } }, { onSuccess: () => setSelected(new Set()) });
  const cards: [string, string, number, string][] = [
    ['critical', 'Critical & High', counts.critical, 'fi-card-critical'],
    ['open', 'Open signals', counts.open, ''],
    ['mine', 'Assigned to you', counts.mine, 'fi-card-mine'],
  ];
  return (
    <div className="sec-page fi-page">
      <div className="fi-top">
        <span className="syn-grow" />
        <TimePicker compact />
      </div>
      <div className="fi-filters">
        <div className="fi-seg">
          <button className={!searching ? 'active' : ''} onClick={() => setSearching(false)} title="Filters" aria-label="Filters">
            <Filter size={14} />
          </button>
          <button className={searching ? 'active' : ''} onClick={() => setSearching(true)} title="Search" aria-label="Search">
            <Search size={14} />
          </button>
        </div>
        {searching ? (
          <form
            className="sec-search fi-grow"
            onSubmit={(e) => {
              e.preventDefault();
              set('q', draft.trim() || null);
            }}
          >
            <Search size={14} />
            <input placeholder="Search signals: @workflow.rule.name:… @http.client_ip:…" value={draft} onChange={(e) => setDraft(e.target.value)} onBlur={() => set('q', draft.trim() || null)} />
          </form>
        ) : (
          <>
            <LabeledSelect label="Severity" value={severity} options={SEVERITIES.map((s) => [s, s[0].toUpperCase() + s.slice(1)])} onChange={(v) => set('severity', v || null)} />
            <LabeledSelect label="Service" value={service} options={services} onChange={(v) => set('service', v || null)} />
            <LabeledSelect label="Security Activity" value={activity} options={activities} onChange={(v) => set('activity', v || null)} />
            <button className="btn" onClick={() => setSearching(true)}>
              <Plus size={14} /> Filter
            </button>
          </>
        )}
      </div>
      <div className="fi-cards">
        {cards.map(([k, label, n, cls]) => (
          <button key={k} className={`fi-card ${cls}${card === k ? ' active' : ''}`} onClick={() => set('card', card === k ? null : k)}>
            {k === 'critical' ? <AlertTriangle size={16} /> : <Inbox size={16} />}
            <b>{label}</b>
            <span className="syn-grow" />
            <span className="fi-card-count">{n}</span>
            <Filter size={13} className="muted" />
          </button>
        ))}
      </div>
      <div className="fi-table-box">
        <div className="fi-toolbar">
          <span>
            <b>{rows.length}</b> Signals
          </span>
          <span className="syn-toolbar-sep" />
          <Menu
            triggerClassName={`link-btn${selected.size ? '' : ' disabled'}`}
            trigger={
              <>
                Triage <ChevronDown size={12} />
              </>
            }
            items={
              selected.size
                ? [
                    { label: 'Open', onClick: () => bulkState('open') },
                    { label: 'Under Review', onClick: () => bulkState('under_review') },
                    ...ARCHIVE_REASONS.map(([v, l]) => ({ label: `Archive: ${l}`, onClick: () => bulkState('archived', v) })),
                  ]
                : [{ label: 'Select signals first', onClick: () => undefined }]
            }
          />
          {selected.size > 0 && <span className="muted">{selected.size} selected</span>}
        </div>
        {signals.isLoading ? (
          <Loading />
        ) : signals.isError ? (
          <ErrorState error={signals.error} onRetry={() => signals.refetch()} />
        ) : !rows.length ? (
          <div className="fi-empty">
            <Search size={40} />
            <p>No signals match.</p>
            <span className="muted">Signals appear when the application security detection rules match your security traces.</span>
          </div>
        ) : (
          <table className="mon-table syn-table fi-signals">
            <thead>
              <tr>
                <th className="sec-check">
                  <input type="checkbox" checked={rows.every((s) => selected.has(s.id))} onChange={(e) => setSelected(e.target.checked ? new Set(rows.map((s) => s.id)) : new Set())} />
                </th>
                <th>SEVERITY</th>
                <th>WHEN</th>
                <th>WHAT</th>
                <th>WHERE</th>
                <th>WHO</th>
              </tr>
            </thead>
            <tbody>
              {rows.map((s) => {
                const ip = entity(s, 'ip');
                return (
                  <tr key={s.id} className={s.id === open ? 'selected' : ''} onClick={() => set('signal', s.id)}>
                    <td className="sec-check" onClick={(e) => e.stopPropagation()}>
                      <input type="checkbox" checked={selected.has(s.id)} onChange={() => toggle(s.id)} />
                    </td>
                    <td>
                      <SeverityBadge severity={s.severity} />
                    </td>
                    <td className="nowrap">
                      {new Date(s.createdAt).toLocaleString(undefined, { month: 'short', day: 'numeric', hour: 'numeric', minute: '2-digit', second: '2-digit' })}
                      <div className="muted sec-sub">Last seen: {timeAgo(s.lastSeen)}</div>
                    </td>
                    <td>
                      {s.state !== 'open' && <span className="fi-state">{(STATE_LABEL[s.state] ?? s.state).toUpperCase()}</span>}
                      {s.assignee && (
                        <span className="fi-state">
                          <Users size={11} /> {s.assignee.name.toUpperCase()}
                        </span>
                      )}
                      <b className="sec-title">{s.title}</b>
                    </td>
                    <td className="nowrap fi-where">
                      {s.services.slice(0, 1).map((x) => (
                        <span key={x} className="fi-service">
                          <Globe size={12} /> {x}
                        </span>
                      ))}
                      {s.env && <span className="tag-chip">env:{s.env}</span>}
                    </td>
                    <td className="nowrap">{ip && ip.count > 1 ? <span className="fi-chip">{compact(ip.count)} IPs</span> : ip?.top[0] ? <span className="fi-chip mono">{ip.top[0]}</span> : null}</td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        )}
      </div>
      {open && createPortal(<AppsecSignalPanel id={open} onClose={() => set('signal', null)} />, document.body)}
    </div>
  );
}

const WINDOW = 15 * MIN;

/** One signal: what happened (service, attackers, trigger, security traces) and the next steps. */
function AppsecSignalPanel({ id, onClose }: { id: string; onClose: () => void }) {
  const q = useSignal(id);
  const state = useSignalState();
  const assign = useSignalAssignee();
  const block = useBlock();
  const users = useUsers({ limit: 200, offset: 0, statuses: ['active'] });
  const [summaryOpen, setSummaryOpen] = useState(true);
  const s = q.data;
  const range = useMemo(() => (s ? { from: s.firstSeen - WINDOW, to: Math.max(s.lastSeen, s.firstSeen) + WINDOW } : { from: 0, to: 0 }), [s]);
  const traces = useSecurityTraces(range, s?.logsQuery ?? '');
  const members = (users.data?.users ?? []).map((u) => ({ id: u.id, name: u.name || u.email }));
  const ip = s ? entity(s, 'ip') : undefined;
  const chart = useMemo<EChartsOption>(
    () => ({
      grid: { left: 36, right: 12, top: 10, bottom: 22 },
      tooltip: { trigger: 'axis' },
      xAxis: { type: 'time', min: range.from, max: range.to },
      yAxis: { type: 'value', minInterval: 1, splitNumber: 3 },
      series: [
        {
          name: 'a',
          type: 'bar',
          barMaxWidth: 10,
          itemStyle: { color: token('--chart-1') },
          data: (traces.data?.histogram ?? []).map((b) => [b.ts, Object.values(b.counts).reduce((x, y) => x + y, 0)]),
          markArea: s ? { silent: true, itemStyle: { color: 'rgba(80, 140, 230, 0.08)' }, data: [[{ xAxis: s.firstSeen }, { xAxis: Math.max(s.lastSeen, s.firstSeen + MIN) }]] } : undefined,
        },
      ],
    }),
    [traces.data, range, s],
  );
  const endpoints = useMemo(() => {
    const m = new Map<string, { service: string; env: string; method: string; route: string; status: string; errors: number; n: number }>();
    for (const t of traces.data?.traces ?? []) {
      const k = `${t.service} ${t.method} ${t.route}`;
      const e = m.get(k) ?? { service: t.service, env: t.env, method: t.method, route: t.route, status: t.status, errors: 0, n: 0 };
      e.n++;
      if (Number(t.status) >= 500) e.errors++;
      m.set(k, e);
    }
    return [...m.values()].sort((a, b) => b.n - a.n);
  }, [traces.data]);
  const blockable = ip?.top ?? [];
  return (
    <div className="sec-panel-backdrop" onMouseDown={(e) => e.target === e.currentTarget && onClose()}>
      <aside className="sec-panel fi-panel" role="dialog" aria-label="Signal">
        {q.isLoading ? (
          <Loading />
        ) : q.isError ? (
          <ErrorState error={q.error} />
        ) : (
          s && (
            <>
              <header className="sec-panel-head">
                <span className="muted">
                  <Siren size={14} /> Threat · {new Date(s.createdAt).toLocaleString()} <em className="sec-ago">{timeAgo(s.createdAt)}</em>
                </span>
                <span className="syn-grow" />
                {q.data?.ruleExists && (
                  <Link className="btn" to={`/security/siem/rules/${s.ruleId}`}>
                    Explore Rule <ArrowRight size={13} />
                  </Link>
                )}
                <button className="btn btn-ghost" onClick={onClose} aria-label="Close">
                  <X size={16} />
                </button>
              </header>
              <div className="sec-panel-title">
                <SeverityBadge severity={s.severity} />
                <h2>{s.title}</h2>
              </div>
              <p className="muted sec-panel-counts">
                Past month signal count: <b>{q.data!.ruleSignalsMonth}</b> <span className="sec-sep" /> Past day signal count: <b>{q.data!.ruleSignalsDay}</b>
              </p>
              <div className="sec-panel-body fi-panel-body">
                <section className="sec-what fi-what">
                  <h4>WHAT HAPPENED</h4>
                  <dl>
                    <dt>SERVICE</dt>
                    <dd>
                      {s.services.map((x) => (
                        <span key={x} className="fi-service">
                          <Globe size={12} /> {x}
                        </span>
                      ))}
                      {s.env && <span className="tag-chip">env:{s.env}</span>}
                    </dd>
                    {ip && ip.count > 0 && (
                      <>
                        <dt className="sec-attackers">ATTACKERS</dt>
                        <dd>
                          <span className="fi-chip">{compact(ip.count)} IPs</span> targeted this service
                          <div className="fi-ip-list">
                            {ip.top.map((v) => (
                              <Link key={v} className="fi-chip mono" to={`/security/appsec/investigate/attackers?q=${encodeURIComponent(`@http.client_ip:${v}`)}`}>
                                {v}
                              </Link>
                            ))}
                            {ip.count > ip.top.length && <span className="fi-chip">+{ip.count - ip.top.length}</span>}
                          </div>
                        </dd>
                      </>
                    )}
                    <dt>TRIGGER</dt>
                    <dd>
                      <div className="fi-trigger">
                        Triggers when <u>query {s.condition || 'a > 0'}</u>
                        {s.caseName && <span className="muted"> ({s.caseName})</span>}
                        <span className="syn-grow" />
                        {q.data?.ruleExists && (
                          <Link to={`/security/siem/rules/${s.ruleId}`} aria-label="Edit rule">
                            <Pencil size={13} />
                          </Link>
                        )}
                      </div>
                    </dd>
                    <dt>SECURITY TRACES</dt>
                    <dd>
                      <span className="fi-chip">{compact(traces.data?.total ?? s.eventsMatched)} a</span>
                      <div className="sec-panel-chart">{traces.isLoading ? <Loading /> : <Chart option={chart} height={150} />}</div>
                    </dd>
                  </dl>
                  {s.message && <p className="sec-message">{s.message}</p>}
                </section>
                <aside className="fi-next">
                  <h4>NEXT STEPS</h4>
                  <div className="fi-next-block">
                    <span>Triage</span>
                    <Menu
                      align="left"
                      triggerClassName={`btn sec-state-btn state-${s.state}`}
                      trigger={
                        <>
                          {(STATE_LABEL[s.state] ?? s.state).toUpperCase()} <ChevronDown size={13} />
                        </>
                      }
                      items={[
                        { label: 'Open', onClick: () => state.mutate({ id: s.id, input: { state: 'open' } }) },
                        { label: 'Under Review', onClick: () => state.mutate({ id: s.id, input: { state: 'under_review' } }) },
                        ...ARCHIVE_REASONS.map(([v, l]) => ({ label: `Archive: ${l}`, onClick: () => state.mutate({ id: s.id, input: { state: 'archived', archiveReason: v } }) })),
                      ]}
                    />
                    <Menu
                      align="left"
                      triggerClassName="btn"
                      trigger={
                        <>
                          <UserPlus size={14} /> {s.assignee ? s.assignee.name : 'Assign Signal'}
                        </>
                      }
                      items={[
                        ...members.map((m) => ({ label: m.name, onClick: () => assign.mutate({ id: s.id, assigneeId: m.id }) })),
                        ...(s.assignee ? [{ label: 'Unassign', onClick: () => assign.mutate({ id: s.id, assigneeId: null }) }] : []),
                      ]}
                    />
                  </div>
                  <div className="fi-next-block">
                    <span>Respond</span>
                    <button
                      className="btn btn-primary"
                      disabled={!blockable.length || block.isPending || block.isSuccess}
                      title={blockable.length ? `Adds ${blockable.join(', ')} to the denylist` : 'This signal has no attacking IPs'}
                      onClick={() => block.mutate({ kind: 'ip', values: blockable, reason: s.title })}
                    >
                      <Hand size={14} /> {block.isSuccess ? `Blocked ${blockable.length} IP${blockable.length === 1 ? '' : 's'}` : 'Block Attacking IPs'}
                    </button>
                    <Link className="btn" to={`/security/appsec/investigate?q=${encodeURIComponent(s.logsQuery)}`}>
                      <Search size={14} /> View security traces
                    </Link>
                  </div>
                  <div className="fi-next-block">
                    <span>Protect</span>
                    <Link className="btn" to="/security/appsec/policies/denylist">
                      <Hand size={14} /> Open Denylist
                    </Link>
                  </div>
                </aside>
              </div>
              <section className="fi-activity">
                <button className="fi-activity-head" onClick={() => setSummaryOpen((o) => !o)}>
                  {summaryOpen ? <ChevronDown size={15} /> : <ChevronRight size={15} />} Activity Summary
                </button>
                {summaryOpen &&
                  (traces.isLoading ? (
                    <Loading />
                  ) : !endpoints.length ? (
                    <p className="muted">No security traces of this signal are left in the time range.</p>
                  ) : (
                    <div className="fi-activity-box">
                      <div className="fi-activity-svc">
                        <span className="fi-service">
                          <Globe size={12} /> {endpoints[0].service}
                        </span>
                        {endpoints[0].env && <span className="tag-chip">env:{endpoints[0].env}</span>}
                      </div>
                      <table className="mon-table syn-table">
                        <thead>
                          <tr>
                            <th>HTTP STATUS</th>
                            <th>API</th>
                            <th className="num">ERRORS</th>
                            <th className="num">SECURITY TRACES</th>
                          </tr>
                        </thead>
                        <tbody>
                          {endpoints.slice(0, 8).map((e) => (
                            <tr key={`${e.service}${e.method}${e.route}`}>
                              <td>
                                <span className={`fi-status s${e.status[0] ?? ''}`}>{e.status}</span>
                              </td>
                              <td>
                                <MethodBadge method={e.method} /> <span className="mono">{e.route}</span>
                              </td>
                              <td className="num">{e.errors}</td>
                              <td className="num">{compact(e.n)}</td>
                            </tr>
                          ))}
                        </tbody>
                      </table>
                    </div>
                  ))}
              </section>
              <footer className="fi-panel-foot">
                <Link className="btn btn-primary" to={`/security/appsec/investigate?q=${encodeURIComponent(s.logsQuery)}`}>
                  <Search size={14} /> Investigate in full screen
                </Link>
              </footer>
            </>
          )
        )}
      </aside>
    </div>
  );
}
