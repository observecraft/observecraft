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
const logic = await import('../web/src/api/featureFlags/logic.ts');
const mock = await import('../web/src/api/mock/featureFlags.ts');
const { fill } = await import('../web/src/pages/setup/sources.ts');

/** The origins (scheme + host + port) of the URLs written in `text`. */
function urlOrigins(text) {
  return (text.match(/https?:\/\/[^\s'"`)]+/g) ?? []).map((u) => new URL(u).origin);
}

test('flag keys are derived from names in kebab-case', () => {
  assert.equal(logic.flagKeyFromName('New Payment Flow'), 'new-payment-flow');
  assert.equal(logic.flagKeyFromName('  Catalog search: v2!! '), 'catalog-search-v2');
  assert.equal(logic.flagKeyFromName('Ünïcode & dots.v3'), 'unicode-dots-v3');
  assert.equal(logic.flagKeyFromName('---'), '');
  assert.equal(logic.flagKeyFromName('a'.repeat(150)).length, 100);
  assert.ok(logic.validKey('checkout-new-payment-flow'));
  assert.ok(logic.validKey('catalog.search_v2'));
  assert.ok(!logic.validKey('Upper'));
  assert.ok(!logic.validKey('-lead'));
});

test('variant values are parsed per type', () => {
  assert.deepEqual(logic.parseVariantValue('integer', ' 42 '), { value: 42 });
  assert.match(logic.parseVariantValue('integer', '4.2').error, /whole number/);
  assert.deepEqual(logic.parseVariantValue('number', '0.25'), { value: 0.25 });
  assert.match(logic.parseVariantValue('number', 'abc').error, /number/);
  assert.deepEqual(logic.parseVariantValue('string', 'blue'), { value: 'blue' });
  assert.deepEqual(logic.parseVariantValue('json', '{"limit": 3}'), { value: { limit: 3 } });
  assert.match(logic.parseVariantValue('json', '{oops').error, /JSON/);
  assert.match(logic.parseVariantValue('json', '3').error, /object or an array/);
  assert.equal(logic.formatVariantValue('json', { a: 1 }), '{\n  "a": 1\n}');
  assert.equal(logic.formatVariantValue('integer', 3), '3');
  // Boolean flags come with their two fixed variants.
  assert.deepEqual(
    logic.defaultVariants('boolean').map((v) => v.value),
    [true, false],
  );
});

test('a flag draft names the steps left to complete', () => {
  const d = logic.newFlagDraft();
  assert.deepEqual(logic.draftProblems(d), [1]);
  d.name = 'New payment flow';
  assert.deepEqual(logic.draftProblems(d), []);
  const typed = logic.withFlagType(d, 'integer');
  assert.equal(typed.variants.length, 2);
  typed.variants[1].text = 'x';
  assert.deepEqual(logic.draftProblems(typed), [2]);
  typed.variants[1].text = '7';
  typed.variants[1].key = typed.variants[0].key;
  assert.deepEqual(logic.draftProblems(typed), [2]);
  typed.variants[1].key = 'high';
  const input = logic.inputFromDraft(typed);
  assert.equal(input.key, 'new-payment-flow');
  assert.equal(input.flagType, 'integer');
  assert.equal(input.defaultVariant, typed.variants[0].key);
  assert.deepEqual(
    input.variants.map((v) => v.value),
    [typed.variants[0].text === '' ? 0 : Number(typed.variants[0].text), 7],
  );
  // An explicit key wins over the derived one.
  assert.equal(logic.inputFromDraft({ ...typed, key: 'payments.flow-v2' }).key, 'payments.flow-v2');
});

test('splits and rollouts', () => {
  assert.equal(
    logic.splitTotal([
      { variant: 'a', weight: 33.33 },
      { variant: 'b', weight: 66.67 },
    ]),
    100,
  );
  assert.ok(
    logic.splitValid([
      { variant: 'a', weight: 50 },
      { variant: 'b', weight: 50 },
    ]),
  );
  assert.ok(
    !logic.splitValid([
      { variant: 'a', weight: 50 },
      { variant: 'b', weight: 40 },
    ]),
  );
  assert.equal(logic.serveLabel({ variant: 'on' }), 'on');
  assert.equal(
    logic.serveLabel({
      split: [
        { variant: 'on', weight: 10 },
        { variant: 'off', weight: 90 },
      ],
    }),
    'on 10% · off 90%',
  );
  const r = {
    variant: 'on',
    steps: [
      { percent: 10, durationMinutes: 60 },
      { percent: 100, durationMinutes: 0 },
    ],
    startedAt: 1_000_000,
  };
  assert.equal(logic.rolloutPercent(r, 999_999), 0);
  assert.equal(logic.rolloutPercent(r, 1_000_000), 10);
  assert.equal(logic.rolloutPercent(r, 1_000_000 + 60 * 60_000), 100);
  assert.equal(logic.rolloutPercent({ ...r, startedAt: 0 }, 5_000_000), 0);
  assert.equal(logic.rolloutStepIndex(r, 1_000_000 + 30 * 60_000), 0);
  assert.equal(logic.rolloutStepIndex(r, 1_000_000 + 61 * 60_000), 1);
});

test('environment queries match like the intake', () => {
  assert.ok(logic.envMatches('production', 'Production'));
  assert.ok(logic.envMatches('staging-*', 'staging-eu'));
  assert.ok(logic.envMatches('*-local', 'dev-local'));
  assert.ok(!logic.envMatches('prod', 'production'));
  assert.ok(!logic.envMatches('', 'x'));
});

test('evaluation totals from per-second rates', () => {
  // Points are per-second rates over their interval (here a minute).
  assert.equal(
    logic.totalFromRates(
      [
        [0, 0.5],
        [60_000, 1],
        [120_000, 0],
      ],
      60_000,
    ),
    90,
  );
  // Empty intervals are left out of a series: one point is still a count.
  assert.equal(logic.totalFromRates([[0, 0.0072]], 1_800_000), 13);
  assert.equal(logic.totalFromRates([], 60_000), 0);
  // The interval the API uses (a day in 48 points → 30 minutes).
  assert.equal(logic.metricStepMs({ from: 0, to: 86_400_000 }, 48), 1_800_000);
  assert.equal(logic.metricStepMs({ from: 0, to: 3_600_000 }, 48), 120_000);
  assert.equal(logic.fmtCount(1234), '1.2K');
  assert.equal(logic.fmtCount(12), '12');
  assert.equal(logic.fmtCount(3_400_000), '3.4M');
});

test('flag search and facets', () => {
  const flags = [
    {
      key: 'checkout-new-payment-flow',
      name: 'New payment flow',
      flagType: 'boolean',
      teams: ['payments'],
      tags: ['service:storefront-api'],
      lifecycle: 'temporary',
      environments: [{ config: { enabled: true } }],
    },
    { key: 'catalog-search-v2', name: 'Catalog search v2', flagType: 'string', teams: ['search'], tags: [], lifecycle: 'permanent', environments: [{ config: { enabled: false } }] },
  ];
  assert.deepEqual(
    logic.filterFlags(flags, 'payment', {}).map((f) => f.key),
    ['checkout-new-payment-flow'],
  );
  assert.deepEqual(
    logic.filterFlags(flags, 'service:storefront-api', {}).map((f) => f.key),
    ['checkout-new-payment-flow'],
  );
  assert.deepEqual(
    logic.filterFlags(flags, '', { type: ['string'] }).map((f) => f.key),
    ['catalog-search-v2'],
  );
  assert.deepEqual(
    logic.filterFlags(flags, '', { status: ['off'] }).map((f) => f.key),
    ['catalog-search-v2'],
  );
});

test('OpenFeature snippets point at this install', () => {
  const values = { intake: 'https://intake.example.com', apiKey: '<API_KEY>' };
  const s = logic.ofrepSnippets('checkout-new-payment-flow', 'boolean').map((x) => ({ ...x, code: fill(x.code, values) }));
  assert.deepEqual(
    s.map((x) => x.id),
    ['browser', 'node', 'go', 'python'],
  );
  for (const x of s) {
    assert.ok(urlOrigins(x.code).some((origin) => origin === 'https://intake.example.com'), x.id);
    assert.ok(x.code.includes('<API_KEY>'), x.id);
    assert.ok(x.code.includes('checkout-new-payment-flow'), x.id);
  }
  assert.ok(s[0].code.includes('@openfeature/ofrep-web-provider'));
  assert.ok(s[1].code.includes('getBooleanValue'));
  assert.ok(logic.ofrepSnippets('catalog-search-v2', 'string')[2].code.includes('StringValue'));
});

test('the mock keeps versions, history and validation like the server', async () => {
  mock.resetFeatureFlags();
  const page = await mock.listFeatureFlags();
  assert.ok(page.flags.some((f) => f.key === 'checkout-new-payment-flow'));
  assert.ok(page.flags.some((f) => f.key === 'catalog-search-v2'));
  assert.ok(page.environments.some((e) => e.isProduction));
  const flag = page.flags.find((f) => f.key === 'checkout-new-payment-flow');
  const prod = flag.environments.find((e) => e.isProduction);
  const next = await mock.updateFeatureFlagEnvironment(flag.id, prod.environmentId, { version: flag.version, config: { ...prod.config, enabled: !prod.config.enabled } });
  assert.equal(next.version, flag.version + 1);
  await assert.rejects(mock.updateFeatureFlagEnvironment(flag.id, prod.environmentId, { version: flag.version, config: prod.config }), /changed by someone else/);
  const history = await mock.listFeatureFlagHistory(flag.id);
  assert.match(history[0].action, /enabled|disabled/);
  await assert.rejects(
    mock.createFeatureFlag({
      key: 'checkout-new-payment-flow',
      name: 'x',
      flagType: 'boolean',
      variants: logic.defaultVariants('boolean'),
      defaultVariant: 'false',
      sdkAccess: 'all',
      lifecycle: 'temporary',
    }),
    /already exists/,
  );
  const created = await mock.createFeatureFlag({
    key: 'order-service-retry-budget',
    name: 'Retry budget',
    flagType: 'integer',
    variants: [
      { key: 'low', value: 1 },
      { key: 'high', value: 5 },
    ],
    defaultVariant: 'low',
    sdkAccess: 'server',
    lifecycle: 'permanent',
  });
  assert.ok(created.environments.every((e) => !e.config.enabled && e.config.defaultServe.variant === 'low'));
  await assert.rejects(
    mock.updateFeatureFlagEnvironment(created.id, created.environments[0].environmentId, { version: created.version, config: { enabled: true, rules: [], defaultServe: { variant: 'nope' } } }),
    /not a variant/,
  );
  const env = await mock.createFeatureFlagEnvironment({ name: 'QA', queries: ['qa-*'], isProduction: false, requiresApproval: false });
  assert.equal(env.name, 'QA');
  await assert.rejects(mock.createFeatureFlagEnvironment({ name: 'qa', queries: [] }), /already exists/);
  await mock.deleteFeatureFlag(created.id);
  assert.ok(!(await mock.listFeatureFlags()).flags.some((f) => f.id === created.id));
  const evals = await mock.featureFlagEvaluations({ from: Date.now() - 86_400_000, to: Date.now() });
  assert.ok(evals.find((e) => e.flag === 'checkout-new-payment-flow').total > 0);
});

test('list toolbar filters: mine, team, stale and production status', async () => {
  const { toolbarFilter } = await import('../web/src/api/featureFlags/logic.ts');
  const NOW = Date.UTC(2026, 9, 28);
  const DAY = 86_400_000;
  const env = (isProduction, enabled) => ({ environmentId: isProduction ? 'p' : 's', environmentName: isProduction ? 'Production' : 'Staging', isProduction, config: { enabled } });
  const flags = [
    { key: 'a', createdBy: 'Alex Morgan', teams: ['payments'], lifecycle: 'temporary', updatedAt: NOW - 40 * DAY, environments: [env(true, true), env(false, true)] },
    { key: 'b', createdBy: 'Sam Lee', teams: ['search'], lifecycle: 'temporary', updatedAt: NOW - 2 * DAY, environments: [env(true, false)] },
    { key: 'c', createdBy: 'Alex Morgan', teams: [], lifecycle: 'permanent', updatedAt: NOW - 90 * DAY, environments: [env(true, false)] },
  ];
  const keys = (f) => toolbarFilter(flags, f, 'Alex Morgan', NOW).map((x) => x.key);
  assert.deepEqual(keys({}), ['a', 'b', 'c']);
  assert.deepEqual(keys({ mine: true }), ['a', 'c']);
  assert.deepEqual(keys({ team: 'search' }), ['b']);
  // Stale: temporary flags untouched for 30 days; permanent flags never are.
  assert.deepEqual(keys({ stale: true }), ['a']);
  assert.deepEqual(keys({ prod: 'on' }), ['a']);
  assert.deepEqual(keys({ prod: 'off' }), ['b', 'c']);
});
