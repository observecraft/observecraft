// Log indexes (routing, retention, quotas, exclusion filters) and the org's data retention settings.
// Outside live mode the in-browser mock (mock/logIndexes.ts) answers, with in-memory state for the session.
import type { components } from '../generated/schema';
import { API_MODE, del, get, patch, post, put } from '../http';
import * as mock from '../mock/logIndexes';

type S = components['schemas'];
export type LogIndex = S['LogIndex'];
export type LogIndexInput = S['LogIndexInput'];
export type ExclusionFilter = S['ExclusionFilter'];
export type OrgDataSettings = S['OrgDataSettings'];
export type MetricRetention = S['MetricRetention'];

const live = API_MODE === 'live';

export const listIndexes = () => (live ? get('/api/v1/logs/indexes', {}) : mock.listIndexes());
export const createIndex = (i: LogIndexInput) => (live ? post('/api/v1/logs/indexes', i) : mock.createIndex(i));
export const updateIndex = (id: string, i: LogIndexInput) => (live ? put('/api/v1/logs/indexes/{id}', { id }, i) : mock.updateIndex(id, i));
export const deleteIndex = (id: string) => (live ? del('/api/v1/logs/indexes/{id}', { id }) : mock.deleteIndex(id));
export const reorderIndexes = (ids: string[]) => (live ? post('/api/v1/logs/indexes/reorder', { ids }) : mock.reorderIndexes(ids));
export const dataSettings = () => (live ? get('/api/v1/org/data-settings', {}) : mock.dataSettings());
export const updateDataSettings = (p: { spanRetentionDays?: number; metrics?: MetricRetention }) => (live ? patch('/api/v1/org/data-settings', {}, p) : mock.updateDataSettings(p));
