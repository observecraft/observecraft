// The Kubernetes hints of Agent checks (web/src/api/integrations/kubernetesHints.ts): valid Autodiscovery annotations
// (anywhere in the shown manifests or chart values) or Cluster Agent cluster checks, running the tile's check.
import assert from 'node:assert/strict';
import { createRequire } from 'node:module';
import test from 'node:test';
import { kubernetesHint } from '../web/src/api/integrations/kubernetesHints.ts';

const yaml = createRequire(new URL('../web/package.json', import.meta.url))('js-yaml');

/** Every check configuration in a variant's YAML: `ad.datadoghq.com/<container>.checks` annotations and `<check>.yaml` confd entries. */
function checkConfigs(code) {
  const found = [];
  const walk = (node) => {
    if (!node || typeof node !== 'object') return;
    for (const [key, value] of Object.entries(node)) {
      const annotation = key.match(/^ad\.datadoghq\.com\/([a-z0-9-]+)\.checks$/);
      if (annotation) for (const [check, config] of Object.entries(JSON.parse(value))) found.push({ check, config, container: annotation[1] });
      else if (/^[a-z_]+\.yaml$/.test(key) && typeof value === 'string') found.push({ check: key.slice(0, -5), config: yaml.load(value), cluster: true });
      else walk(value);
    }
  };
  for (const doc of yaml.loadAll(code)) walk(doc);
  return found;
}

/** The configurations of `check` in every variant of its hint (each variant has at least one). */
function configsOf(check) {
  const hint = kubernetesHint(check);
  assert.ok(hint && hint.variants.length > 0, check);
  return hint.variants.map((v) => {
    assert.ok(v.label && v.text && v.filename, `${check}: ${v.label}`);
    const configs = checkConfigs(v.code).filter((c) => c.check === check);
    assert.ok(configs.length > 0, `${check}: ${v.label} runs the check`);
    for (const c of configs) assert.ok(c.config.instances?.length >= 1, `${check}: ${v.label} has an instance`);
    return { variant: v, configs };
  });
}

for (const [check, endpoint] of [
  ['gitlab', 'openmetrics_endpoint'],
  ['gitlab_runner', 'prometheus_endpoint'],
]) {
  test(`${check}: every variant is a pod annotation running the check`, () => {
    for (const { variant, configs } of configsOf(check)) {
      assert.equal(variant.filename, 'Pod annotations');
      assert.match(configs[0].config.instances[0][endpoint], /^http:\/\/%%host%%:?\d*\//, variant.label);
    }
  });
}

for (const check of ['kafka', 'confluent_platform']) {
  test(`${check}: JMX checks on the pods' JMX port, for the Agent's JMX image`, () => {
    for (const { variant, configs } of configsOf(check)) {
      for (const { config } of configs) {
        assert.equal(config.init_config?.is_jmx, true, variant.label);
        const [instance] = config.instances;
        assert.equal(instance.host, '%%host%%', variant.label);
        assert.match(String(instance.port), /^\d{4,5}$/, variant.label);
      }
      assert.match(variant.text, /jmx/i, `${variant.label} says the Agent needs JMX`);
    }
  });
}

test('kafka: Strimzi, the Bitnami chart and KRaft with the apache/kafka image', () => {
  const variants = configsOf('kafka').map((x) => x.variant);
  const labels = variants.map((v) => v.label).join('\n');
  assert.match(labels, /Strimzi/);
  assert.match(labels, /Bitnami/);
  assert.match(labels, /apache\/kafka/);
  const strimzi = variants.find((v) => /Strimzi/.test(v.label));
  assert.match(strimzi.code, /jmxOptions: \{\}/);
  assert.match(strimzi.code, /kind: KafkaNodePool/);
  const bitnami = variants.find((v) => /Bitnami/.test(v.label));
  assert.match(bitnami.code, /JMX_PORT/);
  assert.match(bitnami.code, /podAnnotations/);
  assert.match(variants.find((v) => /apache\/kafka/.test(v.label)).code, /KAFKA_JMX_PORT/);
});

test('kafka_consumer: one cluster check per Kafka cluster, against its bootstrap servers', () => {
  for (const { variant, configs } of configsOf('kafka_consumer')) {
    for (const { config, cluster } of configs) {
      assert.ok(cluster, `${variant.label}: a cluster check, not one per broker pod`);
      assert.equal(config.cluster_check, true, variant.label);
      assert.match(config.instances[0].kafka_connect_str, /^[a-z0-9.-]+:\d+$/, variant.label);
    }
  }
});

test('strimzi: the cluster, topic and user operators’ metrics endpoints', () => {
  const endpoints = configsOf('strimzi').flatMap((x) => x.configs.flatMap((c) => c.config.instances.flatMap((i) => Object.entries(i))));
  const keys = new Set(endpoints.map(([k]) => k));
  for (const key of ['cluster_operator_endpoint', 'topic_operator_endpoint', 'user_operator_endpoint']) assert.ok(keys.has(key), key);
  for (const [, url] of endpoints) assert.match(url, /^http:\/\/%%host%%:\d+\/metrics$/);
});

test('checks without a hint have none', () => assert.equal(kubernetesHint('mysql'), undefined));

/** The check's one instance in a variant: from its pod annotations (with a logs annotation naming its source), or from
 *  the cluster check the chart's values run on the cluster checks runners. */
function instanceOf(check, v) {
  const configs = checkConfigs(v.code).filter((c) => c.check === check);
  assert.equal(configs.length, 1, v.label);
  const [{ config, cluster }] = configs;
  if (cluster) {
    assert.equal(v.filename, 'values.yaml (Datadog chart)', v.label);
    assert.equal(yaml.load(v.code).clusterChecksRunner?.enabled, true, `${v.label}: runs on the cluster checks runners`);
    assert.equal(config.cluster_check, true, v.label);
  } else {
    assert.equal(v.filename, 'Pod annotations', v.label);
    const annotations = yaml.load(v.code);
    const logs = Object.keys(annotations).find((k) => k.endsWith('.logs'));
    if (logs) assert.ok(JSON.parse(annotations[logs])[0].source, v.label);
  }
  assert.equal(config.instances.length, 1, v.label);
  return config.instances[0];
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
