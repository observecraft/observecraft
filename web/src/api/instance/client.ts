// Instance administration: organizations of this install (instance admins only).
import type { components } from '../generated/schema';
import { get, patch, post } from '../http';

type S = components['schemas'];
export type InstanceOrg = S['InstanceOrg'];
export type InstanceAuditEvent = S['InstanceAuditEvent'];

export const listOrgs = () => get('/api/v1/instance/orgs', {});
export const createOrg = (id: string, name: string, adminEmail: string) => post('/api/v1/instance/orgs', { id, name, adminEmail });
export const updateOrg = (id: string, p: { name?: string; status?: 'active' | 'suspended' }) => patch('/api/v1/instance/orgs/{id}', { id }, p);
export const instanceAudit = () => get('/api/v1/instance/audit', {});
export const updateOrgLimits = (id: string, l: { maxLogRetentionDays: number; maxSpanRetentionDays: number; maxIndexes: number; maxMetrics?: components['schemas']['MetricRetention'] }) =>
  patch('/api/v1/instance/orgs/{id}/limits', { id }, l);

/** Deletion takes a confirmation body, which the typed `del` helper doesn't send. */
export async function deleteOrg(id: string): Promise<void> {
  const res = await fetch(`/api/v1/instance/orgs/${encodeURIComponent(id)}`, {
    method: 'DELETE',
    credentials: 'same-origin',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ confirm: id }),
  });
  if (!res.ok) {
    const err = (await res.json().catch(() => ({}))) as { error?: string };
    throw new Error(err.error ?? `Request failed (${res.status})`);
  }
}
