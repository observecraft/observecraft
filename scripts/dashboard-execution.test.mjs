import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';
import { readEventStream } from '../web/src/api/eventStream.ts';
import { scalarMetricValue, groupedMetricValues } from '../web/src/api/dashboards/reduce.ts';
const require = createRequire(new URL('../web/package.json', import.meta.url));
const { outputFiles } = require('esbuild').buildSync({
  entryPoints: [fileURLToPath(new URL('../web/src/api/dashboards/execution.ts', import.meta.url))],
  bundle: true,
  platform: 'node',
  format: 'esm',
  write: false,
});
const { collectDashboardQueries, mergeDashboardFrame, decodeDashboardFrame } = await import(`data:text/javascript;base64,${Buffer.from(outputFiles[0].text).toString('base64')}`);
const dashboards = JSON.parse(readFileSync(new URL('../server/seeds/integrations/kubernetes/dashboards.json', import.meta.url)));
const resolve = (query) => query.replace(/\$\w+(?:\.value)?/g, '*');

test('the complete dashboard plan is deduplicated and fits one request, including nested groups', () => {
  for (const dashboard of dashboards) {
    const queries = collectDashboardQueries(dashboard.widgets, resolve, { from: 0, to: 3600000 });
    assert.ok(queries.length > 0 && queries.length <= 256, dashboard.title);
    assert.equal(new Set(queries.map((q) => JSON.stringify([q.query, q.points, q.aggregator]))).size, queries.length);
    assert.equal(new Set(queries.map((q) => q.id)).size, queries.length);
    const duplicate = collectDashboardQueries([...dashboard.widgets, ...dashboard.widgets], resolve, { from: 0, to: 3600000 });
    assert.deepEqual(duplicate, queries);
    console.log(`${dashboard.title}: ${queries.length} unique queries, one dashboard request`);
  }
});

test('tables and treemaps ask the server for one value per series; timeseries keep their points', () => {
  const dashboard = dashboards.find((d) => d.id === 'kubernetes-kubernetes-cpu-memory');
  const queries = collectDashboardQueries(dashboard.widgets, resolve, { from: 0, to: 3600000 });
  const usage = 'sum:kubernetes.cpu.usage.total{*,*,*,*,*,*,*} by {kube_cluster_name,env,kube_deployment,kube_daemon_set,kube_stateful_set}';
  // The same query feeds a table (avg over the range) and a timeseries (every point): two entries.
  assert.deepEqual(
    queries
      .filter((q) => q.query === usage)
      .map((q) => q.aggregator ?? null)
      .sort(),
    ['avg', null],
  );
  const reduced = queries.filter((q) => q.aggregator).length;
  assert.ok(reduced > queries.length / 2, `${reduced} of ${queries.length} queries reduced on the server`);
});

test('queries whose points are changed before aggregating stay whole', () => {
  const table = (query, aggregator) => ({
    definition: {
      type: 'integration',
      original: { type: 'query_table', requests: [{ response_format: 'scalar', queries: [{ name: 'query1', data_source: 'metrics', query, ...(aggregator ? { aggregator } : {}) }] }] },
    },
  });
  const plan = (widgets) => collectDashboardQueries(widgets, resolve, { from: 0, to: 3600000 }).map((q) => [q.query, q.aggregator ?? null]);
  assert.deepEqual(plan([table('sum:a{*} by {host}', 'last')]), [['sum:a{*} by {host}', 'last']]);
  assert.deepEqual(plan([table('sum:b{*} by {host}', 'mean')]), [['sum:b{*} by {host}', 'avg']]);
  assert.deepEqual(plan([table('sum:c{*} by {host}')]), [['sum:c{*} by {host}', 'avg']]);
  // fill() adds points and as_count() scales by the gap between points: the panel needs them all.
  assert.deepEqual(plan([table('sum:d{*} by {host}.fill(zero)', 'avg')]), [['sum:d{*} by {host}', null]]);
  assert.deepEqual(plan([table('sum:e{*} by {host}.as_count()', 'sum')]), [['sum:e{*} by {host}', null]]);
  // An aggregation the server doesn't have stays in the panel.
  assert.deepEqual(plan([table('sum:f{*} by {host}', 'percentile')]), [['sum:f{*} by {host}', null]]);
  // A query value with a timeseries background also draws the series over time.
  const value = {
    definition: {
      type: 'integration',
      original: {
        type: 'query_value',
        timeseries_background: { type: 'area' },
        requests: [{ response_format: 'scalar', queries: [{ name: 'query1', data_source: 'metrics', query: 'sum:g{*}', aggregator: 'avg' }] }],
      },
    },
  };
  assert.deepEqual(plan([value]).sort(), [
    ['sum:g{*}', null],
    ['sum:g{*}', 'avg'],
  ]);
});

test('series sent as values every step from a start decode to points; points stay as they are', () => {
  const frame = {
    range: { from: 0, to: 60000 },
    data_scans: 1,
    unique_queries: 2,
    results: [
      { id: 'q0', kind: 'gauge', step_ms: 10000, replace_from: 0, series: [{ name: 'a', tags: {}, start: 10000, values: [1, null, 3.5, 2] }] },
      {
        id: 'q1',
        kind: 'gauge',
        step_ms: 10000,
        replace_from: 0,
        series: [
          {
            name: 'b',
            tags: {},
            points: [
              [0, 1],
              [15000, 2],
            ],
          },
        ],
      },
    ],
  };
  const decoded = decodeDashboardFrame(frame);
  assert.deepEqual(decoded.results[0].series, [
    {
      name: 'a',
      tags: {},
      points: [
        [10000, 1],
        [30000, 3.5],
        [40000, 2],
      ],
    },
  ]);
  assert.deepEqual(decoded.results[1].series, [
    {
      name: 'b',
      tags: {},
      points: [
        [0, 1],
        [15000, 2],
      ],
    },
  ]);
});

test('live bucket replacement removes old points, vanished groups and expired history', () => {
  const frame = (results, from) => ({ results, range: { from, to: 40000 }, data_scans: 1, unique_queries: 1, error: null });
  const result = (series, replace_from) => ({ id: 'q0', kind: 'gauge', step_ms: 10000, replace_from, error: null, series });
  const before = frame(
    [
      result(
        [
          {
            name: 'a',
            points: [
              [0, 1],
              [10000, 2],
              [20000, 3],
            ],
          },
          { name: 'b', points: [[20000, 4]] },
        ],
        0,
      ),
    ],
    0,
  );
  const after = frame(
    [
      result(
        [
          {
            name: 'a',
            points: [
              [20000, 8],
              [30000, 9],
            ],
          },
        ],
        20000,
      ),
    ],
    15000,
  );
  const merged = mergeDashboardFrame(before, after, false);
  assert.deepEqual(merged.results[0].series, [
    {
      name: 'a',
      points: [
        [10000, 2],
        [20000, 8],
        [30000, 9],
      ],
    },
  ]);
  assert.deepEqual(
    before.results[0].series[0].points,
    [
      [0, 1],
      [10000, 2],
      [20000, 3],
    ],
    'does not mutate the previous frame',
  );
  assert.deepEqual(mergeDashboardFrame(before, after, true), after);
});

test('SSE survives arbitrary chunk and UTF-8 boundaries and ignores keepalives', async () => {
  const wire = new TextEncoder().encode(': heartbeat\r\nevent: snapshot\r\ndata: {"label":"caf\u00e9",\r\ndata: "n":1}\r\n\r\nevent: delta\ndata: {"n":2}\n\n');
  const chunks = new ReadableStream({
    start(controller) {
      for (const byte of wire) controller.enqueue(Uint8Array.of(byte));
      controller.close();
    },
  });
  const events = [];
  await readEventStream(chunks, (event, data) => events.push([event, data]));
  assert.deepEqual(events, [
    ['snapshot', { label: 'caf\u00e9', n: 1 }],
    ['delta', { n: 2 }],
  ]);
});

test('scalar and grouped projections keep time aggregation separate from space aggregation', () => {
  const series = [
    {
      name: 'host:a',
      points: [
        [0, 2],
        [10000, 4],
      ],
    },
    {
      name: 'host:b',
      points: [
        [0, 6],
        [10000, 8],
      ],
    },
  ];
  assert.equal(scalarMetricValue(series, 'sum:x{*} by {host}', 'avg'), 10);
  assert.equal(scalarMetricValue(series, 'avg:x{*} by {host}', 'last'), 6);
  assert.deepEqual(groupedMetricValues(series, 'sum:x{*} by {host}', 'avg'), [
    { group: 'host:b', tags: { host: 'b' }, value: 7 },
    { group: 'host:a', tags: { host: 'a' }, value: 3 },
  ]);
});
