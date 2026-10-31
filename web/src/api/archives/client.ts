// Log archives (S3-compatible buckets), Archive Search (DuckDB over archived Parquet) and rehydration.
import type { components } from '../generated/schema';
import { API_MODE, del, get, patch, post, postTo } from '../http';
import * as mock from '../mock/logsConfig';

type S = components['schemas'];
export type LogArchive = S['LogArchive'];
export type ArchiveTarget = S['ArchiveTarget'];
export type ArchiveRun = S['ArchiveRun'];
export type ArchiveSearch = S['ArchiveSearch'];
export type ArchiveSearchDetail = S['ArchiveSearchDetail'];
export type Rehydration = S['Rehydration'];
export type CreateArchive = S['CreateArchive'];
export type ArchivePatch = S['ArchivePatch'];

// Outside live mode the in-browser mock (mock/logsConfig.ts) answers archives and rehydrations; Archive Search needs the server.
const live = API_MODE === 'live';

export const listArchives = () => (live ? get('/api/v1/logs/archives', {}) : mock.listArchives());
export const testArchive = (t: ArchiveTarget) => (live ? post('/api/v1/logs/archives/test', t) : mock.testArchive(t));
export const createArchive = (a: CreateArchive) => (live ? post('/api/v1/logs/archives', a) : mock.createArchive(a));
export const updateArchive = (id: string, p: ArchivePatch) => (live ? patch('/api/v1/logs/archives/{id}', { id }, p) : mock.updateArchive(id, p));
export const deleteArchive = (id: string) => (live ? del('/api/v1/logs/archives/{id}', { id }) : mock.deleteArchive(id));
export const archiveRuns = (id: string) => (live ? get('/api/v1/logs/archives/{id}/runs', { id }) : mock.archiveRuns(id));
export const runArchive = (id: string) => postTo('/api/v1/logs/archives/{id}/run', { id }, undefined);

export const startArchiveSearch = (archiveId: string, query: string, range: { from: number; to: number }, maxResults = 1000) =>
  post('/api/v1/logs/archive-searches', { archiveId, query, range, maxResults });
export const listArchiveSearches = () => get('/api/v1/logs/archive-searches', {});
export const archiveSearch = (id: string, offset = 0, limit = 200) => get('/api/v1/logs/archive-searches/{id}', { id }, undefined, { offset, limit });
export const deleteArchiveSearch = (id: string) => del('/api/v1/logs/archive-searches/{id}', { id });

export const listRehydrations = () => (live ? get('/api/v1/logs/rehydrations', {}) : mock.listRehydrations());
export const rehydrate = (r: { archiveId: string; name: string; query: string; range: { from: number; to: number }; retentionDays: number }) =>
  live ? post('/api/v1/logs/rehydrations', r) : mock.rehydrate(r);
export const expireRehydration = (id: string) => (live ? del('/api/v1/logs/rehydrations/{id}', { id }) : mock.expireRehydration(id));
