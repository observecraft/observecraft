// Dashboards + metric query data access. In "live" mode (VITE_API_MODE=live) every call goes to the
// Rust API (dashboards in Postgres, metric queries over ClickHouse); otherwise the in-browser mock
// answers, so the UI still runs without a backend.
import * as dash from '../mock/dashboards';
import * as metrics from '../mock/metricsEngine';
import type { components } from '../generated/schema';
import { API_MODE, get, patch, post, postTo } from '../http';
import type { TimeRange } from '../types';
import type {
  CreateDashboardRequest,
  Dashboard,
  DashboardList,
  DashboardSearchPage,
  DashboardSearchRequest,
  DashboardSummary,
  DistributionResponse,
  GroupedScalar,
  MetricCatalogEntry,
  MetricSeries,
  MonitorSummary,
  ScalarRequest,
  TimeseriesRequest,
  User,
  Widget,
} from './types';

export type DashboardPatch = dash.DashboardPatch;

const live = API_MODE === 'live';

type WireSummary = components['schemas']['DashboardSummary'];
type WireDashboard = components['schemas']['Dashboard'];

const toSummary = (d: WireSummary): DashboardSummary => ({
  ...d,
  description: d.description ?? undefined,
  logo: d.logo ?? undefined,
  author: d.author ?? null,
  viewedAt: d.viewedAt ?? null,
  deletedAt: d.deletedAt ?? null,
});

// Widgets are opaque JSON to the server; their schema lives in ./types.
const toDashboard = (d: WireDashboard): Dashboard => ({ ...toSummary(d), layoutType: d.layoutType, templateVariables: d.templateVariables, widgets: d.widgets as unknown as Widget[] });

export async function searchDashboards(req: DashboardSearchRequest): Promise<DashboardSearchPage> {
  if (!live) return dash.searchDashboards(req);
  const page = await post('/api/v1/dashboards/search', req);
  return { total: page.total, dashboards: page.dashboards.map(toSummary) };
}

export async function recentlyViewed(limit: number): Promise<DashboardSummary[]> {
  if (!live) return dash.recentlyViewed(limit);
  return (await get('/api/v1/dashboards/recent', {}, undefined, { limit })).map(toSummary);
}

export async function getDashboard(id: string): Promise<Dashboard> {
  if (!live) return dash.getDashboard(id);
  return toDashboard(await get('/api/v1/dashboards/{id}', { id }));
}

export async function createDashboard(req: CreateDashboardRequest): Promise<Dashboard> {
  if (!live) return dash.createDashboard(req);
  return toDashboard(await post('/api/v1/dashboards', req));
}

export async function updateDashboard(id: string, p: DashboardPatch): Promise<Dashboard> {
  if (!live) return dash.updateDashboard(id, p);
  const body = { ...p, widgets: p.widgets as unknown as components['schemas']['DashboardPatch']['widgets'] };
  return toDashboard(await patch('/api/v1/dashboards/{id}', { id }, body));
}

export async function cloneDashboard(id: string): Promise<Dashboard> {
  if (!live) return dash.cloneDashboard(id);
  return toDashboard(await postTo('/api/v1/dashboards/{id}/clone', { id }, undefined));
}

export async function deleteDashboards(ids: string[]): Promise<void> {
  if (!live) return dash.deleteDashboards(ids);
  await post('/api/v1/dashboards/delete', { ids });
}

export async function restoreDashboard(id: string): Promise<void> {
  if (!live) return dash.restoreDashboard(id);
  await postTo('/api/v1/dashboards/{id}/restore', { id }, undefined);
}

export async function listLists(): Promise<DashboardList[]> {
  if (!live) return dash.listLists();
  return get('/api/v1/dashboard-lists', {});
}

export async function createList(name: string): Promise<DashboardList> {
  if (!live) return dash.createList(name);
  return post('/api/v1/dashboard-lists', { name });
}

export async function addToList(listId: string, ids: string[]): Promise<DashboardList> {
  if (!live) return dash.addToList(listId, ids);
  return postTo('/api/v1/dashboard-lists/{id}/items', { id: listId }, { dashboardIds: ids });
}

export async function monitorSummary(query: string): Promise<MonitorSummary> {
  if (!live) return dash.monitorSummary(query);
  const r = await post('/api/v1/monitors/summary', { query });
  return { counts: r.counts, monitors: r.monitors.map((m) => ({ ...m, status: m.status as MonitorSummary['monitors'][number]['status'] })) };
}

/** The mock data's user; in live mode the signed-in user comes from `/api/v1/me`. */
export const mockUser = (): User => dash.CURRENT_USER;

// ---------------------------------------------------------------- metric queries

export async function queryTimeseries(req: TimeseriesRequest): Promise<MetricSeries[]> {
  if (!live) return metrics.queryTimeseries(req);
  return post('/api/v1/query/timeseries', req);
}

export async function queryScalar(req: ScalarRequest): Promise<number> {
  if (!live) return metrics.queryScalar(req);
  return post('/api/v1/query/scalar', req);
}

export async function queryGrouped(req: ScalarRequest): Promise<GroupedScalar[]> {
  if (!live) return metrics.queryGrouped(req);
  return post('/api/v1/query/grouped', req);
}

export async function queryDistribution(req: { query: string; range: TimeRange }): Promise<DistributionResponse> {
  if (!live) return metrics.queryDistribution(req);
  return post('/api/v1/query/distribution', req);
}

export async function metricCatalog(): Promise<MetricCatalogEntry[]> {
  if (!live) return metrics.metricCatalog();
  return (await get('/api/v1/metrics', {})).map((m) => ({ ...m, unit: m.unit ?? undefined }));
}

export async function tagValues(key: string): Promise<string[]> {
  if (!live) return metrics.tagValues(key);
  return get('/api/v1/tags/{key}/values', { key });
}
