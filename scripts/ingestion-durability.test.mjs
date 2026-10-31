// Configuration invariants protecting Kafka ownership through the complete Collector pipeline.
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import test from 'node:test';

const require = createRequire(new URL('../web/package.json', import.meta.url));
const { load } = require('js-yaml');
const config = load(readFileSync(new URL('../deploy/otel/collector.yaml', import.meta.url), 'utf8'));
const vector = load(readFileSync(new URL('../deploy/vector/aggregator.yaml', import.meta.url), 'utf8'));

test('every storage exporter preserves Kafka ownership until export, with unlimited transient retries', () => {
  for (const [name, exporter] of Object.entries(config.exporters)) {
    assert.equal(exporter.sending_queue.wait_for_result, true, `${name}: enqueueing is not delivery`);
    assert.equal(exporter.retry_on_failure.enabled, true, name);
    assert.equal(exporter.retry_on_failure.max_elapsed_time, '0s', `${name}: storage outages must not expire accepted data`);
    assert.equal(exporter.sending_queue.storage, undefined, `${name}: Collector cannot await a persistent queue`);
    assert.equal(exporter.sending_queue.sizer, 'bytes', name);
    assert.ok(exporter.sending_queue.queue_size <= 32 * 1024 * 1024, `${name}: working memory is bounded`);
  }
  for (const [name, receiver] of Object.entries(config.receivers)) {
    if (!name.startsWith('kafka/')) continue;
    assert.deepEqual(receiver.message_marking, { after: true, on_error: false }, name);
    assert.equal(receiver.error_backoff.enabled, true, name);
  }
  for (const [name, pipeline] of Object.entries(config.service.pipelines)) {
    assert.ok(!(pipeline.processors ?? []).some((processor) => processor.split('/')[0] === 'batch'), `${name}: a batch processor can return before export`);
  }
});

test('Vector acknowledges durable log sinks after delivery and keeps live tail best effort', () => {
  for (const name of ['clickhouse', 'indexer']) {
    assert.equal(vector.sinks[name].acknowledgements.enabled, true, name);
    assert.equal(vector.sinks[name].buffer.type, 'memory', `${name}: local disk persistence must not release Kafka ownership`);
    assert.equal(vector.sinks[name].buffer.when_full, 'block', name);
  }
  assert.equal(vector.sinks.live.acknowledgements.enabled, false);
  assert.equal(vector.sinks.live.buffer.when_full, 'drop_newest');
});
