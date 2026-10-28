import { useState, type ReactNode } from 'react';
import { BrandLogo } from '../../components/brand/BrandLogo';
import { recipientLogo } from '../../lib/channelLogos';
import { AtSign, ChevronDown, ChevronRight, Hash, Mail, Search, Users, VolumeX, Webhook } from 'lucide-react';
import type { FacetCount, MonitorStatus } from '../../api/monitors/client';
import { Menu } from '../../components/ui/Dropdown';

export const STATUS_LABEL: Record<MonitorStatus, string> = {
  alert: 'ALERT',
  warn: 'WARN',
  nodata: 'NO DATA',
  ok: 'OK',
};
export const STATUS_NAME: Record<MonitorStatus, string> = {
  alert: 'Alert',
  warn: 'Warn',
  nodata: 'No Data',
  ok: 'OK',
};
export const PRIORITY_LABEL: Record<string, string> = {
  '1': 'P1 (Critical)',
  '2': 'P2 (High)',
  '3': 'P3 (Medium)',
  '4': 'P4 (Low)',
  '5': 'P5 (Info)',
  '0': 'Not Defined',
};

export function StatusBadge({ status, small = false }: { status: MonitorStatus; small?: boolean }) {
  return <span className={`mon-status mst-${status}${small ? ' small' : ''}`}>{STATUS_LABEL[status]}</span>;
}

export function toggle<T>(list: T[], v: T): T[] {
  return list.includes(v) ? list.filter((x) => x !== v) : [...list, v];
}

/** "2h 13m", "45m", "3d 4h". */
export function fmtDuration(ms: number): string {
  const m = Math.max(1, Math.round(ms / 60_000));
  if (m < 60) return `${m}m`;
  const h = Math.floor(m / 60);
  if (h < 24) return m % 60 ? `${h}h ${m % 60}m` : `${h}h`;
  const d = Math.floor(h / 24);
  return h % 24 ? `${d}d ${h % 24}h` : `${d}d`;
}

export function fmtDateTime(ms: number): string {
  return new Date(ms).toLocaleString(undefined, {
    month: 'short',
    day: 'numeric',
    year: 'numeric',
    hour: 'numeric',
    minute: '2-digit',
  });
}

export function fmtValue(v: number | null | undefined): string {
  if (v === null || v === undefined) return '—';
  const a = Math.abs(v);
  return a >= 100 ? v.toFixed(0) : a >= 1 ? v.toFixed(2).replace(/\.?0+$/, '') : v.toPrecision(2);
}

/** Muted-left cell: speaker icon plus the remaining time, ∞ when muted with no end. */
export function MutedLeft({ muted, until }: { muted: boolean; until?: number | null }) {
  if (!muted) return null;
  return (
    <span className="mon-muted" title={until ? `Muted until ${fmtDateTime(until)}` : 'Muted until unmuted'}>
      <VolumeX size={15} /> {until ? fmtDuration(until - Date.now()) : '∞'}
    </span>
  );
}

type RecipientKind = 'slack' | 'pagerduty' | 'webhook' | 'email' | 'team' | 'user';

export function recipientKind(r: string): RecipientKind {
  if (r.includes('@')) return 'email';
  if (r.startsWith('slack-')) return 'slack';
  if (r.startsWith('pagerduty-') || r.startsWith('opsgenie-')) return 'pagerduty';
  if (r.startsWith('webhook-')) return 'webhook';
  if (r.startsWith('team-')) return 'team';
  return 'user';
}

const KIND_LABEL: Record<RecipientKind, string> = {
  slack: 'Slack',
  pagerduty: 'On-call',
  webhook: 'Webhook',
  email: 'Email',
  team: 'Team',
  user: 'User',
};

export function RecipientIcon({ r, size = 15 }: { r: string; size?: number }) {
  const kind = recipientKind(r);
  const logo = recipientLogo(r);
  const icon = logo ? (
    <BrandLogo name={logo} size={size} />
  ) : kind === 'slack' ? (
    <Hash size={size} />
  ) : kind === 'webhook' ? (
    <Webhook size={size} />
  ) : kind === 'email' ? (
    <Mail size={size} />
  ) : kind === 'team' ? (
    <Users size={size} />
  ) : kind === 'pagerduty' ? (
    <b>P</b>
  ) : (
    <AtSign size={size} />
  );
  return (
    <span className={`mon-rcpt-icon k-${kind}`} title={`${KIND_LABEL[kind]}: @${r}`}>
      {icon}
    </span>
  );
}

/** Recipient label as in Datadog's facet ("Slack checkout-alerts", "Webhook: incident-bot"). */
export function recipientLabel(r: string): string {
  const kind = recipientKind(r);
  const rest = r.replace(/^(slack|pagerduty|opsgenie|webhook|team)-/, '');
  return kind === 'email' || kind === 'user' ? r : `${KIND_LABEL[kind]}: ${rest}`;
}

export function RecipientIcons({ recipients, max = 2 }: { recipients: string[]; max?: number }) {
  // One icon per kind, like the list in the reference UI.
  const kinds: string[] = [];
  const shown = recipients.filter((r) => {
    const k = recipientKind(r);
    if (kinds.includes(k)) return false;
    kinds.push(k);
    return true;
  });
  return (
    <span className="mon-rcpts">
      {shown.slice(0, max).map((r) => (
        <RecipientIcon key={r} r={r} />
      ))}
      {recipients.length > Math.min(max, shown.length) && <span className="mon-more">+{recipients.length - Math.min(max, shown.length)}</span>}
    </span>
  );
}

export function TagChips({ tags, max = 3 }: { tags: string[]; max?: number }) {
  return (
    <span className="mon-tags">
      {tags.slice(0, max).map((t) => (
        <span key={t} className="mon-tag" title={t}>
          {t}
        </span>
      ))}
      {tags.length > max && (
        <span className="mon-more" title={tags.slice(max).join(', ')}>
          +{tags.length - max}
        </span>
      )}
    </span>
  );
}

export function Facet({ title, children, defaultOpen = true, badge }: { title: string; children: ReactNode; defaultOpen?: boolean; badge?: string }) {
  const [open, setOpen] = useState(defaultOpen);
  return (
    <div className="mon-facet">
      <button className="mon-facet-title" onClick={() => setOpen((o) => !o)}>
        {open ? <ChevronDown size={16} /> : <ChevronRight size={16} />} {title}
        {badge && <span className="badge-new">{badge}</span>}
      </button>
      {open && <div className="mon-facet-body">{children}</div>}
    </div>
  );
}

export function FacetCheck({ checked, onChange, label, count, indent = false, color }: { checked: boolean; onChange: () => void; label: ReactNode; count?: number; indent?: boolean; color?: string }) {
  return (
    <label className={`mon-check${indent ? ' indent' : ''}`}>
      <input type="checkbox" className="checkbox" checked={checked} onChange={onChange} />
      {color && <i className="mon-check-bar" style={{ background: color }} />}
      <span className="mon-check-label">{label}</span>
      {count !== undefined && <span className="mon-count">{fmtCount(count)}</span>}
    </label>
  );
}

export function fmtCount(n: number): string {
  return n >= 1000 ? `${(n / 1000).toFixed(n >= 10_000 ? 0 : 2).replace(/\.?0+$/, '')}K` : String(n);
}

/**
 * A multi-value facet. Nothing selected means "all", shown as every box checked; unchecking a value
 * selects all the others (the reference UI's behavior).
 */
export function ValuesFacet({
  values,
  selected,
  onChange,
  label = (v) => v,
  icon,
  searchable = false,
  limit = 8,
}: {
  values: FacetCount[];
  selected: string[];
  onChange: (next: string[]) => void;
  label?: (v: string) => ReactNode;
  icon?: (v: string) => ReactNode;
  searchable?: boolean;
  limit?: number;
}) {
  const [filter, setFilter] = useState('');
  const [more, setMore] = useState(false);
  const f = filter.trim().toLowerCase();
  const shown = values.filter((v) => !f || v.value.toLowerCase().includes(f));
  const all = values.map((v) => v.value);
  const isOn = (v: string) => selected.length === 0 || selected.includes(v);
  const flip = (v: string) => {
    const base = selected.length === 0 ? all : selected;
    const next = toggle(base, v);
    onChange(next.length === all.length ? [] : next);
  };
  return (
    <>
      {searchable && values.length > limit && (
        <label className="mon-facet-search">
          <Search size={14} />
          <input placeholder={`Filter ${values.length} values`} value={filter} onChange={(e) => setFilter(e.target.value)} />
        </label>
      )}
      {shown.slice(0, more ? undefined : limit).map((v) => (
        <FacetCheck
          key={v.value}
          checked={isOn(v.value)}
          onChange={() => flip(v.value)}
          label={
            <>
              {icon?.(v.value)} {label(v.value)}
            </>
          }
          count={v.count}
        />
      ))}
      {shown.length === 0 && <p className="muted mon-facet-empty">No values</p>}
      {shown.length > limit && (
        <button className="link-btn mon-view-more" onClick={() => setMore((m) => !m)}>
          {more ? 'View fewer values' : 'View more values'}
        </button>
      )}
    </>
  );
}

export const MUTE_OPTIONS: { label: string; minutes: number | null }[] = [
  { label: '1 hour', minutes: 60 },
  { label: '4 hours', minutes: 240 },
  { label: '12 hours', minutes: 720 },
  { label: '1 day', minutes: 1440 },
  { label: '1 week', minutes: 10_080 },
  { label: 'Until unmuted', minutes: null },
];

export function MuteMenu({
  onMute,
  onUnmute,
  disabled,
  label = 'Mute',
  triggerClassName = 'btn',
}: {
  onMute: (minutes: number | null) => void;
  onUnmute?: () => void;
  disabled?: boolean;
  label?: ReactNode;
  triggerClassName?: string;
}) {
  if (disabled) {
    return (
      <button className={triggerClassName} disabled>
        {label} <ChevronDown size={13} />
      </button>
    );
  }
  return (
    <Menu
      align="left"
      triggerClassName={triggerClassName}
      trigger={
        <>
          {label} <ChevronDown size={13} />
        </>
      }
      items={[
        ...MUTE_OPTIONS.map((o) => ({
          label: `Mute for ${o.label.toLowerCase()}`,
          onClick: () => onMute(o.minutes),
        })),
        ...(onUnmute ? [{ label: 'Unmute', onClick: onUnmute }] : []),
      ]}
    />
  );
}
