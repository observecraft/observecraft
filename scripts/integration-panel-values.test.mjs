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

test('a scalar panel applies a function over time (derivative) to the series, then reduces with the query aggregator', async () => {
  const def = {
    type: 'query_value',
    requests: [
      {
        queries: [{ name: 'query1', query: 'avg:clickhouse.events.LogError.total{*}', data_source: 'metrics', aggregator: 'last' }],
        formulas: [{ formula: 'derivative(query1)' }],
        response_format: 'scalar',
      },
    ],
  };
  const asked = [];
  const result = await queryIntegrationPanel(
    def,
    (q) => q,
    { from: 0, to: 180_000 },
    {
      catalog: async () => [{ name: 'clickhouse.events.LogError.total', kind: 'gauge', description: '', tags: [] }],
      timeseries: async (req) => (
        asked.push(req),
        [
          {
            name: '*',
            points: [
              [0, 100],
              [60_000, 160],
              [120_000, 400],
              [180_000, 460],
            ],
          },
        ]
      ),
    },
  );
  assert.equal(asked[0].aggregator, undefined, 'every point, not one value per series');
  assert.equal(result.length, 1);
  assert.deepEqual(result[0].points, [[180_000, 1]], 'the last change per second');
});

test('a formula’s number_format unit is the unit its values show in (ClickHouse’s bytes, bytes per second, percent)', async () => {
  const unit = async (number_format) =>
    (
      await queryIntegrationPanel(
        {
          type: 'query_value',
          requests: [
            {
              queries: [{ name: 'query1', query: 'avg:clickhouse.asynchronous_metrics.TotalBytesOfMergeTreeTables{*}', data_source: 'metrics', aggregator: 'avg' }],
              formulas: [{ formula: 'query1', number_format }],
              response_format: 'scalar',
            },
          ],
        },
        (q) => q,
        { from: 0, to: 60_000 },
        { catalog: async () => [], timeseries: async () => [{ name: '*', points: [[60_000, 7.8e11]] }] },
      )
    )[0].unit;
  assert.equal(await unit({ unit: { type: 'canonical_unit', unit_name: 'byte_in_decimal_bytes_family' } }), 'byte');
  assert.equal(await unit({ unit: { type: 'canonical_unit', unit_name: 'byte_in_decimal_bytes_family', per_unit_name: 'second' } }), 'byte/second');
  assert.equal(await unit({ unit: { type: 'canonical_unit', unit_name: 'percent' } }), 'percent');
  assert.equal(await unit({ unit: { type: 'custom_unit_label', label: 'cores' } }), 'cores');
  assert.equal(await unit({ unit: { type: 'canonical_unit' } }), undefined, 'no unit named: the metric’s');
});
