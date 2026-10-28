// SQL Editor helpers without React: syntax highlighting, CSV export, tab names and the bits of a
// query the mock reads (its first table and LIMIT).

export type SqlTokenKind = 'keyword' | 'function' | 'string' | 'number' | 'comment' | 'operator' | 'text';
export interface SqlToken {
  kind: SqlTokenKind;
  text: string;
}

const KEYWORDS = new Set(
  (
    'select from where and or not as join inner left right full outer cross semi anti on using group by order having limit offset with union all distinct ' +
    'in is null like ilike between case when then else end asc desc nulls first last exists interval over partition rows range preceding following ' +
    'current row unbounded cast true false except intersect qualify prewhere minute minutes hour hours day days week weeks month months year years second seconds'
  ).split(' '),
);

const WORD = /[A-Za-z_][A-Za-z0-9_]*/y;
const NUMBER = /\d+(\.\d*)?([eE][+-]?\d+)?|\.\d+([eE][+-]?\d+)?/y;
const SPACE = /\s+/y;

function match(re: RegExp, s: string, at: number): string | null {
  re.lastIndex = at;
  const m = re.exec(s);
  return m ? m[0] : null;
}

/** Ends of a quoted run starting at `at` (doubled quotes stay inside); unterminated runs reach the end. */
function quoted(s: string, at: number, q: string): number {
  let i = at + 1;
  while (i < s.length) {
    if (s[i] === '\\' && q === "'") i += 2;
    else if (s[i] === q) {
      if (s[i + 1] === q) i += 2;
      else return i + 1;
    } else i += 1;
  }
  return s.length;
}

/** Splits SQL into tokens for display; the tokens' text joined is the input. */
export function highlightSql(sql: string): SqlToken[] {
  const out: SqlToken[] = [];
  const push = (kind: SqlTokenKind, text: string) => {
    const last = out[out.length - 1];
    if (last && last.kind === kind && (kind === 'text' || kind === 'operator')) last.text += text;
    else out.push({ kind, text });
  };
  let i = 0;
  while (i < sql.length) {
    const c = sql[i];
    if (c === '-' && sql[i + 1] === '-') {
      const end = sql.indexOf('\n', i);
      const stop = end === -1 ? sql.length : end;
      push('comment', sql.slice(i, stop));
      i = stop;
    } else if (c === '/' && sql[i + 1] === '*') {
      const end = sql.indexOf('*/', i + 2);
      const stop = end === -1 ? sql.length : end + 2;
      push('comment', sql.slice(i, stop));
      i = stop;
    } else if (c === "'") {
      const stop = quoted(sql, i, "'");
      push('string', sql.slice(i, stop));
      i = stop;
    } else if (c === '`' || c === '"') {
      const stop = quoted(sql, i, c);
      push('text', sql.slice(i, stop));
      i = stop;
    } else if (/\d/.test(c) || (c === '.' && /\d/.test(sql[i + 1] ?? ''))) {
      const n = match(NUMBER, sql, i) ?? c;
      push('number', n);
      i += n.length;
    } else if (/[A-Za-z_]/.test(c)) {
      const w = match(WORD, sql, i) ?? c;
      const rest = sql.slice(i + w.length);
      const kind: SqlTokenKind = KEYWORDS.has(w.toLowerCase()) ? 'keyword' : /^\s*\(/.test(rest) ? 'function' : 'text';
      push(kind, w);
      i += w.length;
    } else if (/\s/.test(c)) {
      const sp = match(SPACE, sql, i) ?? c;
      push('text', sp);
      i += sp.length;
    } else {
      push(/[=<>!+\-*/%|]/.test(c) ? 'operator' : 'text', c);
      i += 1;
    }
  }
  return out;
}

function csvCell(v: unknown): string {
  if (v === null || v === undefined) return '';
  let s = typeof v === 'object' ? JSON.stringify(v) : String(v);
  // Keep spreadsheets from evaluating a cell as a formula.
  if (/^[=+\-@\t\r]/.test(s) && typeof v === 'string') s = `'${s}`;
  return /[",\r\n]/.test(s) ? `"${s.replace(/"/g, '""')}"` : s;
}

/** RFC 4180 CSV of a result. */
export function toCsv(columns: string[], rows: unknown[][]): string {
  return [columns, ...rows].map((r) => r.map(csvCell).join(',')).join('\r\n') + '\r\n';
}

const MONTHS = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec'];

/** "Untitled, Oct 28, 14:05". */
export function untitledName(at: Date): string {
  const pad = (n: number) => String(n).padStart(2, '0');
  return `Untitled, ${MONTHS[at.getMonth()]} ${at.getDate()}, ${pad(at.getHours())}:${pad(at.getMinutes())}`;
}

/** The first of `tables` a query reads (FROM / JOIN), in lower case. */
export function firstTable(sql: string, tables: string[]): string | null {
  const known = new Set(tables);
  for (const m of sql.matchAll(/\b(?:from|join)\s+[`"]?([A-Za-z_][A-Za-z0-9_.]*)/gi)) {
    const name = m[1].toLowerCase();
    if (known.has(name)) return name;
  }
  return null;
}

/** The query's trailing LIMIT, if any. */
export function limitOf(sql: string): number | null {
  const m = /\blimit\s+(\d+)\s*;?\s*$/i.exec(sql.trim());
  return m ? Number(m[1]) : null;
}

/** The first lines of a query, for template cards. */
export function snippet(sql: string, lines = 4): string {
  const all = sql.split('\n');
  return all.length > lines ? `${all.slice(0, lines).join('\n')}\n…` : sql;
}

/** Display text of a result cell. */
export function cellText(v: unknown): string {
  if (v === null || v === undefined) return 'NULL';
  if (typeof v === 'object') return JSON.stringify(v);
  return String(v);
}
