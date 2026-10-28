// Metrics rules helpers shared by Metrics → Settings and the mock (server twins: routes/metric_rules.rs,
// oc-intake/src/metric_rules.rs).

/** Whether a rule's metric pattern (`*` for any characters, case ignored) matches the metric name. */
export function globMatch(pattern: string, name: string): boolean {
  const escaped = pattern
    .toLowerCase()
    .split('*')
    .map((p) => p.replace(/[.+?^${}()|[\]\\]/g, '\\$&'))
    .join('.*');
  return new RegExp(`^${escaped}$`).test(name.toLowerCase());
}

/** What a rule does, in a few words. */
export function ruleSummary(r: { action: string; tags: string[] }): string {
  if (r.action === 'drop') return 'Drop the metrics';
  if (r.action === 'strip_tags') return `Strip ${r.tags.join(', ')}`;
  return `Query by ${r.tags.join(', ')} only`;
}

/** Metric patterns and tag keys the API accepts. */
export const validPattern = (p: string) => /^[A-Za-z0-9_.\-/*]{1,200}$/.test(p);
export const validTagKey = (t: string) => /^[A-Za-z0-9_.\-/]{1,200}$/.test(t);

/** `a, b ,, c` → unique trimmed tag keys. */
export const parseTags = (text: string): string[] => [
  ...new Set(
    text
      .split(',')
      .map((t) => t.trim())
      .filter(Boolean),
  ),
];
