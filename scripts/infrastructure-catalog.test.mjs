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
const model = await import('../web/src/pages/infrastructure/catalogModel.ts');
const mock = await import('../web/src/api/mock/catalog.ts');

const now = Date.now();
const range = { from: now - 15 * 60e3, to: now };
const search = (query, extra = {}) => mock.searchCatalog({ range, query, offset: 0, limit: 500, ...extra });

test('a resource links to its own page: the host panel or the Kubernetes Explorer panel', () => {
  assert.equal(model.catalogLink({ resourceType: 'host', id: 'host:i-0a1b2c3d4e5f60001', name: 'i-0a1b2c3d4e5f60001', kubeKind: null }), '/infrastructure?host=i-0a1b2c3d4e5f60001');
  assert.equal(model.catalogLink({ resourceType: 'kube_pod', id: 'prod-eu/pod 1', name: 'pod 1', kubeKind: 'pods' }), '/orchestration/explorer?resource=pods&panel=prod-eu%2Fpod+1');
  assert.equal(
    model.catalogLink({ resourceType: 'ecs_task', id: 'arn:aws:ecs:task/1', name: '1', kubeKind: 'ecs_tasks' }),
    '/orchestration/explorer?resource=ecs_tasks&panel=arn%3Aaws%3Aecs%3Atask%2F1',
  );
  assert.equal(model.catalogLink({ resourceType: 'container', id: 'uid/c', name: 'c', kubeKind: 'containers' }), '/orchestration/explorer?resource=containers&panel=uid%2Fc');
  assert.equal(model.catalogLink({ resourceType: 'kube_node', id: 'x', name: 'x', kubeKind: null }), null);
});

test('every resource type has a label and facets have names', () => {
  for (const t of ['host', 'kube_cluster', 'kube_node', 'kube_deployment', 'kube_pod', 'container', 'ecs_task']) assert.ok(model.TYPE_LABELS[t], t);
  assert.equal(model.facetLabel('resource_type'), 'Resource Type');
  assert.equal(model.facetLabel('some_tag'), 'some_tag');
});

test('mock catalog lists hosts and Kubernetes resources with facet counts', async () => {
  const page = await search('');
  const typeFacet = page.facets.find((f) => f.key === 'resource_type');
  const types = new Set(typeFacet.values.map((v) => v.value));
  for (const t of ['host', 'kube_cluster', 'kube_node', 'kube_deployment', 'kube_pod']) assert.ok(types.has(t), `has ${t}`);
  assert.ok(!types.has('kube_replicaset'));
  assert.equal(
    typeFacet.values.reduce((n, v) => n + v.count, 0),
    page.total,
  );
  // Demo data only: AWS account 123456789012 and example hosts.
  for (const r of mock.catalogResources().filter((x) => x.account)) assert.equal(r.account, '123456789012');
});

test('mock catalog filters by type and tags, facets ignore their own filter, and pages', async () => {
  const deployments = await search('resource_type:kube_deployment');
  assert.ok(deployments.total > 0);
  assert.ok(deployments.items.every((r) => r.resourceType === 'kube_deployment'));
  assert.ok(deployments.facets.find((f) => f.key === 'resource_type').values.length > 1);
  const prod = await search('resource_type:kube_deployment env:prod');
  assert.ok(prod.total > 0 && prod.total < deployments.total);
  assert.ok(prod.items.every((r) => r.env === 'prod'));
  const named = await search('resource_type:kube_deployment order');
  assert.ok(named.items.every((r) => r.name.includes('order')));
  const paged = await search('', { limit: 5, offset: 5, sort: 'name' });
  assert.equal(paged.items.length, 5);
  const sorted = [...paged.items].map((r) => r.name.toLowerCase());
  assert.deepEqual(sorted, [...sorted].sort());
});

test('mock catalog groups by resource category with regions, envs, teams and services', async () => {
  const page = await search('', { groupBy: 'resource_category' });
  const keys = page.groups.map((g) => g.key);
  assert.deepEqual([...keys].sort(), ['compute', 'containers', 'kubernetes']);
  assert.equal(
    page.groups.reduce((n, g) => n + g.count, 0),
    page.total,
  );
  const counts = page.groups.map((g) => g.count);
  assert.deepEqual(
    counts,
    [...counts].sort((a, b) => b - a),
  );
  const compute = page.groups.find((g) => g.key === 'compute');
  assert.ok(compute.regions.length > 0 && compute.envs.length > 0 && compute.teams.length > 0);
  assert.ok((await search('resource_category:containers')).items.every((r) => r.resourceCategory === 'containers'));
  assert.equal((await search('')).groups, null);
});

test('categories have labels', () => {
  for (const c of ['compute', 'containers', 'kubernetes', 'ecs']) assert.ok(model.CATEGORY_LABELS[c], c);
});
