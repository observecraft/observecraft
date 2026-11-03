// Assist (AI chat). In "live" mode the Rust API (routes/assist.rs) answers with the install's model;
// otherwise the in-browser mock (mock/assist.ts) plays scripted replies over the demo data.
//   GET    /api/v1/assist/status                       getAssistStatus
//   GET    /api/v1/assist/settings  (+ PUT)            getAssistSettings / updateAssistSettings
//   GET    /api/v1/assist/conversations                listConversations
//   GET    /api/v1/assist/conversations/:id (+ DELETE) getConversation / deleteConversation
//   POST   /api/v1/assist/chat                         sendMessage (Server-Sent Events)
//   POST   /api/v1/assist/conversations/:id/approve    approveActions (Server-Sent Events)
//   POST   /api/v1/assist/reports                      assistReport
import { API_MODE, del, get, post, postStream, postStreamTo, put } from '../http';
import * as mock from '../mock/assist';
import type { AssistEvent, AssistReport, AssistReportRequest, AssistSettings, AssistStatus, ChatRequest, ConversationSummary, ConversationView } from './types';

export * from './types';

const live = API_MODE === 'live';

export const getAssistStatus = (): Promise<AssistStatus> => (live ? get('/api/v1/assist/status', {}) : mock.status());
export const getAssistSettings = (): Promise<AssistSettings> => (live ? get('/api/v1/assist/settings', {}) : mock.getSettings());
export const updateAssistSettings = (s: AssistSettings): Promise<void> => (live ? put('/api/v1/assist/settings', {}, s) : mock.updateSettings(s));
export const listConversations = (): Promise<ConversationSummary[]> => (live ? get('/api/v1/assist/conversations', {}) : mock.listConversations());
export const getConversation = (id: string, signal?: AbortSignal): Promise<ConversationView> => (live ? get('/api/v1/assist/conversations/{id}', { id }, signal) : mock.getConversation(id));
export const deleteConversation = (id: string): Promise<void> => (live ? del('/api/v1/assist/conversations/{id}', { id }) : mock.deleteConversation(id));
export const assistReport = (req: AssistReportRequest, signal?: AbortSignal): Promise<AssistReport> => (live ? post('/api/v1/assist/reports', req, signal) : mock.report(req));

/** Sends a message; `onEvent` gets the reply as it streams (conversation, text, tool_start/end, approval, error, done). */
export async function sendMessage(req: ChatRequest, onEvent: (e: AssistEvent) => void, signal: AbortSignal): Promise<void> {
  if (!live) return mock.chat(req, onEvent, signal);
  await postStream('/api/v1/assist/chat', req, (_, e) => onEvent(e), signal);
}

/** Runs the approved pending actions (the others are declined) and streams the rest of the reply. */
export async function approveActions(id: string, approved: string[], autoApprove: boolean, onEvent: (e: AssistEvent) => void, signal: AbortSignal): Promise<void> {
  if (!live) return mock.approve(id, approved, onEvent, signal);
  await postStreamTo('/api/v1/assist/conversations/{id}/approve', { id }, { approved, autoApprove }, (_, e) => onEvent(e), signal);
}
