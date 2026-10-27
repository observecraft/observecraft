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
const { collectDashboardQueries, mergeDashboardFrame } = await import(`data:text/javascript;base64,${Buffer.from(outputFiles[0].text).toString('base64')}`);
const dashboards = JSON.parse(readFileSync(new URL('../server/seeds/integrations/kubernetes/dashboards.json', import.meta.url)));
const resolve = (query) => query.replace(/\$\w+(?:\.value)?/g, '*');

test('the complete dashboard plan is deduplicated and fits one request, including nested groups', () => {
  for (const dashboard of dashboards) {
    const queries = collectDashboardQueries(dashboard.widgets, resolve, { from: 0, to: 3600000 });
    assert.ok(queries.length > 0 && queries.length <= 256, dashboard.title);
    assert.equal(new Set(queries.map((q) => JSON.stringify([q.query, q.points]))).size, queries.length);
    assert.equal(new Set(queries.map((q) => q.id)).size, queries.length);
    const duplicate = collectDashboardQueries([...dashboard.widgets, ...dashboard.widgets], resolve, { from: 0, to: 3600000 });
    assert.deepEqual(duplicate, queries);
    console.log(`${dashboard.title}: ${queries.length} unique queries, one dashboard request`);
  }
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
