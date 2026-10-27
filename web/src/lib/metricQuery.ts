// Metric query syntax used by dashboard widgets (subset of the Datadog-style grammar):
//   <space_agg>:<metric>{<filters>} [by {<tag>,<tag>}]
// filters: `*`, `tag:value`, `!tag:value`, `$template_var`, comma-separated.
// Shared by the widget editor (build/parse) and the query backend (evaluate).
import type { SpaceAggregator, TemplateVariable, TemplateVarValues } from '../api/dashboards/types';

export interface ParsedMetricQuery {
  aggregator: SpaceAggregator;
  metric: string;
  filters: { key: string; value: string; negate: boolean }[];
  /** Unresolved template variable references ("$kube_cluster_name"). */
  vars: string[];
  by: string[];
}

const RE = /^\s*(avg|sum|min|max):([\w.]+)\{([^}]*)\}(?:\s+by\s+\{([^}]*)\})?\s*$/;

const QUERY_IN_FORMULA = /(?:avg|sum|min|max):[\w.]+\{[^}]*\}(?:\s+by\s+\{[^}]*\})?/g;

/**
 * The metric queries of a monitor formula (`sum:a{*} by {x} - sum:b{*} by {x}`, `default_zero(… * 100 / …)`),
 * or null when it isn't one: arithmetic (+ - * /), numbers, parentheses and default_zero() between queries.
 */
export function formulaQueries(q: string): ParsedMetricQuery[] | null {
  const queries = (q.match(QUERY_IN_FORMULA) ?? []).map(parseMetricQuery);
  if (queries.length === 0 || queries.some((x) => x === null)) return null;
  const rest = q.replace(QUERY_IN_FORMULA, ' Q ').replace(/default_zero\(/g, '(');
  // Operands and operators alternate: Q or a number, then an operator, …; parentheses balance.
  const tokens = rest.match(/Q|\d+(?:\.\d+)?|[-+*/()]|\S/g) ?? [];
  let depth = 0;
  let expectOperand = true;
  for (const t of tokens) {
    if (t === '(' && expectOperand) depth++;
    else if (t === ')' && !expectOperand && depth > 0) depth--;
    else if ((t === 'Q' || /^\d/.test(t)) && expectOperand) expectOperand = false;
    else if ('+-*/'.includes(t) && t.length === 1 && !expectOperand) expectOperand = true;
    else return null;
  }
  return depth === 0 && !expectOperand ? (queries as ParsedMetricQuery[]) : null;
}

export function parseMetricQuery(q: string): ParsedMetricQuery | null {
  const m = RE.exec(q);
  if (!m) return null;
  const filters: ParsedMetricQuery['filters'] = [];
  const vars: string[] = [];
  for (const raw of m[3]
    .split(',')
    .map((s) => s.trim())
    .filter(Boolean)) {
    if (raw === '*') continue;
    if (raw.startsWith('$')) {
      vars.push(raw);
      continue;
    }
    const negate = raw.startsWith('!');
    const body = negate ? raw.slice(1) : raw;
    const i = body.indexOf(':');
    if (i > 0) filters.push({ key: body.slice(0, i), value: body.slice(i + 1), negate });
  }
  const by = (m[4] ?? '')
    .split(',')
    .map((s) => s.trim())
    .filter(Boolean);
  return { aggregator: m[1] as SpaceAggregator, metric: m[2], filters, vars, by };
}

export function formatMetricQuery(p: ParsedMetricQuery): string {
  const parts = [...p.filters.map((f) => `${f.negate ? '!' : ''}${f.key}:${f.value}`), ...p.vars];
  const by = p.by.length ? ` by {${p.by.join(',')}}` : '';
  return `${p.aggregator}:${p.metric}{${parts.length ? parts.join(',') : '*'}}${by}`;
}

/**
 * Replaces `$var` references with `prefix:value` filters (dropped when the value is "*").
 * `prefixes` maps variable name → tag key.
 */
export function resolveTemplateVars(q: ParsedMetricQuery, values: TemplateVarValues, prefixes: Record<string, string>): ParsedMetricQuery {
  const filters = [...q.filters];
  for (const ref of q.vars) {
    const name = ref.slice(1);
    const value = values[name];
    if (value && value !== '*') {
      if (prefixes[name] === '') {
        const parsed = parseMetricQuery(`avg:scope{${value}}`);
        if (parsed) filters.push(...parsed.filters);
      } else filters.push({ key: prefixes[name] ?? name, value, negate: false });
    }
  }
  return { ...q, filters, vars: [] };
}

/** Human label for a series group: "kube_cluster_name:a, kube_namespace:b" → used in legends/top lists. */
export function groupLabel(tags: Record<string, string>): string {
  const entries = Object.entries(tags);
  return entries.length ? entries.map(([k, v]) => `${k}:${v}`).join(', ') : '*';
}

/** Resolves `$vars` in a query string against a dashboard's template variables. Invalid queries pass through. */
export function resolveQuery(query: string, tvs: TemplateVariable[], values: TemplateVarValues): string {
  const prefixes = Object.fromEntries(tvs.map((t) => [t.name, t.prefix]));
  const vals = Object.fromEntries(tvs.map((t) => [t.name, values[t.name] ?? t.default]));
  const expanded = query.replace(/\$([\w]+)\.value\b/g, (reference, name: string) => vals[name] ?? reference);
  const parsed = parseMetricQuery(expanded);
  if (!parsed) return expanded;
  return formatMetricQuery(resolveTemplateVars(parsed, vals, prefixes));
}
