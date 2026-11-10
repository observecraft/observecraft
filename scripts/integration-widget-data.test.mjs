// Data of the geomap, distribution, hostmap and sunburst (pie) widgets (web/src/api/integrations/widgetData.ts).
import assert from 'node:assert/strict';
import test from 'node:test';
import { countryValues, distributionBuckets, groupLabel, hostmapColor, hostmapGroups, hostmapQuery, hostmapScope, sunburstTree } from '../web/src/api/integrations/widgetData.ts';

test('a group reads without its query and tag keys', () => {
  assert.equal(groupLabel('query1 · country_code:RO'), 'RO');
  assert.equal(groupLabel('os_version:macOS 15.1, team:Workstations'), 'macOS 15.1, Workstations');
  assert.equal(groupLabel('policy:Screen lock: 15 minutes'), 'Screen lock: 15 minutes', 'only the first colon is the key');
  assert.equal(groupLabel('US'), 'US');
});

test('a geomap reads metric groups by their country tag', () => {
  assert.deepEqual(
    countryValues([
      { name: 'country_code:RO', value: 3 },
      { name: 'query1 · country_code:ro', value: 1 },
    ]),
    [{ iso: 'RO', value: 4 }],
  );
});

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

test('a host map reads its fill per host, or per container, split by its group tags', () => {
  const gatekeeper = {
    type: 'hostmap',
    requests: { fill: { q: 'avg:process.stat.container.cpu.total_pct{short_image:gatekeeper,$cluster_name} by {host}' } },
    node_type: 'container',
    group: ['kube_deployment'],
  };
  assert.equal(hostmapQuery(gatekeeper), 'avg:process.stat.container.cpu.total_pct{short_image:gatekeeper,$cluster_name} by {container_id,kube_deployment}');
  const argocd = { type: 'hostmap', requests: { fill: { q: 'avg:system.load.1{field:apps:argocd} by {host}' } }, node_type: 'host', group: ['cluster_name'] };
  assert.equal(hostmapQuery(argocd), 'avg:system.load.1{field:apps:argocd} by {host,cluster_name}');
  assert.equal(hostmapQuery({ type: 'hostmap', requests: { fill: { q: 'avg:system.cpu.user{*}' } } }), 'avg:system.cpu.user{*} by {host}');
  assert.equal(hostmapQuery({ type: 'hostmap', requests: {} }), undefined);
});

test('a host map draws a box per group value, its nodes by name', () => {
  const groups = hostmapGroups(
    [
      { name: 'container_id:c2, kube_deployment:gatekeeper-controller-manager', value: 4 },
      { name: 'container_id:c9, kube_deployment:gatekeeper-audit', value: 12 },
      { name: 'container_id:c1, kube_deployment:gatekeeper-controller-manager', value: 3 },
    ],
    ['kube_deployment'],
  );
  assert.deepEqual(groups, [
    { label: 'gatekeeper-audit', nodes: [{ name: 'c9', value: 12 }] },
    {
      label: 'gatekeeper-controller-manager',
      nodes: [
        { name: 'c1', value: 3 },
        { name: 'c2', value: 4 },
      ],
    },
  ]);
  assert.deepEqual(hostmapGroups([{ name: 'host:a', value: 1 }], []), [{ label: '', nodes: [{ name: 'a', value: 1 }] }]);
});

test('an empty host map names the metric and the scope it looked in, without the unset variables', () => {
  assert.equal(hostmapScope('avg:system.load.1{field:apps:argocd,$host,$kube_cluster_name} by {host,cluster_name}'), 'system.load.1{field:apps:argocd}');
  assert.equal(hostmapScope('avg:process.stat.container.cpu.total_pct{$cluster_name} by {container_id}'), 'process.stat.container.cpu.total_pct{*}');
});
