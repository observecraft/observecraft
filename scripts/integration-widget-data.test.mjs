// Data of the geomap, distribution, hostmap and sunburst (pie) widgets (web/src/api/integrations/widgetData.ts).
import assert from 'node:assert/strict';
import test from 'node:test';
import { countryValues, distributionBuckets, hostmapColor, sunburstTree } from '../web/src/api/integrations/widgetData.ts';

test('a geomap sums each country’s groups by ISO code and leaves the rest out', () => {
  assert.deepEqual(
    countryValues([
      { name: 'US', value: 10 },
      { name: 'fr', value: 2 },
      { name: 'US', value: 5 },
      { name: 'N/A', value: 7 },
      { name: '*', value: 1 },
    ]),
    [
      { iso: 'US', value: 15 },
      { iso: 'FR', value: 2 },
    ],
  );
});

test('a distribution counts the values in even buckets between their minimum and maximum', () => {
  assert.deepEqual(distributionBuckets([1, 2, 2, 3, 10], 3), [
    { from: 1, to: 4, count: 4 },
    { from: 4, to: 7, count: 0 },
    { from: 7, to: 10, count: 1 },
  ]);
  assert.deepEqual(distributionBuckets([5, 5], 4), [{ from: 5, to: 5, count: 2 }], 'one value: one bucket');
  assert.deepEqual(distributionBuckets([], 4), []);
});

test('a host map colours a value by where it falls between the smallest and largest', () => {
  assert.equal(hostmapColor(0, 0, 100), hostmapColor(-5, 0, 100), 'clamped');
  assert.notEqual(hostmapColor(0, 0, 100), hostmapColor(100, 0, 100));
  assert.match(hostmapColor(50, 0, 100), /^#[0-9a-f]{6}$/);
  assert.equal(hostmapColor(3, 3, 3), hostmapColor(0, 0, 1), 'all equal: the lowest colour');
});

test('a sunburst has a ring per group-by tag, inner tag first, named by tag values, largest first', () => {
  const tree = sunburstTree([
    { name: 'entrypoint:web, protocol:http', value: 30 },
    { name: 'entrypoint:web, protocol:https', value: 50 },
    { name: 'entrypoint:metrics, protocol:http', value: 20 },
    { name: 'query1 · entrypoint:websecure, protocol:https', value: 0 },
  ]);
  assert.equal(tree.total, 100);
  assert.deepEqual(tree.nodes, [
    {
      name: 'web',
      value: 80,
      children: [
        { name: 'https', value: 50, children: [] },
        { name: 'http', value: 30, children: [] },
      ],
    },
    { name: 'metrics', value: 20, children: [{ name: 'http', value: 20, children: [] }] },
  ]);
  assert.deepEqual(tree.legend, [
    { name: 'web', value: 80, percent: 80 },
    { name: 'metrics', value: 20, percent: 20 },
  ]);
});

test('an ungrouped or single-tag sunburst is one ring; negative values are left out', () => {
  const tree = sunburstTree([
    { name: 'service:a', value: 3 },
    { name: 'service:b', value: -1 },
    { name: 'service:c', value: 1 },
  ]);
  assert.deepEqual(
    tree.nodes.map((n) => [n.name, n.value, n.children.length]),
    [
      ['a', 3, 0],
      ['c', 1, 0],
    ],
  );
  assert.equal(sunburstTree([{ name: '*', value: 4 }]).nodes[0].name, 'All');
});
