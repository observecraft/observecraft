#!/usr/bin/env node
// Golden rows for the Rust log normalization (server/crates/oc-api/src/log_normalize.rs): every case below goes
// through deploy/vector/normalize.vrl in Vector itself (its VRL CLI, Docker), and the inputs and Vector's outputs
// are written next to the Rust module. Rerun when normalize.vrl or the cases change:
//
//   node scripts/log-normalize-golden.mjs
//
// A case is a Vector event as normalize.vrl receives it (`source_type`, `resources`, `attributes`, `message`,
// `_oc_kafka_headers`, …); `_t` is its event timestamp as a timestamp value (sources set one). Every case has an
// event time, so no row depends on the current time; generated ids (UUIDv7) are compared by format only.
import { execFileSync } from 'node:child_process';
import { mkdtempSync, readFileSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const out = join(root, 'server/crates/oc-api/src/log_normalize');
const compose = readFileSync(join(root, 'deploy/docker-compose.yml'), 'utf8');
const image = compose.match(/image: (timberio\/vector:\S+)/)[1];

const T = '2026-10-25T18:00:00Z';
const otlp = (extra) => ({ source_type: 'opentelemetry', _t: T, ...extra });
const json = (extra) => ({ source_type: 'http_server', 'x-oc-org': 'acme', _t: T, ...extra });
const agent = (message, status, extra = {}) => json({ message, status, ddsource: 'python', service: 'catalog-service', hostname: 'node-1', ...extra });

const cases = [
  // deploy/vector/tests.yaml
  ['tests: resource service labels and version retain priority over workload metadata', otlp({
    resources: {
      'oc.org_id': 'example-org', 'service.name': 'billing', 'service.version': '1', 'deployment.environment.name': 'staging',
      'k8s.deployment.name': 'catalog-service', 'k8s.pod.labels.tags.datadoghq.com/service': 'order-service',
      'k8s.pod.labels.tags.datadoghq.com/version': '2', 'k8s.pod.labels.tags.datadoghq.com/env': 'benchmark', 'example.custom': 'retained',
    },
    message: 'order accepted',
  })],
  ['tests: tenant-only resources keep JSON FireLens and explicit environment tags', {
    source_type: 'http_server', 'x-oc-org': 'example-org', _t: T, resources: { 'oc.org_id': 'example-org', 'oc.retention_days': '7' },
    tags: { env: 'benchmark' }, ecs_task_definition: 'order-service:17', message: 'order accepted',
  }],
  ['tests: accepted OTLP log retains its ID and event time through replay', {
    source_type: 'opentelemetry', _t: '2026-10-20T12:00:00Z', resources: { 'oc.org_id': 'example-org' },
    attributes: { 'oc.ingest_id': '019a4000-0000-7000-8000-000000000004', 'oc.ingest_received_at': '2026-10-25T18:00:00Z', 'order.id': '42' },
    message: 'order accepted',
  }],
  ['tests: accepted JSON log retains its ID and fallback time through replay', {
    source_type: 'kafka', _oc_kafka_headers: { 'x-oc-org': 'example-org' },
    _oc_ingest: { id: '019a4000-0000-7000-8000-000000000001', received_at: '2026-10-25T18:00:00Z' }, _t: '2026-10-31T00:00:00Z', message: 'order accepted',
  }],
  ['tests: accepted JSON log keeps explicit event time and hides reserved metadata', {
    source_type: 'http_server', 'x-oc-org': 'example-org', _oc_ingest: { id: '019a4000-0000-7000-8000-000000000002', received_at: '2026-10-25T18:00:00Z' },
    timestamp: '2026-10-20T12:00:00Z', message: 'order accepted',
  }],
  ['tests: kubernetes container log with a JSON line (OTLP from the Collector)', otlp({
    message: '{"level":"error","msg":"payment declined","order_id":42}',
    resources: {
      'oc.org_id': 'acme', 'k8s.namespace.name': 'shop', 'k8s.pod.name': 'checkout-7d9f-abc', 'k8s.container.name': 'checkout',
      'k8s.deployment.name': 'checkout', 'k8s.node.name': 'node-1', 'k8s.cluster.name': 'prod-eu', 'container.image.name': 'ghcr.io/example-org/checkout',
      'k8s.pod.labels.tags.datadoghq.com/env': 'prod', 'k8s.pod.labels.tags.datadoghq.com/service': 'checkout-api',
    },
  })],
  ['tests: JSON log from Kafka takes its organization from the intake record header only', {
    source_type: 'kafka', _t: T, 'x-oc-org': 'someone-else', _oc_kafka_headers: { 'x-oc-org': 'acme', 'x-oc-retention-days': '15' },
    _oc_kafka_key: 'acme', _oc_kafka_topic: 'oc.logs.json', _oc_kafka_partition: 3, _oc_kafka_offset: 42, message: 'payment authorized', service: 'billing',
  }],
  ['tests: ECS FireLens record with a JSON log (oc-intake JSON logs)', json({
    log: '{"level":"WARN","message":"slow upstream","latency_ms":812}', container_name: '/web',
    ecs_cluster: 'arn:aws:ecs:eu-west-1:123456789012:cluster/shop-prod', ecs_task_definition: 'web:17',
  })],
  ['tests: plain JSON with Datadog reserved names and ddtags', json({
    msg: 'card authorised', severity: 'debug', 'dd.service': 'billing', hostname: 'pay-1', ddtags: 'team:payments,env:staging', eventTime: '2026-10-25T18:00:02Z',
  })],
  ['tests: logs without an organization are dropped', { source_type: 'http_server', _t: T, message: 'who sent this?' }],
  ['tests: plain-text klog level', otlp({ message: 'E1025 18:47:43.427741       1 controller.go:12] sync failed', resources: { 'oc.org_id': 'acme', 'k8s.container.name': 'kube-controller-manager' } })],
  ['tests: plain-text leading level word', otlp({ message: '2026-10-25T18:47:43Z WARN disk almost full', resources: { 'oc.org_id': 'acme', 'k8s.container.name': 'catalog' } })],
  ['tests: Datadog Agent stderr line with a leading INFO level is info', agent('INFO:catalog.api.middleware: GET /api/v1/items 200', 'error')],
  ['tests: Datadog Agent stderr JSON line with level info is info', agent('{"level":"info","msg":"order placed"}', 'error', { ddsource: 'go', service: 'order-service' })],
  ['tests: Datadog Agent stderr line without a level keeps the stream status', agent('Traceback (most recent call last):', 'error', { service: 'billing' })],
  ['tests: Datadog Agent stderr klog info line is info', agent('I1027 19:54:21.008123       1 client.go:12] "remote connection EOF"', 'error')],
  ['tests: Datadog Agent stderr logfmt line with level=info is info', agent('level=info ts=2026-10-27T19:54:21Z caller=index_set.go:12 msg="uploading table"', 'error')],
  ['tests: Datadog Agent stderr Python line with a dashed level is info', agent('2026-10-27 19:54:21,008 - azure.identity._credentials.default - INFO - token acquired', 'error')],
  ['tests: Datadog Agent stderr Python line with a dashed WARNING level is warn', agent('2026-10-27 19:54:21,008 - catalog.cache - WARNING - cache miss storm', 'error')],
  ['tests: Datadog Agent stderr loguru line with a padded INFO level is info', agent('2026-11-01 10:22:11.690 | INFO     | db.reader:get_evaluation:69 - Retrieved evaluation', 'error', { service: 'system-evaluator-api' })],
  ['tests: Datadog Agent stderr loguru line with a padded ERROR level is error', agent('2026-11-01 10:22:11.690 | ERROR    | db.writer:save:12 - Insert failed', 'info', { service: 'system-evaluator-api' })],
  ["tests: Datadog Agent's own log line with a piped ERROR level is error", agent('2026-10-27 20:10:53 UTC | CORE | ERROR | (pkg/collector/worker/check_logger.go:71 in Error) | check:redisdb | Error running check', 'info', { ddsource: 'agent', service: 'agent' })],

  // Values: every type in attributes, nested objects, arrays, empty values.
  ['values: attribute types', otlp({
    resources: { 'oc.org_id': 'acme', 'service.name': 'order-service' }, message: 'typed',
    attributes: { s: 'text', i: 42, neg: -7, f: 1.5, whole: 100.0, tiny: 0.000001, big: 12345678901234, t: true, n: null, arr: [1, 'two', { three: 3 }], nested: { a: { b: 'deep', c: [1, 2] }, empty: {} }, empty_s: '', unicode: 'zażółć ✓' },
  })],
  ['values: JSON body object', otlp({ resources: { 'oc.org_id': 'acme' }, message: { msg: 'from object body', http: { status_code: 503, method: 'GET' }, level: 'warning' } })],
  ['values: JSON string body with whitespace and a non-object JSON', otlp({ resources: { 'oc.org_id': 'acme' }, message: '  [1,2,3]  ' })],
  ['values: message that is JSON inside a JSON field', json({ log: '{"msg":"inner","level":"debug","extra":{"k":"v"}}', stream: 'stdout' })],
  ['values: message JSON without a message field keeps the raw text', json({ log: '{"level":"error","code":17}' })],
  ['values: non-string message is encoded', json({ message: { code: 3, text: 'object message' } })],
  ['values: numeric message', json({ message: 12345 })],
  ['values: no message at all', json({ foo: 'bar' })],

  // Levels.
  ['levels: OTLP severity text wins', otlp({ resources: { 'oc.org_id': 'acme' }, severity_text: 'WARNING', message: 'ERROR in text' })],
  ['levels: OTLP numeric severity only (fatal)', otlp({ resources: { 'oc.org_id': 'acme' }, severity_number: 21, message: 'no level word here' })],
  ['levels: OTLP numeric severity only (warn)', otlp({ resources: { 'oc.org_id': 'acme' }, severity_number: 13, message: 'no level word here' })],
  ['levels: OTLP numeric severity only (debug)', otlp({ resources: { 'oc.org_id': 'acme' }, severity_number: 5, message: 'no level word here' })],
  ['levels: out of range severity number', otlp({ resources: { 'oc.org_id': 'acme' }, severity_number: 99, message: 'ok' })],
  ['levels: syslog severity', json({ message: 'disk', syslog: { severity: 'crit', appname: 'smartd', hostname: 'nas-1' } })],
  ['levels: bracketed level', json({ message: '[error] upstream timed out' })],
  ['levels: angle level', json({ message: '<warn> slow' })],
  ['levels: level after two words', json({ message: '2026-10-25 18:00:00 notice: started' })],
  ['levels: verbose and trace', json({ message: 'x', level: 'verbose' })],
  ['levels: single letter', json({ message: 'x', level: 'W' })],
  ['levels: emergency', json({ message: 'x', level: 'EMERGENCY' })],
  ['levels: numeric level field', json({ message: 'x', level: 3 })],
  ['levels: logfmt lvl', json({ message: 'ts=1 lvl=warn msg=hello' })],
  ['levels: unknown word', json({ message: 'x', level: 'chatty' })],

  // Timestamps.
  ['time: seconds', json({ message: 'x', timestamp: 1761415200 })],
  ['time: milliseconds', json({ message: 'x', date: 1761415200123 })],
  ['time: nanoseconds', json({ message: 'x', '@timestamp': 1761415200123456789 })],
  ['time: RFC 3339 with offset', json({ message: 'x', '@timestamp': '2026-10-25T21:00:00.5+03:00' })],
  ['time: unparseable string falls back', json({ message: 'x', eventTime: 'yesterday' })],
  ['time: OTLP event time', { source_type: 'opentelemetry', _t: '2026-10-24T01:02:03.456789Z', resources: { 'oc.org_id': 'acme' }, message: 'x' }],
  ['time: received_at fallback', { source_type: 'kafka', _oc_kafka_headers: { 'x-oc-org': 'acme' }, _oc_ingest: { id: 'not-a-uuid', received_at: '2026-10-25T17:59:00Z' }, _t: '2026-10-31T00:00:00Z', message: 'x' }],

  // Trace and span ids.
  ['trace: OTLP ids', otlp({ resources: { 'oc.org_id': 'acme' }, message: 'x', trace_id: '0123456789ABCDEF0123456789ABCDEF', span_id: '0123456789ABCDEF' })],
  ['trace: Datadog ids in attributes', json({ message: 'x', dd: { trace_id: 'abcdefabcdefabcdefabcdefabcdefab', span_id: 'abcdefabcdefabcd' } })],
  ['trace: zero and malformed ids', json({ message: 'x', trace_id: '00000000000000000000000000000000', span_id: 'xyz' })],
  ['trace: decimal Datadog id is not hex 32', json({ message: 'x', dd: { trace_id: '1234567890', span_id: '42' } })],

  // Tags, service, host, source.
  ['tags: ECS resources', otlp({ resources: { 'oc.org_id': 'acme', 'aws.ecs.cluster.arn': 'arn:aws:ecs:us-east-1:123456789012:cluster/prod', 'aws.ecs.task.family': 'web', 'aws.ecs.task.revision': '3', 'cloud.region': 'us-east-1', 'cloud.account.id': '123456789012' }, message: 'x' })],
  ['tags: image short name with tag and digest', otlp({ resources: { 'oc.org_id': 'acme', 'container.image.name': 'registry.example.com:5000/team/web:1.2@sha256:abc', 'container.name': 'web-1' }, message: 'x' })],
  ['tags: unknown_service placeholder is skipped', otlp({ resources: { 'oc.org_id': 'acme', 'service.name': 'unknown_service:python', 'k8s.deployment.name': 'catalog-service' }, message: 'x' })],
  ['tags: no service anywhere', otlp({ resources: { 'oc.org_id': 'acme' }, message: 'x' })],
  ['tags: deployment.environment and explicit env tag', { source_type: 'http_server', 'x-oc-org': 'acme', _t: T, tags: { env: 'prod' }, resources: { 'deployment.environment': 'staging', 'service.version': '9' }, message: 'x' }],
  ['tags: ddtags edge cases', json({ message: 'x', ddtags: ' a:1 , b , :c, d:e:f,,env:qa ' })],
  ['tags: ddsource and host from hostname', json({ message: 'x', ddsource: 'nginx', hostname: 'web-1', host: '' })],
  ['tags: resource values of other types', otlp({ resources: { 'oc.org_id': 'acme', 'process.pid': 4242, 'k8s.node.name': 'node-2', 'array.attr': ['a', 'b'] }, message: 'x' })],
  ['tags: FireLens container name and task definition without revision', json({ message: 'x', container_name: '/api', ecs_task_definition: 'api', ecs_task_arn: 'arn:aws:ecs:task/1' })],
  ['tags: syslog appname and dd.service precedence', json({ message: 'x', service: '', syslog: { appname: 'cron' }, dd: { service: 'later' } })],
  ['tags: scope name', otlp({ resources: { 'oc.org_id': 'acme' }, message: 'x', scope: { name: 'io.opentelemetry.logback', version: '1.0' } })],

  // Organization.
  ['org: from org_id field', { source_type: 'vector', _t: T, org_id: 'trusted-agent', message: 'x' }],
  ['org: Kafka header wins over the event field', { source_type: 'kafka', _t: T, org_id: 'spoofed', _oc_kafka_headers: { 'x-oc-org': 'acme' }, message: 'x' }],
  ['org: Kafka without a header is dropped', { source_type: 'kafka', _t: T, _oc_kafka_headers: {}, message: 'x' }],
];

// Lines shaped like the logs-benchmark generator's (VictoriaMetrics/logs-benchmark, internal/source/synthetic):
// 30-40 structured fields from a fixed seed.
let seed = 42;
const rand = () => ((seed = (seed * 1103515245 + 12345) % 2147483648) / 2147483648);
const pick = (xs) => xs[Math.floor(rand() * xs.length)];
for (let i = 0; i < 40; i++) {
  const attributes = { 'http.method': pick(['GET', 'POST', 'PUT']), 'http.status_code': pick([200, 201, 404, 500]), 'duration_ms': Math.round(rand() * 100000) / 100, 'user.id': `u-${Math.floor(rand() * 1e6)}`, retry: rand() < 0.5, region: pick(['eu-west-1', 'us-east-1']) };
  for (let k = 0; k < 28; k++) attributes[`field_${k}`] = pick([`v${Math.floor(rand() * 1000)}`, Math.floor(rand() * 1e4), rand() < 0.5]);
  const level = pick(['info', 'warn', 'error', 'debug']);
  const resources = { 'oc.org_id': 'acme', 'service.name': pick(['order-service', 'billing', 'catalog-service']), 'host.name': `i-0a1b2c3d4e5f600${i % 10}` };
  cases.push([`synthetic ${i}`, i % 2 ? otlp({ resources, attributes, severity_text: level, message: `request ${i} handled` }) : json({ ...attributes, level, message: `request ${i} handled`, service: resources['service.name'], host: resources['host.name'] })]);
}

const dir = mkdtempSync(join(tmpdir(), 'oc-normalize-'));
writeFileSync(join(dir, 'input.ndjson'), cases.map(([, e], i) => JSON.stringify({ ...e, _i: i })).join('\n') + '\n');
// `_i` pairs each answer with its case: a dropped log answers `aborted`, without it.
const prelude = 'ix = ._i\ndel(._i)\nif exists(._t) { .timestamp = parse_timestamp!(._t, "%+"); del(._t) }\n';
const vrl = readFileSync(join(root, 'deploy/vector/normalize.vrl'), 'utf8');
writeFileSync(join(dir, 'program.vrl'), prelude + vrl + '\nencode_json({ "i": ix, "row": . })\n');
const stdout = execFileSync('docker', ['run', '--rm', '-v', `${dir}:/w`, image, 'vrl', '--input', '/w/input.ndjson', '--program', '/w/program.vrl'], { encoding: 'utf8' });
const lines = stdout.split('\n');
const errors = lines.filter((l) => l.startsWith('error'));
if (errors.length) throw new Error(errors.join('\n'));
const rows = cases.map(() => null);
for (const l of lines.filter((l) => l.startsWith('"'))) {
  const { i, row } = JSON.parse(JSON.parse(l));
  rows[i] = row;
}
// Only the cases named as dropped may have no row.
const wrong = cases.filter(([name], i) => (rows[i] === null) !== name.includes('dropped')).map(([name]) => name);
if (wrong.length) throw new Error(`rows missing or unexpected for: ${wrong.join('; ')}`);

writeFileSync(join(out, 'cases.ndjson'), cases.map(([name, event], i) => JSON.stringify({ name, event, expected: rows[i] })).join('\n') + '\n');
console.log(`${cases.length} cases (${rows.filter((r) => r === null).length} dropped) → ${join(out, 'cases.ndjson')} (${image})`);
