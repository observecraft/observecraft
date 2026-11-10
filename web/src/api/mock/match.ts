// Mock-only query evaluator. In production the query service evaluates searches;
// the UI only parses/edits query strings (lib/searchQuery.ts).
import type { SearchTerm } from '../../lib/searchQuery';
import type { JsonValue, LogEvent } from '../types';

/** A field's value, or every value it has inside arrays of objects (`@install_targets.name`). */
export type FieldGetter = (key: string) => string | string[] | undefined;

export function getPath(obj: Record<string, JsonValue>, path: string): JsonValue | undefined {
  let cur: JsonValue | undefined = obj;
  for (const part of path.split('.')) {
    if (cur === null || typeof cur !== 'object' || Array.isArray(cur)) return undefined;
    cur = (cur as Record<string, JsonValue>)[part];
  }
  return cur;
}

export function scalar(v: JsonValue | undefined): string | undefined {
  return v === undefined || v === null || typeof v === 'object' ? undefined : String(v);
}

/** The scalars a path reaches through arrays of objects, as Datadog flattens them (`install_targets.name` of `install_targets: [{name}]`). */
function pathValues(cur: JsonValue | undefined, parts: string[]): string[] {
  if (Array.isArray(cur)) return parts.length ? cur.flatMap((item) => pathValues(item, parts)) : [];
  if (!parts.length) {
    const v = scalar(cur);
    return v === undefined || v === '' ? [] : [v];
  }
  if (cur === null || typeof cur !== 'object') return [];
  return pathValues((cur as Record<string, JsonValue>)[parts[0]], parts.slice(1));
}

/** Every value of a query key in a log: `logField`'s, else the values `@path` reaches inside arrays of objects. */
export function logFieldValues(log: LogEvent, key: string): string[] {
  const v = logField(log, key);
  if (v !== undefined) return [v];
  return key.startsWith('@') ? pathValues(log.attributes, key.slice(1).split('.')) : [];
}

/** Resolves a query key against a log: reserved fields, `@attribute.path`, or a tag. */
export function logField(log: LogEvent, key: string): string | undefined {
  switch (key) {
    case 'index':
      return 'main'; // single index until the backend supports several
    case 'service':
    case 'status':
    case 'host':
    case 'source':
      return log[key];
  }
  if (key.startsWith('@')) return scalar(getPath(log.attributes, key.slice(1)));
  return log.tags[key];
}

/** `>46080`, `<=1.5`: a numeric comparison of the value, as the server makes it; undefined when the pattern isn't one. */
function compare(pattern: string, actual: string): boolean | undefined {
  const m = /^(>=|<=|>|<)(-?\d+(?:\.\d+)?(?:e[+-]?\d+)?)$/i.exec(pattern);
  if (!m) return undefined;
  const [v, n] = [Number(actual.trim()), Number(m[2])];
  if (actual.trim() === '' || !Number.isFinite(v)) return false;
  return m[1] === '>' ? v > n : m[1] === '>=' ? v >= n : m[1] === '<' ? v < n : v <= n;
}

function matchesPattern(pattern: string, actual: string): boolean {
  const numeric = compare(pattern, actual);
  if (numeric !== undefined) return numeric;
  const p = pattern.toLowerCase();
  const a = actual.toLowerCase();
  if (!p.includes('*')) return p === a;
  const re = new RegExp(
    `^${p
      .split('*')
      .map((s) => s.replace(/[.+?^${}()|[\]\\]/g, '\\$&'))
      .join('.*')}$`,
  );
  return re.test(a);
}

function matchesText(pattern: string, text: string): boolean {
  const p = pattern.toLowerCase();
  return p.includes('*') ? matchesPattern(`*${p}*`, text) : text.toLowerCase().includes(p);
}

/** True when every term holds. `skipKey` ignores one facet's filters (for facet value counts). */
export function matchesTerms(get: FieldGetter, text: string, terms: SearchTerm[], skipKey?: string): boolean {
  for (const t of terms) {
    let hit: boolean;
    if (t.kind === 'text') {
      hit = matchesText(t.text, text);
    } else if (t.kind === 'any') {
      hit = t.alternatives.some((a) => matchesTerms(get, text, a, skipKey));
    } else {
      if (t.key === skipKey) continue;
      const v = get(t.key);
      const values = v === undefined ? [] : Array.isArray(v) ? v : [v];
      hit = values.some((actual) => t.values.some((p) => matchesPattern(p, actual)));
    }
    if (hit === t.negate) return false;
  }
  return true;
}

export function matchLog(log: LogEvent, terms: SearchTerm[], skipKey?: string): boolean {
  return matchesTerms((k) => logFieldValues(log, k), log.message, terms, skipKey);
}
