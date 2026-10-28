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
const { collectorConfig } = await import('../web/src/pages/setup/collector.ts');
const { formatDuration } = await import('../web/src/pages/data/format.ts');
const { ENGINES, engineOptions, JOB_SETUP } = await import('../web/src/pages/data/dataSetup.ts');

test('collector config wires the receiver into a metrics pipeline to the intake', () => {
  const yaml = collectorConfig('redis', '  redis:\n    endpoint: localhost:6379');
  assert.match(yaml, /^receivers:\n {2}redis:\n {4}endpoint: localhost:6379\n/);
  assert.match(yaml, /metrics\/redis:\n {6}receivers: \[redis\]/);
  assert.match(yaml, /endpoint: \{\{intake\}\}/);
  assert.match(yaml, /authorization: "Bearer \{\{apiKey\}\}"/);
  const prom = collectorConfig('gpu', '  prometheus/dcgm:\n    config: {}');
  assert.match(prom, /receivers: \[prometheus\/dcgm\]/);
});

test('durations read naturally', () => {
  assert.equal(formatDuration(null), '—');
  assert.equal(formatDuration(850), '850 ms');
  assert.equal(formatDuration(42_000), '42s');
  assert.equal(formatDuration(192_000), '3m 12s');
  assert.equal(formatDuration(3_720_000), '1h 02m');
});

test('every engine has Agent and OpenTelemetry steps; every job source has steps', () => {
  assert.deepEqual(
    ENGINES.map((e) => e.engine),
    ['postgres', 'mysql', 'sqlserver', 'mongodb', 'redis'],
  );
  for (const e of ENGINES) {
    const [agent, otel] = engineOptions(e);
    assert.ok(
      agent.steps.some((s) => s.code?.includes('instances:')),
      `${e.name}: an Agent check config`,
    );
    assert.ok(
      otel.steps.some((s) => s.code?.includes('otlphttp/observecraft')),
      `${e.name}: a Collector config`,
    );
  }
  assert.ok(JOB_SETUP.every((o) => o.steps.length > 0));
});
