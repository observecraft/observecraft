// Tagging policies, their coverage, and where resources come from (provenance). In "live" mode from the Rust API,
// otherwise the in-browser mock (mock/tagging.ts).
//   GET    /api/v1/tagging-policies            listTaggingPolicies
//   GET    /api/v1/tagging-policies/coverage   getTagCoverage
//   GET    /api/v1/tagging-policies/:id        getTaggingPolicy
//   POST   /api/v1/tagging-policies            createTaggingPolicy
//   PUT    /api/v1/tagging-policies/:id        updateTaggingPolicy
//   DELETE /api/v1/tagging-policies/:id        deleteTaggingPolicy
//   GET    /api/v1/provenance/resources        listResourceProvenance
import { API_MODE, del, get, post, put } from '../http';
import * as mock from '../mock/tagging';
import type { ProvenanceList, TagCoverage, TaggingPolicy, TaggingPolicyInput, TaggingPolicyList, TagTarget } from './types';

export * from './types';

const live = API_MODE === 'live';

export const listTaggingPolicies = (): Promise<TaggingPolicyList> => (live ? get('/api/v1/tagging-policies', {}) : mock.listPolicies());
export const getTagCoverage = (policy?: string): Promise<TagCoverage> => (live ? get('/api/v1/tagging-policies/coverage', {}, undefined, policy ? { policy } : undefined) : mock.coverage(policy));
export const getTaggingPolicy = (id: string): Promise<TaggingPolicy> => (live ? get('/api/v1/tagging-policies/{id}', { id }) : mock.getPolicy(id));
export const createTaggingPolicy = (input: TaggingPolicyInput): Promise<TaggingPolicy> => (live ? post('/api/v1/tagging-policies', input) : mock.createPolicy(input));
export const updateTaggingPolicy = (id: string, input: TaggingPolicyInput): Promise<TaggingPolicy> => (live ? put('/api/v1/tagging-policies/{id}', { id }, input) : mock.updatePolicy(id, input));
export const deleteTaggingPolicy = (id: string): Promise<void> => (live ? del('/api/v1/tagging-policies/{id}', { id }) : mock.deletePolicy(id));
export const listResourceProvenance = (target?: TagTarget, missing = false): Promise<ProvenanceList> => {
  const query: Record<string, string> = {};
  if (target) query.target = target;
  if (missing) query.missing = 'true';
  return live ? get('/api/v1/provenance/resources', {}, undefined, query) : mock.provenance(target, missing);
};
