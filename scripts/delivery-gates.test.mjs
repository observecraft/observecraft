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
const model = await import('../web/src/pages/delivery/gatesModel.ts');
const mock = await import('../web/src/api/mock/deliveryGates.ts');

/** True when `url` appears in `text` as a whole URL: no URL character right before or after it. */
function hasUrl(text, url) {
  const urlChar = /[A-Za-z0-9._~:/?#[\]@!$&'()*+,;=%-]/;
  for (let at = text.indexOf(url); at !== -1; at = text.indexOf(url, at + 1)) {
    const before = text[at - 1];
    const after = text[at + url.length];
    if ((before === undefined || !urlChar.test(before)) && (after === undefined || !urlChar.test(after))) return true;
  }
  return false;
}

test('monitor rules match tags and name words like the server', () => {
  const tags = ['service:billing', 'env:prod', 'team:payments'];
  assert.ok(model.monitorMatches('service:billing env:prod', 'High latency', tags));
  assert.ok(model.monitorMatches('SERVICE:Billing', 'High latency', tags));
  assert.ok(model.monitorMatches('team:*', 'High latency', tags));
  assert.ok(model.monitorMatches('service:billing latency', 'High latency on checkout', tags));
  assert.ok(!model.monitorMatches('service:billing env:staging', 'High latency', tags));
  assert.ok(!model.monitorMatches('region:*', 'High latency', tags));
  assert.ok(!model.monitorMatches('   ', 'High latency', tags));
});

test('the verdict honours dry runs', () => {
  const r = (status, dryRun) => ({ status, dryRun });
  assert.equal(model.gateVerdict(false, []), 'passed');
  assert.equal(model.gateVerdict(false, [r('passed', false), r('failed', false)]), 'failed');
  assert.equal(model.gateVerdict(false, [r('failed', true)]), 'dry_run');
  assert.equal(model.gateVerdict(true, [r('failed', false)]), 'dry_run');
});

test('every CI/CD tool gets a snippet that calls the evaluation API with the gate', () => {
  const opts = { apiUrl: 'https://observecraft.example.com', service: 'billing', env: 'prod', identifier: 'pre-deploy' };
  const expectedEvaluateUrl = 'https://observecraft.example.com/api/v1/deployment-gates/evaluate';
  assert.deepEqual(
    model.CI_TOOLS.map((t) => t.id),
    ['argo', 'github', 'gitlab', 'jenkins', 'circleci', 'custom'],
  );
  for (const tool of model.CI_TOOLS) {
    const s = model.ciSnippet(tool.id, opts);
    assert.ok(hasUrl(s, expectedEvaluateUrl), tool.id);
    assert.ok(s.includes('billing') && s.includes('prod') && s.includes('pre-deploy'), tool.id);
    assert.ok(/OC_APP_KEY/.test(s), `${tool.id} reads the application key from a secret`);
    assert.ok(!/ocak_[a-z0-9]/i.test(s), `${tool.id} never embeds a key`);
  }
  assert.ok(model.ciSnippet('argo', opts).includes('AnalysisTemplate'));
  assert.ok(model.ciSnippet('github', opts).includes('steps:'));
  assert.ok(model.ciSnippet('gitlab', opts).includes('stage:'));
  assert.ok(model.ciSnippet('jenkins', opts).includes('stage('));
  assert.ok(model.ciSnippet('circleci', opts).includes('jobs:'));
});

test('suggested PR rules prefill the editor and are marked as needing Code Security', () => {
  assert.deepEqual(
    model.PR_SUGGESTIONS.map((s) => s.title),
    ['Code Vulnerabilities', 'Code Quality', 'Library Vulnerabilities', 'License Violations'],
  );
  for (const s of model.PR_SUGGESTIONS) {
    assert.ok(s.description.length > 10);
    assert.equal(model.PR_SOURCES[s.input.source].evaluable, false, s.title);
    assert.equal(s.input.blocking, true);
  }
  assert.equal(model.PR_SOURCES.test_failures.evaluable, true);
  assert.equal(model.PR_SOURCES.flaky_tests.evaluable, true);
  assert.equal(model.PR_SOURCES.pipeline_failures.evaluable, true);
});

test('evaluations filter by text and status', () => {
  const evals = [
    { id: '1', service: 'billing', env: 'prod', identifier: 'default', version: '1.2.0', status: 'failed' },
    { id: '2', service: 'order-service', env: 'staging', identifier: 'canary', version: '3.1.0', status: 'passed' },
    { id: '3', service: 'billing', env: 'staging', identifier: 'default', version: '1.2.0', status: 'dry_run' },
  ];
  assert.deepEqual(
    model.filterEvaluations(evals, '', null).map((e) => e.id),
    ['1', '2', '3'],
  );
  assert.deepEqual(
    model.filterEvaluations(evals, 'billing', null).map((e) => e.id),
    ['1', '3'],
  );
  assert.deepEqual(
    model.filterEvaluations(evals, 'billing staging', null).map((e) => e.id),
    ['3'],
  );
  assert.deepEqual(
    model.filterEvaluations(evals, '', 'failed').map((e) => e.id),
    ['1'],
  );
});

test('the mock keeps gates and evaluates them', async () => {
  const gates = await mock.listGates();
  assert.ok(gates.length > 0, 'demo gates for make web-mock');
  const gate = await mock.createGate({ service: 'catalog-service', env: 'prod', identifier: '', dryRun: false, rules: [{ type: 'monitor', query: 'service:catalog-service' }] });
  assert.equal(gate.identifier, 'default');
  assert.equal(gate.rules[0].name.length > 0, true);
  await assert.rejects(() => mock.createGate({ service: 'catalog-service', env: 'prod', identifier: 'default', dryRun: false, rules: [{ type: 'monitor', query: 'x:y' }] }), /already exists/);
  await assert.rejects(() => mock.createGate({ service: '', env: 'prod', identifier: '', dryRun: false, rules: [] }));
  const ev = await mock.evaluateGate({ service: 'catalog-service', env: 'prod' });
  assert.ok(['passed', 'failed', 'dry_run'].includes(ev.status));
  assert.equal(ev.pass, ev.status !== 'failed');
  assert.equal((await mock.listEvaluations())[0].id, ev.id, 'newest first');
  const detail = await mock.getEvaluation(ev.id);
  for (const m of detail.rules.flatMap((r) => r.monitors ?? [])) assert.ok('currentStatus' in m);
  await assert.rejects(() => mock.evaluateGate({ service: 'nope', env: 'prod' }), /No deployment gate/);
  await mock.deleteGate(gate.id);
  assert.ok(!(await mock.listGates()).some((g) => g.id === gate.id));
});

test('the mock PR gates start empty and evaluate commits without faking Code Security', async () => {
  assert.equal((await mock.listPrRules()).length, 0, 'the first-run page shows in make web-mock');
  const tests = await mock.createPrRule({ name: 'No failing tests', source: 'test_failures', repositories: [], threshold: 0, minSeverity: 'high', blocking: true, enabled: true });
  await mock.createPrRule({ ...model.PR_SUGGESTIONS[0].input, name: 'Vulnerabilities' });
  await assert.rejects(
    () => mock.createPrRule({ name: 'no failing TESTS', source: 'flaky_tests', repositories: [], threshold: 0, minSeverity: 'high', blocking: true, enabled: true }),
    /already exists/,
  );
  const result = await mock.evaluatePr({ repository: 'example-org/billing', sha: 'a1b2c3d4e5f6' });
  assert.equal(result.rules.length, 2);
  assert.equal(result.rules.find((r) => r.name === 'Vulnerabilities').status, 'not_evaluated');
  assert.equal(result.pass, result.status === 'passed');
  await mock.updatePrRule(tests.id, { ...tests, enabled: false });
  assert.equal((await mock.evaluatePr({ repository: 'example-org/billing', sha: 'a1b2c3d4e5f6' })).rules.length, 1, 'disabled rules are skipped');
  for (const r of await mock.listPrRules()) await mock.deletePrRule(r.id);
});
