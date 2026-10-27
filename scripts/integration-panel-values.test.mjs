import assert from 'node:assert/strict';
import test from 'node:test';
import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';
const require = createRequire(new URL('../web/package.json', import.meta.url));
const { outputFiles } = require('esbuild').buildSync({
  entryPoints: [fileURLToPath(new URL('../web/src/api/integrations/panels.ts', import.meta.url))],
  bundle: true,
  platform: 'node',
  format: 'esm',
  write: false,
  define: { 'import.meta.env': '{}' },
});
const { queryIntegrationPanel } = await import(`data:text/javascript;base64,${Buffer.from(outputFiles[0].text).toString('base64')}`);

test('panel formulas keep raw CPU values and units, and remove non-Job groups before projection', async () => {
  const def = {
    type: 'query_table',
    requests: [{ queries: [{ name: 'cpu', query: 'sum:kubernetes.cpu.usage.total{*} by {kube_job}', data_source: 'metrics' }], formulas: [{ formula: 'exclude_null(cpu)', alias: 'CPU usage' }] }],
  };
  const result = await queryIntegrationPanel(
    def,
    (q) => q,
    { from: 0, to: 60000 },
    {
      catalog: async () => [{ name: 'kubernetes.cpu.usage.total', kind: 'gauge', unit: 'n{core}', description: '', tags: [] }],
      timeseries: async () => [
        { name: 'kube_job:N/A', tags: { kube_job: 'N/A' }, points: [[0, 2600000000]] },
        { name: 'kube_job:inventory-reconcile', tags: { kube_job: 'inventory-reconcile' }, points: [[0, 0]] },
      ],
    },
  );
  assert.equal(result.length, 1);
  assert.equal(result[0].unit, 'n{core}');
  assert.equal(result[0].points[0][1], 0);
  assert.equal(result[0].group, 'kube_job:inventory-reconcile');
});
