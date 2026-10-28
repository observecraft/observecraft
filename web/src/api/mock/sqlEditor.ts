// In-browser SQL Editor for mock mode: the tables and templates the server serves, sample results
// (mock/sqlEditorRun.ts) and saved queries kept in memory for the session.
import templates from '../../../../server/seeds/sql_templates.json';
import type { SavedSqlQuery, SavedSqlQueryInput, SqlResult, SqlRunRequest, SqlSchema, SqlTemplate } from '../sqlEditor/client';
import { SQL_FUNCTIONS, SQL_TABLES } from './sqlEditorCatalog';
import { runMock } from './sqlEditorRun';

const delay = <T>(v: T, ms = 120): Promise<T> => new Promise((r) => setTimeout(() => r(v), ms));
const NOW = Date.now();

let saved: SavedSqlQuery[] = [
  {
    id: 'q-1',
    name: 'Alerting monitors by team',
    sql: "SELECT arrayJoin(teams) AS team, count() AS alerting\nFROM monitors\nWHERE status = 'alert'\nGROUP BY team\nORDER BY alerting DESC",
    team: 'sre',
    author: 'Alex Morgan',
    updatedBy: 'Alex Morgan',
    createdAt: NOW - 3 * 86_400_000,
    updatedAt: NOW - 2 * 3_600_000,
  },
  {
    id: 'q-2',
    name: 'Containers in CrashLoopBackOff',
    sql: "SELECT cluster, namespace, pod, name, restarts\nFROM containers\nWHERE status = 'CrashLoopBackOff'\nORDER BY restarts DESC",
    team: 'platform',
    author: 'Sam Rivera',
    updatedBy: 'Sam Rivera',
    createdAt: NOW - 9 * 86_400_000,
    updatedAt: NOW - 6 * 86_400_000,
  },
];
let nextId = 3;

function validate(i: SavedSqlQueryInput): SavedSqlQueryInput {
  const name = i.name.trim();
  if (!name) throw new Error('Name the query');
  if (name.length > 200) throw new Error('Names are at most 200 characters');
  if (!i.sql.trim()) throw new Error('The query is empty');
  return { name, sql: i.sql, team: (i.team ?? '').trim() };
}

export const getSchema = (): Promise<SqlSchema> => delay({ tables: SQL_TABLES, functions: SQL_FUNCTIONS, maxRows: 10_000 });
export const listTemplates = (): Promise<SqlTemplate[]> => delay(templates as SqlTemplate[]);
export const run = (req: SqlRunRequest): Promise<SqlResult> =>
  new Promise((resolve, reject) =>
    setTimeout(() => {
      try {
        resolve(runMock(req.sql));
      } catch (e) {
        reject(e);
      }
    }, 250),
  );
export const listQueries = (): Promise<SavedSqlQuery[]> => delay([...saved].sort((a, b) => b.updatedAt - a.updatedAt));

export async function createQuery(i: SavedSqlQueryInput): Promise<SavedSqlQuery> {
  const v = validate(i);
  const now = Date.now();
  const q: SavedSqlQuery = { id: `q-${nextId++}`, name: v.name, sql: v.sql, team: v.team ?? '', author: 'You', updatedBy: 'You', createdAt: now, updatedAt: now };
  saved = [q, ...saved];
  return delay(q);
}

export async function updateQuery(id: string, i: SavedSqlQueryInput): Promise<SavedSqlQuery> {
  const v = validate(i);
  const q = saved.find((s) => s.id === id);
  if (!q) throw new Error('Query not found');
  Object.assign(q, { name: v.name, sql: v.sql, team: v.team ?? '', updatedBy: 'You', updatedAt: Date.now() });
  return delay({ ...q });
}

export async function deleteQuery(id: string): Promise<void> {
  if (!saved.some((s) => s.id === id)) throw new Error('Query not found');
  saved = saved.filter((s) => s.id !== id);
  return delay(undefined);
}
