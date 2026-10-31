// TanStack Query bindings for AI Guard.
import { keepPreviousData, useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import * as api from '../../../api/aiGuard/client';
import type { AiGuardMessage, AiGuardQuery, AiGuardSearchRequest, AiGuardSettings, TimeRange } from '../../../api/aiGuard/client';

const keys = { all: ['ai-guard'] as const };

export const useAiGuardSummary = (q: AiGuardQuery) => useQuery({ queryKey: [...keys.all, 'summary', q], queryFn: ({ signal }) => api.getAiGuardSummary(q, signal), placeholderData: keepPreviousData });
export const useAiGuardEvaluations = (req: AiGuardSearchRequest) =>
  useQuery({ queryKey: [...keys.all, 'evaluations', req], queryFn: ({ signal }) => api.searchAiGuardEvaluations(req, signal), placeholderData: keepPreviousData });
export const useAiGuardEvaluation = (id: string | null) => useQuery({ queryKey: [...keys.all, 'evaluation', id], queryFn: () => api.getAiGuardEvaluation(id!), enabled: !!id });
export const useAiDiscover = (range: TimeRange) => useQuery({ queryKey: [...keys.all, 'discover', range], queryFn: () => api.getAiDiscover(range), placeholderData: keepPreviousData });
export const useAiGuardSettings = () => useQuery({ queryKey: [...keys.all, 'settings'], queryFn: api.getAiGuardSettings });
export const useAiGuardCategories = () => useQuery({ queryKey: [...keys.all, 'categories'], queryFn: api.listAiGuardCategories, staleTime: Infinity });

export function useUpdateAiGuardSettings() {
  const qc = useQueryClient();
  return useMutation({ mutationFn: (s: AiGuardSettings) => api.updateAiGuardSettings(s), onSuccess: () => qc.invalidateQueries({ queryKey: keys.all }) });
}

export const usePlayground = () => useMutation({ mutationFn: ({ messages, meta }: { messages: AiGuardMessage[]; meta?: Record<string, unknown> }) => api.runAiGuardPlayground(messages, meta) });
