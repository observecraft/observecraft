import assert from 'node:assert/strict';
import test from 'node:test';
import { configFrom, phases, distribution, kafkaOffsets, totalLag, memoryBytes, percentile, stageMetrics, stageDelta } from './ingestion-benchmark/model.mjs';

test('ingestion stage timings aggregate replicas and subtract warmup without inventing absent metrics', () => {
  const parsed = stageMetrics({
    api1: 'oc_log_ingest_stage_seconds_count{stage="admission"} 3\noc_log_ingest_stage_seconds_sum{stage="admission"} 0.75\n',
    api2: 'oc_log_ingest_stage_seconds_count{stage="admission"} 2\noc_log_ingest_stage_seconds_sum{stage="admission"} 0.25\n# ignored',
  });
  assert.deepEqual(stageDelta({ admission: { count: 1, seconds: 0.2 } }, parsed), { admission: { count: 4, seconds: 0.8, meanMs: 200 } });
  assert.deepEqual(stageMetrics({ oldApi: 'oc_cache_hits_total 1' }), {});
  assert.deepEqual(stageDelta({}, { admission: { count: 0, seconds: 0 } }).admission.meanMs, null);
});
test('counter resets and invalid ingestion stage observations fail instead of reporting a speedup', () => {
  assert.throws(() => stageDelta({ admission: { count: 5, seconds: 1 } }, { admission: { count: 1, seconds: 0.2 } }));
  assert.throws(() => stageMetrics({ api1: 'oc_log_ingest_stage_seconds_sum{stage="admission"} 1e999' }));
});

test('latency percentiles use actual nearest-rank observations without mutating samples', () => {
  const values = Array.from({ length: 20 }, (_, n) => 20 - n);
  assert.deepEqual(distribution(values), { samples: 20, p50: 10, p95: 19, p99: 20, max: 20 });
  assert.equal(values[0], 20);
  assert.equal(percentile([7], 0.99), 7);
});
test('absent latency observations remain unknown, not a zero-millisecond success', () => {
  assert.deepEqual(distribution([]), { samples: 0, p50: null, p95: null, p99: null, max: null });
});
test('the maximum supported sample count does not depend on JavaScript argument limits', () => {
  assert.equal(distribution(Array.from({ length: 100000 }, (_, i) => i)).max, 99999);
});
test('Docker working-set units distinguish binary MiB from decimal MB', () => {
  assert.equal(memoryBytes('1.5MiB'), 1572864);
  assert.equal(memoryBytes('1.5MB'), 1500000);
  assert.throws(() => memoryBytes('unknown'));
});
test('Kafka lag is measured in records and only for the target consumer and topic', () => {
  const output =
    'GROUP TOPIC PARTITION CURRENT-OFFSET LOG-END-OFFSET LAG\noc-vector oc.logs.json 1 5 8 3\noc-other oc.logs.json 0 0 100 100\noc-vector oc.logs.live 0 0 200 200\noc-vector oc.logs.json 0 2 3 1';
  assert.deepEqual(
    kafkaOffsets(output).map((p) => p.partition),
    [0, 1],
  );
  assert.equal(totalLag(kafkaOffsets(output)), 4);
});
test('unknown offsets are inferred from zero only for explicitly fresh benchmark topics', () => {
  const nonempty = 'oc-vector oc.logs.json 0 - 10 -';
  assert.equal(totalLag(kafkaOffsets(nonempty)), null);
  assert.equal(totalLag(kafkaOffsets(nonempty, 'oc.logs.json', true)), 10);
  assert.equal(totalLag(kafkaOffsets('oc-vector oc.logs.json 0 - 0 -')), 0);
  assert.equal(totalLag(kafkaOffsets('consumer is starting')), null);
});
test('the default matrix compares both rates before adding metric and storage-fault profiles', () => {
  const cfg = configFrom({});
  assert.equal(cfg.replicas, 2);
  assert.deepEqual(
    phases(cfg).map((p) => p.name),
    ['unlimited-1000', 'quota-1000', 'unlimited-5000', 'quota-5000', 'metrics-5000', 'slow-5000', 'paused-5000'],
  );
  assert.equal(phases(cfg).at(-1).holdSeconds, 5);
});
test('invalid rates and excessive request counts fail before starting any containers', () => {
  for (const rates of ['', '1000,1000', 'abc', '0', '100001']) assert.throws(() => configFrom({ OC_BENCH_RATES: rates }));
  assert.throws(() => configFrom({ OC_BENCH_SECONDS: '300', OC_BENCH_RATES: '100000', OC_BENCH_BATCH: '10' }));
  assert.throws(() => configFrom({ OC_BENCH_API_REPLICAS: '0' }));
});
test('throughput ramps and fault experiments can run independently', () => {
  assert.equal(phases(configFrom({ OC_BENCH_MODE: 'throughput' })).length, 5);
  assert.deepEqual(
    phases(configFrom({ OC_BENCH_MODE: 'faults' })).map((p) => p.name),
    ['slow-5000', 'paused-5000'],
  );
  assert.throws(() => configFrom({ OC_BENCH_MODE: 'anything' }));
});
