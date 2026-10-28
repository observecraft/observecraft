// Event Management data access. In "live" mode events come from the Rust API (events the Datadog
// Agent and the events API send through the intake, in ClickHouse); otherwise the in-browser mock answers.
//   POST /api/v1/events/search   searchEvents
//   POST /api/v1/events/fields   eventFields
//   POST /api/v1/events/work-items   workItems
import type { components } from '../generated/schema';
import { API_MODE, del, get, post, put } from '../http';
import * as mock from '../mock/events';
import * as settings from '../mock/eventSettings';

const live = API_MODE === 'live';

export type EventItem = components['schemas']['Event'];
export type EventPage = components['schemas']['EventPage'];
export type EventSearchRequest = components['schemas']['EventSearchRequest'];
export type EventBucket = components['schemas']['EventBucket'];

export async function searchEvents(req: EventSearchRequest): Promise<EventPage> {
  if (!live) return mock.searchEvents(req);
  return post('/api/v1/events/search', req);
}

/** Tag keys of the matching events, for the facet panel's "+ Add". */
export async function eventFields(req: { range: EventSearchRequest['range']; query?: string }): Promise<{ key: string; count: number }[]> {
  if (!live) return mock.eventFields(req);
  return post('/api/v1/events/fields', { range: req.range, query: req.query ?? '' });
}

export type WorkItem = components['schemas']['WorkItem'];
export type WorkItemsPage = components['schemas']['WorkItemsPage'];

/** Triage Inbox: warning and error events correlated into work items. */
export async function workItems(req: { range: EventSearchRequest['range']; query?: string }): Promise<WorkItemsPage> {
  if (!live) return mock.workItems(req);
  return post('/api/v1/events/work-items', { range: req.range, query: req.query ?? '' });
}

// Event Management → Settings: correlators, maintenance windows and work item attributes.
//   GET/POST /api/v1/events/correlators, PUT/DELETE /api/v1/events/correlators/{id}
//   GET/POST /api/v1/events/maintenance-windows, DELETE /api/v1/events/maintenance-windows/{id}
//   GET/POST /api/v1/events/work-item-attributes, PUT/DELETE /api/v1/events/work-item-attributes/{id}
//   PUT /api/v1/events/work-items/{id}/attributes
export type EventCorrelator = components['schemas']['EventCorrelator'];
export type EventCorrelatorInput = components['schemas']['EventCorrelatorInput'];
export type EventMaintenanceWindow = components['schemas']['EventMaintenanceWindow'];
export type EventMaintenanceWindowInput = components['schemas']['EventMaintenanceWindowInput'];
export type WorkItemAttribute = components['schemas']['WorkItemAttribute'];
export type WorkItemAttributeInput = components['schemas']['WorkItemAttributeInput'];

export const listCorrelators = (): Promise<EventCorrelator[]> => (live ? get('/api/v1/events/correlators', {}) : settings.listCorrelators());
export const createCorrelator = (i: EventCorrelatorInput): Promise<EventCorrelator> => (live ? post('/api/v1/events/correlators', i) : settings.createCorrelator(i));
export const updateCorrelator = (id: string, i: EventCorrelatorInput): Promise<EventCorrelator> => (live ? put('/api/v1/events/correlators/{id}', { id }, i) : settings.updateCorrelator(id, i));
export const deleteCorrelator = (id: string): Promise<void> => (live ? del('/api/v1/events/correlators/{id}', { id }) : settings.deleteCorrelator(id));

export const listMaintenanceWindows = (): Promise<EventMaintenanceWindow[]> => (live ? get('/api/v1/events/maintenance-windows', {}) : settings.listWindows());
export const createMaintenanceWindow = (i: EventMaintenanceWindowInput): Promise<EventMaintenanceWindow> => (live ? post('/api/v1/events/maintenance-windows', i) : settings.createWindow(i));
export const deleteMaintenanceWindow = (id: string): Promise<void> => (live ? del('/api/v1/events/maintenance-windows/{id}', { id }) : settings.deleteWindow(id));

export const listWorkItemAttributes = (): Promise<WorkItemAttribute[]> => (live ? get('/api/v1/events/work-item-attributes', {}) : settings.listAttributes());
export const createWorkItemAttribute = (i: WorkItemAttributeInput): Promise<WorkItemAttribute> => (live ? post('/api/v1/events/work-item-attributes', i) : settings.createAttribute(i));
export const updateWorkItemAttribute = (id: string, i: WorkItemAttributeInput): Promise<WorkItemAttribute> =>
  live ? put('/api/v1/events/work-item-attributes/{id}', { id }, i) : settings.updateAttribute(id, i);
export const deleteWorkItemAttribute = (id: string): Promise<void> => (live ? del('/api/v1/events/work-item-attributes/{id}', { id }) : settings.deleteAttribute(id));

/** Sets one attribute of a work item ('' clears it). */
export const setWorkItemAttribute = (workItemId: string, attributeId: string, value: string): Promise<void> =>
  live ? put('/api/v1/events/work-items/{id}/attributes', { id: workItemId }, { attributeId, value }) : settings.setValue(workItemId, attributeId, value);
