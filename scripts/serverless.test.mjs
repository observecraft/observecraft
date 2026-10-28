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
const mock = await import('../web/src/api/mock/serverless.ts');
const model = await import('../web/src/pages/serverless/serverlessModel.ts');

const HOUR = 3_600_000;
const range = { from: 1_760_000_000_000 - HOUR, to: 1_760_000_000_000 };

test('cost estimates follow the list prices, billed per started millisecond', () => {
  const x86 = mock.estimateCost(1e6, 100, 1024);
  assert.ok(Math.abs(x86 - (0.2 + 100_000 * 0.0000166667)) < 1e-9);
  assert.ok(mock.estimateCost(1e6, 100, 1024, 'arm64') < x86);
  assert.equal(mock.estimateCost(1, 0.2, 1024), mock.estimateCost(1, 1, 1024));
});

test('runtime upgrades match the API', () => {
  assert.equal(mock.runtimeUpgrade('nodejs18.x'), 'nodejs24.x');
  assert.equal(mock.runtimeUpgrade('nodejs24.x'), null);
  assert.equal(mock.runtimeUpgrade('python3.9'), 'python3.14');
  assert.equal(mock.runtimeUpgrade('python3.14'), null);
  assert.equal(mock.runtimeUpgrade('java8.al2'), 'java25');
  assert.equal(mock.runtimeUpgrade('go1.x'), 'provided.al2023');
  assert.equal(mock.runtimeUpgrade('provided.al2'), 'provided.al2023');
  assert.equal(mock.runtimeUpgrade('provided.al2023'), null);
  assert.equal(mock.runtimeUpgrade('nodejs26.x'), null);
  assert.equal(mock.runtimeUpgrade('cobol'), null);
});

test('insight thresholds', () => {
  const base = { invocations: 100, errors: 0, errorRatePct: 0, coldStartPct: 1, timeouts: 0, outOfMemory: 0, throttles: 0, memoryUsedPct: 50, memoryMb: 1024 };
  const kinds = (patch) => mock.detectInsights({ ...base, ...patch }).map((i) => i.kind);
  assert.deepEqual(kinds({}), []);
  assert.deepEqual(kinds({ errors: 8, errorRatePct: 8 }), ['high_errors']);
  assert.deepEqual(kinds({ coldStartPct: 12 }), ['cold_starts']);
  assert.deepEqual(kinds({ invocations: 5, coldStartPct: 60 }), [], 'too few invocations');
  assert.deepEqual(kinds({ memoryUsedPct: 95 }), ['high_memory']);
  assert.deepEqual(kinds({ memoryUsedPct: 99, outOfMemory: 2 }), ['out_of_memory'], 'out of memory, not just high');
  assert.deepEqual(kinds({ memoryUsedPct: 10 }), ['over_provisioned']);
  assert.deepEqual(kinds({ memoryUsedPct: 10, memoryMb: 128 }), []);
});

test('the demo fleet: fictional functions with insights, upgrades and costs', async () => {
  const page = await mock.searchFunctions({ range, query: '' });
  assert.equal(page.enabled, true);
  assert.equal(page.functions.length, 10);
  for (const f of page.functions) {
    assert.equal(f.account, '123456789012');
    assert.ok(f.arn.startsWith(`arn:aws:lambda:${f.region}:123456789012:function:`));
    assert.ok(f.costUsd > 0);
  }
  const names = page.functions.map((f) => f.name);
  for (const n of ['checkout-webhook', 'order-confirmation-mailer', 'inventory-sync', 'image-resizer']) assert.ok(names.includes(n), n);
  const insight = page.facets.find((f) => f.key === 'insight').values.map((v) => v.value);
  for (const k of ['high_errors', 'cold_starts', 'timeouts', 'out_of_memory', 'throttles', 'high_memory', 'over_provisioned']) assert.ok(insight.includes(k), k);
  const resizer = page.functions.find((f) => f.name === 'image-resizer');
  assert.deepEqual(
    resizer.upgrades.map((u) => [u.component, u.latest]),
    [
      ['runtime', 'nodejs24.x'],
      ['datadog_lambda', 'v10.12.1'],
      ['extension', '88'],
    ],
  );
  assert.equal(page.summary.upgradable, page.functions.filter((f) => f.upgrades.length).length);
});

test('query, insight terms and the upgrade toggle filter; facets keep the whole fleet', async () => {
  const eu = await mock.searchFunctions({ range, query: 'region:eu-west-1' });
  assert.deepEqual(eu.functions.map((f) => f.name).sort(), ['inventory-reservation-expirer', 'inventory-sync']);
  assert.equal(eu.facets.find((f) => f.key === 'functionname').values.length, 10);
  const timeouts = await mock.searchFunctions({ range, query: 'insight:timeouts' });
  assert.deepEqual(
    timeouts.functions.map((f) => f.name),
    ['inventory-sync'],
  );
  const notTimeouts = await mock.searchFunctions({ range, query: '-insight:timeouts' });
  assert.equal(notTimeouts.functions.length, 9);
  const text = await mock.searchFunctions({ range, query: 'mailer' });
  assert.deepEqual(
    text.functions.map((f) => f.name),
    ['order-confirmation-mailer'],
  );
  const upgrades = await mock.searchFunctions({ range, query: '', upgradeAvailable: true });
  assert.ok(upgrades.functions.length > 0 && upgrades.functions.length < 10);
  assert.ok(upgrades.functions.every((f) => f.upgrades.length > 0));
});

test('series per charted function, invocations of a function', async () => {
  const series = await mock.serverlessTimeseries({ range, functions: ['us-east-1/checkout-webhook', 'eu-west-1/inventory-sync', 'nowhere/none'] });
  const inv = series.filter((s) => s.metric === 'invocations');
  assert.deepEqual(
    inv.map((s) => s.function),
    ['us-east-1/checkout-webhook', 'eu-west-1/inventory-sync'],
  );
  assert.ok(inv[0].points.length >= 50);
  assert.ok(series.some((s) => s.metric === 'duration' && s.unit === 'millisecond'));
  const calls = await mock.functionInvocations({ range, function: 'checkout-webhook' });
  assert.equal(calls.length, 25);
  assert.ok(calls.every((c) => c.traceId.length === 32 && c.trigger === 'api-gateway'));
  assert.deepEqual(await mock.functionInvocations({ range, function: 'inventory-reservation-expirer' }), [], 'not traced');
});

test('source filters and grouping by service', async () => {
  const { functions } = await mock.searchFunctions({ range, query: '' });
  assert.equal(model.applySourceFilters(functions, model.NO_SOURCE_FILTERS).length, 10);
  const noTraces = model.applySourceFilters(functions, { ...model.NO_SOURCE_FILTERS, traces: 'without' });
  assert.deepEqual(
    noTraces.map((f) => f.name),
    ['inventory-reservation-expirer'],
  );
  const enhancedNoLogs = model.applySourceFilters(functions, { enhanced: 'with', traces: 'any', logs: 'without' });
  assert.equal(enhancedNoLogs.length, 0);
  const groups = model.groupByService([...functions, { ...functions[0], id: 'x/solo', name: 'solo', service: null, invocations: 1 }]);
  assert.equal(groups[groups.length - 1].service, model.NO_SERVICE);
  for (let i = 1; i < groups.length; i++) assert.ok(groups[i - 1].invocations >= groups[i].invocations, 'busiest group first');
  const checkout = groups.find((g) => g.service === 'checkout');
  assert.deepEqual(
    checkout.functions.map((f) => f.name),
    ['checkout-webhook', 'payment-confirmation'],
  );
  assert.equal(checkout.invocations, checkout.functions[0].invocations + checkout.functions[1].invocations);
});

test('insight cards: most severe first', async () => {
  const { functions } = await mock.searchFunctions({ range, query: '' });
  const cards = model.insightCards(functions);
  assert.equal(
    cards.length,
    functions.reduce((a, f) => a + f.insights.length, 0),
  );
  const severities = cards.map((c) => model.INSIGHT_INFO[c.kind].severity);
  assert.deepEqual(
    severities,
    [...severities].sort((a, b) => ['error', 'warn', 'info'].indexOf(a) - ['error', 'warn', 'info'].indexOf(b)),
  );
  assert.equal(model.formatInsightValue('cold_starts', 14.25), '14.3 %');
  assert.equal(model.formatInsightValue('timeouts', 1200), '1,200');
});

test('formatting', () => {
  assert.equal(model.formatCount(null), '—');
  assert.equal(model.formatCount(950), '950');
  assert.equal(model.formatCount(12_345), '12.3k');
  assert.equal(model.formatCount(2_500_000), '2.5M');
  assert.equal(model.formatMs(12.34), '12.3 ms');
  assert.equal(model.formatMs(380), '380 ms');
  assert.equal(model.formatMs(2400), '2.40 s');
  assert.equal(model.formatCost(0.004), '< $0.01');
  assert.equal(model.formatCost(1234.5), '$1,234.50');
  assert.equal(model.formatMemory(512), '512 MB');
  assert.equal(model.formatMemory(3008), '2.9 GB');
  assert.equal(model.formatMemory(2048), '2 GB');
});
