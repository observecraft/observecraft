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
const mock = await import('../web/src/api/mock/logMetrics.ts');
const engine = await import('../web/src/api/mock/metricsEngine.ts');

const DEMO_SERVICES = ['api-gateway', 'storefront-api', 'order-service', 'billing', 'catalog-service', 'inventory-service'];
const now = Date.now();
const lastHour = { from: now - 3_600_000, to: now };

test('the demo seed is deterministic and uses the demo services', async () => {
  const metrics = await mock.listLogMetrics();
  assert.ok(metrics.length >= 3);
  assert.deepEqual(
    metrics.map((m) => m.name),
    [...metrics.map((m) => m.name)].sort(),
    'sorted by name',
  );
  assert.ok(metrics.some((m) => m.aggregation === 'count') && metrics.some((m) => m.aggregation === 'distribution' && m.path?.startsWith('@')));
  const services = metrics.flatMap((m) => [...m.query.matchAll(/service:\(?([\w-]+(?: OR [\w-]+)*)\)?/g)].flatMap((x) => x[1].split(' OR ')));
  assert.ok(services.length > 0);
  for (const s of services) assert.ok(DEMO_SERVICES.includes(s), `${s} is a demo service`);
  assert.deepEqual(await mock.listLogMetrics(), metrics, 'same answer twice');
});

test('definitions are validated like the server does', async () => {
  const ok = { name: 'logs.test.count', query: 'service:billing', aggregation: 'count', groupBy: [] };
  const bad = [
    { ...ok, name: '1logs' },
    { ...ok, name: 'logs checkout' },
    { ...ok, name: 'trace.http.hits' },
    { ...ok, aggregation: 'distribution', path: null },
    { ...ok, aggregation: 'distribution', path: 'duration' },
    { ...ok, groupBy: Array.from({ length: 11 }, (_, i) => ({ path: `@a${i}`, tagName: '' })) },
    {
      ...ok,
      groupBy: [
        { path: 'service', tagName: '' },
        { path: 'service', tagName: '' },
      ],
    },
  ];
  for (const input of bad) await assert.rejects(mock.createLogMetric(input), undefined, JSON.stringify(input));
  const seeded = (await mock.listLogMetrics())[0];
  await assert.rejects(mock.createLogMetric({ ...ok, name: seeded.name }), /already exists/);
});

test('create, update and delete', async () => {
  const created = await mock.createLogMetric({
    name: ' logs.test.latency ',
    query: ' service:order-service ',
    aggregation: 'distribution',
    path: '@duration',
    groupBy: [
      { path: ' @http.status_code ', tagName: '' },
      { path: 'env', tagName: 'environment' },
    ],
  });
  assert.equal(created.name, 'logs.test.latency');
  assert.equal(created.query, 'service:order-service');
  assert.deepEqual(
    created.groupBy.map((g) => [g.path, g.tagName]),
    [
      ['@http.status_code', 'http.status_code'],
      ['env', 'environment'],
    ],
  );
  const count = { ...created, aggregation: 'count', path: '@duration' };
  const updated = await mock.updateLogMetric(created.id, count);
  assert.equal(updated.aggregation, 'count');
  assert.equal(updated.path, null, 'a count has no measure');
  await assert.rejects(mock.updateLogMetric(created.id, { ...count, name: 'logs.renamed' }), /renamed/);
  await mock.deleteLogMetric(created.id);
  assert.ok(!(await mock.listLogMetrics()).some((m) => m.id === created.id));
  await assert.rejects(mock.deleteLogMetric(created.id), /not found/);
});

test('generated metrics are graphable in the mock metric catalog', async () => {
  const created = await mock.createLogMetric({
    name: 'logs.test.errors',
    query: 'service:order-service status:error',
    aggregation: 'count',
    groupBy: [
      { path: 'service', tagName: '' },
      { path: '@http.status_code', tagName: '' },
    ],
  });
  const entry = engine.metricCatalog().find((m) => m.name === 'logs.test.errors');
  assert.ok(entry, 'in the catalog');
  assert.deepEqual(entry.tags, ['service', 'http.status_code']);
  const series = await engine.queryTimeseries({ queries: ['sum:logs.test.errors{*} by {service}'], range: lastHour, points: 60 });
  assert.deepEqual(
    series.map((s) => s.name),
    ['service:order-service'],
    'only the logs the query matches',
  );
  assert.ok(series[0].points.length > 10 && series[0].points.some(([, v]) => v > 0));
  const again = await engine.queryTimeseries({ queries: ['sum:logs.test.errors{*} by {service}'], range: lastHour, points: 60 });
  assert.deepEqual(again, series, 'deterministic');
  await mock.deleteLogMetric(created.id);
  assert.ok(!engine.metricCatalog().some((m) => m.name === 'logs.test.errors'), 'gone once deleted');
});

test('distributions are queried by statistic, as on the server', async () => {
  const latency = (await mock.listLogMetrics()).find((m) => m.aggregation === 'distribution');
  const names = engine.metricCatalog().map((m) => m.name);
  for (const stat of ['count', 'avg', 'p50', 'p95', 'p99', 'max']) assert.ok(names.includes(`${latency.name}.${stat}`), `${latency.name}.${stat}`);
  assert.ok(!names.includes(latency.name));
  const [p50, p99] = await Promise.all(['p50', 'p99'].map((s) => engine.queryTimeseries({ queries: [`avg:${latency.name}.${s}{*}`], range: lastHour, points: 30 })));
  assert.ok(
    p99[0].points.every(([, v], i) => v >= p50[0].points[i][1]),
    'p99 ≥ p50',
  );
});
