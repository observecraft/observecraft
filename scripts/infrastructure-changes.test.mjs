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
const model = await import('../web/src/pages/infrastructure/changeModel.ts');
const mock = await import('../web/src/api/mock/changes.ts');

const HOUR = 3600e3;
const now = Date.now();
const week = { from: now - 7 * 24 * HOUR, to: now };
const search = (query, range = week) => mock.searchChanges({ range, query, offset: 0, limit: 500 });

test('a change links to the changed object: Kubernetes Explorer, APM service, monitor or nothing', () => {
  assert.equal(
    model.changeLink({ kubeKind: 'deployments', kubeUid: 'uid 1', resourceType: 'kube_deployment', resource: 'billing', source: 'kubernetes' }),
    '/orchestration/explorer?resource=deployments&panel=uid+1',
  );
  assert.equal(model.changeLink({ kubeKind: null, kubeUid: null, resourceType: 'service', resource: 'order-service', source: 'apm' }), '/apm/services/order-service');
  assert.equal(model.changeLink({ kubeKind: null, kubeUid: null, resourceType: 'monitor', resource: 'High error rate', source: 'audit', details: { target: 'monitor:12' } }), '/monitors/12');
  assert.equal(model.changeLink({ kubeKind: null, kubeUid: null, resourceType: 'event', resource: 'api', source: 'events' }), null);
});

test('change types have labels and colors', () => {
  for (const t of ['deployment', 'rollout', 'scale', 'config', 'crash', 'feature_flag']) {
    assert.ok(model.CHANGE_TYPE_LABELS[t], t);
    assert.ok(model.CHANGE_TYPE_TOKENS[t], t);
  }
});

test('mock changes: newest first, in range, every type, fictional demo data', async () => {
  const page = await search('');
  assert.ok(page.total > 10);
  const ts = page.items.map((c) => c.timestamp);
  assert.deepEqual(
    ts,
    [...ts].sort((a, b) => b - a),
  );
  assert.ok(page.items.every((c) => c.timestamp >= week.from && c.timestamp <= week.to));
  const types = new Set(page.items.map((c) => c.changeType));
  for (const t of ['deployment', 'rollout', 'scale', 'config', 'crash']) assert.ok(types.has(t), t);
  for (const c of page.items.filter((x) => x.author)) assert.match(c.author, /@example\.com$|^[A-Z][a-z]+ [A-Z][a-z]+$/);
  // Rollouts carry old and new images.
  const rollout = page.items.find((c) => c.changeType === 'rollout');
  assert.ok(rollout.previous && rollout.current && rollout.previous !== rollout.current);
});

test('mock changes: filters, facets that ignore their own filter, and a histogram by type', async () => {
  const rollouts = await search('change_type:rollout');
  assert.ok(rollouts.items.every((c) => c.changeType === 'rollout'));
  assert.ok(rollouts.facets.find((f) => f.key === 'change_type').values.length > 1);
  const svc = await search('service:order-service');
  assert.ok(svc.total > 0 && svc.items.every((c) => c.service === 'order-service'));
  const bucketed = rollouts.histogram.reduce((n, b) => n + Object.values(b.counts).reduce((a, v) => a + v, 0), 0);
  assert.equal(bucketed, rollouts.total);
  assert.ok(rollouts.histogram.every((b) => Object.keys(b.counts).every((k) => k === 'rollout')));
  const lastHour = await search('', { from: now - HOUR, to: now });
  assert.ok(lastHour.total < (await search('')).total);
});
