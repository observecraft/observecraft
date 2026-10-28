import assert from 'node:assert/strict';
import test from 'node:test';
import { registerHooks } from 'node:module';

// The web sources use extensionless relative imports (bundler resolution); resolve them to `.ts` here.
registerHooks({
  resolve(specifier, context, next) {
    try {
      return next(specifier, context);
    } catch (err) {
      if (specifier.startsWith('.')) return next(`${specifier}.ts`, context);
      throw err;
    }
  },
});
const layout = await import('../web/src/pages/infrastructure/cloudMapLayout.ts');
const mock = await import('../web/src/api/mock/cloudMap.ts');

const node = (id, extra = {}) => ({ id, name: id, resourceType: 'kube_deployment', health: 'ok', service: null, alerts: 0, errorRate: null, aggregate: null, resource: null, ...extra });
const group = (key, value, nodes = [], groups = []) => ({ key, value, count: nodes.length, health: 'ok', groups, nodes });

test('isometric projection maps the ground axes to the two diagonals', () => {
  const unit = 10;
  const cos = Math.cos(Math.PI / 6) * unit;
  const p = layout.project(1, 0, 0, 'iso', unit);
  assert.ok(Math.abs(p.x - cos) < 1e-9 && Math.abs(p.y - 5) < 1e-9, 'x goes right and down');
  const q = layout.project(0, 1, 0, 'iso', unit);
  assert.ok(Math.abs(q.x + cos) < 1e-9 && Math.abs(q.y - 5) < 1e-9, 'y goes left and down');
  assert.deepEqual(layout.project(0, 0, 1, 'iso', unit), { x: 0, y: -10 }, 'z goes up');
  assert.deepEqual(layout.project(2, 3, 5, 'flat', unit), { x: 20, y: 30 }, '2D ignores height');
  assert.equal(layout.planeTransform(1, 2, 0, 'flat', 10), 'translate(10 20)');
  assert.match(layout.planeTransform(0, 0, 0, 'iso', 10), /^matrix\(0\.87 0\.5 -0\.87 0\.5 0 0\)$/);
});

test('boxes show their top and the two sides facing the viewer', () => {
  const iso = layout.boxFaces(0, 0, 2, 1, 0, 1, 'iso', 10);
  assert.ok(iso.top && iso.left && iso.right);
  assert.equal(iso.top.split(' ').length, 4);
  const flat = layout.boxFaces(0, 0, 2, 1, 0, 1, 'flat', 10);
  assert.equal(flat.top, '0,0 20,0 20,10 0,10');
  assert.equal(flat.left, null);
});

test('blocks fill a near-square grid inside their group, groups pack in rows', () => {
  assert.equal(layout.gridColumns(0), 0);
  assert.equal(layout.gridColumns(1), 1);
  assert.equal(layout.gridColumns(10), 4);
  const packed = layout.shelfPack(
    [
      { w: 4, d: 2 },
      { w: 4, d: 3 },
      { w: 4, d: 1 },
    ],
    9,
    1,
  );
  assert.deepEqual(packed.positions, [
    { x: 0, y: 0 },
    { x: 5, y: 0 },
    { x: 0, y: 4 },
  ]);
  assert.deepEqual([packed.w, packed.d], [9, 5]);

  const root = group(
    'root',
    '',
    [],
    [
      group('cluster', 'shop-prod-eu', [], [group('namespace', 'checkout', [node('a'), node('b'), node('c')]), group('namespace', 'payments', [node('d')])]),
      group('cluster', 'shop-staging', [], [group('namespace', 'catalog', [node('e')])]),
    ],
  );
  const out = layout.layoutDiagram(root);
  assert.equal(out.groups.length, 5);
  assert.equal(out.blocks.length, 5);
  const byId = Object.fromEntries(out.groups.map((g) => [g.group.value, g]));
  // Children sit inside their parent, behind its label strip (on the front edge); levels nest.
  const prod = byId['shop-prod-eu'];
  const checkout = byId.checkout;
  assert.equal(prod.level, 0);
  assert.equal(checkout.level, 1);
  assert.ok(checkout.x >= prod.x + layout.PAD && checkout.y >= prod.y + layout.PAD);
  assert.ok(checkout.y + checkout.d <= prod.y + prod.d - layout.HEADER + 1e-9);
  assert.ok(checkout.x + checkout.w <= prod.x + prod.w && checkout.y + checkout.d <= prod.y + prod.d);
  for (const b of out.blocks.filter((b) => ['a', 'b', 'c'].includes(b.id))) {
    assert.ok(b.x >= checkout.x && b.x + layout.BLOCK <= checkout.x + checkout.w && b.y >= checkout.y + layout.PAD && b.y + layout.BLOCK <= checkout.y + checkout.d - layout.HEADER + 1e-9, b.id);
  }
  // Top-level groups don't overlap.
  const staging = byId['shop-staging'];
  assert.ok(staging.x >= prod.x + prod.w || staging.y >= prod.y + prod.d);
  // Groups are wide enough for their label.
  const narrow = layout.layoutDiagram(group('root', '', [], [group('namespace', 'a-rather-long-namespace-name', [node('z')])]));
  assert.ok(narrow.groups[0].w >= layout.labelWidth(narrow.groups[0].group));
  assert.ok(layout.labelWidth(group('namespace', 'a-rather-long-namespace-name')) > layout.labelWidth(group('namespace', 'x')));
  // An ungrouped diagram is one platform.
  assert.equal(layout.layoutDiagram(group('root', '', [node('x')])).groups.length, 1);
  assert.equal(layout.layoutDiagram(group('root', '')).blocks.length, 0);
});

test('paint order is back to front; services anchor on their workload', () => {
  const items = [
    { id: 'front', x: 5, y: 5, level: 0 },
    { id: 'back', x: 0, y: 0, level: 0 },
    { id: 'raised', x: 0, y: 0, level: 1 },
  ];
  assert.deepEqual(
    layout.paintOrder(items).map((i) => i.id),
    ['back', 'front', 'raised'],
  );
  const blocks = [
    { id: 'svc', x: 0, y: 0, level: 0, node: node('svc', { resourceType: 'kube_service', service: 'billing' }) },
    { id: 'dep', x: 1, y: 0, level: 0, node: node('dep', { service: 'billing' }) },
    { id: 'none', x: 2, y: 0, level: 0, node: node('none') },
  ];
  const anchors = layout.serviceAnchors(blocks);
  assert.equal(anchors.get('billing').id, 'dep');
  assert.equal(anchors.size, 1);
});

test('fit, zoom around the cursor and on-screen checks', () => {
  const v = layout.fitView({ minX: 0, minY: 0, maxX: 100, maxY: 50 }, 300, 200, 50, 10);
  assert.equal(v.k, 2);
  assert.deepEqual([v.tx, v.ty], [50, 50]);
  const z = layout.zoomAt(v, 2, 150, 100);
  // The point under the cursor stays put.
  assert.deepEqual([(150 - v.tx) / v.k, (100 - v.ty) / v.k], [(150 - z.tx) / z.k, (100 - z.ty) / z.k]);
  assert.equal(layout.zoomAt(v, 1000, 0, 0).k, 6);
  assert.ok(layout.onScreen({ x: 10, y: 10 }, v));
  assert.ok(!layout.onScreen({ x: 1000, y: 10 }, v));
});

test('mock diagram: Kubernetes by cluster and namespace, hosts by cloud, overlays', async () => {
  const k8s = await mock.getCloudMap({ provider: 'kubernetes', query: '', groupBy: [], overlay: 'default' });
  assert.deepEqual(k8s.groupBy, ['cluster', 'namespace']);
  assert.deepEqual(
    k8s.root.groups.map((g) => g.value),
    ['shop-prod-eu', 'shop-staging'],
  );
  const namespaces = k8s.root.groups[0].groups.map((g) => g.value);
  for (const ns of ['checkout', 'catalog', 'payments']) assert.ok(namespaces.includes(ns), ns);
  assert.equal(k8s.providers.find((p) => p.provider === 'hosts').clouds.includes('aws'), true);

  const hosts = await mock.getCloudMap({ provider: 'hosts', query: '', groupBy: [], overlay: 'default' });
  assert.ok(hosts.total > 400);
  // Big groups fold into "+N more" blocks.
  const all = [];
  const walk = (g) => (all.push(...g.nodes), g.groups.forEach(walk));
  walk(hosts.root);
  assert.ok(all.some((n) => n.aggregate));
  assert.ok(all.length < hosts.total);
  assert.ok(layout.layoutDiagram(hosts.root).blocks.length === all.length);

  const health = await mock.getCloudMap({ provider: 'kubernetes', query: 'env:prod', groupBy: ['namespace'], overlay: 'health' });
  const flat = [];
  const walk2 = (g) => (flat.push(...g.nodes), g.groups.forEach(walk2));
  walk2(health.root);
  const inventory = flat.find((n) => n.name === 'inventory-service' && n.resourceType === 'kube_deployment');
  assert.equal(inventory.health, 'error');
  assert.ok(inventory.alerts > 0 && inventory.errorRate > 0.05);

  const deps = await mock.getCloudMap({ provider: 'kubernetes', query: '', groupBy: [], overlay: 'dependencies' });
  assert.ok(deps.edges.some((e) => e.from === 'api-gateway' && e.to === 'storefront-api'));
  await assert.rejects(mock.getCloudMap({ provider: 'hosts', query: '', groupBy: ['password'], overlay: 'default' }), /Can't group by/);
});

test('mock diagram: many small groups merge into Other', () => {
  const nodes = Array.from({ length: 40 }, (_, i) => node(`n${i}`, { resource: { resourceType: 'host', tags: [`team:t${i}`], attributes: {}, team: `t${i}` } }));
  const tree = mock.buildTree(nodes, ['team']);
  assert.equal(tree.groups.length, mock.MAX_GROUPS_PER_LEVEL);
  assert.equal(tree.groups.at(-1).value, '__other__');
  assert.equal(tree.groups.at(-1).count, 40 - mock.MAX_GROUPS_PER_LEVEL + 1);
});
