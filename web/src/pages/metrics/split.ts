// Split graph (Metrics Explorer): one small chart per group of a grouped query.

interface Series {
  name: string;
  points: [number, number][];
}

/** The groups to draw, highest peak first, and how many were left out past `limit`. */
export function splitSeries<S extends Series>(series: S[], limit: number): { shown: S[]; hidden: number } {
  const peak = (s: S) => (s.points.length ? Math.max(...s.points.map((p) => p[1])) : -Infinity);
  const sorted = [...series].sort((a, b) => peak(b) - peak(a) || a.name.localeCompare(b.name));
  return { shown: sorted.slice(0, limit), hidden: Math.max(0, sorted.length - limit) };
}
