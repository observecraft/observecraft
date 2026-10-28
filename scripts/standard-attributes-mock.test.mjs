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
const mock = await import('../web/src/api/mock/standardAttributes.ts');
const logs = await import('../web/src/api/mock/index.ts');

const seed = JSON.parse(readFileSync(new URL('../server/seeds/standard_attributes.json', import.meta.url), 'utf8')).attributes;
const now = Date.now();
const lastHour = { from: now - 3_600_000, to: now };
const input = (path, type, aliases, extra = {}) => ({ path, type, description: '', aliases, preserveSource: true, ...extra });

test('the list is the server defaults, sorted by path', async () => {
  const list = await mock.resetStandardAttributes();
  assert.equal(list.length, seed.length);
  assert.deepEqual(
    list.map((a) => a.path),
    seed.map((a) => a.path).sort(),
  );
  assert.ok(list.every((a) => a.isDefault && !a.modified && a.description));
  const status = list.find((a) => a.path === 'http.status_code');
  assert.equal(status.type, 'integer');
  assert.ok(status.aliases.includes('status_code') && status.aliases.includes('http.response.status_code'));
  assert.deepEqual(await mock.listStandardAttributes(), list, 'deterministic');
});

test('adding, editing and validating like the server', async () => {
  await mock.resetStandardAttributes();
  const added = await mock.createStandardAttribute(input(' @checkout.cart_id ', 'string', [' cartId '], { description: ' Cart ' }));
  assert.deepEqual(
    { path: added.path, aliases: added.aliases, description: added.description, isDefault: added.isDefault },
    { path: 'checkout.cart_id', aliases: ['cartId'], description: 'Cart', isDefault: false },
  );
  await assert.rejects(mock.createStandardAttribute(input('checkout.cart_id', 'string', [])), /already a standard attribute/);
  await assert.rejects(mock.createStandardAttribute(input('message', 'string', [])), /reserved/);
  await assert.rejects(mock.createStandardAttribute(input('x', 'string', ['status_code'])), /already an alias of http.status_code/);
  await assert.rejects(mock.createStandardAttribute(input('x', 'string', ['http.url'])), /standard attribute itself/);
  await assert.rejects(mock.createStandardAttribute(input('logger.thread', 'string', ['logger'])), /contains the attribute/);
  await assert.rejects(mock.createStandardAttribute(input('x', 'string', ['y', 'y'])), /twice/);
  await assert.rejects(mock.createStandardAttribute(input('a..b', 'string', [])), /Attribute paths/);

  const edited = await mock.updateStandardAttribute('http.status_code', input('http.status_code', 'integer', ['code']));
  assert.deepEqual([edited.aliases, edited.isDefault, edited.modified], [['code'], true, true]);
  await assert.rejects(mock.updateStandardAttribute('http.status_code', input('http.code', 'integer', [])), /renamed/);
  await assert.rejects(mock.updateStandardAttribute('nope', input('nope', 'string', [])), /No standard attribute/);
});

test('removing hides a default until a reset; custom attributes go', async () => {
  await mock.resetStandardAttributes();
  await mock.createStandardAttribute(input('checkout.cart_id', 'string', []));
  await mock.deleteStandardAttribute('usr.email');
  await mock.deleteStandardAttribute('checkout.cart_id');
  const list = await mock.listStandardAttributes();
  assert.equal(list.length, seed.length - 1);
  assert.ok(!list.some((a) => a.path === 'usr.email' || a.path === 'checkout.cart_id'));
  // A removed default can be added back.
  const back = await mock.createStandardAttribute(input('usr.email', 'string', ['email']));
  assert.equal(back.isDefault, true);
  assert.equal((await mock.resetStandardAttributes()).length, seed.length);
});

test('the explorer marks standard attributes in fields and groups added ones', async () => {
  await mock.resetStandardAttributes();
  const fields = logs.mockLogFields({ range: lastHour, query: '' });
  const method = fields.find((f) => f.key === '@http.method');
  assert.ok(method, 'nested attributes are listed by path');
  assert.equal(method.standard, true);
  assert.ok(
    fields.some((f) => !f.standard),
    'other fields are not standard',
  );
  const { facets } = logs.mockSearchLogs({ range: lastHour, query: '', facets: ['@http.url', '@flow_id', 'kube_node'] });
  const group = (key) => facets.find((f) => f.key === key)?.group;
  assert.deepEqual([group('@http.url'), group('@flow_id'), group('kube_node')], ['STANDARD ATTRIBUTES', 'ADDED', 'ADDED']);
  await mock.deleteStandardAttribute('http.url');
  const again = logs.mockSearchLogs({ range: lastHour, query: '', facets: ['@http.url'] });
  assert.equal(again.facets.find((f) => f.key === '@http.url').group, 'ADDED');
  await mock.resetStandardAttributes();
});
