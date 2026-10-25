// Sign-in, sessions, MFA, application keys and login-method administration.
import type { components } from '../generated/schema';
import { get, post, postTo } from '../http';

type S = components['schemas'];
export type LoginConfig = S['LoginConfig'];
export type SessionInfo = S['SessionInfo'];
export type AppKey = S['AppKey'];
export type AuthSettingsView = S['AuthSettingsView'];
export type AuthSettingsUpdate = S['AuthSettingsUpdate'];
export type AuditEvent = S['AuditEvent'];
export type SecurityStatus = S['SecurityStatus'];

export const loginConfig = () => get('/api/v1/auth/config', {});
export const login = (email: string, password: string) => post('/api/v1/auth/login', { email, password });
export const verifyMfa = (code: string) => post('/api/v1/auth/mfa/verify', { code });
export const logout = () => post('/api/v1/auth/logout', undefined as never);
export const discoverSso = (email: string) => post('/api/v1/auth/sso/discover', { email });
export const linkInfo = (token: string) => get('/api/v1/auth/link/{token}', { token });
export const acceptLink = (token: string, password: string, name?: string) => postTo('/api/v1/auth/link/{token}', { token }, { password, name });

export const logoutAll = () => post('/api/v1/me/logout-all', undefined as never);
export const securityStatus = () => get('/api/v1/me/security', {});
export const changePassword = (newPassword: string, currentPassword?: string) => post('/api/v1/me/password', { newPassword, currentPassword });
export const listSessions = () => get('/api/v1/me/sessions', {});
export const revokeSession = (id: string) => request('DELETE', `/api/v1/me/sessions/${encodeURIComponent(id)}`);
export const mfaSetup = () => post('/api/v1/me/mfa/setup', undefined as never);
export const mfaEnable = (code: string) => post('/api/v1/me/mfa/enable', { code });
export const mfaDisable = (code: string) => post('/api/v1/me/mfa/disable', { code });
export const listAppKeys = () => get('/api/v1/me/app-keys', {});
export const createAppKey = (name: string) => post('/api/v1/me/app-keys', { name });
export const revokeAppKey = (id: string) => request('DELETE', `/api/v1/me/app-keys/${encodeURIComponent(id)}`);

export const getLoginMethods = () => get('/api/v1/org/login-methods', {});
export const updateLoginMethods = (u: AuthSettingsUpdate) => put('/api/v1/org/login-methods', u);
export const loginLink = (userId: string) => postTo('/api/v1/users/{id}/login-link', { id: userId }, undefined);
export const auditTrail = (query: string, offset: number, limit: number) => get('/api/v1/audit', {}, undefined, { query, offset, limit });
export const deleteMapping = (id: number) => request('DELETE', `/api/v1/saml/mappings/${id}`);

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

