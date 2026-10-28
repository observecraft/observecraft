// Change panels: each group's present value against its value a period earlier.
import type { PanelSeries } from './panels';

export interface ChangeOptions {
  /** absolute: present − past; relative: the change in percent of the past value. */
  change_type?: string;
  increase_good?: boolean;
  /** change | name | present | past */
  order_by?: string;
  order_dir?: string;
}

export interface ChangeRow {
  group: string;
  present: number;
  past: number;
  change: number;
  /** Whether the change goes the good way (an increase when increase_good). */
  good: boolean;
}

/** Rows from a change panel's series: the time-shifted formula (`__shift`) is the past, the other the present. */
export function changeRows(series: PanelSeries[], options: ChangeOptions): ChangeRow[] {
  const present = new Map<string, number>();
  const past = new Map<string, number>();
  for (const s of series) {
    const value = s.points.at(-1)?.[1];
    if (value === undefined) continue;
    (s.column?.includes('__shift') ? past : present).set(s.group ?? s.name, value);
  }
  const rows = [...present].map(([group, now]) => {
    const before = past.get(group) ?? 0;
    const change = options.change_type === 'relative' ? (before === 0 ? NaN : ((now - before) / Math.abs(before)) * 100) : now - before;
    return { group, present: now, past: before, change, good: now >= before === (options.increase_good ?? true) };
  });
  const key = (r: ChangeRow): number | string => (options.order_by === 'name' ? r.group : options.order_by === 'present' ? r.present : options.order_by === 'past' ? r.past : r.change);
  const dir = options.order_dir === 'asc' ? 1 : -1;
  return rows.sort((a, b) => {
    const [x, y] = [key(a), key(b)];
    return dir * (typeof x === 'string' ? x.localeCompare(y as string) : (x as number) - (y as number));
  });
}
