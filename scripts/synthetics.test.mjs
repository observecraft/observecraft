import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import { registerHooks } from 'node:module';

// The web sources use extensionless relative imports (bundler resolution); resolve them to `.ts` here.
registerHooks({
  resolve(specifier, context, next) {
    try {
      return next(specifier, context);
    } catch (err) {
      if (specifier.startsWith('.')) return next(`${specifier}.ts`, context);
      throw err;
    }
  },
  load(url, context, next) {
    if (url.endsWith('.json')) return { format: 'module', source: `export default ${readFileSync(new URL(url), 'utf8')};`, shortCircuit: true };
    return next(url, context);
  },
});
const logic = await import('../web/src/api/synthetics/logic.ts');
const mock = await import('../web/src/api/mock/synthetics.ts');

const MIN = 60_000;
const HOUR = 60 * MIN;
const a = (type, operator, target, property = '') => ({ type, operator, property, target });
const passes = (assertion, observed) => {
  const r = logic.evaluateAssertion(assertion, observed);
  assert.equal(r.error, null, JSON.stringify(assertion));
  return r.passed;
};

test('status codes, response times, headers and bodies are checked like the server checks them', () => {
  const headers = [
    { name: 'Content-Type', value: 'application/json; charset=utf-8' },
    { name: 'x-cache', value: 'HIT' },
  ];
  const o = { statusCode: 200, responseTimeMs: 120.4, headers, body: '{"status":"ok","items":[{"id":7,"name":"cart"}],"count":3}' };
  assert.ok(passes(a('statusCode', 'is', '200'), o));
  assert.ok(passes(a('statusCode', 'matches', '^2\\d\\d$'), o));
  assert.ok(!passes(a('statusCode', 'isNot', '200'), o));
  assert.ok(passes(a('responseTime', 'lessThan', '1000'), o));
  assert.equal(logic.evaluateAssertion(a('responseTime', 'lessThan', '100'), o).actual, '120');
  assert.ok(passes(a('header', 'contains', 'application/json', 'content-type'), o));
  assert.ok(!passes(a('header', 'is', '1', 'x-missing'), o));
  assert.ok(passes(a('header', 'doesNotContain', '1', 'x-missing'), o), 'a missing header passes negative operators');
  assert.ok(passes(a('body', 'contains', '"status":"ok"'), o));
  assert.ok(passes(a('bodyJsonPath', 'is', 'ok', '$.status'), o));
  assert.ok(passes(a('bodyJsonPath', 'is', '7', '$.items[0].id'), o));
  assert.ok(passes(a('bodyJsonPath', 'contains', 'car', "$.items[0]['name']"), o));
  assert.ok(passes(a('bodyJsonPath', 'moreThan', '2', 'count'), o));
  assert.equal(logic.evaluateAssertion(a('bodyJsonPath', 'is', 'ok', '$.status'), { responseTimeMs: 1, body: '<html>' }).error, 'The body is not JSON');
  assert.ok(!passes(a('statusCode', 'is', '200'), { responseTimeMs: 1 }), 'no response fails');
});

test('certificates and DNS records', () => {
  assert.ok(passes(a('certificate', 'isInMoreThan', '30'), { responseTimeMs: 1, certDaysLeft: 42.5 }));
  assert.ok(!passes(a('certificate', 'isInMoreThan', '60'), { responseTimeMs: 1, certDaysLeft: 42.5 }));
  const records = [
    { type: 'A', value: '203.0.113.10', ttl: 60 },
    { type: 'A', value: '203.0.113.11', ttl: 60 },
  ];
  assert.ok(passes(a('recordSome', 'is', '203.0.113.11', 'A'), { responseTimeMs: 1, records }));
  assert.ok(!passes(a('recordEvery', 'is', '203.0.113.11', 'A'), { responseTimeMs: 1, records }));
  assert.ok(passes(a('recordEvery', 'matches', '^203\\.0\\.113\\.', 'A'), { responseTimeMs: 1, records }));
  assert.ok(!passes(a('recordSome', 'is', '::1', 'AAAA'), { responseTimeMs: 1, records }));
});

test('JSON paths parse', () => {
  assert.deepEqual(logic.parseJsonPath('$.a.b[2][\'c d\']["e"]'), ['a', 'b', 2, 'c d', 'e']);
  assert.deepEqual(logic.parseJsonPath('a[0]'), ['a', 0]);
  assert.deepEqual(logic.parseJsonPath('$'), []);
  for (const bad of ['', '$..a', '$.a[', '$.a[x]', '$a']) assert.equal(logic.parseJsonPath(bad), null, bad);
});

test('uptime is the share of time OK, No Data and time before the test excluded', () => {
  const h = HOUR;
  const r = logic.uptimeOf('alert', 0, [[3 * h, 'ok', 'alert']], 0, 4 * h);
  assert.equal(r.uptime, 75);
  assert.deepEqual(r.segments, [
    { status: 'ok', from: 0, to: 3 * h },
    { status: 'alert', from: 3 * h, to: 4 * h },
  ]);
  assert.equal(logic.uptimeOf('ok', 0, [], h, 2 * h).uptime, 100);
  const mixed = logic.uptimeOf(
    'ok',
    h,
    [
      [2 * h, 'nodata', 'ok'],
      [3 * h, 'ok', 'alert'],
      [3 * h + 30 * MIN, 'alert', 'ok'],
    ],
    0,
    4 * h,
  );
  assert.equal(mixed.uptime, 75);
  assert.equal(mixed.segments[0].from, h);
  assert.equal(logic.uptimeOf('nodata', 0, [], 0, h).uptime, null);
  assert.equal(logic.fmtUptime(99.999), '99.99%');
  assert.equal(logic.fmtUptime(100), '100%');
  assert.equal(logic.fmtUptime(null), '—');
});

test('the search box filters by words, type, env, status and tags', () => {
  const t = { name: 'Checkout API health', domain: 'api.example.com', tags: ['service:order-service'], type: 'http', env: 'prod', team: 'checkout', status: 'alert', paused: false };
  for (const q of ['', 'checkout', 'api.example', 'type:http', 'env:prod', 'team:checkout', 'status:alert', 'service:order-service', 'order']) assert.ok(logic.matchesTestQuery(t, q), q);
  for (const q of ['type:dns', 'env:staging', 'status:ok', 'service:billing', 'inventory']) assert.ok(!logic.matchesTestQuery(t, q), q);
  assert.ok(logic.matchesTestQuery({ ...t, paused: true }, 'status:paused'));
});

test('switching the request type keeps the host and drops assertions that no longer apply', () => {
  const d = {
    ...logic.newDraft('http'),
    request: { ...logic.defaultRequest('http'), url: 'https://shop.example.com/health' },
    assertions: [a('statusCode', 'is', '200'), a('responseTime', 'lessThan', '500')],
  };
  const ssl = logic.withType(d, 'ssl');
  assert.equal(ssl.request.host, 'shop.example.com');
  assert.equal(ssl.request.port, 443);
  assert.deepEqual(
    ssl.assertions.map((x) => x.type),
    ['responseTime'],
  );
  assert.equal(logic.withType(ssl, 'http').request.url, 'https://shop.example.com/');
  assert.equal(logic.requestProblem('http', d.request), null);
  assert.match(logic.requestProblem('http', { ...d.request, url: 'shop.example.com' }), /full URL/);
  assert.match(logic.requestProblem('tcp', { ...ssl.request, port: null }), /port/);
  assert.match(logic.requestProblem('ssl', { ...ssl.request, host: 'https://shop.example.com' }), /without scheme/);
});

test('snippets fill the draft', () => {
  const by = (id) => logic.SNIPPETS.find((s) => s.id === id);
  const base = logic.newDraft('http');
  const health = by('health').apply(base);
  assert.equal(health.request.url, 'https://shop.example.com/health');
  assert.deepEqual(
    health.assertions.map((x) => x.type),
    ['statusCode', 'responseTime'],
  );
  const bearer = by('bearer').apply(by('bearer').apply(base));
  assert.deepEqual(bearer.request.headers, [{ name: 'Authorization', value: 'Bearer <token>' }], 'applied twice, one header');
  assert.deepEqual(by('basic').apply(base).request.basicAuth, { username: '', password: '' });
  const up = by('uptime').apply(base);
  assert.deepEqual([up.frequencySeconds, up.retry.count, up.minFailureDurationMinutes], [60, 1, 2]);
  const cert = by('certificate').apply({ ...base, request: { ...base.request, url: 'https://shop.example.com/' } });
  assert.equal(cert.type, 'ssl');
  assert.equal(cert.request.host, 'shop.example.com');
  assert.deepEqual(cert.assertions, [a('certificate', 'isInMoreThan', '30')]);
});

test('failures and assertions read as sentences', () => {
  assert.equal(logic.assertionText(a('certificate', 'isInMoreThan', '30')), 'certificate expires in more than 30 days');
  assert.equal(logic.assertionText(a('header', 'contains', 'json', 'content-type')), 'header content-type contains json');
  const run = { passed: false, error: null, assertions: [{ assertion: a('statusCode', 'is', '200'), passed: false, actual: '503', error: null }] };
  assert.equal(logic.failureSummary(run), 'Assertion failed: status code is 200 (got 503)');
  assert.equal(logic.failureSummary({ ...run, error: 'Timed out after 30 s' }), 'Timed out after 30 s');
  const suggested = logic.suggestAssertions('http', { statusCode: 201, durationMs: 730, responseHeaders: [{ name: 'Content-Type', value: 'application/json' }], records: [], certificate: null });
  assert.deepEqual(suggested, [a('statusCode', 'is', '201'), a('responseTime', 'lessThan', '1500'), a('header', 'is', 'application/json', 'content-type')]);
});

test('the mock lists tests with uptime, results and an alerting test', async () => {
  const now = Date.now();
  const page = await mock.searchSyntheticTests({ query: '', range: { from: now - 4 * HOUR, to: now } });
  assert.ok(page.tests.length >= 6);
  assert.equal(page.location.id, 'observecraft-server');
  const checkout = page.tests.find((t) => t.name === 'Checkout API health');
  assert.equal(checkout.status, 'alert');
  assert.equal(page.tests[0].status, 'alert', 'worst first');
  assert.ok(checkout.uptime > 90 && checkout.uptime < 100, String(checkout.uptime));
  assert.ok(page.tests.some((t) => t.paused));
  const results = await mock.listSyntheticResults(checkout.id, { from: now - HOUR, to: now });
  assert.ok(results.results.length >= 55 && results.results.length <= 61, String(results.results.length));
  assert.equal(results.results[0].passed, false);
  assert.ok(results.results.some((r) => r.passed));
  const detail = await mock.getSyntheticResult(checkout.id, results.results[0].id);
  assert.equal(detail.detail.statusCode, 503);
  assert.equal(detail.detail.assertions.find((x) => x.assertion.type === 'statusCode').passed, false);
  const filtered = await mock.searchSyntheticTests({ query: 'type:ssl', range: { from: now - HOUR, to: now } });
  assert.deepEqual(
    filtered.tests.map((t) => t.type),
    ['ssl'],
  );
});

test('the mock sends requests, refuses private targets and saves tests', async () => {
  const sent = await mock.trySyntheticRequest({ type: 'http', request: { ...logic.defaultRequest('http'), url: 'https://api.example.com/health' }, assertions: [] });
  assert.equal(sent.result.statusCode, 200);
  assert.deepEqual(
    sent.suggestedAssertions.map((x) => x.type),
    ['statusCode', 'responseTime', 'header'],
  );
  const refused = await mock.trySyntheticRequest({ type: 'tcp', request: { ...logic.defaultRequest('tcp'), host: '10.0.0.5', port: 5432 }, assertions: [] });
  assert.match(refused.result.error, /private/);
  await assert.rejects(mock.createSyntheticTest({ name: ' ', type: 'http', request: logic.defaultRequest('http'), frequencySeconds: 300 }), /name is required/);
  const created = await mock.createSyntheticTest({ ...logic.newDraft('dns'), name: 'Shop DNS', request: { ...logic.defaultRequest('dns'), host: 'shop.example.com' } });
  assert.equal(created.status, 'nodata');
  assert.equal(created.domain, 'shop.example.com');
  const updated = await mock.updateSyntheticTest(created.id, { ...logic.draftOf(created), paused: true });
  assert.equal(updated.paused, true);
  await mock.deleteSyntheticTest(created.id);
  await assert.rejects(mock.getSyntheticTest(created.id), /not found/);
});

test('snippets are grouped by category, like the editor shows them', async () => {
  const { SNIPPETS, SNIPPET_CATEGORIES } = await import('../web/src/api/synthetics/logic.ts');
  assert.ok(SNIPPETS.every((s) => SNIPPET_CATEGORIES.includes(s.category)));
  assert.deepEqual(
    SNIPPET_CATEGORIES.filter((c) => SNIPPETS.some((s) => s.category === c)),
    SNIPPET_CATEGORIES,
  );
});
