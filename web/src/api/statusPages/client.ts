// Status pages (Incident Response → Status Pages): components, notices and the read-only rendering at
// /status/<slug>. Outside live mode the in-browser mock (mock/statusPages.ts) answers.
import { API_MODE, del, get, post, postTo, put } from '../http';
import * as mock from '../mock/statusPages';
import type { NoticeInput, NoticeUpdateInput, PublicStatusPage, StatusPageDetail, StatusPageInput, StatusPageSummary } from './types';

export * from './types';

const live = API_MODE === 'live';

export const listStatusPages = (): Promise<StatusPageSummary[]> => (live ? get('/api/v1/status-pages', {}) : mock.listStatusPages());
export const createStatusPage = (i: StatusPageInput): Promise<StatusPageDetail> => (live ? post('/api/v1/status-pages', i) : mock.createStatusPage(i));
export const getStatusPage = (id: string, signal?: AbortSignal): Promise<StatusPageDetail> => (live ? get('/api/v1/status-pages/{id}', { id }, signal) : mock.getStatusPage(id));
export const updateStatusPage = (id: string, i: StatusPageInput): Promise<StatusPageDetail> => (live ? put('/api/v1/status-pages/{id}', { id }, i) : mock.updateStatusPage(id, i));
export const deleteStatusPage = (id: string): Promise<void> => (live ? del('/api/v1/status-pages/{id}', { id }) : mock.deleteStatusPage(id));

/** Opens a notice with its first update. */
export const createStatusPageNotice = (id: string, i: NoticeInput): Promise<StatusPageDetail> => (live ? postTo('/api/v1/status-pages/{id}/notices', { id }, i) : mock.createStatusPageNotice(id, i));
/** Posts an update to an open notice; status `resolved` resolves it. */
export const updateStatusPageNotice = (id: string, noticeId: string, i: NoticeUpdateInput): Promise<StatusPageDetail> =>
  live ? postTo('/api/v1/status-pages/{id}/notices/{notice_id}/updates', { id, notice_id: noticeId }, i) : mock.updateStatusPageNotice(id, noticeId, i);
export const deleteStatusPageNotice = (id: string, noticeId: string): Promise<void> =>
  live ? del('/api/v1/status-pages/{id}/notices/{notice_id}', { id, notice_id: noticeId }) : mock.deleteStatusPageNotice(id, noticeId);

/** The read-only page at /status/<slug>; needs no sign-in for a public page. */
export const getPublicStatusPage = (slug: string, signal?: AbortSignal): Promise<PublicStatusPage> =>
  live ? get('/api/v1/public/status-pages/{slug}', { slug }, signal) : mock.getPublicStatusPage(slug);
