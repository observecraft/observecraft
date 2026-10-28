// Infrastructure → Resources → Policies: rules on the resource catalog and their compliance. In "live"
// mode they come from the Rust API; otherwise the in-browser mock answers (mock/policies.ts).
//   POST   /api/v1/infrastructure/policies/search       searchPolicies
//   GET    /api/v1/infrastructure/policies/attributes   policyAttributes
//   POST   /api/v1/infrastructure/policies/preview      previewPolicy
//   GET    /api/v1/infrastructure/policies/:id          getPolicy
//   POST   /api/v1/infrastructure/policies              createPolicy
//   PUT    /api/v1/infrastructure/policies/:id          updatePolicy
//   DELETE /api/v1/infrastructure/policies/:id          deletePolicy
import { API_MODE, del, get, post, put } from '../http';
import * as mock from '../mock/policies';
import type { ResourcePolicy, ResourcePolicyDetail, ResourcePolicyInput, ResourcePolicyList, ResourcePolicyPreview, ResourcePolicyPreviewRequest, ResourcePolicyTypeAttributes } from './types';

export * from './types';

const live = API_MODE === 'live';

/** Policies with their compliance on the resources matching `query` (the page's filters), and the templates. */
export const searchPolicies = (query: string, signal?: AbortSignal): Promise<ResourcePolicyList> =>
  live ? post('/api/v1/infrastructure/policies/search', { query }, signal) : mock.searchPolicies(query);
export const policyAttributes = (signal?: AbortSignal): Promise<ResourcePolicyTypeAttributes[]> => (live ? get('/api/v1/infrastructure/policies/attributes', {}, signal) : mock.policyAttributes());
export const previewPolicy = (req: ResourcePolicyPreviewRequest, signal?: AbortSignal): Promise<ResourcePolicyPreview> =>
  live ? post('/api/v1/infrastructure/policies/preview', req, signal) : mock.previewPolicy(req);
export const getPolicy = (id: string, query: string, signal?: AbortSignal): Promise<ResourcePolicyDetail> =>
  live ? get('/api/v1/infrastructure/policies/{id}', { id }, signal, query ? { query } : undefined) : mock.getPolicy(id, query);
export const createPolicy = (input: ResourcePolicyInput): Promise<ResourcePolicy> => (live ? post('/api/v1/infrastructure/policies', input) : mock.createPolicy(input));
export const updatePolicy = (id: string, input: ResourcePolicyInput): Promise<ResourcePolicy> => (live ? put('/api/v1/infrastructure/policies/{id}', { id }, input) : mock.updatePolicy(id, input));
export const deletePolicy = (id: string): Promise<void> => (live ? del('/api/v1/infrastructure/policies/{id}', { id }) : mock.deletePolicy(id));
