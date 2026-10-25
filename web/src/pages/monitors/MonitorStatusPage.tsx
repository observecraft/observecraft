import { useMemo, useState } from 'react';
import { Link, useNavigate, useParams } from 'react-router-dom';
import {
  ArrowRight,
  BookOpen,
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
import { Menu } from '../../components/ui/Dropdown';
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
          <button className="icon-plain" title="Edit" onClick={() => navigate(`/monitors/${id}/edit`)}>
            <Pencil size={18} />
          </button>
          <button
            className="icon-plain"
            title="Clone"
            disabled={clone.isPending}
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
                onClick: () => window.confirm(`Delete “${m.name}”? This can't be undone.`) && bulk.mutate({ ids: [id], action: 'delete' }, { onSuccess: () => navigate('/monitors') }),
              },
            ]}
          />
        </span>
      </header>
      <div className="mon-status-scroll">
        <section className="mon-status-head">
          <div className="mon-status-title">
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
        <History m={m} />
        <section className="mon-query-box">
          <div className="mon-query-head">
            <span className="muted">Query</span>
            <Link to={`/metrics?query=${encodeURIComponent(m.metricQuery)}`}>View in Metrics Explorer</Link>
          </div>
          <code>{m.query}</code>
        </section>
        <Lower m={m} onMute={mute} onUnmute={unmute} onDeclare={() => setDeclaring(true)} />
      </div>
      {declaring && <DeclareModal onClose={() => setDeclaring(false)} initialTitle={`[Monitor] ${m.name}`} detection="monitor" />}
    </div>
  );
}

function DetailsPanel({ m }: { m: MonitorDetail }) {
  const o = m.options;
  const rows: [string, string][] = [
    ['Type', m.type === 'apm' ? 'APM' : 'Metric'],
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

function History({ m }: { m: MonitorDetail }) {
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
    return keys.sort((a, b) => rank[current(a)] - rank[current(b)] || a.localeCompare(b));
  }, [segs, m.groups]);
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
      <div className="mon-history-bar">
        <TimePicker />
      </div>
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
  const [runbook, setRunbook] = useState(true);
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
          <pre>{m.message || <span className="muted">No message. Edit the monitor to add one with @-mentions.</span>}</pre>
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
        {runbook && (
          <div className="mon-runbook">
            <div>
              <b>
                <BookOpen size={15} /> Add a Runbook
              </b>
              <p className="muted">Link the steps responders should follow in the monitor message, e.g. “Runbook: https://wiki.example.com/runbooks/high-load”.</p>
            </div>
            <Link to={`/monitors/${m.id}/edit`} className="link-btn">
              Add Runbook <ArrowRight size={14} />
            </Link>
            <button className="icon-plain" title="Dismiss" onClick={() => setRunbook(false)}>
              <X size={16} />
            </button>
          </div>
        )}
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
