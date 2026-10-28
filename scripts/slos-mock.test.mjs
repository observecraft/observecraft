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
const slos = await import('../web/src/api/mock/slos.ts');

const HOUR = 3_600_000;
const DAY = 24 * HOUR;
const all = { query: '', types: [], states: [], burnRates: [], creators: [], teams: [], services: [], envs: [], tags: [], window: null, sort: null, offset: 0, limit: 50 };
const byName = async (name) => (await slos.searchSlos({ ...all, query: name })).slos.find((s) => s.name === name);

test('the seed has every state, one breached and one in warning', async () => {
  const page = await slos.searchSlos(all);
  assert.equal(page.total, 8);
  assert.deepEqual(page.summary, { breached: 1, warning: 1, ok: 5, noData: 1, total: 8 });
  assert.equal(page.slos[0].status.state, 'breached', 'worst state first');
  assert.equal(page.slos[1].status.state, 'warning');
  const types = Object.fromEntries(page.facets.type.map((f) => [f.value, f.count]));
  assert.deepEqual(types, { metric: 5, time_slice: 2, monitor: 1 });
  for (const s of page.slos.filter((x) => x.status.state !== 'no_data')) {
    assert.ok(s.status.sli > 99 && s.status.sli <= 100, `${s.name}: plausible SLI`);
    assert.equal(s.status.window, s.primaryWindow);
  }
});

test('status, error budget and burn rate follow the targets', async () => {
  const billing = await byName('Billing payment success');
  const st = billing.status;
  assert.equal(st.unit, 'events');
  assert.ok(st.sli < st.warning && st.sli >= st.target);
  assert.ok(Math.abs(st.budgetAllowed - st.total * (1 - st.target / 100)) < 1e-6);
  assert.ok(Math.abs(st.budgetUsed - (st.total - st.good)) < 1e-6);
  assert.ok(Math.abs(st.budgetRemaining - ((st.budgetAllowed - st.budgetUsed) / st.budgetAllowed) * 100) < 1e-6);
  assert.ok(st.burnRate1h >= 10, 'billing burns fast in the last hour');
  const page = await slos.searchSlos({ ...all, burnRates: ['high'] });
  assert.deepEqual(
    page.slos.map((s) => s.name),
    ['Billing payment success'],
  );

  const hosts = await byName('Platform host health');
  assert.equal(hosts.status.state, 'breached');
  assert.equal(hosts.status.unit, 'minutes');
  assert.equal(Math.round(hosts.status.total), 30 * 24 * 60 - 120, 'the org-wide 2-hour correction is excluded');
  assert.equal(Math.round(hosts.status.budgetUsed), 120);
  assert.ok(hosts.status.budgetRemaining < 0, 'overspent');

  const staging = await byName('Storefront API availability (staging)');
  assert.equal(staging.status.state, 'no_data');
  assert.equal(staging.status.sli, null);
});

test('the window toggle measures each SLO over that window', async () => {
  const week = await slos.searchSlos({ ...all, window: '7d' });
  assert.ok(week.slos.every((s) => s.status.window === '7d'));
  const quarter = await slos.searchSlos({ ...all, window: '90d' });
  const hosts = quarter.slos.find((s) => s.name === 'Platform host health');
  // The 90d target applies over 90 days: 2 hours down in 90 days meets 99.9%.
  assert.equal(hosts.status.state, 'ok');
  assert.equal(hosts.status.target, 99.9);
  // A window without its own target uses the primary target.
  const day = (await slos.searchSlos({ ...all, window: '1d', query: 'Billing' })).slos[0];
  assert.equal(day.status.target, 99.95);
});

test('facets ignore their own filter and text matches tags', async () => {
  const page = await slos.searchSlos({ ...all, teams: ['catalog'] });
  assert.equal(page.total, 2);
  assert.ok(page.facets.teams.length > 2, 'the team facet still lists other teams');
  assert.equal(page.summary.total, 2);
  const services = await slos.searchSlos({ ...all, query: 'service:order-service' });
  assert.deepEqual(services.slos.map((s) => s.name).sort(), ['Checkout API availability', 'Order processing success']);
  const envs = Object.fromEntries((await slos.searchSlos(all)).facets.envs.map((f) => [f.value, f.count]));
  assert.deepEqual(envs, { prod: 7, staging: 1 });
});

test('the detail has a status per target, the history and its corrections', async () => {
  const hosts = await byName('Platform host health');
  const d = await slos.getSlo(hosts.id);
  assert.deepEqual(
    d.statuses.map((s) => s.window),
    ['30d', '90d'],
  );
  assert.equal(d.history.length, 120, '6-hour buckets over 30 days');
  const last = d.history[d.history.length - 1];
  assert.ok(Math.abs(last.sli - hosts.status.sli) < 1e-9, 'the running SLI ends at the window SLI');
  assert.ok(Math.abs(last.budgetRemaining - hosts.status.budgetRemaining) < 1e-6);
  assert.ok(
    d.history.some((h) => h.bucketSli < 100),
    'the outage shows in its bucket',
  );
  assert.deepEqual(
    d.monitors.map((m) => m.id),
    [1001, 1002],
  );
  assert.ok(
    d.corrections.some((c) => c.sloId === null),
    'org-wide corrections apply',
  );
  const week = await slos.getSlo(hosts.id, '7d');
  assert.equal(week.history.length, 84, '2-hour buckets over 7 days');
});

test('corrections are excluded from the SLI', async () => {
  const billing = await byName('Billing payment success');
  const before = billing.status;
  const now = Math.floor(Date.now() / HOUR) * HOUR;
  // Exclude the 5-hour incident 70 hours ago.
  const c = await slos.createCorrection({ sloId: billing.id, category: 'deployment', description: 'Payment provider migration', start: now - 71 * HOUR, end: now - 64 * HOUR });
  const after = (await byName('Billing payment success')).status;
  assert.ok(after.sli > before.sli);
  assert.ok(after.total < before.total);
  assert.equal(after.state, 'ok');
  assert.equal((await slos.listCorrections(billing.id)).filter((x) => x.id === c.id).length, 1);
  await slos.updateCorrection(c.id, { sloId: billing.id, category: 'other', description: 'Shorter', start: now - 71 * HOUR, end: now - 70 * HOUR });
  assert.equal((await byName('Billing payment success')).status.state, 'warning');
  await slos.deleteCorrection(c.id);
  assert.equal((await byName('Billing payment success')).status.sli, before.sli);
  await assert.rejects(slos.createCorrection({ category: 'other', start: now, end: now }), /ends after it starts/);
  await assert.rejects(slos.createCorrection({ category: 'other', start: now, end: now + 91 * DAY }), /at most 90 days/);
});

test('create, update and delete validate like the server', async () => {
  const input = {
    name: '  Gateway availability ',
    type: 'metric',
    goodQuery: 'sum:trace.http.request.hits{service:api-gateway,!http.status_code:5*}',
    totalQuery: 'sum:trace.http.request.hits{service:api-gateway}',
    targets: [{ window: '30d', target: 99.9, warning: 99.95 }],
    tags: ['service:api-gateway', ' service:api-gateway', ''],
    teams: ['platform'],
  };
  const created = await slos.createSlo(input);
  assert.equal(created.name, 'Gateway availability');
  assert.deepEqual(created.tags, ['service:api-gateway']);
  assert.equal(created.primaryWindow, '30d');
  assert.ok(created.status.sli > 99);
  const updated = await slos.updateSlo(created.id, { ...input, targets: [{ window: '7d', target: 99.5 }], primaryWindow: '7d' });
  assert.equal(updated.status.window, '7d');
  assert.equal(updated.status.target, 99.5);
  await assert.rejects(slos.createSlo({ ...input, name: ' ' }), /Name/);
  await assert.rejects(slos.createSlo({ ...input, goodQuery: '' }), /good events/);
  await assert.rejects(slos.createSlo({ ...input, targets: [{ window: '7d', target: 100 }] }), /below 100/);
  await assert.rejects(slos.createSlo({ ...input, targets: [{ window: '7d', target: 99, warning: 98 }] }), /warning/);
  await assert.rejects(slos.createSlo({ ...input, primaryWindow: '90d' }), /primary/);
  await assert.rejects(slos.createSlo({ ...input, type: 'monitor', monitorIds: [] }), /monitors/);
  await assert.rejects(slos.createSlo({ ...input, type: 'time_slice', sliceQuery: 'avg:x{*}', comparator: '<', threshold: 1, sliceMinutes: 2 }), /1 or 5/);
  await slos.deleteSlo(created.id);
  await assert.rejects(slos.getSlo(created.id), /not found/);
});
