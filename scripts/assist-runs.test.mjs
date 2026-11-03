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
const mock = await import('../web/src/api/mock/assistRuns.ts');
const { lineDiff } = await import('../web/src/lib/lineDiff.ts');

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

test('an investigation is queued, runs its steps and concludes; a cancelled one stops', async () => {
  mock.resetAssistRunsMock();
  const run = await mock.startRun({ kind: 'investigation', question: 'Why is checkout slow?', monitorId: null, signalId: null });
  assert.equal((await mock.getRun(run.id)).status, 'queued');
  await sleep(1000);
  const running = await mock.getRun(run.id);
  assert.equal(running.status, 'running');
  assert.ok(running.steps.length >= 1 && !running.conclusion);
  const other = await mock.startRun({ kind: 'investigation', question: 'And billing?', monitorId: null, signalId: null });
  await mock.cancelRun(other.id);
  await sleep(6500);
  const done = await mock.getRun(run.id);
  assert.equal(done.status, 'concluded');
  assert.equal(done.verdict, 'conclusive');
  assert.ok(done.conclusion.evidence.length > 0);
  assert.equal((await mock.getRun(other.id)).status, 'cancelled');
  const report = await mock.runReport({ kind: 'investigation', range: { from: Date.now() - 60_000, to: Date.now() + 1 }, monitorId: null });
  assert.equal(report.total, 2);
});

test('runs are refused past the hourly limit', async () => {
  mock.resetAssistRunsMock();
  await mock.putRunSettings({ knowledge: '', runsPerHour: 1, automaticRunsPerDay: 10, notify: [], securityAutoSeverities: [], securityRunAs: null });
  await mock.startRun({ kind: 'investigation', question: 'one', monitorId: null, signalId: null });
  await assert.rejects(mock.startRun({ kind: 'investigation', question: 'two', monitorId: null, signalId: null }), /limit/);
});

test('an agent is drafted from a description, saved and run with its skills', async () => {
  mock.resetAssistRunsMock();
  const d = await mock.draftAgent('Every morning find new errors in the logs and tell me which matter');
  assert.equal(d.schedule, '0 9 * * *');
  assert.ok(d.skills.includes('errors') && d.skills.includes('logs'));
  await assert.rejects(mock.createAgent({ ...d, schedule: 'every morning', tags: [], allowActions: false, enabled: true }), /cron/);
  const a = await mock.createAgent({ ...d, tags: [], allowActions: false, enabled: true });
  const run = await mock.runAgent(a.id, 'Only billing');
  assert.equal(run.kind, 'agent');
  assert.equal((await mock.getAgent(a.id)).runs, 1);
});

test('a code session proposes a change and opens one pull request', async () => {
  mock.resetAssistRunsMock();
  await assert.rejects(mock.startCodeSession('example-org/unknown', 'x'), /isn't added/);
  const s = await mock.startCodeSession('example-org/storefront', 'Cart lookups are slow');
  await assert.rejects(mock.openPullRequest(s.id), /no change/);
  await sleep(6000);
  const done = await mock.getRun(s.id);
  assert.equal(done.change.files[0].path, 'app/controllers/carts_controller.rb');
  const pr = await mock.openPullRequest(s.id);
  assert.ok(pr.branch.startsWith('assist/query-the-open-cart'));
  assert.equal(pr.draft, true);
  await assert.rejects(mock.openPullRequest(s.id), /already/);
});

test('usage adds up per application and credits are counted', async () => {
  mock.resetAssistRunsMock();
  const u = await mock.usage({ from: Date.now() - 7 * 86_400_000, to: Date.now() + 1 });
  assert.equal(
    u.byApp.reduce((a, x) => a + x.uses, 0),
    u.daily.length ? u.byApp.reduce((a, x) => a + x.uses, 0) : 0,
  );
  assert.equal(u.credits, Math.ceil((u.inputTokens + u.outputTokens) / 1000));
  assert.ok(u.estCost > 0);
  const c = await mock.getCredits();
  assert.equal(c.tokensPerCredit, 1000);
});

test('a line diff keeps, removes and adds lines in order', () => {
  const d = lineDiff('a\nb\nc\n', 'a\nx\nc\nd\n');
  assert.deepEqual(
    d.map((l) => `${l.kind[0]}${l.text}`),
    ['ka', 'rb', 'ax', 'kc', 'ad'],
  );
  assert.deepEqual(d[0], { kind: 'keep', text: 'a', oldLine: 1, newLine: 1 });
  assert.deepEqual(lineDiff('', 'new\n'), [{ kind: 'add', text: 'new', oldLine: null, newLine: 1 }]);
});
