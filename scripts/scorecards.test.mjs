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
const logic = await import('../web/src/api/scorecards/logic.ts');
const mock = await import('../web/src/api/mock/scorecards.ts');

const DAY = 86_400_000;
const NOW = 1_800_000_000_000;
const o = (level, state) => ({ level, state });

test('levels stop at the first failing level, like the server', () => {
  assert.equal(logic.serviceLevel([]), 0);
  assert.equal(logic.serviceLevel([o(1, 'skip')]), 0);
  assert.equal(logic.serviceLevel([o(1, 'fail'), o(2, 'pass')]), 0);
  assert.equal(logic.serviceLevel([o(1, 'pass'), o(2, 'fail'), o(3, 'pass')]), 1);
  assert.equal(logic.serviceLevel([o(1, 'pass'), o(1, 'skip'), o(2, 'pass'), o(3, 'fail')]), 2);
  assert.equal(logic.serviceLevel([o(1, 'pass'), o(3, 'pass')]), 3);
  assert.equal(logic.score([o(1, 'pass'), o(2, 'fail'), o(2, 'skip'), o(3, 'pass'), o(3, 'pass')]), 75);
  assert.equal(logic.score([o(1, 'skip')]), null);
});

test('service references match whole names only', () => {
  assert.ok(logic.referencesService('avg:trace.error_rate{env:prod,service:order-service} > 5', 'order-service'));
  assert.ok(logic.referencesService('logs("service:billing status:error") by {host}', 'billing'));
  assert.ok(logic.referencesService('service:(billing OR order-service)', 'order-service'));
  assert.ok(logic.referencesService('service:"billing"', 'billing'));
  assert.ok(!logic.referencesService('service:billing-watcher', 'billing'));
  assert.ok(!logic.referencesService('myservice:billing', 'billing'));
  assert.ok(!logic.referencesService('avg:billing.invoices{*}', 'billing'));
});

test('built-in checks follow the server', () => {
  const base = { team: '', contacts: 0, repoUrl: '', docsUrl: '', versions: [], logs: 0, tracedLogs: 0, lastCiDeploy: null, monitors: [], slos: [] };
  const v = (version, firstDays, recent) => ({ version, firstSeen: NOW - firstDays * DAY, lastSeen: NOW - 1000, recentSpans: recent });
  assert.equal(logic.evaluateBuiltin('deployment_tracking', base, NOW).reason, 'No spans in the past 7 days');
  assert.equal(logic.evaluateBuiltin('deployment_tracking', { ...base, versions: [v('', 9, 4), v('1.3.0', 2, 3)] }, NOW).state, 'pass');
  const deployed = logic.evaluateBuiltin('deployed_recently', { ...base, versions: [v('1.0.0', 15, 0), v('1.1.0', 4, 10)] }, NOW);
  assert.deepEqual(deployed, { state: 'pass', reason: 'Version 1.1.0 first seen 4 days ago' });
  assert.equal(logic.evaluateBuiltin('deployed_recently', { ...base, versions: [v('1.0.0', 90, 10)] }, NOW).state, 'fail');
  assert.equal(logic.evaluateBuiltin('logs_correlation', { ...base, logs: 400, tracedLogs: 100 }, NOW).reason, '25% of the logs of the past 7 days carry a trace id');
  assert.equal(logic.evaluateBuiltin('docs_defined', { ...base, docsUrl: '  ' }, NOW).state, 'fail');
  assert.equal(logic.evaluateBuiltin('monitors_defined', { ...base, monitors: ['a', 'b', 'c', 'd'] }, NOW).reason, '4 monitors: a, b, c and 1 more');
  assert.deepEqual(logic.evaluateBuiltin('on_call_defined', { ...base, team: 'checkout' }, NOW), { state: 'skip', reason: 'On-Call is not configured' });
});

test('rule summaries group by entity, kind and team; rollups order teams by score', () => {
  const services = [
    { service: 'order-service', team: 'checkout', level: 1, score: 50 },
    { service: 'inventory-service', team: 'checkout', level: 3, score: 100 },
    { service: 'billing', team: '', level: 0, score: null },
    { service: 'catalog-service', team: 'catalog', level: 2, score: 90 },
  ];
  const outcomes = [
    { ruleId: 'r', service: 'order-service', state: 'fail', reason: '' },
    { ruleId: 'r', service: 'inventory-service', state: 'pass', reason: '' },
    { ruleId: 'r', service: 'billing', state: 'skip', reason: '' },
  ];
  const byTeam = logic.summarize(outcomes, services, 'team');
  assert.deepEqual(
    byTeam.map((g) => [g.label, g.pass, g.fail, g.skip]),
    [
      ['checkout', 1, 1, 0],
      ['No team', 0, 0, 1],
    ],
  );
  assert.equal(byTeam[0].passPct, 50);
  assert.deepEqual(
    logic.summarize(outcomes, services, 'kind').map((g) => [g.key, g.total]),
    [['service', 3]],
  );
  assert.equal(logic.summarize(outcomes, services, 'entity')[0].key, 'order-service');
  const roll = logic.teamRollup(services);
  assert.deepEqual(
    roll.map((t) => [t.team, t.services, t.avgScore]),
    [
      ['checkout', 2, 75],
      ['catalog', 1, 90],
      ['', 1, null],
    ],
  );
  assert.deepEqual(roll[0].levels, [0, 1, 0, 1]);
  assert.deepEqual(logic.levelCounts(services), [1, 1, 1, 1]);
});

test('recommended rules are grouped like the first-run card', () => {
  const rec = [
    { key: 'slos_defined', name: 'SLOs defined', level: 2, scorecard: 'Production Readiness', description: '' },
    { key: 'team_defined', name: 'Team defined', level: 1, scorecard: 'Ownership & Documentation', description: '' },
    { key: 'logs_correlation', name: 'Logs correlation is active', level: 3, scorecard: 'Observability Best Practices', description: '' },
    { key: 'monitors_defined', name: 'Monitors defined', level: 1, scorecard: 'Production Readiness', description: '' },
  ];
  assert.deepEqual(
    logic.recommendedGroups(rec).map((g) => [g.scorecard, g.rules.map((r) => r.key)]),
    [
      ['Observability Best Practices', ['logs_correlation']],
      ['Ownership & Documentation', ['team_defined']],
      ['Production Readiness', ['monitors_defined', 'slos_defined']],
    ],
  );
});

test('the mock starts unset, sets up the chosen rules and evaluates them from mock data', async () => {
  mock.resetScorecardsMock();
  const first = await mock.getScorecards();
  assert.equal(first.setUp, false);
  assert.equal(first.recommended.length, 10);
  const keys = first.recommended.map((r) => r.key).filter((k) => k !== 'docs_defined');
  const ov = await mock.setupScorecards({ rules: keys });
  assert.equal(ov.setUp, true);
  assert.equal(ov.scorecards.length, 3);
  assert.equal(ov.rules.length, 10);
  const docs = ov.rules.find((r) => r.builtin === 'docs_defined');
  assert.equal(docs.enabled, false);
  assert.ok(!ov.outcomes.some((x) => x.ruleId === docs.id));
  assert.ok(ov.services.length >= 4);
  const order = ov.services.find((s) => s.service === 'order-service');
  assert.ok(order, 'order-service is listed');
  const monitors = ov.rules.find((r) => r.builtin === 'monitors_defined');
  const orderMonitors = ov.outcomes.find((x) => x.ruleId === monitors.id && x.service === 'order-service');
  assert.equal(orderMonitors.state, 'pass', orderMonitors.reason);
  const onCall = ov.rules.find((r) => r.builtin === 'on_call_defined');
  assert.ok(ov.outcomes.filter((x) => x.ruleId === onCall.id).every((x) => x.state === 'skip'));
  for (const s of ov.services) {
    const mine = ov.outcomes.filter((x) => x.service === s.service).map((x) => ({ level: ov.rules.find((r) => r.id === x.ruleId).level, state: x.state }));
    assert.equal(s.level, logic.serviceLevel(mine), s.service);
  }
});

test('custom rules take reported outcomes within their scope; metadata edits re-evaluate', async () => {
  mock.resetScorecardsMock();
  const { recommended } = await mock.getScorecards();
  await mock.setupScorecards({ rules: recommended.map((r) => r.key) });
  let ov = await mock.createScorecardRule({ name: 'Load tested this quarter', scorecard: 'Reliability', level: 2, scope: 'team:checkout', description: '' });
  assert.equal(ov.scorecards.length, 4);
  const rule = ov.rules.find((r) => r.name === 'Load tested this quarter');
  assert.equal(rule.builtin, null);
  const inScope = ov.outcomes.filter((x) => x.ruleId === rule.id);
  assert.ok(inScope.length >= 1 && inScope.every((x) => x.state === 'skip' && x.reason === 'No outcome reported for this service'));
  const teamOf = new Map(ov.services.map((s) => [s.service, s.team]));
  assert.ok(inScope.every((x) => teamOf.get(x.service) === 'checkout'));
  ov = await mock.setScorecardOutcomes(rule.id, { outcomes: [{ service: inScope[0].service, state: 'pass', remarks: 'k6 run' }] });
  const reported = ov.outcomes.find((x) => x.ruleId === rule.id && x.service === inScope[0].service);
  assert.deepEqual([reported.state, reported.reason], ['pass', 'k6 run']);
  await assert.rejects(() => mock.setScorecardOutcomes(ov.rules.find((r) => r.builtin === 'team_defined').id, { outcomes: [{ service: 'billing', state: 'pass', remarks: '' }] }));
  await assert.rejects(() => mock.deleteScorecardRule(ov.rules.find((r) => r.builtin === 'team_defined').id));

  const team = ov.rules.find((r) => r.builtin === 'team_defined');
  const without = ov.services.find((s) => !s.team);
  assert.ok(without, 'a service without a team');
  ov = await mock.updateServiceMetadata(without.service, { team: 'billing', contacts: [], repoUrl: '', docsUrl: '' });
  assert.equal(ov.outcomes.find((x) => x.ruleId === team.id && x.service === without.service).state, 'pass');
  await assert.rejects(() => mock.updateServiceMetadata(without.service, { team: '', contacts: [], repoUrl: 'javascript:alert(1)', docsUrl: '' }));

  ov = await mock.updateScorecardRule(team.id, { enabled: false });
  assert.ok(!ov.outcomes.some((x) => x.ruleId === team.id));
  await mock.deleteScorecardRule(rule.id);
  assert.ok(!(await mock.getScorecards()).rules.some((r) => r.id === rule.id));
});
