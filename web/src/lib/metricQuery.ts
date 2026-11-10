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

const RE = /^\s*(avg|sum|min|max|count):([\w.]+)\{([^}]*)\}(?:\s+by\s+\{([^}]*)\})?\s*$/;

export const QUERY_IN_FORMULA = /(?:avg|sum|min|max|count):[\w.]+\{[^}]*\}(?:\s+by\s+\{[^}]*\})?/g;

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

/** Splits on `separator` outside parentheses. */
function splitTop(text: string, separator: RegExp): string[] {
  const out: string[] = [];
  let depth = 0;
  let start = 0;
  for (let i = 0; i < text.length; i++) {
    if (text[i] === '(') depth++;
    else if (text[i] === ')') depth--;
    else if (depth === 0) {
      const m = separator.exec(text.slice(i));
      if (m && m.index === 0) {
        out.push(text.slice(start, i));
        start = i + m[0].length;
        i = start - 1;
      }
    }
  }
  out.push(text.slice(start));
  return out.map((t) => t.trim()).filter(Boolean);
}

/**
 * Datadog's boolean tag filters (`$host AND code:4* OR code:5*`, `env IN (prod, qa)`, `NOT service:billing`) as our
 * comma-separated filters: AND is a comma, and values one tag may take are `tag:(a OR b)`; OR binds the values of
 * one tag (it never joins different tags here). Null when an OR joins different tags.
 */
function booleanFilters(text: string): string | null {
  const normalized = text
    .replace(
      /([\w.@-]+)\s+NOT\s+IN\s*\(([^()]*)\)/g,
      (_, k: string, list: string) =>
        `!${k}:(${list
          .split(',')
          .map((v) => v.trim())
          .filter(Boolean)
          .join(' OR ')})`,
    )
    .replace(
      /([\w.@-]+)\s+IN\s*\(([^()]*)\)/g,
      (_, k: string, list: string) =>
        `${k}:(${list
          .split(',')
          .map((v) => v.trim())
          .filter(Boolean)
          .join(' OR ')})`,
    )
    .replace(/(^|[\s(,])(?:NOT\s+|-)(?=[\w.@$-]+:)/g, '$1!');
  const out: string[] = [];
  for (const term of splitTop(normalized, /^(?:\s+AND\s+|\s*,\s*)/)) {
    const inner = /^\((.*)\)$/.test(term) && !/^[\w.@-]+:\(/.test(term) ? term.slice(1, -1) : term;
    const operands = splitTop(inner, /^\s+OR\s+/);
    if (operands.length === 1) {
      out.push(operands[0]);
      continue;
    }
    // A bare tag in an OR (`status:400 OR s`): no series is tagged that way, so it adds nothing.
    const pairs = operands.filter((o) => o.includes(':')).map((o) => [o.slice(0, o.indexOf(':')), o.slice(o.indexOf(':') + 1)]);
    if (!pairs.length || pairs.some(([k]) => k !== pairs[0][0] || k.startsWith('!'))) return null;
    out.push(`${pairs[0][0]}:(${pairs.map(([, v]) => v.replace(/^\((.*)\)$/, '$1')).join(' OR ')})`);
  }
  return out.join(',');
}

const BOOLEAN = /\s(?:AND|OR|IN)\s|(?:^|[\s,])NOT\s|^\(|[\s,]\(/;

export function parseMetricQuery(q: string): ParsedMetricQuery | null {
  const m = RE.exec(q);
  if (!m) return null;
  const text = BOOLEAN.test(m[3]) ? booleanFilters(m[3]) : m[3];
  if (text === null) return null;
  const filters: ParsedMetricQuery['filters'] = [];
  const vars: string[] = [];
  for (const raw of text
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
      } else filters.push({ key: (prefixes[name] ?? name).replace(/^@/, ''), value, negate: false });
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
  // Not a metric query: a log, signal or event search of an integration panel.
  if (!parsed) return resolveSearchQuery(expanded, tvs, values, false);
  return formatMetricQuery(resolveTemplateVars(parsed, vals, prefixes));
}

/**
 * Applies template variables to a search (logs, signals, events, monitors) as Datadog does: each `$name` the query
 * writes becomes `prefix:value` where it stands, and is dropped while the variable is `*`. Names may hold dots
 * (`$logs.request.path`); the longest name that matches wins. Variables the query doesn't name don't apply, unless it
 * names none and `appendUnnamed`: our own widgets' searches are written without references and take every variable
 * that is set.
 */
export function resolveSearchQuery(query: string, tvs: TemplateVariable[], values: TemplateVarValues, appendUnnamed = true): string {
  const byName = new Map(tvs.map((t) => [t.name, t]));
  const names = [...byName.keys()].sort((a, b) => b.length - a.length).map((n) => n.replace(/[.*+?^${}()|[\]\\]/g, '\\$&'));
  const quote = (v: string) => (/[\s:"()]/.test(v) ? `"${v.replace(/"/g, '\\"')}"` : v);
  const term = (tv: TemplateVariable) => {
    const value = values[tv.name] ?? tv.default;
    if (!value || value === '*') return '';
    return tv.prefix ? `${tv.prefix}:${quote(value)}` : value;
  };
  // `$name.value`: the value alone (`host:$host.value`), `*` while unset.
  const valueOf = names.length ? new RegExp(`\\$(${names.join('|')})\\.value\\b`, 'g') : null;
  const byValue = !!valueOf?.test(query);
  if (valueOf) query = query.replace(valueOf, (_, name: string) => quote(values[name] ?? byName.get(name)!.default ?? '*') || '*');
  // `@$name` reads the variable as an attribute (`@prefix:value`); it goes with the variable while that is `*`.
  const reference = names.length ? new RegExp(`(@?)\\$(${names.join('|')})(?![\\w.])`, 'g') : null;
  const named = !!reference?.test(query) || byValue;
  const resolved = named
    ? query.replace(reference!, (_, at: string, name: string) => {
        const t = term(byName.get(name)!);
        return t && at && !t.startsWith('@') ? `@${t}` : t;
      })
    : appendUnnamed
      ? [query, ...tvs.map(term)].join(' ')
      : query;
  return resolved.replace(/\s+/g, ' ').trim();
}

/**
 * A widget title with the dashboard's variables it names, as Datadog shows them: `$name` as `prefix:value`,
 * `$name.value` as the value alone, `*` while the variable is unset. Other `$` text stays as written.
 */
export function resolveTitle(title: string, tvs: TemplateVariable[], values: TemplateVarValues): string {
  const byName = new Map(tvs.map((t) => [t.name, t]));
  const names = [...byName.keys()].sort((a, b) => b.length - a.length).map((n) => n.replace(/[.*+?^${}()|[\]\\]/g, '\\$&'));
  if (!names.length) return title;
  return title.replace(new RegExp(`\\$(${names.join('|')})(\\.value)?(?![\\w.])`, 'g'), (_, name: string, dotValue?: string) => {
    const tv = byName.get(name)!;
    const value = values[name] ?? tv.default ?? '*';
    if (!value || value === '*') return '*';
    return dotValue || !tv.prefix ? value : `${tv.prefix}:${value}`;
  });
}
