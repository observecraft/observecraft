// Host Map: hosts as hexagons, grouped like the host list and colored by a usage percentage.

export interface MapHost {
  name: string;
  group?: string | null;
}

/** Hosts by group, in the order groups first appear; ungrouped hosts under "All hosts". */
export function mapGroups<H extends MapHost>(hosts: H[]): { key: string; hosts: H[] }[] {
  const groups = new Map<string, H[]>();
  for (const h of hosts) {
    const key = h.group || 'All hosts';
    groups.set(key, [...(groups.get(key) ?? []), h]);
  }
  return [...groups].map(([key, list]) => ({ key, hosts: list }));
}

/** 0 % green → 100 % red; unknown is neutral. */
export function fillColor(pct: number | null | undefined): string {
  if (pct == null || !Number.isFinite(pct)) return 'var(--bg-subtle)';
  const hue = Math.round(130 * (1 - Math.min(100, Math.max(0, pct)) / 100));
  return `hsl(${hue} 55% 45%)`;
}
