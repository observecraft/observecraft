// Search syntax shared by Logs, Error Tracking, … (Datadog-compatible subset):
//   service:order-service            tag / reserved attribute
//   service:(a OR b)              union
//   service:order*              wildcard
//   @http.status_code:5*          event attribute (dotted path)
//   -status:info                  negation
//   "bad response"  timeout       free text on the message
// Terms are ANDed. This module only parses/edits queries; evaluation is the backend's job
// (see api/mock/match.ts for the mock's evaluator).

export type Term =
  | { kind: 'tag'; key: string; values: string[]; negate: boolean }
  | { kind: 'text'; text: string; negate: boolean };

/** Splits on whitespace, keeping quoted strings and parenthesised groups intact. */
export function tokenize(q: string): string[] {
  const out: string[] = [];
  let cur = '';
  let depth = 0;
  let quoted = false;
  for (const ch of q) {
    if (ch === '"') quoted = !quoted;
    else if (!quoted && ch === '(') depth++;
    else if (!quoted && ch === ')') depth = Math.max(0, depth - 1);
    if (!quoted && depth === 0 && /\s/.test(ch)) {
      if (cur) out.push(cur);
      cur = '';
      continue;
    }
    cur += ch;
  }
  if (cur) out.push(cur);
  return out;
}

function unquote(s: string): string {
  return s.length >= 2 && s.startsWith('"') && s.endsWith('"') ? s.slice(1, -1) : s;
}

export function parseTerm(token: string): Term {
  let s = token;
  let negate = false;
  if (s.length > 1 && s.startsWith('-')) {
    negate = true;
    s = s.slice(1);
  }
  const m = /^(@?[\w.-]+):(.+)$/.exec(s);
  if (m) {
    const raw = m[2];
    const values =
      raw.startsWith('(') && raw.endsWith(')')
        ? raw
            .slice(1, -1)
            .split(/\s+OR\s+/i)
            .map((v) => unquote(v.trim()))
            .filter(Boolean)
        : [unquote(raw)];
    return { kind: 'tag', key: m[1], values, negate };
  }
  return { kind: 'text', text: unquote(s), negate };
}

export function parseQuery(q: string): Term[] {
  return tokenize(q)
    .filter((t) => t !== 'AND')
    .map(parseTerm);
}

function formatValue(v: string): string {
  return /[\s()"]/.test(v) ? `"${v}"` : v;
}

export function formatTerm(t: Term): string {
  const neg = t.negate ? '-' : '';
  if (t.kind === 'text') return neg + formatValue(t.text);
  const value = t.values.length > 1 ? `(${t.values.map(formatValue).join(' OR ')})` : formatValue(t.values[0]);
  return `${neg}${t.key}:${value}`;
}

export function formatQuery(terms: Term[]): string {
  return terms.map(formatTerm).join(' ');
}

/** Facet checkbox semantics: add/remove one value in the key's (non-negated) union term. */
export function toggleValue(terms: Term[], key: string, value: string): Term[] {
  const idx = terms.findIndex((t) => t.kind === 'tag' && t.key === key && !t.negate);
  if (idx === -1) return [...terms, { kind: 'tag', key, values: [value], negate: false }];
  const t = terms[idx] as Extract<Term, { kind: 'tag' }>;
  const values = t.values.includes(value) ? t.values.filter((v) => v !== value) : [...t.values, value];
  const next = [...terms];
  if (values.length === 0) next.splice(idx, 1);
  else next[idx] = { ...t, values };
  return next;
}

export function clearKey(terms: Term[], key: string): Term[] {
  return terms.filter((t) => !(t.kind === 'tag' && t.key === key));
}

/** Facet checkbox state derived from the query. */
export function selectedValues(terms: Term[]): Record<string, string[]> {
  const out: Record<string, string[]> = {};
  for (const t of terms) if (t.kind === 'tag' && !t.negate) out[t.key] = [...(out[t.key] ?? []), ...t.values];
  return out;
}

/** Appends `key:value` to a query string unless that exact filter is already present. */
export function addTerm(query: string, key: string, value: string): string {
  const terms = parseQuery(query);
  const exists = terms.some((t) => t.kind === 'tag' && t.key === key && !t.negate && t.values.length === 1 && t.values[0] === value);
  return exists ? query : formatQuery([...terms, { kind: 'tag', key, values: [value], negate: false }]);
}

// ---- "Exclude" facet mode (Error Tracking): every value starts checked; unchecking excludes it.

type TagTerm = Extract<Term, { kind: 'tag' }>;

function findTag(terms: Term[], key: string, negate: boolean): number {
  return terms.findIndex((t) => t.kind === 'tag' && t.key === key && t.negate === negate);
}

/** Checked unless excluded by `-key:…`, or not included by a positive `key:…` term. */
export function isValueChecked(terms: Term[], key: string, value: string): boolean {
  const pos = findTag(terms, key, false);
  if (pos !== -1) return (terms[pos] as TagTerm).values.includes(value);
  const neg = findTag(terms, key, true);
  return neg === -1 || !(terms[neg] as TagTerm).values.includes(value);
}

/** Checkbox click in exclude mode: edits the positive term if present, else the `-key:(…)` term. */
export function toggleChecked(terms: Term[], key: string, value: string): Term[] {
  if (findTag(terms, key, false) !== -1) return toggleValue(terms, key, value);
  const idx = findTag(terms, key, true);
  if (idx === -1) return [...terms, { kind: 'tag', key, values: [value], negate: true }];
  const t = terms[idx] as TagTerm;
  const values = t.values.includes(value) ? t.values.filter((v) => v !== value) : [...t.values, value];
  const next = [...terms];
  if (values.length === 0) next.splice(idx, 1);
  else next[idx] = { ...t, values };
  return next;
}

/** Label click: keep only this value. */
export function onlyValue(terms: Term[], key: string, value: string): Term[] {
  return [...clearKey(terms, key), { kind: 'tag', key, values: [value], negate: false }];
}
