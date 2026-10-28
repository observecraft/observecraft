// Incident management data access (Postgres via the API). Outside live mode the in-browser mock
// (mock/incidents.ts) answers, with in-memory state for the session.
import type { components } from '../generated/schema';
import { API_MODE, del, get, patch, post, postTo, put } from '../http';
import * as mock from '../mock/incidents';

type S = components['schemas'];
export type IncidentSummary = S['IncidentSummary'];
export type IncidentDetail = S['IncidentDetail'];
export type IncidentStatus = S['IncidentStatus'];
export type IncidentSearchRequest = S['IncidentSearchRequest'];
export type TimelineEvent = S['TimelineEvent'];
export type DeclareRequest = S['DeclareRequest'];
export type IncidentPatch = S['IncidentPatch'];
export type TeamMember = S['TeamMember'];
export type Page = S['Page'];
export type IncidentNotification = S['Notification'];
export type IncidentDocument = S['Document'];
export type Postmortem = S['Postmortem'];

export const SEVERITIES = ['SEV-1', 'SEV-2', 'SEV-3', 'SEV-4', 'SEV-5', 'UNKNOWN'] as const;
export const SEVERITY_LABEL: Record<string, string> = { 'SEV-1': 'Critical', 'SEV-2': 'High', 'SEV-3': 'Moderate', 'SEV-4': 'Low', 'SEV-5': 'Minor', UNKNOWN: 'Unknown' };

const live = API_MODE === 'live';
const ir = (id: number) => ({ id: String(id) });

export async function searchIncidents(req: IncidentSearchRequest) {
  if (!live) return mock.searchIncidents(req);
  return post('/api/v1/incidents/search', req);
}

export async function getIncident(id: number) {
  if (!live) return mock.getIncident(id);
  return get('/api/v1/incidents/{id}', ir(id));
}

export async function declareIncident(req: DeclareRequest) {
  if (!live) return mock.declareIncident(req);
  return post('/api/v1/incidents', req);
}

export async function updateIncident(id: number, p: IncidentPatch) {
  if (!live) return mock.updateIncident(id, p);
  return patch('/api/v1/incidents/{id}', ir(id), p);
}

export async function getTimeline(id: number) {
  if (!live) return mock.getTimeline(id);
  return get('/api/v1/incidents/{id}/timeline', ir(id));
}

export async function addNote(id: number, body: string) {
  if (!live) return mock.addNote(id, body);
  await postTo('/api/v1/incidents/{id}/timeline', ir(id), { body });
}

export async function addImpact(id: number, description: string, startAt: number) {
  if (!live) return mock.addImpact(id, description, startAt);
  await postTo('/api/v1/incidents/{id}/impacts', ir(id), { description, startAt });
}

export async function addFollowup(id: number, title: string) {
  if (!live) return mock.addFollowup(id, title);
  await postTo('/api/v1/incidents/{id}/followups', ir(id), { title });
}

export async function setFollowupDone(id: number, followupId: number, done: boolean) {
  if (!live) return mock.setFollowupDone(id, followupId, done);
  await patch('/api/v1/incidents/{id}/followups/{followup_id}', { id: String(id), followup_id: String(followupId) }, { done });
}

export async function setImportant(id: number, eventId: number, important: boolean) {
  if (!live) return mock.setImportant(id, eventId, important);
  await patch('/api/v1/incidents/{id}/timeline/{event_id}', { id: String(id), event_id: String(eventId) }, { important });
}

export async function listResponders(id: number) {
  if (!live) return mock.listResponders(id);
  return get('/api/v1/incidents/{id}/responders', ir(id));
}

export async function addResponder(id: number, userId: string, role: string) {
  if (!live) return mock.addResponder(id, userId, role);
  await postTo('/api/v1/incidents/{id}/responders', ir(id), { userId, role });
}

export async function removeResponder(id: number, userId: string) {
  if (!live) return mock.removeResponder(id, userId);
  await del('/api/v1/incidents/{id}/responders/{user_id}', { id: String(id), user_id: userId });
}

export async function listPages(id: number) {
  if (!live) return mock.listPages(id);
  return get('/api/v1/incidents/{id}/pages', ir(id));
}

export async function createPage(id: number, team: string, title: string) {
  if (!live) return mock.createPage(id, team, title);
  return postTo('/api/v1/incidents/{id}/pages', ir(id), { team, title });
}

export async function updatePage(id: number, pageId: number, status: 'acknowledged' | 'resolved') {
  if (!live) return mock.updatePage(id, pageId, status);
  await patch('/api/v1/incidents/{id}/pages/{page_id}', { id: String(id), page_id: String(pageId) }, { status });
}

export async function listNotifications(id: number) {
  if (!live) return mock.listNotifications(id);
  return get('/api/v1/incidents/{id}/notifications', ir(id));
}

export async function createNotification(id: number, recipients: string[], subject: string, body: string) {
  if (!live) return mock.createNotification(id, recipients, subject, body);
  await postTo('/api/v1/incidents/{id}/notifications', ir(id), { recipients, subject, body });
}

export async function listDocuments(id: number) {
  if (!live) return mock.listDocuments(id);
  return get('/api/v1/incidents/{id}/documents', ir(id));
}

export async function addDocument(id: number, title: string, url: string) {
  if (!live) return mock.addDocument(id, title, url);
  await postTo('/api/v1/incidents/{id}/documents', ir(id), { title, url });
}

export async function removeDocument(id: number, docId: number) {
  if (!live) return mock.removeDocument(id, docId);
  await del('/api/v1/incidents/{id}/documents/{doc_id}', { id: String(id), doc_id: String(docId) });
}

export async function getPostmortem(id: number) {
  if (!live) return mock.getPostmortem(id);
  return get('/api/v1/incidents/{id}/postmortem', ir(id));
}

export async function generatePostmortem(id: number, template: 'standard' | 'brief') {
  if (!live) return mock.generatePostmortem(id, template);
  return postTo('/api/v1/incidents/{id}/postmortem', ir(id), { template });
}

export async function savePostmortem(id: number, content: string) {
  if (!live) return mock.savePostmortem(id, content);
  await put('/api/v1/incidents/{id}/postmortem', ir(id), { content });
}
