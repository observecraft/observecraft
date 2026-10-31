// Small pieces shared by the Investigate and Inventory pages of App & API Protection: service, IP
// and path chips, the explorer toolbar and the orange activity sparkline.
import type { ReactNode } from 'react';
import type { Facet } from '../../../api/types';
import { selectedValues, type Term } from '../../../lib/searchQuery';
import { Globe, Hand, PanelLeftClose, PanelLeftOpen } from 'lucide-react';
import { compact } from '../../../lib/format';
import { MethodBadge } from './AppsecLayout';
import './investigate.css';

const SERVICE_COLORS = ['#3d6fd8', '#7a4fd1', '#2e9d57', '#d27a1f', '#c2408a', '#1f8fa6', '#8a6d1f', '#5b6bd9'];

function hash(s: string): number {
  let h = 0;
  for (let i = 0; i < s.length; i++) h = (h * 31 + s.charCodeAt(i)) | 0;
  return Math.abs(h);
}

/** A service: a round colored icon and its name. */
export function ServiceChip({ name, env, more }: { name: string; env?: string; more?: number }) {
  return (
    <span className="inv-svc">
      <span className="inv-svc-chip">
        <span className="inv-svc-icon" style={{ background: SERVICE_COLORS[hash(name) % SERVICE_COLORS.length] }}>
          <Globe size={11} />
        </span>
        {name}
      </span>
      {env && <span className="inv-env">env:{env}</span>}
      {more ? <span className="inv-more">+{more}</span> : null}
    </span>
  );
}

/** The emoji flag of an ISO 3166 country code (none for anything else). */
export function flag(iso: string): string {
  return /^[A-Za-z]{2}$/.test(iso) ? String.fromCodePoint(...[...iso.toUpperCase()].map((c) => 0x1f1a5 + c.charCodeAt(0))) : '';
}

const names = (() => {
  try {
    return new Intl.DisplayNames(['en'], { type: 'region' });
  } catch {
    return null;
  }
})();

export function countryName(iso: string): string {
  if (!/^[A-Za-z]{2}$/.test(iso)) return iso;
  try {
    return names?.of(iso.toUpperCase()) ?? iso;
  } catch {
    return iso;
  }
}

/** A client IP: its country's flag, the address and, when on the denylist, the blocked hand. */
export function IpChip({ ip, country, blocked, onClick }: { ip: string; country?: string; blocked?: boolean; onClick?: () => void }) {
  return (
    <span className={`inv-ip${blocked ? ' blocked' : ''}`} title={country ? countryName(country) : undefined} onClick={onClick}>
      {country && <span className="inv-flag">{flag(country) || country}</span>}
      <span className="mono">{ip}</span>
      {blocked && <Hand size={12} />}
    </span>
  );
}

/** `METHOD` chip and the route in a chip. */
export function MethodPath({ method, route }: { method: string; route: string }) {
  return (
    <span className="inv-endpoint">
      <MethodBadge method={method} />
      <span className="inv-path mono">{route}</span>
    </span>
  );
}

/** "Hide Controls | N found" and what's on the right. */
export function ExplorerToolbar({ controls, onToggle, children, right }: { controls: boolean; onToggle: () => void; children: ReactNode; right?: ReactNode }) {
  return (
    <div className="inv-toolbar">
      <button className="link-btn" onClick={onToggle}>
        {controls ? <PanelLeftClose size={15} /> : <PanelLeftOpen size={15} />} {controls ? 'Hide Controls' : 'Show Controls'}
      </button>
      <span className="inv-sep" />
      {children}
      <span className="syn-grow" />
      {right}
    </div>
  );
}

/** Bars of activity with the maximum written at the end of a dashed line. */
export function ActivitySpark({ values, tone = 'orange' }: { values: number[]; tone?: 'orange' | 'blue' }) {
  const max = Math.max(0, ...values);
  return (
    <span className={`inv-spark ${tone}`}>
      <span className="inv-spark-bars">
        {values.map((v, i) => (
          <i key={i} style={{ height: v ? `${Math.max(8, (v / Math.max(1, max)) * 100)}%` : 0 }} />
        ))}
        <span className="inv-spark-max" />
      </span>
      <small>{compact(max)}</small>
    </span>
  );
}

/** Downloads rows as a CSV file. */
export function downloadCsv(name: string, header: string[], rows: (string | number)[][]) {
  const esc = (v: string | number) => {
    const s = String(v);
    return /[",\n]/.test(s) ? `"${s.replace(/"/g, '""')}"` : s;
  };
  const blob = new Blob([[header, ...rows].map((r) => r.map(esc).join(',')).join('\n')], { type: 'text/csv' });
  const a = document.createElement('a');
  a.href = URL.createObjectURL(blob);
  a.download = name;
  a.click();
  URL.revokeObjectURL(a.href);
}

interface LocalFacet<T> {
  key: string;
  label: string;
  group: string;
  get: (row: T) => string[];
}

/** Facets of rows, and the rows the checked values keep (values of one facet are ORed, facets ANDed). */
export function localFacets<T>(rows: T[], defs: LocalFacet<T>[], terms: Term[]) {
  const sel = selectedValues(terms);
  const facets: Facet[] = defs.map((f) => {
    const counts = new Map<string, number>();
    for (const r of rows) for (const v of new Set(f.get(r))) counts.set(v, (counts.get(v) ?? 0) + 1);
    return { key: f.key, label: f.label, group: f.group, values: [...counts.entries()].sort((a, b) => b[1] - a[1]).map(([value, count]) => ({ value, count })) };
  });
  const shown = rows.filter((r) => defs.every((f) => !sel[f.key]?.length || f.get(r).some((v) => sel[f.key].includes(v))));
  return { facets, shown };
}
