import { useMemo, useState } from 'react';
import { createPortal } from 'react-dom';
import { Link, useNavigate, useSearchParams } from 'react-router-dom';
import type { EChartsOption } from 'echarts';
import { ArrowRight, ChevronDown, ChevronRight, Filter, Globe, Hand, LayoutList, List, Plus, Search, ThumbsDown, ThumbsUp, Users, Workflow, X } from 'lucide-react';
import type { SecuritySignal, SignalEntity } from '../../../api/security/client';
import { Chart } from '../../../components/charts/Chart';
import { TimePicker } from '../../../components/shell/PageHeader';
import { TimeProvider, useTime } from '../../../components/shell/TimeContext';
import { Menu } from '../../../components/ui/Dropdown';
import { Empty, ErrorState, Loading } from '../../../components/ui/AsyncState';
import { timeAgo } from '../../../lib/format';
import { ARCHIVE_REASONS, SeverityBadge, STATE_LABEL } from '../common';
import { useSignals, useSignalState } from '../queries';
import { CategoryChip } from './AiGuardLayout';
import { ServiceChip } from './InvestigatePage';
import { useAiGuardCategories } from './queries';

const MIN = 60_000;
const SCOPE = 'ai_guard:true';
const SEV_COLOR: Record<string, string> = { critical: '#a3161a', high: '#d9363e', medium: '#e8833d', low: '#e8b53d', info: '#8592a3' };
const entity = (s: SecuritySignal, k: string) => (s.entities as unknown as Record<string, SignalEntity | undefined>)[k];
const tagValue = (s: SecuritySignal, k: string) => s.tags.find((t) => t.startsWith(`${k}:`))?.slice(k.length + 1) ?? '';

/** AI Guard → Signals: the Signals Explorer of AI Guard's detection rules. */
export function AiGuardSignalsPage() {
  return (
    <TimeProvider initial="1w">
      <Signals />
    </TimeProvider>
  );
}

function Signals() {
  const [params, setParams] = useSearchParams();
  const { range } = useTime();
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
  const query = params.get('q') ?? '';
  const view = params.get('view') ?? 'list';
  const [draft, setDraft] = useState(query);
  const r = useMemo(() => ({ from: Math.floor(range.from / MIN) * MIN, to: Math.ceil(range.to / MIN) * MIN }), [range.from, range.to]);
  const signals = useSignals({ range: r, query: [SCOPE, query].filter(Boolean).join(' '), state: '', limit: 500 });
  const all = signals.data?.signals ?? [];
  const open = params.get('signal');
  const filters = query.split(/\s+/).filter(Boolean).length;
  const chart = useMemo<EChartsOption>(() => {
    const h = signals.data?.histogram ?? [];
    const sevs = ['critical', 'high', 'medium', 'low', 'info'] as const;
    return {
      grid: { left: 30, right: 8, top: 6, bottom: 20 },
      tooltip: { trigger: 'axis' },
      xAxis: { type: 'time' },
      yAxis: { type: 'value', splitNumber: 2, minInterval: 1 },
      series: sevs.map((sev) => ({ name: sev, type: 'bar' as const, stack: 's', barMaxWidth: 10, data: h.map((b) => [b.ts, b[sev]]), itemStyle: { color: SEV_COLOR[sev] } })),
    };
  }, [signals.data]);
  const byRule = useMemo(() => {
    const m = new Map<string, SecuritySignal[]>();
    for (const s of all) m.set(s.ruleName, [...(m.get(s.ruleName) ?? []), s]);
    return [...m.entries()].sort((a, b) => b[1].length - a[1].length);
  }, [all]);
  return (
    <div className="sec-page aig-page">
      <div className="aig-views">
        <span className="aig-views-link">
          <LayoutList size={14} /> Views
        </span>
        <b>Signals Explorer</b>
        <button className="btn btn-sm" disabled title="Saved views aren't available yet">
          <Plus size={13} /> Save
        </button>
        <span className="aig-grow" />
        <TimePicker compact />
      </div>
      <form
        className="aig-signal-search"
        onSubmit={(e) => {
          e.preventDefault();
          set('q', draft.trim() || null);
        }}
      >
        <Search size={14} />
        <span className="aig-filter-count">
          <Filter size={13} /> {filters}
        </span>
        <input
          placeholder="Filter your signals: status:high service:shopping-assistant @workflow.triage.state:open…"
          value={draft}
          onChange={(e) => setDraft(e.target.value)}
          onBlur={() => set('q', draft.trim() || null)}
        />
      </form>
      <div className="aig-visualize">
        <span className="aig-seg-label">Visualize as</span>
        <button className={view === 'list' ? 'active' : ''} onClick={() => set('view', null)}>
          <List size={13} /> List
        </button>
        <button className={view === 'rules' ? 'active' : ''} onClick={() => set('view', 'rules')}>
          <LayoutList size={13} /> Rules List
        </button>
      </div>
      <div className="aig-signal-chart">{signals.isLoading ? <Loading /> : <Chart option={chart} height={90} />}</div>
      {signals.isLoading ? (
        <Loading />
      ) : signals.isError ? (
        <ErrorState error={signals.error} />
      ) : !all.length ? (
        <Empty>
          <b>No signals.</b>
          <p className="muted">AI Guard's detection rules raise signals when unsafe steps aren't blocked, data leaves an AI service, or credentials appear in answers.</p>
        </Empty>
      ) : view === 'rules' ? (
        <table className="mon-table syn-table aig-table">
          <thead>
            <tr>
              <th>RULE</th>
              <th className="num">SIGNALS</th>
              <th>HIGHEST SEVERITY</th>
              <th>LAST SIGNAL</th>
            </tr>
          </thead>
          <tbody>
            {byRule.map(([name, list]) => {
              const sev = ['critical', 'high', 'medium', 'low', 'info'].find((x) => list.some((s) => s.severity === x)) ?? 'info';
              return (
                <tr key={name} className="aig-row" onClick={() => set('signal', list[0].id)}>
                  <td>
                    <b>{name}</b>
                  </td>
                  <td className="num">{list.length}</td>
                  <td>
                    <SeverityBadge severity={sev} />
                  </td>
                  <td className="muted">{timeAgo(list[0].createdAt)}</td>
                </tr>
              );
            })}
          </tbody>
        </table>
      ) : (
        <div className="aig-signal-list">
          {all.map((s) => (
            <button key={s.id} className={`aig-signal-row${open === s.id ? ' active' : ''}`} onClick={() => set('signal', s.id)}>
              <SeverityBadge severity={s.severity} />
              <span className="aig-signal-main">
                <b>{s.title}</b>
                <span className="aig-signal-tags">
                  {tagValue(s, 'category') && <span className="aig-chip">category:{tagValue(s, 'category')}</span>}
                  {s.state !== 'open' && <span className="aig-chip">{STATE_LABEL[s.state] ?? s.state}</span>}
                </span>
              </span>
              <span className="muted nowrap">{timeAgo(s.createdAt)}</span>
            </button>
          ))}
        </div>
      )}
      {open && <SignalPanel signal={all.find((s) => s.id === open)} onClose={() => set('signal', null)} />}
    </div>
  );
}

/** The signal's summary, from our own templates over its rule, group and entities. */
function summaryOf(s: SecuritySignal): string {
  const service = s.groupValues.service ?? s.services[0] ?? 'an AI service';
  const n = s.eventsMatched;
  const steps = `${n} step${n === 1 ? '' : 's'}`;
  const cats = entity(s, 'attack_category')?.top ?? [];
  const tools = entity(s, 'tool')?.top ?? [];
  const cat = cats.length ? ` flagged as ${cats.slice(0, 3).join(', ')}` : '';
  const tool = tools.length ? ` through ${tools.slice(0, 2).join(' and ')}` : '';
  switch (s.ruleId) {
    case 'def-aig-001':
      return `${service} received content from a tool${tool} that carried hidden instructions to the model (${steps}${cat}). AI Guard flagged the steps, but the service runs in monitoring mode, so the agent kept going.`;
    case 'def-aig-002':
      return `An agent run of ${service} tried to send data out of the application${tool} (${steps}${cat}). The step wasn't blocked: check the destination and what was sent.`;
    case 'def-aig-003':
      return `${service} returned personal data or credentials in ${steps}. The sensitive data scanners found them in model answers or outgoing tool calls.`;
    default:
      return `${service} had ${steps} AI Guard rated unsafe${cat} without blocking them.`;
  }
}

function SignalPanel({ signal: s, onClose }: { signal?: SecuritySignal; onClose: () => void }) {
  const navigate = useNavigate();
  const setState = useSignalState();
  const cats = useAiGuardCategories();
  const [what, setWhat] = useState(true);
  const [moreTags, setMoreTags] = useState(false);
  const [moreActions, setMoreActions] = useState(false);
  const [helpful, setHelpful] = useState<'' | 'up' | 'down'>('');
  if (!s) return null;
  const service = s.groupValues.service ?? s.services[0] ?? '';
  const ips = entity(s, 'ip');
  const users = entity(s, 'user');
  const topCats = entity(s, 'attack_category')?.top ?? [];
  const category = tagValue(s, 'category') || 'custom';
  const lead = topCats[0];
  const describe = (c: string) => cats.data?.find((x) => x.category === c)?.description ?? '';
  const tags = s.tags.filter((t) => t !== SCOPE);
  const evals = `/security/ai-guard?q=${encodeURIComponent(`service:${service}`)}&assessment=unsafe`;
  return createPortal(
    <div className="sec-panel-backdrop" onClick={onClose}>
      <aside className="sec-panel aig-panel aig-signal-panel" onClick={(e) => e.stopPropagation()}>
        <div className="aig-panel-head">
          <span className="aig-chip">
            Threat | {new Date(s.createdAt).toLocaleString()} | {timeAgo(s.createdAt)}
          </span>
          <span className="aig-grow" />
          <Link className="aig-head-link" to={`/security/siem/rules/${encodeURIComponent(s.ruleId)}`}>
            Explore Rule
          </Link>
          <button className="btn btn-ghost btn-icon" onClick={onClose} aria-label="Close">
            <X size={16} />
          </button>
        </div>
        <h3 className="aig-signal-title">
          <SeverityBadge severity={s.severity} /> {s.title}
        </h3>
        <div className="aig-tags">
          {(moreTags ? tags : tags.slice(0, 4)).map((t) => (
            <span key={t} className="aig-chip">
              {t}
            </span>
          ))}
          {tags.length > 4 && (
            <button className="aig-chip link" onClick={() => setMoreTags(!moreTags)}>
              {moreTags ? 'less' : `+${tags.length - 4}`}
            </button>
          )}
        </div>
        <div className="aig-signal-body">
          <div className="aig-signal-left">
            <section className="aig-box">
              <h4>Summary</h4>
              <p>{summaryOf(s)}</p>
              {lead && (
                <>
                  <h5 className="aig-cat-head" title={describe(lead)}>
                    {lead.replace(/-/g, ' ')} <span className="aig-help">?</span>
                  </h5>
                  <p>{describe(lead)}</p>
                </>
              )}
              <div className="aig-helpful">
                Was this helpful?
                <button className={`btn btn-ghost btn-icon${helpful === 'up' ? ' active' : ''}`} onClick={() => setHelpful('up')} aria-label="Helpful">
                  <ThumbsUp size={13} />
                </button>
                <button className={`btn btn-ghost btn-icon${helpful === 'down' ? ' active' : ''}`} onClick={() => setHelpful('down')} aria-label="Not helpful">
                  <ThumbsDown size={13} />
                </button>
              </div>
            </section>
            <section className="aig-box">
              <button className="aig-collapse" onClick={() => setWhat(!what)}>
                {what ? <ChevronDown size={15} /> : <ChevronRight size={15} />} What happened
              </button>
              {what && (
                <>
                  <span className="aig-chip">Category: {category[0].toUpperCase() + category.slice(1)}</span>
                  <p>{s.message}</p>
                  {topCats.length > 0 && (
                    <p className="aig-cats">
                      {topCats.map((c) => (
                        <CategoryChip key={c} category={c} />
                      ))}
                    </p>
                  )}
                  <Link to={evals}>More info: the unsafe evaluations →</Link>
                  <dl className="aig-what">
                    <dt>SERVICE</dt>
                    <dd>
                      {service ? <ServiceChip service={service} /> : '—'} {s.env && <span className="aig-chip">env:{s.env}</span>}
                    </dd>
                    <dt className="aig-red">ATTACKERS</dt>
                    <dd>
                      {ips?.count ? (
                        <>
                          <span className="aig-chip">
                            {ips.count} IP{ips.count === 1 ? '' : 's'}
                          </span>{' '}
                          targeted this service{' '}
                          {ips.top.slice(0, 3).map((ip) => (
                            <span key={ip} className="aig-chip mono">
                              <Globe size={11} /> {ip}
                            </span>
                          ))}
                        </>
                      ) : (
                        <span className="muted">No client IP on the evaluations.</span>
                      )}
                    </dd>
                    {users?.count ? (
                      <>
                        <dt>USERS</dt>
                        <dd>
                          {users.top.slice(0, 3).map((u) => (
                            <span key={u} className="aig-chip">
                              <Users size={11} /> {u}
                            </span>
                          ))}
                        </dd>
                      </>
                    ) : null}
                  </dl>
                </>
              )}
            </section>
          </div>
          <aside className="aig-next">
            <span className="aig-next-arrow">
              <ArrowRight size={18} />
            </span>
            <h4>NEXT STEPS</h4>
            <div className="aig-next-group">
              <span>Triage</span>
              <Menu
                triggerClassName="btn aig-next-state"
                trigger={
                  <>
                    {(STATE_LABEL[s.state] ?? s.state).toUpperCase()} <ChevronDown size={13} />
                  </>
                }
                items={[
                  { label: 'Open', onClick: () => setState.mutate({ id: s.id, input: { state: 'open' } }) },
                  { label: 'Under review', onClick: () => setState.mutate({ id: s.id, input: { state: 'under_review' } }) },
                  ...ARCHIVE_REASONS.map(([reason, label]) => ({ label: `Archive: ${label}`, onClick: () => setState.mutate({ id: s.id, input: { state: 'archived', archiveReason: reason } }) })),
                ]}
              />
              <span className="muted">Reason: {s.state === 'archived' ? s.archiveReason.replace(/_/g, ' ') : '-'}</span>
            </div>
            <div className="aig-next-group">
              <span>Respond</span>
              <button className="btn" onClick={() => navigate('/incidents')}>
                Create Case <ChevronDown size={13} />
              </button>
              <button className="btn" onClick={() => navigate('/workflows')}>
                <Workflow size={13} /> Run Workflow
              </button>
              <button className="btn" disabled title="AI Guard blocks through the SDK; IPs can be blocked from App & API Protection's denylist">
                <Hand size={13} /> Block Attacking IPs
              </button>
              <button className="btn" disabled title="AI Guard blocks through the SDK; users can be blocked from App & API Protection's denylist">
                <Hand size={13} /> Block Attacking Users
              </button>
            </div>
            <button className="aig-collapse" onClick={() => setMoreActions(!moreActions)}>
              {moreActions ? <ChevronDown size={14} /> : <ChevronRight size={14} />} More actions
            </button>
            {moreActions && (
              <div className="aig-next-group">
                <Link className="btn" to={evals}>
                  View unsafe evaluations
                </Link>
                <Link className="btn" to="/security/ai-guard/settings">
                  Turn on blocking
                </Link>
              </div>
            )}
          </aside>
        </div>
      </aside>
    </div>,
    document.body,
  );
}
