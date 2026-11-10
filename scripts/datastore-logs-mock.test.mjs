// The mock's PostgreSQL, PgBouncer, Redis and ClickHouse logs (web/src/api/mock/datastoreLogs.ts): server lines parsed by
// their integration pipelines, as integrations-core's samples are.
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { registerHooks } from 'node:module';
import test from 'node:test';

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
const { datastoreLogs } = await import('../web/src/api/mock/datastoreLogs.ts');
const { rng } = await import('../web/src/api/mock/generator.ts');

const now = Date.UTC(2026, 10, 10, 12);
const logs = datastoreLogs(rng(29), now);
const of = (source) => logs.filter((l) => l.source === source);

test('every datastore has a day of logs, dated by their own lines', () => {
  for (const source of ['postgresql', 'pgbouncer', 'redis', 'clickhouse']) {
    const list = of(source);
    assert.ok(list.length >= 200, `${source}: ${list.length}`);
    assert.ok(
      list.every((l) => l.timestamp <= now && l.timestamp > now - 86_400_000 - 1000),
      source,
    );
  }
});

test('PostgreSQL lines get the pipeline’s statement, duration, user and status', () => {
  const slow = of('postgresql').find((l) => l.attributes.duration > 1e9);
  assert.ok(slow, 'a slow query (duration in nanoseconds)');
  assert.match(slow.attributes.db.statement, /^SELECT p\.id/);
  assert.equal(slow.attributes.db.operation, 'SELECT');
  assert.ok(['order_service', 'catalog_service', 'billing', 'inventory_service'].includes(slow.attributes.db.user));
  assert.ok(of('postgresql').some((l) => l.status === 'error' && /duplicate key/.test(l.message)));
  assert.ok(
    of('postgresql').every((l) => l.tags.db),
    'the db tag',
  );
});

test('Redis, PgBouncer and ClickHouse lines get their statuses and attributes', () => {
  assert.ok(of('redis').some((l) => l.status === 'warn' && l.attributes.redis.severity === 'warning'));
  assert.ok(of('redis').every((l) => l.attributes.pid === 1 && l.message.startsWith('1:')));
  assert.ok(of('pgbouncer').some((l) => l.status === 'warn'));
  assert.ok(
    of('pgbouncer').some((l) => l.attributes.db?.user || l.attributes.usr?.name || l.attributes.network?.client?.ip),
    JSON.stringify(of('pgbouncer')[0].attributes),
  );
  assert.ok(of('clickhouse').some((l) => l.status === 'error'));
  assert.ok(of('clickhouse').every((l) => l.attributes.logger?.thread_name !== undefined || l.attributes.thread_id !== undefined || Object.keys(l.attributes).length > 0));
});
