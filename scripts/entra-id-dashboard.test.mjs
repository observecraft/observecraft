// The Microsoft Entra ID Overview dashboard (scripts/entra-id-dashboard.mjs) over the tenant's Entra ID logs.
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { registerHooks } from 'node:module';
import test from 'node:test';
import { buildDashboard } from './entra-id-dashboard.mjs';

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
const { mockAggregateLogs } = await import('../web/src/api/mock/index.ts');
const { logRequest } = await import('../web/src/api/integrations/logs.ts');

const dashboard = buildDashboard();
const all = (widgets) => widgets.flatMap((w) => [w, ...(w.definition.type === 'group' ? all(w.definition.widgets) : [])]);
const panels = all(dashboard.widgets).filter((w) => w.definition.type === 'integration');
const streams = all(dashboard.widgets).filter((w) => w.definition.type === 'log_stream');

test('the seed is the script output', () => {
  const seed = readFileSync(new URL('../server/seeds/integrations/azure-active-directory/dashboards.json', import.meta.url), 'utf8');
  assert.equal(seed, JSON.stringify([dashboard], null, 2) + '\n', 'run node scripts/entra-id-dashboard.mjs');
});

test('one dashboard with our logo, over the Entra ID logs only', () => {
  assert.equal(dashboard.id, 'azure-active-directory-overview');
  assert.equal(dashboard.logo, 'azure-active-directory');
  assert.ok(all(dashboard.widgets).some((w) => w.definition.type === 'image' && w.definition.logo === 'azure-active-directory'));
  assert.ok(panels.length >= 30 && streams.length >= 5, `${panels.length} panels, ${streams.length} streams`);
  for (const w of panels) for (const r of w.definition.original.requests) for (const q of r.queries) assert.equal(q.data_source, 'logs', w.definition.title);
  for (const q of [...panels.flatMap((w) => w.definition.original.requests.flatMap((r) => r.queries.map((q) => q.search.query))), ...streams.map((w) => w.definition.query)]) {
    assert.match(q, /^source:azure\.activedirectory( |$)/, q);
    assert.doesNotMatch(q, /\(@|\) OR \(/, `the search syntax has no groups across attributes: ${q}`);
  }
});

test('every panel shows data in mock mode', () => {
  const to = Date.now();
  const range = { from: to - 7 * 86_400_000, to };
  for (const w of panels) {
    for (const r of w.definition.original.requests) {
      for (const q of r.queries) {
        const buckets = mockAggregateLogs(logRequest(q, (s) => s, range, false)).data.buckets;
        assert.ok(buckets.length > 0 && buckets.some((b) => b.computes.c0 > 0), `${w.definition.title}: ${q.search.query} ${JSON.stringify(q.group_by)}`);
      }
    }
  }
});
