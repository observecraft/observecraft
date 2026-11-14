// Remediation of security findings (ADR-0008): the level a finding gets (explain, fix in code, needs provenance),
// Assist runs that explain or fix it, and the merge request opened from a fix. In "live" mode the Rust API
// (routes/remediation.rs) answers; otherwise the in-browser mock (mock/remediation.ts).
//   POST /api/v1/remediation                             remediationOf
//   POST /api/v1/remediation/explain                     explainFinding     (an Assist run of kind explain)
//   POST /api/v1/remediation/fix                         fixFinding         (a manifest bump, or an Assist run of kind fix)
//   GET  /api/v1/remediation/fixes/{id}                  getFix
//   POST /api/v1/remediation/fixes/{id}/merge-request    openMergeRequest
import type { components } from '../generated/schema';
import type { RunSummary } from '../assist/types';
import { API_MODE, get, post, postTo } from '../http';
import * as mock from '../mock/remediation';

const live = API_MODE === 'live';
type S = components['schemas'];

export type FindingRef = S['FindingRef'];
export type Remediation = S['Remediation'];
export type RemediationLevel = S['RemediationLevel'];
export type FixView = S['FixView'];
export type FixTarget = S['FixTarget'];
export type MergeRequest = S['MergeRequest'];

export const remediationOf = (r: FindingRef, signal?: AbortSignal): Promise<Remediation> => (live ? post('/api/v1/remediation', r, signal) : mock.remediationOf(r));
export const explainFinding = (r: FindingRef): Promise<RunSummary> => (live ? post('/api/v1/remediation/explain', r) : mock.explainFinding(r));
export const fixFinding = (r: FindingRef): Promise<FixView> => (live ? post('/api/v1/remediation/fix', r) : mock.fixFinding(r));
export const getFix = (id: string, signal?: AbortSignal): Promise<FixView> => (live ? get('/api/v1/remediation/fixes/{id}', { id }, signal) : mock.getFix(id));
export const openMergeRequest = (id: string): Promise<FixView> => (live ? postTo('/api/v1/remediation/fixes/{id}/merge-request', { id }, undefined) : mock.openMergeRequest(id));
