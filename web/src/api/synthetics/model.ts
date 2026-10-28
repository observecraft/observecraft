// Test suites, the Results Explorer's query syntax, Test Coverage matching, global variables and the
// enforced tag rules, shared by the pages and the in-browser mock. They follow the server
// (routes/synthetics_suites.rs, synthetics_explorer.rs, synthetics_settings.rs).
import { parseQuery } from '../../lib/searchQuery';
import type { components } from '../generated/schema';
import type { SyntheticAssertion, SyntheticRequest } from './types';

type MonitorStatus = components['schemas']['MonitorStatus'];

// ---------------------------------------------------------------- suites

/** A suite's state: the worst of its running tests (Alert, Warn, OK); No Data when none has a state. */
export function suiteStatus(tests: { status: MonitorStatus; paused: boolean }[]): MonitorStatus {
  const running = tests.filter((t) => !t.paused).map((t) => t.status);
  for (const s of ['alert', 'warn', 'ok'] as MonitorStatus[]) if (running.includes(s)) return s;
  return 'nodata';
}

/** Words match name, description and tags; `env:`, `team:`, `status:` filter; other `key:value` match tags. */
export function matchesSuiteQuery(s: { name: string; description: string; env: string; team: string; tags: string[]; status: MonitorStatus }, query: string): boolean {
  return query
    .split(/\s+/)
    .filter(Boolean)
    .every((raw) => {
      const w = raw.toLowerCase();
      const i = w.indexOf(':');
      const [k, v] = i > 0 ? [w.slice(0, i), w.slice(i + 1)] : ['', w];
      const tags = s.tags.map((x) => x.toLowerCase());
      if (k === 'env') return s.env.toLowerCase() === v;
      if (k === 'team') return s.team.toLowerCase() === v;
      if (k === 'status') return s.status === v || (v === 'no_data' && s.status === 'nodata');
      if (k) return tags.includes(w);
      return s.name.toLowerCase().includes(w) || s.description.toLowerCase().includes(w) || tags.some((x) => x.includes(w));
    });
}

/** The suites list's query with the facets' unions (`team:(a OR b)`) and negations (`-env:staging`). */
export function matchesSuiteTerms(s: Parameters<typeof matchesSuiteQuery>[0], query: string): boolean {
  return parseQuery(query).every((t) => {
    const hit = t.kind === 'text' ? t.text.split(/\s+/).every((w) => matchesSuiteQuery(s, w)) : t.values.some((v) => matchesSuiteQuery(s, `${t.key}:${v}`));
    return t.negate ? !hit : hit;
  });
}

// ---------------------------------------------------------------- results explorer

/** What the explorer's query filters on (a stored run and its test). */
export interface ResultFacts {
  testId: string;
  testName: string;
  testType: string;
  location: string;
  passed: boolean;
  statusCode?: number | null;
  error?: string | null;
  attempts: number;
  durationMs: number;
  trigger: string;
  env: string;
  team: string;
  tags: string[];
}

/** `*` wildcards; without one, the value is contained anywhere (case-insensitive). */
function likeMatch(actual: string, pattern: string): boolean {
  const a = actual.toLowerCase();
  const p = pattern.toLowerCase();
  if (!p.includes('*')) return a.includes(p);
  const re = new RegExp(
    `^${p
      .split('*')
      .map((x) => x.replace(/[.*+?^${}()|[\]\\]/g, '\\$&'))
      .join('.*')}$`,
  );
  return re.test(a);
}

function compare(actual: number | null | undefined, value: string, key: string, whole: boolean): boolean {
  const m = /^(>=|<=|>|<)?\s*(.*)$/.exec(value)!;
  const op = m[1] ?? '=';
  const n = Number(m[2]);
  if (m[2].trim() === '' || !Number.isFinite(n) || (whole && !Number.isInteger(n))) throw new Error(`'${m[2]}' is not a ${whole ? 'whole ' : ''}number (${key})`);
  if (actual == null) return false;
  return op === '>=' ? actual >= n : op === '<=' ? actual <= n : op === '>' ? actual > n : op === '<' ? actual < n : actual === n;
}

/** One `key:value` condition; null when the run has nothing to compare (it then fails, and passes negated). */
function condition(r: ResultFacts, rawKey: string, value: string): boolean | null {
  const key = rawKey.replace(/^@+/, '');
  switch (key) {
    case 'type':
      return value === 'result';
    case 'status':
    case 'result.status': {
      const v = value.toLowerCase();
      if (['passed', 'pass', 'ok'].includes(v)) return r.passed;
      if (['failed', 'fail', 'error'].includes(v)) return !r.passed;
      throw new Error(`status is passed or failed, not '${value}'`);
    }
    case 'test.name':
    case 'test':
      return likeMatch(r.testName, value);
    case 'test.id':
    case 'test_id':
      return r.testId.toLowerCase() === value.toLowerCase();
    case 'test.type':
    case 'test_type':
    case 'subtype':
      return r.testType === value.toLowerCase();
    case 'location':
      return r.location === value;
    case 'trigger':
    case 'result.trigger':
      return r.trigger === value.toLowerCase();
    case 'env':
      return r.env === value;
    case 'team':
      return r.team === value;
    case 'http.status_code':
    case 'http_status':
    case 'status_code':
      if (value.includes('*')) return r.statusCode == null ? null : likeMatch(String(r.statusCode), value);
      return r.statusCode == null ? (compare(0, value, key, true), null) : compare(r.statusCode, value, key, true);
    case 'result.retries':
    case 'retries':
      return compare(r.attempts - 1, value, key, true);
    case 'result.duration':
    case 'duration':
      return compare(r.durationMs, value, key, false);
    default:
      return r.tags.includes(`${key}:${value}`);
  }
}

/** The explorer query (see routes/synthetics_explorer.rs) over one run; throws on an invalid term. */
export function matchesResultQuery(r: ResultFacts, query: string): boolean {
  return parseQuery(query).every((t) => {
    const hit =
      t.kind === 'text'
        ? likeMatch(r.testName, t.text) || likeMatch(r.error ?? '', t.text)
        : t.values.map((v) => condition(r, t.key, v)).reduce<boolean | null>((acc, x) => (acc === true || x === true ? true : acc === false || x === false ? false : null), null);
    return t.negate ? hit !== true : hit === true;
  });
}

export type ResultsVisualization = 'list' | 'timeseries' | 'toplist' | 'bars' | 'table';

export interface RecommendedQuery {
  id: string;
  icon: 'http' | 'retry' | 'id' | 'manual';
  title: string;
  description: string;
  query: string;
  visualization: ResultsVisualization;
}

/** The explorer's starting points. */
export const RECOMMENDED_QUERIES: RecommendedQuery[] = [
  {
    id: 'http-errors',
    icon: 'http',
    title: 'Failed HTTP runs with error codes',
    description: 'Which tests failed because the endpoint answered with a 4xx or 5xx status.',
    query: 'status:failed @test.type:http @http.status_code:>=400',
    visualization: 'toplist',
  },
  {
    id: 'retries',
    icon: 'retry',
    title: 'Runs that needed retries',
    description: 'Runs that only finished after one or more retries: often a sign of a flaky endpoint.',
    query: '@result.retries:>0',
    visualization: 'list',
  },
  {
    id: 'test-ids',
    icon: 'id',
    title: 'Find test IDs',
    description: 'Every test with its ID, run count and failures, to reference tests from the API or CI.',
    query: '@type:result',
    visualization: 'table',
  },
  {
    id: 'manual',
    icon: 'manual',
    title: 'Runs started by hand',
    description: 'Runs started with Run Test Now, which are recorded but never change the alert state.',
    query: '@trigger:manual',
    visualization: 'list',
  },
];

// ---------------------------------------------------------------- coverage

const METHODS = ['GET', 'POST', 'PUT', 'PATCH', 'DELETE', 'HEAD', 'OPTIONS'];

/** An endpoint [method, route] of an entry span: its `http.route` and method, or a `METHOD /path` name. */
export function endpointOf(spanName: string, route: string, method: string): [string, string] | null {
  if (route.startsWith('/')) return [method.trim().toUpperCase(), route.trim()];
  const m = /^(\S+) (\/.*)$/.exec(spanName.trim());
  return m && METHODS.includes(m[1]) ? [m[1], m[2].trim()] : null;
}

/** A URL path served by a route template (`/orders/{id}`, `/orders/:id`, `/orders/?`, `/static/*`). */
export function routeMatches(route: string, path: string): boolean {
  const r = route.split('/').filter(Boolean);
  const p = path.split('/').filter(Boolean);
  for (let i = 0; i < r.length; i++) {
    const seg = r[i];
    if ((seg === '*' || seg === '**' || seg.startsWith('{*')) && i === r.length - 1) return p.length >= i;
    if (i >= p.length) return false;
    const param = seg.startsWith('{') || seg.startsWith(':') || seg.startsWith('<') || seg === '?' || seg === '*';
    if (!param && seg !== p[i]) return false;
  }
  return r.length === p.length;
}

/** The [method, path] an HTTP test calls; variables in the URL don't matter. */
export function testTarget(method: string, url: string): [string, string] | null {
  try {
    const u = new URL(neutralized(url.trim()));
    if (!/^https?:$/.test(u.protocol)) return null;
    return [method.trim().toUpperCase(), u.pathname || '/'];
  } catch {
    return null;
  }
}

// ---------------------------------------------------------------- global variables

const NAME = /^[A-Z][A-Z0-9_]{0,79}$/;
const PLACEHOLDER = /\{\{\s*([^{}]*?)\s*\}\}/g;

export const validVariableName = (name: string): boolean => NAME.test(name);

/** The variable names a text uses (`{{ NAME }}`), in order, without repeats. */
export function placeholders(text: string): string[] {
  const out: string[] = [];
  for (const m of text.matchAll(PLACEHOLDER)) if (NAME.test(m[1]) && !out.includes(m[1])) out.push(m[1]);
  return out;
}

/** Every `{{ NAME }}` replaced by a plain word, to check a URL's shape. */
export function neutralized(text: string): string {
  return text.replace(PLACEHOLDER, (all, name: string) => (NAME.test(name) ? 'var' : all));
}

/** The request and assertions with variables filled in; throws naming unknown ones. */
export function substitute(request: SyntheticRequest, assertions: SyntheticAssertion[], vars: Record<string, string>): { request: SyntheticRequest; assertions: SyntheticAssertion[] } {
  const missing: string[] = [];
  const fill = (text: string) =>
    text.replace(PLACEHOLDER, (all, name: string) => {
      if (!NAME.test(name)) return all;
      if (name in vars) return vars[name];
      if (!missing.includes(name)) missing.push(name);
      return all;
    });
  const out: SyntheticRequest = {
    ...request,
    url: fill(request.url),
    host: fill(request.host),
    body: fill(request.body),
    dnsServer: fill(request.dnsServer),
    headers: request.headers.map((h) => ({ ...h, value: fill(h.value) })),
    basicAuth: request.basicAuth ? { username: fill(request.basicAuth.username), password: fill(request.basicAuth.password) } : request.basicAuth,
  };
  const filled = assertions.map((a) => ({ ...a, target: fill(a.target) }));
  if (missing.length) throw new Error(`Unknown global variables: ${missing.join(', ')}`);
  return { request: out, assertions: filled };
}

/** Every variable a test definition uses. */
export function usedVariables(request: SyntheticRequest, assertions: SyntheticAssertion[]): string[] {
  const texts = [
    request.url,
    request.host,
    request.body,
    request.dnsServer,
    ...request.headers.map((h) => h.value),
    ...(request.basicAuth ? [request.basicAuth.username, request.basicAuth.password] : []),
    ...assertions.map((a) => a.target),
  ];
  const out: string[] = [];
  for (const t of texts) for (const n of placeholders(t)) if (!out.includes(n)) out.push(n);
  return out;
}

// ---------------------------------------------------------------- enforced tag rules

/** Why a test can't be saved under the enforced rules (null: it can). */
export function enforcementProblem(s: { enforceTagKeys: boolean; enforceEnv: boolean; defaults: { tags: string[] } }, tags: string[], env: string): string | null {
  if (s.enforceTagKeys) {
    const keys: string[] = [];
    for (const t of s.defaults.tags) {
      const key = (t.includes(':') ? t.slice(0, t.indexOf(':')) : t).trim();
      if (key && !keys.includes(key)) keys.push(key);
    }
    const has = (key: string) => tags.some((t) => t.includes(':') && t.slice(0, t.indexOf(':')).trim() === key && t.slice(t.indexOf(':') + 1).trim() !== '');
    const missing = keys.filter((k) => !has(k));
    if (missing.length) return `Tests need a tag for each of: ${missing.join(', ')}`;
  }
  if (s.enforceEnv && !env.trim()) return 'Tests need an environment (env)';
  return null;
}

// ---------------------------------------------------------------- CSV

/** RFC 4180 CSV: cells with commas, quotes or line breaks are quoted. */
export function toCsv(header: string[], rows: (string | number)[][]): string {
  const cell = (v: string | number) => {
    const s = String(v);
    return /[",\n\r]/.test(s) ? `"${s.replace(/"/g, '""')}"` : s;
  };
  return [header, ...rows].map((r) => r.map(cell).join(',')).join('\n') + '\n';
}
