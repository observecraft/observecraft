// A budget's `metrics_query` (`aws.cost.amortized{env:prod} by {team}`, no aggregator) as cost search
// syntax, for reading what its tag combinations cost. Shared by the budget pages and the mock.

export interface BudgetScope {
  /** Cost search syntax: the metric's provider and the query's filters. */
  query: string;
  /** The `by {…}` tags. */
  by: string[];
  costType: 'amortized' | 'billed';
}

const CLOUDS = ['aws', 'azure', 'gcp', 'oci'];

export function budgetScope(metricsQuery: string): BudgetScope | null {
  const m = /^\s*(?:\w+:)?([\w.]+)\s*\{([^}]*)\}\s*(?:by\s*\{([^}]*)\})?\s*$/.exec(metricsQuery);
  if (!m) return null;
  const [, metric, filters, by] = m;
  const parts: string[] = [];
  let costType: BudgetScope['costType'] = 'amortized';
  if (metric !== 'all.cost') {
    const [provider, kind] = metric.split('.cost.');
    if (!kind) return null;
    costType = kind.split('.').includes('amortized') ? 'amortized' : 'billed';
    if (provider === 'custom') parts.push(...CLOUDS.map((c) => `-providername:${c}`));
    else parts.push(`providername:${provider}`);
  }
  for (const f of filters.split(',').map((x) => x.trim())) {
    if (!f || f === '*') continue;
    const neg = f.startsWith('!') || f.startsWith('-');
    const kv = neg ? f.slice(1) : f;
    const i = kv.indexOf(':');
    if (i < 0) continue;
    const v = kv.slice(i + 1);
    parts.push(`${neg ? '-' : ''}${kv.slice(0, i)}:${/[\s():"]/.test(v) ? `"${v.replace(/\\/g, '\\\\').replace(/"/g, '\\"')}"` : v}`);
  }
  return {
    query: parts.join(' '),
    by: (by ?? '')
      .split(',')
      .map((x) => x.trim())
      .filter(Boolean),
    costType,
  };
}
