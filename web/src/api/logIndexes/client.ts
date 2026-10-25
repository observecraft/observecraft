// Log indexes (routing, retention, quotas, exclusion filters) and the org's data retention settings.
import type { components } from '../generated/schema';
import { del, get, patch, post, put } from '../http';

type S = components['schemas'];
export type LogIndex = S['LogIndex'];
export type LogIndexInput = S['LogIndexInput'];
export type ExclusionFilter = S['ExclusionFilter'];
export type OrgDataSettings = S['OrgDataSettings'];

export const listIndexes = () => get('/api/v1/logs/indexes', {});
export const createIndex = (i: LogIndexInput) => post('/api/v1/logs/indexes', i);
export const updateIndex = (id: string, i: LogIndexInput) => put('/api/v1/logs/indexes/{id}', { id }, i);
export const deleteIndex = (id: string) => del('/api/v1/logs/indexes/{id}', { id });
export const reorderIndexes = (ids: string[]) => post('/api/v1/logs/indexes/reorder', { ids });
export const dataSettings = () => get('/api/v1/org/data-settings', {});
export const updateDataSettings = (spanRetentionDays: number) => patch('/api/v1/org/data-settings', {}, { spanRetentionDays });
