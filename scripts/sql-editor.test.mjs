import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';
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
const model = await import('../web/src/pages/sqlEditor/sqlEditorModel.ts');
const catalog = await import('../web/src/api/mock/sqlEditorCatalog.ts');
const templates = JSON.parse(readFileSync(new URL('../server/seeds/sql_templates.json', import.meta.url), 'utf8'));

test('highlighting keeps every character and tags keywords, functions, strings, numbers and comments', () => {
  const sql = "SELECT service, count() AS n -- per service\nFROM spans WHERE status = 'it''s' AND duration_ms > 1.5e2 /* slow */";
  const tokens = model.highlightSql(sql);
  assert.equal(tokens.map((t) => t.text).join(''), sql);
  const kinds = (kind) => tokens.filter((t) => t.kind === kind).map((t) => t.text);
  assert.deepEqual(kinds('keyword'), ['SELECT', 'AS', 'FROM', 'WHERE', 'AND']);
  assert.deepEqual(kinds('function'), ['count']);
  assert.deepEqual(kinds('string'), ["'it''s'"]);
  assert.deepEqual(kinds('number'), ['1.5e2']);
  assert.deepEqual(kinds('comment'), ['-- per service', '/* slow */']);
  // Unterminated strings and comments still cover the rest of the text.
  for (const open of ["SELECT 'abc", 'SELECT /* abc', 'SELECT `col'])
    assert.equal(
      model
        .highlightSql(open)
        .map((t) => t.text)
        .join(''),
      open,
    );
});

test('CSV quotes what needs quoting', () => {
  const csv = model.toCsv(
    ['name', 'note', 'tags', 'n'],
    [
      ['i-0a1b2c3d4e5f60001', 'has, comma', ['a', 'b'], 3],
      ['i-0a1b2c3d4e5f60002', 'say "hi"\nthere', null, null],
    ],
  );
  assert.equal(csv, 'name,note,tags,n\r\ni-0a1b2c3d4e5f60001,"has, comma","[""a"",""b""]",3\r\ni-0a1b2c3d4e5f60002,"say ""hi""\nthere",,\r\n');
  // Spreadsheet formulas are neutralised.
  assert.equal(model.toCsv(['x'], [['=1+1']]), "x\r\n'=1+1\r\n");
});

test('new tabs are named after the time they were opened', () => {
  assert.equal(model.untitledName(new Date(2026, 9, 28, 14, 5)), 'Untitled, Oct 28, 14:05');
});

test('the first table and the limit are read from a query', () => {
  const names = catalog.SQL_TABLES.map((t) => t.name);
  assert.equal(model.firstTable('select * from HOSTS h join spans s on 1=1', names), 'hosts');
  assert.equal(model.firstTable('WITH x AS (SELECT name FROM monitors) SELECT * FROM x', names), 'monitors');
  assert.equal(model.firstTable('SELECT 1', names), null);
  assert.equal(model.limitOf('SELECT * FROM hosts LIMIT 5'), 5);
  assert.equal(model.limitOf('SELECT * FROM hosts'), null);
});

test('every template reads tables of the catalog and the mock answers it', async () => {
  const mock = await import('../web/src/api/mock/sqlEditorRun.ts');
  const names = new Set(catalog.SQL_TABLES.map((t) => t.name));
  assert.ok(templates.length >= 8);
  for (const t of templates) {
    const table = model.firstTable(t.sql, [...names]);
    assert.ok(table && names.has(table), `${t.id} reads a known table`);
    const result = mock.runMock(t.sql);
    assert.ok(result.columns.length > 0, t.id);
  }
  assert.throws(() => mock.runMock('SELECT * FROM otel.otel_traces'), /Unknown table/);
  assert.throws(() => mock.runMock('DROP TABLE hosts'), /Only SELECT/);
});

test('mock tables use fictional demo data', () => {
  const text = JSON.stringify(catalog.SAMPLE_ROWS);
  assert.match(text, /i-0a1b2c3d4e5f6000/);
  assert.match(text, /storefront-api/);
});
