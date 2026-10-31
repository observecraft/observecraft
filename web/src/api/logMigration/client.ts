// Logs → Configuration → Migration. In "live" mode from the Rust API, otherwise the in-browser mock
// (mock/logMigration.ts).
//   POST   /api/v1/logs/translate               Splunk SPL → our search + aggregation, explained
//   GET    /api/v1/logs/imports                 imports of exported Splunk assets
//   POST   /api/v1/logs/imports                 upload the exports (CSV): translated and stored
//   GET    /api/v1/logs/imports/:id             an import's assets and their translations
//   POST   /api/v1/logs/imports/:id/migrate     create the monitors and dashboards
//   DELETE /api/v1/logs/imports/:id
import { API_MODE, del, get, post, postTo } from '../http';
import * as mock from '../mock/logMigration';
import type { CreateImport, LogImport, LogImportDetail, SplTranslation } from './types';

export * from './types';

const live = API_MODE === 'live';

export const translateSpl = (query: string, macros?: string): Promise<SplTranslation> =>
  live ? (post('/api/v1/logs/translate', { source: 'spl', query, macros: macros ?? null }) as Promise<SplTranslation>) : mock.translate(query);
export const listImports = (): Promise<LogImport[]> => (live ? (get('/api/v1/logs/imports', {}) as Promise<LogImport[]>) : mock.listImports());
export const getImport = (id: string): Promise<LogImportDetail> => (live ? (get('/api/v1/logs/imports/{id}', { id }) as Promise<LogImportDetail>) : mock.getImport(id));
export const createImport = (input: CreateImport): Promise<LogImportDetail> =>
  live ? (post('/api/v1/logs/imports', { source: 'splunk', ...input }) as Promise<LogImportDetail>) : mock.createImport(input);
export const migrateImport = (id: string, assetIds: string[]): Promise<LogImportDetail> =>
  live ? (postTo('/api/v1/logs/imports/{id}/migrate', { id }, { assetIds }) as Promise<LogImportDetail>) : mock.migrate(id, assetIds);
export const deleteImport = (id: string): Promise<void> => (live ? del('/api/v1/logs/imports/{id}', { id }) : mock.deleteImport(id));
