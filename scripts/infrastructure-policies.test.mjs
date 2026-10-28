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
const logic = await import('../web/src/api/policies/logic.ts');
const mock = await import('../web/src/api/mock/policies.ts');

const row = (resourceType, id, tags, attributes = {}) => ({
  id,
  resourceType,
  resourceCategory: 'kubernetes',
  name: id,
  cloud: null,
  account: null,
  region: null,
  cluster: 'shop-prod-eu',
  namespace: 'checkout',
  env: tags.find((t) => t.startsWith('env:'))?.slice(4) ?? null,
  service: null,
  team: tags.find((t) => t.startsWith('team:'))?.slice(5) ?? null,
  status: null,
  health: null,
  createdAt: null,
  lastSeen: 0,
  tags,
  attributes,
  kubeKind: null,
});
const c = (attribute, operator, values = []) => ({ attribute, operator, values });

test('versions compare numerically like the server', () => {
  assert.deepEqual(logic.parseVersion('v1.31.2'), [1, 31, 2]);
  assert.deepEqual(logic.parseVersion('1.29.3-eks-a1b2c3'), [1, 29, 3]);
  assert.equal(logic.parseVersion('latest'), null);
  assert.equal(logic.compareVersions('v1.9', '1.10'), -1);
  assert.equal(logic.compareVersions('1.30', 'v1.30.0'), 0);
  assert.equal(logic.compareVersions('24.04.1', '22.04'), 1);
  assert.equal(logic.compareVersions('2.4', 'nope'), null);
});

test('conditions check every value; negative ones pass on missing attributes', () => {
  const vals = ['2.4', 'latest'];
  assert.equal(logic.conditionFailure(c('image_tags', 'not_one_of', ['latest']), vals), 'image_tags is latest');
  assert.equal(logic.conditionFailure(c('image_tags', 'one_of', ['2.4', 'LATEST']), vals), null);
  assert.equal(logic.conditionFailure(c('image_tags', 'matches', ['^\\d+\\.\\d+$']), vals), 'image_tags is 2.4, latest');
  assert.equal(logic.conditionFailure(c('team', 'exists'), []), 'team is missing');
  assert.equal(logic.conditionFailure(c('team', 'not_exists'), []), null);
  assert.equal(logic.conditionFailure(c('replicas', 'gte', ['2']), ['1']), 'replicas is 1');
  assert.equal(logic.conditionFailure(c('replicas', 'lte', ['2']), ['2']), null);
  assert.equal(logic.conditionFailure(c('version', 'version_gte', ['1.31']), ['v1.30.6']), 'version is v1.30.6');
  assert.equal(logic.conditionFailure(c('version', 'version_lt', ['1.31']), ['v1.30.6']), null);
  assert.equal(logic.conditionFailure(c('cpu_limits', 'lte', ['4']), []), 'cpu_limits is missing');
});

test('targets are validated before they are saved or previewed', () => {
  const t = (conditions) => ({ resourceType: 'kube_deployment', scope: '', conditions });
  assert.equal(logic.validateTarget(t([c('team', 'exists')])), null);
  assert.match(logic.validateTarget(t([])), /at least one condition/);
  assert.match(logic.validateTarget(t([c('team', 'exists', ['x'])])), /takes no value/);
  assert.match(logic.validateTarget(t([c('replicas', 'gte', ['two'])])), /not a number/);
  assert.match(logic.validateTarget(t([c('version', 'version_gte', ['new'])])), /not a version/);
  assert.match(logic.validateTarget(t([c('name', 'matches', ['('])])), /invalid pattern/);
  assert.match(logic.validateTarget(t([c('bad key!', 'exists')])), /not a valid attribute/);
});

test('scope and page filters narrow what is scored', () => {
  const all = [
    row('kube_deployment', 'prod-1', ['env:prod'], { replicas: '1' }),
    row('kube_deployment', 'prod-3', ['env:prod', 'team:payments'], { replicas: '3' }),
    row('kube_deployment', 'staging-1', ['env:staging'], { replicas: '1' }),
    row('kube_node', 'node', ['env:prod']),
  ];
  const target = { resourceType: 'kube_deployment', scope: 'env:prod', conditions: [c('replicas', 'gte', ['2'])] };
  const { evaluation, violations } = logic.evaluate(target, all);
  assert.deepEqual(evaluation, { evaluated: 2, noncompliant: 1, score: 50 });
  assert.deepEqual(
    violations.map((v) => [v.resource.id, v.reason]),
    [['prod-1', 'replicas is 1']],
  );
  assert.deepEqual(logic.evaluate(target, all, [{ kind: 'tag', key: 'team', values: ['payments'], negate: false }]).evaluation, { evaluated: 1, noncompliant: 0, score: 100 });
  assert.equal(logic.score(3, 1), 66.7);
  assert.equal(logic.score(0, 0), null);
});

test('mock: templates preview on the demo catalog and demo policies have a history', async () => {
  const list = await mock.searchPolicies('');
  assert.equal(list.templates.length, 10);
  const latest = list.templates.find((t) => t.key === 'kube_no_latest_tag');
  assert.ok(latest.available);
  // notification-worker runs the latest tag in shop-staging.
  assert.equal(latest.preview.noncompliant, 1);
  assert.ok(list.templates.find((t) => t.key === 'kube_node_version').preview.noncompliant > 0, 'staging nodes run v1.30');
  assert.equal(list.templates.find((t) => t.key === 'ecs_service_tasks').available, false);
  assert.ok(list.facets.some((f) => f.key === 'team'));
  assert.equal(list.policies.length, 2);
  const detail = await mock.getPolicy(list.policies[0].id, '');
  assert.ok(detail.history.length > 20);
  assert.ok(detail.violations.every((v) => v.reason));
  // Page filters narrow the scores.
  const staging = await mock.searchPolicies('env:staging');
  const tagPolicy = staging.policies.find((p) => p.template === 'kube_no_latest_tag');
  assert.equal(tagPolicy.evaluation.noncompliant, 1);
  assert.ok(tagPolicy.evaluation.evaluated < list.policies.find((p) => p.template === 'kube_no_latest_tag').evaluation.evaluated);
});

test('mock: policies are created, previewed, updated and deleted with validation', async () => {
  const input = {
    name: 'Prod replicas',
    description: '',
    priority: 1,
    category: 'reliability',
    resourceType: 'kube_deployment',
    scope: 'env:prod',
    conditions: [c('replicas', 'gte', ['2'])],
    template: null,
  };
  const preview = await mock.previewPolicy(input);
  assert.ok(preview.evaluation.evaluated > 0);
  const created = await mock.createPolicy(input);
  assert.equal(created.evaluation.evaluated, preview.evaluation.evaluated);
  await assert.rejects(mock.createPolicy(input), /already exists/);
  await assert.rejects(mock.createPolicy({ ...input, name: 'x', conditions: [] }), /at least one condition/);
  const updated = await mock.updatePolicy(created.id, { ...input, priority: 0, conditions: [c('replicas', 'gte', ['3'])] });
  assert.equal(updated.priority, 0);
  assert.equal((await mock.getPolicy(created.id, '')).history.length, 1);
  const attrs = await mock.policyAttributes();
  const dep = attrs.find((a) => a.resourceType === 'kube_deployment');
  assert.ok(['replicas', 'image_tags', 'cpu_limits'].every((k) => dep.attributes.some((a) => a.key === k)));
  await mock.deletePolicy(created.id);
  await assert.rejects(mock.getPolicy(created.id, ''), /not found/);
});
