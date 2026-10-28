// Mock-only log pipeline engine: a compact TypeScript version of the server's
// (server/crates/oc-api/src/pipelines) so `make web-mock` previews grok rules and processors.
// Same rule syntax and processor semantics; fewer edge cases (no atomic groups, ASCII-only `\w`).
import { parseQuery } from '../../lib/searchQuery';
import type { JsonValue } from '../types';
import type { GrokTestRequest, GrokTestResult, PipelineTestResult, Processor, SampleLog } from '../pipelines/types';
import { matchesTerms } from './match';

type Obj = Record<string, JsonValue>;

// ---------------------------------------------------------------- attributes

function lookup(m: Obj, path: string): JsonValue | undefined {
  if (path in m) return m[path];
  for (let i = path.indexOf('.'); i >= 0; i = path.indexOf('.', i + 1)) {
    const head = m[path.slice(0, i)];
    if (head && typeof head === 'object' && !Array.isArray(head)) {
      const found = lookup(head, path.slice(i + 1));
      if (found !== undefined) return found;
    }
  }
  return undefined;
}

const isObj = (v: JsonValue | undefined): v is Obj => !!v && typeof v === 'object' && !Array.isArray(v);

function objectPrefix(m: Obj, path: string): number {
  for (let i = path.indexOf('.'); i >= 0; i = path.indexOf('.', i + 1)) if (isObj(m[path.slice(0, i)])) return i;
  return -1;
}

function setIn(m: Obj, path: string, v: JsonValue) {
  if (path in m) {
    m[path] = v;
    return;
  }
  const i = objectPrefix(m, path);
  if (i >= 0) return setIn(m[path.slice(0, i)] as Obj, path.slice(i + 1), v);
  const dot = path.indexOf('.');
  if (dot < 0) {
    m[path] = v;
    return;
  }
  const head = path.slice(0, dot);
  if (!isObj(m[head])) m[head] = {};
  setIn(m[head] as Obj, path.slice(dot + 1), v);
}

function removeIn(m: Obj, path: string): JsonValue | undefined {
  if (path in m) {
    const v = m[path];
    delete m[path];
    return v;
  }
  const i = objectPrefix(m, path);
  return i >= 0 ? removeIn(m[path.slice(0, i)] as Obj, path.slice(i + 1)) : undefined;
}

export const text = (v: JsonValue | undefined): string => (v === undefined || v === null ? '' : typeof v === 'string' ? v : JSON.stringify(v));

function flatten(m: Obj, prefix = '', out: Record<string, string> = {}): Record<string, string> {
  for (const [k, v] of Object.entries(m)) {
    const key = prefix ? `${prefix}.${k}` : k;
    if (isObj(v) && Object.keys(v).length) flatten(v, key, out);
    else out[key] = text(v);
  }
  return out;
}

/** The log while processors run: the sample plus a changed date. */
class Log {
  s: SampleLog;
  constructor(s: SampleLog) {
    this.s = s;
  }
  get(path: string): JsonValue | undefined {
    if (path === 'message') return this.s.message;
    const v = lookup(this.s.attributes, path);
    if (v !== undefined) return v;
    if (path === 'service' || path === 'status' || path === 'host' || path === 'source') return this.s[path] || undefined;
    return undefined;
  }
  set(path: string, v: JsonValue) {
    if (path === 'message') this.s.message = text(v);
    else setIn(this.s.attributes, path, v);
  }
  remove(path: string) {
    if (path === 'message') this.s.message = '';
    else removeIn(this.s.attributes, path);
  }
}

// ---------------------------------------------------------------- filters

const CLAUSE = /(?:^|(?<=\s))(-?)(@?[\w.-]+):(?:([[{])\s*(\S+)\s+TO\s+(\S+?)\s*([\]}])|(>=|<=|>|<)([^\s)]+)|\s*"((?:[^"\\]|\\.)*)")(?=\s|$)/g;

type Clause = { key: string; negate: boolean; test: (v: string | undefined) => boolean };

/** A search filter (log search syntax plus ranges, comparisons and quoted literals) as a predicate. */
export function compileFilter(query: string): (log: Log) => boolean {
  const clauses: Clause[] = [];
  const rest = query.replace(CLAUSE, (_m, neg: string, key: string, open: string, lo: string, hi: string, close: string, op: string, n: string, lit: string | undefined) => {
    let test: Clause['test'];
    if (lit !== undefined) test = (v) => v !== undefined && v.toLowerCase() === lit.replace(/\\"/g, '"').toLowerCase();
    else {
      const num = (s: string) => (s === '*' ? NaN : Number(s));
      const [a, b, incA, incB] = open ? [num(lo), num(hi), open === '[', close === ']'] : op.startsWith('>') ? [Number(n), NaN, op === '>=', true] : [NaN, Number(n), true, op === '<='];
      test = (v) => {
        const x = v === undefined || v.trim() === '' ? NaN : Number(v);
        if (Number.isNaN(x)) return false;
        const above = Number.isNaN(a) || (incA ? x >= a : x > a);
        const below = Number.isNaN(b) || (incB ? x <= b : x < b);
        return above && below;
      };
    }
    clauses.push({ key, negate: neg === '-', test });
    return ' ';
  });
  const terms = parseQuery(rest).filter((t) => !(t.kind === 'text' && t.text === '*' && !t.negate));
  return (log) => {
    const flat = flatten(log.s.attributes);
    const field = (key: string): string | undefined => {
      if (key === 'service' || key === 'status' || key === 'host' || key === 'source') return log.s[key];
      if (key.startsWith('@')) return flat[key.slice(1)];
      return log.s.tags[key];
    };
    return matchesTerms(field, log.s.message, terms) && clauses.every((c) => c.test(field(c.key)) !== c.negate);
  };
}

// ---------------------------------------------------------------- dates

const MONTHS = ['jan', 'feb', 'mar', 'apr', 'may', 'jun', 'jul', 'aug', 'sep', 'oct', 'nov', 'dec'];
const ZONES: Record<string, number> = { UTC: 0, GMT: 0, Z: 0, EST: -300, EDT: -240, CST: -360, CDT: -300, MST: -420, MDT: -360, PST: -480, PDT: -420, CET: 60, CEST: 120 };

/** Minutes east of UTC. */
function zoneOffset(z: string): number | undefined {
  const m = /^(?:UTC|GMT)?([+-])(\d{1,2}):?(\d{2})?$/i.exec(z.trim());
  if (m) return (m[1] === '-' ? -1 : 1) * (Number(m[2]) * 60 + Number(m[3] ?? 0));
  return ZONES[z.trim().toUpperCase()];
}

interface DatePattern {
  fragment: string;
  parse: (s: string, nowMs: number) => number | undefined;
}

function compileDate(pattern: string, tz?: string): DatePattern {
  const defaultOffset = tz === undefined ? 0 : zoneOffset(tz);
  if (defaultOffset === undefined) throw new Error(`Unsupported time zone '${tz}'`);
  let fragment = '';
  let named = '';
  const seen = new Set<string>();
  const esc = (s: string) => s.replace(/[.*+?^${}()|[\]\\/]/g, '\\$&');
  for (let i = 0; i < pattern.length;) {
    const c = pattern[i];
    if (c === "'") {
      const end = pattern.indexOf("'", i + 1);
      const lit = end < 0 ? pattern.slice(i + 1) : pattern.slice(i + 1, end);
      fragment += esc(lit || "'");
      named += esc(lit || "'");
      i = end < 0 ? pattern.length : end + 1;
      continue;
    }
    if (!/[A-Za-z]/.test(c)) {
      fragment += esc(c);
      named += esc(c);
      i++;
      continue;
    }
    let n = 1;
    while (pattern[i + n] === c) n++;
    i += n;
    const d = (min: number, max: number) => (n >= max ? `\\d{${max}}` : `\\d{${min},${max}}`);
    const fields: Record<string, [string, string]> = {
      y: n === 2 ? ['yy', '\\d{2}'] : ['y', n >= 4 ? '\\d{4}' : '\\d{1,4}'],
      M: n >= 4 ? ['MMM', '[A-Za-z]+'] : n === 3 ? ['MMM', '[A-Za-z]{3}'] : ['M', d(1, 2)],
      d: ['d', d(1, 2)],
      H: ['H', d(1, 2)],
      h: ['h', d(1, 2)],
      m: ['m', d(1, 2)],
      s: ['s', d(1, 2)],
      S: ['S', `\\d{${n}}`],
      a: ['a', '[AaPp][Mm]'],
      E: ['E', '[A-Za-z]+'],
      Z: ['Z', 'Z|[+-]\\d{2}(?::?\\d{2})?'],
      X: ['Z', 'Z|[+-]\\d{2}(?::?\\d{2})?'],
      z: ['z', '[A-Za-z]{1,5}(?:[+-]\\d{1,2}(?::?\\d{2})?)?|[+-]\\d{2}(?::?\\d{2})?'],
    };
    const f = fields[c];
    if (!f) throw new Error(`Unsupported date pattern letter '${c}' in "${pattern}"`);
    fragment += `(?:${f[1]})`;
    named += seen.has(f[0]) ? `(?:${f[1]})` : `(?<${f[0]}>${f[1]})`;
    seen.add(f[0]);
  }
  const re = new RegExp(`^${named}$`);
  return {
    fragment,
    parse(s, nowMs) {
      const g = re.exec(s)?.groups;
      if (!g) return undefined;
      const num = (k: string) => (g[k] === undefined ? undefined : Number(g[k]));
      const month = g.MMM ? MONTHS.indexOf(g.MMM.slice(0, 3).toLowerCase()) + 1 : (num('M') ?? 1);
      if (month < 1) return undefined;
      let hour = num('H') ?? 0;
      if (g.h !== undefined) hour = (Number(g.h) % 12) + (g.a?.toLowerCase() === 'pm' ? 12 : 0);
      const ms = g.S ? Number((g.S + '000').slice(0, 3)) : 0;
      const zone = g.Z ?? g.z;
      const offset = zone === undefined ? defaultOffset : zoneOffset(zone);
      if (offset === undefined) return undefined;
      const at = (year: number) => Date.UTC(year, month - 1, num('d') ?? 1, hour, num('m') ?? 0, num('s') ?? 0, ms) - offset * 60_000;
      if (g.y) return at(Number(g.y));
      if (g.yy) return at(2000 + Number(g.yy));
      const year = new Date(nowMs).getUTCFullYear();
      const t = at(year);
      return t > nowMs + 86_400_000 ? at(year - 1) : t;
    },
  };
}

/** Epoch ms of a date attribute: epoch numbers (s / ms / µs / ns), ISO 8601, RFC 2822, Apache's format. */
export function parseAnyDate(v: JsonValue | undefined, nowMs: number): number | undefined {
  if (typeof v === 'number' || (typeof v === 'string' && /^\d+(\.\d+)?$/.test(v.trim()))) {
    const n = Number(v);
    const a = Math.abs(n);
    return Math.round(a > 1e17 ? n / 1e6 : a > 1e14 ? n / 1e3 : a > 1e11 ? n : n * 1e3);
  }
  if (typeof v !== 'string' || !v.trim()) return undefined;
  const s = v.trim();
  for (const p of ['dd/MMM/yyyy:HH:mm:ss Z', 'yyyy-MM-dd HH:mm:ss.SSS z', 'yyyy-MM-dd HH:mm:ss z', 'yyyy-MM-dd HH:mm:ss.SSS', 'yyyy-MM-dd HH:mm:ss', 'MMM d HH:mm:ss']) {
    const t = compileDate(p).parse(s, nowMs);
    if (t !== undefined) return t;
  }
  // ISO 8601 without a zone is UTC here, as on the server.
  const iso = /^\d{4}-\d{2}-\d{2}T[\d:.]+$/.test(s) ? `${s}Z` : s;
  const t = Date.parse(iso);
  return Number.isNaN(t) ? undefined : t;
}

// ---------------------------------------------------------------- grok

const INT = '[+-]?\\d+';
const NUM = '[+-]?(?:\\d+(?:\\.\\d*)?|\\.\\d+)';
const NUM_EXT = `${NUM}(?:[eE][+-]?\\d+)?`;
const IPV4 = '(?:(?:25[0-5]|2[0-4]\\d|1\\d\\d|[1-9]?\\d)\\.){3}(?:25[0-5]|2[0-4]\\d|1\\d\\d|[1-9]?\\d)';
const IPV6 = '(?:[0-9A-Fa-f]{0,4}:){2,7}[0-9A-Fa-f]{0,4}';
const HOSTNAME = '[A-Za-z0-9][A-Za-z0-9_-]{0,62}(?:\\.[A-Za-z0-9_-]{1,63})*\\.?';
const DQ = '"(?:[^"\\\\]|\\\\.)*"';
const SQ = "'(?:[^'\\\\]|\\\\.)*'";

type Kind = 'str' | 'int' | 'num' | 'unquote' | { bool: string } | { date: DatePattern };
type Filter = (v: JsonValue) => JsonValue | undefined;
interface Capture {
  group: string;
  path: string;
  kind: Kind;
  filter?: Filter;
}
interface Rule {
  name: string;
  re: RegExp;
  captures: Capture[];
}

const reEscape = (s: string) => s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
/** Case-insensitive word without regex modifiers (not every engine has `(?i:…)`). */
const anyCase = (w: string) => [...w].map((c) => (/[a-z]/i.test(c) ? `[${c.toLowerCase()}${c.toUpperCase()}]` : reEscape(c))).join('');

function matcher(name: string, args: string[]): [string, Kind] {
  switch (name) {
    case 'word':
      return ['\\b\\w+\\b', 'str'];
    case 'notSpace':
      return ['\\S+', 'str'];
    case 'data':
      return ['[\\s\\S]*?', 'str'];
    case 'integer':
    case 'integerExt':
      return [INT, 'int'];
    case 'integerStr':
      return [INT, 'str'];
    case 'number':
      return [NUM, 'num'];
    case 'numberExt':
      return [NUM_EXT, 'num'];
    case 'numberStr':
    case 'numberExtStr':
      return [NUM_EXT, 'str'];
    case 'boolean': {
      const [yes, no] = args.length === 2 ? args : ['true', 'false'];
      return [`${anyCase(yes)}|${anyCase(no)}`, { bool: yes }];
    }
    case 'ipv4':
      return [IPV4, 'str'];
    case 'ipv6':
      return [IPV6, 'str'];
    case 'ip':
      return [`${IPV4}|${IPV6}`, 'str'];
    case 'hostname':
      return [HOSTNAME, 'str'];
    case 'ipOrHost':
      return [`${IPV4}|${IPV6}|${HOSTNAME}`, 'str'];
    case 'port':
      return ['\\d{1,5}', 'str'];
    case 'quotedString':
      return [`${DQ}|${SQ}`, 'unquote'];
    case 'doubleQuotedString':
      return [DQ, 'unquote'];
    case 'singleQuotedString':
      return [SQ, 'unquote'];
    case 'uuid':
      return ['[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}', 'str'];
    case 'mac':
      return ['(?:[0-9A-Fa-f]{2}[:-]){5}[0-9A-Fa-f]{2}', 'str'];
    case 'regex':
      if (args.length !== 1) throw new Error('regex takes one argument: regex("pattern")');
      return [args[0], 'str'];
    case 'date': {
      if (args.length < 1 || args.length > 2) throw new Error('date takes a pattern and an optional time zone');
      const d = compileDate(args[0], args[1]);
      return [d.fragment, { date: d }];
    }
  }
  throw new Error(`Unknown matcher '${name}'`);
}

const num = (v: JsonValue): number | undefined => (typeof v === 'number' ? v : typeof v === 'string' && v.trim() !== '' && !Number.isNaN(Number(v)) ? Number(v) : undefined);

function compileFilterFn(name: string, args: string[]): Filter {
  switch (name) {
    case 'number':
      return (v) => num(v);
    case 'integer':
      return (v) => (num(v) === undefined ? undefined : Math.trunc(num(v)!));
    case 'boolean':
      return (v) => (typeof v === 'boolean' ? v : /^true$/i.test(text(v)) ? true : /^false$/i.test(text(v)) ? false : undefined);
    case 'nullIf':
      return (v) => (text(v) === args[0] ? undefined : v);
    case 'lowercase':
      return (v) => (typeof v === 'string' ? v.toLowerCase() : v);
    case 'uppercase':
      return (v) => (typeof v === 'string' ? v.toUpperCase() : v);
    case 'json':
      return (v) => {
        try {
          return JSON.parse(text(v)) as JsonValue;
        } catch {
          return undefined;
        }
      };
    case 'scale':
      return (v) => (num(v) === undefined ? undefined : num(v)! * Number(args[0]));
    case 'array': {
      const [brackets, sep] = args.length === 2 ? args : ['', args[0] ?? ','];
      return (v) => {
        let s = text(v).trim();
        if (brackets && s.startsWith(brackets[0])) s = s.slice(1);
        if (brackets && s.endsWith(brackets[1])) s = s.slice(0, -1);
        return s
          .split(sep)
          .map((x) => x.trim())
          .filter(Boolean);
      };
    }
    case 'keyvalue': {
      const sep = args[0] ?? '=';
      const extra = (args[1] ?? '').replace(/[\\\]^-]/g, '\\$&');
      const pairs = new RegExp(`([\\w.\\-@]+)${reEscape(sep)}(?:"([^"]*)"|'([^']*)'|<([^>]*)>|([\\w.\\-@${extra}]+))`, 'g');
      return (v) => {
        const out: Obj = {};
        for (const m of text(v).matchAll(pairs)) {
          const value = m[2] ?? m[3] ?? m[4] ?? m[5]?.replace(/[,;]+$/, '');
          if (value) out[m[1]] = value;
        }
        return out;
      };
    }
    case 'decodeuricomponent':
      return (v) => {
        try {
          return decodeURIComponent(text(v));
        } catch {
          return text(v);
        }
      };
    case 'querystring':
      return (v) => Object.fromEntries(new URLSearchParams(text(v).replace(/^\?/, '')));
  }
  throw new Error(`Unknown filter '${name}'`);
}

/** `%{matcher(args):attribute:filter(args)}` → its parts. */
function parseSpec(spec: string): { matcher: string; args: string[]; attr: string; filter?: [string, string[]] } {
  let i = 0;
  const until = (stops: string) => {
    const start = i;
    while (i < spec.length && !stops.includes(spec[i])) i++;
    return spec.slice(start, i);
  };
  const args = (): string[] => {
    const out: string[] = [];
    if (spec[i] !== '(') return out;
    i++;
    for (;;) {
      while (spec[i] === ' ') i++;
      if (spec[i] === ')') {
        i++;
        return out;
      }
      if (spec[i] === '"' || spec[i] === "'") {
        const q = spec[i++];
        let s = '';
        while (i < spec.length && spec[i] !== q) {
          if (spec[i] === '\\' && (spec[i + 1] === '\\' || spec[i + 1] === q)) i++;
          s += spec[i++];
        }
        i++;
        out.push(s);
      } else out.push(until(',)').trim());
      while (spec[i] === ' ') i++;
      if (spec[i] === ',') i++;
      else if (spec[i] !== ')') throw new Error(`Bad arguments in %{${spec}}`);
    }
  };
  if (spec.startsWith('?>')) i = 2;
  const matcherName = until('(:').trim();
  const margs = args();
  let attr = '';
  let filter: [string, string[]] | undefined;
  if (spec[i] === ':') {
    i++;
    attr = until(':').trim();
    if (spec[i] === ':') {
      i++;
      const f = until('(').trim();
      const fargs = args();
      if (f) filter = [f, fargs];
    }
  }
  return { matcher: matcherName, args: margs, attr, filter };
}

function closingBrace(s: string): number {
  let quote = '';
  for (let i = 0; i < s.length; i++) {
    const c = s[i];
    if (quote) {
      if (c === '\\') i++;
      else if (c === quote) quote = '';
    } else if (c === '"' || c === "'") quote = c;
    else if (c === '}') return i;
  }
  return -1;
}

function ruleLines(textRules: string): [string, string][] {
  return textRules
    .split('\n')
    .map((l) => l.trim())
    .filter((l) => l && !l.startsWith('#'))
    .map((l) => {
      const m = /^(\S+)\s+(.+)$/.exec(l);
      if (!m) throw new Error(`Rule '${l}' has no pattern (write one rule per line: name pattern)`);
      return [m[1], m[2].trim()];
    });
}

export function compileGrok(supportRules: string, matchRules: string): Rule[] {
  const support = ruleLines(supportRules);
  const matching = ruleLines(matchRules);
  if (!matching.length) throw new Error('Add at least one match rule');
  const defs = new Map([...support, ...matching]);
  return matching.map(([name, pattern]) => {
    const captures: Capture[] = [];
    const expand = (p: string, depth: number): string => {
      if (depth > 16) throw new Error(`Rule '${name}': rules include each other in a loop`);
      let out = '';
      let rest = p;
      for (let i = rest.indexOf('%{'); i >= 0; i = rest.indexOf('%{')) {
        out += rest.slice(0, i);
        const after = rest.slice(i + 2);
        const end = closingBrace(after);
        if (end < 0) throw new Error(`Rule '${name}': a %{ is never closed`);
        const spec = parseSpec(after.slice(0, end));
        const def = defs.get(spec.matcher);
        const [re, kind]: [string, Kind] = def !== undefined && !spec.args.length ? [expand(def, depth + 1), 'str'] : matcher(spec.matcher, spec.args);
        if (!spec.attr && !spec.filter) out += `(?:${re})`;
        else {
          const group = `oc${captures.length}`;
          captures.push({ group, path: spec.attr, kind, filter: spec.filter && compileFilterFn(spec.filter[0], spec.filter[1]) });
          out += `(?<${group}>${re})`;
        }
        rest = after.slice(end + 1);
      }
      // JavaScript has no atomic groups: they match as plain groups here.
      return (out + rest).replace(/\(\?>/g, '(?:');
    };
    let re: RegExp;
    try {
      re = new RegExp(`^(?:${expand(pattern, 0)})$`);
    } catch (e) {
      if (e instanceof SyntaxError) throw new Error(`Rule '${name}': invalid pattern (${e.message})`);
      throw new Error(`Rule '${name}': ${(e as Error).message}`);
    }
    return { name, re, captures };
  });
}

function convert(kind: Kind, s: string, nowMs: number): JsonValue | undefined {
  if (kind === 'str') return s;
  if (kind === 'int') return Number.isNaN(Number(s)) ? undefined : Math.trunc(Number(s));
  if (kind === 'num') return Number.isNaN(Number(s)) ? undefined : Number(s);
  if (kind === 'unquote') return s.slice(1, -1).replace(/\\(.)/g, '$1');
  if ('bool' in kind) return s.toLowerCase() === kind.bool.toLowerCase();
  return kind.date.parse(s, nowMs);
}

/** The first rule matching the whole text, and its `[path, value]` pairs (repeats become arrays). */
export function grokParse(rules: Rule[], input: string, nowMs: number): { rule: string; values: [string, JsonValue][] } | null {
  for (const rule of rules) {
    const m = rule.re.exec(input);
    if (!m) continue;
    const values: [string, JsonValue][] = [];
    const repeated = new Set<number>();
    for (const c of rule.captures) {
      const raw = m.groups?.[c.group];
      if (raw === undefined) continue;
      let v = convert(c.kind, raw, nowMs);
      if (v !== undefined && c.filter) v = c.filter(v);
      if (v === undefined) continue;
      const i = values.findIndex(([p]) => p === c.path);
      if (i < 0) values.push([c.path, v]);
      else if (repeated.has(i)) (values[i][1] as JsonValue[]).push(v);
      else {
        values[i][1] = [values[i][1], v];
        repeated.add(i);
      }
    }
    return { rule: rule.name, values };
  }
  return null;
}

function place(log: Log, values: [string, JsonValue][]) {
  for (const [path, v] of values) {
    if (path) log.set(path, v);
    else if (isObj(v)) for (const [k, x] of Object.entries(v)) log.set(k, x);
  }
}

/** The grok editor's preview: each sample's matching rule and extracted (nested) attributes. */
export function parseGrok(req: GrokTestRequest, nowMs = Date.now()): GrokTestResult {
  const none = () => ({ matched: false, rule: null, attributes: {} });
  let rules: Rule[];
  try {
    rules = compileGrok(req.supportRules ?? '', req.matchRules ?? '');
  } catch (e) {
    return { error: (e as Error).message, samples: (req.samples ?? []).map(none) };
  }
  return {
    error: null,
    samples: (req.samples ?? []).map((s) => {
      const parsed = grokParse(rules, s, nowMs);
      if (!parsed) return none();
      const log = new Log({ message: '', status: '', service: '', host: '', source: '', tags: {}, attributes: {}, date: null, traceId: '' });
      place(log, parsed.values);
      return { matched: true, rule: parsed.rule, attributes: log.s.message ? { ...log.s.attributes, message: log.s.message } : log.s.attributes };
    }),
  };
}

// ---------------------------------------------------------------- processors

/** Datadog's status words and syslog numbers as one of ours. */
export function statusOf(v: JsonValue | undefined): string {
  const s = text(v).trim().toLowerCase();
  if (s !== '' && !Number.isNaN(Number(s))) {
    const n = Math.trunc(Number(s));
    return n >= 0 && n <= 3 ? 'error' : n === 4 ? 'warn' : n === 7 ? 'debug' : 'info';
  }
  if (['emerg', 'f', 'a', 'c', 'err', 'panic'].some((p) => s.startsWith(p)) || s === 'e') return 'error';
  if (s.startsWith('w')) return 'warn';
  if (s.startsWith('d') || s.startsWith('trace') || s.startsWith('verbose')) return 'debug';
  return 'info';
}

function urlDetails(s: string): Obj | undefined {
  let u: URL;
  let absolute = true;
  try {
    u = new URL(s);
  } catch {
    if (!s.startsWith('/')) return undefined;
    u = new URL(s, 'http://relative.invalid');
    absolute = false;
  }
  const out: Obj = absolute
    ? {
        scheme: u.protocol.replace(/:$/, ''),
        host: u.hostname,
        ...(u.port || u.protocol === 'http:' || u.protocol === 'https:' ? { port: Number(u.port || (u.protocol === 'https:' ? 443 : 80)) } : {}),
      }
    : {};
  out.path = u.pathname;
  if (u.search.length > 1) out.queryString = Object.fromEntries(u.searchParams);
  return out;
}

/** `a * (b - 2)` over attributes; undefined when an attribute is missing (unless replaced by 0). */
function arithmetic(expr: string): (get: (a: string) => number | undefined) => number | undefined {
  const tokens = expr.match(/\d+(?:\.\d+)?|\.\d+|[A-Za-z_@](?:[\w.@]|\\-)*|[-+*/()]|\S/g) ?? [];
  let pos = 0;
  type Node = (get: (a: string) => number | undefined) => number | undefined;
  const bin =
    (l: Node, op: string, r: Node): Node =>
    (g) => {
      const a = l(g);
      const b = r(g);
      if (a === undefined || b === undefined) return undefined;
      return op === '+' ? a + b : op === '-' ? a - b : op === '*' ? a * b : b === 0 ? undefined : a / b;
    };
  const factor = (): Node => {
    const t = tokens[pos++];
    if (t === undefined) throw new Error('The expression ends too early');
    if (t === '-') {
      const f = factor();
      return (g) => (f(g) === undefined ? undefined : -f(g)!);
    }
    if (t === '(') {
      const e = sum();
      if (tokens[pos++] !== ')') throw new Error("Missing ')' in the expression");
      return e;
    }
    if (/^[\d.]/.test(t)) return () => Number(t);
    if (/^[A-Za-z_@]/.test(t)) {
      const name = t.replace(/^@/, '').replace(/\\-/g, '-');
      return (g) => g(name);
    }
    throw new Error(`Unexpected '${t}' in the expression`);
  };
  const product = (): Node => {
    let l = factor();
    while (tokens[pos] === '*' || tokens[pos] === '/') l = bin(l, tokens[pos++], factor());
    return l;
  };
  const sum = (): Node => {
    let l = product();
    while (tokens[pos] === '+' || tokens[pos] === '-') l = bin(l, tokens[pos++], product());
    return l;
  };
  const root = sum();
  if (pos !== tokens.length) throw new Error(`Unexpected '${tokens[pos]}' in the expression`);
  return root;
}

type Step = (log: Log, nowMs: number) => void;

function lookupRow(line: string): [string, string] | undefined {
  const m = /^\s*(?:"([^"]*)"|([^,]*?))\s*,\s*(.*?)\s*$/.exec(line);
  if (!m) return undefined;
  return [m[1] ?? m[2], m[3].replace(/^"(.*)"$/, '$1')];
}

function compileStep(p: Processor, depth: number): Step | null {
  if (p.enabled === false) return null;
  const first = (log: Log, sources: string[]) => sources.map((s) => log.get(s)).find((v) => v !== undefined);
  switch (p.type) {
    case 'grok-parser': {
      const rules = compileGrok(p.grok?.supportRules ?? '', p.grok?.matchRules ?? '');
      const source = p.source || 'message';
      return (log, nowMs) => {
        const v = log.get(source);
        if (typeof v !== 'string') return;
        const parsed = grokParse(rules, v, nowMs);
        if (parsed) place(log, parsed.values);
      };
    }
    case 'status-remapper':
      return (log) => {
        const v = first(log, p.sources);
        if (v !== undefined) log.s.status = statusOf(v);
      };
    case 'date-remapper':
      return (log, nowMs) => {
        const t = p.sources.map((s) => parseAnyDate(log.get(s), nowMs)).find((x) => x !== undefined);
        if (t !== undefined) log.s.date = new Date(t).toISOString();
      };
    case 'service-remapper':
      return (log) => {
        const v = p.sources.map((s) => text(log.get(s))).find(Boolean);
        if (v) log.s.service = v;
      };
    case 'message-remapper':
      return (log) => {
        const v = first(log, p.sources);
        if (v !== undefined) log.s.message = text(v);
      };
    case 'trace-id-remapper':
      return (log) => {
        const v = text(first(log, p.sources)).trim().toLowerCase();
        if (/^\d+$/.test(v)) log.s.traceId = BigInt(v).toString(16).padStart(32, '0');
        else if (/^[0-9a-f]{1,32}$/.test(v)) log.s.traceId = v.padStart(32, '0');
      };
    case 'attribute-remapper':
      if (!p.target?.trim()) throw new Error('The remapper needs a target');
      return (log) => {
        const fromTag = p.sourceType === 'tag';
        const read = (s: string) => (fromTag ? log.s.tags[s] : log.get(s));
        const source = p.sources.find((s) => read(s) !== undefined);
        if (source === undefined) return;
        let v = read(source)!;
        const n = num(v);
        if (p.targetFormat === 'string') v = text(v);
        else if (p.targetFormat === 'integer' || p.targetFormat === 'double') {
          if (n === undefined) return;
          v = p.targetFormat === 'integer' ? Math.trunc(n) : n;
        }
        const toTag = p.targetType === 'tag';
        const exists = toTag ? p.target in log.s.tags : log.get(p.target) !== undefined;
        if (exists && !p.overrideOnConflict && source !== p.target) return;
        if (!p.preserveSource && source !== p.target) {
          if (fromTag) delete log.s.tags[source];
          else log.remove(source);
        }
        if (toTag) log.s.tags[p.target] = text(v);
        else log.set(p.target, v);
      };
    case 'category-processor': {
      const rules = (p.categories ?? []).map((c) => [compileFilter(c.filter?.query ?? ''), c.name] as const);
      return (log) => {
        const hit = rules.find(([f]) => f(log));
        if (hit) log.set(p.target, hit[1]);
      };
    }
    case 'string-builder-processor': {
      const parts = p.template.split(/(%\{[^}]*\})/).filter(Boolean);
      return (log) => {
        let out = '';
        for (const part of parts) {
          const m = /^%\{(.*)\}$/.exec(part);
          if (!m) out += part;
          else {
            const v = log.get(m[1].trim());
            if (v === undefined && !p.replaceMissing) return;
            out += text(v);
          }
        }
        log.set(p.target, out);
      };
    }
    case 'lookup-processor': {
      const table = new Map((p.lookupTable ?? []).map(lookupRow).filter((r): r is [string, string] => !!r));
      return (log) => {
        const v = log.get(p.source);
        if (v === undefined) return;
        const found = table.get(text(v)) ?? (p.defaultLookup || undefined);
        if (found !== undefined) log.set(p.target, found);
      };
    }
    case 'arithmetic-processor': {
      const expr = arithmetic(p.expression);
      return (log) => {
        const v = expr((a) => {
          const raw = log.get(a);
          return (raw === undefined ? undefined : num(raw)) ?? (p.replaceMissing ? 0 : undefined);
        });
        if (v !== undefined && Number.isFinite(v)) log.set(p.target, v);
      };
    }
    case 'url-parser':
      return (log) => {
        const v = (p.sources?.length ? p.sources : ['http.url']).map((s) => log.get(s)).find((x) => x !== undefined);
        const details = typeof v === 'string' ? urlDetails(v) : undefined;
        if (details) log.set(p.target || 'http.url_details', details);
      };
    case 'pipeline': {
      if (depth >= 1) throw new Error('Pipelines nest one level deep');
      const nested = compilePipeline(p.filter?.query ?? '', p.processors ?? [], depth + 1);
      if (nested.errors.length) throw new Error(`in nested pipeline '${p.name ?? ''}': ${nested.errors.join('; ')}`);
      return (log, nowMs) => void nested.apply(log, nowMs);
    }
  }
  return null;
}

function compilePipeline(filter: string, processors: Processor[], depth = 0) {
  const steps: Step[] = [];
  const errors: string[] = [];
  processors.forEach((p, i) => {
    try {
      const step = compileStep(p, depth);
      if (step) steps.push(step);
    } catch (e) {
      errors.push(`Processor ${i + 1} (${p.name || p.type}): ${(e as Error).message}`);
    }
  });
  const matches = compileFilter(filter);
  return {
    errors,
    apply(log: Log, nowMs: number): boolean {
      if (!matches(log)) return false;
      for (const s of steps) s(log, nowMs);
      return true;
    },
  };
}

/** Compile errors of a processor list (what the server rejects on save). */
export function processorErrors(filter: string, processors: Processor[]): string[] {
  return compilePipeline(filter, processors).errors;
}

/** Runs processors on a sample log, as the server's test endpoint does. */
export function runPipeline(filter: string, processors: Processor[], sample: SampleLog, nowMs = Date.now()): PipelineTestResult {
  const { errors, apply } = compilePipeline(filter, processors);
  const log = new Log(structuredClone(sample));
  const matched = apply(log, nowMs);
  return { matched, log: log.s, errors };
}
