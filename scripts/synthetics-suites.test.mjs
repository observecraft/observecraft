import assert from 'node:assert/strict';
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
});
const model = await import('../web/src/api/synthetics/model.ts');
const logic = await import('../web/src/api/synthetics/logic.ts');
const mock = await import('../web/src/api/mock/synthetics.ts');

const HOUR = 3_600_000;

test('a suite is as bad as its worst running test', () => {
  const t = (status, paused = false) => ({ status, paused });
  assert.equal(model.suiteStatus([]), 'nodata');
  assert.equal(model.suiteStatus([t('ok'), t('nodata')]), 'ok');
  assert.equal(model.suiteStatus([t('ok'), t('alert'), t('warn')]), 'alert');
  assert.equal(model.suiteStatus([t('ok'), t('warn')]), 'warn');
  assert.equal(model.suiteStatus([t('ok'), t('alert', true)]), 'ok');
  assert.equal(model.suiteStatus([t('alert', true)]), 'nodata');
});

test('the suites search box matches like the server', () => {
  const s = { name: 'Checkout journey', description: 'Cart, payment and order confirmation', env: 'prod', team: 'checkout', tags: ['service:order-service', 'tier:critical'], status: 'alert' };
  for (const q of ['', 'checkout', 'PAYMENT', 'env:prod', 'team:checkout', 'status:alert', 'service:order-service', 'critical', 'journey env:prod']) assert.ok(model.matchesSuiteQuery(s, q), q);
  for (const q of ['env:staging', 'team:billing', 'status:ok', 'service:billing', 'inventory']) assert.ok(!model.matchesSuiteQuery(s, q), q);
  // The list's facets add unions and negations.
  for (const q of ['team:(billing OR checkout)', '-env:staging', '"Checkout journey"', 'tier:critical status:(alert OR warn)']) assert.ok(model.matchesSuiteTerms(s, q), q);
  for (const q of ['team:(billing OR catalog)', '-env:prod', '-checkout']) assert.ok(!model.matchesSuiteTerms(s, q), q);
});

test('results queries filter like the server', () => {
  const r = {
    testId: 'abc',
    testName: 'Checkout API health',
    testType: 'http',
    location: 'observecraft-server',
    passed: false,
    statusCode: 503,
    error: null,
    attempts: 2,
    durationMs: 320,
    trigger: 'scheduled',
    env: 'prod',
    team: 'checkout',
    tags: ['service:order-service'],
  };
  const yes = [
    '',
    '@type:result',
    'status:failed',
    '@test.type:http',
    '@test.name:checkout',
    '@test.name:Checkout*',
    '@http.status_code:>=400',
    '@http.status_code:5*',
    '@result.retries:>0',
    '@result.duration:>300',
    '-status:passed',
    '@test.type:(ssl OR http)',
    'health',
    'service:order-service',
    'env:prod team:checkout',
    'trigger:scheduled',
    '@test.id:abc',
    'status:failed AND health',
  ];
  for (const q of yes) assert.ok(model.matchesResultQuery(r, q), q);
  const no = ['type:batch', 'status:passed', '@http.status_code:<400', '@http.status_code:2*', '@result.retries:0', '-@test.type:http', 'timeout', 'service:billing', 'trigger:manual'];
  for (const q of no) assert.ok(!model.matchesResultQuery(r, q), q);
  assert.ok(!model.matchesResultQuery({ ...r, statusCode: null }, '@http.status_code:>=400'));
  assert.ok(model.matchesResultQuery({ ...r, statusCode: null }, '-@http.status_code:>=400'), 'negating a missing value matches');
  assert.throws(() => model.matchesResultQuery(r, 'status:maybe'), /passed or failed/);
  assert.throws(() => model.matchesResultQuery(r, '@result.duration:>slow'), /number/);
});

test('recommended queries parse and say what they find', () => {
  assert.equal(model.RECOMMENDED_QUERIES.length, 4);
  for (const q of model.RECOMMENDED_QUERIES) {
    assert.ok(q.description.length > 20, q.title);
    model.matchesResultQuery(
      { testName: '', testType: 'http', location: '', passed: true, statusCode: 200, error: null, attempts: 1, durationMs: 1, trigger: 'scheduled', env: '', team: '', tags: [], testId: '' },
      q.query,
    );
  }
});

test('coverage matches test URLs to APM endpoints', () => {
  assert.deepEqual(model.endpointOf('GET /orders/{id}', '', ''), ['GET', '/orders/{id}']);
  assert.deepEqual(model.endpointOf('http.request', '/orders/{id}', 'get'), ['GET', '/orders/{id}']);
  assert.equal(model.endpointOf('process-payment', '', 'POST'), null);
  assert.ok(model.routeMatches('/orders/{id}', '/orders/42'));
  assert.ok(model.routeMatches('/orders/:id/items', '/orders/42/items/'));
  assert.ok(model.routeMatches('/orders/?', '/orders/42'));
  assert.ok(model.routeMatches('/static/*', '/static/css/app.css'));
  assert.ok(!model.routeMatches('/orders/{id}', '/orders'));
  assert.ok(!model.routeMatches('/api/products', '/api/orders'));
  assert.deepEqual(model.testTarget('get', 'https://shop.example.com/api/products?page=2'), ['GET', '/api/products']);
  assert.deepEqual(model.testTarget('POST', 'https://{{ HOST }}/api/orders'), ['POST', '/api/orders']);
  assert.equal(model.testTarget('GET', 'nope'), null);
});

test('global variables fill requests and assertions', () => {
  assert.deepEqual(model.placeholders('https://{{ HOST }}/a?k={{KEY}}&again={{ HOST }} {{ lower }}'), ['HOST', 'KEY']);
  const request = { ...logic.defaultRequest('http'), url: 'https://{{ HOST }}/x', headers: [{ name: 'Authorization', value: 'Bearer {{ TOKEN }}' }] };
  const out = model.substitute(request, [{ type: 'body', operator: 'contains', property: '', target: '{{ HOST }}' }], { HOST: 'shop.example.com', TOKEN: 't' });
  assert.equal(out.request.url, 'https://shop.example.com/x');
  assert.equal(out.request.headers[0].value, 'Bearer t');
  assert.equal(out.assertions[0].target, 'shop.example.com');
  assert.throws(() => model.substitute(request, [], {}), /Unknown global variables: HOST, TOKEN/);
  assert.ok(model.validVariableName('API_TOKEN') && !model.validVariableName('api_token') && !model.validVariableName('2FA'));
  assert.equal(logic.requestProblem('http', request), null, 'the editor accepts variables in the URL');
  assert.equal(logic.requestProblem('ssl', { ...logic.defaultRequest('ssl'), host: '{{ HOST }}' }), null);
  assert.match(logic.requestProblem('http', { ...request, url: '{{ BASE }}/health' }), /full URL/);
  assert.equal(logic.testDomain('http', request), '{{ HOST }}');
});

test('enforced tag rules', () => {
  const s = { enforceTagKeys: true, enforceEnv: true, defaults: { tags: ['team:', 'service:storefront-api'] } };
  assert.equal(model.enforcementProblem(s, ['team:checkout'], 'prod'), 'Tests need a tag for each of: service');
  assert.equal(model.enforcementProblem(s, ['team:a', 'service:b'], ''), 'Tests need an environment (env)');
  assert.equal(model.enforcementProblem(s, ['team:a', 'service:b'], 'prod'), null);
  assert.equal(model.enforcementProblem({ ...s, enforceTagKeys: false, enforceEnv: false }, [], ''), null);
});

test('the editor starts new tests from the default settings', () => {
  const d = logic.newDraft('http', {
    locations: ['observecraft-server'],
    tags: ['team:web'],
    timeoutSeconds: 10,
    frequencySeconds: 900,
    retry: { count: 2, intervalMs: 500 },
    minFailureDurationMinutes: 5,
  });
  assert.deepEqual([d.frequencySeconds, d.retry.count, d.retry.intervalMs, d.minFailureDurationMinutes, d.request.timeoutSeconds], [900, 2, 500, 5, 10]);
  assert.deepEqual(d.tags, ['team:web']);
  assert.equal(logic.newDraft('http').frequencySeconds, 300);
});

test('CSV exports quote their cells', () => {
  const csv = model.toCsv(
    ['NAME', 'TAGS'],
    [
      ['Checkout, journey', 'a "b"'],
      ['plain', ''],
    ],
  );
  assert.equal(csv, 'NAME,TAGS\n"Checkout, journey","a ""b"""\nplain,\n');
});

test('the mock keeps suites with their state and uptime', async () => {
  const now = Date.now();
  const range = { from: now - 4 * HOUR, to: now };
  const page = await mock.searchSyntheticSuites({ query: '', range });
  assert.ok(page.suites.length >= 3);
  const checkout = page.suites.find((s) => s.name === 'Checkout journey');
  assert.equal(checkout.status, 'alert');
  assert.ok(checkout.uptime > 50 && checkout.uptime < 100, String(checkout.uptime));
  const tests = (await mock.searchSyntheticTests({ query: '', range })).tests;
  const created = await mock.createSyntheticSuite({
    name: 'Storefront smoke',
    tags: ['team:storefront'],
    env: 'prod',
    team: 'storefront',
    description: '',
    testIds: [tests.find((t) => t.name === 'Storefront DNS').id],
  });
  assert.equal(created.status, 'ok');
  assert.equal(created.tests.length, 1);
  const updated = await mock.updateSyntheticSuite(created.id, { name: 'Storefront smoke', tags: [], env: 'qa', team: '', description: 'x', testIds: [] });
  assert.equal(updated.status, 'nodata');
  assert.equal((await mock.searchSyntheticSuites({ query: 'env:qa', range })).suites.length, 1);
  await mock.deleteSyntheticSuite(created.id);
  await assert.rejects(mock.getSyntheticSuite(created.id), /not found/);
  await assert.rejects(mock.createSyntheticSuite({ name: ' ', testIds: [] }), /name is required/);
});

test('the mock explorer searches every test result', async () => {
  const now = Date.now();
  const range = { from: now - HOUR, to: now };
  const all = await mock.searchSyntheticResultsAll({ query: '', range, offset: 0, limit: 50 });
  assert.ok(all.total > 60, String(all.total));
  assert.equal(all.results.length, 50);
  assert.ok(all.results[0].startedAt >= all.results[49].startedAt, 'newest first');
  assert.ok(all.series.length > 0 && all.bucketMs > 0);
  assert.ok(all.groups.length >= 4);
  assert.ok(all.facets.find((f) => f.key === 'status').values.length >= 1);
  const failed = await mock.searchSyntheticResultsAll({ query: 'status:failed @http.status_code:>=400', range, offset: 0, limit: 50 });
  assert.ok(failed.total > 0);
  assert.ok(failed.results.every((r) => !r.passed && r.statusCode >= 400));
  assert.equal(
    failed.series.reduce((n, p) => n + p.passed, 0),
    0,
  );
  await assert.rejects(mock.searchSyntheticResultsAll({ query: 'status:maybe', range, offset: 0, limit: 10 }), /passed or failed/);
});

test('the mock computes coverage from APM endpoints and keeps settings and variables', async () => {
  const now = Date.now();
  const cov = await mock.getSyntheticCoverage({ range: { from: now - HOUR, to: now }, env: null });
  assert.ok(cov.endpoints.length >= 3);
  assert.ok(
    cov.endpoints.some((e) => e.tests.length > 0),
    'a demo test covers an endpoint',
  );
  assert.ok(cov.endpoints.some((e) => e.tests.length === 0));
  const settings = await mock.getSyntheticSettings();
  const saved = await mock.updateSyntheticSettings({ ...settings, enforceEnv: true, defaults: { ...settings.defaults, frequencySeconds: 900 } });
  assert.equal(saved.defaults.frequencySeconds, 900);
  assert.equal((await mock.getSyntheticSettings()).enforceEnv, true);
  await assert.rejects(mock.createSyntheticTest({ ...logic.newDraft('dns'), name: 'No env', request: { ...logic.defaultRequest('dns'), host: 'shop.example.com' } }), /environment/);
  await mock.updateSyntheticSettings({ ...saved, enforceEnv: false });
  const vars = await mock.listSyntheticVariables();
  assert.ok(
    vars.some((v) => v.secure && v.value === null),
    'secure values are not returned',
  );
  const v = await mock.createSyntheticVariable({ name: 'SHOP_API_KEY', value: 'k-123', secure: true, description: '', tags: [] });
  assert.equal(v.value, null);
  await assert.rejects(mock.createSyntheticVariable({ name: 'SHOP_API_KEY', value: 'x', secure: false, description: '', tags: [] }), /exists/);
  await assert.rejects(mock.createSyntheticVariable({ name: 'bad name', value: 'x', secure: false, description: '', tags: [] }), /letters/);
  const renamed = await mock.updateSyntheticVariable(v.id, { name: 'SHOP_KEY', value: null, secure: true, description: 'd', tags: [] });
  assert.equal(renamed.name, 'SHOP_KEY');
  await mock.deleteSyntheticVariable(v.id);
  assert.ok(!(await mock.listSyntheticVariables()).some((x) => x.id === v.id));
});
