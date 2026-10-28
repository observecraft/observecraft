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

const { compileFormula, FormulaError } = await import('../web/src/lib/sheetFormula.ts');
const { applyCalculated, applyFilters, applySort, pivotTable, computeSheet } = await import('../web/src/lib/sheetGrid.ts');
const mock = await import('../web/src/api/mock/sheets.ts');
const { ME_ID } = await import('../web/src/api/mock/identity.ts');

const evalWith = (src, row = {}, cols = Object.keys(row)) => compileFormula(src, cols)(row);

test('formulas: arithmetic with precedence, parentheses and unary minus', () => {
  assert.equal(evalWith('1 + 2 * 3'), 7);
  assert.equal(evalWith('=(1 + 2) * 3'), 9);
  assert.equal(evalWith('-2 ^ 2'), -4);
  assert.equal(evalWith('2 ^ 3 ^ 2'), 512);
  assert.equal(evalWith('10 / 4'), 2.5);
  assert.equal(evalWith('7 % 3'), 1);
  assert.equal(evalWith('-(3 - 5)'), 2);
  assert.equal(evalWith('1.5e3 + .5'), 1500.5);
});

test('formulas: column references, bare and bracketed', () => {
  const row = { avg: 120, max: 300, 'p99 latency': 450, service: 'billing' };
  assert.equal(evalWith('max - avg', row), 180);
  assert.equal(evalWith('[p99 latency] / 1000', row), 0.45);
  assert.equal(evalWith('CONCAT(service, ": ", ROUND(avg / 7, 1))', row), 'billing: 17.1');
  assert.throws(
    () => compileFormula('nope * 2', ['avg']),
    (e) => e instanceof FormulaError && /Unknown column nope/.test(e.message),
  );
  assert.throws(() => compileFormula('[missing]', ['avg']), FormulaError);
});

test('formulas: IF, comparisons, logic and ROUND', () => {
  const row = { errors: 12, requests: 400 };
  assert.equal(evalWith('IF(errors / requests > 0.02, "high", "ok")', row), 'high');
  assert.equal(evalWith('IF(errors = 12, 1, 0)', row), 1);
  assert.equal(evalWith('IF(errors <> 12, 1, 0)', row), 0);
  assert.equal(evalWith('IF(AND(errors > 1, requests >= 400), "both", "no")', row), 'both');
  assert.equal(evalWith('IF(OR(errors > 100, NOT(requests < 10)), 1, 2)', row), 1);
  assert.equal(evalWith('ROUND(errors / requests * 100, 2)', row), 3);
  assert.equal(evalWith('ROUND(2.5)'), 3);
  assert.equal(evalWith('ABS(-3) + MIN(4, 2, 9) + MAX(1, 5)'), 10);
  assert.equal(evalWith('"a" & "b"'), 'ab');
  assert.equal(evalWith('IF(1 < 2, "only then")'), 'only then');
  assert.equal(evalWith('IF(1 > 2, "only then")'), null);
});

test('formulas: errors are FormulaErrors, never code execution', () => {
  for (const bad of ['1 +', '(1', 'FOO(1)', 'IF(1)', '"open', '1 2', 'constructor', '__proto__', 'alert(1)', 'a.b']) {
    assert.throws(() => compileFormula(bad, ['a'])({ a: 1 }), FormulaError, bad);
  }
  assert.throws(
    () => evalWith('1 / 0'),
    (e) => e instanceof FormulaError && /Division by zero/.test(e.message),
  );
  assert.throws(
    () => evalWith('"x" * 2'),
    (e) => e instanceof FormulaError && /number/.test(e.message),
  );
  // Empty cells count as 0 in arithmetic, "" in text.
  assert.equal(evalWith('a + 1', { a: null }), 1);
  assert.equal(evalWith('CONCAT(a, "!")', { a: null }), '!');
});

const table = () => ({
  columns: [
    { key: 'service', label: 'service', type: 'string' },
    { key: 'region', label: 'region', type: 'string' },
    { key: 'requests', label: 'requests', type: 'number' },
    { key: 'errors', label: 'errors', type: 'number' },
  ],
  rows: [
    { service: 'billing', region: 'eu', requests: 100, errors: 5 },
    { service: 'billing', region: 'us', requests: 300, errors: 3 },
    { service: 'order-service', region: 'eu', requests: 200, errors: 20 },
    { service: 'catalog-service', region: 'us', requests: 50, errors: 0 },
  ],
});

test('grid: calculated columns, with per-cell errors', () => {
  const t = applyCalculated(table(), [
    { id: 'c1', name: 'error rate', formula: 'ROUND(errors / requests * 100, 1)' },
    { id: 'c2', name: 'per error', formula: 'requests / errors' },
    { id: 'c3', name: 'broken', formula: 'nope + 1' },
  ]);
  assert.deepEqual(
    t.columns.map((c) => c.key),
    ['service', 'region', 'requests', 'errors', 'error rate', 'per error', 'broken'],
  );
  assert.deepEqual(
    t.rows.map((r) => r['error rate']),
    [5, 1, 10, 0],
  );
  assert.equal(t.rows[3]['per error'], null);
  assert.match(t.errors['per error'], /Division by zero/);
  assert.match(t.errors.broken, /Unknown column nope/);
  // A calculated column can use an earlier one.
  const chained = applyCalculated(table(), [
    { id: 'c1', name: 'ok', formula: 'requests - errors' },
    { id: 'c2', name: 'ok share', formula: 'ok / requests' },
  ]);
  assert.equal(chained.rows[0]['ok share'], 0.95);
});

test('grid: filters and sort', () => {
  const f = applyFilters(table(), [
    { column: 'requests', op: '>=', value: '100' },
    { column: 'service', op: 'contains', value: 'service' },
  ]);
  assert.deepEqual(
    f.rows.map((r) => r.service),
    ['order-service'],
  );
  assert.equal(applyFilters(table(), [{ column: 'region', op: '!=', value: 'eu' }]).rows.length, 2);
  assert.equal(applyFilters(table(), [{ column: 'region', op: '=', value: 'EU' }]).rows.length, 2, 'text matches ignore case');
  const s = applySort(table(), { column: 'errors', dir: 'desc' });
  assert.deepEqual(
    s.rows.map((r) => r.errors),
    [20, 5, 3, 0],
  );
  const byName = applySort(table(), { column: 'service', dir: 'asc' });
  assert.equal(byName.rows[0].service, 'billing');
});

test('grid: pivot groups by one column and aggregates others', () => {
  const p = pivotTable(table(), {
    groupBy: 'region',
    aggregations: [
      { column: 'requests', fn: 'sum' },
      { column: 'errors', fn: 'avg' },
      { column: 'service', fn: 'count' },
      { column: 'errors', fn: 'max' },
    ],
  });
  assert.deepEqual(
    p.columns.map((c) => c.key),
    ['region', 'sum(requests)', 'avg(errors)', 'count(service)', 'max(errors)'],
  );
  assert.deepEqual(p.rows, [
    { region: 'eu', 'sum(requests)': 300, 'avg(errors)': 12.5, 'count(service)': 2, 'max(errors)': 20 },
    { region: 'us', 'sum(requests)': 350, 'avg(errors)': 1.5, 'count(service)': 2, 'max(errors)': 3 },
  ]);
});

test('grid: computeSheet runs calculated → filters → pivot → sort', () => {
  const out = computeSheet(table(), {
    calculated: [{ id: 'c1', name: 'rate', formula: 'errors / requests' }],
    filters: [{ column: 'rate', op: '>', value: '0' }],
    pivot: { groupBy: 'service', aggregations: [{ column: 'requests', fn: 'sum' }] },
    sort: { column: 'sum(requests)', dir: 'desc' },
  });
  assert.deepEqual(
    out.rows.map((r) => [r.service, r['sum(requests)']]),
    [
      ['billing', 400],
      ['order-service', 200],
    ],
  );
});

// ---------------------------------------------------------------- mock spreadsheets

test('mock: seeded spreadsheets about the demo shop, searchable, mine and per team', async () => {
  const all = await mock.searchSpreadsheets({ query: '', view: 'all', team: null });
  assert.ok(all.total >= 3);
  assert.ok(all.teams.includes('checkout'));
  for (const s of (await mock.searchSpreadsheets({ query: '', view: 'mine', team: null })).spreadsheets) assert.equal(s.author?.id, ME_ID);
  const found = await mock.searchSpreadsheets({ query: 'latency', view: 'all', team: null });
  assert.ok(found.spreadsheets.every((s) => /latency/i.test(s.name) || /latency/i.test(s.author?.name ?? '')));
  for (const s of (await mock.searchSpreadsheets({ query: '', view: 'all', team: 'checkout' })).spreadsheets) assert.equal(s.team, 'checkout');
});

test('mock: create, rename, save and delete', async () => {
  const created = await mock.createSpreadsheet({ name: 'Orders by region' });
  assert.equal(created.author?.id, ME_ID);
  assert.equal(created.definition.sheets.length, 1);
  const renamed = await mock.updateSpreadsheet(created.id, { name: 'Orders per region', team: 'checkout' });
  assert.equal(renamed.name, 'Orders per region');
  assert.equal(renamed.team, 'checkout');
  await assert.rejects(mock.updateSpreadsheet(created.id, { name: ' ' }), /1–200 characters/);
  await assert.rejects(mock.updateSpreadsheet(created.id, { definition: { sheets: [{ id: 'x', name: 'A', source: { type: 'sql' } }] } }), /unknown data source 'sql'/);
  await mock.deleteSpreadsheet(created.id);
  await assert.rejects(mock.getSpreadsheet(created.id), /not found/);
});
