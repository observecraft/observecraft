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
const logic = await import('../web/src/pages/apm/service/logic.ts');
const apm = await import('../web/src/api/mock/apm.ts');

const now = Date.now();
const lastHour = { from: now - 3600e3, to: now };

test("the env selector shows each env's requests per second, and NO DATA without any", () => {
  const envs = [
    { env: 'dev', reqPerSec: 0.4 },
    { env: 'it', reqPerSec: null },
    { env: 'prod', reqPerSec: 0.2 },
  ];
  assert.deepEqual(
    logic.envOptions(envs, null).map((o) => [o.value, o.label, o.hint]),
    [
      ['*', '*', '0.6'],
      ['dev', 'dev', '0.4'],
      ['it', 'it', 'NO DATA'],
      ['prod', 'prod', '0.2'],
    ],
  );
  // The env in the URL stays selectable when the service has none of its requests.
  assert.deepEqual(
    logic.envOptions(envs, 'qa').map((o) => [o.value, o.hint]),
    [
      ['*', '0.6'],
      ['dev', '0.4'],
      ['it', 'NO DATA'],
      ['prod', '0.2'],
      ['qa', 'NO DATA'],
    ],
  );
  assert.deepEqual([0, 0.004, 1.234, 12.34, 1234].map(logic.envRateLabel), ['0', '<0.01', '1.23', '12.3', '1234']);
});

test("the mock's service filters list env rates and an env without traffic", () => {
  const f = apm.getServiceFilters({ range: lastHour, service: 'storefront-api' });
  const byEnv = Object.fromEntries(f.envs.map((e) => [e.env, e.reqPerSec]));
  assert.ok(byEnv.prod > 0 && byEnv.staging > 0, JSON.stringify(f.envs));
  assert.ok(byEnv.prod > byEnv.staging);
  assert.equal(byEnv.dev, null, 'dev had requests before the range only');
  const summary = apm.getServiceSummary({ range: lastHour, service: 'storefront-api' });
  assert.ok(Math.abs(byEnv.prod + byEnv.staging - summary.reqPerSec) < 1e-9, 'the envs add up to the service');
});

test('primary tags narrow the service as span search terms', async () => {
  const { primaryTagQuery } = await import('../web/src/api/apm/scope.ts');
  assert.equal(primaryTagQuery({}), '');
  assert.equal(primaryTagQuery({ cluster: 'prod-eu-1', namespace: 'shop' }), 'kube_cluster_name:"prod-eu-1" kube_namespace:"shop"');
  assert.equal(primaryTagQuery({ cluster: '*', namespace: 'a"b' }), 'kube_namespace:"a\\"b"');
});

test("the mock's primary tags list the service's clusters and namespaces and narrow every section", () => {
  const f = apm.getServiceFilters({ range: lastHour, service: 'order-service' });
  assert.deepEqual(f.namespaces, ['shop']);
  assert.deepEqual(f.clusters, ['prod-eu-1', 'staging-eu-1']);
  assert.deepEqual(apm.getServiceFilters({ range: lastHour, service: 'order-service', env: 'prod' }).clusters, ['prod-eu-1']);
  const all = apm.getServiceSummary({ range: lastHour, service: 'order-service' });
  const staging = apm.getServiceSummary({ range: lastHour, service: 'order-service', cluster: 'staging-eu-1' });
  const prod = apm.getServiceSummary({ range: lastHour, service: 'order-service', env: 'prod' });
  assert.ok(staging.requests > 0 && staging.requests < all.requests);
  assert.equal(staging.requests + prod.requests, all.requests);
  assert.equal(apm.getServiceSummary({ range: lastHour, service: 'order-service', namespace: 'payments' }).requests, 0);
  const res = apm.listResources({ range: lastHour, service: 'order-service', cluster: 'staging-eu-1' });
  assert.equal(
    res.reduce((a, r) => a + r.requests, 0),
    staging.requests,
  );
  const down = apm.getDependencies({ range: lastHour, service: 'order-service', namespace: 'payments' });
  assert.equal(down.downstream.length, 0);
});

test('a primary tag in the URL stays selectable', () => {
  assert.deepEqual(logic.tagValues(['shop'], 'payments'), ['shop', 'payments']);
  assert.deepEqual(logic.tagValues(['shop'], '*'), ['shop']);
  assert.deepEqual(logic.tagValues(['shop'], null), ['shop']);
});

test('the monitor bar splits the monitors by status, the worst first', () => {
  const m = (status) => ({ id: 1, name: status, status });
  assert.deepEqual(logic.monitorSegments([]), []);
  assert.deepEqual(logic.monitorSegments([m('ok'), m('alert'), m('ok'), m('nodata'), m('warn'), m('ok'), m('ok')]), [
    { status: 'alert', count: 1, label: 'ALERT: 1 Monitor' },
    { status: 'warn', count: 1, label: 'WARN: 1' },
    { status: 'nodata', count: 1, label: 'NO DATA: 1' },
    { status: 'ok', count: 4, label: 'OK: 4 Monitors' },
  ]);
});

test('the mock lists monitors naming the service', () => {
  const monitors = apm.listServiceMonitors({ range: lastHour, service: 'storefront-api' });
  assert.ok(monitors.length > 0);
  const order = { alert: 3, warn: 2, nodata: 1, ok: 0 };
  for (let i = 1; i < monitors.length; i++) assert.ok(order[monitors[i - 1].status] >= order[monitors[i].status]);
  assert.deepEqual(apm.listServiceMonitors({ range: lastHour, service: 'no-such-service' }), []);
});

test("a resource's monitors as status badges, the worst first", () => {
  const m = (status, resources) => ({ id: 1, name: status, status, resources });
  const monitors = [m('ok', ['GET /cart']), m('alert', ['GET /cart', 'POST /orders']), m('ok', ['GET /cart']), m('warn', [])];
  assert.deepEqual(logic.resourceMonitorBadges(monitors, 'GET /cart'), [
    { status: 'alert', count: 1, label: '1 ALERT' },
    { status: 'ok', count: 2, label: '2 OK' },
  ]);
  assert.deepEqual(logic.resourceMonitorBadges(monitors, 'GET /orders'), []);
});

test("the mock's monitors name the resources they target", () => {
  const monitors = apm.listServiceMonitors({ range: lastHour, service: 'storefront-api' });
  const checkout = monitors.find((m) => m.name === 'Storefront API checkout error rate is high');
  assert.deepEqual(checkout.resources, ['POST /api/orders']);
  assert.ok(['alert', 'warn'].includes(checkout.status), checkout.status);
  assert.deepEqual(monitors.find((m) => m.name === 'Storefront API p95 latency above SLO').resources, []);
});
