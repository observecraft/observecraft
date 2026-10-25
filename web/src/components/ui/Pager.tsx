import { ArrowLeft, ArrowRight } from 'lucide-react';
import './ui.css';

interface Props {
  offset: number;
  limit: number;
  total: number;
  onChange: (offset: number) => void;
}

/** "← 1 2 … 23 →" pager for server-side offset/limit paging. */
export function Pager({ offset, limit, total, onChange }: Props) {
  const pages = Math.max(1, Math.ceil(total / limit));
  const current = Math.floor(offset / limit);
  if (pages <= 1) return null;
  const shown = new Set([0, pages - 1, current - 1, current, current + 1].filter((p) => p >= 0 && p < pages));
  const list = [...shown].sort((a, b) => a - b);
  return (
    <nav className="pager" aria-label="Pagination">
      <button disabled={current === 0} onClick={() => onChange((current - 1) * limit)} aria-label="Previous page">
        <ArrowLeft size={16} />
      </button>
      {list.map((p, i) => (
        <span key={p} className="pager-item">
          {i > 0 && p - list[i - 1] > 1 && <span className="pager-gap">…</span>}
          <button className={p === current ? 'active' : ''} onClick={() => onChange(p * limit)}>
            {p + 1}
          </button>
        </span>
      ))}
      <button disabled={current >= pages - 1} onClick={() => onChange((current + 1) * limit)} aria-label="Next page">
        <ArrowRight size={16} />
      </button>
    </nav>
  );
}

/** Byte counts as "387 MiB", "508 B". */
export function formatBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  const units = ['KiB', 'MiB', 'GiB', 'TiB'];
  let v = n / 1024;
  let u = 0;
  while (v >= 1024 && u < units.length - 1) {
    v /= 1024;
    u++;
  }
  return `${v < 10 ? v.toFixed(1) : Math.round(v)} ${units[u]}`;
}

/** "2 days", "17 hours" — compact age. */
export function formatAge(sec: number): string {
  if (sec < 3600) return `${Math.max(1, Math.round(sec / 60))} minutes`;
  if (sec < 86400) return `${Math.round(sec / 3600)} hours`;
  const d = Math.round(sec / 86400);
  return d === 1 ? '1 day' : `${d} days`;
}

/** Inline usage bar used in tables (CPU %, memory). */
export function UsageBar({ pct, tone = 'primary' }: { pct: number; tone?: 'primary' | 'ok' }) {
  return (
    <span className={`usage-bar usage-${tone}`}>
      <span style={{ width: `${Math.min(100, Math.max(2, pct))}%` }} />
    </span>
  );
}
