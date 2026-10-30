import { useMemo, useState } from 'react';
import { useMutation, useQuery } from '@tanstack/react-query';
import { Link, useNavigate, useParams } from 'react-router-dom';
import {
  ArrowRight,
  BookOpen,
  Check,
  List,
  Search,
  XCircle,
  ChevronDown,
  ChevronRight,
  Copy,
  ExternalLink,
  FileText,
  Gauge,
  LayoutDashboard,
  MoreVertical,
  Network,
  Pencil,
  Server,
  Siren,
  Upload,
  Volume2,
  VolumeX,
  X,
} from 'lucide-react';
import type { MonitorDetail, MonitorEvent, MonitorStatus, StatusSegment } from '../../api/monitors/client';
import { TimePicker } from '../../components/shell/PageHeader';
import { TimeProvider, useTime } from '../../components/shell/TimeContext';
import { ErrorState, Loading } from '../../components/ui/AsyncState';
import { Menu, Select } from '../../components/ui/Dropdown';
import { workItems } from '../../api/events/client';
import { createNotebook, searchNotebooks } from '../../api/notebooks/client';
import { newCellId } from '../../api/notebooks/templates';
import { Markdown } from '../dashboards/widgets/markdown';
import { UserAvatar } from '../../components/ui/UserAvatar';
import { DeclareModal } from '../incidents/DeclareModal';
import { MUTE_OPTIONS, RecipientIcon, STATUS_NAME, StatusBadge, fmtDateTime, fmtDuration, fmtValue, recipientLabel } from './common';
import { messageRecipients, renderMessage } from './message';
import { useBulk, useCloneMonitor, useMonitor, useMonitorHistory } from './queries';
import './monitors.css';

const BAR_COLOR: Record<MonitorStatus, string> = {
  alert: 'var(--mon-alert)',
  warn: 'var(--mon-warn)',
  nodata: 'var(--mon-nodata)',
  ok: 'var(--mon-ok)',
};

export function MonitorStatusPage() {
  return (
    <TimeProvider initial="4h">
      <StatusInner />
    </TimeProvider>
  );
}

function StatusInner() {
  const id = Number(useParams().id);
  const navigate = useNavigate();
  const q = useMonitor(id);
  const bulk = useBulk();
  const clone = useCloneMonitor();
  const [declaring, setDeclaring] = useState(false);
  const [details, setDetails] = useState(false);
  const [filters, setFilters] = useState<GroupFilter>(NO_FILTER);
  if (q.isPending) return <Loading label="Loading monitor…" />;
  if (q.isError) return <ErrorState error={q.error} onRetry={() => q.refetch()} />;
  const m = q.data;
  const exportJson = () => {
    const input = {
      name: m.name,
      type: m.type,
      query: m.metricQuery,
      options: m.options,
      message: m.message,
      priority: m.priority ?? null,
      tags: m.tags,
      teams: m.teams,
      draft: m.draft,
    };
    const a = document.createElement('a');
    a.href = URL.createObjectURL(new Blob([JSON.stringify(input, null, 2)], { type: 'application/json' }));
    a.download = `monitor-${m.id}.json`;
    a.click();
    URL.revokeObjectURL(a.href);
  };
  const mute = (minutes: number | null) => bulk.mutate({ ids: [id], action: 'mute', muteMinutes: minutes });
  const unmute = () => bulk.mutate({ ids: [id], action: 'unmute' });

  return (
    <div className={`mon-status-page s-${m.status}`}>
      <header className="mon-crumbs">
        <Link to="/monitors" className="mon-crumb-root">
          <Gauge size={22} /> Monitors
        </Link>
        <ChevronRight size={18} className="muted" />
        <span>Status</span>
        <span className="mon-crumbs-right">
          <button className="icon-plain" title="Edit" onClick={() => navigate(m.syntheticTestId ? `/synthetics/tests/${m.syntheticTestId}/edit` : `/monitors/${id}/edit`)}>
            <Pencil size={18} />
          </button>
          <button
            className="icon-plain"
            title="Clone"
            disabled={clone.isPending || !!m.syntheticTestId}
            onClick={() =>
              clone.mutate(id, {
                onSuccess: (c) => navigate(`/monitors/${c.id}/edit`),
              })
            }
          >
            <Copy size={18} />
          </button>
          <button className="icon-plain" title="Export as JSON" onClick={exportJson}>
            <Upload size={18} />
          </button>
          <Menu
            triggerClassName="icon-plain"
            trigger={<MoreVertical size={18} />}
            items={[
              {
                label: 'Resolve',
                onClick: () => bulk.mutate({ ids: [id], action: 'resolve' }),
                disabled: m.status === 'ok',
              },
              {
                label: 'Delete',
                // A synthetic test's monitor goes with its test.
                disabled: !!m.syntheticTestId,
                onClick: () => window.confirm(`Delete “${m.name}”? This can't be undone.`) && bulk.mutate({ ids: [id], action: 'delete' }, { onSuccess: () => navigate('/monitors') }),
              },
            ]}
          />
        </span>
      </header>
      <div className="mon-status-scroll">
        <section className="mon-status-head">
          <div className="mon-status-title">
            {m.priority != null && <span className="mon-priority">P{m.priority}</span>}
            <StatusBadge status={m.status} />
            {m.draft && <span className="mon-draft">DRAFT</span>}
            <h1>{m.name}</h1>
          </div>
          <div className="mon-status-actions">
            {m.creator && (
              <span className="mon-creator" title="Creator">
                <UserAvatar id={m.creator.id} name={m.creator.name} size={20} /> {m.creator.name.toUpperCase()}
              </span>
            )}
            <button className={`btn${details ? ' btn-active' : ''}`} onClick={() => setDetails((d) => !d)}>
              <Gauge size={15} /> Details
            </button>
            <Menu
              triggerClassName="btn"
              trigger={m.muted ? <VolumeX size={16} /> : <Volume2 size={16} />}
              items={[
                ...MUTE_OPTIONS.map((o) => ({
                  label: `Mute for ${o.label.toLowerCase()}`,
                  onClick: () => mute(o.minutes),
                })),
                ...(m.muted ? [{ label: 'Unmute', onClick: unmute }] : []),
              ]}
            />
            {m.status !== 'ok' && (
              <button className="btn" title="Resolve" disabled={bulk.isPending} onClick={() => bulk.mutate({ ids: [id], action: 'resolve' })}>
                <Check size={16} />
              </button>
            )}
            <button className="btn" onClick={() => setDeclaring(true)}>
              <Siren size={15} /> Declare Incident
            </button>
          </div>
        </section>
        {m.muted && (
          <div className="mon-muted-banner">
            <VolumeX size={16} /> Notifications are muted {m.mutedUntil ? `for another ${fmtDuration(m.mutedUntil - Date.now())} (until ${fmtDateTime(m.mutedUntil)})` : 'until unmuted'}.
            <button className="link-btn" onClick={unmute}>
              Unmute
            </button>
          </div>
        )}
        {details && <DetailsPanel m={m} />}
        {m.syntheticTestId && (
          <div className="mon-muted-banner">
            This monitor alerts for a synthetic test; its message, tags and alert conditions are edited with the test.
            <Link to={`/synthetics/tests/${m.syntheticTestId}`}>View the test</Link>
          </div>
        )}
        <GroupFilters m={m} filters={filters} onChange={setFilters} />
        <div className={`mon-overview${m.syntheticTestId ? ' no-query' : ''}`}>
          <History m={m} filters={filters} />
          {!m.syntheticTestId && (
            <section className="mon-query-box">
              <div className="mon-query-head">
                <span className="muted">Query</span>
                <Link to={`/metrics?query=${encodeURIComponent(m.metricQuery)}`}>View in Metrics Explorer</Link>
              </div>
              <code>{m.query}</code>
            </section>
          )}
        </div>
        <Lower m={m} onMute={mute} onUnmute={unmute} onDeclare={() => setDeclaring(true)} />
      </div>
      {declaring && <DeclareModal onClose={() => setDeclaring(false)} initialTitle={`[Monitor] ${m.name}`} detection="monitor" />}
    </div>
  );
}

function DetailsPanel({ m }: { m: MonitorDetail }) {
  const o = m.options;
  const rows: [string, string][] = [
    ['Type', m.type === 'apm' ? 'APM' : m.type === 'log' ? 'Logs' : m.type === 'synthetics' ? 'Synthetic test' : 'Metric'],
    ['Priority', m.priority ? `P${m.priority}` : 'Not defined'],
    ['Detection', o?.detection === 'change' ? `Change (${o.changeMode === 'pct_change' ? '%' : 'absolute'}) vs ${o.changeMinutes} min before` : 'Threshold'],
    [
      'Thresholds',
      o
        ? [
            `alert ${fmtValue(o.thresholds.critical)}`,
            o.thresholds.warning != null && `warn ${fmtValue(o.thresholds.warning)}`,
            o.thresholds.criticalRecovery != null && `alert recovery ${fmtValue(o.thresholds.criticalRecovery)}`,
          ]
            .filter(Boolean)
            .join(' · ')
        : '—',
    ],
    ['No data', o?.notifyNoData ? `Notify after ${o.noDataMinutes} min` : 'Do not notify'],
    ['Tags', m.tags.join(', ') || '—'],
    ['Teams', m.teams.join(', ') || '—'],
    ['Created', fmtDateTime(m.createdAt)],
    ['Last modified', fmtDateTime(m.modifiedAt)],
    ['Last evaluated', m.evaluatedAt ? fmtDateTime(m.evaluatedAt) : 'Not yet'],
  ];
  return (
    <section className="mon-details">
      {rows.map(([k, v]) => (
        <div key={k}>
          <span className="muted">{k}</span>
          <span>{v}</span>
        </div>
      ))}
    </section>
  );
}

// ================================================================ status history

const MAX_GROUPS = 8;

/** The filters above the status bars: a value per group tag, the status, muted, and a text search. */
interface GroupFilter {
  tags: Record<string, string>;
  status: string;
  muted: string;
  text: string;
}

const NO_FILTER: GroupFilter = { tags: {}, status: '*', muted: '*', text: '' };

/** `kube_cluster_name:a,kube_namespace:b` → its tags. */
function groupTags(group: string): [string, string][] {
  if (group === '*') return [];
  return group.split(',').map((t) => {
    const i = t.indexOf(':');
    return i < 0 ? [t, ''] : [t.slice(0, i), t.slice(i + 1)];
  });
}

function groupMatches(m: MonitorDetail, group: string, f: GroupFilter): boolean {
  const tags = Object.fromEntries(groupTags(group));
  const state = m.groups.find((x) => x.group === group);
  if (Object.entries(f.tags).some(([k, v]) => v && tags[k] !== v)) return false;
  if (f.status !== '*' && (state?.status ?? 'ok') !== f.status) return false;
  if (f.muted !== '*' && String(m.muted || !!state?.muted) !== f.muted) return false;
  return !f.text || group.toLowerCase().includes(f.text.toLowerCase());
}

function GroupFilters({ m, filters, onChange }: { m: MonitorDetail; filters: GroupFilter; onChange: (f: GroupFilter) => void }) {
  const [searching, setSearching] = useState(!!filters.text);
  const keys = useMemo(() => {
    const out = new Map<string, Set<string>>();
    for (const g of m.groups) for (const [k, v] of groupTags(g.group)) (out.get(k) ?? out.set(k, new Set()).get(k)!).add(v);
    return [...out].map(([k, vs]) => [k, [...vs].sort()] as const);
  }, [m.groups]);
  const active = filters !== NO_FILTER && (Object.values(filters.tags).some(Boolean) || filters.status !== '*' || filters.muted !== '*' || !!filters.text);
  return (
    <section className="mon-filterbar">
      {keys.map(([k, values]) => (
        <Select
          key={k}
          label={k}
          value={filters.tags[k] ?? ''}
          options={[{ value: '', label: '*' }, ...values.map((v) => ({ value: v, label: v }))]}
          onChange={(v) => onChange({ ...filters, tags: { ...filters.tags, [k]: v } })}
          triggerClassName={`dd-trigger mon-filter${filters.tags[k] ? ' set' : ''}`}
        />
      ))}
      {keys.length > 0 && <span className="mon-filter-sep" />}
      <Select
        label="status"
        value={filters.status}
        options={[{ value: '*', label: '*' }, ...(['alert', 'warn', 'nodata', 'ok'] as MonitorStatus[]).map((st) => ({ value: st, label: STATUS_NAME[st] }))]}
        onChange={(v) => onChange({ ...filters, status: v })}
        triggerClassName={`dd-trigger mon-filter mon-filter-small${filters.status !== '*' ? ' set' : ''}`}
      />
      <Select
        label="muted"
        value={filters.muted}
        options={[
          { value: '*', label: '*' },
          { value: 'true', label: 'true' },
          { value: 'false', label: 'false' },
        ]}
        onChange={(v) => onChange({ ...filters, muted: v })}
        triggerClassName={`dd-trigger mon-filter mon-filter-small${filters.muted !== '*' ? ' set' : ''}`}
      />
      {searching ? (
        <label className="mon-filter-search">
          <Search size={15} />
          <input autoFocus placeholder="Search groups" value={filters.text} onChange={(e) => onChange({ ...filters, text: e.target.value })} />
        </label>
      ) : (
        <button className="btn mon-filter-btn" title="Search groups" onClick={() => setSearching(true)}>
          <Search size={16} />
        </button>
      )}
      <button className="link-btn mon-clear" disabled={!active} onClick={() => (onChange(NO_FILTER), setSearching(false))}>
        <XCircle size={16} /> Clear Filters
      </button>
      <span className="mon-filterbar-time">
        <TimePicker />
      </span>
    </section>
  );
}

function History({ m, filters }: { m: MonitorDetail; filters: GroupFilter }) {
  const t = useTime();
  const h = useMonitorHistory(m.id, t.range);
  const [all, setAll] = useState(false);
  const segs = h.data?.segments ?? [];
  const groups = useMemo(() => {
    const keys = [...new Set(segs.map((s) => s.group))];
    // Worst current status first.
    const rank: Record<MonitorStatus, number> = {
      alert: 0,
      warn: 1,
      nodata: 2,
      ok: 3,
    };
    const current = (g: string) => m.groups.find((x) => x.group === g)?.status ?? 'ok';
    return keys.filter((g) => groupMatches(m, g, filters)).sort((a, b) => rank[current(a)] - rank[current(b)] || a.localeCompare(b));
  }, [segs, m, filters]);
  const counts = { alert: 0, warn: 0, nodata: 0, ok: 0 };
  m.groups.forEach((g) => counts[g.status]++);
  const { from, to } = t.range;
  const ticks = useMemo(() => {
    const span = to - from;
    const step = [5, 15, 30, 60, 120, 240, 720, 1440].map((x) => x * 60e3).find((s) => span / s <= 9) ?? 2880 * 60e3;
    const out: number[] = [];
    for (let x = Math.ceil(from / step) * step; x < to; x += step) out.push(x);
    return out;
  }, [from, to]);
  const pct = (x: number) => `${((x - from) / (to - from)) * 100}%`;
  const fmtTick = (x: number) =>
    new Date(x).toLocaleTimeString(undefined, {
      hour: '2-digit',
      minute: '2-digit',
      hour12: false,
    });
  const shown = all ? groups : groups.slice(0, MAX_GROUPS);

  return (
    <section className="mon-history">
      {h.isPending ? (
        <Loading label="Loading history…" />
      ) : h.isError ? (
        <ErrorState error={h.error} onRetry={() => h.refetch()} />
      ) : groups.length === 0 ? (
        <p className="mon-empty">
          No evaluations in this timeframe yet
          {m.evaluatedAt ? '' : ' — the monitor is evaluated every minute'}.
        </p>
      ) : (
        <div className="mon-tl">
          <div className="mon-tl-axis">
            <span className="mon-tl-edge left">{fmtDateTime(from)}</span>
            {ticks.map((x) => (
              <span key={x} className="mon-tl-tick" style={{ left: pct(x) }}>
                {fmtTick(x)}
              </span>
            ))}
            <span className="mon-tl-edge right">{fmtDateTime(to)}</span>
          </div>
          {shown.map((g) => (
            <div key={g} className="mon-tl-row">
              <span className="mon-tl-label" title={g}>
                <i
                  style={{
                    background: BAR_COLOR[m.groups.find((x) => x.group === g)?.status ?? 'ok'],
                  }}
                />{' '}
                {g}
              </span>
              <div className="mon-tl-track">
                {segs
                  .filter((s) => s.group === g)
                  .map((s, i) => (
                    <Segment key={i} s={s} left={pct(Math.max(s.from, from))} width={`${((Math.min(s.to, to) - Math.max(s.from, from)) / (to - from)) * 100}%`} />
                  ))}
              </div>
            </div>
          ))}
          <div className="mon-tl-foot">
            <span className="mon-tl-legend">
              {(['alert', 'warn', 'nodata', 'ok'] as MonitorStatus[]).map((s) => (
                <span key={s}>
                  <i style={{ background: BAR_COLOR[s] }} /> {STATUS_NAME[s]}
                </span>
              ))}
            </span>
            <span className="mon-tl-counts" title="Groups by current status">
              {(['alert', 'warn', 'nodata', 'ok'] as MonitorStatus[]).map((s) => (
                <span key={s} className={`c-${s}`}>
                  {counts[s]}
                </span>
              ))}
              {groups.length > MAX_GROUPS && (
                <button className="link-btn" onClick={() => setAll((a) => !a)}>
                  {all ? 'Show fewer groups' : `View all ${groups.length} groups`}
                </button>
              )}
            </span>
          </div>
        </div>
      )}
    </section>
  );
}

function Segment({ s, left, width }: { s: StatusSegment; left: string; width: string }) {
  return (
    <span
      className="mon-tl-seg"
      style={{ left, width, background: BAR_COLOR[s.status] }}
      title={`${STATUS_NAME[s.status]} · ${fmtDateTime(s.from)} – ${fmtDateTime(s.to)} (${fmtDuration(s.to - s.from)})`}
    />
  );
}

// ================================================================ lower panels

function Lower({ m, onMute, onUnmute, onDeclare }: { m: MonitorDetail; onMute: (minutes: number | null) => void; onUnmute: () => void; onDeclare: () => void }) {
  const t = useTime();
  const h = useMonitorHistory(m.id, t.range);
  const thresholds = m.options?.thresholds ?? { critical: 0 };
  const rules = (['alert', 'warn', 'nodata'] as MonitorStatus[])
    .map((status) => {
      const text = renderMessage(m.message, {
        status,
        previous: 'ok',
        value: null,
        thresholds,
        monitorName: m.name,
        group: {},
      });
      return { status, who: messageRecipients(text) };
    })
    .concat([
      {
        status: 'ok' as MonitorStatus,
        who: messageRecipients(
          renderMessage(m.message, {
            status: 'ok',
            previous: 'alert',
            value: null,
            thresholds,
            monitorName: m.name,
            group: {},
          }),
        ),
      },
    ])
    .filter((r) => r.who.length > 0);
  const recipients = messageRecipients(m.message);
  const services = m.tags.filter((x) => x.startsWith('service:')).map((x) => x.slice(8));
  const hosts = m.groups.filter((g) => g.group.startsWith('host:') && g.status !== 'ok').map((g) => g.group.slice(5).split(',')[0]);

  return (
    <div className="mon-lower">
      <section className="mon-events">
        <h3>Event Timeline</h3>
        {h.data && h.data.events.length === 0 && <p className="muted">No events in scope</p>}
        <ul>
          {(h.data?.events ?? []).slice(0, 50).map((e) => (
            <EventRow key={e.id} e={e} />
          ))}
        </ul>
      </section>
      <section className="mon-msg">
        <div className="mon-msg-box">
          <h4>Message Template</h4>
          {m.message ? (
            <div className="mon-msg-md">
              <Markdown source={autolink(m.message)} />
            </div>
          ) : (
            <p className="muted">No message. Edit the monitor to add one with @-mentions.</p>
          )}
        </div>
        {recipients.length > 0 && (
          <>
            <p className="muted mon-msg-label">Recipients from message</p>
            <div className="mon-msg-rcpts">
              {recipients.map((r) => (
                <span key={r} className="mon-rcpt-chip">
                  <RecipientIcon r={r} /> {recipientLabel(r)}
                </span>
              ))}
            </div>
          </>
        )}
        {rules.length > 0 && (
          <div className="mon-rules">
            {rules.map((r) => (
              <div key={r.status} className="mon-rule">
                When <span className={`mon-rule-status rs-${r.status}`}>{r.status === 'ok' ? 'RECOVERED' : STATUS_NAME[r.status].toUpperCase()}</span> <ArrowRight size={14} /> Notify
                {r.who.map((w) => (
                  <span key={w} className="mon-rcpt-chip">
                    <RecipientIcon r={w} /> {recipientLabel(w)}
                  </span>
                ))}
              </div>
            ))}
          </div>
        )}
        <h3 className="mon-runbook-title">Runbook</h3>
        <Runbook m={m} />
        <Troubleshooting m={m} />
      </section>
      <aside className="mon-next">
        <h3>Next Steps</h3>
        {m.muted ? (
          <button className="btn mon-wide" onClick={onUnmute}>
            <Volume2 size={15} /> Unmute
          </button>
        ) : (
          <Menu
            align="left"
            triggerClassName="btn mon-wide"
            trigger={
              <>
                <Volume2 size={15} /> Mute <ChevronDown size={14} />
              </>
            }
            items={MUTE_OPTIONS.map((o) => ({
              label: `For ${o.label.toLowerCase()}`,
              onClick: () => onMute(o.minutes),
            }))}
          />
        )}
        <p className="muted mon-next-label">More Actions</p>
        <button className="btn mon-wide" onClick={onDeclare}>
          <Siren size={15} /> Declare Incident
        </button>
        <button className="btn mon-wide" disabled title="Needs an issue tracker integration">
          <FileText size={15} /> Create Work Item
        </button>
        <p className="muted mon-next-label">Suggested Resources</p>
        <ul className="mon-resources">
          <li>
            <Link to={`/event/explorer?${new URLSearchParams({ query: eventScope(m) })}`}>
              <List size={16} /> Event Explorer <ExternalLink size={14} />
            </Link>
          </li>
          <li>
            <Link to={`/metrics?query=${encodeURIComponent(m.metricQuery)}`}>
              <Gauge size={16} /> Metrics Explorer <ExternalLink size={14} />
            </Link>
          </li>
          {services.map((s) => (
            <li key={s}>
              <Link to={`/apm/services/${encodeURIComponent(s)}`}>
                <Network size={16} /> {s} <ExternalLink size={14} />
              </Link>
            </li>
          ))}
          {hosts.slice(0, 3).map((hst) => (
            <li key={hst}>
              <Link to={`/infrastructure?host=${encodeURIComponent(hst)}`}>
                <Server size={16} /> {hst} <ExternalLink size={14} />
              </Link>
            </li>
          ))}
          {m.teams.slice(0, 2).map((team) => (
            <li key={team}>
              <Link to={`/dashboards?q=${encodeURIComponent(team)}`}>
                <LayoutDashboard size={16} /> {team} dashboards <ExternalLink size={14} />
              </Link>
            </li>
          ))}
        </ul>
      </aside>
    </div>
  );
}

function EventRow({ e }: { e: MonitorEvent }) {
  const [open, setOpen] = useState(false);
  const to = e.toStatus as MonitorStatus;
  return (
    <li className="mon-event">
      <button onClick={() => setOpen((o) => !o)}>
        <i style={{ background: BAR_COLOR[to] }} />
        <span className="mon-event-main">
          <b>
            {STATUS_NAME[e.fromStatus as MonitorStatus]} → {STATUS_NAME[to]}
          </b>{' '}
          {e.group !== '*' && <span className="mon-tag">{e.group}</span>}
          <span className="muted">
            {fmtDateTime(e.createdAt)}
            {e.value !== null && e.value !== undefined && ` · value ${fmtValue(e.value)}`}
          </span>
          <span className="muted">{e.muted ? 'Notification suppressed' : e.recipients.length ? `Notified ${e.recipients.map((r) => `@${r}`).join(', ')}` : 'No recipients'}</span>
        </span>
      </button>
      {open && e.message && <pre className="mon-event-msg">{e.message}</pre>}
    </li>
  );
}

/** Bare URLs in a monitor message as Markdown links, so they render clickable. */
function autolink(text: string): string {
  return text.replace(/(^|[\s(])(https?:\/\/[^\s)<>]+)/g, (all, pre: string, url: string) => (pre === '(' ? all : `${pre}[${url}](${url})`));
}

/** The events and work items that concern a monitor: its scope tags (`service:`, `env:`, `team:`, …). */
function eventScope(m: MonitorDetail): string {
  const tags = m.tags.filter((t) => /^[\w.-]+:[^\s]+$/.test(t) && !t.startsWith('monitor'));
  return tags.length ? (tags.length === 1 ? tags[0] : `(${tags.join(' OR ')})`) : '';
}

/** The monitor's runbook: a notebook tagged with it, or a prompt to start one. */
function Runbook({ m }: { m: MonitorDetail }) {
  const navigate = useNavigate();
  const [dismissed, setDismissed] = useState(false);
  const tag = `monitor_id:${m.id}`;
  const found = useQuery({
    queryKey: ['notebooks', 'runbook', m.id],
    queryFn: () => searchNotebooks({ query: tag, view: 'all', sort: 'modified', dir: 'desc', limit: 5, offset: 0 }),
  });
  const create = useMutation({
    mutationFn: () =>
      createNotebook({
        title: `Runbook: ${m.name}`,
        tags: [tag],
        teams: m.teams,
        time: '4h',
        cells: [
          {
            id: newCellId(),
            definition: {
              type: 'markdown',
              text: `## When this monitor alerts\n\nMonitor: [${m.name}](${window.location.origin}/monitors/${m.id})\n\n1. Check what changed.\n2. Mitigate.\n3. Note what you did here.`,
            },
          },
        ] as never[],
      }),
    onSuccess: (nb) => navigate(`/notebooks/${nb.id}`),
  });
  const notebook = found.data?.notebooks.find((n) => n.tags.includes(tag));
  if (notebook)
    return (
      <Link className="mon-runbook mon-runbook-linked" to={`/notebooks/${notebook.id}`}>
        <BookOpen size={16} /> <b>{notebook.title}</b>
        <ArrowRight size={14} />
      </Link>
    );
  if (dismissed || found.isPending) return null;
  return (
    <div className="mon-runbook">
      <div>
        <b>
          <BookOpen size={15} /> Add a Runbook with Notebooks
        </b>
        <p className="muted">Use a Notebook to combine live data, graphs, and rich text for faster issue resolution.</p>
      </div>
      <button className="link-btn" disabled={create.isPending} onClick={() => create.mutate()}>
        Add Runbook <ArrowRight size={14} />
      </button>
      <button className="icon-plain" title="Dismiss" onClick={() => setDismissed(true)}>
        <X size={16} />
      </button>
    </div>
  );
}

const WORK_TABS = [
  { id: 'open', label: 'Open' },
  { id: 'closed', label: 'Closed' },
  { id: 'all', label: 'All' },
] as const;

/** Troubleshooting: Event Management's work items in the monitor's scope over the last week. */
function Troubleshooting({ m }: { m: MonitorDetail }) {
  const [tab, setTab] = useState<(typeof WORK_TABS)[number]['id']>('open');
  const scope = eventScope(m);
  const range = useMemo(() => ({ from: Date.now() - 7 * 86_400_000, to: Date.now() }), []);
  const q = useQuery({ queryKey: ['monitor-work-items', m.id, scope], queryFn: () => workItems({ range, query: scope }), enabled: !!scope });
  const items = (q.data?.items ?? []).filter((w) => !w.maintenance && (tab === 'all' || w.state === tab));
  const count = (id: string) => (id === 'open' ? q.data?.open : id === 'closed' ? q.data?.closed : (q.data?.open ?? 0) + (q.data?.closed ?? 0));
  return (
    <section className="mon-trouble">
      <h3>Troubleshooting</h3>
      <p className="muted">Work Items</p>
      <div className="mon-work-tabs">
        {WORK_TABS.map((t) => (
          <button key={t.id} className={tab === t.id ? 'active' : ''} onClick={() => setTab(t.id)}>
            {t.label} {tab === t.id && count(t.id) != null && <span className="mon-work-count">{count(t.id)}</span>}
          </button>
        ))}
      </div>
      {!scope ? (
        <p className="muted mon-work-empty">Tag the monitor (service, env, team, …) to see the work items in its scope.</p>
      ) : q.isPending ? (
        <Loading label="Loading work items…" />
      ) : items.length === 0 ? (
        <p className="muted mon-work-empty">No {tab === 'all' ? '' : `${tab} `}work items in this monitor's scope over the past week.</p>
      ) : (
        <ul className="mon-work-list">
          {items.slice(0, 20).map((w) => (
            <li key={w.id}>
              <Link to={`/event/inbox?${new URLSearchParams({ item: w.id })}`}>
                <span className={`mon-work-sev sev-${w.status}`}>{w.status === 'error' ? 'ERROR' : 'WARN'}</span>
                <span className="mon-work-main">
                  <b>{w.summary || w.reason || w.source}</b>
                  <small className="muted">
                    {w.source} · {w.count} events · last seen {fmtDateTime(w.lastSeen)}
                  </small>
                </span>
                <span className={`mon-work-state st-${w.state}`}>{w.state.toUpperCase()}</span>
              </Link>
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}
