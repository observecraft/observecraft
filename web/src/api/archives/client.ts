// Log archives (S3-compatible buckets), Archive Search (DuckDB over archived Parquet) and rehydration.
import type { components } from '../generated/schema';
import { del, get, patch, post, postTo } from '../http';

type S = components['schemas'];
export type LogArchive = S['LogArchive'];
export type ArchiveTarget = S['ArchiveTarget'];
export type ArchiveRun = S['ArchiveRun'];
export type ArchiveSearch = S['ArchiveSearch'];
export type ArchiveSearchDetail = S['ArchiveSearchDetail'];
export type Rehydration = S['Rehydration'];
export type CreateArchive = S['CreateArchive'];

export const listArchives = () => get('/api/v1/logs/archives', {});
export const testArchive = (t: ArchiveTarget) => post('/api/v1/logs/archives/test', t);
export const createArchive = (a: CreateArchive) => post('/api/v1/logs/archives', a);
export const updateArchive = (id: string, p: { name?: string; enabled?: boolean; query?: string }) => patch('/api/v1/logs/archives/{id}', { id }, p);
export const deleteArchive = (id: string) => del('/api/v1/logs/archives/{id}', { id });
export const archiveRuns = (id: string) => get('/api/v1/logs/archives/{id}/runs', { id });
export const runArchive = (id: string) => postTo('/api/v1/logs/archives/{id}/run', { id }, undefined);

export const startArchiveSearch = (archiveId: string, query: string, range: { from: number; to: number }, maxResults = 1000) =>
  post('/api/v1/logs/archive-searches', { archiveId, query, range, maxResults });
export const listArchiveSearches = () => get('/api/v1/logs/archive-searches', {});
export const archiveSearch = (id: string, offset = 0, limit = 200) => get('/api/v1/logs/archive-searches/{id}', { id }, undefined, { offset, limit });
export const deleteArchiveSearch = (id: string) => del('/api/v1/logs/archive-searches/{id}', { id });

export const listRehydrations = () => get('/api/v1/logs/rehydrations', {});
export const rehydrate = (r: { archiveId: string; name: string; query: string; range: { from: number; to: number }; retentionDays: number }) => post('/api/v1/logs/rehydrations', r);
export const expireRehydration = (id: string) => del('/api/v1/logs/rehydrations/{id}', { id });
