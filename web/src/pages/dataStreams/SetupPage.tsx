import { useSearchParams } from 'react-router-dom';
import { collectorConfig } from '../setup/collector';
import { SetupOptions, type SetupOption } from '../setup/SetupOptions';

// {{intake}} and {{apiKey}} are filled in per install by SetupOptions.
export const STREAM_SETUP: SetupOption[] = [
  {
    id: 'tracer',
    name: 'Datadog tracer',
    summary:
      'The map, pipelines and measurements come from the spans your Kafka clients already create. Datadog tracing libraries instrument the Kafka producer and consumer clients out of the box; send their traces through the Datadog Agent to this install.',
    steps: [
      {
        title: 'Point the Agent at ObserveCraft (datadog.yaml)',
        code: 'api_key: {{apiKey}}\ndd_url: {{intake}}\napm_config:\n  enabled: true\n  apm_dd_url: {{intake}}',
      },
      {
        title: 'Run producers and consumers with the tracer',
        text: 'Every service that publishes or consumes needs the tracer, so a message can be followed from one to the next. The Kafka integration is on by default and puts the trace context in the message headers; keep that propagation on.',
        code: 'DD_SERVICE=order-service\nDD_ENV=prod\nDD_AGENT_HOST=localhost',
      },
      {
        title: 'Optional: Data Streams checkpoints',
        text: 'DD_DATA_STREAMS_ENABLED=true makes the tracer also send pathway checkpoints. ObserveCraft does not read them yet, so leave it off: the map is built from the produce and consume spans, which the tracer sends either way.',
      },
      { title: 'Check the map', text: 'Services and topics appear on the Map tab within a minute of the first messages. A consume span whose parent is the publish span gives the hop its latency.' },
    ],
  },
  {
    id: 'otel',
    name: 'OpenTelemetry SDK',
    summary:
      "OpenTelemetry's Kafka instrumentation (Java agent, Python, Node.js, .NET, Go contrib) creates producer and consumer spans with the messaging semantic conventions. Export them over OTLP to this install.",
    steps: [
      {
        title: 'Export traces to ObserveCraft',
        code: 'OTEL_SERVICE_NAME=billing\nOTEL_EXPORTER_OTLP_ENDPOINT={{intake}}\nOTEL_EXPORTER_OTLP_HEADERS="authorization=Bearer {{apiKey}}"\nOTEL_RESOURCE_ATTRIBUTES=deployment.environment=prod',
      },
      {
        title: 'What the spans need',
        text: 'messaging.system (kafka, rabbitmq, …), messaging.destination.name (the topic), a producer or consumer span kind, and on consumers messaging.consumer.group.name. Consume spans linked to the publish span (parent or span link) give end-to-end latency.',
      },
    ],
  },
  {
    id: 'kafka-agent',
    name: 'Kafka metrics: Datadog Agent',
    summary:
      "The Agent's kafka check reads the brokers over JMX; its kafka_consumer check reads offsets, consumer lag and the cluster's topics, partitions and consumer groups. Together they fill the Kafka Console and the consumer lag on the map.",
    steps: [
      {
        title: 'Broker metrics (conf.d/kafka.d/conf.yaml, on each broker)',
        code: 'instances:\n  - host: localhost\n    port: 9999\n    tags:\n      - kafka_cluster:shop-events',
      },
      {
        title: 'Consumer lag and cluster metadata (conf.d/kafka_consumer.d/conf.yaml)',
        text: 'Run it once per cluster. data_streams_enabled adds the lag in time; the cluster tag groups broker and consumer metrics under one name.',
        code: 'instances:\n  - kafka_connect_str: kafka-1:9092\n    monitor_unlisted_consumer_groups: true\n    data_streams_enabled: true\n    tags:\n      - kafka_cluster:shop-events',
      },
      { title: 'Restart the Agent', text: 'The cluster shows up in the Kafka Console with its brokers, topics and consumer groups.' },
    ],
  },
  {
    id: 'kafka-otel',
    name: 'Kafka metrics: OpenTelemetry Collector',
    summary: "The Collector's kafkametrics receiver reports brokers, partitions, offsets and consumer group lag over OTLP.",
    steps: [
      {
        title: 'Add the kafkametrics receiver',
        code: collectorConfig(
          'kafka',
          '  kafkametrics:\n    brokers: [kafka-1:9092]\n    cluster_alias: shop-events\n    scrapers: [brokers, topics, consumers]\n    collection_interval: 30s\n    resource_attributes:\n      kafka.cluster.alias:\n        enabled: true',
        ),
      },
    ],
  },
];

/** Data Streams → Setup: how messaging spans and Kafka metrics reach this install. */
export function SetupPage() {
  const [params] = useSearchParams();
  const tab = params.get('tab');
  return (
    <div className="dsm-page dsm-setup">
      <h2>Send data streams to ObserveCraft</h2>
      <p className="muted">
        Two sources feed this product: traces of the services that publish and consume messages (Map, Pipelines, Measure), and Kafka metrics (Kafka Console, consumer lag). Start with either.
      </p>
      <SetupOptions key={tab ?? ''} options={STREAM_SETUP} initial={STREAM_SETUP.some((o) => o.id === tab) ? tab! : undefined} />
    </div>
  );
}
