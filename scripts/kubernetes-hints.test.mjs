// The Kubernetes hints of Agent checks (web/src/api/integrations/kubernetesHints.ts): valid Autodiscovery annotations.
import assert from 'node:assert/strict';
import { createRequire } from 'node:module';
import test from 'node:test';
import { kubernetesHint } from '../web/src/api/integrations/kubernetesHints.ts';

const yaml = createRequire(new URL('../web/package.json', import.meta.url))('js-yaml');

for (const [check, endpoint] of [
  ['gitlab', 'openmetrics_endpoint'],
  ['gitlab_runner', 'prometheus_endpoint'],
]) {
  test(`${check}: every variant is a pod annotation running the check`, () => {
    const hint = kubernetesHint(check);
    assert.ok(hint && hint.variants.length > 0, check);
    for (const v of hint.variants) {
      const annotations = yaml.load(v.annotations);
      const [key, value] = Object.entries(annotations)[0];
      assert.match(key, /^ad\.datadoghq\.com\/[a-z0-9-]+\.checks$/, v.label);
      const config = JSON.parse(value)[check];
      assert.ok(config && config.instances.length === 1, v.label);
      assert.match(config.instances[0][endpoint], /^http:\/\/%%host%%:?\d*\//, v.label);
    }
  });
}

test('checks without a hint have none', () => assert.equal(kubernetesHint('mysql'), undefined));

/** The check's instance in a variant: from its pod annotations, or from the cluster check in the chart's values. */
function instanceOf(check, v) {
  if (v.annotations) {
    const annotations = yaml.load(v.annotations);
    const key = Object.keys(annotations).find((k) => k.endsWith('.checks'));
    assert.match(key, /^ad\.datadoghq\.com\/[a-z0-9-]+\.checks$/, v.label);
    const logs = Object.keys(annotations).find((k) => k.endsWith('.logs'));
    if (logs) assert.ok(JSON.parse(annotations[logs])[0].source, v.label);
    const config = JSON.parse(annotations[key])[check];
    assert.ok(config && config.instances.length === 1, v.label);
    return config.instances[0];
  }
  const values = yaml.load(v.values);
  assert.equal(values.clusterChecksRunner?.enabled, true, `${v.label}: runs on the cluster checks runners`);
  const confd = yaml.load(values.clusterAgent.confd[`${check}.yaml`]);
  assert.equal(confd.cluster_check, true, v.label);
  assert.equal(confd.instances.length, 1, v.label);
  return confd.instances[0];
}

test('PostgreSQL: Bitnami, CloudNativePG, Azure flexible server (managed identity) and RDS (IAM), never DBM', () => {
  const hint = kubernetesHint('postgres');
  const labels = hint.variants.map((v) => v.label).join(' | ');
  for (const name of ['Bitnami', 'CloudNativePG', 'Azure Database for PostgreSQL', 'RDS']) assert.match(labels, new RegExp(name));
  for (const v of hint.variants) {
    const i = instanceOf('postgres', v);
    assert.ok(i.host && i.port === 5432 && i.username, v.label);
    assert.notEqual(i.dbm, true, v.label);
  }
  const azure = instanceOf(
    'postgres',
    hint.variants.find((v) => /Azure/.test(v.label)),
  );
  assert.equal(azure.ssl, 'require');
  assert.deepEqual(azure.azure.managed_authentication.enabled, true);
  assert.match(azure.azure.managed_authentication.auth_type, /^(managed_identity|workload_identity)$/);
  assert.equal(azure.azure.deployment_type, 'flexible_server');
  const rds = instanceOf(
    'postgres',
    hint.variants.find((v) => /RDS/.test(v.label)),
  );
  assert.equal(rds.aws.managed_authentication.enabled, true);
  assert.ok(rds.aws.region);
});

test('Redis and Valkey run the redisdb check against their pods', () => {
  const redis = kubernetesHint('redisdb');
  assert.ok(redis.variants.some((v) => /Valkey/.test(v.label)));
  for (const v of redis.variants) assert.equal(instanceOf('redisdb', v).port, 6379, v.label);
});

test('ClickHouse runs its check against the HTTP interface of its pods', () => {
  for (const v of kubernetesHint('clickhouse').variants) assert.equal(instanceOf('clickhouse', v).port, 8123, v.label);
  assert.ok(kubernetesHint('clickhouse').variants.some((v) => /Altinity/.test(v.label)));
});

test('PgBouncer runs its check against the admin console of its pods', () => {
  for (const v of kubernetesHint('pgbouncer').variants) assert.match(instanceOf('pgbouncer', v).database_url, /^postgresql:\/\/datadog:.+@%%host%%:6432\/pgbouncer$/, v.label);
});
