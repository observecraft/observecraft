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
const db = await import('../web/src/api/mock/databases.ts');

const HOUR = 3_600_000;
const range = { from: 1_760_000_000_000 - HOUR, to: 1_760_000_000_000 };

test('the database fleet covers several engines with fictional hosts', async () => {
  const page = await db.searchDatabases({ range, query: '' });
  assert.equal(page.enabled, true);
  assert.equal(page.instances.length, 6);
  const engines = page.facets.find((f) => f.key === 'engine');
  assert.deepEqual(engines.values.map((v) => v.value).sort(), ['mongodb', 'mysql', 'postgres', 'redis']);
  for (const i of page.instances) {
    assert.ok(i.host.startsWith('i-0a1b2c3d4e5f6'));
    assert.ok(i.connections >= 0 && (i.maxConnections == null || i.connections <= i.maxConnections));
    assert.ok(i.cacheHitPct == null || (i.cacheHitPct > 0 && i.cacheHitPct <= 100));
  }
  assert.deepEqual(page.unconfigured, [{ host: 'i-0a1b2c3d4e5f60016', engine: 'redis', process: 'redis-server' }]);
});

test('the query filters instances; facets count the whole fleet', async () => {
  const pg = await db.searchDatabases({ range, query: 'engine:postgres env:prod' });
  assert.equal(pg.instances.length, 2);
  assert.equal(pg.facets.find((f) => f.key === 'env').values.length, 2);
  const text = await db.searchDatabases({ range, query: 'inventory' });
  assert.deepEqual(
    text.instances.map((i) => i.engine),
    ['mysql'],
  );
});

test('an instance has its current values and series', async () => {
  const d = await db.databaseInstance({ range, id: 'postgres:i-0a1b2c3d4e5f60013:5432' });
  assert.equal(d.instance.service, 'orders-db-replica');
  assert.deepEqual(
    d.series.map((s) => s.name),
    ['connections', 'throughput', 'cache_hit', 'replication_lag', 'size'],
  );
  assert.ok(d.series[0].points.length >= 50);
  assert.ok(
    d.series[3].points.every(([, v]) => v > 0),
    'a replica lags',
  );
  const missing = await db.databaseInstance({ range, id: 'postgres:nowhere' });
  assert.equal(missing.instance, null);
});

test('data jobs: failures first in facets, query on status and framework', async () => {
  const page = await db.searchDataJobs({ range, query: '' });
  assert.equal(page.jobs.length, 5);
  const status = Object.fromEntries(page.facets.find((f) => f.key === 'status').values.map((v) => [v.value, v.count]));
  assert.deepEqual(status, { failed: 2, succeeded: 2, running: 1 });
  const failedAirflow = await db.searchDataJobs({ range, query: 'framework:airflow status:failed' });
  assert.deepEqual(
    failedAirflow.jobs.map((j) => j.name),
    ['catalog_feed_sync'],
  );
});
