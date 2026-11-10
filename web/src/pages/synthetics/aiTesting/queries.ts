// TanStack Query bindings for AI Testing: jobs are Assist runs of kind `testing`, read with the Assist runs hooks.
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import * as api from '../../../api/aiTesting/client';
import type { AiTestingJobInput, AiTestingProfileInput, AiTestingProposalSearch } from '../../../api/aiTesting/client';
import { syntheticKeys } from '../queries';

const K = ['ai-testing'] as const;

/** Proposals; polled while a job they belong to may still add some. */
export const useProposals = (q: AiTestingProposalSearch, poll = false) =>
  useQuery({ queryKey: [...K, 'proposals', q], queryFn: ({ signal }) => api.searchAiTestingProposals(q, signal), refetchInterval: poll ? 2000 : false });

export const useProfiles = () => useQuery({ queryKey: [...K, 'profiles'], queryFn: api.listAiTestingProfiles });

export function useStartJob() {
  const qc = useQueryClient();
  return useMutation({ mutationFn: (input: AiTestingJobInput) => api.startAiTestingJob(input), onSuccess: () => qc.invalidateQueries({ queryKey: ['assist-runs'] }) });
}

/** Accepting creates tests and suites: their lists refresh too. */
export function useDecide() {
  const qc = useQueryClient();
  const done = () => Promise.all([qc.invalidateQueries({ queryKey: K }), qc.invalidateQueries({ queryKey: syntheticKeys.all })]);
  return {
    accept: useMutation({ mutationFn: (ids: string[]) => api.acceptAiTestingProposals({ ids, feedback: '' }), onSuccess: done }),
    reject: useMutation({ mutationFn: ({ ids, feedback }: { ids: string[]; feedback: string }) => api.rejectAiTestingProposals({ ids, feedback }), onSuccess: done }),
  };
}

export function useSaveProfile(id?: string) {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (input: AiTestingProfileInput) => (id ? api.updateAiTestingProfile(id, input) : api.createAiTestingProfile(input)),
    onSuccess: () => qc.invalidateQueries({ queryKey: [...K, 'profiles'] }),
  });
}

export function useDeleteProfile() {
  const qc = useQueryClient();
  return useMutation({ mutationFn: (id: string) => api.deleteAiTestingProfile(id), onSuccess: () => qc.invalidateQueries({ queryKey: [...K, 'profiles'] }) });
}
