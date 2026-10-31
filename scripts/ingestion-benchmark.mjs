// Isolated, resource-capped JSON log pipeline benchmark. Requires a current application image.
//   node scripts/ingestion-benchmark.mjs
// See docs/benchmarks/ingestion.md for scope, configuration and interpretation.
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { createHash } from 'node:crypto';
import { readFileSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { configFrom, phases, kafkaOffsets, totalLag, memoryBytes } from './ingestion-benchmark/model.mjs';

const cfg = configFrom(process.env);
const root = fileURLToPath(new URL('../', import.meta.url));
const scripts = fileURLToPath(new URL('./ingestion-benchmark', import.meta.url));
const prefix = `oc-bench-${process.pid}-${Date.now()}`;
const network = `${prefix}-net`;
const apiNames = Array.from({ length: cfg.replicas }, (_, n) => `api${n + 1}`);
const names = Object.fromEntries(['clickhouse', 'postgres', 'kafka', 'observer', 'valkey', 'proxy', 'migrate', 'intake', 'vector', 'driver', ...apiNames].map((n) => [n, `${prefix}-${n}`]));
const caps = {
  clickhouse: [4, '3g'],
  postgres: [2, '1g'],
  kafka: [2, '2g'],
  observer: [1, '512m'],
  valkey: [0.5, '256m'],
  proxy: [1, '256m'],
  intake: [2, '512m'],
  vector: [2, '512m'],
  driver: [1, '256m'],
  ...Object.fromEntries(apiNames.map((n) => [n, [2, '768m']])),
};
const plan = phases(cfg).map((phase, n) => ({
  ...phase,
  orgs: Array.from({ length: cfg.orgs }, (_, i) => `bench-${n}-org-${i}`),
  keys: Array.from({ length: cfg.orgs }, (_, i) => `ocik_disposable_bench_${n}_${i}`),
}));
const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
let stopping = false,
  admin;
const commands = new Set();
const stop = () => {
  stopping = true;
  for (const child of commands) child.kill();
};
process.on('SIGINT', stop);
process.on('SIGTERM', stop);
async function docker(args, input, allowFailure = false, timeout = 120000) {
  return new Promise((resolve, reject) => {
    const child = spawn('docker', args, { stdio: ['pipe', 'pipe', 'pipe'] });
    commands.add(child);
    let stdout = '',
      stderr = '';
    const timer = setTimeout(() => child.kill(), timeout);
    child.stdout.on('data', (b) => {
      stdout += b;
    });
    child.stderr.on('data', (b) => {
      stderr += b;
    });
    child.on('error', reject);
    child.on('close', (code) => {
      clearTimeout(timer);
      commands.delete(child);
      if (!allowFailure && code !== 0) reject(new Error(`docker ${args.slice(0, 3).join(' ')}: ${stderr || stdout}`));
      else resolve((allowFailure ? stdout + stderr : stdout).trim());
    });
    child.stdin.on('error', () => {});
    child.stdin.end(input);
  });
}
function run(name, args, image, command = []) {
  const cap = caps[name];
  return docker(['run', '-d', '--name', names[name], '--network', network, '--network-alias', name, ...(cap ? ['--cpus', String(cap[0]), '--memory', cap[1]] : []), ...args, image, ...command]);
}
async function until(check, description, seconds = 120) {
  const end = Date.now() + seconds * 1000;
  while (Date.now() < end) {
    if (stopping) throw new Error('Benchmark interrupted');
    try {
      if (await check()) return;
    } catch {
      /* startup or intermediate state */
    }
    await sleep(1000);
  }
  throw new Error(`Timed out: ${description}`);
}
function pg(query, variables = {}) {
  return docker(
    ['exec', '-i', names.postgres, 'psql', '-U', 'observecraft', '-d', 'observecraft', '-At', '-v', 'ON_ERROR_STOP=1', ...Object.entries(variables).flatMap(([k, v]) => ['-v', `${k}=${v}`])],
    query,
  );
}
async function ch(query, orgs, parameters = {}) {
  const url = new URL(`${admin}/query`);
  const params = { query: `${query} FORMAT JSONEachRow`, ...parameters };
  orgs.forEach((org, i) => {
    params[`param_org${i}`] = org;
  });
  const r = await fetch(url, { method: 'POST', body: JSON.stringify(params), signal: AbortSignal.timeout(30000) });
  if (!r.ok) throw new Error(await r.text());
  const text = (await r.text()).trim();
  return text ? text.split('\n').map(JSON.parse) : [];
}
async function control(value) {
  const r = await fetch(`${admin}/control`, { method: 'POST', body: JSON.stringify(value), signal: AbortSignal.timeout(5000) });
  assert.equal(r.status, 200);
  return r.json();
}
async function offsets() {
  return kafkaOffsets(
    await docker(['exec', names.observer, '/opt/kafka/bin/kafka-consumer-groups.sh', '--bootstrap-server', 'kafka:9092', '--describe', '--group', 'oc-vector'], undefined, true),
    'oc.logs.json',
    true,
  );
}
const clause = (orgs, column) => `${column} IN (${orgs.map((_, i) => `{org${i}:String}`).join(',')})`;
async function stored(phase) {
  const where = clause(phase.orgs, 'org_id');
  const [counts] = await ch(`SELECT (SELECT count() FROM oc.logs_raw_read WHERE ${where}) AS raw, (SELECT count() FROM oc.logs_read WHERE ${where}) AS indexed`, phase.orgs);
  const variables = Object.fromEntries(phase.orgs.map((org, i) => [`org${i}`, org]));
  const condition = phase.orgs.map((_, i) => `:'org${i}'`).join(',');
  const metadata = JSON.parse(
    await pg(
      `SELECT json_build_object(
    'receipts', (SELECT count(*) FROM log_quota_admissions WHERE org_id IN (${condition})),
    'reserved', (SELECT COALESCE(sum(n),0) FROM log_quota_counters WHERE org_id IN (${condition})),
    'outboxBatches', (SELECT count(*) FROM log_metric_outbox WHERE org_id IN (${condition})),
    'outboxBytes', (SELECT COALESCE(sum(octet_length(sums)::bigint+octet_length(histograms)),0) FROM log_metric_outbox WHERE org_id IN (${condition})),
    'admissionTableBytes', pg_total_relation_size('log_quota_admissions'),
    'outboxTableBytes', pg_total_relation_size('log_metric_outbox'),
    'walLsn', pg_current_wal_lsn()::text);`,
      variables,
    ),
  );
  const metrics = phase.metrics
    ? Number((await ch(`SELECT sum(Value) AS n FROM otel.metrics_samples_read WHERE ${clause(phase.orgs, 'OrgId')} AND MetricName = 'logs.bench.count'`, phase.orgs))[0].n)
    : null;
  return { raw: Number(counts.raw), indexed: Number(counts.indexed), metrics, ...metadata };
}
async function resourceSample(phase, started) {
  const [resources, kafka, storage, proxy] = await Promise.all([
    docker(['stats', '--no-stream', '--format', '{{json .}}', ...Object.keys(caps).map((n) => names[n])]),
    offsets(),
    stored(phase),
    fetch(`${admin}/status`).then((r) => r.json()),
  ]);
  return {
    at: new Date().toISOString(),
    elapsedSeconds: (Date.now() - started) / 1000,
    lagRecords: totalLag(kafka),
    partitions: kafka,
    storage,
    proxy,
    resources: resources
      .split('\n')
      .filter(Boolean)
      .map((line) => {
        const row = JSON.parse(line);
        return { component: row.Name.replace(`${prefix}-`, ''), cpuPercent: Number(row.CPUPerc.replace('%', '')), memoryBytes: memoryBytes(row.MemUsage.split('/')[0]) };
      }),
  };
}
function drive(phase, total, rate) {
  return docker(
    [
      'exec',
      names.driver,
      'node',
      '/bench/load.mjs',
      JSON.stringify({ orgs: phase.orgs, keys: phase.keys, total, rate, batch: cfg.batch, concurrency: cfg.concurrency, messageBytes: cfg.messageBytes }),
    ],
    undefined,
    false,
    (cfg.seconds + 180) * 1000,
  ).then(JSON.parse);
}
async function measure(phase) {
  console.log(`Warmup: ${phase.name}`);
  const warmCount = cfg.orgs * cfg.batch;
  await drive(phase, warmCount, 10000);
  await until(async () => {
    const s = await stored(phase);
    return s.raw === warmCount && s.indexed === warmCount && s.outboxBatches === 0 && (!phase.metrics || s.metrics === warmCount) && totalLag(await offsets()) === 0;
  }, `${phase.name}: warmup drained`);
  const baseline = await stored(phase);
  const startOffsets = await offsets();
  const samples = [],
    samplingErrors = [];
  const started = Date.now();
  await control({ reset: true, delayMs: phase.delayMs ?? 0, hold: Boolean(phase.holdSeconds) });
  let sampling = true;
  const monitor = (async () => {
    while (sampling) {
      try {
        samples.push(await resourceSample(phase, started));
      } catch (e) {
        samplingErrors.push(String(e));
      }
      if (sampling) await sleep(1000);
    }
  })();
  let release, releaseError;
  const releaseJob = phase.holdSeconds
    ? (async () => {
        await sleep(phase.holdSeconds * 1000);
        // Snapshot before release proves committed offsets did not advance through held storage.
        const during = await offsets();
        release = { at: new Date().toISOString(), offsets: during, proxy: await control({ hold: false }) };
        assert.deepEqual(
          during.map((p) => p.committed),
          startOffsets.map((p) => p.committed),
          'held storage must not advance Kafka commits',
        );
      })().catch((e) => {
        releaseError = e;
      })
    : Promise.resolve();
  let load, final, completed;
  try {
    console.log(`Measure: ${phase.name}, ${phase.rate} logs/s, ${cfg.seconds}s`);
    load = await drive(phase, phase.rate * cfg.seconds, phase.rate);
    await releaseJob;
    if (releaseError) throw releaseError;
    await control({ delayMs: 0, hold: false });
    const expected = warmCount + load.accepted;
    await until(
      async () => {
        final = await stored(phase);
        return final.raw >= expected && final.indexed >= expected && final.outboxBatches === 0 && (!phase.metrics || final.metrics >= expected) && totalLag(await offsets()) === 0;
      },
      `${phase.name}: acknowledged rows stored and committed`,
      120,
    );
    completed = Date.now();
  } finally {
    try {
      await control({ delayMs: 0, hold: false });
    } catch {
      /* cleanup still runs after an interrupted control request */
    }
    sampling = false;
    await Promise.allSettled([monitor, releaseJob]);
  }
  assert.ok(final.raw >= warmCount + load.accepted && final.raw <= warmCount + load.offered, 'stored distinct events stay between acknowledged and offered source counts');
  assert.equal(final.indexed, final.raw, 'every raw log reaches the index');
  // Unacknowledged-but-stored requests must not hide a missing acknowledged batch in the totals.
  const batches = await ch(
    `SELECT JSONExtractString(attributes, 'bench.request') AS batch, count() AS n FROM oc.logs_read WHERE ${clause(phase.orgs, 'org_id')} AND startsWith(JSONExtractString(attributes, 'bench.request'), {run:String}) GROUP BY batch`,
    phase.orgs,
    { param_run: load.runToken },
  );
  const indexedBatches = new Map(batches.map((b) => [b.batch, Number(b.n)]));
  for (const [batch, count] of Object.entries(load.acknowledgedBatches)) assert.equal(indexedBatches.get(batch), count, `acknowledged batch ${batch} is present in full`);
  if (phase.quota) {
    assert.equal(final.receipts, final.raw);
    assert.equal(Number(final.reserved), final.raw);
  } else assert.equal(final.receipts, 0, 'unlimited orgs do not create quota receipts');
  if (phase.metrics) assert.equal(final.metrics, final.raw, 'generated count equals distinct source logs');
  if (phase.holdSeconds) assert.ok(release.proxy.held > 0, 'fault must intercept an actual INSERT');
  const walBytes = Number(await pg("SELECT pg_wal_lsn_diff(pg_current_wal_lsn(), :'start')::bigint;", { start: baseline.walLsn }));
  const peaks = {};
  for (const s of samples)
    for (const r of s.resources) {
      const peak = (peaks[r.component] ??= { cpuPercent: 0, memoryBytes: 0 });
      peak.cpuPercent = Math.max(peak.cpuPercent, r.cpuPercent);
      peak.memoryBytes = Math.max(peak.memoryBytes, r.memoryBytes);
    }
  const result = {
    name: phase.name,
    orgs: phase.orgs,
    quota: phase.quota,
    metrics: Boolean(phase.metrics),
    offeredRate: phase.rate,
    durationSeconds: cfg.seconds,
    storageDelayMs: phase.delayMs ?? 0,
    holdSeconds: phase.holdSeconds ?? 0,
    load,
    totalToCommitSeconds: (completed - started) / 1000,
    drainedAfterSenderSeconds: (completed - Date.parse(load.sendFinishedAt)) / 1000,
    peakLagRecords: Math.max(0, ...samples.map((s) => s.lagRecords ?? 0)),
    sampledPeakOutboxBytes: Math.max(0, ...samples.map((s) => s.storage.outboxBytes)),
    sampledResourcePeaks: peaks,
    walBytes,
    unacknowledgedStored: final.raw - warmCount - load.accepted,
    walBytesPerAcceptedLog: walBytes / Math.max(1, load.accepted),
    admissionTableGrowthBytes: final.admissionTableBytes - baseline.admissionTableBytes,
    baseline,
    final,
    release,
    samples,
    samplingErrors,
  };
  console.log(
    `${phase.name}: accepted=${load.accepted}, stored=${final.indexed - warmCount}, ACK p95=${load.intakeAckMs.p95?.toFixed(1)}ms, readable probe p95=${load.indexedProbeVisibilityMs.p95?.toFixed(1)}ms, peak lag=${result.peakLagRecords} Kafka records`,
  );
  return result;
}

const report = { schemaVersion: 1, startedAt: new Date().toISOString(), config: cfg, caps, phases: [] };
const output = cfg.output ?? join(tmpdir(), `${prefix}.json`);
try {
  const [image, engine] = await Promise.all([docker(['image', 'inspect', cfg.image, '--format', '{{json .Config.Labels}}']), docker(['info', '--format', '{{json .}}'])]);
  report.applicationRevision = JSON.parse(image)['org.opencontainers.image.revision'] ?? null;
  const info = JSON.parse(engine);
  report.host = { cpus: info.NCPU, memoryBytes: info.MemTotal, os: info.OperatingSystem, architecture: info.Architecture, dockerVersion: info.ServerVersion };
  report.harnessSha256 = createHash('sha256')
    .update(readFileSync(fileURLToPath(import.meta.url)))
    .update(readFileSync(join(scripts, 'model.mjs')))
    .update(readFileSync(join(scripts, 'proxy.mjs')))
    .update(readFileSync(join(scripts, 'load.mjs')))
    .digest('hex');
  report.vectorConfigSha256 = Object.fromEntries(
    ['aggregator.yaml', 'normalize.vrl'].map((name) => [
      name,
      createHash('sha256')
        .update(readFileSync(join(root, 'deploy/vector', name)))
        .digest('hex'),
    ]),
  );
  await docker(['network', 'create', network]);
  await Promise.all([
    run('clickhouse', ['-e', 'CLICKHOUSE_SKIP_USER_SETUP=1'], 'clickhouse/clickhouse-server:26.8'),
    run('postgres', ['-e', 'POSTGRES_USER=observecraft', '-e', 'POSTGRES_PASSWORD=observecraft', '-e', 'POSTGRES_DB=observecraft'], 'postgres:16'),
    run('valkey', [], 'valkey/valkey:8.1.11-alpine'),
    run('proxy', ['-v', `${scripts}:/bench:ro`, '-p', '127.0.0.1::8080', '-e', `BENCH_APIS=${apiNames.join(',')}`], 'node:24-bookworm-slim', ['node', '/bench/proxy.mjs']),
    run('driver', ['-v', `${scripts}:/bench:ro`, '--entrypoint', 'sleep'], 'node:24-bookworm-slim', ['infinity']),
    run('observer', ['-e', 'KAFKA_HEAP_OPTS=-Xms64m -Xmx256m', '--entrypoint', 'sleep'], 'apache/kafka:4.1.0', ['infinity']),
    run(
      'kafka',
      [
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
        'KAFKA_HEAP_OPTS=-Xms256m -Xmx768m',
      ],
      'apache/kafka:4.1.0',
    ),
  ]);
  admin = `http://${await docker(['port', names.proxy, '8080/tcp'])}`;
  await until(async () => (await docker(['exec', names.postgres, 'pg_isready', '-U', 'observecraft'], undefined, true)).includes('accepting connections'), 'Postgres');
  const appEnv = [
    '-e',
    'OC_POSTGRES_URL=postgres://observecraft:observecraft@postgres:5432/observecraft',
    '-e',
    'OC_CLICKHOUSE_URL=http://proxy:8123',
    '-e',
    'OC_CLICKHOUSE_USER=default',
    '-e',
    'OC_CLICKHOUSE_PASSWORD=',
    '-e',
    'OC_SEED_DEMO=false',
    '-e',
    'OC_CACHE_URL=redis://valkey:6379/0',
    '-e',
    'OC_KAFKA_BROKERS=kafka:9092',
    '-e',
    'OC_OSV_URL=off',
    '-e',
    'OC_INTERNAL_TOKEN=disposable-benchmark-token',
    '-e',
    'OC_SECRET_KEY=disposable-benchmark-secret',
  ];
  await run('migrate', [...appEnv, '--entrypoint', '/usr/local/bin/observecraft-server'], cfg.image, ['migrate']);
  assert.equal(await docker(['wait', names.migrate], undefined, false, 300000), '0', 'application migrations');
  for (const phase of plan)
    for (let i = 0; i < phase.orgs.length; i++)
      await pg(
        `
    INSERT INTO orgs(id,name) VALUES (:'org',:'org');
    INSERT INTO ingest_keys(org_id,name,key_hash,prefix) VALUES (:'org','benchmark',decode(:'hash','hex'),'ocik_bench');
    INSERT INTO log_indexes(org_id,name,filter,retention_days,daily_quota,position) VALUES (:'org','main','',7,${phase.quota ? '1000000000' : 'NULL'},0);
    ${phase.metrics ? "INSERT INTO log_metrics(org_id,name,query,aggregation) VALUES (:'org','logs.bench.count','','count');" : ''}
  `,
        { org: phase.orgs[i], hash: createHash('sha256').update(phase.keys[i]).digest('hex') },
      );
  await until(async () => (await docker(['exec', names.kafka, '/opt/kafka/bin/kafka-broker-api-versions.sh', '--bootstrap-server', 'kafka:9092'], undefined, true)).includes('kafka:9092'), 'Kafka');
  for (const topic of ['oc.logs.json', 'oc.logs.live'])
    await docker([
      'exec',
      names.kafka,
      '/opt/kafka/bin/kafka-topics.sh',
      '--bootstrap-server',
      'kafka:9092',
      '--create',
      '--topic',
      topic,
      '--partitions',
      '12',
      '--replication-factor',
      '1',
      '--config',
      'max.message.bytes=67108864',
    ]);
  await Promise.all(apiNames.map((name) => run(name, [...appEnv, '-e', 'OC_RUN_MIGRATIONS=false', '--entrypoint', '/usr/local/bin/observecraft-server'], cfg.image)));
  await run(
    'intake',
    ['-e', 'OC_POSTGRES_URL=postgres://observecraft:observecraft@postgres:5432/observecraft', '-e', 'OC_KAFKA_BROKERS=kafka:9092', '--entrypoint', '/usr/local/bin/oc-intake'],
    cfg.image,
  );
  await until(async () => (await fetch(`${admin}/ready`)).ok, 'APIs and intake');
  await run(
    'vector',
    [
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
      '-e',
      'OC_INTERNAL_TOKEN=disposable-benchmark-token',
    ],
    'timberio/vector:0.42.0-debian',
    ['--config', '/etc/vector/aggregator.yaml'],
  );
  report.images = (await docker(['inspect', '--format', '{{.Name}} {{.Image}} {{.Config.Image}}', ...Object.values(names)])).split('\n').map((line) => {
    const [name, digest, reference] = line.split(' ');
    return { component: name.replace(`/${prefix}-`, ''), digest, reference };
  });
  for (const phase of plan) {
    if (stopping) throw new Error('Benchmark interrupted');
    report.phases.push(await measure(phase));
    writeFileSync(output, JSON.stringify(report, null, 2));
  }
  report.completedAt = new Date().toISOString();
  console.log(`Report: ${output}`);
} catch (e) {
  report.error = String(e.stack ?? e);
  console.error(report.error);
  for (const name of ['vector', ...apiNames, 'proxy']) console.error(await docker(['logs', '--tail', '12', names[name]], undefined, true));
  process.exitCode = 1;
} finally {
  try {
    writeFileSync(output, JSON.stringify(report, null, 2));
  } finally {
    if (admin) {
      try {
        await control({ hold: false, delayMs: 0 });
      } catch {}
    }
    await Promise.all(Object.values(names).map((name) => docker(['rm', '-f', '-v', name], undefined, true)));
    await docker(['network', 'rm', network], undefined, true);
  }
}
