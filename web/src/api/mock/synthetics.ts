// In-browser synthetic tests for mock mode (Digital Experience → Synthetic Tests). Tests live in memory
// for the session; their scheduled results are computed on the fly from each test's cadence and a few
// fictional outages, so ranges, uptime and charts behave like the server's (routes/synthetics.rs).
import { LOCATION_ID, defaultRequest, evaluateAssertion, matchesTestQuery, requestProblem, suggestAssertions, testDomain, uptimeOf, type Observed } from '../synthetics/logic';
import type {
  SyntheticAssertion,
  SyntheticLocation,
  SyntheticRequest,
  SyntheticResult,
  SyntheticResults,
  SyntheticResultSummary,
  SyntheticRunDetail,
  SyntheticTest,
  SyntheticTestInput,
  SyntheticTestsPage,
  SyntheticTestSearchRequest,
  SyntheticTestType,
  SyntheticTryRequest,
  SyntheticTryResult,
} from '../synthetics/types';
import type {
  SyntheticCoverage,
  SyntheticCoverageRequest,
  SyntheticResultRow,
  SyntheticResultSearchPage,
  SyntheticResultSearchRequest,
  SyntheticSettings,
  SyntheticSuite,
  SyntheticSuiteInput,
  SyntheticSuitesPage,
  SyntheticSuiteSearchRequest,
  SyntheticVariable,
  SyntheticVariableInput,
} from '../synthetics/types';
import { endpointOf, enforcementProblem, matchesResultQuery, matchesSuiteQuery, routeMatches, substitute, suiteStatus, testTarget, usedVariables, validVariableName } from '../synthetics/model';
import type { components } from '../generated/schema';
import { listResources } from './apm';

type MonitorStatus = components['schemas']['MonitorStatus'];

const MIN = 60_000;
const HOUR = 60 * MIN;
const DAY = 24 * HOUR;
const MAX_RESULTS = 5000;
const FREQUENCIES = [60, 300, 900, 1800, 3600, 21_600, 43_200, 86_400, 604_800];

interface MockTest extends SyntheticTest {
  /** [from, to) when the target fails (to = Infinity: still failing). */
  outages: [number, number][];
  /** Typical response time (ms). */
  baseMs: number;
}

interface MockSuite {
  id: string;
  name: string;
  description: string;
  env: string;
  team: string;
  tags: string[];
  testIds: string[];
  createdBy: string;
  updatedBy: string;
  createdAt: number;
  updatedAt: number;
}

interface MockVariable {
  id: string;
  name: string;
  value: string;
  secure: boolean;
  description: string;
  tags: string[];
  createdBy: string;
  updatedBy: string;
  createdAt: number;
  updatedAt: number;
}

interface Db {
  tests: MockTest[];
  manual: SyntheticResult[];
  seq: number;
  suites: MockSuite[];
  variables: MockVariable[];
  settings: SyntheticSettings;
}

let state: Db | null = null;

const fail = (message: string): never => {
  throw new Error(message);
};

export function mockLocation(): SyntheticLocation {
  const host = (globalThis as { location?: { hostname?: string } }).location?.hostname || 'localhost';
  return { id: LOCATION_ID, name: `ObserveCraft server (${host})`, privateTargets: false };
}

/** A stable pseudo-random number in [0, 1) for (seed, t). */
function noise(seed: number, t: number): number {
  let h = (Math.imul(seed, 2_654_435_761) + Math.floor(t / 1000)) >>> 0;
  h = Math.imul(h ^ (h >>> 16), 2_246_822_507) >>> 0;
  h = Math.imul(h ^ (h >>> 13), 3_266_489_909) >>> 0;
  return ((h ^ (h >>> 16)) >>> 0) / 4_294_967_296;
}

function seedOf(id: string): number {
  let h = 0;
  for (const c of id) h = (h * 31 + c.charCodeAt(0)) >>> 0;
  return h;
}

function req(type: SyntheticTestType, patch: Partial<SyntheticRequest>): SyntheticRequest {
  return { ...defaultRequest(type), ...patch };
}

const A = (type: SyntheticAssertion['type'], operator: SyntheticAssertion['operator'], target: string, property = ''): SyntheticAssertion => ({ type, operator, property, target });

function db(): Db {
  if (state) return state;
  const now = Date.now();
  const s: Db = { tests: [], manual: [], seq: 1, suites: [], variables: [], settings: defaultSettings() };
  const add = (
    t: Omit<MockTest, 'domain' | 'locations' | 'status' | 'muted' | 'monitorId' | 'uptime' | 'lastRunAt' | 'nextRunAt' | 'failingSince' | 'updatedBy' | 'updatedAt'> & { updatedAt?: number },
  ) => {
    s.tests.push({
      ...t,
      domain: testDomain(t.type, t.request),
      locations: [LOCATION_ID],
      status: 'nodata',
      muted: false,
      monitorId: 9000 + s.tests.length,
      uptime: null,
      lastRunAt: null,
      nextRunAt: now,
      failingSince: null,
      updatedBy: t.createdBy,
      updatedAt: t.updatedAt ?? t.createdAt,
    });
  };
  const common = { retry: { count: 0, intervalMs: 300 }, minLocationFailed: 1, priority: null, paused: false, createdBy: 'Priya Shah' };
  add({
    ...common,
    id: 'f1d2c3b4-0001-4a00-8000-00000000a001',
    name: 'Storefront homepage',
    type: 'http',
    request: req('http', { url: 'https://shop.example.com/' }),
    assertions: [A('statusCode', 'is', '200'), A('responseTime', 'lessThan', '1000'), A('header', 'contains', 'text/html', 'content-type')],
    frequencySeconds: 60,
    minFailureDurationMinutes: 2,
    env: 'prod',
    team: 'storefront',
    tags: ['service:storefront-api', 'team:storefront'],
    message: '{{#is_alert}}The storefront homepage is failing. @slack-storefront-alerts{{/is_alert}}\n{{#is_recovery}}The storefront homepage is back. @slack-storefront-alerts{{/is_recovery}}',
    createdAt: now - 40 * DAY,
    outages: [
      [now - 9 * HOUR, now - 9 * HOUR + 22 * MIN],
      [now - 3 * DAY - 2 * HOUR, now - 3 * DAY - HOUR - 35 * MIN],
    ],
    baseMs: 180,
  });
  add({
    ...common,
    id: 'f1d2c3b4-0002-4a00-8000-00000000a002',
    name: 'Checkout API health',
    type: 'http',
    request: req('http', { url: 'https://api.example.com/checkout/health', headers: [{ name: 'Accept', value: 'application/json' }] }),
    assertions: [A('statusCode', 'is', '200'), A('bodyJsonPath', 'is', 'ok', '$.status'), A('responseTime', 'lessThan', '800')],
    frequencySeconds: 60,
    minFailureDurationMinutes: 0,
    retry: { count: 1, intervalMs: 500 },
    env: 'prod',
    team: 'checkout',
    tags: ['service:order-service', 'tier:critical'],
    message: '{{#is_alert}}Checkout health check failing. @webhook-oncall{{/is_alert}}',
    priority: 1,
    createdAt: now - 25 * DAY,
    outages: [
      [now - 14 * MIN, Infinity],
      [now - 2 * DAY - 5 * HOUR, now - 2 * DAY - 5 * HOUR + 9 * MIN],
    ],
    baseMs: 95,
  });
  add({
    ...common,
    id: 'f1d2c3b4-0003-4a00-8000-00000000a003',
    name: 'Catalog search',
    type: 'http',
    request: req('http', { method: 'POST', url: 'https://api.example.com/catalog/search', body: '{"query":"running shoes","limit":20}', bodyType: 'application/json' }),
    assertions: [A('statusCode', 'is', '200'), A('bodyJsonPath', 'moreThan', '0', '$.total'), A('responseTime', 'lessThan', '1500')],
    frequencySeconds: 300,
    minFailureDurationMinutes: 10,
    env: 'prod',
    team: 'catalog',
    tags: ['service:catalog-service'],
    message: '{{#is_alert}}Catalog search is failing. @slack-catalog{{/is_alert}}',
    createdAt: now - 18 * DAY,
    outages: [[now - 6 * DAY, now - 6 * DAY + 50 * MIN]],
    baseMs: 420,
  });
  add({
    ...common,
    id: 'f1d2c3b4-0004-4a00-8000-00000000a004',
    name: 'Storefront certificate',
    type: 'ssl',
    request: req('ssl', { host: 'shop.example.com', port: 443 }),
    assertions: [A('certificate', 'isInMoreThan', '30')],
    frequencySeconds: 3600,
    minFailureDurationMinutes: 0,
    env: 'prod',
    team: 'storefront',
    tags: ['service:api-gateway'],
    message: '{{#is_alert}}The shop.example.com certificate expires soon. @slack-platform{{/is_alert}}',
    createdAt: now - 60 * DAY,
    outages: [],
    baseMs: 48,
  });
  add({
    ...common,
    id: 'f1d2c3b4-0005-4a00-8000-00000000a005',
    name: 'Storefront DNS',
    type: 'dns',
    request: req('dns', { host: 'shop.example.com' }),
    assertions: [A('recordSome', 'is', '203.0.113.10', 'A'), A('responseTime', 'lessThan', '500')],
    frequencySeconds: 900,
    minFailureDurationMinutes: 0,
    env: 'prod',
    team: 'platform',
    tags: ['service:api-gateway'],
    message: '',
    createdAt: now - 30 * DAY,
    outages: [],
    baseMs: 14,
  });
  add({
    ...common,
    id: 'f1d2c3b4-0006-4a00-8000-00000000a006',
    name: 'Inventory service port',
    type: 'tcp',
    request: req('tcp', { host: 'inventory.example.com', port: 8443 }),
    assertions: [A('responseTime', 'lessThan', '300')],
    frequencySeconds: 300,
    minFailureDurationMinutes: 5,
    env: 'staging',
    team: 'inventory',
    tags: ['service:inventory-service'],
    message: '{{#is_alert}}inventory-service is unreachable. @slack-inventory{{/is_alert}}',
    paused: true,
    createdAt: now - 12 * DAY,
    updatedAt: now - 2 * DAY,
    outages: [],
    baseMs: 22,
  });
  add({
    ...common,
    id: 'f1d2c3b4-0007-4a00-8000-00000000a007',
    name: 'Billing webhook receiver',
    type: 'http',
    request: req('http', { method: 'POST', url: 'https://billing.example.com/webhooks/ping', body: '{"event":"ping"}', bodyType: 'application/json' }),
    assertions: [A('statusCode', 'lessThan', '300')],
    frequencySeconds: 900,
    minFailureDurationMinutes: 0,
    env: 'prod',
    team: 'billing',
    tags: ['service:billing'],
    message: '',
    createdAt: now - 4 * MIN,
    outages: [],
    baseMs: 130,
  });
  add({
    ...common,
    id: 'f1d2c3b4-0008-4a00-8000-00000000a008',
    name: 'Product listing API',
    type: 'http',
    request: req('http', { url: 'https://{{ SHOP_HOST }}/api/products?limit=20', headers: [{ name: 'Accept', value: 'application/json' }] }),
    assertions: [A('statusCode', 'is', '200'), A('responseTime', 'lessThan', '1200')],
    frequencySeconds: 300,
    minFailureDurationMinutes: 0,
    env: 'prod',
    team: 'catalog',
    tags: ['service:api-gateway', 'team:catalog'],
    message: '',
    createdAt: now - 9 * DAY,
    outages: [[now - 5 * DAY, now - 5 * DAY + 20 * MIN]],
    baseMs: 160,
  });
  const suite = (id: string, name: string, description: string, env: string, team: string, tags: string[], tests: number[], age: number): MockSuite => ({
    id,
    name,
    description,
    env,
    team,
    tags,
    testIds: tests.map((n) => s.tests[n - 1].id),
    createdBy: 'Priya Shah',
    updatedBy: 'Priya Shah',
    createdAt: now - age,
    updatedAt: now - age,
  });
  s.suites = [
    suite('5a1e0000-0001-4b00-8000-00000000b001', 'Checkout journey', 'Cart, payment and order confirmation APIs', 'prod', 'checkout', ['service:order-service', 'tier:critical'], [2, 3, 1], 20 * DAY),
    suite('5a1e0000-0002-4b00-8000-00000000b002', 'Storefront availability', 'Homepage, certificate and DNS of the shop', 'prod', 'storefront', ['service:storefront-api'], [1, 4, 5], 30 * DAY),
    suite('5a1e0000-0003-4b00-8000-00000000b003', 'Back office integrations', 'Billing webhooks and the inventory service', 'staging', 'billing', ['service:billing'], [7, 6], 3 * DAY),
  ];
  const variable = (id: string, name: string, value: string, secure: boolean, description: string): MockVariable => ({
    id,
    name,
    value,
    secure,
    description,
    tags: [],
    createdBy: 'Priya Shah',
    updatedBy: 'Priya Shah',
    createdAt: now - 10 * DAY,
    updatedAt: now - 10 * DAY,
  });
  s.variables = [
    variable('7a7a0000-0001-4c00-8000-00000000c001', 'SHOP_HOST', 'shop.example.com', false, 'Host of the public storefront'),
    variable('7a7a0000-0002-4c00-8000-00000000c002', 'CHECKOUT_API_TOKEN', 'ck_live_example_token', true, 'Token of the synthetic checkout user'),
  ];
  state = s;
  return s;
}

function find(id: string): MockTest {
  return db().tests.find((t) => t.id === id) ?? fail('Synthetic test not found');
}

function failingAt(t: MockTest, at: number): boolean {
  return t.outages.some(([from, to]) => at >= from && at < to);
}

/** Times of the test's scheduled runs in [from, to] (none while paused after its last update). */
function runTimes(t: MockTest, from: number, to: number): number[] {
  const step = t.frequencySeconds * 1000;
  const end = Math.min(to, Date.now(), t.paused ? t.updatedAt : Infinity);
  const start = Math.max(from, t.createdAt + step);
  const out: number[] = [];
  for (let at = Math.ceil(start / step) * step; at <= end; at += step) out.push(at);
  return out;
}

/** Status-code-like outcome and response for a fictional target. */
function respond(type: SyntheticTestType, r: SyntheticRequest, seed: number, at: number, failing: boolean, baseMs: number): { run: SyntheticRunDetail; observed: Observed } {
  const durationMs = Math.round(baseMs * (0.7 + noise(seed, at) * 0.8) * (failing ? 3 : 1));
  const run: SyntheticRunDetail = {
    passed: false,
    error: null,
    durationMs,
    assertions: [],
    resolvedIp: '203.0.113.10',
    method: null,
    url: null,
    redirects: [],
    statusCode: null,
    responseHeaders: [],
    body: null,
    bodySize: null,
    bodyTruncated: false,
    certificate: null,
    dnsServer: null,
    records: [],
  };
  const host = testDomain(type, r);
  if (/^(localhost|127\.|10\.|192\.168\.|169\.254\.)/.test(host)) {
    run.error = `${host} is a private address; private targets are disabled on this server (OC_SYNTHETICS_ALLOW_PRIVATE)`;
    run.resolvedIp = null;
    return { run, observed: { responseTimeMs: durationMs } };
  }
  if (type === 'http') {
    const json = /\/(api|health|webhooks)|^api\./.test(r.url) || r.url.includes('api.');
    const status = failing ? 503 : r.method === 'POST' && r.url.includes('webhooks') ? 202 : 200;
    const body = failing
      ? json
        ? '{"status":"unavailable","error":"upstream connect error"}'
        : '<html><body><h1>503 Service Unavailable</h1></body></html>'
      : json
        ? JSON.stringify({ status: 'ok', service: host.split('.')[0], version: '2.14.1', total: 128, checks: { database: 'ok', cache: 'ok' } })
        : '<!doctype html><html><head><title>Example Shop</title></head><body><main>Welcome to Example Shop</main></body></html>';
    run.method = r.method;
    run.url = r.url;
    run.statusCode = status;
    run.responseHeaders = [
      { name: 'content-type', value: json ? 'application/json; charset=utf-8' : 'text/html; charset=utf-8' },
      { name: 'cache-control', value: 'no-store' },
      { name: 'x-request-id', value: `req-${Math.floor(noise(seed, at + 1) * 1e9).toString(16)}` },
      { name: 'content-length', value: String(body.length) },
    ];
    run.body = body;
    run.bodySize = body.length;
    return { run, observed: { statusCode: status, responseTimeMs: durationMs, headers: run.responseHeaders, body } };
  }
  if (type === 'ssl') {
    const notAfter = Math.floor(at / DAY) * DAY + 63 * DAY;
    run.certificate = {
      subject: `CN=${host}`,
      issuer: 'CN=Example Issuing CA R3,O=Example Trust Services,C=US',
      notBefore: notAfter - 90 * DAY,
      notAfter,
      daysLeft: (notAfter - at) / DAY,
      trusted: true,
      trustError: null,
      protocol: 'TLS 1.3',
      cipher: 'TLS13_AES_128_GCM_SHA256',
    };
    return { run, observed: { responseTimeMs: durationMs, certDaysLeft: run.certificate.daysLeft } };
  }
  if (type === 'dns') {
    run.resolvedIp = null;
    run.dnsServer = r.dnsServer ? `${r.dnsServer}:${r.dnsServerPort ?? 53}` : 'system resolver';
    run.records = [
      { type: 'A', value: '203.0.113.10', ttl: 300 },
      { type: 'AAAA', value: '2001:db8::10', ttl: 300 },
      { type: 'MX', value: '10 mail.example.com', ttl: 3600 },
      { type: 'TXT', value: 'v=spf1 include:_spf.example.com ~all', ttl: 3600 },
      { type: 'NS', value: 'ns1.example.net', ttl: 86400 },
    ];
    return { run, observed: { responseTimeMs: durationMs, records: run.records } };
  }
  if (failing) run.error = `Connection to 203.0.113.10:${r.port ?? 0} failed: Connection refused (os error 61)`;
  return { run, observed: { responseTimeMs: durationMs } };
}

function runOf(type: SyntheticTestType, r: SyntheticRequest, assertions: SyntheticAssertion[], seed: number, at: number, failing: boolean, baseMs: number): SyntheticRunDetail {
  const { run, observed } = respond(type, r, seed, at, failing, baseMs);
  if (run.error) return run;
  run.assertions = assertions.map((a) => evaluateAssertion(a, observed));
  run.passed = run.assertions.every((a) => a.passed);
  return run;
}

function variableValues(): Record<string, string> {
  return Object.fromEntries(db().variables.map((v) => [v.name, v.value]));
}

/** Hides secure values in what a run shows, as the server does. */
function masked(run: SyntheticRunDetail): SyntheticRunDetail {
  const secrets = db()
    .variables.filter((v) => v.secure && v.value)
    .map((v) => v.value);
  if (!secrets.length) return run;
  const hide = (s: string) => secrets.reduce((acc, x) => acc.split(x).join('••••••'), s);
  return {
    ...run,
    url: run.url && hide(run.url),
    error: run.error && hide(run.error),
    body: run.body && hide(run.body),
    redirects: run.redirects.map(hide),
    responseHeaders: run.responseHeaders.map((h) => ({ ...h, value: hide(h.value) })),
    assertions: run.assertions.map((a) => ({ ...a, assertion: { ...a.assertion, target: hide(a.assertion.target) }, actual: a.actual && hide(a.actual) })),
  };
}

/** A run of a test's definition with the global variables filled in. */
function runWithVariables(type: SyntheticTestType, r: SyntheticRequest, assertions: SyntheticAssertion[], seed: number, at: number, failing: boolean, baseMs: number): SyntheticRunDetail {
  let filled: { request: SyntheticRequest; assertions: SyntheticAssertion[] };
  try {
    filled = substitute(r, assertions, variableValues());
  } catch (e) {
    return { ...respond(type, r, seed, at, false, baseMs).run, passed: false, error: (e as Error).message, statusCode: null, responseHeaders: [], body: null, bodySize: null, url: null, method: null };
  }
  return masked(runOf(type, filled.request, filled.assertions, seed, at, failing, baseMs));
}

function scheduledRun(t: MockTest, at: number): SyntheticRunDetail {
  return runWithVariables(t.type, t.request, t.assertions, seedOf(t.id), at, failingAt(t, at), t.baseMs);
}

/** Failed runs of a test with retries took every attempt. */
const attemptsOf = (t: MockTest, run: SyntheticRunDetail) => (run.passed ? 1 : 1 + (t.retry.count ?? 0));

/** The test's state changes: Alert once an outage lasted the minimum failure duration, OK when it ends. */
function transitions(t: MockTest, now: number): [number, MonitorStatus, MonitorStatus][] {
  const step = t.frequencySeconds * 1000;
  const out: [number, MonitorStatus, MonitorStatus][] = [];
  const first = Math.ceil((t.createdAt + step) / step) * step;
  for (const [from, to] of [...t.outages].sort((a, b) => a[0] - b[0])) {
    const firstFail = Math.ceil(from / step) * step;
    const alertAt = Math.ceil((firstFail + t.minFailureDurationMinutes * MIN) / step) * step;
    if (alertAt >= to || alertAt > now || alertAt < first) continue;
    out.push([alertAt, 'ok', 'alert']);
    const recover = Math.ceil(to / step) * step;
    if (Number.isFinite(to) && recover <= now) out.push([recover, 'alert', 'ok']);
  }
  return out;
}

function currentStatus(t: MockTest, now: number): MonitorStatus {
  if (runTimes(t, t.createdAt, now).length === 0) return 'nodata';
  const last = transitions(t, now)
    .filter(([at]) => at <= now)
    .pop();
  return last ? last[2] : 'ok';
}

function view(t: MockTest, range: { from: number; to: number } | null): SyntheticTest {
  const now = Date.now();
  const status = currentStatus(t, now);
  const runs = runTimes(t, t.createdAt, now);
  const last = runs[runs.length - 1] ?? null;
  const step = t.frequencySeconds * 1000;
  const failing = last !== null && !scheduledRun(t, last).passed;
  let failingSince: number | null = null;
  if (failing) failingSince = t.outages.filter(([f, to]) => last >= f && last < to).map(([f]) => Math.ceil(f / step) * step)[0] ?? last;
  const uptime = range
    ? uptimeOf(
        status,
        t.createdAt,
        transitions(t, now).filter(([at]) => at >= range.from && at <= range.to),
        range.from,
        Math.min(range.to, now),
      ).uptime
    : null;
  const { outages: _o, baseMs: _b, ...rest } = t;
  return { ...rest, status, uptime, lastRunAt: last, nextRunAt: last === null ? now : last + step, failingSince };
}

const delay = <T>(v: T, ms = 120): Promise<T> => new Promise((r) => setTimeout(() => r(v), ms));

export async function searchSyntheticTests(req: SyntheticTestSearchRequest): Promise<SyntheticTestsPage> {
  const rank = (t: SyntheticTest) => (t.paused ? 0 : { alert: 4, warn: 3, nodata: 2, ok: 1 }[t.status]);
  const tests = db()
    .tests.map((t) => view(t, req.range))
    .filter((t) => matchesTestQuery(t, req.query ?? ''))
    .sort((a, b) => rank(b) - rank(a) || a.name.toLowerCase().localeCompare(b.name.toLowerCase()));
  return delay({ tests, location: mockLocation() });
}

export async function getSyntheticTest(id: string): Promise<SyntheticTest> {
  return delay(view(find(id), null));
}

function check(input: SyntheticTestInput, before?: MockTest): void {
  if (!input.name.trim()) fail('The name is required');
  const problem = requestProblem(input.type, input.request);
  if (problem) fail(problem);
  if (!FREQUENCIES.includes(input.frequencySeconds)) fail('Run every 1, 5, 15 or 30 minutes, 1, 6 or 12 hours, daily or weekly');
  if ((input.retry?.count ?? 0) < 0 || (input.retry?.count ?? 0) > 3) fail('Retry 0–3 times, 0–60000 ms apart');
  const tags = input.tags ?? [];
  const env = input.env ?? '';
  // Enforced tag rules apply when tags or env change (pausing an older test still works).
  const changed = !before || before.env !== env || before.tags.join('\n') !== tags.join('\n');
  const enforced = changed ? enforcementProblem(db().settings as Parameters<typeof enforcementProblem>[0], tags, env) : null;
  if (enforced) fail(enforced);
}

function stored(input: SyntheticTestInput, base: Pick<MockTest, 'id' | 'createdAt' | 'createdBy' | 'outages' | 'baseMs' | 'monitorId'>): MockTest {
  return {
    ...base,
    name: input.name.trim(),
    type: input.type,
    domain: testDomain(input.type, input.request),
    request: input.request,
    assertions: input.assertions ?? [],
    locations: [LOCATION_ID],
    frequencySeconds: input.frequencySeconds,
    retry: { count: input.retry?.count ?? 0, intervalMs: input.retry?.intervalMs ?? 300 },
    minFailureDurationMinutes: input.minFailureDurationMinutes ?? 0,
    minLocationFailed: 1,
    env: (input.env ?? '').trim(),
    team: (input.team ?? '').trim(),
    tags: [...new Set((input.tags ?? []).map((t) => t.trim()).filter(Boolean))],
    message: input.message ?? '',
    priority: input.priority ?? null,
    paused: input.paused ?? false,
    status: 'nodata',
    muted: false,
    uptime: null,
    lastRunAt: null,
    nextRunAt: Date.now(),
    failingSince: null,
    updatedBy: 'Alex Morgan',
    updatedAt: Date.now(),
  };
}

export async function createSyntheticTest(input: SyntheticTestInput): Promise<SyntheticTest> {
  check(input);
  const s = db();
  const t = stored(input, { id: crypto.randomUUID(), createdAt: Date.now(), createdBy: 'Alex Morgan', outages: [], baseMs: 120, monitorId: 9000 + s.tests.length + s.seq++ });
  s.tests.push(t);
  return delay(view(t, null));
}

export async function updateSyntheticTest(id: string, input: SyntheticTestInput): Promise<SyntheticTest> {
  const before = find(id);
  check(input, before);
  const s = db();
  const t = stored(input, before);
  s.tests = s.tests.map((x) => (x.id === id ? t : x));
  return delay(view(t, null));
}

export async function deleteSyntheticTest(id: string): Promise<void> {
  find(id);
  const s = db();
  s.tests = s.tests.filter((t) => t.id !== id);
  s.manual = s.manual.filter((r) => r.testId !== id);
  return delay(undefined);
}

const ID_SPAN = 1e9;

function summary(id: number, at: number, run: SyntheticRunDetail, trigger: string, attempts = 1): SyntheticResultSummary {
  return {
    id,
    location: LOCATION_ID,
    startedAt: at,
    durationMs: run.durationMs,
    passed: run.passed,
    statusCode: run.statusCode,
    error: run.error,
    attempts,
    trigger,
    failedAssertions: run.assertions.filter((a) => !a.passed).length,
  };
}

export async function listSyntheticResults(id: string, range: { from: number; to: number }): Promise<SyntheticResults> {
  const t = find(id);
  const index = db().tests.indexOf(t) + 1;
  const step = t.frequencySeconds * 1000;
  const times = runTimes(t, range.from, range.to);
  const scheduled = times.slice(-MAX_RESULTS).map((at) => {
    const run = scheduledRun(t, at);
    return summary(index * ID_SPAN + Math.round(at / step), at, run, 'scheduled', attemptsOf(t, run));
  });
  const manual = db()
    .manual.filter((r) => r.testId === id && r.startedAt >= range.from && r.startedAt <= range.to)
    .map((r) => summary(r.id, r.startedAt, r.detail, 'manual'));
  const results = [...scheduled, ...manual].sort((a, b) => b.startedAt - a.startedAt).slice(0, MAX_RESULTS);
  const now = Date.now();
  const events = transitions(t, now).filter(([at]) => at >= range.from && at <= range.to);
  const { uptime, segments } = uptimeOf(currentStatus(t, now), t.createdAt, events, range.from, Math.min(range.to, now));
  return delay({ uptime, segments, results, total: times.length + manual.length });
}

export async function getSyntheticResult(id: string, resultId: number): Promise<SyntheticResult> {
  const t = find(id);
  const manual = db().manual.find((r) => r.id === resultId && r.testId === id);
  if (manual) return delay(manual);
  const index = db().tests.indexOf(t) + 1;
  if (Math.floor(resultId / ID_SPAN) !== index) fail('Result not found');
  const at = (resultId % ID_SPAN) * t.frequencySeconds * 1000;
  const detail = scheduledRun(t, at);
  if (detail.passed) detail.body = null;
  return delay({ id: resultId, testId: id, location: LOCATION_ID, startedAt: at, attempts: attemptsOf(t, detail), trigger: 'scheduled', detail });
}

export async function runSyntheticTest(id: string): Promise<SyntheticResult> {
  const t = find(id);
  const s = db();
  const at = Date.now();
  const detail = runWithVariables(t.type, t.request, t.assertions, seedOf(t.id), at, failingAt(t, at), t.baseMs);
  const result: SyntheticResult = { id: 1e15 + s.seq++, testId: id, location: LOCATION_ID, startedAt: at, attempts: 1, trigger: 'manual', detail };
  s.manual.push(result);
  return delay(result, 400);
}

export async function trySyntheticRequest(r: SyntheticTryRequest): Promise<SyntheticTryResult> {
  const problem = requestProblem(r.type, r.request);
  if (problem) fail(problem);
  const at = Date.now();
  const host = testDomain(r.type, r.request);
  // Hosts that say so fail, to show a failing response.
  const filled = substitute(r.request, r.assertions ?? [], variableValues());
  const failing = /(^|[.-])(down|fail|broken)([.-]|$)/.test(testDomain(r.type, filled.request));
  const result = masked(runOf(r.type, filled.request, filled.assertions, seedOf(host), at, failing, r.type === 'dns' ? 15 : r.type === 'tcp' ? 20 : 150));
  return delay({ location: mockLocation(), result, suggestedAssertions: result.error ? [] : suggestAssertions(r.type, result) }, 450);
}

export async function listSyntheticLocations(): Promise<SyntheticLocation[]> {
  return delay([mockLocation()]);
}

// ================================================================ test suites

/** Scheduled runs of a test in the range: [all, passed]. */
function runCounts(t: MockTest, range: { from: number; to: number }): [number, number] {
  const times = runTimes(t, range.from, range.to).slice(-MAX_RESULTS);
  return [times.length, times.filter((at) => scheduledRun(t, at).passed).length];
}

function suiteView(s: MockSuite, range: { from: number; to: number }): SyntheticSuite {
  const tests = s.testIds
    .map((id) => db().tests.find((t) => t.id === id))
    .filter((t): t is MockTest => !!t)
    .map((t) => ({ t, v: view(t, null) }));
  let runs = 0;
  let passed = 0;
  for (const { t } of tests) {
    const [r, p] = runCounts(t, range);
    runs += r;
    passed += p;
  }
  const members = tests.map(({ v }) => ({ id: v.id, name: v.name, type: v.type, status: v.status, paused: v.paused }));
  const { testIds: _ids, ...rest } = s;
  return { ...rest, tests: members, status: suiteStatus(members), uptime: runs > 0 ? (passed * 100) / runs : null };
}

function findSuite(id: string): MockSuite {
  return db().suites.find((s) => s.id === id) ?? fail('Test suite not found');
}

function checkSuite(input: SyntheticSuiteInput): Omit<MockSuite, 'id' | 'createdBy' | 'updatedBy' | 'createdAt' | 'updatedAt'> {
  const name = input.name.trim();
  if (!name) fail('The name is required');
  const env = (input.env ?? '').trim();
  if (/[\s,]/.test(env) || (input.team ?? '').includes(',')) fail('Environments have no spaces or commas, teams no commas (at most 100 characters)');
  const testIds = [...new Set(input.testIds ?? [])];
  if (testIds.length > 200) fail('A suite has at most 200 tests');
  if (testIds.some((id) => !db().tests.some((t) => t.id === id))) fail('Unknown test: every test of a suite must exist in the organization');
  return {
    name,
    description: (input.description ?? '').trim(),
    env,
    team: (input.team ?? '').trim(),
    tags: [...new Set((input.tags ?? []).map((t) => t.trim()).filter(Boolean))],
    testIds,
  };
}

const lastDay = () => ({ from: Date.now() - DAY, to: Date.now() });

export async function searchSyntheticSuites(req: SyntheticSuiteSearchRequest): Promise<SyntheticSuitesPage> {
  const rank = { alert: 3, warn: 2, nodata: 1, ok: 0 };
  const suites = db()
    .suites.map((s) => suiteView(s, req.range))
    .filter((s) => matchesSuiteQuery(s, req.query ?? ''))
    .sort((a, b) => rank[b.status] - rank[a.status] || a.name.toLowerCase().localeCompare(b.name.toLowerCase()));
  return delay({ suites });
}

export async function getSyntheticSuite(id: string): Promise<SyntheticSuite> {
  return delay(suiteView(findSuite(id), lastDay()));
}

export async function createSyntheticSuite(input: SyntheticSuiteInput): Promise<SyntheticSuite> {
  const now = Date.now();
  const s: MockSuite = { ...checkSuite(input), id: crypto.randomUUID(), createdBy: 'Alex Morgan', updatedBy: 'Alex Morgan', createdAt: now, updatedAt: now };
  db().suites.push(s);
  return delay(suiteView(s, lastDay()));
}

export async function updateSyntheticSuite(id: string, input: SyntheticSuiteInput): Promise<SyntheticSuite> {
  const before = findSuite(id);
  const s: MockSuite = { ...before, ...checkSuite(input), updatedBy: 'Alex Morgan', updatedAt: Date.now() };
  db().suites = db().suites.map((x) => (x.id === id ? s : x));
  return delay(suiteView(s, lastDay()));
}

export async function deleteSyntheticSuite(id: string): Promise<void> {
  findSuite(id);
  db().suites = db().suites.filter((s) => s.id !== id);
  return delay(undefined);
}

// ================================================================ results explorer

function explorerBucket(range: { from: number; to: number }): number {
  const target = (range.to - range.from) / 60;
  return [60_000, 300_000, 600_000, 1_800_000, 3_600_000, 10_800_000, 21_600_000, 43_200_000].find((s) => s >= target) ?? 86_400_000;
}

/** Every stored run of every test in the range, newest first (scheduled runs capped per test). */
function allResults(range: { from: number; to: number }): SyntheticResultRow[] {
  const s = db();
  const rows: SyntheticResultRow[] = [];
  s.tests.forEach((t, i) => {
    const step = t.frequencySeconds * 1000;
    const base = { testId: t.id, testName: t.name, testType: t.type, location: LOCATION_ID, env: t.env, team: t.team, tags: t.tags };
    for (const at of runTimes(t, range.from, range.to).slice(-MAX_RESULTS)) {
      const run = scheduledRun(t, at);
      rows.push({
        ...base,
        id: (i + 1) * ID_SPAN + Math.round(at / step),
        startedAt: at,
        durationMs: run.durationMs,
        passed: run.passed,
        statusCode: run.statusCode,
        error: run.error,
        attempts: attemptsOf(t, run),
        trigger: 'scheduled',
        failedAssertions: run.assertions.filter((a) => !a.passed).length,
      });
    }
    for (const r of s.manual.filter((m) => m.testId === t.id && m.startedAt >= range.from && m.startedAt <= range.to)) {
      rows.push({
        ...base,
        id: r.id,
        startedAt: r.startedAt,
        durationMs: r.detail.durationMs,
        passed: r.detail.passed,
        statusCode: r.detail.statusCode,
        error: r.detail.error,
        attempts: r.attempts,
        trigger: 'manual',
        failedAssertions: r.detail.assertions.filter((a) => !a.passed).length,
      });
    }
  });
  return rows.sort((a, b) => b.startedAt - a.startedAt || b.id - a.id);
}

const RESULT_FACETS: [string, string, (r: SyntheticResultRow) => string][] = [
  ['status', 'Status', (r) => (r.passed ? 'passed' : 'failed')],
  ['@test.type', 'Test Type', (r) => r.testType],
  ['@test.name', 'Test Name', (r) => r.testName],
  ['location', 'Location', (r) => r.location],
  ['@http.status_code', 'HTTP Status Code', (r) => (r.statusCode == null ? '' : String(r.statusCode))],
  ['@trigger', 'Trigger', (r) => r.trigger],
  ['env', 'Env', (r) => r.env],
  ['team', 'Team', (r) => r.team],
];

export async function searchSyntheticResultsAll(req: SyntheticResultSearchRequest): Promise<SyntheticResultSearchPage> {
  const query = req.query ?? '';
  const matched = allResults(req.range).filter((r) => matchesResultQuery(r, query));
  const limit = Math.min(Math.max(req.limit ?? 50, 1), 500);
  const offset = Math.max(req.offset ?? 0, 0);
  const step = explorerBucket(req.range);
  const buckets = new Map<number, { t: number; passed: number; failed: number }>();
  const groups = new Map<string, { rows: SyntheticResultRow[] }>();
  for (const r of matched) {
    const t = Math.floor(r.startedAt / step) * step;
    const b = buckets.get(t) ?? { t, passed: 0, failed: 0 };
    if (r.passed) b.passed++;
    else b.failed++;
    buckets.set(t, b);
    const g = groups.get(r.testId) ?? { rows: [] };
    g.rows.push(r);
    groups.set(r.testId, g);
  }
  const p95 = (xs: number[]) => {
    const sorted = [...xs].sort((a, b) => a - b);
    const pos = 0.95 * (sorted.length - 1);
    const lo = Math.floor(pos);
    return sorted[lo] + (sorted[Math.min(lo + 1, sorted.length - 1)] - sorted[lo]) * (pos - lo);
  };
  const facets = RESULT_FACETS.map(([key, label, of]) => {
    const counts = new Map<string, number>();
    for (const r of matched) {
      const v = of(r);
      if (v) counts.set(v, (counts.get(v) ?? 0) + 1);
    }
    const values = [...counts.entries()]
      .sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0]))
      .slice(0, 20)
      .map(([value, count]) => ({ value, count }));
    return { key, label, values };
  });
  return delay({
    total: matched.length,
    results: matched.slice(offset, offset + limit),
    series: [...buckets.values()].sort((a, b) => a.t - b.t),
    bucketMs: step,
    groups: [...groups.entries()]
      .map(([testId, { rows }]) => ({
        testId,
        testName: rows[0].testName,
        testType: rows[0].testType,
        runs: rows.length,
        failed: rows.filter((r) => !r.passed).length,
        avgDurationMs: rows.reduce((n, r) => n + r.durationMs, 0) / rows.length,
        p95DurationMs: p95(rows.map((r) => r.durationMs)),
        lastRunAt: rows[0].startedAt,
      }))
      .sort((a, b) => b.runs - a.runs || a.testName.localeCompare(b.testName))
      .slice(0, 100),
    facets,
  });
}

// ================================================================ test coverage

export async function getSyntheticCoverage(req: SyntheticCoverageRequest): Promise<SyntheticCoverage> {
  const http = db().tests.filter((t) => t.type === 'http');
  const targets = http.flatMap((t) => {
    const target = testTarget(t.request.method, t.request.url);
    return target ? [{ id: t.id, name: t.name, method: target[0], path: target[1] }] : [];
  });
  const merged = new Map<string, { service: string; method: string; route: string; requests: number; errors: number }>();
  for (const r of listResources({ range: req.range, env: req.env })) {
    const e = endpointOf(r.resource, '', '');
    if (!e) continue;
    const key = `${r.service} ${e[0]} ${e[1]}`;
    const m = merged.get(key) ?? { service: r.service, method: e[0], route: e[1], requests: 0, errors: 0 };
    m.requests += Math.round(r.requests);
    m.errors += Math.round(r.errors);
    merged.set(key, m);
  }
  const endpoints = [...merged.values()]
    .map((e) => ({ ...e, tests: targets.filter((t) => (!e.method || t.method === e.method) && routeMatches(e.route, t.path)).map(({ id, name }) => ({ id, name })) }))
    .sort((a, b) => b.requests - a.requests || a.route.localeCompare(b.route));
  return delay({ endpoints, httpTests: http.length });
}

// ================================================================ settings and global variables

function defaultSettings(): SyntheticSettings {
  return {
    enforceTagKeys: false,
    enforceEnv: false,
    defaults: { locations: [LOCATION_ID], tags: [], timeoutSeconds: 30, frequencySeconds: 300, retry: { count: 0, intervalMs: 300 }, minFailureDurationMinutes: 0 },
  };
}

export async function getSyntheticSettings(): Promise<SyntheticSettings> {
  return delay(structuredClone(db().settings));
}

export async function updateSyntheticSettings(input: SyntheticSettings): Promise<SyntheticSettings> {
  const d = { ...defaultSettings().defaults!, ...input.defaults };
  if (d.timeoutSeconds < 1 || d.timeoutSeconds > 60) fail('The default timeout is 1–60 seconds');
  if (!FREQUENCIES.includes(d.frequencySeconds)) fail('Run every 1, 5, 15 or 30 minutes, 1, 6 or 12 hours, daily or weekly');
  if ((d.retry.count ?? 0) < 0 || (d.retry.count ?? 0) > 3) fail('Retry 0–3 times, 0–60000 ms apart');
  if (d.minFailureDurationMinutes < 0 || d.minFailureDurationMinutes > 120) fail('Alert after 0–120 minutes of failures');
  const tags = [...new Set(d.tags.map((t) => t.trim()).filter(Boolean))];
  if (tags.some((t) => /[\s,]/.test(t))) fail('Up to 20 default tags of at most 200 characters, without commas or spaces');
  db().settings = { enforceTagKeys: !!input.enforceTagKeys, enforceEnv: !!input.enforceEnv, defaults: { ...d, locations: d.locations.length ? d.locations : [LOCATION_ID], tags } };
  return delay(structuredClone(db().settings));
}

function variableView(v: MockVariable): SyntheticVariable {
  const usedBy = db()
    .tests.filter((t) => usedVariables(t.request, t.assertions).includes(v.name))
    .map((t) => ({ id: t.id, name: t.name }));
  const { value, ...rest } = v;
  return { ...rest, value: v.secure ? null : value, usedBy };
}

function checkVariable(input: SyntheticVariableInput, id: string | null): void {
  if (!validVariableName(input.name.trim())) fail('Variable names are upper-case letters, digits and underscores, start with a letter and have at most 80 characters');
  if (id === null && input.value == null) fail('The value is required');
  if (db().variables.some((v) => v.name === input.name.trim() && v.id !== id)) fail(`A global variable named ${input.name.trim()} exists`);
}

export async function listSyntheticVariables(): Promise<SyntheticVariable[]> {
  return delay([...db().variables].sort((a, b) => a.name.localeCompare(b.name)).map(variableView));
}

export async function createSyntheticVariable(input: SyntheticVariableInput): Promise<SyntheticVariable> {
  checkVariable(input, null);
  const now = Date.now();
  const v: MockVariable = {
    id: crypto.randomUUID(),
    name: input.name.trim(),
    value: input.value ?? '',
    secure: !!input.secure,
    description: (input.description ?? '').trim(),
    tags: input.tags ?? [],
    createdBy: 'Alex Morgan',
    updatedBy: 'Alex Morgan',
    createdAt: now,
    updatedAt: now,
  };
  db().variables.push(v);
  return delay(variableView(v));
}

export async function updateSyntheticVariable(id: string, input: SyntheticVariableInput): Promise<SyntheticVariable> {
  const before = db().variables.find((v) => v.id === id) ?? fail('Global variable not found');
  checkVariable(input, id);
  const v: MockVariable = {
    ...before,
    name: input.name.trim(),
    value: input.value ?? before.value,
    secure: !!input.secure,
    description: (input.description ?? '').trim(),
    tags: input.tags ?? [],
    updatedBy: 'Alex Morgan',
    updatedAt: Date.now(),
  };
  db().variables = db().variables.map((x) => (x.id === id ? v : x));
  return delay(variableView(v));
}

export async function deleteSyntheticVariable(id: string): Promise<void> {
  if (!db().variables.some((v) => v.id === id)) fail('Global variable not found');
  db().variables = db().variables.filter((v) => v.id !== id);
  return delay(undefined);
}
