// Small pieces shared by the archive pages (Archive Search, Archiving, Rehydrations).
import { useQuery } from '@tanstack/react-query';
import { listArchives } from '../../api/archives/client';

const HOUR = 3_600_000;

/** `<input type="datetime-local">` value for an epoch ms (local time). */
export function toLocalInput(ms: number): string {
  const d = new Date(ms - new Date(ms).getTimezoneOffset() * 60_000);
  return d.toISOString().slice(0, 16);
}

export function fromLocalInput(v: string): number {
  return new Date(v).getTime();
}

const PRESETS: [string, number][] = [
  ['Past 1h', HOUR],
  ['Past 24h', 24 * HOUR],
  ['Past 7d', 7 * 24 * HOUR],
  ['Past 30d', 30 * 24 * HOUR],
];

/** Absolute time range picker: from / to plus quick presets. */
export function RangeFields({ range, onChange }: { range: { from: number; to: number }; onChange: (r: { from: number; to: number }) => void }) {
  return (
    <div className="ar-range">
      <label className="form-label">
        From
        <input type="datetime-local" value={toLocalInput(range.from)} onChange={(e) => onChange({ ...range, from: fromLocalInput(e.target.value) })} />
      </label>
      <label className="form-label">
        To
        <input type="datetime-local" value={toLocalInput(range.to)} onChange={(e) => onChange({ ...range, to: fromLocalInput(e.target.value) })} />
      </label>
      <div className="ar-presets">
        {PRESETS.map(([label, ms]) => (
          <button
            key={label}
            type="button"
            className="btn btn-sm"
            onClick={() => {
              const to = Date.now();
              onChange({ from: to - ms, to });
            }}
          >
            {label}
          </button>
        ))}
      </div>
    </div>
  );
}

export const useArchives = () => useQuery({ queryKey: ['archives'], queryFn: listArchives, retry: 1 });

export function formatSize(n: number): string {
  if (n >= 1 << 30) return `${(n / (1 << 30)).toFixed(1)} GiB`;
  if (n >= 1 << 20) return `${(n / (1 << 20)).toFixed(1)} MiB`;
  if (n >= 1 << 10) return `${(n / (1 << 10)).toFixed(1)} KiB`;
  return `${n} B`;
}

export function formatRange(from: number, to: number): string {
  const f = (ms: number) => new Date(ms).toLocaleString(undefined, { month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit' });
  return `${f(from)} – ${f(to)}`;
}
