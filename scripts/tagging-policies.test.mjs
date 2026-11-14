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
const logic = await import('../web/src/api/tagging/logic.ts');
const mock = await import('../web/src/api/mock/tagging.ts');
const snippets = await import('../web/src/pages/infrastructure/taggingSnippets.ts');

const res = (target, name, tags, extra = {}) => ({
  id: `id-${name}`,
  target,
  name,
  resourceType: target === 'cloud' ? 'AwsS3Bucket' : 'kube_deployment',
  provider: target === 'cloud' ? 'aws' : null,
  account: null,
  region: null,
  cluster: null,
  namespace: null,
  env: null,
  team: tags.find((t) => t.startsWith('team:'))?.slice(5) ?? null,
  tags,
  provenance: null,
  ...extra,
});
const rule = (target, required, scope = {}) => ({ target, scope: { provider: '', account: '', cluster: '', resourceType: '', names: '', ...scope }, required });
const req = (key, allowedValues = []) => ({ key, allowedValues });

test('required tags accept any alternative and allowed values ignore case (as tagging_rules.rs)', () => {
  const r = rule('cloud', [req('managed-by'), req('tf_repo|git_repo'), req('env', ['prod', 'staging'])]);
  assert.deepEqual(logic.gaps(r, res('cloud', 'a', ['Managed_By:terraform', 'git_repo:infra-terraform', 'env:PROD'])), []);
  assert.deepEqual(logic.gaps(r, res('cloud', 'b', ['managed-by:terraform', 'env:production'])), [
    { key: 'tf_repo|git_repo', value: null },
    { key: 'env', value: 'production' },
  ]);
  assert.equal(logic.describeGap({ key: 'env', value: 'production' }), 'env=production not allowed');
  assert.equal(logic.describeGap({ key: 'team', value: null }), 'team missing');
});

test('scopes match comma-separated patterns and leave out resources lacking the field', () => {
  const a = res('cloud', 'example-shop-assets', [], { account: '123456789012' });
  assert.ok(logic.applies(rule('cloud', [req('team')], { account: '999, 1234*' }), a));
  assert.ok(!logic.applies(rule('cloud', [req('team')], { cluster: 'shop-*' }), a));
  assert.ok(!logic.applies(rule('service', [req('team')]), a));
  assert.ok(logic.applies(rule('cloud', [req('team')], { names: 'example-shop-*', resourceType: 'aws*bucket' }), a));
});

test('coverage counts resources per policy, account, cluster and team, worst first', () => {
  const policies = [
    { id: 'p1', name: 'Owners', rule: rule('cloud', [req('team')]) },
    { id: 'p2', name: 'Argo', rule: rule('kubernetes', [req('argocd.argoproj.io/tracking-id')]) },
  ];
  const resources = [
    res('cloud', 'a', ['team:storefront'], { account: '111' }),
    res('cloud', 'b', [], { account: '222' }),
    res('kubernetes', 'c', [], { cluster: 'shop-prod-eu', team: 'checkout' }),
    res('image', 'd', []),
  ];
  const c = logic.coverage(policies, resources, 10);
  assert.deepEqual(c.total, { evaluated: 3, compliant: 1, score: 33.3 });
  assert.deepEqual(
    c.byPolicy.map((r) => [r.key, r.evaluated, r.compliant]),
    [
      ['p1', 2, 1],
      ['p2', 1, 0],
    ],
  );
  assert.equal(c.byAccount[0].key, '222');
  assert.deepEqual(
    c.byTeam.map((r) => r.key),
    ['(no team)', 'checkout', 'storefront'],
  );
  assert.equal(c.noncompliantTotal, 2);
  assert.deepEqual(c.noncompliant[0].policies[0].gaps, [{ key: 'team', value: null }]);
});

test('the mock lists provenance per convention and keeps its numbers consistent', async () => {
  const list = await mock.provenance(undefined, false);
  const via = new Set(list.resources.map((r) => r.provenance?.via).filter(Boolean));
  for (const v of ['tag', 'argocd', 'oci', 'git_metadata']) assert.ok(via.has(v), `no ${v} provenance in the mock`);
  assert.equal(list.summary.total, list.resources.length);
  const missing = await mock.provenance('cloud', true);
  assert.ok(missing.resources.length > 0 && missing.resources.every((r) => r.target === 'cloud' && !r.provenance));
  const cov = await mock.coverage();
  assert.equal(
    cov.total.evaluated,
    cov.byTarget.reduce((n, r) => n + r.evaluated, 0),
  );
  assert.ok(cov.history.length >= 30);
  // Every resource name is fictional demo data.
  for (const r of list.resources) assert.doesNotMatch(r.id, /lumenova|datadoghq/i);
});

test('mock policies can be created, edited and deleted', async () => {
  const before = (await mock.listPolicies()).policies.length;
  const p = await mock.createPolicy({
    name: 'Buckets name a cost center',
    description: '',
    target: 'cloud',
    scope: { provider: 'aws', account: '', cluster: '', resourceType: 'AwsS3Bucket', names: '' },
    required: [req('cost-center')],
    enabled: true,
  });
  assert.equal((await mock.listPolicies()).policies.length, before + 1);
  await assert.rejects(mock.createPolicy({ name: 'Buckets name a cost center', target: 'cloud', required: [req('x')] }), /already exists/);
  const u = await mock.updatePolicy(p.id, { name: p.name, target: 'cloud', scope: p.scope, required: p.required, enabled: false });
  assert.equal(u.enabled, false);
  await mock.deletePolicy(p.id);
  assert.equal((await mock.listPolicies()).policies.length, before);
});

test('each convention has a snippet', () => {
  const keys = snippets.SNIPPETS.map((s) => s.key);
  for (const k of ['terraform-aws', 'terraform-azure', 'yor-gitlab', 'docker-oci', 'unified-service-tagging']) assert.ok(keys.includes(k), k);
  assert.match(snippets.SNIPPETS.find((s) => s.key === 'terraform-aws').code, /default_tags/);
  assert.match(snippets.SNIPPETS.find((s) => s.key === 'docker-oci').code, /org\.opencontainers\.image\.source/);
  assert.match(snippets.SNIPPETS.find((s) => s.key === 'unified-service-tagging').code, /DD_GIT_REPOSITORY_URL/);
  assert.deepEqual(
    snippets.snippetsFor('image').map((s) => s.key),
    ['docker-oci'],
  );
});

test('a resource’s scope reads as its account’s name or its cluster and namespace, the raw id in the title', async () => {
  const { resourceScope } = await import('../web/src/components/provenance/scope.ts');
  const names = new Map([['azure:0b1c2d3e-4f50-4a6b-8c7d-9e0f1a2b3c4d', 'Storefront Production']]);
  const az = resourceScope({ target: 'cloud', provider: 'azure', account: '0b1c2d3e-4f50-4a6b-8c7d-9e0f1a2b3c4d', region: 'westeurope', cluster: null, namespace: null }, names);
  assert.deepEqual(az, { logo: 'azure', text: 'Storefront Production', title: 'Azure subscription 0b1c2d3e-4f50-4a6b-8c7d-9e0f1a2b3c4d · westeurope' });
  const aws = resourceScope({ target: 'cloud', provider: 'aws', account: '123456789012', region: 'eu-west-1', cluster: null, namespace: null }, names);
  assert.deepEqual(aws, { logo: 'aws', text: '123456789012', title: 'AWS account 123456789012 · eu-west-1' });
  assert.equal(resourceScope({ target: 'cloud', provider: 'gcp', account: 'example-storefront', region: null, cluster: null, namespace: null }, names).logo, 'googlecloud');
  const k = resourceScope({ target: 'kubernetes', provider: null, account: null, region: 'eu-west-1', cluster: 'shop-prod-eu', namespace: 'checkout' }, names);
  assert.deepEqual(k, { logo: 'kubernetes', text: 'shop-prod-eu · checkout', title: 'Kubernetes cluster shop-prod-eu · namespace checkout' });
  assert.deepEqual(resourceScope({ target: 'service', provider: null, account: null, region: null, cluster: null, namespace: null }, names), { logo: null, text: '—', title: '' });
});
