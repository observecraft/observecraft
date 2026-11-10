// Mock Kafka broker logs (`source:kafka`) for the Kafka dashboards' log widgets, as the Datadog Agent tails the brokers'
// server.log and integrations-core's Kafka pipeline parses its log4j lines: the level as status, the logger
// (the class in parentheses) as attributes. The demo's three brokers of the prod cluster and one in qa.
import type { LogEvent, LogStatus } from '../types';
import type { Rand } from './generator';

const HOUR = 3_600_000;
const BROKERS: [host: string, id: number, env: string][] = [
  ['i-0a1b2c3d4e5f60001', 1, 'prod'],
  ['i-0a1b2c3d4e5f60002', 2, 'prod'],
  ['i-0a1b2c3d4e5f60003', 3, 'prod'],
  ['i-0a1b2c3d4e5f60006', 1, 'qa'],
];
const TOPICS = ['orders.created', 'payments.authorized', 'inventory.reserved', 'order-events'];

/** [level, logger, message(broker, topic-partition)]: the lines a healthy cluster logs, with a few warnings and errors. */
const LINES: [LogStatus, string, (broker: number, tp: string) => string][] = [
  ['info', 'kafka.log.LogCleaner', () => 'Cleaner 0: Beginning cleaning of log __consumer_offsets-12'],
  ['info', 'kafka.server.ReplicaFetcherManager', (b, tp) => `[ReplicaFetcherManager on broker ${b}] Removed fetcher for partitions Set(${tp})`],
  ['info', 'kafka.coordinator.group.GroupCoordinator', (b) => `[GroupCoordinator ${b}]: Stabilized group order-service-consumers generation 42 (__consumer_offsets-7) with 3 members`],
  ['info', 'kafka.log.UnifiedLog', (_b, tp) => `[UnifiedLog partition=${tp}, dir=/var/lib/kafka/data] Rolled new log segment at offset 1048576 in 2 ms.`],
  ['info', 'kafka.server.metadata.BrokerMetadataPublisher', (b) => `[BrokerMetadataPublisher id=${b}] Updating metadata.version to 4.1-IV1 at offset OffsetAndEpoch(offset=1203, epoch=4).`],
  [
    'warn',
    'kafka.server.ReplicaManager',
    (b, tp) => `[ReplicaManager broker=${b}] Leader ${b} failed to record follower 3's position 88213 for partition ${tp} since the replica is not recognized to be one of the assigned replicas`,
  ],
  ['warn', 'org.apache.kafka.clients.NetworkClient', (b) => `[RaftManager id=${b}] Connection to node 3 (kafka-2.kafka-headless/10.0.12.33:9093) could not be established. Node may not be available.`],
  ['error', 'kafka.server.ReplicaManager', (b, tp) => `[ReplicaManager broker=${b}] Error processing append operation on partition ${tp}`],
  ['error', 'kafka.network.Processor', () => 'Closing socket for 10.0.12.34:9092-10.0.14.71:51544-17 because of error'],
];

/** A day of broker logs, mostly INFO. */
export function kafkaLogs(r: Rand, now: number): LogEvent[] {
  const out: LogEvent[] = [];
  for (let i = 0; i < 480; i++) {
    const ts = Math.round(now - r() * 24 * HOUR);
    // Errors and warnings are rarer than the routine lines.
    const roll = r();
    const candidates = LINES.filter(([level]) => (roll < 0.06 ? level === 'error' : roll < 0.18 ? level === 'warn' : level === 'info'));
    const [status, logger, message] = candidates[Math.floor(r() * candidates.length)];
    const [host, broker, env] = BROKERS[Math.floor(r() * BROKERS.length)];
    const tp = `${TOPICS[Math.floor(r() * TOPICS.length)]}-${Math.floor(r() * 12)}`;
    const text = message(broker, tp);
    out.push({
      id: `kafka-${ts}-${i}`,
      timestamp: ts,
      status,
      service: 'kafka',
      host,
      source: 'kafka',
      message: text,
      tags: { env, kafka_cluster: env === 'prod' ? 'shop-prod' : 'shop-qa' },
      // As the Kafka pipeline leaves them: the level as `status`, the class in parentheses as the logger.
      attributes: { status: status.toUpperCase(), logger: { name: logger } },
    });
  }
  return out;
}
