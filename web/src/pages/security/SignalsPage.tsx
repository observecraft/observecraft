import { useMemo, useState } from 'react';
import { createPortal } from 'react-dom';
import { Link, useSearchParams } from 'react-router-dom';
import type { EChartsOption } from 'echarts';
import { ArrowRight, ChevronDown, ExternalLink, FileSearch, PanelLeftClose, PanelLeftOpen, Search, Siren, UserPlus, X } from 'lucide-react';
import type { SecuritySignal, SignalEntity } from '../../api/security/client';
import type { Facet } from '../../api/types';
import { Chart } from '../../components/charts/Chart';
import { FacetPanel } from '../../components/facets/FacetPanel';
import { TimePicker } from '../../components/shell/PageHeader';
import { useTime } from '../../components/shell/TimeContext';
import { Menu } from '../../components/ui/Dropdown';
import { ErrorState, Loading } from '../../components/ui/AsyncState';
import { compact, timeAgo } from '../../lib/format';
import { formatQuery, parseQuery } from '../../lib/searchQuery';
import { token } from '../../theme/chartTheme';
import { useUsers } from '../settings/queries';
import { ARCHIVE_REASONS, SeverityBadge, STATE_LABEL } from './common';
import { useBulkSignals, useSignal, useSignalAssignee, useSignalLogs, useSignals, useSignalState } from './queries';

const MIN = 60_000;
const SEVERITY_COLORS: Record<string, string> = { critical: '#8b1a1a', high: '#d33', medium: '#f0883e', low: '#e8b931', info: '#8792a7' };
const FACET_LABELS: Record<string, [string, string]> = {
  '@workflow.triage.state': ['Signal State', 'TRIAGE'],
  assignee: ['Assignee', 'TRIAGE'],
  status: ['Severity', 'CORE'],
  source: ['Source', 'CORE'],
  '@workflow.rule.type': ['Rule Type', 'CORE'],
  '@workflow.rule.name': ['Rule', 'CORE'],
  service: ['Service', 'CORE'],
  env: ['Env', 'CORE'],
  tactic: ['Tactic', 'MITRE ATT&CK'],
};
const TABS: [string, string][] = [
  ['', 'All'],
  ['open', 'Open'],
  ['under_review', 'Under Review'],
  ['archived', 'Archived'],
];

/** Signals Explorer (Security → Signals; Cloud SIEM → Signals with `scope`). */
export function SignalsPage({ scope = '' }: { scope?: string }) {
  const [params, setParams] = useSearchParams();
  const { range } = useTime();
  const query = params.get('q') ?? '';
  const state = params.get('state') ?? '';
  const open = params.get('signal');
  const [draft, setDraft] = useState(query);
  const [controls, setControls] = useState(true);
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
  const signals = useSignals({ range: r, query: [scope, query].filter(Boolean).join(' '), state, limit: 200 });
  const bulk = useBulkSignals();
  const data = signals.data;
  const terms = parseQuery(query);
  const facets: Facet[] = (data?.facets ?? []).map((f) => ({ key: f.key, label: FACET_LABELS[f.key]?.[0] ?? f.key, group: FACET_LABELS[f.key]?.[1] ?? 'CORE', values: f.values }));
  const histogram = useMemo<EChartsOption>(
    () => ({
      grid: { left: 30, right: 12, top: 10, bottom: 22 },
      tooltip: { trigger: 'axis' },
      xAxis: { type: 'time', min: r.from, max: r.to },
      yAxis: { type: 'value', minInterval: 1, splitNumber: 2 },
      series: (['info', 'low', 'medium', 'high', 'critical'] as const).map((sev) => ({
        name: sev,
        type: 'bar',
        stack: 's',
        barMaxWidth: 14,
        itemStyle: { color: SEVERITY_COLORS[sev] },
        data: (data?.histogram ?? []).map((b) => [b.ts, b[sev]]),
      })),
    }),
    [data, r.from, r.to],
  );
  const rows = data?.signals ?? [];
  const toggle = (id: string) =>
    setSelected((s) => {
      const n = new Set(s);
      if (n.has(id)) n.delete(id);
      else n.add(id);
      return n;
    });
  const bulkState = (s: string, archiveReason?: string) => bulk.mutate({ ids: [...selected], state: { state: s, archiveReason } }, { onSuccess: () => setSelected(new Set()) });
  return (
    <div className="sec-explorer">
      <div className="sec-explorer-top">
        <h2 className="sec-h">Signals Explorer</h2>
        <span className="syn-grow" />
        <TimePicker compact />
      </div>
      <form
        className="sec-search"
        onSubmit={(e) => {
          e.preventDefault();
          set('q', draft.trim() || null);
        }}
      >
        <Search size={15} />
        <input placeholder="Filter your signals: status:high source:cloudtrail @usr.email:…" value={draft} onChange={(e) => setDraft(e.target.value)} onBlur={() => set('q', draft.trim() || null)} />
      </form>
      <section className="sec-histogram">
        <Chart option={histogram} height={110} />
      </section>
      <div className={`syn-explorer-body${controls ? '' : ' no-controls'}`}>
        {controls && (
          <FacetPanel
            facets={facets}
            terms={terms}
            onChange={(t) => (setDraft(formatQuery(t)), set('q', formatQuery(t) || null))}
            defaultOpen={['@workflow.triage.state', 'status', 'source']}
            verified={[]}
            noun="facets"
            valueLabel={(k, v) => (k === '@workflow.triage.state' ? (STATE_LABEL[v] ?? v) : v)}
          />
        )}
        <div className="syn-explorer-main">
          <div className="syn-toolbar">
            <button className="link-btn" onClick={() => setControls((c) => !c)}>
              {controls ? <PanelLeftClose size={15} /> : <PanelLeftOpen size={15} />} {controls ? 'Hide Controls' : 'Show Controls'}
            </button>
            <span className="syn-toolbar-sep" />
            <span>
              <b>{compact(data?.total ?? 0)}</b> signal{data?.total === 1 ? '' : 's'} found
            </span>
            <span className="syn-grow" />
            {selected.size > 0 && (
              <>
                <span className="muted">{selected.size} selected</span>
                <Menu
                  triggerClassName="btn"
                  trigger={
                    <>
                      Set state <ChevronDown size={13} />
                    </>
                  }
                  items={[
                    { label: 'Open', onClick: () => bulkState('open') },
                    { label: 'Under Review', onClick: () => bulkState('under_review') },
                    ...ARCHIVE_REASONS.map(([v, l]) => ({ label: `Archive: ${l}`, onClick: () => bulkState('archived', v) })),
                  ]}
                />
              </>
            )}
          </div>
          <nav className="sec-tabs">
            {TABS.map(([v, l]) => (
              <button key={v} className={state === v ? 'active' : ''} onClick={() => set('state', v || null)}>
                {l} <span className="sec-count">{v ? (data?.states[v] ?? 0) : Object.values(data?.states ?? {}).reduce((a, b) => a + b, 0)}</span>
              </button>
            ))}
          </nav>
          {signals.isLoading ? (
            <Loading />
          ) : signals.isError ? (
            <ErrorState error={signals.error} onRetry={() => signals.refetch()} />
          ) : !rows.length ? (
            <div className="sec-empty">
              <b>No signals match.</b>
              <p className="muted">Signals appear when detection rules match your logs. Widen the time range, change the search, or check the content packs.</p>
            </div>
          ) : (
            <div className="mon-table-wrap">
              <table className="mon-table syn-table sec-signals">
                <thead>
                  <tr>
                    <th className="sec-check">
                      <input type="checkbox" checked={rows.every((s) => selected.has(s.id))} onChange={(e) => setSelected(e.target.checked ? new Set(rows.map((s) => s.id)) : new Set())} />
                    </th>
                    <th>SEVERITY</th>
                    <th>CREATION DATE</th>
                    <th>TITLE</th>
                    <th>STATE</th>
                    <th>ASSIGNEE</th>
                  </tr>
                </thead>
                <tbody>
                  {rows.map((s) => (
                    <tr key={s.id} className={s.id === open ? 'selected' : ''} onClick={() => set('signal', s.id)}>
                      <td className="sec-check" onClick={(e) => e.stopPropagation()}>
                        <input type="checkbox" checked={selected.has(s.id)} onChange={() => toggle(s.id)} />
                      </td>
                      <td>
                        <SeverityBadge severity={s.severity} />
                      </td>
                      <td className="nowrap">
                        {new Date(s.createdAt).toLocaleString(undefined, { month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit', second: '2-digit' })}
                        <div className="muted sec-sub">Last seen: {timeAgo(s.lastSeen)}</div>
                      </td>
                      <td>
                        <b className="sec-title">{s.title}</b>
                        <div className="sec-tags">
                          {s.tags
                            .filter((t) => /^(source|tactic|technique|service|env):/.test(t))
                            .slice(0, 4)
                            .map((t) => (
                              <span key={t} className="tag-chip">
                                {t}
                              </span>
                            ))}
                        </div>
                      </td>
                      <td className="nowrap">{STATE_LABEL[s.state] ?? s.state}</td>
                      <td className="nowrap">{s.assignee?.name ?? <span className="muted">Unassigned</span>}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          )}
        </div>
      </div>
      {open && createPortal(<SignalPanel id={open} onClose={() => set('signal', null)} />, document.body)}
    </div>
  );
}

const WINDOW = 15 * MIN;

/** The side panel of one signal: what happened, the next steps and its logs. */
export function SignalPanel({ id, onClose }: { id: string; onClose: () => void }) {
  const q = useSignal(id);
  const state = useSignalState();
  const assign = useSignalAssignee();
  const users = useUsers({ limit: 200, offset: 0, statuses: ['active'] });
  const s = q.data;
  const range = useMemo(() => (s ? { from: s.firstSeen - WINDOW, to: Math.max(s.lastSeen, s.firstSeen) + WINDOW } : { from: 0, to: 0 }), [s]);
  const logs = useSignalLogs(s?.logsQuery, range);
  const members = (users.data?.users ?? []).map((u) => ({ id: u.id, name: u.name || u.email }));
  const chart = useMemo<EChartsOption>(
    () => ({
      grid: { left: 36, right: 12, top: 10, bottom: 22 },
      tooltip: { trigger: 'axis' },
      xAxis: { type: 'time', min: range.from, max: range.to },
      yAxis: { type: 'value', minInterval: 1, splitNumber: 3 },
      series: [
        { name: 'a', type: 'bar', barMaxWidth: 10, itemStyle: { color: token('--chart-1') }, data: (logs.histogram.data ?? []).map((b) => [b.t, Object.values(b.counts).reduce((x, y) => x + y, 0)]) },
      ],
    }),
    [logs.histogram.data, range],
  );
  return (
    <div className="sec-panel-backdrop" onMouseDown={(e) => e.target === e.currentTarget && onClose()}>
      <aside className="sec-panel" role="dialog" aria-label="Signal">
        {q.isLoading ? (
          <Loading />
        ) : q.isError ? (
          <ErrorState error={q.error} />
        ) : (
          s && (
            <>
              <header className="sec-panel-head">
                <span className="muted">
                  <Siren size={14} /> {s.ruleType === 'log_detection' ? 'Log Detection' : s.ruleType.replace(/_/g, ' ')} · {new Date(s.createdAt).toLocaleString()}{' '}
                  <em className="sec-ago">{timeAgo(s.createdAt)}</em>
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
              <div className="sec-panel-body">
                <section className="sec-what">
                  <h4>WHAT HAPPENED</h4>
                  <dl>
                    <dt>SERVICE</dt>
                    <dd>
                      {s.services.map((x) => (
                        <span key={x} className="tag-chip">
                          {x}
                        </span>
                      ))}
                      {s.env && <span className="tag-chip">env:{s.env}</span>}
                      {s.source && <span className="tag-chip">source:{s.source}</span>}
                    </dd>
                    {Object.keys(s.groupValues).length > 0 && (
                      <>
                        <dt>GROUP</dt>
                        <dd>
                          {Object.entries(s.groupValues).map(([k, v]) => (
                            <span key={k} className="tag-chip mono">
                              {k}:{v}
                            </span>
                          ))}
                        </dd>
                      </>
                    )}
                    <Entities entities={s.entities as unknown as Record<string, SignalEntity>} />
                    <dt>TRIGGER</dt>
                    <dd className="sec-trigger">
                      Triggers when <u>{s.condition || 'the query matches'}</u>
                      {s.caseName && <span className="muted"> ({s.caseName})</span>}
                    </dd>
                    <dt>MATCHED LOGS</dt>
                    <dd>
                      <span className="tag-chip">
                        {compact(s.eventsMatched)} {s.queries[0]?.name ?? 'a'}
                      </span>
                      <div className="sec-panel-chart">{logs.histogram.isLoading ? <Loading /> : <Chart option={chart} height={150} />}</div>
                    </dd>
                  </dl>
                  {s.message && <p className="sec-message">{s.message}</p>}
                </section>
                <aside className="sec-next">
                  <h4>NEXT STEPS</h4>
                  <div className="sec-next-block">
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
                    {s.state === 'archived' && <span className="muted sec-sub">Reason: {ARCHIVE_REASONS.find(([v]) => v === s.archiveReason)?.[1] ?? s.archiveReason}</span>}
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
                    {s.stateUpdatedBy && (
                      <span className="muted sec-sub">
                        Updated by {s.stateUpdatedBy} {s.stateUpdatedAt ? timeAgo(s.stateUpdatedAt) : ''}
                      </span>
                    )}
                  </div>
                  <div className="sec-next-block">
                    <span>Respond</span>
                    <Link className="btn" to={`/logs?query=${encodeURIComponent(s.logsQuery)}&from=${range.from}&to=${range.to}`}>
                      <FileSearch size={14} /> View related logs
                    </Link>
                    <Link className="btn" to="/incidents">
                      <Siren size={14} /> Declare an incident <ExternalLink size={12} />
                    </Link>
                  </div>
                </aside>
              </div>
              <section className="sec-samples">
                <h4>Activity Summary</h4>
                {logs.samples.isLoading ? (
                  <Loading />
                ) : !logs.samples.data?.logs.length ? (
                  <p className="muted">No logs of this signal are left in the index you can read.</p>
                ) : (
                  <table className="mon-table syn-table">
                    <thead>
                      <tr>
                        <th>DATE</th>
                        <th>HOST / IP</th>
                        <th>MESSAGE</th>
                      </tr>
                    </thead>
                    <tbody>
                      {logs.samples.data.logs.map((l) => (
                        <tr key={l.id}>
                          <td className="nowrap">{new Date(l.timestamp).toLocaleTimeString()}</td>
                          <td className="mono">{String((l.attributes.network as { client?: { ip?: string } } | undefined)?.client?.ip ?? l.host ?? '')}</td>
                          <td className="sec-log-msg">{l.message}</td>
                        </tr>
                      ))}
                    </tbody>
                  </table>
                )}
              </section>
            </>
          )
        )}
      </aside>
    </div>
  );
}

const ENTITY_LABELS: [string, string, string][] = [
  ['ip', 'ATTACKERS', 'IPs'],
  ['user', 'USERS', 'users'],
  ['user_name', 'USERS', 'users'],
  ['host', 'HOSTS', 'hosts'],
  ['country', 'COUNTRIES', 'countries'],
];

function Entities({ entities }: { entities: Record<string, SignalEntity> }) {
  return (
    <>
      {ENTITY_LABELS.filter(([k]) => entities[k]?.count).map(([k, label, noun]) => (
        <div key={k} className="sec-entity">
          <dt className={k === 'ip' ? 'sec-attackers' : ''}>{label}</dt>
          <dd>
            <span className="tag-chip">
              {compact(entities[k].count)} {noun}
            </span>{' '}
            {entities[k].top.map((v) => (
              <span key={v} className="tag-chip mono">
                {v}
              </span>
            ))}
            {entities[k].count > entities[k].top.length && <span className="tag-chip">+{entities[k].count - entities[k].top.length}</span>}
          </dd>
        </div>
      ))}
    </>
  );
}

// App & API Protection → Signals has its own columns and panel (attackers, security traces, blocking).
export { AppsecSignalsPage } from './appsec/SignalsPage';

export function SiemSignalsPage() {
  return <SignalsPage scope="@workflow.rule.type:log_detection" />;
}

export type { SecuritySignal };
