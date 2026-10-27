import assert from 'node:assert/strict';
import test from 'node:test';
import { hasReportedMetrics, reportedMetricNames } from '../web/src/api/integrations/detection.ts';

test('an empty organization and built-in trace catalog do not detect integrations', () => {
  const names = reportedMetricNames([{ name: 'trace.redis.hits', kind: 'span' }]);
  assert.equal(names.size, 0);
  assert.equal(hasReportedMetrics('redis', [{ name: 'redis.hits' }], names), false);
});

test('specific received OTel and Datadog metrics identify only the matching integration', () => {
  for (const metric of ['postgresql.connections', 'postgres.connections']) {
    const names = new Set([metric]);
    assert.equal(hasReportedMetrics('postgresql', [], names), true);
    assert.equal(hasReportedMetrics('redis', [], names), false);
  }
  assert.equal(hasReportedMetrics('kubernetes', [], new Set(['kubernetes_state.node.count'])), true);
});

test('generic application metrics and similar product names do not provide evidence', () => {
  assert.equal(hasReportedMetrics('nginx', [{ name: 'http.server.duration' }], new Set(['http.server.duration'])), false);
  assert.equal(hasReportedMetrics('redis', [{ name: 'redis.hits' }], new Set(['redisearch.hits'])), false);
});

test('histogram statistic names match their declared metric', () => {
  assert.equal(hasReportedMetrics('custom', [{ name: 'custom.duration' }], new Set(['custom.duration.p95'])), true);
});
