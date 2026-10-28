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
const layout = await import('../web/src/pages/dataStreams/mapLayout.ts');
const mock = await import('../web/src/api/mock/dataStreams.ts');
const fmt = await import('../web/src/pages/dataStreams/format.ts');

const HOUR = 3_600_000;
const now = Date.now();
const range = { from: now - HOUR, to: now };
const e = (source, target) => ({ source, target });

test('layers follow the flow: producer, topic, consumer, topic, consumer', () => {
  const ids = ['service:billing', 'service:order-service', 'service:notification-worker', 'topic:kafka:orders.created', 'topic:kafka:payments.authorized'];
  const edges = [
    e('service:order-service', 'topic:kafka:orders.created'),
    e('topic:kafka:orders.created', 'service:billing'),
    e('service:billing', 'topic:kafka:payments.authorized'),
    e('topic:kafka:payments.authorized', 'service:notification-worker'),
  ];
  const l = layout.layers(ids, edges);
  assert.deepEqual(
    ids.map((id) => l.get(id)),
    [2, 0, 4, 1, 3],
  );
});

test('cycles do not loop the layering', () => {
  const ids = ['a', 'b', 't1', 't2'];
  const l = layout.layers(ids, [e('a', 't1'), e('t1', 'b'), e('b', 't2'), e('t2', 'a')]);
  assert.equal(l.get('a'), 0, 'the first node reached starts the walk');
  assert.equal(l.get('t2'), 3);
});

test('columns are centered and ordered by what feeds them', () => {
  const ids = ['p', 'x', 'y', 'q'];
  const g = layout.layoutGraph(ids, [e('p', 'y'), e('q', 'x')], { columnGap: 100, rowGap: 50, margin: 10 });
  assert.equal(g.nodes.get('p').x, 10);
  assert.equal(g.nodes.get('y').x, 110);
  assert.ok(g.nodes.get('y').y < g.nodes.get('x').y, 'y follows p, which sorts first');
  assert.equal(g.width, 120);
  assert.equal(g.height, 70);
});

test('edges leave right and enter left; back edges loop below', () => {
  assert.equal(layout.edgePath({ x: 0, y: 0 }, { x: 200, y: 50 }, 50), 'M50,0 C100,0 100,50 150,50');
  assert.match(layout.edgePath({ x: 200, y: 0 }, { x: 0, y: 0 }, 50), /^M250,0 C330,0 330,70/);
  assert.deepEqual(layout.edgeMid({ x: 200, y: 0 }, { x: 0, y: 10 }), { x: 100, y: 80 });
});

test('the mock map has both edge kinds, lag on inventory-service and pathways end to end', async () => {
  const m = await mock.streamMap({ range });
  assert.equal(m.enabled, true);
  const created = m.nodes.find((n) => n.id === 'topic:kafka:orders.created');
  assert.equal(created.kind, 'topic');
  assert.ok(Math.abs(created.produced - 6.2) < 1e-9, 'published once, delivered twice');
  assert.ok(Math.abs(created.consumed - 12.4) < 1e-9);
  const lagging = m.edges.find((x) => x.target === 'service:inventory-service');
  assert.equal(lagging.kind, 'consume');
  assert.equal(lagging.lagHigh, true);
  assert.deepEqual(lagging.consumerGroups, ['inventory-consumer']);
  assert.equal(m.edges.find((x) => x.target === 'service:billing').lagHigh, false);
  const ids = m.pathways.map((p) => p.id);
  assert.ok(ids.includes('service:order-service>topic:kafka:orders.created>service:billing>topic:kafka:payments.authorized>service:notification-worker'));
  assert.ok(ids.includes('service:workflow-engine>topic:kafka:order-events>service:notification-worker'));
  const viaInventory = m.pathways.find((p) => p.nodes.includes('service:inventory-service'));
  assert.equal(viaInventory.hops, 2);
  assert.equal(viaInventory.lagHigh, true);
  assert.ok(Math.abs(viaInventory.latencyP50Ms - (1450 + 11.2)) < 1e-9, 'hop latencies add up');
});

test('old ranges have no data: onboarding', async () => {
  const old = { from: now - 30 * 24 * HOUR - HOUR, to: now - 30 * 24 * HOUR };
  assert.equal((await mock.streamMap({ range: old })).enabled, false);
  assert.equal((await mock.kafkaConsole({ range: old })).enabled, false);
});

test('measure walks the hops between two services', async () => {
  const r = await mock.streamMeasure({ range, start: 'service:order-service', end: 'service:catalog-service' });
  assert.deepEqual(r.path, ['service:order-service', 'topic:kafka:orders.created', 'service:inventory-service', 'topic:kafka:inventory.reserved', 'service:catalog-service']);
  assert.ok(r.series.length >= 30);
  assert.ok(r.series.every((p) => p.p99Ms >= p.p50Ms));
  const none = await mock.streamMeasure({ range, start: 'service:catalog-service', end: 'service:order-service' });
  assert.deepEqual(none.path, []);
  await assert.rejects(() => mock.streamMeasure({ range, start: 'service:billing', end: 'service:billing' }));
});

test('node detail: throughput and lag per consumer group', async () => {
  const t = await mock.streamNode({ range, node: 'topic:kafka:orders.created' });
  assert.deepEqual(
    t.lag.map((l) => l.group),
    ['billing-consumer', 'inventory-consumer'],
  );
  assert.ok(t.produced.length > 0 && t.consumed.length > 0);
  const producerOnly = await mock.streamNode({ range, node: 'service:order-service' });
  assert.equal(producerOnly.consumed.length, 0);
  assert.equal(producerOnly.lag.length, 0);
});

test('the Kafka console: clusters, health from the values, fictional names', async () => {
  const k = await mock.kafkaConsole({ range });
  assert.deepEqual(
    k.clusters.map((c) => c.name),
    ['shop-events', 'analytics-events'],
  );
  const shop = k.clusters[0];
  assert.equal(shop.brokers, 3);
  assert.equal(shop.topics, 4);
  assert.equal(shop.partitions, 15);
  assert.equal(shop.checksPassing + shop.checksFailing, k.health.filter((h) => h.cluster === 'shop-events' && h.status !== 'no_data').length);
  const failing = k.health.filter((h) => h.cluster === 'shop-events' && h.status === 'failing').map((h) => h.name);
  assert.ok(failing.includes('Consumers keeping up'), 'inventory-consumer lags');
  assert.ok(failing.includes('Connectors running'), 'the CDC source failed');
  const inventory = k.consumerGroups.find((g) => g.name === 'inventory-consumer');
  assert.equal(inventory.lagHigh, true);
  assert.equal(inventory.partitions.length, 6);
  for (const p of inventory.partitions) assert.equal(p.brokerOffset - p.consumerOffset, p.lag);
  for (const b of k.brokers) assert.match(b.host, /^i-0a1b2c3d4e5f600\d\d\.ec2\.internal$/);
  assert.equal(k.charts.length, 2);
  assert.ok(k.charts.every((c) => c.messagesIn.length > 30));
});

test('lag thresholds prefer time, like the API', () => {
  assert.equal(mock.lagIsHigh(50_000, 10), false);
  assert.equal(mock.lagIsHigh(10, 61), true);
  assert.equal(mock.lagIsHigh(10_001, null), true);
  assert.equal(mock.lagIsHigh(null, null), false);
});

test('formats rates, latencies and lag', () => {
  assert.equal(fmt.fmtRate(6.2), '6.2 msg/s');
  assert.equal(fmt.fmtRate(1520), '1.52K msg/s');
  assert.equal(fmt.fmtRate(0.004), '< 0.01 msg/s');
  assert.equal(fmt.fmtMs(0.42), '0.42 ms');
  assert.equal(fmt.fmtMs(1450), '1.45 s');
  assert.equal(fmt.fmtMs(9.4), '9.4 ms');
  assert.equal(fmt.fmtMs(null), '—');
  assert.equal(fmt.fmtLagSeconds(96), '1m 36s');
  assert.equal(fmt.fmtLagSeconds(0.7), '0.7s');
  assert.equal(fmt.fmtCount(4820), '4.82K');
  assert.equal(fmt.nodeName('topic:kafka:orders.created'), 'orders.created');
  assert.equal(fmt.nodeName('service:billing'), 'billing');
});
