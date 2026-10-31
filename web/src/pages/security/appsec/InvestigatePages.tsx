import { useMemo, useState, type ReactNode } from 'react';
import { createPortal } from 'react-dom';
import { Link, useSearchParams } from 'react-router-dom';
import type { EChartsOption } from 'echarts';
import { BarChart3, ChartPie, Code2, Copy, Download, ExternalLink, Filter, Globe2, Grid2x2, Hand, LineChart, List, ListOrdered, Search, Shield, ShieldPlus, Table2, Tag, User, X } from 'lucide-react';
import type { AppsecUser, Attacker, SecurityTrace } from '../../../api/appsec/client';
import type { Facet } from '../../../api/types';
import { Chart } from '../../../components/charts/Chart';
import { FacetPanel } from '../../../components/facets/FacetPanel';
import { TimePicker } from '../../../components/shell/PageHeader';
import { useTime } from '../../../components/shell/TimeContext';
import { Empty, ErrorState, Loading } from '../../../components/ui/AsyncState';
import { compact, timeAgo } from '../../../lib/format';
import { formatQuery, parseQuery, type Term } from '../../../lib/searchQuery';
import { token } from '../../../theme/chartTheme';
import { AapTabs, INVESTIGATE_TABS, MethodBadge } from './AppsecLayout';
import { countryName, downloadCsv, ExplorerToolbar, flag, IpChip, localFacets, MethodPath, ServiceChip } from './common';
import { useAppsecUsers, useAttackers, useBlock, useDenylist, useSaveWaf, useSecurityTraces, useUnblock } from './queries';

const MIN = 60_000;
const PALETTE = ['--chart-1', '--chart-2', '--chart-3', '--chart-4', '--chart-5', '--chart-6', '--chart-7', '--chart-8'];
const FACET_LABEL: Record<string, string> = {
  service: 'Service',
  env: 'Env',
  '@appsec.type': 'Attack Type',
  '@appsec.category': 'Category',
  '@http.status_code': 'Status Code',
  '@appsec.blocked': 'Blocked',
  '@http.client_ip': 'Client IP',
  country: 'Country',
  '@http.route': 'Route',
};
const VISUALIZE: [string, typeof List][] = [
  ['List', List],
  ['Timeseries', LineChart],
  ['Top List', ListOrdered],
  ['Bar Chart', BarChart3],
  ['Table', Table2],
  ['Pie Chart', ChartPie],
  ['Tree Map', Grid2x2],
  ['Geomap', Globe2],
];

function useRange() {
  const { range } = useTime();
  return useMemo(() => ({ from: Math.floor(range.from / MIN) * MIN, to: Math.ceil(range.to / MIN) * MIN }), [range.from, range.to]);
}

function useQueryParam() {
  const [params, setParams] = useSearchParams();
  const q = params.get('q') ?? '';
  const setQ = (v: string) => setParams((p) => (v ? p.set('q', v) : p.delete('q'), p), { replace: true });
  return [q, setQ, params, setParams] as const;
}

function SearchBar({ value, onChange, placeholder, after }: { value: string; onChange: (v: string) => void; placeholder: string; after?: ReactNode }) {
  const [draft, setDraft] = useState(value);
  return (
    <form
      className="inv-search"
      onSubmit={(e) => {
        e.preventDefault();
        onChange(draft.trim());
      }}
    >
      <Search size={15} />
      <input placeholder={placeholder} value={draft} onChange={(e) => setDraft(e.target.value)} onBlur={() => onChange(draft.trim())} />
      {after}
    </form>
  );
}

const activityClass = (a: string) => (a.startsWith('security_response') ? 'resp' : a.startsWith('business_logic') ? 'biz' : 'attack');
const activityName = (a: string) => a.split('.').slice(1).join('.') || a;
const dateTime = (ms: number) => new Date(ms).toLocaleString(undefined, { month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit', second: '2-digit', hour12: false });
const shortDate = (ms: number) => new Date(ms).toLocaleString(undefined, { month: 'short', day: 'numeric', hour: 'numeric', minute: '2-digit' });
const blockReason = (from: string) => ({ reason: `Blocked from the ${from}`, durationMinutes: null });

/** Whether an attack got through: blocked or answered with a client error means it didn't. */
function qualification(t: SecurityTrace): { label: string; tone: string } | null {
  if (!t.activities.some((a) => a.startsWith('attack_attempt'))) return null;
  const s = Number(t.status);
  if (t.blocked || (s >= 400 && s < 500)) return { label: 'None successful', tone: 'ok' };
  return { label: 'Unknown', tone: 'unknown' };
}

function ActivityChip({ a }: { a: string }) {
  const cls = activityClass(a);
  return (
    <span className={`inv-activity ${cls}`}>
      {cls === 'resp' ? <Hand size={12} /> : cls === 'biz' ? <Tag size={12} /> : <Shield size={12} />}
      {activityName(a)}
    </span>
  );
}

/** Investigate → Traces: the security traces explorer. */
export function SecurityTracesPage() {
  const r = useRange();
  const [query, setQuery] = useQueryParam();
  const [controls, setControls] = useState(true);
  const [graphs, setGraphs] = useState(true);
  const [open, setOpen] = useState<SecurityTrace | null>(null);
  const q = useSecurityTraces(r, query);
  const d = q.data;
  const terms = parseQuery(query);
  const facets: Facet[] = (d?.facets ?? []).map((f) => ({ key: f.key, label: FACET_LABEL[f.key] ?? f.key, group: 'CORE', values: f.values.map((v) => ({ value: v.name, count: v.count })) }));
  const types = [...new Set((d?.histogram ?? []).flatMap((b) => Object.keys(b.counts)))];
  const chart: EChartsOption = {
    grid: { left: 36, right: 8, top: 10, bottom: 44 },
    tooltip: { trigger: 'axis' },
    legend: { bottom: 0, type: 'scroll', icon: 'rect', itemWidth: 9, itemHeight: 9 },
    xAxis: { type: 'time', min: r.from, max: r.to },
    yAxis: { type: 'value', minInterval: 1, splitNumber: 3 },
    series: types.map((t, i) => ({
      name: t,
      type: 'bar',
      stack: 'a',
      barMaxWidth: 8,
      itemStyle: { color: token(PALETTE[i % PALETTE.length]) },
      data: (d?.histogram ?? []).map((b) => [b.ts, b.counts[t] ?? 0]),
    })),
  };
  const maxAct = Math.max(1, ...(d?.topActivities ?? []).map((a) => a.count));
  const maxUser = Math.max(1, ...(d?.topUsers ?? []).map((a) => a.count));
  return (
    <div className="sec-explorer inv-page">
      <AapTabs tabs={INVESTIGATE_TABS} />
      <div className="inv-top">
        <SearchBar
          value={query}
          onChange={setQuery}
          placeholder="Search by any tag on any security trace"
          after={
            <span className="inv-search-code" title="Search syntax: key:value, @attribute:value, -negation, free text">
              <Code2 size={14} />
            </span>
          }
        />
        <span className="inv-in">
          <span>in</span>
          <b>Application Security</b>
        </span>
        <TimePicker compact />
      </div>
      <div className="inv-visualize">
        <span className="inv-seg">
          <span className="inv-seg-label">Visualize as</span>
          {VISUALIZE.map(([label, Icon], i) => (
            <button key={label} className={i === 0 ? 'active' : ''} disabled={i > 0} title={i > 0 ? 'Only the list is available here' : undefined}>
              <Icon size={13} /> {label}
            </button>
          ))}
        </span>
        <span className="syn-grow" />
        <button className="link-btn" onClick={() => setGraphs((g) => !g)}>
          {graphs ? 'Hide Graphs' : 'Show Graphs'}
        </button>
      </div>
      {graphs && (
        <div className="inv-graphs">
          <section>
            <h5>Security Traces by Type</h5>
            <Chart option={chart} height={220} />
          </section>
          <section>
            <h5>Top Users</h5>
            {d?.topUsers.length ? (
              <ul className="inv-toplist">
                {d.topUsers.map((u) => (
                  <li key={u.name}>
                    <b>{compact(u.count)}</b>
                    <span className="inv-toplist-bar">
                      <i style={{ width: `${(u.count / maxUser) * 100}%` }} />
                      <span>{u.name}</span>
                    </span>
                  </li>
                ))}
              </ul>
            ) : (
              <p className="muted inv-none">No authenticated users on these traces.</p>
            )}
          </section>
          <section>
            <h5>Top Security Activities</h5>
            <ul className="inv-toplist">
              {(d?.topActivities ?? []).map((a) => (
                <li key={a.name}>
                  <b>{compact(a.count)}</b>
                  <span className="inv-toplist-bar">
                    <i style={{ width: `${(a.count / maxAct) * 100}%` }} />
                    <span>{a.name}</span>
                  </span>
                </li>
              ))}
            </ul>
          </section>
        </div>
      )}
      <div className={`syn-explorer-body${controls ? '' : ' no-controls'}`}>
        {controls && (
          <FacetPanel
            facets={facets}
            terms={terms}
            onChange={(t: Term[]) => setQuery(formatQuery(t))}
            defaultOpen={['service', '@appsec.type', '@http.status_code']}
            verified={[]}
            noun="facets"
            valueSearch
          />
        )}
        <div className="syn-explorer-main">
          <ExplorerToolbar
            controls={controls}
            onToggle={() => setControls((c) => !c)}
            right={
              <Link className="btn btn-sm" to="/security/appsec/policies/rules">
                <ShieldPlus size={13} /> Add to Detection Rule
              </Link>
            }
          >
            <span>
              <b>{compact(d?.total ?? 0)}</b> traces found
            </span>
          </ExplorerToolbar>
          {q.isLoading ? (
            <Loading />
          ) : q.isError ? (
            <ErrorState error={q.error} onRetry={() => q.refetch()} />
          ) : !d!.traces.length ? (
            <Empty>
              <b>No security traces.</b>
              <p className="muted">Enable App &amp; API Protection in the tracers (DD_APPSEC_ENABLED=true) to see attacks and business logic events.</p>
            </Empty>
          ) : (
            <table className="mon-table syn-table inv-table">
              <thead>
                <tr>
                  <th>DATE</th>
                  <th>STATUS CODE</th>
                  <th>BLOCKED</th>
                  <th>SERVICE</th>
                  <th>ENDPOINT</th>
                  <th>SECURITY ACTIVITY</th>
                  <th>QUALIFICATION</th>
                  <th>CLIENT IP</th>
                  <th>USER</th>
                </tr>
              </thead>
              <tbody>
                {d!.traces.map((t) => {
                  const ql = qualification(t);
                  return (
                    <tr key={`${t.traceId}${t.spanId}`} onClick={() => setOpen(t)} className={open === t ? 'selected' : ''}>
                      <td className="nowrap">{dateTime(t.timestamp)}</td>
                      <td>
                        <span className={`aap-status s${t.status[0]}`}>{t.status}</span>
                      </td>
                      <td>{t.blocked && <Hand size={14} className="aap-blocked" />}</td>
                      <td>
                        <ServiceChip name={t.service} />
                      </td>
                      <td>
                        <MethodPath method={t.method} route={t.route} />
                      </td>
                      <td className="nowrap">
                        {t.activities.slice(0, 2).map((a) => (
                          <ActivityChip key={a} a={a} />
                        ))}
                      </td>
                      <td className="nowrap">{ql && <span className={`inv-qual ${ql.tone}`}>{ql.label}</span>}</td>
                      <td>
                        <IpChip ip={t.clientIp} country={t.country} blocked={t.activities.includes('security_response.block_ip')} />
                      </td>
                      <td>{t.user}</td>
                    </tr>
                  );
                })}
              </tbody>
            </table>
          )}
        </div>
      </div>
      {open && createPortal(<TracePanel t={open} onClose={() => setOpen(null)} />, document.body)}
    </div>
  );
}

const STATUS_TEXT: Record<string, string> = {
  '200': 'OK',
  '201': 'CREATED',
  '204': 'NO CONTENT',
  '301': 'MOVED',
  '302': 'FOUND',
  '400': 'BAD REQUEST',
  '401': 'UNAUTHORIZED',
  '403': 'FORBIDDEN',
  '404': 'NOT FOUND',
  '429': 'TOO MANY REQUESTS',
  '500': 'SERVER ERROR',
  '502': 'BAD GATEWAY',
  '503': 'UNAVAILABLE',
};

const shellQuote = (s: string) => `'${s.replace(/'/g, "'\\''")}'`;
const curl = (t: SecurityTrace) => `curl -X ${t.method} ${shellQuote(t.url)}${t.userAgent ? ` -H ${shellQuote(`User-Agent: ${t.userAgent}`)}` : ''}`;

/** A security trace: the request, what the In-App WAF saw, and the way to the full trace in APM. */
function TracePanel({ t, onClose }: { t: SecurityTrace; onClose: () => void }) {
  const [tab, setTab] = useState<'security' | 'overview'>('security');
  const [more, setMore] = useState(false);
  const apm = `/apm/traces?trace=${t.traceId}&span=${t.spanId}`;
  const attrs: [string, string][] = [
    ['service', t.service],
    ['env', t.env],
    ['http.method', t.method],
    ['http.route', t.route],
    ['http.url', t.url],
    ['http.status_code', t.status],
    ['http.client_ip', t.clientIp],
    ['network.client.geoip.country.iso_code', t.country],
    ['http.useragent', t.userAgent],
    ['usr.id', t.user],
    ['appsec.blocked', String(t.blocked)],
  ];
  return (
    <div className="sec-panel-backdrop" onMouseDown={(e) => e.target === e.currentTarget && onClose()}>
      <aside className="sec-panel inv-drawer" role="dialog" aria-label="Security trace">
        <header className="inv-drawer-head">
          <div className="inv-crumbs">
            <ServiceChip name={t.service} />
            <span className="muted">›</span>
            <b>{t.method}</b>
            <span className="muted">›</span>
            <span className="muted">trace_id</span> <span className="mono">{t.traceId}</span>
          </div>
          <span className="syn-grow" />
          <Link className="btn btn-ghost" to={apm} title="Open in APM">
            <ExternalLink size={15} />
          </Link>
          <button className="btn btn-ghost" onClick={onClose} aria-label="Close">
            <X size={16} />
          </button>
        </header>
        <div className="inv-drawer-sub">
          <MethodBadge method={t.method} />
          <span className="mono inv-ellipsis">{t.url}</span>
          <span className={`aap-status s${t.status[0]}`}>
            {t.status} {STATUS_TEXT[t.status] ?? ''}
          </span>
          <span className="muted nowrap">
            {dateTime(t.timestamp)} ({timeAgo(t.timestamp)})
          </span>
        </div>
        <div className="inv-trace-cta">
          <span>The complete trace (flame graph, waterfall, span list and map) is in APM.</span>
          <Link className="btn btn-sm" to={apm}>
            View Trace in APM <ExternalLink size={12} />
          </Link>
        </div>
        <div className="inv-span-head">
          <ServiceChip name={t.service} />
          <i className="muted">
            {t.method} {t.route}
          </i>
        </div>
        <nav className="inv-tabs">
          <span className="muted">Span:</span>
          <button className={tab === 'overview' ? 'active' : ''} onClick={() => setTab('overview')}>
            Overview
          </button>
          <button className={tab === 'security' ? 'active' : ''} onClick={() => setTab('security')}>
            Security <span className="inv-count">{t.triggers.length || t.activities.length}</span>
          </button>
        </nav>
        <div className="inv-drawer-body">
          {tab === 'overview' ? (
            <table className="mon-table syn-table inv-attrs">
              <tbody>
                {attrs
                  .filter(([, v]) => v)
                  .map(([k, v]) => (
                    <tr key={k}>
                      <td className="mono muted">{k}</td>
                      <td className="mono">{v}</td>
                    </tr>
                  ))}
              </tbody>
            </table>
          ) : (
            <>
              <div className="inv-request">
                <div className="inv-request-head">
                  <b>REQUEST</b> from <IpChip ip={t.clientIp} country={t.country} />
                  {t.userAgent && (
                    <span className="inv-ua">
                      <Globe2 size={12} /> {t.userAgent}
                    </span>
                  )}
                  {t.user && (
                    <span className="inv-ua">
                      <User size={12} /> {t.user}
                    </span>
                  )}
                  {t.blocked && (
                    <span className="inv-activity resp">
                      <Hand size={12} /> Blocked
                    </span>
                  )}
                </div>
                <div className="inv-request-line">
                  <span className="muted">{new Date(t.timestamp).toLocaleString()}</span>
                  <span className="syn-grow" />
                  <button className="link-btn" onClick={() => navigator.clipboard?.writeText(curl(t))}>
                    <Copy size={13} /> Copy Request as cURL
                  </button>
                </div>
                <p className="mono inv-url">
                  {t.method} {t.url}
                </p>
                <button className="link-btn inv-more-btn" onClick={() => setMore((m) => !m)}>
                  {more ? 'See less details' : 'See more details'}
                </button>
                {more && (
                  <dl className="inv-kv">
                    <dt>Route</dt>
                    <dd className="mono">{t.route}</dd>
                    <dt>Environment</dt>
                    <dd>{t.env || '—'}</dd>
                    <dt>Country</dt>
                    <dd>{t.country ? `${flag(t.country)} ${countryName(t.country)}` : '—'}</dd>
                    <dt>Response</dt>
                    <dd>{t.status}</dd>
                  </dl>
                )}
              </div>
              <h4>Security activity</h4>
              <div className="sec-tags">
                {t.activities.map((a) => (
                  <ActivityChip key={a} a={a} />
                ))}
              </div>
              {t.triggers.length > 0 && (
                <>
                  <h4>In-App WAF rules triggered</h4>
                  <table className="mon-table syn-table inv-table">
                    <thead>
                      <tr>
                        <th>RULE</th>
                        <th>TYPE</th>
                        <th>WHAT MATCHED</th>
                      </tr>
                    </thead>
                    <tbody>
                      {t.triggers.map((x) => (
                        <tr key={x.ruleId}>
                          <td>
                            {x.ruleName} <div className="muted sec-sub mono">{x.ruleId}</div>
                          </td>
                          <td>
                            <span className={`inv-activity ${x.category === 'attack_attempt' ? 'attack' : 'biz'}`}>{x.type}</span>
                          </td>
                          <td className="mono csm-detail">{x.matches.join('\n')}</td>
                        </tr>
                      ))}
                    </tbody>
                  </table>
                </>
              )}
            </>
          )}
        </div>
      </aside>
    </div>
  );
}

// ---------------------------------------------------------------- facets computed in the browser

/** The query's facet terms, and its free text (what the server searches). */
function splitQuery(query: string) {
  const terms = parseQuery(query);
  return { terms, text: formatQuery(terms.filter((t) => t.kind === 'text')) };
}

function toggleIn(set: Set<string>, v: string): Set<string> {
  const n = new Set(set);
  if (!n.delete(v)) n.add(v);
  return n;
}

/** Investigate → Users: authenticated users of the security traces. */
export function AppsecUsersPage() {
  const r = useRange();
  const [query, setQuery] = useQueryParam();
  const { terms, text } = splitQuery(query);
  const q = useAppsecUsers(r, text);
  const deny = useDenylist();
  const block = useBlock();
  const unblock = useUnblock();
  const [controls, setControls] = useState(true);
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const blockedUsers = new Set((deny.data ?? []).filter((e) => e.kind === 'user').map((e) => e.value));
  const { facets, shown } = localFacets<AppsecUser>(
    q.data ?? [],
    [
      { key: 'usr.id', label: 'User ID', group: 'USER', get: (u) => [u.user] },
      { key: 'blocked', label: 'Blocked', group: 'STATUS', get: (u) => [String(blockedUsers.has(u.user))] },
      { key: 'service', label: 'Service', group: 'LOCATION', get: (u) => u.services },
      { key: 'country', label: 'Country ISO Code', group: 'LOCATION', get: (u) => u.countries },
      { key: 'risk', label: 'Risks', group: 'RISKS', get: (u) => u.risks },
    ],
    terms,
  );
  const csv = () =>
    downloadCsv(
      'appsec-users.csv',
      ['user', 'countries', 'risks', 'services', 'security_traces', 'last_activity'],
      shown.map((u) => [u.user, u.countries.join(' '), u.risks.join(' '), u.services.join(' '), u.traces, new Date(u.lastSeen).toISOString()]),
    );
  return (
    <div className="sec-explorer inv-page">
      <AapTabs tabs={INVESTIGATE_TABS} />
      <div className={`syn-explorer-body${controls ? '' : ' no-controls'}`}>
        {controls && <FacetPanel facets={facets} terms={terms} onChange={(t: Term[]) => setQuery(formatQuery(t))} defaultOpen={['risk', 'blocked']} verified={[]} noun="facets" />}
        <div className="syn-explorer-main">
          <SearchBar value={query} onChange={setQuery} placeholder="Search users by tag or user ID" />
          <ExplorerToolbar
            controls={controls}
            onToggle={() => setControls((c) => !c)}
            right={
              <button className="btn btn-sm" onClick={csv} disabled={!shown.length}>
                <Download size={13} /> Download as CSV
              </button>
            }
          >
            <span>
              <b>{shown.length}</b> users found
            </span>
            <span className="inv-sep" />
            <TimePicker compact />
            <button
              className="btn btn-sm btn-primary"
              disabled={!selected.size || block.isPending}
              onClick={() => block.mutate({ kind: 'user', values: [...selected], ...blockReason('User Explorer') }, { onSuccess: () => setSelected(new Set()) })}
            >
              <Hand size={13} /> Block Attackers
            </button>
            <Link className="btn btn-sm" to="/security/appsec/signals">
              Explore Signals
            </Link>
          </ExplorerToolbar>
          {q.isLoading ? (
            <Loading />
          ) : q.isError ? (
            <ErrorState error={q.error} />
          ) : (
            <table className="mon-table syn-table inv-table">
              <thead>
                <tr>
                  <th className="sec-check" />
                  <th>USER</th>
                  <th>LOCATION</th>
                  <th>RISKS</th>
                  <th>SERVICES</th>
                  <th className="num">SECURITY TRACES</th>
                  <th className="num">LOGINS (OK / FAILED)</th>
                  <th>LAST ACTIVITY</th>
                  <th className="num">ACTIONS</th>
                </tr>
              </thead>
              <tbody>
                {!shown.length ? (
                  <tr className="inv-empty-row">
                    <td colSpan={9}>
                      <div className="inv-empty">
                        <User size={36} />
                        <h3>No users found</h3>
                        <p className="muted">The User Explorer needs authenticated user information on your traces: the tracers' user tracking (usr.id) and login events.</p>
                        <Link className="btn btn-primary" to="/security/appsec/policies/business-logic">
                          Track login events
                        </Link>
                      </div>
                    </td>
                  </tr>
                ) : (
                  shown.map((u) => {
                    const blocked = blockedUsers.has(u.user);
                    return (
                      <tr key={u.user}>
                        <td className="sec-check">
                          <input type="checkbox" checked={selected.has(u.user)} onChange={() => setSelected((s) => toggleIn(s, u.user))} />
                        </td>
                        <td>
                          <span className="inv-user">
                            <User size={13} /> <b>{u.user}</b> {blocked && <Hand size={12} className="aap-blocked" />}
                          </span>
                        </td>
                        <td className="nowrap">
                          {u.countries.map((c) => (
                            <span key={c} className="inv-chip" title={countryName(c)}>
                              {flag(c)} {c}
                            </span>
                          ))}
                        </td>
                        <td>
                          {u.risks.map((x) => (
                            <span key={x} className="inv-risk">
                              {x.replace(/_/g, ' ')}
                            </span>
                          ))}
                        </td>
                        <td>{u.services[0] && <ServiceChip name={u.services[0]} more={u.services.length - 1} />}</td>
                        <td className="num">
                          <Link to={`/security/appsec/investigate?q=${encodeURIComponent(`@usr.id:${u.user}`)}`}>{compact(u.traces)}</Link>
                        </td>
                        <td className="num">
                          {u.loginSuccess} / {u.loginFailure}
                        </td>
                        <td className="nowrap">{shortDate(u.lastSeen)}</td>
                        <td className="num">
                          {blocked ? (
                            <button className="btn btn-sm" onClick={() => unblock.mutate({ kind: 'user', value: u.user })}>
                              <Hand size={13} /> Unblock
                            </button>
                          ) : (
                            <button className="btn btn-sm" onClick={() => block.mutate({ kind: 'user', values: [u.user], ...blockReason('User Explorer') })}>
                              <Hand size={13} /> Block
                            </button>
                          )}
                        </td>
                      </tr>
                    );
                  })
                )}
              </tbody>
            </table>
          )}
        </div>
      </div>
    </div>
  );
}

type ViewBy = 'ip' | 'ua';

/** Investigate → Attackers: client IPs behind the security traces, blocked or not. */
export function AttackersPage() {
  const r = useRange();
  const [query, setQuery] = useQueryParam();
  const { terms, text } = splitQuery(query);
  const q = useAttackers(r, text);
  const block = useBlock();
  const unblock = useUnblock();
  const [controls, setControls] = useState(true);
  const [viewBy, setViewBy] = useState<ViewBy>('ip');
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [open, setOpen] = useState<string | null>(null);
  const all = q.data?.attackers ?? [];
  const { facets, shown } = localFacets<Attacker>(
    all,
    [
      { key: '@http.client_ip', label: 'IP', group: 'ACTOR', get: (a) => [a.ip] },
      { key: 'blocked', label: 'Blocked', group: 'STATUS', get: (a) => [String(a.denylisted)] },
      { key: 'attackers', label: 'Attackers', group: 'STATUS', get: (a) => (a.suspicious ? ['suspicious'] : []) },
      { key: 'service', label: 'Service', group: 'LOCATION', get: (a) => a.services },
      { key: 'env', label: 'Environment', group: 'LOCATION', get: (a) => a.envs },
      { key: 'country', label: 'Country ISO Code', group: 'LOCATION', get: (a) => (a.country ? [a.country] : []) },
      { key: '@appsec.type', label: 'Category', group: 'THREAT INTELLIGENCE', get: (a) => a.attackTypes },
    ],
    terms,
  );
  const only = (key: string, value: string) => setQuery(formatQuery([...terms.filter((t) => !(t.kind === 'tag' && t.key === key)), { kind: 'tag', key, values: [value], negate: false }]));
  const byUa = useMemo(() => {
    const m = new Map<string, { ua: string; ips: Set<string>; services: Set<string>; traces: number; lastSeen: number }>();
    for (const a of shown)
      for (const ua of a.userAgents) {
        const e = m.get(ua) ?? { ua, ips: new Set(), services: new Set(), traces: 0, lastSeen: 0 };
        e.ips.add(a.ip);
        a.services.forEach((s) => e.services.add(s));
        e.traces += a.traces;
        e.lastSeen = Math.max(e.lastSeen, a.lastSeen);
        m.set(ua, e);
      }
    return [...m.values()].sort((x, y) => y.traces - x.traces);
  }, [shown]);
  const current = all.find((a) => a.ip === open);
  return (
    <div className="sec-explorer inv-page">
      <AapTabs tabs={INVESTIGATE_TABS} />
      <div className="inv-cards two">
        <button className="inv-card" onClick={() => only('attackers', 'suspicious')} title="Show the suspicious attackers">
          <h5>
            <Filter size={13} /> Suspicious Attackers
          </h5>
          <p>
            <b>{q.data?.suspicious ?? 0}</b> IPs
          </p>
        </button>
        <button className="inv-card" onClick={() => only('blocked', 'false')} title="Show the attackers that aren't blocked">
          <h5>
            <Filter size={13} /> Flagged Attackers
          </h5>
          <p>
            <b>{q.data?.flagged ?? 0}</b> IPs
          </p>
        </button>
      </div>
      <div className={`syn-explorer-body${controls ? '' : ' no-controls'}`}>
        {controls && <FacetPanel facets={facets} terms={terms} onChange={(t: Term[]) => setQuery(formatQuery(t))} defaultOpen={['attackers', 'blocked']} verified={[]} noun="facets" />}
        <div className="syn-explorer-main">
          <SearchBar value={query} onChange={setQuery} placeholder="Search by any tag on any attacker" />
          <div className="inv-visualize">
            <span className="inv-seg">
              <span className="inv-seg-label">View by</span>
              <button className={viewBy === 'ip' ? 'active' : ''} onClick={() => setViewBy('ip')}>
                IP
              </button>
              <button className={viewBy === 'ua' ? 'active' : ''} onClick={() => setViewBy('ua')}>
                User Agent
              </button>
              <button disabled title="Needs the IP's network (ASN), which the traces don't carry">
                ASN
              </button>
              <button disabled title="Needs attacker clustering">
                Cluster
              </button>
            </span>
          </div>
          <ExplorerToolbar controls={controls} onToggle={() => setControls((c) => !c)}>
            <span>
              <b>{viewBy === 'ip' ? shown.length : byUa.length}</b> {viewBy === 'ip' ? 'IPs' : 'user agents'} found
            </span>
            <span className="inv-sep" />
            <TimePicker compact />
            <button
              className="btn btn-sm btn-primary"
              disabled={!selected.size || block.isPending}
              onClick={() => block.mutate({ kind: 'ip', values: [...selected], ...blockReason('Attackers Explorer') }, { onSuccess: () => setSelected(new Set()) })}
            >
              <Hand size={13} /> Block Attackers
            </button>
            <Link className="btn btn-sm" to="/security/appsec/signals">
              Explore Signals
            </Link>
          </ExplorerToolbar>
          {q.isLoading ? (
            <Loading />
          ) : q.isError ? (
            <ErrorState error={q.error} />
          ) : viewBy === 'ua' ? (
            <table className="mon-table syn-table inv-table">
              <thead>
                <tr>
                  <th>USER AGENT</th>
                  <th className="num">IPS</th>
                  <th>SERVICES</th>
                  <th>LAST ACTIVITY</th>
                  <th className="num">SECURITY TRACES</th>
                </tr>
              </thead>
              <tbody>
                {byUa.map((u) => (
                  <tr key={u.ua}>
                    <td className="csm-detail">{u.ua}</td>
                    <td className="num">{u.ips.size}</td>
                    <td>{u.services.size > 0 && <ServiceChip name={[...u.services][0]} more={u.services.size - 1} />}</td>
                    <td className="nowrap">{shortDate(u.lastSeen)}</td>
                    <td className="num">{compact(u.traces)}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          ) : (
            <table className="mon-table syn-table inv-table">
              <thead>
                <tr>
                  <th className="sec-check">
                    <input type="checkbox" checked={shown.length > 0 && selected.size === shown.length} onChange={(e) => setSelected(e.target.checked ? new Set(shown.map((a) => a.ip)) : new Set())} />
                  </th>
                  <th>IP</th>
                  <th>ATTACK TYPES</th>
                  <th>SERVICES</th>
                  <th>USER AGENTS</th>
                  <th>LAST ACTIVITY</th>
                  <th className="num">SECURITY TRACES</th>
                  <th className="num">ACTIONS</th>
                </tr>
              </thead>
              <tbody>
                {shown.map((a) => (
                  <tr key={a.ip} onClick={() => setOpen(a.ip)} className={open === a.ip ? 'selected' : ''}>
                    <td className="sec-check" onClick={(e) => e.stopPropagation()}>
                      <input type="checkbox" checked={selected.has(a.ip)} onChange={() => setSelected((s) => toggleIn(s, a.ip))} />
                    </td>
                    <td className="nowrap">
                      <IpChip ip={a.ip} country={a.country} blocked={a.denylisted} />
                      {a.suspicious && !a.denylisted && <span className="inv-risk">suspicious</span>}
                    </td>
                    <td className="nowrap">
                      {a.attackTypes.slice(0, 2).map((t) => (
                        <span key={t} className="inv-activity attack">
                          <Shield size={12} />
                          {t}
                        </span>
                      ))}
                      {a.attackTypes.length > 2 && <span className="inv-more">+{a.attackTypes.length - 2}</span>}
                    </td>
                    <td>{a.services[0] && <ServiceChip name={a.services[0]} env={a.envs[0]} more={a.services.length - 1} />}</td>
                    <td className="inv-ua-cell">
                      <span className="csm-detail">{a.userAgents[0]}</span>
                      {a.userAgents.length > 1 && <span className="inv-more">+{a.userAgents.length - 1}</span>}
                    </td>
                    <td className="nowrap">{shortDate(a.lastSeen)}</td>
                    <td className="num" onClick={(e) => e.stopPropagation()}>
                      <Link to={`/security/appsec/investigate?q=${encodeURIComponent(`@http.client_ip:${a.ip}`)}`}>{compact(a.traces)}</Link>
                    </td>
                    <td className="num" onClick={(e) => e.stopPropagation()}>
                      {a.denylisted ? (
                        <button className="btn btn-sm" onClick={() => unblock.mutate({ kind: 'ip', value: a.ip })}>
                          <Hand size={13} /> Unblock
                        </button>
                      ) : (
                        <button className="btn btn-sm" onClick={() => block.mutate({ kind: 'ip', values: [a.ip], ...blockReason('Attackers Explorer') })}>
                          <Hand size={13} /> Block
                        </button>
                      )}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          )}
        </div>
      </div>
      {current && createPortal(<AttackerPanel a={current} onClose={() => setOpen(null)} />, document.body)}
    </div>
  );
}

type AttackerTab = 'intel' | 'history' | 'users' | 'endpoints' | 'agents';

/** One attacker: what it did, whether it's blocked, and the way to its traces. */
function AttackerPanel({ a, onClose }: { a: Attacker; onClose: () => void }) {
  const r = useRange();
  const ipQuery = `@http.client_ip:${a.ip}`;
  const traces = useSecurityTraces(r, ipQuery);
  const users = useAppsecUsers(r, '');
  const deny = useDenylist();
  const block = useBlock();
  const unblock = useUnblock();
  const pass = useSaveWaf('exclusion_filter');
  const [tab, setTab] = useState<AttackerTab>('intel');
  const entry = (deny.data ?? []).find((e) => e.kind === 'ip' && e.value === a.ip);
  const associated = (users.data ?? []).filter((u) => u.ips.includes(a.ip));
  const td = traces.data;
  const logins = (td?.topActivities ?? []).filter((x) => x.name.startsWith('business_logic.users.login')).reduce((s, x) => s + x.count, 0);
  const endpoints = useMemo(() => {
    const m = new Map<string, { method: string; route: string; service: string; count: number }>();
    for (const t of td?.traces ?? []) {
      const k = `${t.service} ${t.method} ${t.route}`;
      const e = m.get(k) ?? { method: t.method, route: t.route, service: t.service, count: 0 };
      e.count++;
      m.set(k, e);
    }
    return [...m.values()].sort((x, y) => y.count - x.count);
  }, [td]);
  const types = [...new Set((td?.histogram ?? []).flatMap((b) => Object.keys(b.counts)))];
  const chart: EChartsOption = {
    grid: { left: 36, right: 8, top: 8, bottom: 24 },
    tooltip: { trigger: 'axis' },
    xAxis: { type: 'time', min: r.from, max: r.to },
    yAxis: { type: 'value', minInterval: 1, splitNumber: 2 },
    series: types.map((t, i) => ({
      name: t,
      type: 'bar',
      stack: 'a',
      barMaxWidth: 8,
      itemStyle: { color: token(PALETTE[i % PALETTE.length]) },
      data: (td?.histogram ?? []).map((b) => [b.ts, b.counts[t] ?? 0]),
    })),
  };
  const tabs: [AttackerTab, string, number | null][] = [
    ['intel', 'Threat Intel', null],
    ['history', 'History', null],
    ['users', 'Associated Users', associated.length],
    ['endpoints', 'Endpoint Requests', endpoints.length],
    ['agents', 'Top User Agents', a.userAgents.length],
  ];
  const blockAll = () => block.mutate({ kind: 'ip', values: [a.ip], ...blockReason('Attackers Explorer') });
  return (
    <div className="sec-panel-backdrop" onMouseDown={(e) => e.target === e.currentTarget && onClose()}>
      <aside className="sec-panel inv-drawer" role="dialog" aria-label={a.ip}>
        <header className="inv-drawer-head">
          <h2 className="inv-ip-title">
            <span className="inv-flag">{flag(a.country)}</span> <span className="mono">{a.ip}</span>
            <button className="btn btn-ghost btn-sm" title="Copy" onClick={() => navigator.clipboard?.writeText(a.ip)}>
              <Copy size={13} />
            </button>
          </h2>
          <span className="syn-grow" />
          <button
            className="btn btn-sm"
            disabled={pass.isPending || pass.isSuccess}
            onClick={() => pass.mutate({ description: `Passlisted from the Attackers Explorer: ${a.ip}`, enabled: true, ip_list: [a.ip] })}
          >
            {pass.isSuccess ? 'Added to Passlist' : 'Add to Passlist'}
          </button>
          {a.denylisted ? (
            <button className="btn btn-sm" onClick={() => unblock.mutate({ kind: 'ip', value: a.ip })}>
              <Hand size={13} /> Unblock
            </button>
          ) : (
            <button className="btn btn-sm btn-primary" onClick={blockAll}>
              <Hand size={13} /> Block
            </button>
          )}
          <button className="btn btn-ghost" onClick={onClose} aria-label="Close">
            <X size={16} />
          </button>
        </header>
        <div className="inv-drawer-sub">
          <span className="inv-query mono">{ipQuery}</span>
          <Link to={`/security/appsec/investigate?q=${encodeURIComponent(`${ipQuery} @appsec.category:business_logic`)}`}>
            {logins} logins <ExternalLink size={11} />
          </Link>
          <Link to={`/security/appsec/investigate?q=${encodeURIComponent(ipQuery)}`}>
            Security traces <ExternalLink size={11} />
          </Link>
          <span className="syn-grow" />
          <TimePicker compact />
        </div>
        <div className="inv-drawer-body">
          <section className="inv-summary">
            <h4>SUMMARY</h4>
            <div className="inv-observed">
              <span>
                <span className="muted">First observed:</span> {new Date(a.firstSeen).toLocaleString()}
              </span>
              <span>
                <span className="muted">Last observed:</span> {new Date(a.lastSeen).toLocaleString()} ({timeAgo(a.lastSeen)})
              </span>
            </div>
            <dl className="inv-summary-rows">
              <dt>BLOCKING STATUS</dt>
              <dd>
                <div className="inv-row">
                  {a.denylisted ? (
                    <span className="inv-blocked-pill">
                      <Hand size={12} /> Blocked
                    </span>
                  ) : (
                    <span className="inv-risk">Not blocked</span>
                  )}
                  {entry && (
                    <span className="muted">
                      since {new Date(entry.createdAt).toLocaleString()}
                      {entry.expiresAt ? `, until ${new Date(entry.expiresAt).toLocaleString()}` : ''}
                    </span>
                  )}
                  <span className="syn-grow" />
                  <Link to="/security/appsec/policies/denylist">
                    Open Denylist <ExternalLink size={11} />
                  </Link>
                </div>
                {!a.denylisted && (
                  <div className="inv-warn">
                    <span>This IP reaches {a.services.length === 1 ? 'a service' : `${a.services.length} services`} without being blocked.</span>
                    <span className="syn-grow" />
                    <button className="btn btn-sm" onClick={blockAll}>
                      <Hand size={13} /> Block on all services
                    </button>
                  </div>
                )}
              </dd>
              <dt>THREAT INTELLIGENCE</dt>
              <dd>{a.suspicious ? <span className="inv-risk">Suspicious: attacks from this IP reached several endpoints</span> : 'No threat intelligence detected.'}</dd>
              <dt>LAST INFORMATION</dt>
              <dd className="inv-row">
                <span className="inv-chip">Location: {a.country ? `${flag(a.country)} ${countryName(a.country)}` : 'unknown'}</span>
                {a.envs.map((e) => (
                  <span key={e} className="inv-env">
                    env:{e}
                  </span>
                ))}
              </dd>
              <dt>ASSOCIATED USERS</dt>
              <dd className="inv-row">
                <span className="inv-chip">{associated.length} associated users</span>
              </dd>
              <dt>SECURITY TRACES</dt>
              <dd>
                <p>
                  We detected <Link to={`/security/appsec/investigate?q=${encodeURIComponent(ipQuery)}`}>{compact(td?.total ?? a.traces)}</Link> <b>security traces</b> during the selected period
                  {a.blockedTraces ? `, ${compact(a.blockedTraces)} of them blocked` : ''}.
                </p>
                <Chart option={chart} height={110} />
              </dd>
            </dl>
          </section>
          <nav className="inv-tabs">
            {tabs.map(([k, label, n]) => (
              <button key={k} className={tab === k ? 'active' : ''} onClick={() => setTab(k)}>
                {label} {n !== null && <span className="inv-count">{n}</span>}
              </button>
            ))}
          </nav>
          {tab === 'intel' &&
            (a.attackTypes.length ? (
              <table className="mon-table syn-table inv-table">
                <thead>
                  <tr>
                    <th>THREAT</th>
                    <th>SOURCE</th>
                    <th>SECURITY ACTIVITY</th>
                  </tr>
                </thead>
                <tbody>
                  {a.attackTypes.map((t) => (
                    <tr key={t}>
                      <td>{t.replace(/_/g, ' ')}</td>
                      <td>In-App WAF</td>
                      <td>
                        <span className="inv-activity attack">
                          <Shield size={12} />
                          {t}
                        </span>
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            ) : (
              <div className="inv-empty small">
                <Search size={30} />
                <p>No threat intel found</p>
              </div>
            ))}
          {tab === 'history' && (
            <table className="mon-table syn-table inv-table">
              <thead>
                <tr>
                  <th>DATE</th>
                  <th>STATUS</th>
                  <th>ENDPOINT</th>
                  <th>SECURITY ACTIVITY</th>
                </tr>
              </thead>
              <tbody>
                {(td?.traces ?? []).slice(0, 50).map((t) => (
                  <tr key={`${t.traceId}${t.spanId}`}>
                    <td className="nowrap">{dateTime(t.timestamp)}</td>
                    <td>
                      <span className={`aap-status s${t.status[0]}`}>{t.status}</span>
                    </td>
                    <td>
                      <MethodPath method={t.method} route={t.route} />
                    </td>
                    <td className="nowrap">
                      {t.activities.slice(0, 2).map((x) => (
                        <ActivityChip key={x} a={x} />
                      ))}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          )}
          {tab === 'users' &&
            (associated.length ? (
              <table className="mon-table syn-table inv-table">
                <thead>
                  <tr>
                    <th>USER</th>
                    <th className="num">LOGINS (OK / FAILED)</th>
                    <th>LAST ACTIVITY</th>
                  </tr>
                </thead>
                <tbody>
                  {associated.map((u) => (
                    <tr key={u.user}>
                      <td>{u.user}</td>
                      <td className="num">
                        {u.loginSuccess} / {u.loginFailure}
                      </td>
                      <td>{timeAgo(u.lastSeen)}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            ) : (
              <div className="inv-empty small">
                <User size={30} />
                <p>No user logged in from this IP</p>
              </div>
            ))}
          {tab === 'endpoints' && (
            <table className="mon-table syn-table inv-table">
              <thead>
                <tr>
                  <th>ENDPOINT</th>
                  <th>SERVICE</th>
                  <th className="num">REQUESTS</th>
                </tr>
              </thead>
              <tbody>
                {endpoints.map((e) => (
                  <tr key={`${e.service}${e.method}${e.route}`}>
                    <td>
                      <MethodPath method={e.method} route={e.route} />
                    </td>
                    <td>
                      <ServiceChip name={e.service} />
                    </td>
                    <td className="num">{e.count}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          )}
          {tab === 'agents' && (
            <ul className="inv-agents">
              {a.userAgents.map((u) => (
                <li key={u} className="mono">
                  {u}
                </li>
              ))}
            </ul>
          )}
        </div>
      </aside>
    </div>
  );
}
