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
const mock = await import('../web/src/api/mock/notebooks.ts');
const { NOTEBOOK_TEMPLATES, fromTemplate } = await import('../web/src/api/notebooks/templates.ts');
const { ME_ID } = await import('../web/src/api/mock/identity.ts');

const search = (view, extra = {}) => mock.searchNotebooks({ query: '', view, sort: 'modified', dir: 'desc', offset: 0, limit: 50, ...extra });
const titles = async (view, extra) => (await search(view, extra)).notebooks.map((n) => n.title);
const md = (id, text) => ({ id, definition: { type: 'markdown', text } });

test('the seed is about the demo shop, with live query cells', async () => {
  const all = await search('all');
  assert.ok(all.total >= 4);
  const checkout = all.notebooks.find((n) => n.title === 'Checkout latency investigation');
  assert.ok(checkout, 'the checkout investigation is seeded');
  const nb = await mock.getNotebook(checkout.id);
  const types = nb.cells.map((c) => c.definition.type);
  for (const t of ['markdown', 'timeseries', 'log_stream']) assert.ok(types.includes(t), `has a ${t} cell`);
  assert.ok(JSON.stringify(nb.cells).includes('order-service'));
  assert.ok(all.notebooks.some((n) => /^Postmortem/.test(n.title) && n.teams.includes('billing')));
  assert.ok(all.authors.length >= 2 && all.authors.some((a) => a.id === ME_ID));
  assert.ok(all.teams.includes('checkout') && all.teams.includes('payments'));
});

test('lists: mine, my teams, favorites, recents and recently deleted', async () => {
  for (const n of (await search('mine')).notebooks) assert.equal(n.author?.id, ME_ID);
  // The current user is on the platform and sre teams.
  for (const n of (await search('team')).notebooks) assert.ok(n.teams.some((t) => t === 'platform' || t === 'sre'));
  assert.ok((await search('team')).total >= 1);
  for (const n of (await search('favorites')).notebooks) assert.ok(n.favorite);
  const deleted = await search('deleted');
  assert.ok(deleted.total >= 1);
  for (const n of deleted.notebooks) assert.ok(n.deletedAt);
  for (const view of ['all', 'mine', 'team', 'favorites', 'recents']) for (const n of (await search(view)).notebooks) assert.equal(n.deletedAt, null, `${view} hides deleted notebooks`);
});

test('opening a notebook puts it first in Recents', async () => {
  const last = (await search('all', { sort: 'title', dir: 'asc' })).notebooks.at(-1);
  await mock.getNotebook(last.id);
  const recents = (await search('recents')).notebooks;
  assert.equal(recents[0].id, last.id);
  assert.ok(recents[0].viewedAt);
});

test('search matches titles, authors, tags and cell text; filters by author and team', async () => {
  assert.deepEqual(await titles('all', { query: 'checkout latency' }), ['Checkout latency investigation']);
  assert.ok((await titles('all', { query: 'postmortem' })).length >= 1, 'tag match');
  const byText = await titles('all', { query: 'trace.latency.p95' });
  assert.ok(byText.includes('Checkout latency investigation'), 'cell text match');
  for (const n of (await search('all', { author: ME_ID })).notebooks) assert.equal(n.author?.id, ME_ID);
  for (const n of (await search('all', { team: 'payments' })).notebooks) assert.ok(n.teams.includes('payments'));
});

test('create, edit, favorite, clone, delete and restore', async () => {
  const nb = await mock.createNotebook({ title: '  Cart abandonment  ', teams: ['checkout', 'checkout'], cells: [md('a', '# Notes')] });
  assert.equal(nb.title, 'Cart abandonment');
  assert.deepEqual(nb.teams, ['checkout']);
  assert.equal(nb.author?.id, ME_ID);
  assert.equal(nb.time, '1h');
  const edited = await mock.updateNotebook(nb.id, { cells: [...nb.cells, { id: 'b', definition: { type: 'log_stream', query: 'service:order-service', limit: 20 }, time: '15m' }], time: '4h' });
  assert.equal(edited.cellCount, 2);
  assert.equal(edited.time, '4h');
  assert.ok(edited.modifiedAt >= nb.modifiedAt);

  await mock.setNotebookFavorite(nb.id, true);
  assert.ok((await search('favorites')).notebooks.some((n) => n.id === nb.id));
  await mock.setNotebookFavorite(nb.id, false);
  assert.ok(!(await search('favorites')).notebooks.some((n) => n.id === nb.id));

  const copy = await mock.cloneNotebook(nb.id);
  assert.equal(copy.title, 'Clone of Cart abandonment');
  assert.deepEqual(copy.cells, edited.cells);

  await mock.deleteNotebook(nb.id);
  assert.ok(!(await search('all')).notebooks.some((n) => n.id === nb.id));
  assert.ok((await search('deleted')).notebooks.some((n) => n.id === nb.id));
  await assert.rejects(mock.updateNotebook(nb.id, { title: 'x' }), /restore/);
  const restored = await mock.restoreNotebook(nb.id);
  assert.equal(restored.deletedAt, null);
  assert.ok((await search('all')).notebooks.some((n) => n.id === nb.id));
  await assert.rejects(mock.restoreNotebook(nb.id), /Recently Deleted/);
});

test('cells are validated like the server does', async () => {
  const reject = (cells, pattern) => assert.rejects(mock.createNotebook({ title: 'Bad', cells }), pattern);
  await reject([{ id: 'a', definition: { type: 'iframe' } }], /unsupported/);
  await reject([md('a', 'x'), md('a', 'y')], /twice/);
  await reject([{ id: 'a', definition: { type: 'timeseries', display: 'line', requests: [] } }], /metric query/);
  await reject([{ id: 'a', definition: { type: 'log_stream', query: '', limit: 0 } }], /limit/);
  await reject([{ ...md('a', 'x'), time: '3y' }], /time range/);
  await reject([md('a', 'x'.repeat(50_001))], /characters/);
  await assert.rejects(mock.createNotebook({ title: ' ' }), /title/);
  await assert.rejects(mock.getNotebook('nope'), /not found/);
});

test('every template makes a valid notebook', async () => {
  assert.deepEqual(
    NOTEBOOK_TEMPLATES.map((t) => t.name),
    ['Postmortem', 'Runbook', 'Incident Report', 'Investigation'],
  );
  for (const t of NOTEBOOK_TEMPLATES) {
    const nb = await mock.createNotebook(fromTemplate(t, 'billing invoices failing'));
    assert.ok(nb.title.includes('billing invoices failing'));
    assert.equal(nb.cellCount, t.cells.length);
    assert.equal(new Set(nb.cells.map((c) => c.id)).size, nb.cells.length, 'fresh unique cell ids');
  }
});

const cells = await import('../web/src/pages/notebooks/cells.ts');

test('cells move, duplicate and convert to and from dashboard widgets', () => {
  const list = [md('a', 'one'), md('b', 'two'), md('c', 'three')];
  assert.deepEqual(
    cells.moveCell(list, 'b', -1).map((c) => c.id),
    ['b', 'a', 'c'],
  );
  assert.deepEqual(
    cells.moveCell(list, 'c', 1).map((c) => c.id),
    ['a', 'b', 'c'],
    'the last cell stays last',
  );
  const dup = cells.duplicateCell(list, 'a');
  assert.equal(dup.length, 4);
  assert.equal(dup[1].definition.text, 'one');
  assert.notEqual(dup[1].id, 'a');
  assert.deepEqual(
    cells.insertCell(list, 1, md('x', '')).map((c) => c.id),
    ['a', 'x', 'b', 'c'],
  );

  const fromWidget = cells.cellFromWidget({ type: 'timeseries', display: 'line', requests: [{ query: 'avg:trace.latency.p95{env:$env}' }] }, (q) => q.replace('$env', 'prod'));
  assert.equal(fromWidget.definition.requests[0].query, 'avg:trace.latency.p95{env:prod}');
  assert.equal(cells.cellFromWidget({ type: 'note', content: '# Hi', background: 'yellow', align: 'left', fontSize: 'md' }).definition.text, '# Hi');
  assert.equal(cells.cellFromWidget({ type: 'pie', request: { query: 'avg:system.mem.used{*}' }, aggregator: 'avg' }), null);
  for (const type of ['markdown', 'timeseries', 'query_value', 'toplist', 'log_stream']) mock.validateCells([{ id: type, definition: cells.newCellDefinition(type) }]);

  const widgets = cells.cellsToWidgets([md('a', '# Notes'), { id: 'b', definition: cells.newCellDefinition('timeseries') }]);
  assert.equal(widgets[0].definition.type, 'note');
  assert.equal(widgets[1].definition.type, 'timeseries');
  assert.equal(widgets[1].layout.y, widgets[0].layout.h, 'stacked in notebook order');
});
