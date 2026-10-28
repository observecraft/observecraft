// Reference tables (Integrations → Reference Tables): CSV files uploaded to look values up by a
// primary key. Outside live mode the in-browser mock (mock/referenceTables.ts) answers.
import { API_MODE, del, get, post, put } from '../http';
import * as mock from '../mock/referenceTables';
import type { ReferenceTable, ReferenceTableFile, ReferenceTableInput, ReferenceTableRows, ReferenceTableRowsQuery } from './types';

export * from './types';

const live = API_MODE === 'live';

export const listReferenceTables = (): Promise<ReferenceTable[]> => (live ? get('/api/v1/reference-tables', {}) : mock.listReferenceTables());
export const createReferenceTable = (i: ReferenceTableInput): Promise<ReferenceTable> => (live ? post('/api/v1/reference-tables', i) : mock.createReferenceTable(i));

/** The table and a page of its rows. */
export function getReferenceTable(name: string, q: ReferenceTableRowsQuery, signal?: AbortSignal): Promise<ReferenceTableRows> {
  if (!live) return mock.getReferenceTable(name, q);
  const query: Record<string, string | number> = { limit: q.limit, offset: q.offset };
  if (q.query?.trim()) query.query = q.query.trim();
  if (q.column) query.column = q.column;
  return get('/api/v1/reference-tables/{name}', { name }, signal, query);
}

/** Replaces every row with those of a new file. */
export const replaceReferenceTable = (name: string, f: ReferenceTableFile): Promise<ReferenceTable> =>
  live ? put('/api/v1/reference-tables/{name}', { name }, f) : mock.replaceReferenceTable(name, f);
export const deleteReferenceTable = (name: string): Promise<void> => (live ? del('/api/v1/reference-tables/{name}', { name }) : mock.deleteReferenceTable(name));
