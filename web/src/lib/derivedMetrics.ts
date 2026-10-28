// Derived metrics: a metric name that queries resolve to a formula over other metric queries. The API
// resolves them (server: routes/derived_metrics.rs); the mock and the editor's preview use these twins.
import { formatMetricQuery, parseMetricQuery, QUERY_IN_FORMULA } from './metricQuery';

/** Derived name → formula. */
export type DerivedFormulas = Record<string, string>;

/**
 * The query or formula with every derived metric query replaced by its formula in parentheses: the
 * derived query's filters are added to each query of the formula and its `by` (when it has one)
 * replaces theirs. Throws when a formula uses another derived metric.
 */
export function expandDerived(text: string, derived: DerivedFormulas): string {
  if (Object.keys(derived).length === 0) return text;
  return text.replace(QUERY_IN_FORMULA, (query) => {
    const outer = parseMetricQuery(query);
    const formula = outer && derived[outer.metric];
    if (!outer || !formula) return query;
    const body = formula.replace(QUERY_IN_FORMULA, (inner) => {
      const q = parseMetricQuery(inner);
      if (!q) return inner;
      if (derived[q.metric]) throw new Error(`'${outer.metric}' uses the derived metric '${q.metric}': derived metrics can't be nested`);
      const filters = [...q.filters, ...outer.filters.filter((f) => !q.filters.some((x) => x.key === f.key && x.value === f.value && x.negate === f.negate))];
      return formatMetricQuery({ ...q, filters, by: outer.by.length ? outer.by : q.by });
    });
    return `(${body})`;
  });
}

/** `sum:a{*} by {host}` → the row fields the editor edits. */
export function queryRow(query: string): { agg: 'avg' | 'sum' | 'min' | 'max'; metric: string; from: string; by: string } {
  const q = parseMetricQuery(query);
  if (!q) return { agg: 'avg', metric: '', from: '', by: '' };
  return { agg: q.aggregator, metric: q.metric, from: q.filters.map((f) => `${f.negate ? '!' : ''}${f.key}:${f.value}`).join(', '), by: q.by.join(', ') };
}

/** Derived metrics whose name, formula or creator contains the search, by name. */
export function filterDerived<T extends { name: string; formula: string; createdBy: string; description: string }>(list: T[], search: string): T[] {
  const s = search.trim().toLowerCase();
  return list.filter((m) => !s || [m.name, m.formula, m.createdBy, m.description].some((v) => v.toLowerCase().includes(s))).sort((a, b) => a.name.localeCompare(b.name));
}

/** The tags a derived metric can be filtered and grouped by: those of the metrics its formula reads. */
export function derivedTags(formula: string, tagsOf: (metric: string) => string[]): string[] {
  const metrics = (formula.match(QUERY_IN_FORMULA) ?? []).map((q) => parseMetricQuery(q)?.metric).filter((m): m is string => Boolean(m));
  return [...new Set(metrics.flatMap(tagsOf))].sort();
}
