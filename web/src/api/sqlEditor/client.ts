// SQL Editor data access. In "live" mode queries run on the Rust API, which checks each SELECT and
// runs it on org-scoped virtual tables in ClickHouse; saved queries live in Postgres. Otherwise the
// in-browser mock answers (mock/sqlEditor.ts) with sample rows.
//   GET            /api/v1/sql/schema         getSqlSchema      (virtual tables, columns, functions)
//   GET            /api/v1/sql/templates      listSqlTemplates  (query templates)
//   POST           /api/v1/sql/run            runSql            (one read-only SELECT)
//   GET/POST       /api/v1/sql/queries        listSavedQueries, createSavedQuery
//   PUT/DELETE     /api/v1/sql/queries/{id}   updateSavedQuery, deleteSavedQuery
import type { components } from '../generated/schema';
import { API_MODE, del, get, post, put } from '../http';
import * as mock from '../mock/sqlEditor';

type S = components['schemas'];
export type SqlSchema = S['SqlSchema'];
export type SqlTableInfo = S['SqlTableInfo'];
export type SqlTableColumn = S['SqlTableColumn'];
export type SqlTemplate = S['SqlTemplate'];
export type SqlRunRequest = S['SqlRunRequest'];
export type SqlResult = S['SqlResult'];
export type SavedSqlQuery = S['SavedSqlQuery'];
export type SavedSqlQueryInput = S['SavedSqlQueryInput'];

const live = API_MODE === 'live';

export const getSqlSchema = (): Promise<SqlSchema> => (live ? get('/api/v1/sql/schema', {}) : mock.getSchema());
export const listSqlTemplates = (): Promise<SqlTemplate[]> => (live ? get('/api/v1/sql/templates', {}) : mock.listTemplates());
export const runSql = (req: SqlRunRequest, signal?: AbortSignal): Promise<SqlResult> => (live ? post('/api/v1/sql/run', req, signal) : mock.run(req));
export const listSavedQueries = (): Promise<SavedSqlQuery[]> => (live ? get('/api/v1/sql/queries', {}) : mock.listQueries());
export const createSavedQuery = (i: SavedSqlQueryInput): Promise<SavedSqlQuery> => (live ? post('/api/v1/sql/queries', i) : mock.createQuery(i));
export const updateSavedQuery = (id: string, i: SavedSqlQueryInput): Promise<SavedSqlQuery> => (live ? put('/api/v1/sql/queries/{id}', { id }, i) : mock.updateQuery(id, i));
export const deleteSavedQuery = (id: string): Promise<void> => (live ? del('/api/v1/sql/queries/{id}', { id }) : mock.deleteQuery(id));
