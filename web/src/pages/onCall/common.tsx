import { useMemo, useState, type ReactNode } from 'react';
import { Link } from 'react-router-dom';
import { ArrowDown, ArrowUp, ChevronDown, ChevronRight, Search, type LucideIcon } from 'lucide-react';
import type { OnCallPage, OnCallPageStatus, OnCallUser } from '../../api/onCall/client';
import { UserAvatar } from '../../components/ui/UserAvatar';
import { fmtDateTime } from '../incidents/queries';

export const TIME_ZONES = [
  'UTC',
  'Europe/London',
  'Europe/Berlin',
  'Europe/Warsaw',
  'Asia/Jerusalem',
  'Asia/Kolkata',
  'Asia/Singapore',
  'Asia/Tokyo',
  'Australia/Sydney',
  'America/New_York',
  'America/Chicago',
  'America/Denver',
  'America/Los_Angeles',
  'America/Sao_Paulo',
];

/** The browser's time zone, listed first when it isn't in TIME_ZONES. */
export function browserTimeZone(): string {
  try {
    return Intl.DateTimeFormat().resolvedOptions().timeZone || 'UTC';
  } catch {
    return 'UTC';
  }
}

export function timeZoneOptions(current?: string): string[] {
  const extra = [browserTimeZone(), current].filter((z): z is string => !!z && !TIME_ZONES.includes(z));
  return [...new Set([...extra, ...TIME_ZONES])];
}

export function PageStatusBadge({ status }: { status: OnCallPageStatus }) {
  return <span className={`onc-status onc-status-${status}`}>{status.toUpperCase()}</span>;
}

export function Person({ user, size = 20 }: { user: Pick<OnCallUser, 'id' | 'name'>; size?: number }) {
  return (
    <span className="onc-person">
      <UserAvatar id={user.id} name={user.name} size={size} />
      <span className="ellipsis">{user.name}</span>
    </span>
  );
}

/** Several people: avatars, then "+N". */
export function People({ users, max = 3, empty = '—' }: { users: Pick<OnCallUser, 'id' | 'name'>[]; max?: number; empty?: ReactNode }) {
  if (!users.length) return <span className="muted">{empty}</span>;
  if (users.length === 1) return <Person user={users[0]} />;
  return (
    <span className="onc-people" title={users.map((u) => u.name).join(', ')}>
      {users.slice(0, max).map((u) => (
        <UserAvatar key={u.id} id={u.id} name={u.name} size={22} />
      ))}
      {users.length > max && <span className="onc-more">+{users.length - max}</span>}
    </span>
  );
}

export function EmptyState({ icon: Icon, title, hint, children }: { icon: LucideIcon; title: string; hint?: ReactNode; children?: ReactNode }) {
  return (
    <div className="onc-empty">
      <Icon size={64} strokeWidth={1.2} />
      <h3>{title}</h3>
      {hint && <p className="muted">{hint}</p>}
      {children}
    </div>
  );
}

export function Segmented<T extends string>({ value, options, onChange }: { value: T; options: [T, string][]; onChange: (v: T) => void }) {
  return (
    <div className="onc-seg" role="tablist">
      {options.map(([v, label]) => (
        <button key={v} role="tab" aria-selected={v === value} className={v === value ? 'active' : ''} onClick={() => onChange(v)}>
          {label}
        </button>
      ))}
    </div>
  );
}

export function Toggle({ on, onChange, label }: { on: boolean; onChange: (v: boolean) => void; label: string }) {
  return (
    <label className="onc-toggle">
      <button type="button" role="switch" aria-checked={on} className={`switch${on ? ' on' : ''}`} onClick={() => onChange(!on)} />
      {label}
    </label>
  );
}

/** A pinned facet box: checkbox per team, with a filter box once the list is long. */
export function TeamFacet({ teams, selected, onChange, counts }: { teams: { handle: string; name: string }[]; selected: string[]; onChange: (v: string[]) => void; counts?: Record<string, number> }) {
  const [open, setOpen] = useState(true);
  const [filter, setFilter] = useState('');
  const shown = teams.filter((t) => !filter || t.name.toLowerCase().includes(filter.toLowerCase()) || t.handle.includes(filter.toLowerCase()));
  const toggle = (h: string) => onChange(selected.includes(h) ? selected.filter((x) => x !== h) : [...selected, h]);
  return (
    <div className="onc-facet">
      <button className="onc-facet-title" onClick={() => setOpen((o) => !o)}>
        {open ? <ChevronDown size={14} /> : <ChevronRight size={14} />}
        Teams
        {selected.length > 0 && (
          <span
            className="onc-facet-clear"
            onClick={(e) => {
              e.stopPropagation();
              onChange([]);
            }}
          >
            Clear
          </span>
        )}
      </button>
      {open && (
        <div className="onc-facet-body">
          {teams.length > 8 && (
            <div className="onc-facet-search">
              <Search size={12} />
              <input placeholder="Filter teams" value={filter} onChange={(e) => setFilter(e.target.value)} />
            </div>
          )}
          {shown.map((t) => (
            <label key={t.handle} className="onc-facet-value">
              <input type="checkbox" checked={selected.includes(t.handle)} onChange={() => toggle(t.handle)} />
              <span className="ellipsis">{t.name}</span>
              {counts && <span className="muted">{counts[t.handle] ?? 0}</span>}
            </label>
          ))}
          {!shown.length && <p className="muted onc-facet-none">No teams</p>}
        </div>
      )}
    </div>
  );
}

type PageSort = 'title' | 'status' | 'team' | 'createdAt';
const STATUS_RANK: Record<OnCallPageStatus, number> = { triggered: 0, acknowledged: 1, resolved: 2 };

/** PAGE | STATUS | PAGED TO | RESPONDERS | CREATION DATE, newest first; headers sort. */
export function PagesTable({ pages, empty, onOpen, compact = false }: { pages: OnCallPage[]; empty: ReactNode; onOpen?: (p: OnCallPage) => void; compact?: boolean }) {
  const [sort, setSort] = useState<{ key: PageSort; desc: boolean }>({ key: 'createdAt', desc: true });
  const rows = useMemo(() => {
    const cmp = (a: OnCallPage, b: OnCallPage) =>
      sort.key === 'title'
        ? a.title.localeCompare(b.title)
        : sort.key === 'status'
          ? STATUS_RANK[a.status] - STATUS_RANK[b.status]
          : sort.key === 'team'
            ? (a.team ?? '').localeCompare(b.team ?? '')
            : a.createdAt - b.createdAt;
    return [...pages].sort((a, b) => (sort.desc ? -cmp(a, b) : cmp(a, b)) || b.createdAt - a.createdAt);
  }, [pages, sort]);
  const head = (key: PageSort, label: string, className?: string) => (
    <th className={className}>
      <button className="onc-sort" onClick={() => setSort((s) => ({ key, desc: s.key === key ? !s.desc : key === 'createdAt' }))}>
        {label}
        {sort.key === key && (sort.desc ? <ArrowDown size={12} /> : <ArrowUp size={12} />)}
      </button>
    </th>
  );
  return (
    <table className={`onc-table${compact ? ' compact' : ''}`}>
      <thead>
        <tr>
          {head('title', 'PAGE', 'onc-col-page')}
          {head('status', 'STATUS', 'onc-col-status')}
          {head('team', 'PAGED TO', 'onc-col-team')}
          <th className="onc-col-resp">RESPONDERS</th>
          {head('createdAt', 'CREATION DATE', 'onc-col-date')}
        </tr>
      </thead>
      <tbody>
        {rows.length === 0 ? (
          <tr className="onc-empty-row">
            <td colSpan={5}>{empty}</td>
          </tr>
        ) : (
          rows.map((p) => (
            <tr key={p.id} onClick={() => onOpen?.(p)}>
              <td className="onc-col-page">
                <span className="onc-page-title ellipsis">
                  {p.urgency === 'low' && <span className="onc-low">LOW</span>}
                  {p.title}
                </span>
              </td>
              <td className="onc-col-status">
                <PageStatusBadge status={p.status} />
              </td>
              <td className="onc-col-team ellipsis">{p.team ? `@${p.team}` : (p.policy?.name ?? '—')}</td>
              <td className="onc-col-resp">
                <People users={p.responders} />
              </td>
              <td className="onc-col-date">{fmtDateTime(p.createdAt)}</td>
            </tr>
          ))
        )}
      </tbody>
    </table>
  );
}

export function ViewAll({ to }: { to: string }) {
  return (
    <Link to={to} className="onc-view-all">
      View All →
    </Link>
  );
}

export function errorText(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}
