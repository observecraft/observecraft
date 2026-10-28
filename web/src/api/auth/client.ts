// Sign-in, sessions, MFA, application keys and login-method administration.
// Outside live mode the account and organization calls (Personal and Organization Settings) are
// answered by the in-browser mock (mock/identity.ts); signing in itself always needs the API.
import type { components } from '../generated/schema';
import { API_MODE, get, post, postTo } from '../http';
import * as mock from '../mock/identity';

const live = API_MODE === 'live';
const done = () => Promise.resolve();

type S = components['schemas'];
export type LoginConfig = S['LoginConfig'];
export type SessionInfo = S['SessionInfo'];
export type AppKey = S['AppKey'];
export type AuthSettingsView = S['AuthSettingsView'];
export type AuthSettingsUpdate = S['AuthSettingsUpdate'];
export type AuditEvent = S['AuditEvent'];
export type SecurityStatus = S['SecurityStatus'];
export type MyOrg = S['MyOrg'];

export const loginConfig = (org?: string) => get('/api/v1/auth/config', {}, undefined, org ? { org } : undefined);
export const login = (email: string, password: string, org?: string) => post('/api/v1/auth/login', { email, password, org: org ?? null });
export const verifyMfa = (code: string) => post('/api/v1/auth/mfa/verify', { code });
export const logout = () => (live ? post('/api/v1/auth/logout', undefined as never) : done());
export const discoverSso = (email: string) => post('/api/v1/auth/sso/discover', { email });
export const linkInfo = (token: string) => get('/api/v1/auth/link/{token}', { token });
export const acceptLink = (token: string, password: string, name?: string) => postTo('/api/v1/auth/link/{token}', { token }, { password, name });

export const logoutAll = () => (live ? post('/api/v1/me/logout-all', undefined as never) : done());
export const securityStatus = () => (live ? get('/api/v1/me/security', {}) : mock.securityStatus());
export const changePassword = (newPassword: string, currentPassword?: string) =>
  live ? post('/api/v1/me/password', { newPassword, currentPassword }) : mock.changePassword(newPassword, currentPassword);
export const listSessions = () => (live ? get('/api/v1/me/sessions', {}) : mock.listSessions());
export const revokeSession = (id: string) => (live ? request('DELETE', `/api/v1/me/sessions/${encodeURIComponent(id)}`) : mock.revokeSession(id));
export const mfaSetup = () => (live ? post('/api/v1/me/mfa/setup', undefined as never) : mock.mfaSetup());
export const mfaEnable = (code: string) => (live ? post('/api/v1/me/mfa/enable', { code }) : mock.mfaEnable(code));
export const mfaDisable = (code: string) => (live ? post('/api/v1/me/mfa/disable', { code }) : mock.mfaDisable(code));
export const myOrgs = () => (live ? get('/api/v1/me/orgs', {}) : mock.myOrgs());
export const switchOrg = (org: string) => (live ? post('/api/v1/me/switch-org', { org }) : done());
export const listAppKeys = () => (live ? get('/api/v1/me/app-keys', {}) : mock.listAppKeys());
export const createAppKey = (name: string) => (live ? post('/api/v1/me/app-keys', { name }) : mock.createAppKey(name));
export const revokeAppKey = (id: string) => (live ? request('DELETE', `/api/v1/me/app-keys/${encodeURIComponent(id)}`) : mock.revokeAppKey(id));

export const getLoginMethods = () => (live ? get('/api/v1/org/login-methods', {}) : mock.getLoginMethods());
export const updateLoginMethods = (u: AuthSettingsUpdate) => (live ? put('/api/v1/org/login-methods', u) : mock.updateLoginMethods(u));
export const loginLink = (userId: string) => (live ? postTo('/api/v1/users/{id}/login-link', { id: userId }, undefined) : mock.loginLink(userId));
export const auditTrail = (query: string, offset: number, limit: number) => (live ? get('/api/v1/audit', {}, undefined, { query, offset, limit }) : mock.auditTrail(query, offset, limit));
export const deleteMapping = (id: number) => (live ? request('DELETE', `/api/v1/saml/mappings/${id}`) : mock.deleteMapping(id));

// DELETE / PUT aren't in the typed helpers; these endpoints have simple bodies.
async function request(method: string, url: string, body?: unknown): Promise<void> {
  const res = await fetch(url, { method, credentials: 'same-origin', headers: body ? { 'content-type': 'application/json' } : undefined, body: body ? JSON.stringify(body) : undefined });
  if (!res.ok) {
    const err = (await res.json().catch(() => ({}))) as { error?: string };
    throw new Error(err.error ?? `Request failed (${res.status})`);
  }
}

async function put(url: string, body: AuthSettingsUpdate): Promise<AuthSettingsView> {
  const res = await fetch(url, { method: 'PUT', credentials: 'same-origin', headers: { 'content-type': 'application/json' }, body: JSON.stringify(body) });
  const json = (await res.json().catch(() => ({}))) as AuthSettingsView & { error?: string };
  if (!res.ok) throw new Error(json.error ?? `Request failed (${res.status})`);
  return json;
}
