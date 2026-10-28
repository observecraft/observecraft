// Pieces shared by the status page detail (Incident Response → Status Pages) and the read-only
// rendering at /status/<slug>: status labels, the components with their uptime bars, and notices.
import type { ReactNode } from 'react';
import { AlertTriangle, Check, OctagonAlert } from 'lucide-react';
import { Link } from 'react-router-dom';
import { groupComponents, uptimeDays, uptimePercent, UPTIME_DAYS, worstStatus, type UptimeDay } from '../../api/statusPages/status';
import {
  COMPONENT_STATUS_LABEL,
  NOTICE_STATUS_LABEL,
  OVERALL_STATUS_LABEL,
  type ComponentStatus,
  type NoticeImpact,
  type NoticeStatus,
  type StatusPageComponent,
  type StatusPageNotice,
  type StatusPageVisualization,
} from '../../api/statusPages/types';
import { Markdown } from '../dashboards/widgets/markdown';
import './statusPages.css';

export function StatusDot({ status }: { status: ComponentStatus }) {
  return <span className={`stp-dot stp-${status}`} aria-hidden />;
}

export function ComponentStatusLabel({ status }: { status: ComponentStatus }) {
  return (
    <span className={`stp-label stp-text-${status}`}>
      <StatusDot status={status} /> {COMPONENT_STATUS_LABEL[status]}
    </span>
  );
}

export function NoticeStatusChip({ status }: { status: NoticeStatus }) {
  return <span className={`stp-notice-chip stp-notice-${status}`}>{NOTICE_STATUS_LABEL[status]}</span>;
}

/** The page's overall status as a banner. */
export function OverallBanner({ status }: { status: ComponentStatus }) {
  return <div className={`stp-banner stp-${status}`}>{OVERALL_STATUS_LABEL[status]}</div>;
}

/** A status as a filled circle with a white mark: a check when operational. */
export function StatusBadge({ status, size = 16 }: { status: ComponentStatus; size?: number }) {
  const Icon = status === 'operational' ? Check : status === 'major_outage' ? OctagonAlert : AlertTriangle;
  return (
    <span className={`stp-badge stp-${status}`} style={{ width: size, height: size }} title={COMPONENT_STATUS_LABEL[status]}>
      <Icon size={Math.round(size * 0.6)} strokeWidth={3} />
    </span>
  );
}

/** A notice's date and time in the viewer's locale, to the minute. */
const when = (t: number) => new Date(t).toLocaleString(undefined, { dateStyle: 'medium', timeStyle: 'short' });

const hours = (ms: number) => {
  const m = Math.round(ms / 60_000);
  return m < 60 ? `${m} min` : `${Math.floor(m / 60)} h ${m % 60} min`;
};

function dayTitle(d: UptimeDay) {
  const date = new Date(d.start).toLocaleDateString(undefined, { timeZone: 'UTC', year: 'numeric', month: 'short', day: 'numeric' });
  if (d.noData) return `${date}: no data`;
  if (d.status === 'operational') return `${date}: no incidents`;
  return `${date}: ${COMPONENT_STATUS_LABEL[d.status]}${d.downMs ? `, ${hours(d.downMs)} of outage` : ''}`;
}

/** One bar per UTC day of the last 60, from the notices' history; grey before the component existed. */
export function UptimeBars({ componentId, notices, now, since }: { componentId: string; notices: StatusPageNotice[]; now: number; since?: number | null }) {
  const days = uptimeDays(componentId, notices, now, UPTIME_DAYS, since);
  const pct = uptimePercent(days, now, since);
  return (
    <div className="stp-bars" role="img" aria-label={`${pct.toFixed(2)}% uptime over the last ${UPTIME_DAYS} days`}>
      {days.map((d) => (
        <span key={d.start} className={`stp-bar ${d.noData ? 'stp-bar-nodata' : `stp-${d.status}`}`} title={dayTitle(d)} />
      ))}
    </div>
  );
}

/** The components by group, each with its current status and uptime bars. */
export function ComponentList({ components, notices, visualization = 'bars_and_uptime' }: { components: StatusPageComponent[]; notices: StatusPageNotice[]; visualization?: StatusPageVisualization }) {
  const now = Date.now();
  if (!components.length) return <p className="muted">No components yet.</p>;
  const row = (c: StatusPageComponent) => (
    <div key={c.id} className="stp-component">
      <div className="stp-component-head">
        <span className="stp-component-name">{c.name}</span>
        <span className="stp-component-state">
          {visualization === 'bars_and_uptime' && (
            <span className="stp-component-pct">{uptimePercent(uptimeDays(c.id, notices, now, UPTIME_DAYS, c.createdAt), now, c.createdAt).toFixed(2)}% uptime</span>
          )}
          {visualization === 'names' && <span className={`stp-text-${c.currentStatus}`}>{COMPONENT_STATUS_LABEL[c.currentStatus]}</span>}
          <StatusBadge status={c.currentStatus} />
        </span>
      </div>
      {visualization !== 'names' && <UptimeBars componentId={c.id} notices={notices} now={now} since={c.createdAt} />}
    </div>
  );
  return (
    <div className="stp-components">
      {groupComponents(components).map((g, i) =>
        g.group ? (
          <section key={g.group} className="stp-group">
            <h3 className="stp-group-title">{g.group}</h3>
            {g.components.map(row)}
          </section>
        ) : (
          <div key={`ungrouped-${i}`} className="stp-ungrouped">
            {g.components.map(row)}
          </div>
        ),
      )}
    </div>
  );
}

/** A resolved notice's components with the worst impact any of its updates gave them. */
function affectedOverTime(notice: StatusPageNotice): NoticeImpact[] {
  const worst = new Map<string, ComponentStatus>();
  for (const i of [...notice.updates].reverse().flatMap((u) => u.impacts)) worst.set(i.componentId, worstStatus([worst.get(i.componentId) ?? 'operational', i.impact]));
  return [...worst].map(([componentId, impact]) => ({ componentId, impact }));
}

/** A notice: its status, the components it affects (open) or affected (resolved), and its updates, newest first. */
export function NoticeCard({ notice, components, actions }: { notice: StatusPageNotice; components: StatusPageComponent[]; actions?: ReactNode }) {
  const name = (id: string) => components.find((c) => c.id === id)?.name ?? 'Removed component';
  const affected = notice.resolvedAt == null ? notice.impacts : affectedOverTime(notice);
  return (
    <article className="stp-notice">
      <header className="stp-notice-head">
        <div>
          <h3 className="stp-notice-title">{notice.title}</h3>
          <div className="stp-notice-meta muted">
            <NoticeStatusChip status={notice.status} />
            <span>
              {when(notice.startedAt)}
              {notice.resolvedAt != null && ` – ${when(notice.resolvedAt)}`}
            </span>
            {notice.incidentId != null && <Link to={`/incidents/${notice.incidentId}`}>IR-{notice.incidentId}</Link>}
          </div>
          {affected.length > 0 && (
            <div className="stp-impacts">
              {affected.map((i) => (
                <span key={i.componentId} className="tag-chip">
                  <StatusDot status={i.impact} /> {name(i.componentId)}: {COMPONENT_STATUS_LABEL[i.impact]}
                </span>
              ))}
            </div>
          )}
        </div>
        {actions}
      </header>
      <ol className="stp-updates">
        {notice.updates.map((u) => (
          <li key={u.id} className="stp-update">
            <div className="stp-update-head">
              <strong>{NOTICE_STATUS_LABEL[u.status]}</strong>
              <span className="muted">
                {when(u.createdAt)}
                {u.createdBy && ` · ${u.createdBy}`}
              </span>
            </div>
            <div className="stp-update-body">
              <Markdown source={u.message} />
            </div>
          </li>
        ))}
      </ol>
    </article>
  );
}
