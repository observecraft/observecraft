// Identity data access: profile, users, roles, teams and SAML group mappings (Postgres via the API).
// There is no in-browser mock; outside live mode calls fail with an explanatory error.
import type { components } from '../generated/schema';
import { API_MODE, get, patch, post } from '../http';

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

export function requireLive(): void {
  if (API_MODE !== 'live') throw new Error('This page reads organization data from the API. Start it and run the UI with VITE_API_MODE=live.');
}

export async function getProfile() {
  requireLive();
  return get('/api/v1/me', {});
}

export async function updateProfile(p: S['ProfilePatch']) {
  requireLive();
  return patch('/api/v1/me', {}, p);
}

export async function searchUsers(req: UserSearchRequest) {
  requireLive();
  return post('/api/v1/users/search', req);
}

export async function inviteUsers(emails: string[], roleIds: string[]) {
  requireLive();
  return post('/api/v1/users/invite', { emails, roleIds });
}

export async function updateUser(id: string, p: S['UserPatch']) {
  requireLive();
  await patch('/api/v1/users/{id}', { id }, p);
}

export async function listRoles() {
  requireLive();
  return get('/api/v1/roles', {});
}

export async function searchTeams(req: TeamSearchRequest) {
  requireLive();
  return post('/api/v1/teams/search', req);
}

export async function createTeam(req: S['CreateTeamRequest']) {
  requireLive();
  await post('/api/v1/teams', req);
}

export async function listMappings(kind: MappingKind) {
  requireLive();
  return get('/api/v1/saml/mappings', {}, undefined, { kind });
}

export async function createMapping(req: S['CreateMappingRequest']) {
  requireLive();
  await post('/api/v1/saml/mappings', req);
}
