// What the geomap, distribution and host map widgets draw from a panel's values.

/** A group's values without its query or tag keys: `query1 · country_code:RO` → `RO`, `os:linux, team:ci` → `linux, ci`
 * (metric groups carry `key:value`, log groups only values). */
export function groupLabel(name: string): string {
  return name
    .replace(/^.* · /, '')
    .split(', ')
    .map((pair) => (/^[\w.@-]+:/.test(pair) ? pair.slice(pair.indexOf(':') + 1) : pair))
    .join(', ');
}

/** A geomap's countries: the groups named by an ISO 3166-1 alpha-2 code, summed per country. */
export function countryValues(values: { name: string; value: number }[]): { iso: string; value: number }[] {
  const out = new Map<string, number>();
  for (const { name, value } of values) {
    const iso = groupLabel(name).trim().toUpperCase();
    if (/^[A-Z]{2}$/.test(iso)) out.set(iso, (out.get(iso) ?? 0) + value);
  }
  return [...out].map(([iso, value]) => ({ iso, value }));
}

/** A distribution's histogram: `count` even buckets from the smallest value to the largest. */
export function distributionBuckets(values: number[], count = 12): { from: number; to: number; count: number }[] {
  const finite = values.filter(Number.isFinite);
  if (!finite.length) return [];
  const [min, max] = [Math.min(...finite), Math.max(...finite)];
  if (min === max) return [{ from: min, to: max, count: finite.length }];
  const width = (max - min) / count;
  const buckets = Array.from({ length: count }, (_, i) => ({ from: min + i * width, to: min + (i + 1) * width, count: 0 }));
  for (const v of finite) buckets[Math.min(count - 1, Math.floor((v - min) / width))].count++;
  return buckets;
}

/** Datadog's default host map palette: green for the lowest values to orange for the highest. */
const HOSTMAP = ['#3a9e48', '#7cb342', '#c0ca33', '#fdd835', '#ffb300', '#fb8c00', '#f4511e'];

/** A host's colour on the map, by where its value falls between the map's smallest and largest. */
export function hostmapColor(value: number, min: number, max: number): string {
  const share = max > min ? Math.min(1, Math.max(0, (value - min) / (max - min))) : 0;
  return HOSTMAP[Math.round(share * (HOSTMAP.length - 1))];
}

export interface SunburstNode {
  name: string;
  value: number;
  children: SunburstNode[];
}

/**
 * A sunburst (pie) widget's rings, as Datadog draws it: one ring per group-by tag (the first tag innermost), each
 * slice named by its tag value and sized by the sum of its groups, largest first; the legend lists the inner ring.
 * Groups are `tag:value, tag:value` (a formula's prefix `query1 · ` dropped); empty and negative ones are left out.
 */
export function sunburstTree(values: { name: string; value: number }[]): { total: number; nodes: SunburstNode[]; legend: { name: string; value: number; percent: number }[] } {
  const root: SunburstNode = { name: '', value: 0, children: [] };
  for (const { name, value } of values) {
    if (!(value > 0)) continue;
    const group = name.replace(/^.* · /, '');
    const path = group === '*' ? ['All'] : group.split(', ').map((pair) => (pair.includes(':') ? pair.slice(pair.indexOf(':') + 1) : pair));
    let node = root;
    node.value += value;
    for (const part of path) {
      let child = node.children.find((c) => c.name === part);
      if (!child) node.children.push((child = { name: part, value: 0, children: [] }));
      child.value += value;
      node = child;
    }
  }
  const sort = (nodes: SunburstNode[]): SunburstNode[] => nodes.sort((a, b) => b.value - a.value).map((n) => ({ ...n, children: sort(n.children) }));
  const nodes = sort(root.children);
  return { total: root.value, nodes, legend: nodes.map((n) => ({ name: n.name, value: n.value, percent: root.value ? (n.value / root.value) * 100 : 0 })) };
}
