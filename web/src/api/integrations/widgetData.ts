// What the geomap, distribution and host map widgets draw from a panel's values.

/** A geomap's countries: the groups named by an ISO 3166-1 alpha-2 code, summed per country. */
export function countryValues(values: { name: string; value: number }[]): { iso: string; value: number }[] {
  const out = new Map<string, number>();
  for (const { name, value } of values) {
    const iso = name.trim().toUpperCase();
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
