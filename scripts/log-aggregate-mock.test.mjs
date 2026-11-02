// The mock's log analytics (web/src/api/mock/index.ts mockAggregateLogs), as /api/v1/logs/aggregate answers.
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { registerHooks } from 'node:module';
import test from 'node:test';

// The web sources use extensionless imports and JSON without attributes (bundler resolution).
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
const { mockAggregateLogs, mockTopLogValues } = await import('../web/src/api/mock/index.ts');

const to = Date.now();
const range = { from: to - 3600_000, to };

test('a count by a facet agrees with the top values', () => {
  const top = mockTopLogValues({ range, query: '', key: 'service', limit: 5 });
  const agg = mockAggregateLogs({ compute: [{ aggregation: 'count', type: 'total' }], filter: { query: '', from: range.from, to: range.to }, group_by: [{ facet: 'service', limit: 5 }] });
  assert.deepEqual(
    agg.data.buckets.map((b) => [b.by.service, b.computes.c0]),
    top.map((t) => [t.value, t.count]),
  );
});

test('no facet is one bucket; unique counts and timeseries', () => {
  const total = mockAggregateLogs({ compute: [{ aggregation: 'count', type: 'total' }], filter: { query: '', from: range.from, to: range.to }, group_by: [] });
  assert.equal(total.data.buckets.length, 1);
  const unique = mockAggregateLogs({ compute: [{ aggregation: 'cardinality', metric: 'service', type: 'total' }], filter: { query: '', from: range.from, to: range.to }, group_by: [] });
  assert.ok(unique.data.buckets[0].computes.c0 >= 1 && unique.data.buckets[0].computes.c0 <= total.data.buckets[0].computes.c0);
  const series = mockAggregateLogs({ compute: [{ aggregation: 'count', type: 'timeseries', interval: '300000' }], filter: { query: '', from: range.from, to: range.to }, group_by: [] });
  const steps = series.data.buckets[0].computes.c0;
  assert.ok(Array.isArray(steps) && steps.length >= 12 && steps.length <= 13, `${steps.length} steps of 5 minutes`);
  assert.equal(
    steps.reduce((n, s) => n + s.value, 0),
    total.data.buckets[0].computes.c0,
  );
});
