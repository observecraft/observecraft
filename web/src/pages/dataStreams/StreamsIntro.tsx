import { Link } from 'react-router-dom';
import { Activity, BookOpen, Gauge, Layers, Timer, Waypoints } from 'lucide-react';
import type { StreamEdge, StreamNode } from '../../api/dataStreams/client';
import { ProductIntro, introButton } from '../../components/ui/ProductIntro';
import { IntegrationLogo } from '../integrations/IntegrationLogo';
import { StreamGraph } from './StreamGraph';

/** The hero's product preview: our own map with a few fictional services and topics. */
function preview(): { nodes: StreamNode[]; edges: StreamEdge[] } {
  const svc = (name: string, produced: number, consumed: number, lagHigh = false): StreamNode => ({
    id: `service:${name}`,
    kind: 'service',
    name,
    produced,
    consumed,
    errors: 0,
    lagHigh,
  });
  const topic = (name: string, rate: number, lagHigh = false): StreamNode => ({ id: `topic:kafka:${name}`, kind: 'topic', name, system: 'kafka', produced: rate, consumed: rate, errors: 0, lagHigh });
  const edge = (source: string, target: string, throughput: number, lagHigh = false): StreamEdge => ({
    source,
    target,
    kind: source.startsWith('service:') ? 'produce' : 'consume',
    throughput,
    errors: 0,
    spanP50Ms: 1.2,
    spanP99Ms: 4,
    latencyP50Ms: 9,
    consumerGroups: [],
    lag: lagHigh ? 4820 : 20,
    lagHigh,
  });
  return {
    nodes: [svc('order-service', 6.2, 0), svc('storefront-api', 2.4, 0), topic('orders.created', 8.6, true), svc('billing', 5.9, 8.6), svc('inventory-service', 6, 8.6, true)],
    edges: [
      edge('service:order-service', 'topic:kafka:orders.created', 6.2),
      edge('service:storefront-api', 'topic:kafka:orders.created', 2.4),
      edge('topic:kafka:orders.created', 'service:billing', 8.6),
      edge('topic:kafka:orders.created', 'service:inventory-service', 8.6, true),
    ],
  };
}

const BENEFITS = [
  { icon: Timer, title: 'Find slowdowns between any two points', text: 'Measure how long messages take from one service to another, hop by hop, and see when it changed.' },
  { icon: Gauge, title: 'Pinpoint the bottleneck', text: 'Throughput and latency on every edge show which producer, topic or consumer holds the pipeline back.' },
  { icon: Layers, title: 'Clear message backlogs', text: 'Consumer lag per group, topic and partition, in messages and in time, so a growing backlog is caught early.' },
  { icon: Activity, title: 'Resolve faster', text: 'Jump from a slow edge to the traces of its producers and consumers, and to the Kafka cluster behind it.' },
];

const START = [
  {
    logo: 'opentelemetry',
    title: 'Trace producers and consumers',
    text: 'Datadog tracers and OpenTelemetry SDKs already create spans for Kafka clients: send them here and the map draws itself.',
    tab: 'tracer',
  },
  { logo: 'apachekafka', title: 'Report Kafka metrics', text: "The Agent's Kafka checks add brokers, partitions and consumer lag for the Kafka Console and the map's lag.", tab: 'kafka-agent' },
  { logo: 'otel', title: 'Or use the Collector', text: "The OpenTelemetry Collector's Kafka receiver reports brokers, offsets and consumer group lag too.", tab: 'kafka-otel' },
];

/** Shown on Map, Pipelines and Measure until messaging spans arrive. */
export function StreamsIntro() {
  const p = preview();
  return (
    <ProductIntro
      icon={Waypoints}
      product="Data Streams Monitoring"
      title="Follow every message from producer to consumer"
      text="See how your services talk through Kafka and other queues: throughput, end-to-end latency and consumer lag on one map."
      actions={
        <>
          <Link className={introButton.primary} to="/data-streams/setup">
            <BookOpen size={16} /> View documentation
          </Link>
          <Link className={introButton.secondary} to="/data-streams/kafka">
            Open Kafka Console
          </Link>
        </>
      }
      preview={<StreamGraph nodes={p.nodes} edges={p.edges} fit />}
      benefits={BENEFITS}
      start={START.map((s) => ({ icon: <IntegrationLogo logo={s.logo} name={s.title} size={28} />, title: s.title, text: s.text, to: `/data-streams/setup?tab=${s.tab}` }))}
    />
  );
}
