// Real Kafka + the shipped Collector + ClickHouse. No application stack is stopped or reused.
//   node scripts/kafka-durability.mjs
// Holds ClickHouse unavailable longer than the old 300-second exporter retry, checks offsets,
// destroys the consumer's local queue, then verifies replay and export from a replacement pod.
// Docker is required. Set OC_DURABILITY_OUTAGE_SECONDS=15 for a quick smoke test (not the retry test).
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { readFileSync, readdirSync } from 'node:fs';
import { fileURLToPath } from 'node:url';

const root = fileURLToPath(new URL('../', import.meta.url));
const prefix = `oc-durability-${process.pid}-${Date.now()}`;
const network = `${prefix}-net`;
const names = Object.fromEntries(['kafka', 'clickhouse', 'proxy', 'collector', 'vector'].map((name) => [name, `${prefix}-${name}`]));
const topic = 'oc.otlp-json.traces';
const group = 'oc-collector-json';
const org = 'example-org-durability';
const outageSeconds = Number(process.env.OC_DURABILITY_OUTAGE_SECONDS ?? 310);
assert.ok(Number.isFinite(outageSeconds) && outageSeconds >= 10);

function docker(args, input, allowFailure = false) {
  const r = spawnSync('docker', args, { input, encoding: 'utf8', timeout: 120_000, maxBuffer: 8 * 1024 * 1024 });
  if (!allowFailure && r.status !== 0) throw new Error(`docker ${args.slice(0, 3).join(' ')}: ${r.error ?? r.stderr}`);
  return (allowFailure ? [r.stdout, r.stderr].filter(Boolean).join('\n') : (r.stdout ?? '')).trim();
}
function run(name, args) {
  docker(['run', '-d', '--name', names[name], '--network', network, '--network-alias', name, ...args]);
}
const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
async function until(check, description, seconds = 90) {
  const end = Date.now() + seconds * 1000;
  while (Date.now() < end) {
    try {
      if (check()) return;
    } catch {
      /* service is starting */
    }
    await sleep(1000);
  }
  throw new Error(`Timed out: ${description}`);
}
const kafka = (args, input, allowFailure) => docker(['exec', '-i', names.kafka, `/opt/kafka/bin/${args[0]}.sh`, ...args.slice(1)], input, allowFailure);
const sql = (query) => docker(['exec', names.clickhouse, 'clickhouse-client', '--query', query]);
function status(enable = false) {
  return JSON.parse(
    docker(['exec', names.proxy, 'node', '-e', `fetch('http://localhost:8080/${enable ? 'enable' : ''}', {method:'${enable ? 'POST' : 'GET'}'}).then(r=>r.text()).then(t=>process.stdout.write(t))`]),
  );
}
function committed(signalTopic = topic, signalGroup = group) {
  const output = kafka(['kafka-consumer-groups', '--bootstrap-server', 'kafka:9092', '--describe', '--group', signalGroup], undefined, true);
  const row = output.split('\n').find((line) => {
    const fields = line.trim().split(/\s+/);
    return fields[0] === signalGroup && fields[1] === signalTopic;
  });
  if (!row) return 0;
  return Number(row.trim().split(/\s+/)[3]) || 0;
}
const streams = [
  [topic, group],
  ['oc.otlp-json.metrics', group],
  ['oc.otlp-json.logs', group],
  ['oc.logs.json', 'oc-vector'],
];
function pending() {
  for (const [name, consumerGroup] of streams) assert.equal(committed(name, consumerGroup), 0, `${name}: Kafka retains ownership until delivery`);
}
function vector() {
  run('vector', [
    '--tmpfs',
    '/var/lib/vector:rw',
    '-v',
    `${root}deploy/vector:/etc/vector:ro`,
    '-e',
    'CLICKHOUSE_USER=default',
    '-e',
    'CLICKHOUSE_PASSWORD=',
    '-e',
    'OC_CLICKHOUSE_HTTP_ENDPOINT=http://proxy:8123',
    '-e',
    'OC_LOGPROC_URL=http://proxy:8080/ingest/logs',
    'timberio/vector:0.42.0-debian',
    '--config',
    '/etc/vector/aggregator.yaml',
  ]);
}
function collector() {
  run('collector', [
    '--tmpfs',
    '/var/lib/otelcol:rw',
    '-v',
    `${root}deploy/otel/collector.yaml:/config.yaml:ro`,
    '-e',
    'CLICKHOUSE_USER=default',
    '-e',
    'CLICKHOUSE_PASSWORD=',
    '-e',
    'OC_CLICKHOUSE_TCP_ENDPOINT=tcp://proxy:9000?dial_timeout=1s',
    '-e',
    'OC_VECTOR_OTLP_ENDPOINT=vector:4317',
    'otel/opentelemetry-collector-contrib:0.163.2',
    '--config=/config.yaml',
  ]);
}

try {
  docker(['network', 'create', network]);
  run('clickhouse', ['-e', 'CLICKHOUSE_SKIP_USER_SETUP=1', 'clickhouse/clickhouse-server:26.8']);
  run('kafka', [
    '-e',
    'KAFKA_NODE_ID=1',
    '-e',
    'KAFKA_PROCESS_ROLES=broker,controller',
    '-e',
    'KAFKA_LISTENERS=INTERNAL://:9092,CONTROLLER://:9093',
    '-e',
    'KAFKA_ADVERTISED_LISTENERS=INTERNAL://kafka:9092',
    '-e',
    'KAFKA_LISTENER_SECURITY_PROTOCOL_MAP=INTERNAL:PLAINTEXT,CONTROLLER:PLAINTEXT',
    '-e',
    'KAFKA_INTER_BROKER_LISTENER_NAME=INTERNAL',
    '-e',
    'KAFKA_CONTROLLER_LISTENER_NAMES=CONTROLLER',
    '-e',
    'KAFKA_CONTROLLER_QUORUM_VOTERS=1@kafka:9093',
    '-e',
    'KAFKA_OFFSETS_TOPIC_REPLICATION_FACTOR=1',
    '-e',
    'KAFKA_GROUP_INITIAL_REBALANCE_DELAY_MS=0',
    '-e',
    'KAFKA_HEAP_OPTS=-Xms256m -Xmx512m',
    'apache/kafka:4.1.0',
  ]);
  run('proxy', ['-v', `${root}scripts/kafka-durability/proxy.mjs:/proxy.mjs:ro`, 'node:24-bookworm-slim', 'node', '/proxy.mjs']);
  await until(() => sql('SELECT 1') === '1', 'ClickHouse startup');
  // Apply the same DDL as the server, on an entirely disposable database server.
  for (const file of readdirSync(`${root}server/migrations/clickhouse`)
    .filter((f) => f.endsWith('.sql'))
    .sort()) {
    const ddl = readFileSync(`${root}server/migrations/clickhouse/${file}`, 'utf8')
      .replace(/^.*\$\{DISTRIBUTED .*$/gm, '')
      .replaceAll('${ON_CLUSTER}', '')
      .replaceAll('${LOCAL}', '')
      .replaceAll('${REPLICATED}', '');
    docker(['exec', '-i', names.clickhouse, 'clickhouse-client', '--multiquery'], ddl);
  }
  await until(() => kafka(['kafka-broker-api-versions', '--bootstrap-server', 'kafka:9092'], undefined, true).includes('kafka:9092'), 'Kafka startup');
  for (const encoding of ['otlp', 'otlp-json'])
    for (const signal of ['traces', 'metrics', 'logs']) {
      kafka(['kafka-topics', '--bootstrap-server', 'kafka:9092', '--create', '--topic', `oc.${encoding}.${signal}`, '--partitions', '1', '--replication-factor', '1']);
    }
  for (const name of ['oc.logs.json', 'oc.logs.live']) kafka(['kafka-topics', '--bootstrap-server', 'kafka:9092', '--create', '--topic', name, '--partitions', '1', '--replication-factor', '1']);
  vector();
  collector();
  console.log('Isolated Kafka, Collector and ClickHouse started');
  const start = BigInt(Date.now()) * 1_000_000n;
  const payload = {
    resourceSpans: [
      {
        scopeSpans: [
          {
            spans: [
              {
                traceId: '0123456789abcdef0123456789abcdef',
                spanId: '0123456789abcdef',
                name: 'order-service.durability',
                startTimeUnixNano: start.toString(),
                endTimeUnixNano: (start + 1_000_000n).toString(),
              },
            ],
          },
        ],
      },
    ],
  };
  kafka(['kafka-console-producer', '--bootstrap-server', 'kafka:9092', '--topic', topic, '--property', 'parse.headers=true'], `x-oc-org:${org}\t${JSON.stringify(payload)}\n`);
  const logs = { resourceLogs: [{ scopeLogs: [{ logRecords: [{ timeUnixNano: start.toString(), body: { stringValue: 'order-service OTLP durability probe' }, severityText: 'INFO' }] }] }] };
  const metrics = { resourceMetrics: [{ scopeMetrics: [{ metrics: [{ name: 'system.cpu.user', gauge: { dataPoints: [{ timeUnixNano: start.toString(), asDouble: 42 }] } }] }] }] };
  // The authenticated intake serializes this reserved metadata before Kafka. Equal events have
  // distinct IDs; replay must preserve both identities and the original fallback timestamp.
  const receivedAt = new Date().toISOString();
  const jsonLogs = [1, 2].map((n) => ({
    message: 'order-service JSON durability probe', service: 'order-service',
    _oc_ingest: { id: `019a4000-0000-7000-8000-00000000000${n}`, received_at: receivedAt },
  }));
  for (const [name, body] of [
    ['oc.logs.json', jsonLogs],
    ['oc.otlp-json.logs', logs],
    ['oc.otlp-json.metrics', metrics],
  ])
    kafka(['kafka-console-producer', '--bootstrap-server', 'kafka:9092', '--topic', name, '--property', 'parse.headers=true'], `x-oc-org:${org}\t${JSON.stringify(body)}\n`);
  await until(() => status().blocked > 0, 'export attempted against the unavailable destination');
  await until(() => status().indexerBlocked > 0, 'logs attempted against the unavailable indexer');
  const downUntil = Date.now() + outageSeconds * 1000;
  while (Date.now() < downUntil) {
    pending();
    console.log(`Destination unavailable: ${Math.ceil((downUntil - Date.now()) / 1000)}s remaining; offset is uncommitted`);
    await sleep(Math.min(25_000, Math.max(0, downUntil - Date.now())));
  }
  pending();
  assert.equal(sql(`SELECT count() FROM otel.otel_traces WHERE OrgId='${org}'`), '0');
  docker(['rm', '-f', names.collector]); // tmpfs simulates losing the consumer's local queue volume
  docker(['rm', '-f', names.vector]);
  vector();
  collector();
  pending();
  status(true);
  await until(() => Number(sql(`SELECT count() FROM otel.otel_traces WHERE OrgId='${org}'`)) >= 1, 'trace exported after consumer replacement');
  await until(() => Number(sql(`SELECT count() FROM oc.logs_raw WHERE org_id='${org}'`)) >= 3 && status().indexed >= 3, 'both log paths delivered to both sinks');
  await until(() => Number(sql(`SELECT count() FROM otel.metrics_samples WHERE OrgId='${org}' AND Value=42`)) >= 1, 'metrics delivered');
  for (const [name, consumerGroup] of streams) await until(() => committed(name, consumerGroup) === 1, `${name}: offset committed after export`);
  const identities = () => sql(`SELECT toString(id), min(toUnixTimestamp64Milli(timestamp)), max(toUnixTimestamp64Milli(timestamp)) FROM oc.logs_raw WHERE org_id='${org}' AND message='order-service JSON durability probe' GROUP BY id ORDER BY id`);
  const beforeReplay = identities();
  assert.equal(beforeReplay.split('\n').length, 2, 'equal events retain distinct IDs');
  for (const event of jsonLogs) assert.ok(beforeReplay.includes(`${event._oc_ingest.id}\t${Date.parse(receivedAt)}\t${Date.parse(receivedAt)}`));
  docker(['stop', names.vector]);
  docker(['rm', names.vector]);
  await until(() => {
    const output = kafka(['kafka-consumer-groups', '--bootstrap-server', 'kafka:9092', '--group', 'oc-vector', '--topic', 'oc.logs.json', '--reset-offsets', '--to-earliest', '--execute'], undefined, true);
    return /oc-vector\s+oc.logs.json\s+0\s+0/.test(output);
  }, 'JSON consumer offsets reset while inactive');
  const indexedBefore = status().indexed;
  vector();
  await until(() => status().indexed >= indexedBefore + 2 && committed('oc.logs.json', 'oc-vector') === 1, 'JSON logs replayed through replacement consumer');
  assert.equal(identities(), beforeReplay, 'replay preserves IDs and fallback timestamps');
  console.log('PASS: traces, metrics, OTLP/JSON logs, both log sinks, outage, consumer volume loss, replay and tenant stamping');
  console.log('PASS: equal JSON events remain distinct and retain their IDs/timestamps after an explicit Kafka offset reset (storage remains at-least-once)');
} catch (error) {
  console.error(docker(['logs', '--tail', '25', names.collector], undefined, true));
  console.error(docker(['logs', '--tail', '25', names.vector], undefined, true));
  throw error;
} finally {
  for (const name of Object.values(names)) docker(['rm', '-f', '-v', name], undefined, true);
  docker(['network', 'rm', network], undefined, true);
}
