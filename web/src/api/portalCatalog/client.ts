// Developer Portal → Software Catalog and Reports. In "live" mode they come from the Rust API
// (routes/portal_catalog.rs, routes/portal_reports.rs); otherwise the in-browser mock answers
// (mock/portalCatalog.ts).
//   POST   /api/v1/portal/catalog                 getPortalCatalog
//   GET    /api/v1/portal/entities                listEntityDefinitions
//   GET    /api/v1/portal/entities/:id            getEntityDefinition
//   POST   /api/v1/portal/entities                upsertEntityDefinition
//   PUT    /api/v1/portal/entities/:id            updateEntityDefinition
//   DELETE /api/v1/portal/entities/:id            deleteEntityDefinition
//   POST   /api/v1/portal/entities/import         importEntityDefinitions
//   POST   /api/v1/portal/reports/teams           getTeamsReport
//   POST   /api/v1/portal/reports/scorecards      getScorecardsReport
//   GET    /api/v1/portal/reports/dashboards      listReportDashboards
//   PUT    /api/v1/portal/reports/dashboards      setReportDashboards
import { API_MODE, del, get, post, put } from '../http';
import * as mock from '../mock/portalCatalog';
import type {
  EntityDefinition,
  PortalCatalog,
  PortalCatalogRequest,
  PortalDefinitionItem,
  PortalImportResult,
  PortalReportDashboard,
  PortalReportRequest,
  PortalScorecardsReport,
  PortalTeamsReport,
} from './types';

export * from './types';

const live = API_MODE === 'live';

export const getPortalCatalog = (req: PortalCatalogRequest, signal?: AbortSignal): Promise<PortalCatalog> => (live ? post('/api/v1/portal/catalog', req, signal) : mock.getPortalCatalog(req));
export const listEntityDefinitions = (): Promise<PortalDefinitionItem[]> => (live ? get('/api/v1/portal/entities', {}) : mock.listEntityDefinitions());
export const getEntityDefinition = (id: string, signal?: AbortSignal): Promise<PortalDefinitionItem> => (live ? get('/api/v1/portal/entities/{id}', { id }, signal) : mock.getEntityDefinition(id));
/** Creates or replaces the definition of (kind, name). */
export const upsertEntityDefinition = (def: EntityDefinition): Promise<PortalDefinitionItem> => (live ? post('/api/v1/portal/entities', def) : mock.upsertEntityDefinition(def));
export const updateEntityDefinition = (id: string, def: EntityDefinition): Promise<PortalDefinitionItem> =>
  live ? put('/api/v1/portal/entities/{id}', { id }, def) : mock.updateEntityDefinition(id, def);
export const deleteEntityDefinition = (id: string): Promise<void> => (live ? del('/api/v1/portal/entities/{id}', { id }) : mock.deleteEntityDefinition(id));
/** Imports YAML (`---`-separated) or JSON entity definition documents, apiVersion v3. */
export const importEntityDefinitions = (content: string): Promise<PortalImportResult> => (live ? post('/api/v1/portal/entities/import', { content }) : mock.importEntityDefinitions(content));

export const getTeamsReport = (req: PortalReportRequest, signal?: AbortSignal): Promise<PortalTeamsReport> => (live ? post('/api/v1/portal/reports/teams', req, signal) : mock.getTeamsReport(req));
export const getScorecardsReport = (req: PortalReportRequest, signal?: AbortSignal): Promise<PortalScorecardsReport> =>
  live ? post('/api/v1/portal/reports/scorecards', req, signal) : mock.getScorecardsReport(req);
export const listReportDashboards = (signal?: AbortSignal): Promise<PortalReportDashboard[]> => (live ? get('/api/v1/portal/reports/dashboards', {}, signal) : mock.listReportDashboards());
export const setReportDashboards = (dashboardIds: string[]): Promise<PortalReportDashboard[]> =>
  live ? put('/api/v1/portal/reports/dashboards', {}, { dashboardIds }) : mock.setReportDashboards(dashboardIds);
