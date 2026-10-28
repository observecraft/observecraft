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
const gpu = await import('../web/src/api/mock/gpu.ts');

const HOUR = 3_600_000;
const range = { from: 1_760_000_000_000 - HOUR, to: 1_760_000_000_000 };

test('states follow the utilization thresholds', () => {
  assert.equal(gpu.gpuState(null), 'no_data');
  assert.equal(gpu.gpuState(0), 'idle');
  assert.equal(gpu.gpuState(4.9), 'idle');
  assert.equal(gpu.gpuState(5), 'underutilized');
  assert.equal(gpu.gpuState(29.9), 'underutilized');
  assert.equal(gpu.gpuState(30), 'active');
});

test('the fleet has busy, underutilized and idle GPUs on several hosts', async () => {
  const page = await gpu.searchGpus({ range, query: '' });
  assert.equal(page.enabled, true);
  assert.equal(page.summary.gpus, 10);
  assert.equal(page.summary.hosts, 4);
  assert.ok(page.summary.idle >= 3, 'idle GPUs to reclaim');
  assert.ok(page.summary.underutilized >= 1);
  assert.ok(page.summary.memUsedBytes > 0 && page.summary.memUsedBytes < page.summary.memTotalBytes);
  for (const d of page.devices) {
    assert.ok(d.host.startsWith('i-0a1b2c3d4e5f6000'), 'fictional hosts');
    assert.equal(d.state, gpu.gpuState(d.utilPct));
  }
  const model = page.facets.find((f) => f.key === 'model');
  assert.deepEqual(
    model.values.map((v) => v.value),
    ['NVIDIA A10G', 'NVIDIA L4', 'NVIDIA T4'],
  );
});

test('the query filters the devices; facets keep counting the whole fleet', async () => {
  const page = await gpu.searchGpus({ range, query: 'state:idle' });
  assert.ok(page.devices.length >= 3);
  assert.ok(page.devices.every((d) => d.state === 'idle'));
  assert.equal(page.summary.gpus, page.devices.length);
  assert.equal(page.facets.find((f) => f.key === 'host').values.length, 4);
  const ml = await gpu.searchGpus({ range, query: 'cluster:shop-ml model:"NVIDIA A10G"' });
  assert.equal(ml.devices.length, 4);
  const text = await gpu.searchGpus({ range, query: 'recs-trainer' });
  assert.equal(text.devices.length, 4, 'free text matches workloads');
});

test('summary math', () => {
  const s = gpu.summarize([
    { host: 'a', utilPct: 10, memUsedBytes: 1, memTotalBytes: 4, state: 'underutilized' },
    { host: 'a', utilPct: null, memUsedBytes: null, memTotalBytes: 4, state: 'no_data' },
    { host: 'b', utilPct: 1, memUsedBytes: 2, memTotalBytes: 4, state: 'idle' },
  ]);
  assert.deepEqual(s, { gpus: 3, hosts: 2, avgUtilPct: 5.5, memUsedBytes: 3, memTotalBytes: 12, idle: 1, underutilized: 1 });
});

test('fleet series cover the range', async () => {
  const series = await gpu.gpuTimeseries({ range, query: '' });
  assert.deepEqual(
    series.map((s) => s.metric),
    ['utilization', 'memory_used', 'temperature', 'power'],
  );
  assert.ok(series[0].points.length >= 50);
  assert.ok(series[0].points.every(([, v]) => v >= 0 && v <= 100));
  const none = await gpu.gpuTimeseries({ range, query: 'host:nothing' });
  assert.equal(none[0].points.length, 0);
  const byHost = await gpu.gpuTimeseries({ range, query: '', groupBy: 'host' });
  const util = byHost.filter((s) => s.metric === 'utilization');
  assert.equal(util.length, 4, 'one utilization series per host');
  assert.ok(util.every((s) => s.group.startsWith('i-0a1b2c3d4e5f6000')));
});
