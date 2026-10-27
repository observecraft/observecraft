import assert from 'node:assert/strict';
import test from 'node:test';
import { createRequire } from 'node:module';
import { pathToFileURL } from 'node:url';
import { cachedPanelSource } from '../web/src/api/integrations/panelQueries.ts';
const require = createRequire(new URL('../web/package.json', import.meta.url));
const { QueryClient } = await import(pathToFileURL(require.resolve('@tanstack/query-core')).href);
const request = { queries: ['sum:kubernetes.pods.running{*}'], range: { from: 0, to: 60000 }, points: 120 };

test('many panels share one catalog fetch and identical metric requests', async () => {
  const client = new QueryClient();
  let catalogs = 0,
    queries = 0;
  const source = cachedPanelSource(client, {
    catalog: async (signal) => {
      assert.ok(signal instanceof AbortSignal);
      catalogs++;
      return [];
    },
    timeseries: async () => {
      queries++;
      return [{ name: '*', points: [[1000, 23]] }];
    },
  });
  await Promise.all(Array.from({ length: 40 }, () => source.catalog()));
  const data = await Promise.all(Array.from({ length: 40 }, () => source.timeseries(request)));
  assert.equal(catalogs, 1);
  assert.equal(queries, 1);
  assert.equal(data[0][0].points[0][1], 23);
  await source.timeseries({ ...request, range: { from: 60000, to: 120000 } });
  assert.equal(queries, 2);
  client.clear();
  await source.catalog();
  assert.equal(catalogs, 2);
  client.clear();
});

test('a failed fetch is surfaced and can be retried without stale failure caching', async () => {
  const client = new QueryClient();
  let attempts = 0;
  const source = cachedPanelSource(client, {
    catalog: async () => {
      if (++attempts === 1) throw new Error('backend unavailable');
      return [];
    },
    timeseries: async () => [],
  });
  await assert.rejects(source.catalog(), /backend unavailable/);
  assert.deepEqual(await source.catalog(), []);
  assert.equal(attempts, 2);
  client.clear();
});
