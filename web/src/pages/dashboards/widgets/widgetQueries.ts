// The metric queries behind a widget, for the actions every widget offers (create a monitor, metrics info).
import type { WidgetDefinition } from '../../../api/dashboards/types';
import { normalizeRequest, type IntegrationWidgetDefinition } from '../../../api/integrations/evaluate';

export interface WidgetQueries {
  /** Metric queries by name (query1, a, …), unresolved: template variables stay `$var`. */
  queries: { name: string; query: string }[];
  /** The first formula over them, when the widget has one. */
  formula?: string;
}

/** A widget's metric queries; empty for notes, images, lists, groups and log or event panels. */
export function widgetQueries(def: WidgetDefinition): WidgetQueries {
  switch (def.type) {
    case 'integration': {
      const original = def.original as IntegrationWidgetDefinition;
      if (!Array.isArray(original.requests)) return { queries: [] };
      const requests = original.requests.map((r) => normalizeRequest(r));
      const queries = requests.flatMap((r) => r.queries.filter((q) => q.data_source === 'metrics').map((q) => ({ name: q.name, query: q.query })));
      return { queries, formula: requests[0]?.formulas?.find((f) => !/_before\(|timeshift|calendar_shift/.test(f.formula))?.formula };
    }
    case 'timeseries':
    case 'table':
      return { queries: def.requests.map((r, i) => ({ name: `query${i + 1}`, query: r.query })) };
    case 'query_value':
    case 'toplist':
    case 'bar_chart':
    case 'change':
    case 'heatmap':
    case 'distribution':
    case 'pie':
    case 'treemap':
      return { queries: [{ name: 'query1', query: def.request.query }] };
    default:
      return { queries: [] };
  }
}

/**
 * The monitor query for a widget: its first formula with each query written out when the formula is plain
 * arithmetic between queries (what metric monitors evaluate), otherwise its first query.
 */
export function monitorQuery({ queries, formula }: WidgetQueries, resolve: (q: string) => string): string | undefined {
  if (!queries.length) return undefined;
  const byName = new Map(queries.map((q) => [q.name, resolve(q.query)]));
  if (formula && /^[\w\s+\-*/().]+$/.test(formula) && !/\b(?!query\d)[a-z_]+\s*\(/i.test(formula)) {
    let ok = true;
    const text = formula.replace(/\b[a-zA-Z_]\w*\b/g, (name) => {
      const q = byName.get(name);
      if (!q) ok = false;
      return q ?? name;
    });
    if (ok) return text;
  }
  return byName.get(queries[0].name);
}

/** Metric names a widget reads, in order of first use. */
export function widgetMetrics(w: WidgetQueries): string[] {
  return [...new Set(w.queries.map((q) => /:([\w.]+)\{/.exec(q.query)?.[1]).filter((m): m is string => !!m))];
}
