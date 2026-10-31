// Building blocks shared by the Logs → Configuration pages: the title row, the flow boxes that link
// the steps a log goes through (pipelines → indexes → archives…), numbered rows, filter queries and
// collapsible list sections.
import type { ReactNode } from 'react';
import { Link } from 'react-router-dom';
import { ChevronDown, ChevronRight, Filter, Lock } from 'lucide-react';
import './logcfg.css';

export function ConfigTitle({ title, extra, actions }: { title: string; extra?: ReactNode; actions?: ReactNode }) {
  return (
    <div className="lc-title-row">
      <h1 className="lc-title">
        {title} {extra}
      </h1>
      {actions && <div className="lc-title-actions">{actions}</div>}
    </div>
  );
}

/** A step of the log flow, linking to its page; `last` drops the connector below it. */
export function FlowStep({ icon, label, to, right, last }: { icon: ReactNode; label: string; to: string; right?: ReactNode; last?: boolean }) {
  return (
    <div className={`lc-flow-step${last ? ' last' : ''}`}>
      <span className="lc-flow-icon">{icon}</span>
      <Link to={to}>{label}</Link>
      {right && <span className="lc-flow-right">{right}</span>}
    </div>
  );
}

/** A position in an ordered list (pipelines, indexes, archives). */
export function Num({ n, muted }: { n: number; muted?: boolean }) {
  return <span className={`lc-num${muted ? ' muted' : ''}`}>{n}</span>;
}

/** A log search, with the filter mark; '' reads as every log. */
export function FilterQuery({ query, title }: { query: string; title?: string }) {
  return (
    <span className="lc-filter" title={title ?? (query || '*')}>
      <Filter size={11} className="lc-filter-icon" />
      <span className="mono">{query || '*'}</span>
    </span>
  );
}

/** The collapsible header of a list (`PIPELINES 5 active | 1 disabled`). */
export function SectionHead({ open, onToggle, label, children, right }: { open: boolean; onToggle: () => void; label: string; children?: ReactNode; right?: ReactNode }) {
  return (
    <div className="lc-section-head">
      <button className="icon-plain" onClick={onToggle} aria-expanded={open} title={open ? 'Collapse' : 'Expand'}>
        {open ? <ChevronDown size={16} /> : <ChevronRight size={16} />}
      </button>
      <span className="lc-section-label">{label}</span>
      {children}
      {right && <span className="lc-section-right">{right}</span>}
    </div>
  );
}

/** Uppercase column headers of a section. */
export function Columns({ cols, template, lock }: { cols: string[]; template: string; lock?: boolean }) {
  return (
    <div className="lc-cols" style={{ gridTemplateColumns: template }}>
      {cols.map((c, i) => (
        <span key={i}>
          {c}
          {lock && i === cols.length - 1 && <Lock size={12} className="lc-lock" />}
        </span>
      ))}
    </div>
  );
}

/** Initials in a circle, for "last edited by". */
export function Avatar({ name }: { name?: string | null }) {
  if (!name) return <span className="lc-avatar system" title="ObserveCraft" />;
  const initials = name
    .split(/\s+/)
    .map((w) => w[0])
    .join('')
    .slice(0, 2)
    .toUpperCase();
  const hue = [...name].reduce((h, c) => (h * 31 + c.charCodeAt(0)) % 360, 7);
  return (
    <span className="lc-avatar" title={name} style={{ background: `hsl(${hue} 45% 42%)` }}>
      {initials}
    </span>
  );
}

/** Five bars filled by a share (0–1) of the logs. */
export function UsageBars({ share, title }: { share: number | undefined; title?: string }) {
  const lit = share === undefined ? 0 : Math.max(share > 0 ? 1 : 0, Math.round(share * 5));
  return (
    <span className="lc-usage" title={title}>
      {[0, 1, 2, 3, 4].map((i) => (
        <i key={i} className={i < lit ? 'on' : ''} />
      ))}
    </span>
  );
}

/** `key:value` tags as chips, the first two then `+N`. */
export function TagChips({ tags, max = 2 }: { tags: string[]; max?: number }) {
  if (!tags.length) return <span />;
  return (
    <span className="lc-tags">
      {tags.slice(0, max).map((t) => {
        const [k, ...v] = t.split(':');
        return (
          <span key={t} className="lc-tag">
            {v.length ? (
              <>
                <span className="muted">{k}:</span>
                {v.join(':')}
              </>
            ) : (
              t
            )}
          </span>
        );
      })}
      {tags.length > max && (
        <span className="lc-tag more" title={tags.slice(max).join(', ')}>
          +{tags.length - max}
        </span>
      )}
    </span>
  );
}

/** A labelled select, as in the filter rows. */
export function LabeledSelect({ label, value, onChange, options }: { label: string; value: string; onChange: (v: string) => void; options: [string, string][] }) {
  return (
    <label className="lc-select">
      <span>{label}</span>
      <select value={value} onChange={(e) => onChange(e.target.value)}>
        {options.map(([v, l]) => (
          <option key={v} value={v}>
            {l}
          </option>
        ))}
      </select>
    </label>
  );
}

export function fmtDate(ms: number | null | undefined): string {
  if (!ms) return '';
  return new Date(ms).toLocaleDateString(undefined, { month: 'short', day: '2-digit', year: 'numeric' });
}

/** A CSV file download. */
export function downloadCsv(name: string, rows: (string | number)[][]) {
  const csv = rows.map((r) => r.map((c) => `"${String(c).replace(/"/g, '""')}"`).join(',')).join('\n');
  const a = document.createElement('a');
  a.href = URL.createObjectURL(new Blob([csv], { type: 'text/csv' }));
  a.download = name;
  a.click();
  URL.revokeObjectURL(a.href);
}
