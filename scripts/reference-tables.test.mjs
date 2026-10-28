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
const file = await import('../web/src/api/referenceTables/file.ts');
const mock = await import('../web/src/api/mock/referenceTables.ts');

const DEMO_SERVICES = ['api-gateway', 'storefront-api', 'order-service', 'billing', 'catalog-service', 'inventory-service'];

test('CSV: header row, quoted values, escaped quotes, CRLF, BOM and blank lines', () => {
  const csv = '﻿sku , name,notes\r\nSKU-1,"Mug, large","Says ""hi"""\r\n\r\nSKU-2,Plate,"two\nlines"\n';
  assert.deepEqual(file.parseCsv(csv), {
    columns: ['sku', 'name', 'notes'],
    rows: [
      ['SKU-1', 'Mug, large', 'Says "hi"'],
      ['SKU-2', 'Plate', 'two\nlines'],
    ],
  });
  assert.deepEqual(file.parseCsv('a,b\n1,\n'), { columns: ['a', 'b'], rows: [['1', '']] });
  assert.deepEqual(file.parseCsv(''), { columns: [], rows: [] });
});

test('CSV: malformed quoting is reported with its line', () => {
  assert.throws(() => file.parseCsv('a,b\n1,"open\n'), /Line 2: a quoted value is not closed/);
  assert.throws(() => file.parseCsv('a,b\n1,2\n"x"y,3\n'), /Line 3: unexpected character after a closing quote/);
});

const ok = {
  columns: ['sku', 'name'],
  rows: [
    ['SKU-1', 'Mug'],
    ['SKU-2', 'Plate'],
  ],
  primaryKey: 'sku',
};

test('file checks: a valid file passes', () => {
  assert.equal(file.checkFile(ok), null);
});

test('file checks: clear errors, as the server words them', () => {
  const cases = [
    [{ ...ok, columns: [], rows: [] }, /no columns/],
    [{ ...ok, rows: [] }, /no rows/],
    [{ ...ok, columns: ['sku', ''] }, /Column 2 has no name/],
    [{ ...ok, columns: ['sku', 'sku'] }, /Column 'sku' appears twice/],
    [{ ...ok, primaryKey: 'id' }, /Primary key 'id' is not a column/],
    [{ ...ok, rows: [['SKU-1', 'Mug'], ['SKU-2']] }, /Row 2 has 1 values, expected 2/],
    [
      {
        ...ok,
        rows: [
          ['SKU-1', 'Mug'],
          [' ', 'Plate'],
        ],
      },
      /Row 2 has an empty primary key \(sku\)/,
    ],
    [
      {
        ...ok,
        rows: [
          ['SKU-1', 'Mug'],
          ['SKU-2', 'Cup'],
          ['SKU-1', 'Plate'],
        ],
      },
      /Primary key value 'SKU-1' appears twice \(rows 1 and 3\)/,
    ],
    [{ ...ok, columns: Array.from({ length: file.MAX_COLUMNS + 1 }, (_, i) => `c${i}`), rows: [] }, /at most 50 columns/],
    [{ ...ok, rows: Array.from({ length: file.MAX_ROWS + 1 }, (_, i) => [`k${i}`, 'v']) }, /at most 100,000 rows/],
  ];
  for (const [input, message] of cases) assert.match(file.checkFile(input) ?? '', message, JSON.stringify(input).slice(0, 80));
});

test('table names: lowercase letters, digits and underscores', () => {
  for (const name of ['products', 'service_owners_2', '9lives']) assert.equal(file.checkName(name), null, name);
  for (const name of ['', 'Products', 'with-dash', 'with space', 'x'.repeat(file.MAX_NAME_CHARS + 1)]) assert.match(file.checkName(name) ?? '', /lowercase letters, digits and underscores/, name);
});

test('the demo seed is deterministic and uses the demo domain', async () => {
  const tables = await mock.listReferenceTables();
  assert.ok(tables.length >= 2);
  assert.deepEqual(
    tables.map((t) => t.name),
    [...tables.map((t) => t.name)].sort(),
  );
  for (const t of tables) {
    assert.equal(file.checkName(t.name), null);
    assert.ok(t.columns.includes(t.primaryKey));
    const page = await mock.getReferenceTable(t.name, { limit: 500, offset: 0 });
    assert.equal(page.total, t.rowCount);
    assert.equal(file.checkFile({ columns: t.columns, rows: page.rows, primaryKey: t.primaryKey }), null);
  }
  const owners = await mock.getReferenceTable('service_owners', { limit: 500, offset: 0 });
  for (const row of owners.rows) assert.ok(DEMO_SERVICES.includes(row[0]), `${row[0]} is a demo service`);
  assert.deepEqual(await mock.listReferenceTables(), tables, 'same answer twice');
});

test('rows: search by any column or one column, paginated', async () => {
  const all = await mock.getReferenceTable('product_catalog', { limit: 10, offset: 0 });
  assert.equal(all.rows.length, 10);
  assert.ok(all.total > 10);
  const next = await mock.getReferenceTable('product_catalog', { limit: 10, offset: 10 });
  assert.notDeepEqual(next.rows[0], all.rows[0]);
  const hit = await mock.getReferenceTable('product_catalog', { query: 'sku-0003', limit: 10, offset: 0 });
  assert.deepEqual(
    hit.rows.map((r) => r[0]),
    ['SKU-0003'],
  );
  const kitchen = await mock.getReferenceTable('product_catalog', { query: 'kitchen', column: 'category', limit: 500, offset: 0 });
  assert.ok(kitchen.total > 0);
  const ci = kitchen.table.columns.indexOf('category');
  for (const r of kitchen.rows) assert.match(r[ci], /kitchen/i);
  const none = await mock.getReferenceTable('product_catalog', { query: 'kitchen', column: 'sku', limit: 10, offset: 0 });
  assert.equal(none.total, 0);
  await assert.rejects(mock.getReferenceTable('product_catalog', { column: 'nope', limit: 10, offset: 0 }), /no column 'nope'/);
  await assert.rejects(mock.getReferenceTable('missing', { limit: 10, offset: 0 }), /not found/);
});

test('create, replace and delete', async () => {
  await assert.rejects(mock.createReferenceTable({ name: 'Bad-Name', description: '', primaryKey: 'sku', fileName: 'x.csv', ...ok }), /lowercase/);
  await assert.rejects(mock.createReferenceTable({ name: 'product_catalog', description: '', fileName: 'x.csv', ...ok }), /already exists/);
  await assert.rejects(
    mock.createReferenceTable({
      name: 'dupes',
      description: '',
      fileName: 'x.csv',
      ...ok,
      rows: [
        ['A', '1'],
        ['A', '2'],
      ],
    }),
    /appears twice/,
  );
  const created = await mock.createReferenceTable({ name: 'warehouse_codes', description: ' Codes ', fileName: 'warehouses.csv', ...ok });
  assert.equal(created.name, 'warehouse_codes');
  assert.equal(created.description, 'Codes');
  assert.equal(created.rowCount, 2);
  assert.equal(created.source, 'local_file');
  assert.ok(created.createdBy);
  assert.ok((await mock.listReferenceTables()).some((t) => t.name === 'warehouse_codes'));

  await assert.rejects(mock.replaceReferenceTable('warehouse_codes', { fileName: 'y.csv', columns: ['code', 'name'], rows: [['W1', 'East']] }), /Primary key 'sku' is not a column/);
  const replaced = await mock.replaceReferenceTable('warehouse_codes', { fileName: 'y.csv', columns: ['sku', 'name', 'zone'], rows: [['W1', 'East', 'a']] });
  assert.equal(replaced.rowCount, 1);
  assert.equal(replaced.fileName, 'y.csv');
  assert.deepEqual(replaced.columns, ['sku', 'name', 'zone']);
  assert.ok(replaced.updatedAt >= created.updatedAt);
  assert.deepEqual((await mock.getReferenceTable('warehouse_codes', { limit: 10, offset: 0 })).rows, [['W1', 'East', 'a']]);

  await mock.deleteReferenceTable('warehouse_codes');
  assert.ok(!(await mock.listReferenceTables()).some((t) => t.name === 'warehouse_codes'));
  await assert.rejects(mock.deleteReferenceTable('warehouse_codes'), /not found/);
});
