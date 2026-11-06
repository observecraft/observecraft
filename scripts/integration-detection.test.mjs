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

test('AWS services are detected by the Datadog prefix of their CloudWatch namespace', async () => {
  const { awsMetricPrefix } = await import('../web/src/api/integrations/detection.ts');
  assert.equal(awsMetricPrefix('AWS/EC2'), 'aws.ec2.');
  assert.equal(awsMetricPrefix('AWS/ApplicationELB'), 'aws.applicationelb.');
  // As Datadog names them: aws.rds.proxy.*, ecs.containerinsights.*.
  assert.equal(awsMetricPrefix('AWS/RDS/Proxy'), 'aws.rds.proxy.');
  assert.equal(awsMetricPrefix('ECS/ContainerInsights'), 'ecs.containerinsights.');
  assert.equal(awsMetricPrefix('/aws/sagemaker/Endpoints'), 'aws.sagemaker_endpoints.');
  assert.equal(hasReportedMetrics('aws-ec2', [], new Set(['aws.ec2.cpuutilization']), [awsMetricPrefix('AWS/EC2')]), true);
  assert.equal(hasReportedMetrics('aws-ec2', [], new Set(['aws.ecs.cpuutilization']), [awsMetricPrefix('AWS/EC2')]), false);
});
