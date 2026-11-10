// AI Testing (Synthetic Monitoring → AI Testing): jobs where Assist's testing agent explores a site in a browser and
// proposes browser tests, reviewed here. Jobs are Assist runs of kind `testing` (listed and read with the Assist
// runs client). In "live" mode the Rust API answers (routes/ai_testing.rs); otherwise mock/aiTesting.ts.
//   POST   /api/v1/synthetics/ai-testing/jobs                 startAiTestingJob
//   POST   /api/v1/synthetics/ai-testing/proposals/search     searchAiTestingProposals
//   POST   /api/v1/synthetics/ai-testing/proposals/accept     acceptAiTestingProposals
//   POST   /api/v1/synthetics/ai-testing/proposals/reject     rejectAiTestingProposals
//   GET    /api/v1/synthetics/ai-testing/profiles             listAiTestingProfiles
//   POST   /api/v1/synthetics/ai-testing/profiles             createAiTestingProfile
//   PUT    /api/v1/synthetics/ai-testing/profiles/:id         updateAiTestingProfile
//   DELETE /api/v1/synthetics/ai-testing/profiles/:id         deleteAiTestingProfile
import { API_MODE, del, get, post, put } from '../http';
import type { RunSummary } from '../assist/types';
import * as mock from '../mock/aiTesting';
import type { AiTestingAccepted, AiTestingDecision, AiTestingJobInput, AiTestingProfile, AiTestingProfileInput, AiTestingProposal, AiTestingProposalSearch } from './types';

export * from './types';

const live = API_MODE === 'live';

export const startAiTestingJob = (input: AiTestingJobInput): Promise<RunSummary> => (live ? post('/api/v1/synthetics/ai-testing/jobs', input) : mock.startAiTestingJob(input));
export const searchAiTestingProposals = (q: AiTestingProposalSearch, signal?: AbortSignal): Promise<AiTestingProposal[]> =>
  live ? post('/api/v1/synthetics/ai-testing/proposals/search', q, signal) : mock.searchAiTestingProposals(q);
export const acceptAiTestingProposals = (d: AiTestingDecision): Promise<AiTestingAccepted> => (live ? post('/api/v1/synthetics/ai-testing/proposals/accept', d) : mock.acceptAiTestingProposals(d));
export const rejectAiTestingProposals = (d: AiTestingDecision): Promise<void> => (live ? post('/api/v1/synthetics/ai-testing/proposals/reject', d) : mock.rejectAiTestingProposals(d));
export const listAiTestingProfiles = (): Promise<AiTestingProfile[]> => (live ? get('/api/v1/synthetics/ai-testing/profiles', {}) : mock.listAiTestingProfiles());
export const createAiTestingProfile = (input: AiTestingProfileInput): Promise<AiTestingProfile> => (live ? post('/api/v1/synthetics/ai-testing/profiles', input) : mock.createAiTestingProfile(input));
export const updateAiTestingProfile = (id: string, input: AiTestingProfileInput): Promise<AiTestingProfile> =>
  live ? put('/api/v1/synthetics/ai-testing/profiles/{id}', { id }, input) : mock.updateAiTestingProfile(id, input);
export const deleteAiTestingProfile = (id: string): Promise<void> => (live ? del('/api/v1/synthetics/ai-testing/profiles/{id}', { id }) : mock.deleteAiTestingProfile(id));
