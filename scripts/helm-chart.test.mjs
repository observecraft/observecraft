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
