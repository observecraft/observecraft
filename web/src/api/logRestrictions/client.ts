// Log data access (Logs → Configuration → Data Access): restriction queries and their role and team
// assignments, managed by org admins. Outside live mode the in-browser mock (mock/logRestrictions.ts) answers.
import { API_MODE, del, get, post, postTo, put } from '../http';
import * as mock from '../mock/logRestrictions';
import type { LogRestrictionQuery, RestrictionPreview, RestrictionQueryInput } from './types';

export * from './types';

const live = API_MODE === 'live';

export const listRestrictionQueries = (): Promise<LogRestrictionQuery[]> => (live ? get('/api/v1/logs/restriction-queries', {}) : mock.listRestrictionQueries());
export const createRestrictionQuery = (i: RestrictionQueryInput): Promise<LogRestrictionQuery> => (live ? post('/api/v1/logs/restriction-queries', i) : mock.createRestrictionQuery(i));
export const updateRestrictionQuery = (id: string, i: RestrictionQueryInput): Promise<LogRestrictionQuery> =>
  live ? put('/api/v1/logs/restriction-queries/{id}', { id }, i) : mock.updateRestrictionQuery(id, i);
export const deleteRestrictionQuery = (id: string): Promise<void> => (live ? del('/api/v1/logs/restriction-queries/{id}', { id }) : mock.deleteRestrictionQuery(id));
export const assignRole = (id: string, roleId: string): Promise<LogRestrictionQuery> =>
  live ? postTo('/api/v1/logs/restriction-queries/{id}/roles', { id }, { roleId }) : mock.assignRole(id, roleId);
export const unassignRole = async (id: string, roleId: string): Promise<void> => {
  if (!live) return void (await mock.unassignRole(id, roleId));
  await del('/api/v1/logs/restriction-queries/{id}/roles/{role_id}', { id, role_id: roleId });
};
export const assignTeam = (id: string, handle: string): Promise<LogRestrictionQuery> =>
  live ? postTo('/api/v1/logs/restriction-queries/{id}/teams', { id }, { handle }) : mock.assignTeam(id, handle);
export const unassignTeam = async (id: string, handle: string): Promise<void> => {
  if (!live) return void (await mock.unassignTeam(id, handle));
  await del('/api/v1/logs/restriction-queries/{id}/teams/{handle}', { id, handle });
};
/** Checks a query and counts the logs of the last hour it matches. */
export const previewRestrictionQuery = (query: string): Promise<RestrictionPreview> => (live ? post('/api/v1/logs/restriction-queries/preview', { query }) : mock.previewRestrictionQuery(query));
