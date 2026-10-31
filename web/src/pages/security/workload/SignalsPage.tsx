import { useMemo, useState } from 'react';
import { Link } from 'react-router-dom';
import type { EChartsOption } from 'echarts';
import { ArrowRight, ChevronDown, ChevronRight, Container, Pencil, Search, Server, ShieldCheck, Siren, UserPlus, X } from 'lucide-react';
import type { SecuritySignal } from '../../../api/security/client';
import { Chart } from '../../../components/charts/Chart';
import { TimeProvider } from '../../../components/shell/TimeContext';
import { Menu } from '../../../components/ui/Dropdown';
import { ErrorState, Loading } from '../../../components/ui/AsyncState';
import { compact, timeAgo } from '../../../lib/format';
import { token } from '../../../theme/chartTheme';
import { useUsers } from '../../settings/queries';
import { ARCHIVE_REASONS, SeverityBadge, STATE_LABEL } from '../common';
import { useSignal, useSignalAssignee, useSignalState } from '../queries';
import { entity, SignalsExplorer, type SignalsConfig } from '../appsec/SignalsPage';
import { useWpEvents } from './exploreQueries';
import './explore.css';

const MIN = 60_000;
const WINDOW = 15 * MIN;
const BASE = '/security/workload-protection';

/** The host a signal is about: its host entity, else its group-by value. */
function hostOf(s: SecuritySignal): string {
  return entity(s, 'host')?.top[0] ?? (s.groupValues as Record<string, string> | undefined)?.host ?? '';
}

export function HostChip({ host }: { host: string }) {
  return (
    <span className="wpx-host">
      <Server size={12} /> {host}
    </span>
  );
}

const WP_SIGNALS: SignalsConfig = {
  scope: '@workflow.rule.type:workload_security',
  defaultSeverity: 'critical,high,medium',
  filters: [
    {
      key: 'host',
      label: 'Host',
      options: (all) => [...new Set(all.map(hostOf).filter(Boolean))].sort().map((h) => [h, h]),
      match: (s, v) => hostOf(s) === v,
    },
  ],
  cards: ['critical', 'mine', 'open'],
  searchPlaceholder: 'Search signals: @workflow.rule.name:… host:…',
  emptyHint: 'Signals appear when the Workload Protection detection rules match the events of your Agents.',
  where: (s) => {
    const host = hostOf(s);
    return (
      <>
        {host && <HostChip host={host} />}
        {s.env && <span className="tag-chip">env:{s.env}</span>}
      </>
    );
  },
  who: (s) => {
    const user = entity(s, 'user_name') ?? entity(s, 'user');
    const container = entity(s, 'container');
    const ip = entity(s, 'ip');
    return (
      <span className="wpx-who">
        {container?.top[0] && (
          <span className="fi-chip" title="Container">
            <Container size={11} /> {container.top[0]}
          </span>
        )}
        {user?.top[0] && <span className="fi-chip">{user.top[0]}</span>}
        {ip?.top[0] && <span className="fi-chip mono">{ip.count > 1 ? `${compact(ip.count)} IPs` : ip.top[0]}</span>}
      </span>
    );
  },
  panel: (id, onClose) => <WorkloadSignalPanel id={id} onClose={onClose} />,
};

/** Workload Protection → Signals: the signals of the workload security detection rules, critical to medium over the past week. */
export function WorkloadSignalsPage() {
  return (
    <TimeProvider initial="1w">
      <SignalsExplorer config={WP_SIGNALS} />
    </TimeProvider>
  );
}

const eventsLink = (q: string) => `${BASE}/investigate?q=${encodeURIComponent(q)}`;

/** One signal: the host, what the Agent saw (trigger, matching events) and the next steps. */
function WorkloadSignalPanel({ id, onClose }: { id: string; onClose: () => void }) {
  const q = useSignal(id);
  const state = useSignalState();
  const assign = useSignalAssignee();
  const users = useUsers({ limit: 200, offset: 0, statuses: ['active'] });
  const [summaryOpen, setSummaryOpen] = useState(true);
  const s = q.data;
  const range = useMemo(() => (s ? { from: s.firstSeen - WINDOW, to: Math.max(s.lastSeen, s.firstSeen) + WINDOW } : { from: 0, to: 0 }), [s]);
  const events = useWpEvents(range, s?.logsQuery ?? '', 50);
  const members = (users.data?.users ?? []).map((u) => ({ id: u.id, name: u.name || u.email }));
  const chart = useMemo<EChartsOption>(() => {
    const ser = events.data?.series;
    const times = ser?.times ?? [];
    const total = times.map((_, i) => (ser?.groups ?? []).reduce((a, g) => a + (g.values[i] ?? 0), 0));
    return {
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
          data: times.map((t, i) => [t, total[i]]),
          markArea: s ? { silent: true, itemStyle: { color: 'rgba(80, 140, 230, 0.08)' }, data: [[{ xAxis: s.firstSeen }, { xAxis: Math.max(s.lastSeen, s.firstSeen + MIN) }]] } : undefined,
        },
      ],
    };
  }, [events.data, range, s]);
  const host = s ? hostOf(s) : '';
  const container = s ? entity(s, 'container') : undefined;
  const rows = events.data?.events ?? [];
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
                    <dt>HOST</dt>
                    <dd>
                      {host ? <HostChip host={host} /> : <span className="muted">—</span>}
                      {s.env && <span className="tag-chip">env:{s.env}</span>}
                      {s.services
                        .filter((x) => x !== 'runtime-security-agent')
                        .map((x) => (
                          <span key={x} className="tag-chip">
                            service:{x}
                          </span>
                        ))}
                    </dd>
                    {container && container.count > 0 && (
                      <>
                        <dt>CONTAINER</dt>
                        <dd>
                          {container.top.map((c) => (
                            <span key={c} className="fi-chip">
                              <Container size={11} /> {c}
                            </span>
                          ))}
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
                    <dt>AGENT EVENTS</dt>
                    <dd>
                      <span className="fi-chip">{compact(events.data?.total ?? s.eventsMatched)} a</span>
                      <div className="sec-panel-chart">{events.isLoading ? <Loading /> : <Chart option={chart} height={150} />}</div>
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
                    <Link className="btn btn-primary" to={eventsLink(s.logsQuery)}>
                      <Search size={14} /> View agent events
                    </Link>
                    {host && (
                      <Link className="btn" to={`/infrastructure?host=${encodeURIComponent(host)}`}>
                        <Server size={14} /> View host
                      </Link>
                    )}
                  </div>
                  <div className="fi-next-block">
                    <span>Protect</span>
                    <Link className="btn" to={`${BASE}/policies`}>
                      <ShieldCheck size={14} /> Agent policies
                    </Link>
                  </div>
                </aside>
              </div>
              <section className="fi-activity">
                <button className="fi-activity-head" onClick={() => setSummaryOpen((o) => !o)}>
                  {summaryOpen ? <ChevronDown size={15} /> : <ChevronRight size={15} />} Activity Summary
                </button>
                {summaryOpen &&
                  (events.isLoading ? (
                    <Loading />
                  ) : !rows.length ? (
                    <p className="muted">No agent events of this signal are left in the time range.</p>
                  ) : (
                    <div className="fi-activity-box">
                      <table className="mon-table syn-table">
                        <thead>
                          <tr>
                            <th>DATE</th>
                            <th>EVENT</th>
                            <th>PROCESS</th>
                            <th>FILE / TARGET</th>
                            <th>USER</th>
                          </tr>
                        </thead>
                        <tbody>
                          {rows.slice(0, 8).map((e) => (
                            <tr key={e.id}>
                              <td className="nowrap">{new Date(e.timestamp).toLocaleTimeString()}</td>
                              <td>
                                <span className="wpx-evt">{e.event}</span>
                              </td>
                              <td className="mono wpx-ellipsis" title={`${e.process} ${e.processArgs}`}>
                                {e.process} <span className="muted">{e.processArgs}</span>
                              </td>
                              <td className="mono wpx-ellipsis">{e.file}</td>
                              <td>{e.user}</td>
                            </tr>
                          ))}
                        </tbody>
                      </table>
                    </div>
                  ))}
              </section>
              <footer className="fi-panel-foot">
                <Link className="btn btn-primary" to={eventsLink(s.logsQuery)}>
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
