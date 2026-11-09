// The ObserveCraft Overview dashboard (scripts/observecraft-dashboard.mjs) over the server's self-monitoring metrics.
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import { OBSERVECRAFT_UNITS, buildDashboard, unitsCsv } from './observecraft-dashboard.mjs';

const seed = (file) => readFileSync(new URL(`../server/seeds/integrations/observecraft/${file}`, import.meta.url), 'utf8');
const dashboard = buildDashboard();

const all = (widgets) => widgets.flatMap((w) => [w, ...(w.definition.type === 'group' ? all(w.definition.widgets) : [])]);
const queries = all(dashboard.widgets).flatMap((w) => (w.definition.type === 'integration' ? (w.definition.original.requests ?? []).flatMap((r) => r.queries ?? []) : []));
const metrics = [...new Set(queries.flatMap((q) => [...(q.query ?? '').matchAll(/^(?:[a-z]+:)?([a-z0-9_.]+)\{/g)].map((m) => m[1])))];

test('the seed is the script output', () => {
  assert.equal(seed('dashboards.json'), JSON.stringify([dashboard], null, 2) + '\n', 'run node scripts/observecraft-dashboard.mjs');
  assert.equal(seed('units.csv'), unitsCsv(), 'run node scripts/observecraft-dashboard.mjs');
});

test('one dashboard with our logo, over self-monitoring metrics and the Agent pods only', () => {
  assert.equal(dashboard.id, 'observecraft-overview');
  assert.equal(dashboard.logo, 'observecraft');
  assert.ok(all(dashboard.widgets).some((w) => w.definition.type === 'image' && w.definition.logo === 'observecraft'));
  const agent = new Set(['container.cpu.usage', 'container.memory.usage', 'kubernetes.containers.restarts']);
  for (const m of metrics) assert.ok(m.startsWith('observecraft.') || agent.has(m), m);
  // The Agent's metrics are only ever ObserveCraft's own pods.
  for (const q of queries) if (!q.query.includes(':observecraft.')) assert.match(q.query, /\{kube_namespace:observecraft\}/, q.query);
});

test('every self-monitoring metric charted has a unit and mock data', async () => {
  const { OBSERVECRAFT_METRICS } = await import('../web/src/api/mock/observecraftMetrics.ts');
  for (const m of metrics.filter((m) => m.startsWith('observecraft.'))) {
    if (!['observecraft.up', 'observecraft.clickhouse.reachable'].includes(m)) assert.ok(OBSERVECRAFT_UNITS[m], `unit of ${m}`);
    assert.ok(OBSERVECRAFT_METRICS[m], `mock of ${m}`);
  }
  for (const [m, unit] of Object.entries(OBSERVECRAFT_UNITS)) assert.equal(OBSERVECRAFT_METRICS[m]?.unit, unit, `mock unit of ${m}`);
});
