// Mock mode (make web-mock) draws every widget of the Vault, Gatekeeper, External Secrets, Argo CD, Traefik Mesh and
// cert-manager dashboards: their metrics, service checks, logs and security signals, template variables at `*`.
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';

// The panel code and the mock behind it import without extensions and read import.meta.env, as the web build does: bundle them.
const require = createRequire(new URL('../web/package.json', import.meta.url));
const { outputFiles } = require('esbuild').buildSync({
  stdin: {
    contents: [
      "export { queryIntegrationPanel } from './api/integrations/panels';",
      "export { searchLogPage, logPatterns } from './api/client';",
      "export { resolveQuery, resolveSearchQuery } from './lib/metricQuery';",
      "export { hostmapQuery } from './api/integrations/widgetData';",
      "export { checkStatusTiles } from './api/integrations/checkStatus';",
    ].join('\n'),
    resolveDir: fileURLToPath(new URL('../web/src', import.meta.url)),
    loader: 'ts',
  },
  bundle: true,
  platform: 'node',
  format: 'esm',
  write: false,
  define: { 'import.meta.env': '{}' },
  logLevel: 'error',
});
const web = await import(`data:text/javascript;base64,${Buffer.from(outputFiles[0].text).toString('base64')}`);
const mock = JSON.parse(readFileSync(new URL('../web/src/api/integrations/coreDashboards.generated.json', import.meta.url), 'utf8'));

/** Widgets whose data mock mode can't have, and why. */
const KNOWN = new Map([
  // Hosts tagged field:apps:argocd by their owner: a tag of the user's own, not one the integration sends.
  ['Host Reporting Argo CD Metrics by Cluster', 'user tag'],
  // A boolean tag filter, `(status:400 OR s) AND $cluster_name`, which metric queries don't support yet.
  ['ACME Request Errors', 'boolean filter'],
  // Signals counted by @evt.name: Vault's audit logs (and so its signals) carry no evt.name.
  ['Security Signals by Events', 'no evt.name'],
]);

const widgets = (items) => items.flatMap((w) => (w.definition.type === 'group' ? widgets(w.definition.widgets) : [w.definition]));

async function hasData(d, def) {
  const range = { from: Date.now() - 3_600_000, to: Date.now() };
  const resolve = (q) => web.resolveQuery(q, d.templateVariables, {});
  if (def.type === 'log_stream') return (await web.searchLogPage({ range, query: web.resolveSearchQuery(def.query, d.templateVariables, {}) }, 50)).logs.length > 0;
  if (def.type !== 'integration') return true;
  const o = def.original;
  if (o.type === 'list_stream' && o.requests?.[0]?.query?.data_source === 'logs_pattern_stream')
    return (await web.logPatterns({ range, query: web.resolveSearchQuery(o.requests[0].query.query_string, d.templateVariables, {}) })).length > 0;
  if (o.type === 'hostmap') return (await web.queryIntegrationPanel({ type: 'query_table', requests: [{ q: web.hostmapQuery(o) }] }, resolve, range)).length > 0;
  const series = await web.queryIntegrationPanel(o, resolve, range);
  if (o.type === 'check_status') return web.checkStatusTiles(o, series).length > 0;
  return series.some((s) => s.points.length > 0);
}

for (const title of [
  'Vault - Overview (OpenMetricsV2)',
  'Vault - Overview (Legacy)',
  'Gatekeeper Overview',
  'External Secrets Operator - Overview',
  'Argo CD Overview',
  'Traefik Mesh Overview',
  'Cert Manager Overview',
]) {
  test(`every widget of ${title} has data in mock mode`, async () => {
    const d = mock.dashboards.find((x) => x.title === title);
    assert.ok(d, title);
    const empty = [];
    for (const def of widgets(d.widgets)) {
      if (KNOWN.has(def.title)) continue;
      try {
        if (!(await hasData(d, def))) empty.push(def.title);
      } catch (error) {
        empty.push(`${def.title}: ${error.message}`);
      }
    }
    assert.deepEqual(empty, []);
  });
}

test('a mock integration has as many instances of a metric as the tags its queries read tell apart', async () => {
  const d = mock.dashboards.find((x) => x.title === 'Argo CD Overview');
  const range = { from: Date.now() - 3_600_000, to: Date.now() };
  const resolve = (q) => web.resolveQuery(q, d.templateVariables, {});
  const controllers = widgets(d.widgets).find((w) => w.title === 'App Controllers').original;
  const [series] = await web.queryIntegrationPanel(controllers, resolve, range);
  assert.equal(series.points.at(-1)[1], 4, 'one application controller per host, not one per gRPC service the API server answers');
});
