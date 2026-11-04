// Renders the observecraft Helm chart (`helm template`, as Argo CD does: no cluster, so `lookup`
// finds nothing) and checks where the bundled Valkey's password comes from. Skipped without helm.
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import test from 'node:test';

const chart = fileURLToPath(new URL('../deploy/helm/observecraft', import.meta.url));
const hasHelm = spawnSync('helm', ['version', '--short']).status === 0;

/** `helm template` with `--set` values; returns { ok, out, err }. */
function render(set = [], only = []) {
  const args = ['template', 'oc', chart, ...set.flatMap((s) => ['--set', s]), ...only.flatMap((t) => ['--show-only', `templates/${t}.yaml`])];
  const r = spawnSync('helm', args, { encoding: 'utf8' });
  return { ok: r.status === 0, out: r.stdout?.replaceAll('\r\n', '\n'), err: r.stderr };
}

/** The rendered document of `kind` named `name`. */
function doc(out, kind, name) {
  const found = out.split(/^---$/m).find((d) => new RegExp(`^kind: ${kind}$`, 'm').test(d) && new RegExp(`^  name: ${name}$`, 'm').test(d));
  assert.ok(found, `${kind} ${name} is rendered`);
  return found;
}

const valkey = (out) => doc(out, 'Deployment', 'oc-observecraft-valkey');
const server = (out) => doc(out, 'Deployment', 'oc-observecraft-server');

test('ingestion profile waits for shards and buffers HTTP errors until query completion', { skip: !hasHelm }, () => {
  const r = render();
  assert.ok(r.ok, r.err);
  assert.match(r.out, /<insert_distributed_sync>1<\/insert_distributed_sync>/);
  assert.match(r.out, /<http_wait_end_of_query>1<\/http_wait_end_of_query>/);
  assert.match(r.out, /<wait_for_async_insert>1<\/wait_for_async_insert>/);
});

test('bundled default: a generated cache-password in the chart secret', { skip: !hasHelm }, () => {
  const r = render();
  assert.ok(r.ok, r.err);
  assert.match(doc(r.out, 'Secret', 'oc-observecraft-secrets'), /^  cache-password: \S+$/m);
  assert.match(valkey(r.out), /name: REDISCLI_AUTH\n\s+valueFrom: \{ secretKeyRef: \{ name: oc-observecraft-secrets, key: cache-password \} \}/);
  assert.match(valkey(r.out), /requirepass/);
  assert.match(server(r.out), /name: OC_CACHE_PASSWORD\n\s+valueFrom: \{ secretKeyRef: \{ name: oc-observecraft-secrets, key: cache-password \} \}/);
  assert.match(server(r.out), /redis:\/\/:\$\(OC_CACHE_PASSWORD\)@oc-observecraft-valkey:6379\/0/);
});

test('existingSecret without a cache password source fails at render time', { skip: !hasHelm }, () => {
  const r = render(['secrets.existingSecret=my-secrets']);
  assert.ok(!r.ok, 'render must fail');
  assert.match(r.err, /cache-password/);
  assert.match(r.err, /cache\.auth\.existingSecret/);
  assert.match(r.err, /cache\.auth\.enabled=false/);
});

test('existingSecret and cache.auth.existingSecret: the password from that secret and key', { skip: !hasHelm }, () => {
  for (const [secret, key, expectedKey] of [
    ['my-secrets', undefined, 'cache-password'],
    ['valkey-auth', 'password', 'password'],
  ]) {
    const r = render(['secrets.existingSecret=my-secrets', `cache.auth.existingSecret=${secret}`, ...(key ? [`cache.auth.key=${key}`] : [])]);
    assert.ok(r.ok, r.err);
    const ref = new RegExp(`valueFrom: \\{ secretKeyRef: \\{ name: ${secret}, key: ${expectedKey} \\} \\}`);
    assert.match(valkey(r.out), ref);
    assert.match(server(r.out), ref);
    assert.doesNotMatch(r.out, /^kind: Secret$/m, 'no chart secret with existingSecret');
  }
});

test('cache.auth.existingSecret without secrets.existingSecret: the chart secret holds no cache-password', { skip: !hasHelm }, () => {
  const r = render(['cache.auth.existingSecret=valkey-auth']);
  assert.ok(r.ok, r.err);
  assert.doesNotMatch(doc(r.out, 'Secret', 'oc-observecraft-secrets'), /cache-password/);
  assert.match(valkey(r.out), /secretKeyRef: \{ name: valkey-auth, key: cache-password \}/);
});

test('cache.auth.enabled=false: Valkey without requirepass, the server without a password', { skip: !hasHelm }, () => {
  for (const set of [['cache.auth.enabled=false'], ['secrets.existingSecret=my-secrets', 'cache.auth.enabled=false']]) {
    const r = render(set);
    assert.ok(r.ok, r.err);
    assert.doesNotMatch(valkey(r.out), /requirepass|REDISCLI_AUTH|cache-password/);
    assert.doesNotMatch(server(r.out), /OC_CACHE_PASSWORD|cache-password/);
    assert.match(server(r.out), /name: OC_CACHE_URL\n\s+value: "redis:\/\/oc-observecraft-valkey:6379\/0"/);
    assert.doesNotMatch(r.out, /cache-password/);
    // Only the server's pods reach Valkey.
    const policy = doc(r.out, 'NetworkPolicy', 'oc-observecraft-valkey');
    assert.match(policy, /ports: \[\{ port: 6379 \}\]\n\s+from:\n\s+- podSelector:\n\s+matchLabels:\n(\s+app\.kubernetes\.io\/\S+: \S+\n)*?\s+app\.kubernetes\.io\/component: server\n/);
    assert.equal(policy.match(/podSelector:/g).length, 2, 'the policy selects Valkey and admits one selector');
  }
});

test('cache.auth is validated by the schema', { skip: !hasHelm }, () => {
  assert.ok(!render(['cache.auth.enabled=nope']).ok);
  assert.ok(!render(['secrets.existingSecret=my-secrets', 'cache.auth.existingSecret=my-secrets', 'cache.auth.key=']).ok);
  assert.ok(!render(['cache.auth.unknown=x']).ok);
});

test('external cache: the URL from the secret, no cache.auth needed', { skip: !hasHelm }, () => {
  const r = render(['secrets.existingSecret=my-secrets', 'cache.mode=external']);
  assert.ok(r.ok, r.err);
  assert.match(server(r.out), /name: OC_CACHE_URL\n\s+valueFrom: \{ secretKeyRef: \{ name: my-secrets, key: cache-url \} \}/);
  assert.doesNotMatch(r.out, /oc-observecraft-valkey/);
});

test('server.podLabels label the server pods (e.g. AKS Workload Identity), next to the selector labels', { skip: !hasHelm }, () => {
  const r = render(['server.podLabels.team=observability']);
  assert.ok(r.ok, r.err);
  const template = server(r.out).split('  template:')[1];
  assert.match(template, /labels:\n(?:\s+\S+: \S+\n)*\s+team: observability\n/);
  assert.match(server(r.out), /matchLabels:\n(?:\s+\S+: \S+\n)+/);
  assert.doesNotMatch(server(r.out).split('  template:')[0], /team: observability/, 'not in the selector, which is immutable');
});

const collector = (out) => doc(out, 'StatefulSet', 'oc-observecraft-collector');
const collectorConfig = (out) => doc(out, 'ConfigMap', 'oc-observecraft-pipeline');

test('collector: the Go heap is held under the memory limiter limit (GOMEMLIMIT), so a backlog slows it instead of OOMKilling it', { skip: !hasHelm }, () => {
  const r = render(['collector.memoryLimitMiB=3072']);
  assert.ok(r.ok, r.err);
  assert.match(collector(r.out), /name: OC_COLLECTOR_MEMORY_LIMIT_MIB, value: "3072"/);
  assert.match(collector(r.out), /name: GOMEMLIMIT, value: "3072MiB"/);
});

test('collector: Kafka receivers buffer one small fetch per partition and keep 64 records in flight', { skip: !hasHelm }, () => {
  const r = render();
  assert.ok(r.ok, r.err);
  const config = collectorConfig(r.out);
  for (const receiver of ['kafka/otlp', 'kafka/otlp_json']) {
    const block = config.split(`  ${receiver}:\n`)[1].split(/\n  \S/)[0];
    assert.match(block, /max_partition_fetch_size: 262144/, receiver);
    assert.match(block, /partition_processing: \{ independent: true, max_buffered_batches: 1, max_in_flight: \{ records: 64 \} \}/, receiver);
  }
});

test('kafka topics job: an Argo CD Sync hook, recreated on every sync (a Job spec is immutable)', { skip: !hasHelm }, () => {
  const r = render();
  assert.ok(r.ok, r.err);
  const job = r.out.split(/^---$/m).find((d) => /^kind: Job$/m.test(d) && /kafka-topics/.test(d));
  assert.ok(job, 'topics job is rendered');
  assert.match(job, /argocd\.argoproj\.io\/hook: Sync/);
  assert.match(job, /argocd\.argoproj\.io\/hook-delete-policy: BeforeHookCreation/);
});

test('collector.autoscaling: a KEDA ScaledObject on the oc-collector consumer lag, and no fixed replica count', { skip: !hasHelm }, () => {
  const off = render();
  assert.ok(off.ok, off.err);
  assert.doesNotMatch(off.out, /^kind: ScaledObject$/m);
  assert.match(collector(off.out), /^  replicas: 2$/m);

  const r = render(['collector.autoscaling.enabled=true', 'collector.autoscaling.minReplicas=1', 'collector.autoscaling.maxReplicas=4', 'collector.autoscaling.lagThreshold=500']);
  assert.ok(r.ok, r.err);
  assert.doesNotMatch(collector(r.out), /^  replicas:/m, 'KEDA owns the replica count');
  const scaled = doc(r.out, 'ScaledObject', 'oc-observecraft-collector');
  assert.match(scaled, /scaleTargetRef:\n\s+apiVersion: apps\/v1\n\s+kind: StatefulSet\n\s+name: oc-observecraft-collector/);
  assert.match(scaled, /minReplicaCount: 1\n\s+maxReplicaCount: 4/);
  assert.match(scaled, /type: kafka/);
  assert.match(scaled, /bootstrapServers: oc-observecraft-kafka:9092/);
  assert.match(scaled, /consumerGroup: oc-collector\n/);
  assert.match(scaled, /lagThreshold: "500"/);
});

test('collector.autoscaling: never more replicas than Kafka partitions (extra consumers would sit idle)', { skip: !hasHelm }, () => {
  const r = render(['collector.autoscaling.enabled=true', 'collector.autoscaling.maxReplicas=20', 'kafka.partitions=12']);
  assert.ok(!r.ok, 'render must fail');
  assert.match(r.err, /maxReplicas.*kafka\.partitions/);
});
