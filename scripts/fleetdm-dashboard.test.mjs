// The FleetDM Overview dashboard (scripts/fleetdm-dashboard.mjs) over the FleetDM integration's metrics and activities.
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { registerHooks } from 'node:module';
import test from 'node:test';
import { FLEETDM_UNITS, buildDashboard } from './fleetdm-dashboard.mjs';

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
const { FLEETDM_METRICS, FLEETDM_SETS } = await import('../web/src/api/mock/fleetdmMetrics.ts');

const dashboard = buildDashboard();
const all = (widgets) => widgets.flatMap((w) => [w, ...(w.definition.type === 'group' ? all(w.definition.widgets) : [])]);
const panels = all(dashboard.widgets).filter((w) => w.definition.type === 'integration');
const queries = panels.flatMap((w) => w.definition.original.requests.flatMap((r) => r.queries.map((q) => ({ title: w.definition.title, ...q }))));

test('the seed is the script output', () => {
  const seed = readFileSync(new URL('../server/seeds/integrations/fleetdm/dashboards.json', import.meta.url), 'utf8');
  assert.equal(seed, JSON.stringify([dashboard], null, 2) + '\n', 'run node scripts/fleetdm-dashboard.mjs');
});

test('one dashboard with our logo, its groups in order', () => {
  assert.equal(dashboard.id, 'fleetdm-overview');
  assert.equal(dashboard.logo, 'fleetdm');
  assert.ok(all(dashboard.widgets).some((w) => w.definition.type === 'image' && w.definition.logo === 'fleetdm'));
  assert.deepEqual(
    dashboard.widgets.map((w) => w.definition.title),
    ['FleetDM Overview', 'Overview', 'Hosts', 'Policies and security', 'Vulnerabilities', 'MDM and disk encryption', 'Activity feed'],
  );
  assert.ok(Object.keys(FLEETDM_UNITS).every((m) => m.startsWith('fleetdm.')));
});

test('every metric charted has mock data with the tags it is grouped by', () => {
  for (const q of queries.filter((q) => q.data_source === 'metrics')) {
    const [, name, filter, by] = /^[a-z]+:([a-z0-9_.]+)\{([^}]*)\}(?: by \{([^}]*)\})?/.exec(q.query);
    const def = FLEETDM_METRICS[name];
    assert.ok(def, `${q.title}: ${name}`);
    const sets = FLEETDM_SETS[def.set];
    for (const tag of [...(by ? by.split(',') : []), ...(filter === '*' ? [] : filter.split(',').map((f) => f.split(':')[0]))]) {
      assert.ok(
        sets.some((t) => tag in t),
        `${q.title}: ${name} has no ${tag}`,
      );
    }
  }
});

test('every activity panel shows data in mock mode', () => {
  const to = Date.now();
  const range = { from: to - 7 * 86_400_000, to };
  for (const q of queries.filter((q) => q.data_source === 'logs')) {
    const buckets = mockAggregateLogs(logRequest(q, (s) => s, range, false)).data.buckets;
    assert.ok(
      buckets.some((b) => b.computes.c0 > 0),
      `${q.title}: ${q.search.query}`,
    );
  }
});
