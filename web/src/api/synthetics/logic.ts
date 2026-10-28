// Synthetic tests: labels, defaults, assertion checks, uptime and editor snippets shared by the pages
// and the in-browser mock. Checks and uptime follow the server (routes/synthetics*.rs).
import type { components } from '../generated/schema';
import type {
  SyntheticAssertion,
  SyntheticAssertionResult,
  SyntheticAssertionType,
  SyntheticDnsRecord,
  SyntheticHeader,
  SyntheticOperator,
  SyntheticRequest,
  SyntheticRunDetail,
  SyntheticTest,
  SyntheticTestType,
  SyntheticUptimeSegment,
} from './types';

import { neutralized } from './model';

type MonitorStatus = components['schemas']['MonitorStatus'];

export const LOCATION_ID = 'observecraft-server';

export const TYPE_LABEL: Record<SyntheticTestType, string> = { http: 'HTTP', ssl: 'SSL', dns: 'DNS', tcp: 'TCP' };
export const TEST_TYPES: SyntheticTestType[] = ['http', 'ssl', 'dns', 'tcp'];
/** Request types shown in the editor that can't be chosen yet. */
export const SOON_TYPES = ['gRPC', 'WebSocket', 'UDP', 'ICMP'];
export const METHODS = ['GET', 'POST', 'PUT', 'PATCH', 'DELETE', 'HEAD', 'OPTIONS'];
export const RECORD_TYPES = ['A', 'AAAA', 'CNAME', 'MX', 'TXT', 'NS'];

export const FREQUENCIES: { seconds: number; label: string }[] = [
  { seconds: 60, label: '1m' },
  { seconds: 300, label: '5m' },
  { seconds: 900, label: '15m' },
  { seconds: 1800, label: '30m' },
  { seconds: 3600, label: '1h' },
  { seconds: 21_600, label: '6h' },
  { seconds: 43_200, label: '12h' },
  { seconds: 86_400, label: '1d' },
  { seconds: 604_800, label: '1w' },
];

export function fmtFrequency(seconds: number): string {
  const f = FREQUENCIES.find((x) => x.seconds === seconds);
  return f ? `every ${f.label}` : `every ${Math.round(seconds / 60)}m`;
}

export const ASSERTION_TYPES: Record<SyntheticTestType, SyntheticAssertionType[]> = {
  http: ['statusCode', 'responseTime', 'header', 'body', 'bodyJsonPath'],
  ssl: ['certificate', 'responseTime'],
  dns: ['recordEvery', 'recordSome', 'responseTime'],
  tcp: ['responseTime'],
};

const TEXT_OPS: SyntheticOperator[] = ['is', 'isNot', 'contains', 'doesNotContain', 'matches', 'doesNotMatch'];
export const OPERATORS: Record<SyntheticAssertionType, SyntheticOperator[]> = {
  statusCode: ['is', 'isNot', 'lessThan', 'moreThan', 'matches', 'doesNotMatch'],
  responseTime: ['lessThan', 'moreThan'],
  header: [...TEXT_OPS, 'lessThan', 'moreThan'],
  bodyJsonPath: [...TEXT_OPS, 'lessThan', 'moreThan'],
  body: TEXT_OPS,
  recordEvery: TEXT_OPS,
  recordSome: TEXT_OPS,
  certificate: ['isInMoreThan'],
};

export const ASSERTION_LABEL: Record<SyntheticAssertionType, string> = {
  statusCode: 'status code',
  responseTime: 'response time',
  header: 'header',
  body: 'body',
  bodyJsonPath: 'body (JSON path)',
  certificate: 'certificate',
  recordEvery: 'every record',
  recordSome: 'at least one record',
};

export const OPERATOR_LABEL: Record<SyntheticOperator, string> = {
  is: 'is',
  isNot: 'is not',
  lessThan: 'is less than',
  moreThan: 'is more than',
  contains: 'contains',
  doesNotContain: 'does not contain',
  matches: 'matches',
  doesNotMatch: 'does not match',
  isInMoreThan: 'expires in more than',
};

/** Header assertions name a header, JSON paths a path and DNS assertions a record type. */
export function propertyKind(kind: SyntheticAssertionType): 'header' | 'path' | 'record' | null {
  if (kind === 'header') return 'header';
  if (kind === 'bodyJsonPath') return 'path';
  if (kind === 'recordEvery' || kind === 'recordSome') return 'record';
  return null;
}

export function assertionUnit(kind: SyntheticAssertionType): string {
  return kind === 'responseTime' ? 'ms' : kind === 'certificate' ? 'days' : '';
}

/** "status code is 200", "header content-type contains json", "certificate expires in more than 30 days". */
export function assertionText(a: SyntheticAssertion): string {
  const subject =
    a.type === 'header'
      ? `header ${a.property}`
      : a.type === 'bodyJsonPath'
        ? `body ${a.property}`
        : a.type === 'recordEvery'
          ? `every ${a.property} record`
          : a.type === 'recordSome'
            ? `at least one ${a.property} record`
            : ASSERTION_LABEL[a.type];
  const unit = assertionUnit(a.type);
  return `${subject} ${OPERATOR_LABEL[a.operator]} ${a.target}${unit ? ` ${unit}` : ''}`;
}

/** A new assertion of a type, with its first operator and a sensible target. */
export function newAssertion(kind: SyntheticAssertionType): SyntheticAssertion {
  const target: Partial<Record<SyntheticAssertionType, string>> = { statusCode: '200', responseTime: '1000', certificate: '30' };
  const property: Partial<Record<SyntheticAssertionType, string>> = { header: 'content-type', bodyJsonPath: '$.status', recordEvery: 'A', recordSome: 'A' };
  return { type: kind, operator: OPERATORS[kind][0], property: property[kind] ?? '', target: target[kind] ?? '' };
}

export function defaultRequest(type: SyntheticTestType): SyntheticRequest {
  return {
    method: 'GET',
    url: type === 'http' ? 'https://' : '',
    host: '',
    port: type === 'ssl' ? 443 : null,
    headers: [],
    body: '',
    bodyType: '',
    basicAuth: null,
    timeoutSeconds: 30,
    followRedirects: true,
    acceptSelfSigned: false,
    dnsServer: '',
    dnsServerPort: null,
  };
}

export function testDomain(type: SyntheticTestType, r: SyntheticRequest): string {
  if (type !== 'http') return r.host;
  // A URL with variables (`https://{{ SHOP_HOST }}/health`): its host part as written.
  if (/\{\{/.test(r.url)) return /^[a-z][a-z0-9+.-]*:\/\/([^/?#]*)/i.exec(r.url)?.[1] ?? '';
  try {
    return new URL(r.url).hostname;
  } catch {
    return '';
  }
}

// ---------------------------------------------------------------- assertions (mock runs)

export interface Observed {
  statusCode?: number | null;
  responseTimeMs: number;
  headers?: SyntheticHeader[];
  body?: string | null;
  certDaysLeft?: number | null;
  records?: SyntheticDnsRecord[];
}

const num = (s: string): number | null => (s.trim() !== '' && Number.isFinite(Number(s)) ? Number(s) : null);
const negative = (op: SyntheticOperator) => op === 'isNot' || op === 'doesNotContain' || op === 'doesNotMatch';

function compareNumber(op: SyntheticOperator, actual: number, target: string): boolean | string {
  const t = num(target);
  if (t === null) return `'${target}' is not a number`;
  if (op === 'is') return actual === t;
  if (op === 'isNot') return actual !== t;
  if (op === 'lessThan') return actual < t;
  if (op === 'moreThan' || op === 'isInMoreThan') return actual > t;
  return `${op} doesn't compare numbers`;
}

function compareText(op: SyntheticOperator, actual: string, target: string): boolean | string {
  switch (op) {
    case 'is':
      return actual === target;
    case 'isNot':
      return actual !== target;
    case 'contains':
      return actual.includes(target);
    case 'doesNotContain':
      return !actual.includes(target);
    case 'matches':
    case 'doesNotMatch':
      try {
        const hit = new RegExp(target).test(actual);
        return op === 'matches' ? hit : !hit;
      } catch {
        return `Invalid pattern '${target}'`;
      }
    default: {
      const a = num(actual);
      return a === null ? `'${actual}' is not a number` : compareNumber(op, a, target);
    }
  }
}

/** `$.a.b[0]['c d']` (the `$` is optional) → keys and indexes; null when it isn't a path. */
export function parseJsonPath(path: string): (string | number)[] | null {
  let rest = path.trim();
  if (!rest) return null;
  const steps: (string | number)[] = [];
  if (rest.startsWith('$')) rest = rest.slice(1);
  else if (!/^[.[]/.test(rest)) {
    const m = /^[^.[]+/.exec(rest)!;
    steps.push(m[0]);
    rest = rest.slice(m[0].length);
  }
  while (rest) {
    let m: RegExpExecArray | null;
    if ((m = /^\.([^.[]+)/.exec(rest))) steps.push(m[1]);
    else if ((m = /^\[\s*(['"])(.*?)\1\s*\]/.exec(rest))) steps.push(m[2]);
    else if ((m = /^\[\s*(\d+)\s*\]/.exec(rest))) steps.push(Number(m[1]));
    else return null;
    rest = rest.slice(m[0].length);
  }
  return steps;
}

export function evaluateAssertion(a: SyntheticAssertion, o: Observed): SyntheticAssertionResult {
  const done = (actual: string | null, outcome: boolean | string): SyntheticAssertionResult =>
    typeof outcome === 'string' ? { assertion: a, passed: false, actual, error: outcome } : { assertion: a, passed: outcome, actual, error: null };
  const op = a.operator;
  switch (a.type) {
    case 'statusCode':
      if (o.statusCode == null) return done(null, false);
      return done(String(o.statusCode), op === 'matches' || op === 'doesNotMatch' ? compareText(op, String(o.statusCode), a.target) : compareNumber(op, o.statusCode, a.target));
    case 'responseTime':
      return done(String(Math.round(o.responseTimeMs)), compareNumber(op, o.responseTimeMs, a.target));
    case 'header': {
      const values = (o.headers ?? []).filter((h) => h.name.toLowerCase() === (a.property ?? '').trim().toLowerCase()).map((h) => h.value);
      if (!values.length) return done(null, negative(op));
      return done(values.join(', '), compareText(op, values.join(', '), a.target));
    }
    case 'body':
      return o.body == null ? done(null, negative(op)) : done(o.body.slice(0, 200), compareText(op, o.body, a.target));
    case 'bodyJsonPath': {
      const steps = parseJsonPath(a.property ?? '');
      if (!steps) return done(null, `Invalid JSON path '${a.property}'`);
      if (o.body == null) return done(null, negative(op));
      let doc: unknown;
      try {
        doc = JSON.parse(o.body);
      } catch {
        return done(null, 'The body is not JSON');
      }
      let v: unknown = doc;
      for (const s of steps) v = v != null && typeof v === 'object' ? (v as Record<string | number, unknown>)[s] : undefined;
      if (v === undefined) return done(null, negative(op));
      const text = typeof v === 'string' ? v : JSON.stringify(v);
      return done(text, compareText(op, text, a.target));
    }
    case 'certificate':
      return o.certDaysLeft == null ? done(null, false) : done(o.certDaysLeft.toFixed(1), compareNumber(op, o.certDaysLeft, a.target));
    case 'recordEvery':
    case 'recordSome': {
      const values = (o.records ?? []).filter((r) => r.type === a.property).map((r) => r.value);
      if (!values.length) return done(null, false);
      const checks = values.map((v) => compareText(op, v, a.target));
      const error = checks.find((c) => typeof c === 'string');
      if (error !== undefined) return done(values.join(', '), error);
      return done(values.join(', '), a.type === 'recordEvery' ? checks.every(Boolean) : checks.some(Boolean));
    }
  }
}

/** Assertions to start from after a Send, as the server suggests them. */
export function suggestAssertions(type: SyntheticTestType, run: SyntheticRunDetail): SyntheticAssertion[] {
  const time: SyntheticAssertion = { type: 'responseTime', operator: 'lessThan', property: '', target: String(Math.max(1000, Math.ceil((run.durationMs * 2) / 100) * 100)) };
  if (type === 'http') {
    const out: SyntheticAssertion[] = [];
    if (run.statusCode != null) out.push({ type: 'statusCode', operator: 'is', property: '', target: String(run.statusCode) });
    out.push(time);
    const ct = run.responseHeaders.find((h) => h.name.toLowerCase() === 'content-type');
    if (ct) out.push({ type: 'header', operator: 'is', property: 'content-type', target: ct.value });
    return out;
  }
  if (type === 'ssl') return [...(run.certificate ? [{ type: 'certificate', operator: 'isInMoreThan', property: '', target: '10' } as SyntheticAssertion] : []), time];
  if (type === 'dns') {
    const r = run.records.find((x) => x.type === 'A') ?? run.records[0];
    return r ? [time, { type: 'recordSome', operator: 'is', property: r.type, target: r.value }] : [time];
  }
  return [time];
}

/** Why a run failed, in one line. */
export function failureSummary(run: Pick<SyntheticRunDetail, 'passed' | 'error' | 'assertions'>): string | null {
  if (run.passed) return null;
  if (run.error) return run.error;
  const failed = run.assertions.filter((a) => !a.passed).map((a) => `${assertionText(a.assertion)} (${a.error ?? (a.actual != null ? `got ${a.actual}` : 'nothing found')})`);
  if (!failed.length) return 'The run failed';
  return failed.length === 1 ? `Assertion failed: ${failed[0]}` : `${failed.length} assertions failed: ${failed.join('; ')}`;
}

// ---------------------------------------------------------------- uptime

/**
 * Share of [from, to) the test was OK, of the time it was OK or alerting (No Data doesn't count, nor
 * time before the test existed), and its state over the range. `events` are [at, from, to] state
 * changes in the range, oldest first; `current` is the state now.
 */
export function uptimeOf(
  current: MonitorStatus,
  createdAt: number,
  events: [number, MonitorStatus, MonitorStatus][],
  from: number,
  to: number,
): { uptime: number | null; segments: SyntheticUptimeSegment[] } {
  const start = Math.max(from, createdAt);
  if (to <= start) return { uptime: null, segments: [] };
  const raw: SyntheticUptimeSegment[] = [];
  let status = events[0]?.[1] ?? current;
  let t = start;
  for (const [at, , next] of events) {
    if (at > to) break;
    if (at > t) raw.push({ status, from: t, to: at });
    status = next;
    t = Math.max(t, at);
  }
  if (to > t) raw.push({ status, from: t, to });
  const segments: SyntheticUptimeSegment[] = [];
  for (const s of raw) {
    const last = segments[segments.length - 1];
    if (last && last.status === s.status && last.to === s.from) last.to = s.to;
    else segments.push({ ...s });
  }
  let up = 0;
  let down = 0;
  for (const s of segments) {
    if (s.status === 'ok') up += s.to - s.from;
    else if (s.status === 'alert' || s.status === 'warn') down += s.to - s.from;
  }
  return { uptime: up + down > 0 ? (up * 100) / (up + down) : null, segments };
}

export function fmtUptime(pct: number | null | undefined): string {
  if (pct == null) return '—';
  if (pct === 100) return '100%';
  return `${Math.min(pct, 99.99).toFixed(2)}%`;
}

// ---------------------------------------------------------------- search

/** Words match name, domain or tags; `type:`, `env:`, `team:` and `status:` (ok, alert, nodata, paused) filter; other `key:value` match tags. */
export function matchesTestQuery(t: Pick<SyntheticTest, 'name' | 'domain' | 'tags' | 'type' | 'env' | 'team' | 'status' | 'paused'>, query: string): boolean {
  const status = t.paused ? 'paused' : t.status;
  return query
    .split(/\s+/)
    .filter(Boolean)
    .every((raw) => {
      const w = raw.toLowerCase();
      const i = w.indexOf(':');
      const [k, v] = i > 0 ? [w.slice(0, i), w.slice(i + 1)] : ['', w];
      const tags = t.tags.map((x) => x.toLowerCase());
      if (k === 'type') return t.type === v;
      if (k === 'env') return t.env.toLowerCase() === v;
      if (k === 'team') return t.team.toLowerCase() === v;
      if (k === 'status') return status === v || (v === 'no_data' && status === 'nodata');
      if (k) return tags.includes(w);
      return t.name.toLowerCase().includes(w) || t.domain.toLowerCase().includes(w) || tags.some((x) => x.includes(w));
    });
}

// ---------------------------------------------------------------- editor

/** What the editor edits (the test input with every field present). */
export interface TestDraft {
  name: string;
  type: SyntheticTestType;
  request: SyntheticRequest;
  assertions: SyntheticAssertion[];
  locations: string[];
  frequencySeconds: number;
  retry: { count: number; intervalMs: number };
  minFailureDurationMinutes: number;
  minLocationFailed: number;
  env: string;
  team: string;
  tags: string[];
  message: string;
  priority: number | null;
  paused: boolean;
}

export const DEFAULT_MESSAGE = '{{#is_alert}}The test is failing.{{/is_alert}}\n{{#is_recovery}}The test passes again.{{/is_recovery}}';

/** The editor's defaults (Settings → Default Settings). */
export interface DraftDefaults {
  locations: string[];
  tags: string[];
  timeoutSeconds: number;
  frequencySeconds: number;
  retry: { count?: number; intervalMs?: number };
  minFailureDurationMinutes: number;
}

export function newDraft(type: SyntheticTestType = 'http', defaults?: DraftDefaults): TestDraft {
  return {
    name: '',
    type,
    request: { ...defaultRequest(type), timeoutSeconds: defaults?.timeoutSeconds ?? 30 },
    assertions: [],
    locations: defaults?.locations.length ? [...defaults.locations] : [LOCATION_ID],
    frequencySeconds: defaults?.frequencySeconds ?? 300,
    retry: { count: defaults?.retry.count ?? 0, intervalMs: defaults?.retry.intervalMs ?? 300 },
    minFailureDurationMinutes: defaults?.minFailureDurationMinutes ?? 0,
    minLocationFailed: 1,
    env: '',
    team: '',
    tags: defaults ? [...defaults.tags] : [],
    message: DEFAULT_MESSAGE,
    priority: null,
    paused: false,
  };
}

export function draftOf(t: SyntheticTest): TestDraft {
  return {
    name: t.name,
    type: t.type,
    request: { ...t.request, headers: t.request.headers.map((h) => ({ ...h })) },
    assertions: t.assertions.map((a) => ({ ...a })),
    locations: [...t.locations],
    frequencySeconds: t.frequencySeconds,
    retry: { count: t.retry.count ?? 0, intervalMs: t.retry.intervalMs ?? 300 },
    minFailureDurationMinutes: t.minFailureDurationMinutes,
    minLocationFailed: t.minLocationFailed,
    env: t.env,
    team: t.team,
    tags: [...t.tags],
    message: t.message,
    priority: t.priority ?? null,
    paused: t.paused,
  };
}

/** Switching the request type keeps the host (from the URL, or back into one) and drops assertions that no longer apply. */
export function withType(d: TestDraft, type: SyntheticTestType): TestDraft {
  if (d.type === type) return d;
  const host = testDomain(d.type, d.request);
  const request = { ...defaultRequest(type), timeoutSeconds: d.request.timeoutSeconds };
  if (type === 'http') request.url = host ? `https://${host}/` : 'https://';
  else request.host = host;
  if (type === 'tcp') request.port = d.request.port ?? 443;
  return { ...d, type, request, assertions: d.assertions.filter((a) => ASSERTION_TYPES[type].includes(a.type)) };
}

export interface Snippet {
  category: SnippetCategory;
  id: string;
  title: string;
  description: string;
  apply: (d: TestDraft) => TestDraft;
}

const withHeader = (d: TestDraft, name: string, value: string): TestDraft => ({
  ...d,
  request: { ...d.request, headers: [...d.request.headers.filter((h) => h.name.toLowerCase() !== name.toLowerCase()), { name, value }] },
});
const withAssertions = (d: TestDraft, add: SyntheticAssertion[]): TestDraft => ({
  ...d,
  assertions: [...d.assertions.filter((a) => !add.some((n) => n.type === a.type && n.property === a.property)), ...add],
});

/** Pre-filled starting points the editor's Snippets panel applies to the draft. */
/** The groups of the editor's Snippets panel, in order. */
export const SNIPPET_CATEGORIES = ['Authentication', 'Performance', 'Validation'] as const;
export type SnippetCategory = (typeof SNIPPET_CATEGORIES)[number];

export const SNIPPETS: Snippet[] = [
  {
    id: 'health',
    category: 'Performance',
    title: 'Health check',
    description: 'GET a health endpoint: status 200 within a second.',
    apply: (d) => {
      const h = withType(d, 'http');
      const base = testDomain('http', h.request);
      return withAssertions({ ...h, request: { ...h.request, method: 'GET', url: base ? `https://${base}/health` : 'https://shop.example.com/health' }, name: h.name || 'Health check' }, [
        { type: 'statusCode', operator: 'is', property: '', target: '200' },
        { type: 'responseTime', operator: 'lessThan', property: '', target: '1000' },
      ]);
    },
  },
  {
    id: 'json',
    category: 'Validation',
    title: 'JSON API',
    description: 'Ask for JSON and check a field of the response.',
    apply: (d) =>
      withAssertions(withHeader(withType(d, 'http'), 'Accept', 'application/json'), [
        { type: 'header', operator: 'contains', property: 'content-type', target: 'application/json' },
        { type: 'bodyJsonPath', operator: 'is', property: '$.status', target: 'ok' },
      ]),
  },
  {
    id: 'basic',
    category: 'Authentication',
    title: 'Basic authentication',
    description: 'Send a username and password with the request.',
    apply: (d) => {
      const h = withType(d, 'http');
      return { ...h, request: { ...h.request, basicAuth: h.request.basicAuth ?? { username: '', password: '' } } };
    },
  },
  {
    id: 'bearer',
    category: 'Authentication',
    title: 'Bearer token',
    description: 'Send an Authorization: Bearer header.',
    apply: (d) => withHeader(withType(d, 'http'), 'Authorization', 'Bearer <token>'),
  },
  {
    id: 'uptime',
    category: 'Performance',
    title: 'High uptime',
    description: 'Run every minute, retry once and alert after 2 minutes of failures.',
    apply: (d) => ({ ...d, frequencySeconds: 60, retry: { count: 1, intervalMs: 300 }, minFailureDurationMinutes: 2 }),
  },
  {
    id: 'certificate',
    category: 'Validation',
    title: 'Certificate expiry',
    description: 'An SSL test that alerts 30 days before the certificate expires.',
    apply: (d) => {
      const s = withType(d, 'ssl');
      return withAssertions({ ...s, frequencySeconds: Math.max(s.frequencySeconds, 3600), name: s.name || 'Certificate expiry' }, [
        { type: 'certificate', operator: 'isInMoreThan', property: '', target: '30' },
      ]);
    },
  },
];

/** The request part the editor can send: what's missing before Send / Save. */
export function requestProblem(type: SyntheticTestType, r: SyntheticRequest): string | null {
  // Global variables (`{{ NAME }}`) are checked once filled in, when the request is sent.
  if (type === 'http') {
    try {
      const u = new URL(neutralized(r.url));
      if (!/^https?:$/.test(u.protocol) || !u.hostname) return 'Enter an http or https URL';
    } catch {
      return 'Enter a full URL, such as https://shop.example.com/health';
    }
    return null;
  }
  const host = neutralized(r.host.trim());
  if (!host) return type === 'dns' ? 'Enter a domain' : 'Enter a host';
  if (/[/\s@]/.test(host) || /^[a-z]+:\/\//i.test(host)) return 'Enter a host name or IP address, without scheme or path';
  if (type === 'tcp' && !r.port) return 'Enter a port';
  return null;
}
