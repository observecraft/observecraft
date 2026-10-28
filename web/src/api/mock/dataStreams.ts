// In-browser Data Streams Monitoring for mock mode: the demo shop's Kafka topology (the same one
// oc-tracegen sends) — orders flow from order-service through `orders.created` to billing and
// inventory-service, then on to notification-worker and catalog-service — evaluated like the API:
// edges and pathways from producer → topic → consumer hops, lag from the consumer groups, and a
// Kafka console with two fictional clusters. Values move with time; inventory-consumer lags behind.
import type {
  KafkaBroker,
  KafkaChange,
  KafkaCluster,
  KafkaClusterChart,
  KafkaConnector,
  KafkaConsole,
  KafkaConsoleRequest,
  KafkaConsumerGroup,
  KafkaHealthCheck,
  KafkaTopic,
  StreamEdge,
  StreamLagSeries,
  StreamLatencyPoint,
  StreamMap,
  StreamMapRequest,
  StreamMeasure,
  StreamMeasureRequest,
  StreamNode,
  StreamNodeDetail,
  StreamNodeRequest,
  StreamPathway,
} from '../dataStreams/client';
import { rng } from './generator';

const DAY = 86_400_000;
/** Demo data goes back two weeks: older ranges show the onboarding. */
const HISTORY_MS = 14 * DAY;

/** Consumer lag above these is high (the API's thresholds). */
export const LAG_HIGH_SECONDS = 60;
export const LAG_HIGH_MESSAGES = 10_000;

export function lagIsHigh(messages: number | null | undefined, seconds: number | null | undefined): boolean {
  if (seconds != null) return seconds > LAG_HIGH_SECONDS;
  return messages != null && messages > LAG_HIGH_MESSAGES;
}

/** One linked hop: messages a producer publishes to a topic and a consumer group processes. */
export interface MockHop {
  producer: string;
  topic: string;
  consumer: string;
  group: string;
  /** Messages per second. */
  rate: number;
  p50: number;
  p99: number;
  /** Consumer lag (messages, all partitions) and its estimate in time (s). */
  lag: number;
  lagSeconds: number;
}

export const HOPS: MockHop[] = [
  { producer: 'order-service', topic: 'orders.created', consumer: 'billing', group: 'billing-consumer', rate: 6.2, p50: 9.4, p99: 31, lag: 24, lagSeconds: 0.7 },
  { producer: 'order-service', topic: 'orders.created', consumer: 'inventory-service', group: 'inventory-consumer', rate: 6.2, p50: 1450, p99: 4210, lag: 4_820, lagSeconds: 96 },
  { producer: 'billing', topic: 'payments.authorized', consumer: 'notification-worker', group: 'notification-consumer', rate: 5.9, p50: 6.1, p99: 18, lag: 9, lagSeconds: 0.2 },
  { producer: 'inventory-service', topic: 'inventory.reserved', consumer: 'catalog-service', group: 'catalog-consumer', rate: 6.0, p50: 11.2, p99: 40, lag: 12, lagSeconds: 0.3 },
  { producer: 'workflow-engine', topic: 'order-events', consumer: 'notification-worker', group: 'notification-consumer', rate: 2.1, p50: 4.5, p99: 15, lag: 3, lagSeconds: 0.1 },
];

export const serviceId = (name: string) => `service:${name}`;
export const topicId = (topic: string, system = 'kafka') => `topic:${system}:${topic}`;

const hasData = (range: { to: number }) => range.to > Date.now() - HISTORY_MS;

/** A gentle wave around 1 at `t`, different per `phase`. */
const wave = (t: number, phase: number, period = 900_000) => 1 + 0.12 * Math.sin(t / period + phase);

// ================================================================ map

/** Edges, nodes and pathways of `hops` (the API's rules, over the mock's whole-range hop values). */
export function buildMap(hops: MockHop[]): StreamMap {
  const edges = new Map<string, StreamEdge>();
  const edge = (source: string, target: string, kind: StreamEdge['kind']) => {
    const key = `${source}|${target}`;
    let e = edges.get(key);
    if (!e) {
      e = {
        source,
        target,
        kind,
        throughput: 0,
        errors: 0,
        spanP50Ms: kind === 'produce' ? 1.2 : 2.4,
        spanP99Ms: kind === 'produce' ? 4.8 : 11,
        consumerGroups: [],
        lagHigh: false,
        latencyP50Ms: null,
        latencyP99Ms: null,
        lag: null,
        lagSeconds: null,
      };
      edges.set(key, e);
    }
    return e;
  };
  const published = new Set<string>();
  for (const h of hops) {
    const p = edge(serviceId(h.producer), topicId(h.topic), 'produce');
    // A producer publishes once per message, whoever consumes it.
    if (!published.has(`${h.producer}|${h.topic}`)) {
      p.throughput += h.rate;
      published.add(`${h.producer}|${h.topic}`);
    }
    const c = edge(topicId(h.topic), serviceId(h.consumer), 'consume');
    c.latencyP50Ms = ((c.latencyP50Ms ?? 0) * c.throughput + h.p50 * h.rate) / (c.throughput + h.rate);
    c.latencyP99Ms = Math.max(c.latencyP99Ms ?? 0, h.p99);
    c.throughput += h.rate;
    if (!c.consumerGroups.includes(h.group)) c.consumerGroups.push(h.group);
    c.lag = Math.max(c.lag ?? 0, h.lag);
    c.lagSeconds = Math.max(c.lagSeconds ?? 0, h.lagSeconds);
    c.lagHigh = lagIsHigh(c.lag, c.lagSeconds);
  }
  const nodes = new Map<string, StreamNode>();
  const node = (id: string, kind: StreamNode['kind'], name: string): StreamNode => {
    let n = nodes.get(id);
    if (!n) {
      n = { id, kind, name, system: kind === 'topic' ? 'kafka' : null, produced: 0, consumed: 0, errors: 0, lag: null, lagSeconds: null, lagHigh: false };
      nodes.set(id, n);
    }
    return n;
  };
  for (const e of edges.values()) {
    const [svcId, tId] = e.kind === 'produce' ? [e.source, e.target] : [e.target, e.source];
    const svc = node(svcId, 'service', svcId.slice('service:'.length));
    const topic = node(tId, 'topic', tId.split(':').slice(2).join(':'));
    if (e.kind === 'produce') {
      svc.produced += e.throughput;
      topic.produced += e.throughput;
    } else {
      svc.consumed += e.throughput;
      topic.consumed += e.throughput;
      for (const n of [svc, topic]) {
        n.lag = Math.max(n.lag ?? 0, e.lag ?? 0);
        n.lagSeconds = Math.max(n.lagSeconds ?? 0, e.lagSeconds ?? 0);
        n.lagHigh = n.lagHigh || e.lagHigh;
      }
    }
  }
  const sorted = [...nodes.values()].sort((a, b) => a.id.localeCompare(b.id));
  const edgeList = [...edges.values()];
  return { enabled: true, nodes: sorted, edges: edgeList, pathways: pathways(hops, edgeList) };
}

/** Chains of hops from services that only produce to services whose messages lead nowhere. */
export function pathways(hops: MockHop[], edges: StreamEdge[]): StreamPathway[] {
  const lagging = new Set(edges.filter((e) => e.lagHigh).map((e) => `${e.source}|${e.target}`));
  const consumers = new Set(hops.map((h) => h.consumer));
  let starts = [...new Set(hops.map((h) => h.producer))].filter((p) => !consumers.has(p));
  if (!starts.length) starts = [...new Set(hops.map((h) => h.producer))];
  const out: StreamPathway[] = [];
  const walk = (at: string, path: MockHop[]) => {
    const next = hops.filter((h) => h.producer === at && !path.some((p) => p.producer === h.consumer || p.consumer === h.consumer));
    if (!next.length || path.length >= 12) {
      if (!path.length) return;
      const nodes = [serviceId(path[0].producer), ...path.flatMap((h) => [topicId(h.topic), serviceId(h.consumer)])];
      out.push({
        id: nodes.join('>'),
        nodes,
        hops: path.length,
        latencyP50Ms: path.reduce((s, h) => s + h.p50, 0),
        latencyP99Ms: path.reduce((s, h) => s + h.p99, 0),
        throughput: path[path.length - 1].rate,
        lagHigh: path.some((h) => lagging.has(`${topicId(h.topic)}|${serviceId(h.consumer)}`)),
      });
      return;
    }
    for (const h of next) walk(h.consumer, [...path, h]);
  };
  for (const s of starts.sort()) walk(s, []);
  return out.sort((a, b) => b.throughput - a.throughput || a.id.localeCompare(b.id));
}

export async function streamMap(req: StreamMapRequest): Promise<StreamMap> {
  if (!hasData(req.range)) return { enabled: false, nodes: [], edges: [], pathways: [] };
  return buildMap(HOPS);
}

// ================================================================ measure

/** The fewest hops from service `start` to `end` (breadth first), or null. */
export function pathBetween(hops: MockHop[], start: string, end: string): MockHop[] | null {
  const prev = new Map<string, MockHop>();
  const seen = new Set([start]);
  const queue = [start];
  while (queue.length) {
    const at = queue.shift()!;
    if (at === end) {
      const path: MockHop[] = [];
      for (let cur = end; cur !== start;) {
        const h = prev.get(cur)!;
        path.unshift(h);
        cur = h.producer;
      }
      return path.length ? path : null;
    }
    for (const h of hops.filter((x) => x.producer === at)) {
      if (!seen.has(h.consumer)) {
        seen.add(h.consumer);
        prev.set(h.consumer, h);
        queue.push(h.consumer);
      }
    }
  }
  return null;
}

/** About 60 interval starts across the range. */
export function steps(range: { from: number; to: number }, points = 60): number[] {
  const nice = [10_000, 30_000, 60_000, 120_000, 300_000, 600_000, 900_000, 1_800_000, 3_600_000, 7_200_000, 14_400_000, 21_600_000, 43_200_000, 86_400_000];
  const step = nice.find((s) => s >= (range.to - range.from) / points) ?? DAY;
  const out: number[] = [];
  for (let t = Math.ceil(range.from / step) * step; t <= range.to; t += step) out.push(t);
  return out;
}

export async function streamMeasure(req: StreamMeasureRequest): Promise<StreamMeasure> {
  const start = req.start.replace(/^service:/, '');
  const end = req.end.replace(/^service:/, '');
  if (start === end) throw new Error('start and end must differ');
  const path = hasData(req.range) ? pathBetween(HOPS, start, end) : null;
  if (!path) return { path: [], series: [], latencyP50Ms: null, latencyP99Ms: null, throughput: null };
  const series: StreamLatencyPoint[] = steps(req.range).map((t) => ({
    t,
    p50Ms: path.reduce((s, h, i) => s + h.p50 * wave(t, i), 0),
    p99Ms: path.reduce((s, h, i) => s + h.p99 * wave(t, i + 2, 600_000), 0),
  }));
  return {
    path: [serviceId(start), ...path.flatMap((h) => [topicId(h.topic), serviceId(h.consumer)])],
    latencyP50Ms: path.reduce((s, h) => s + h.p50, 0),
    latencyP99Ms: path.reduce((s, h) => s + h.p99, 0),
    throughput: path[path.length - 1].rate,
    series,
  };
}

// ================================================================ node

export async function streamNode(req: StreamNodeRequest): Promise<StreamNodeDetail> {
  const ts = hasData(req.range) ? steps(req.range) : [];
  let produced = 0;
  let consumed = 0;
  let lagging: MockHop[] = [];
  if (req.node.startsWith('topic:')) {
    const topic = req.node.split(':').slice(2).join(':');
    const into = HOPS.filter((h) => h.topic === topic);
    produced = [...new Map(into.map((h) => [h.producer, h.rate])).values()].reduce((a, b) => a + b, 0);
    consumed = into.reduce((s, h) => s + h.rate, 0);
    lagging = into;
  } else {
    const name = req.node.replace(/^service:/, '');
    produced = [...new Map(HOPS.filter((h) => h.producer === name).map((h) => [h.topic, h.rate])).values()].reduce((a, b) => a + b, 0);
    lagging = HOPS.filter((h) => h.consumer === name);
    consumed = lagging.reduce((s, h) => s + h.rate, 0);
  }
  const lag: StreamLagSeries[] = lagging.map((h, i) => ({
    group: h.group,
    topic: h.topic,
    points: ts.map((t) => [t, Math.round(h.lag * (h.lagSeconds > LAG_HIGH_SECONDS ? 0.4 + (0.6 * ((t / 60_000) % 20)) / 20 : wave(t, i, 120_000)))] as [number, number]),
  }));
  return {
    produced: produced ? ts.map((t, i) => [t, produced * wave(t, i % 3)] as [number, number]) : [],
    consumed: consumed ? ts.map((t, i) => [t, consumed * wave(t, (i % 3) + 1)] as [number, number]) : [],
    lag,
  };
}

// ================================================================ Kafka console

interface MockTopic {
  cluster: string;
  name: string;
  partitions: number;
  rate: number;
  bytesPerMessage: number;
  /** Consumer groups and their usual lag per partition. */
  groups: [string, number][];
}

export const TOPICS: MockTopic[] = [
  {
    cluster: 'shop-events',
    name: 'orders.created',
    partitions: 6,
    rate: 42,
    bytesPerMessage: 1350,
    groups: [
      ['billing-consumer', 4],
      ['inventory-consumer', 803],
    ],
  },
  { cluster: 'shop-events', name: 'payments.authorized', partitions: 3, rate: 40, bytesPerMessage: 820, groups: [['notification-consumer', 2]] },
  { cluster: 'shop-events', name: 'inventory.reserved', partitions: 3, rate: 41, bytesPerMessage: 640, groups: [['catalog-consumer', 3]] },
  { cluster: 'shop-events', name: 'order-events', partitions: 3, rate: 18, bytesPerMessage: 900, groups: [['notification-consumer', 1]] },
  { cluster: 'analytics-events', name: 'clickstream.events', partitions: 12, rate: 260, bytesPerMessage: 410, groups: [['analytics-loader', 120]] },
];

const BROKERS: Record<string, [string, string][]> = {
  'shop-events': [
    ['1', 'i-0a1b2c3d4e5f60011'],
    ['2', 'i-0a1b2c3d4e5f60012'],
    ['3', 'i-0a1b2c3d4e5f60013'],
  ],
  'analytics-events': [['1', 'i-0a1b2c3d4e5f60021']],
};

const MEMBERS: Record<string, number> = { 'analytics-loader': 6, 'billing-consumer': 3 };

/** Health checks of a cluster from its values (the API's rules). */
export function clusterChecks(c: KafkaCluster, groups: KafkaConsumerGroup[], connectors: KafkaConnector[]): KafkaHealthCheck[] {
  const count = (name: string, v: number | null | undefined, what: string): KafkaHealthCheck =>
    v == null ? { cluster: c.name, name, status: 'no_data', detail: `No ${what} metrics` } : { cluster: c.name, name, status: v > 0 ? 'failing' : 'passing', detail: `${Math.round(v)} ${what}` };
  const mine = groups.filter((g) => g.cluster === c.name);
  const behind = mine.filter((g) => g.lagHigh).map((g) => g.name);
  const out: KafkaHealthCheck[] = [
    count('No offline partitions', c.offline, 'offline partitions'),
    count('No under-replicated partitions', c.underReplicated, 'under-replicated partitions'),
    { cluster: c.name, name: 'One active controller', status: 'passing', detail: '1 active controllers' },
    mine.length
      ? behind.length
        ? { cluster: c.name, name: 'Consumers keeping up', status: 'failing', detail: `High lag: ${behind.join(', ')}` }
        : { cluster: c.name, name: 'Consumers keeping up', status: 'passing', detail: `${mine.length} consumer groups within lag limits` }
      : { cluster: c.name, name: 'Consumers keeping up', status: 'no_data', detail: 'No consumer lag metrics' },
  ];
  const conns = connectors.filter((x) => x.cluster === c.name);
  if (conns.length) {
    const failed = conns.filter((x) => x.failed > 0 || x.state === 'failed').map((x) => x.name);
    out.push(
      failed.length
        ? { cluster: c.name, name: 'Connectors running', status: 'failing', detail: `Failed: ${failed.join(', ')}` }
        : { cluster: c.name, name: 'Connectors running', status: 'passing', detail: `${conns.length} connectors` },
    );
  }
  return out;
}

export async function kafkaConsole(req: KafkaConsoleRequest): Promise<KafkaConsole> {
  if (!hasData(req.range)) return { enabled: false, clusters: [], topics: [], brokers: [], consumerGroups: [], connectors: [], charts: [], health: [], changes: [] };
  const ts = steps(req.range);
  const now = req.range.to;
  const r = rng(Math.floor(now / 600_000));
  const elapsedS = (now - Date.UTC(2025, 0, 1)) / 1000;
  const groups = new Map<string, KafkaConsumerGroup>();
  const topics: KafkaTopic[] = TOPICS.map((t) => {
    const perPartition = t.rate / t.partitions;
    for (const [name, usual] of t.groups) {
      const key = `${t.cluster}|${name}`;
      let g = groups.get(key);
      if (!g) {
        g = { cluster: t.cluster, name, state: 'stable', members: MEMBERS[name] ?? 2, lag: 0, lagMax: 0, lagSeconds: 0, lagHigh: false, consumeRate: 0, topics: [], partitions: [] };
        groups.set(key, g);
      }
      let topicLag = 0;
      let topicSeconds = 0;
      for (let p = 0; p < t.partitions; p++) {
        const lag = Math.round(usual * (0.8 + 0.4 * r()));
        const brokerOffset = Math.floor(elapsedS * perPartition * (1 + 0.05 * p));
        const seconds = lag / perPartition;
        g.partitions.push({ topic: t.name, partition: String(p), lag, lagSeconds: seconds, brokerOffset, consumerOffset: brokerOffset - lag });
        topicLag += lag;
        topicSeconds = Math.max(topicSeconds, seconds);
        g.lagMax = Math.max(g.lagMax, lag);
      }
      g.topics.push({ topic: t.name, lag: topicLag, lagSeconds: topicSeconds });
      g.lag += topicLag;
      g.lagSeconds = Math.max(g.lagSeconds ?? 0, topicSeconds);
      g.consumeRate = (g.consumeRate ?? 0) + t.rate;
      g.lagHigh = lagIsHigh(Math.max(...g.topics.map((x) => x.lag)), g.lagSeconds);
    }
    const lags = t.groups.map(([name]) => groups.get(`${t.cluster}|${name}`)!.topics.find((x) => x.topic === t.name)!.lag);
    return {
      cluster: t.cluster,
      name: t.name,
      partitions: t.partitions,
      replicationFactor: Math.min(3, BROKERS[t.cluster].length),
      underReplicated: 0,
      messagesIn: t.rate * wave(now, t.partitions),
      messagesOut: t.rate * t.groups.length,
      bytesIn: t.rate * t.bytesPerMessage,
      bytesOut: t.rate * t.bytesPerMessage * t.groups.length,
      size: Math.round(t.rate * 7 * 86_400),
      consumerLagMax: Math.max(...lags),
      consumerGroups: t.groups.map(([name]) => name),
    };
  });
  const connectors: KafkaConnector[] = [
    { cluster: 'shop-events', name: 'orders-s3-sink', connectorType: 'sink', class: 'S3SinkConnector', state: 'running', tasks: 2, running: 2, failed: 0 },
    { cluster: 'shop-events', name: 'catalog-cdc-source', connectorType: 'source', class: 'PostgresConnector', state: 'failed', tasks: 1, running: 0, failed: 1 },
  ];
  const brokers: KafkaBroker[] = [];
  const charts: KafkaClusterChart[] = [];
  const clusters: KafkaCluster[] = [];
  const health: KafkaHealthCheck[] = [];
  const consumerGroups = [...groups.values()].sort((a, b) => a.cluster.localeCompare(b.cluster) || a.name.localeCompare(b.name));
  for (const [cluster, nodes] of Object.entries(BROKERS)) {
    const mine = topics.filter((t) => t.cluster === cluster);
    const rate = mine.reduce((s, t) => s + (t.messagesIn ?? 0), 0);
    const partitions = mine.reduce((s, t) => s + (t.partitions ?? 0), 0);
    const bytes = mine.reduce((s, t) => s + (t.bytesIn ?? 0), 0);
    nodes.forEach(([id, host], i) =>
      brokers.push({
        cluster,
        id,
        host: `${host}.ec2.internal`,
        partitions: Math.round((partitions * Math.min(3, nodes.length)) / nodes.length),
        leaders: Math.round(partitions / nodes.length),
        underReplicated: 0,
        messagesIn: rate / nodes.length,
        bytesIn: bytes / nodes.length,
        bytesOut: (bytes / nodes.length) * 2.1,
        controller: i === 0,
      }),
    );
    // shop-events loses a replica for four minutes every half hour.
    const under = (t: number) => (cluster === 'shop-events' && t % 1_800_000 < 240_000 ? 2 : 0);
    const chart: KafkaClusterChart = {
      cluster,
      messagesIn: ts.map((t) => [t, rate * wave(t, cluster.length)]),
      underReplicated: ts.map((t) => [t, under(t)]),
      offline: ts.map((t) => [t, 0]),
    };
    charts.push(chart);
    const c: KafkaCluster = {
      name: cluster,
      brokers: nodes.length,
      topics: mine.length,
      partitions,
      underReplicated: chart.underReplicated.at(-1)?.[1] ?? null,
      offline: 0,
      messagesIn: rate,
      consumerGroups: consumerGroups.filter((g) => g.cluster === cluster).length,
      connectors: connectors.filter((x) => x.cluster === cluster).length,
      checksPassing: 0,
      checksFailing: 0,
      monitors: cluster === 'shop-events' ? 2 : 0,
    };
    const checks = clusterChecks(c, consumerGroups, connectors);
    c.checksPassing = checks.filter((x) => x.status === 'passing').length;
    c.checksFailing = checks.filter((x) => x.status === 'failing').length;
    clusters.push(c);
    health.push(...checks);
  }
  const changes: KafkaChange[] = [
    { ts: now - 7 * 60_000, title: 'Consumer group inventory-consumer is falling behind on orders.created', status: 'warning', cluster: 'shop-events' },
    { ts: now - 26 * 60_000, title: 'Connector catalog-cdc-source task 0 failed', status: 'error', cluster: 'shop-events' },
    { ts: now - 48 * 60_000, title: 'Consumer group billing-consumer rebalanced (3 members)', status: 'info', cluster: 'shop-events' },
  ].filter((c) => c.ts >= req.range.from);
  return { enabled: true, clusters, topics, brokers, consumerGroups, connectors, charts, health, changes };
}
