/** The library pipelines whose name or filter (`source:…`) contains the query, ignoring case. */
export function filterLibrary<P extends { name: string; filter: string }>(pipelines: P[], query: string): P[] {
  const q = query.trim().toLowerCase();
  return q ? pipelines.filter((p) => p.name.toLowerCase().includes(q) || p.filter.toLowerCase().includes(q)) : pipelines;
}
