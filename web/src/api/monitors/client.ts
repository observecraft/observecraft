// Monitors and downtimes. In "live" mode they come from the Rust API (Postgres definitions, evaluation over
// ClickHouse); otherwise the in-browser mock answers (monitors seeded from the recommended templates and
// evaluated over the mock metric series, see mock/monitors.ts).
//   POST   /api/v1/monitors/search             searchMonitors
//   POST   /api/v1/monitors/triggered          searchTriggered
//   GET    /api/v1/monitors/:id                getMonitor
//   POST   /api/v1/monitors                    createMonitor
//   PUT    /api/v1/monitors/:id                updateMonitor
//   POST   /api/v1/monitors/:id/clone          cloneMonitor
//   POST   /api/v1/monitors/bulk               bulkMonitors (mute, unmute, resolve, delete)
//   POST   /api/v1/monitors/:id/history        monitorHistory
//   POST   /api/v1/monitors/preview            previewMonitor
//   POST   /api/v1/monitors/test-notification  sendTestNotification
//   POST   /api/v1/monitors/quality            monitorQuality
//   POST   /api/v1/downtimes/search            searchDowntimes
//   POST   /api/v1/downtimes                   createDowntime
//   DELETE /api/v1/downtimes/:id               cancelDowntime
import type { components } from '../generated/schema';
import { API_MODE, del, get, post, postTo, put } from '../http';
import * as mock from '../mock/monitors';

type S = components['schemas'];
export type MonitorItem = S['MonitorItem'];
export type MonitorDetail = S['MonitorDetail'];
export type MonitorInput = S['MonitorInput'];
export type MonitorOptions = S['MonitorOptions'];
export type MonitorStatus = S['MonitorStatus'];
export type MonitorType = S['MonitorType'];
export type MonitorSearchRequest = S['MonitorSearchRequest'];
export type MonitorSearchPage = S['MonitorSearchPage'];
export type TriggeredSearchRequest = S['TriggeredSearchRequest'];
export type TriggeredGroup = S['TriggeredGroup'];
export type MonitorEvent = S['MonitorEvent'];
export type StatusSegment = S['StatusSegment'];
export type MonitorPreview = S['MonitorPreview'];
export type MonitorQuality = S['MonitorQuality'];
export type MonitorQualityRequest = S['MonitorQualityRequest'];
export type BulkAction = S['BulkAction'];
export type FacetCount = S['FacetCount'];
export type DowntimeItem = S['DowntimeItem'];
export type DowntimeInput = S['DowntimeInput'];
export type DowntimeSearchRequest = S['DowntimeSearchRequest'];
export type TimeRange = S['TimeRange'];

export async function searchMonitors(req: MonitorSearchRequest) {
  if (API_MODE !== 'live') return mock.searchMonitors(req);
  return post('/api/v1/monitors/search', req);
}

export async function searchTriggered(req: TriggeredSearchRequest) {
  if (API_MODE !== 'live') return mock.searchTriggered(req);
  return post('/api/v1/monitors/triggered', req);
}

export async function getMonitor(id: number) {
  if (API_MODE !== 'live') return mock.getMonitor(id);
  return get('/api/v1/monitors/{id}', { id: String(id) });
}

export async function createMonitor(input: MonitorInput) {
  if (API_MODE !== 'live') return mock.createMonitor(input);
  return post('/api/v1/monitors', input);
}

export async function updateMonitor(id: number, input: MonitorInput) {
  if (API_MODE !== 'live') return mock.updateMonitor(id, input);
  return put('/api/v1/monitors/{id}', { id: String(id) }, input);
}

export async function cloneMonitor(id: number) {
  if (API_MODE !== 'live') return mock.cloneMonitor(id);
  return postTo('/api/v1/monitors/{id}/clone', { id: String(id) }, undefined);
}

export async function bulkMonitors(ids: number[], action: BulkAction, muteMinutes?: number | null) {
  if (API_MODE !== 'live') return mock.bulkMonitors(ids, action, muteMinutes);
  return post('/api/v1/monitors/bulk', { ids, action, muteMinutes: muteMinutes ?? null });
}

export async function monitorHistory(id: number, range: TimeRange) {
  if (API_MODE !== 'live') return mock.monitorHistory(id, range);
  return postTo('/api/v1/monitors/{id}/history', { id: String(id) }, { range });
}

export async function previewMonitor(type: MonitorType, query: string, options: MonitorOptions, range: TimeRange, signal?: AbortSignal) {
  if (API_MODE !== 'live') return mock.previewMonitor(type, query, options, range);
  return post('/api/v1/monitors/preview', { type, query, options, range }, signal);
}

/** Sends one rendered transition message to its @recipients now (marked as a test). */
export async function sendTestNotification(req: { monitorId?: number | null; monitorName: string; message: string; status: string; value?: number | null }) {
  if (API_MODE !== 'live') return mock.sendTestNotification({ monitorId: req.monitorId ?? null, monitorName: req.monitorName, message: req.message, status: req.status, value: req.value ?? null });
  return post('/api/v1/monitors/test-notification', { monitorId: req.monitorId ?? null, monitorName: req.monitorName, message: req.message, status: req.status, value: req.value ?? null });
}

export async function monitorQuality(req: MonitorQualityRequest) {
  if (API_MODE !== 'live') return mock.monitorQuality(req);
  return post('/api/v1/monitors/quality', req);
}

export async function searchDowntimes(req: DowntimeSearchRequest) {
  if (API_MODE !== 'live') return mock.searchDowntimes(req);
  return post('/api/v1/downtimes/search', req);
}

export async function createDowntime(input: DowntimeInput) {
  if (API_MODE !== 'live') return mock.createDowntime(input);
  return post('/api/v1/downtimes', input);
}

export async function cancelDowntime(id: number) {
  if (API_MODE !== 'live') return mock.cancelDowntime(id);
  return del('/api/v1/downtimes/{id}', { id: String(id) });
}
