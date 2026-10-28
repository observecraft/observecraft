import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import { registerHooks } from 'node:module';

// The web sources use extensionless relative imports (bundler resolution); resolve them to `.ts` here.
// They import JSON without import attributes, as the bundler allows: load it as a module.
registerHooks({
  resolve(specifier, context, next) {
    try {
      return next(specifier, context);
    } catch (err) {
      if (specifier.startsWith('.')) return next(`${specifier}.ts`, context);
      throw err;
    }
  },
  load(url, context, next) {
    if (url.endsWith('.json')) return { format: 'module', source: `export default ${readFileSync(new URL(url), 'utf8')};`, shortCircuit: true };
    return next(url, context);
  },
});
const mock = await import('../web/src/api/mock/logRestrictions.ts');
const identity = await import('../web/src/api/mock/identity.ts');

const DEMO_SERVICES = ['api-gateway', 'storefront-api', 'order-service', 'billing', 'catalog-service', 'inventory-service'];
const org = identity.MOCK_ORG.id;

test('the demo seed is deterministic, sorted and uses demo roles, teams and services', async () => {
  const queries = await mock.listRestrictionQueries();
  assert.ok(queries.length >= 3);
  assert.deepEqual(
    queries.map((q) => q.name),
    [...queries.map((q) => q.name)].sort((a, b) => a.localeCompare(b)),
  );
  const roles = new Set((await identity.listRoles()).map((r) => r.id));
  const teams = new Set((await identity.searchTeams({ offset: 0, limit: 100 })).teams.map((t) => t.handle));
  for (const q of queries) {
    for (const r of q.roles) assert.ok(roles.has(r.id) && r.name, `${r.id} is a mock role`);
    for (const t of q.teams) assert.ok(teams.has(t.handle) && t.name, `${t.handle} is a mock team`);
    assert.ok(!q.roles.some((r) => r.id === `${org}-admin`), 'the admin role is never restricted');
    for (const m of q.query.matchAll(/service:\(?([\w-]+(?: OR [\w-]+)*)\)?/g)) for (const s of m[1].split(' OR ')) assert.ok(DEMO_SERVICES.includes(s), `${s} is a demo service`);
  }
  assert.deepEqual(await mock.listRestrictionQueries(), queries, 'same answer twice');
});

test('inputs are validated like the server does', async () => {
  const ok = { name: 'Test restriction', description: '', query: 'service:billing', roleIds: [], teamHandles: [] };
  const bad = [
    { ...ok, name: ' ' },
    { ...ok, name: 'x'.repeat(101) },
    { ...ok, query: '' },
    { ...ok, query: 'index:main' },
    { ...ok, query: 'service:(billing' },
    { ...ok, query: '"unterminated' },
    { ...ok, roleIds: [`${org}-admin`] },
    { ...ok, roleIds: ['no-such-role'] },
    { ...ok, teamHandles: ['no-such-team'] },
  ];
  for (const input of bad) await assert.rejects(mock.createRestrictionQuery(input), undefined, JSON.stringify(input));
  const seeded = (await mock.listRestrictionQueries())[0];
  await assert.rejects(mock.createRestrictionQuery({ ...ok, name: seeded.name }), /already exists/);
});

test('create, assign, unassign, update and delete', async () => {
  const created = await mock.createRestrictionQuery({
    name: ' Inventory ',
    description: ' Stock levels ',
    query: ' service:inventory-service ',
    roleIds: [`${org}-standard`, `${org}-standard`],
    teamHandles: ['catalog'],
  });
  assert.equal(created.name, 'Inventory');
  assert.equal(created.query, 'service:inventory-service');
  assert.deepEqual(
    created.roles.map((r) => r.id),
    [`${org}-standard`],
  );
  assert.deepEqual(
    created.teams.map((t) => t.handle),
    ['catalog'],
  );
  const assigned = await mock.assignTeam(created.id, 'search');
  assert.deepEqual(assigned.teams.map((t) => t.handle).sort(), ['catalog', 'search']);
  await assert.rejects(mock.assignRole(created.id, `${org}-admin`), /admin/);
  const unassigned = await mock.unassignRole(created.id, `${org}-standard`);
  assert.equal(unassigned.roles.length, 0);
  const updated = await mock.updateRestrictionQuery(created.id, { name: 'Inventory', description: '', query: 'service:inventory-service env:prod', roleIds: [], teamHandles: [] });
  assert.equal(updated.query, 'service:inventory-service env:prod');
  assert.equal(updated.teams.length, 0);
  await mock.deleteRestrictionQuery(created.id);
  assert.ok(!(await mock.listRestrictionQueries()).some((q) => q.id === created.id));
  await assert.rejects(mock.deleteRestrictionQuery(created.id), /not found/);
});

test('previews count the matching logs of the last hour', async () => {
  const billing = await mock.previewRestrictionQuery('service:billing');
  const all = await mock.previewRestrictionQuery('-service:no-such-service');
  assert.ok(billing.count > 0, 'the demo has billing logs');
  assert.ok(all.count > billing.count);
  assert.equal((await mock.previewRestrictionQuery('service:no-such-service')).count, 0);
  await assert.rejects(mock.previewRestrictionQuery('index:main'), /index/);
});

test('the mock viewer is an admin, so their view is unrestricted', async () => {
  assert.equal(await mock.mockViewRestricted(), false);
});
