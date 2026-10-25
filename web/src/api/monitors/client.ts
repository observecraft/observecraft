// Monitors and downtimes (Postgres + evaluation over ClickHouse via the API). No in-browser mock.
import type { components } from '../generated/schema';
import { del, get, post, postTo, put } from '../http';
import { requireLive } from '../identity/client';

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
  requireLive();
  return post('/api/v1/monitors/search', req);
}

export async function searchTriggered(req: TriggeredSearchRequest) {
  requireLive();
  return post('/api/v1/monitors/triggered', req);
}

export async function getMonitor(id: number) {
  requireLive();
  return get('/api/v1/monitors/{id}', { id: String(id) });
}

export async function createMonitor(input: MonitorInput) {
  requireLive();
  return post('/api/v1/monitors', input);
}

export async function updateMonitor(id: number, input: MonitorInput) {
  requireLive();
  return put('/api/v1/monitors/{id}', { id: String(id) }, input);
}

export async function cloneMonitor(id: number) {
  requireLive();
  return postTo('/api/v1/monitors/{id}/clone', { id: String(id) }, undefined);
}

export async function bulkMonitors(ids: number[], action: BulkAction, muteMinutes?: number | null) {
  requireLive();
  return post('/api/v1/monitors/bulk', { ids, action, muteMinutes: muteMinutes ?? null });
}

export async function monitorHistory(id: number, range: TimeRange) {
  requireLive();
  return postTo('/api/v1/monitors/{id}/history', { id: String(id) }, { range });
}

export async function previewMonitor(type: MonitorType, query: string, options: MonitorOptions, range: TimeRange, signal?: AbortSignal) {
  requireLive();
  return post('/api/v1/monitors/preview', { type, query, options, range }, signal);
}

/** Sends one rendered transition message to its @recipients now (marked as a test). */
export async function sendTestNotification(req: { monitorId?: number | null; monitorName: string; message: string; status: string; value?: number | null }) {
  requireLive();
  return post('/api/v1/monitors/test-notification', { monitorId: req.monitorId ?? null, monitorName: req.monitorName, message: req.message, status: req.status, value: req.value ?? null });
}

export async function monitorQuality(req: MonitorQualityRequest) {
  requireLive();
  return post('/api/v1/monitors/quality', req);
}

export async function searchDowntimes(req: DowntimeSearchRequest) {
  requireLive();
  return post('/api/v1/downtimes/search', req);
}

export async function createDowntime(input: DowntimeInput) {
  requireLive();
  return post('/api/v1/downtimes', input);
}

export async function cancelDowntime(id: number) {
  requireLive();
  return del('/api/v1/downtimes/{id}', { id: String(id) });
}
