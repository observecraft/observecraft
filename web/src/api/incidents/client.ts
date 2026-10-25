// Incident management data access (Postgres via the API). No in-browser mock.
import type { components } from '../generated/schema';
import { del, get, patch, post, postTo, put } from '../http';
import { requireLive } from '../identity/client';

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

export async function searchIncidents(req: IncidentSearchRequest) {
  requireLive();
  return post('/api/v1/incidents/search', req);
}

export async function getIncident(id: number) {
  requireLive();
  return get('/api/v1/incidents/{id}', { id: String(id) });
}

export async function declareIncident(req: DeclareRequest) {
  requireLive();
  return post('/api/v1/incidents', req);
}

export async function updateIncident(id: number, p: IncidentPatch) {
  requireLive();
  return patch('/api/v1/incidents/{id}', { id: String(id) }, p);
}

export async function getTimeline(id: number) {
  requireLive();
  return get('/api/v1/incidents/{id}/timeline', { id: String(id) });
}

export async function addNote(id: number, body: string) {
  requireLive();
  await postTo('/api/v1/incidents/{id}/timeline', { id: String(id) }, { body });
}

export async function addImpact(id: number, description: string, startAt: number) {
  requireLive();
  await postTo('/api/v1/incidents/{id}/impacts', { id: String(id) }, { description, startAt });
}

export async function addFollowup(id: number, title: string) {
  requireLive();
  await postTo('/api/v1/incidents/{id}/followups', { id: String(id) }, { title });
}

export async function setFollowupDone(id: number, followupId: number, done: boolean) {
  requireLive();
  await patch('/api/v1/incidents/{id}/followups/{followup_id}', { id: String(id), followup_id: String(followupId) }, { done });
}

const ir = (id: number) => ({ id: String(id) });

export async function setImportant(id: number, eventId: number, important: boolean) {
  requireLive();
  await patch('/api/v1/incidents/{id}/timeline/{event_id}', { id: String(id), event_id: String(eventId) }, { important });
}

export async function listResponders(id: number) {
  requireLive();
  return get('/api/v1/incidents/{id}/responders', ir(id));
}

export async function addResponder(id: number, userId: string, role: string) {
  requireLive();
  await postTo('/api/v1/incidents/{id}/responders', ir(id), { userId, role });
}

export async function removeResponder(id: number, userId: string) {
  requireLive();
  await del('/api/v1/incidents/{id}/responders/{user_id}', { id: String(id), user_id: userId });
}

export async function listPages(id: number) {
  requireLive();
  return get('/api/v1/incidents/{id}/pages', ir(id));
}

export async function createPage(id: number, team: string, title: string) {
  requireLive();
  return postTo('/api/v1/incidents/{id}/pages', ir(id), { team, title });
}

export async function updatePage(id: number, pageId: number, status: 'acknowledged' | 'resolved') {
  requireLive();
  await patch('/api/v1/incidents/{id}/pages/{page_id}', { id: String(id), page_id: String(pageId) }, { status });
}

export async function listNotifications(id: number) {
  requireLive();
  return get('/api/v1/incidents/{id}/notifications', ir(id));
}

export async function createNotification(id: number, recipients: string[], subject: string, body: string) {
  requireLive();
  await postTo('/api/v1/incidents/{id}/notifications', ir(id), { recipients, subject, body });
}

export async function listDocuments(id: number) {
  requireLive();
  return get('/api/v1/incidents/{id}/documents', ir(id));
}

export async function addDocument(id: number, title: string, url: string) {
  requireLive();
  await postTo('/api/v1/incidents/{id}/documents', ir(id), { title, url });
}

export async function removeDocument(id: number, docId: number) {
  requireLive();
  await del('/api/v1/incidents/{id}/documents/{doc_id}', { id: String(id), doc_id: String(docId) });
}

export async function getPostmortem(id: number) {
  requireLive();
  return get('/api/v1/incidents/{id}/postmortem', ir(id));
}

export async function generatePostmortem(id: number, template: 'standard' | 'brief') {
  requireLive();
  return postTo('/api/v1/incidents/{id}/postmortem', ir(id), { template });
}

export async function savePostmortem(id: number, content: string) {
  requireLive();
  await put('/api/v1/incidents/{id}/postmortem', ir(id), { content });
}
