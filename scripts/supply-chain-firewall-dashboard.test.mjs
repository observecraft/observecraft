// Supply Chain Firewall's integrations-core dashboard in mock mode, over the mock scfw logs
// (web/src/api/mock/supplyChainFirewallLogs.ts) shaped as its pipeline parses them.
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
const { mockAggregateLogs, mockSearchLogs } = await import('../web/src/api/mock/index.ts');
const { logRequest } = await import('../web/src/api/integrations/logs.ts');
const { resolveSearchQuery } = await import('../web/src/lib/metricQuery.ts');

const mock = JSON.parse(readFileSync(new URL('../web/src/api/integrations/coreDashboards.generated.json', import.meta.url), 'utf8'));
const dashboard = mock.dashboards.find((d) => d.logo === 'supply-chain-firewall');
const all = (widgets) => widgets.flatMap((w) => [w, ...(w.definition.type === 'group' ? all(w.definition.widgets) : [])]);
const panels = all(dashboard?.widgets ?? []).filter((w) => w.definition.type === 'integration');
const queries = panels.flatMap((w) => w.definition.original.requests.flatMap((r) => (r.queries ?? []).filter((q) => q.data_source === 'logs').map((q) => [w.definition.title, q])));
const streams = all(dashboard?.widgets ?? []).filter((w) => w.definition.type === 'log_stream');
const resolve = (vars) => (q) => resolveSearchQuery(q, dashboard.templateVariables, vars);
/** Facets the log normalizer doesn't keep as Datadog does: arrays of objects aren't facets. */
const UNSUPPORTED = ['@install_targets.name'];

test('mock mode carries the dashboard', () => {
  assert.ok(dashboard, 'supply-chain-firewall is in MOCK_INTEGRATIONS (node scripts/import-integration-dashboards.mjs --mock)');
  assert.ok(queries.length >= 50 && streams.length >= 1, `${queries.length} queries, ${streams.length} streams`);
});

test('every log panel and stream shows data in mock mode', () => {
  const to = Date.now();
  const range = { from: to - 7 * 86_400_000, to };
  for (const [title, q] of queries) {
    if ((q.group_by ?? []).some((g) => UNSUPPORTED.includes(g.facet))) continue;
    const buckets = mockAggregateLogs(logRequest(q, resolve({}), range, false)).data.buckets;
    assert.ok(
      buckets.some((b) => b.computes.c0 > 0),
      `${title}: ${q.search.query} ${JSON.stringify(q.group_by)}`,
    );
  }
  for (const w of streams) assert.ok(mockSearchLogs({ range, query: resolve({})(w.definition.query) }).total > 0, w.definition.query);
});

test('groups across attributes match: warned commands (`(@warning:true OR @warned:true)`)', () => {
  const to = Date.now();
  const range = { from: to - 7 * 86_400_000, to };
  const warned = mockSearchLogs({ range, query: 'source:scfw (@warning:true OR @warned:true)' }).total;
  assert.ok(warned > 0 && warned === mockSearchLogs({ range, query: 'source:scfw @warning:true' }).total);
  assert.equal(mockSearchLogs({ range, query: 'source:scfw -(@warning:true OR @warning:false)' }).total, 0);
});

test('only facets the normalizer lacks stay empty', () => {
  const skipped = queries.filter(([, q]) => (q.group_by ?? []).some((g) => UNSUPPORTED.includes(g.facet)));
  assert.ok(skipped.length <= 6, `${skipped.length} of ${queries.length}`);
});

test('the template variables filter the panels', () => {
  const to = Date.now();
  const range = { from: to - 7 * 86_400_000, to };
  const count = (vars) => mockAggregateLogs(logRequest(queries[0][1], resolve(vars), range, false)).data.buckets[0]?.computes.c0 ?? 0;
  const total = count({});
  for (const vars of [{ 'User-Name': 'runner' }, { Action: 'BLOCK' }]) {
    const n = count(vars);
    assert.ok(n > 0 && n < total, `${JSON.stringify(vars)}: ${n} of ${total}`);
  }
});
