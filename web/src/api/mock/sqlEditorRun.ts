// Mock-mode query runs: the mock doesn't evaluate SQL. It answers with the sample rows of the first
// table a query reads (up to its LIMIT), refusing what the server refuses outright.
import type { SqlResult } from '../sqlEditor/client';
import { firstTable, limitOf } from '../../pages/sqlEditor/sqlEditorModel';
import { SAMPLE_ROWS, SQL_TABLES } from './sqlEditorCatalog';

export function runMock(sql: string): SqlResult {
  const text = sql.trim().replace(/;\s*$/, '');
  if (!text) throw new Error('Enter a query');
  if (!/^(select|with)\b/i.test(text)) throw new Error('Only SELECT queries can run here');
  if (text.includes(';')) throw new Error('Run one statement at a time');
  const names = SQL_TABLES.map((t) => t.name);
  const name = firstTable(text, names);
  if (!name) {
    const named = /\b(?:from|join)\s+([^\s(),;]+)/i.exec(text)?.[1];
    if (named) throw new Error(`Unknown table ${named}: query the tables listed under Data`);
    return { columns: [{ name: 'result', type: 'UInt8' }], rows: [[1]], truncated: false, elapsedMs: 1, tables: [], notices: [] };
  }
  const table = SQL_TABLES.find((t) => t.name === name)!;
  const sample = SAMPLE_ROWS[name] ?? [];
  const limit = limitOf(text) ?? sample.length;
  return {
    columns: table.columns.map((c) => ({ name: c.name, type: c.type })),
    rows: sample.slice(0, limit).map((r) => table.columns.map((c) => r[c.name] ?? null)),
    truncated: false,
    elapsedMs: 3,
    tables: [name],
    notices: [`Mock mode: sample rows of ${name}. Queries run against your data with the API.`],
  };
}
