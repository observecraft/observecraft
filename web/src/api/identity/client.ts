// Identity data access: profile, users, roles, teams and SAML group mappings (Postgres via the API).
// Outside live mode the in-browser mock (mock/identity.ts) answers, with in-memory state for the session.
import type { components } from '../generated/schema';
import { API_MODE, get, patch, post } from '../http';
import * as mock from '../mock/identity';

type S = components['schemas'];
export type Profile = S['Profile'];
export type UserSummary = S['UserSummary'];
export type UserStatus = S['UserStatus'];
export type UserSearchRequest = S['UserSearchRequest'];
export type Role = S['Role'];
export type Team = S['Team'];
export type TeamSearchRequest = S['TeamSearchRequest'];
export type SamlMapping = S['SamlMapping'];
export type MappingKind = S['MappingKind'];

const live = API_MODE === 'live';

/** For clients without an in-browser mock yet: fails outside live mode with an explanation. */
export function requireLive(): void {
  if (!live) throw new Error('This page reads organization data from the API. Start it and run the UI with VITE_API_MODE=live.');
}

export async function getProfile() {
  if (!live) return mock.getProfile();
  return get('/api/v1/me', {});
}

export async function updateProfile(p: S['ProfilePatch']) {
  if (!live) return mock.updateProfile(p);
  return patch('/api/v1/me', {}, p);
}

export async function searchUsers(req: UserSearchRequest) {
  if (!live) return mock.searchUsers(req);
  return post('/api/v1/users/search', req);
}

export async function inviteUsers(emails: string[], roleIds: string[]) {
  if (!live) return mock.inviteUsers(emails, roleIds);
  return post('/api/v1/users/invite', { emails, roleIds });
}

export async function updateUser(id: string, p: S['UserPatch']) {
  if (!live) return mock.updateUser(id, p);
  await patch('/api/v1/users/{id}', { id }, p);
}

export async function listRoles() {
  if (!live) return mock.listRoles();
  return get('/api/v1/roles', {});
}

export async function searchTeams(req: TeamSearchRequest) {
  if (!live) return mock.searchTeams(req);
  return post('/api/v1/teams/search', req);
}

export async function createTeam(req: S['CreateTeamRequest']) {
  if (!live) return mock.createTeam(req);
  await post('/api/v1/teams', req);
}

export async function listMappings(kind: MappingKind) {
  if (!live) return mock.listMappings(kind);
  return get('/api/v1/saml/mappings', {}, undefined, { kind });
}

export async function createMapping(req: S['CreateMappingRequest']) {
  if (!live) return mock.createMapping(req);
  await post('/api/v1/saml/mappings', req);
}
