// TanStack Query bindings for Security and Cloud SIEM.
import { keepPreviousData, useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import {
  bulkUpdateSignals,
  createSecurityRule,
  createSuppression,
  deleteSecurityRule,
  deleteSuppression,
  getResearchFeed,
  getSecurityInbox,
  getSecurityOverview,
  getSecurityRule,
  getSiemOverview,
  getSignal,
  listContentPacks,
  listSecurityRules,
  listSuppressions,
  searchSignals,
  setContentPack,
  setSecurityRuleEnabled,
  setSignalAssignee,
  setSignalState,
  updateSecurityRule,
  updateSuppression,
  type SecurityRuleInput,
  type SignalBulkInput,
  type SignalSearchRequest,
  type SignalStateInput,
  type SuppressionInput,
} from '../../api/security/client';
import { logHistogram, searchLogPage } from '../../api/client';

const keys = { all: ['security'] as const };

function useInvalidate() {
  const qc = useQueryClient();
  return () => qc.invalidateQueries({ queryKey: keys.all });
}

export const useSecurityOverview = () => useQuery({ queryKey: [...keys.all, 'overview'], queryFn: getSecurityOverview, refetchInterval: 60_000 });
export const useSecurityInbox = () => useQuery({ queryKey: [...keys.all, 'inbox'], queryFn: getSecurityInbox });
export const useResearchFeed = () => useQuery({ queryKey: [...keys.all, 'feed'], queryFn: getResearchFeed, staleTime: 300_000 });

export const useSecurityRules = () => useQuery({ queryKey: [...keys.all, 'rules'], queryFn: listSecurityRules });
export const useSecurityRule = (id: string | undefined) => useQuery({ queryKey: [...keys.all, 'rule', id], queryFn: () => getSecurityRule(id!), enabled: !!id });

export function useSaveRule(id?: string) {
  const done = useInvalidate();
  return useMutation({ mutationFn: (input: SecurityRuleInput) => (id ? updateSecurityRule(id, input) : createSecurityRule(input)), onSuccess: done });
}

export function useDeleteRule() {
  const done = useInvalidate();
  return useMutation({ mutationFn: deleteSecurityRule, onSuccess: done });
}

export function useRuleEnabled() {
  const done = useInvalidate();
  return useMutation({ mutationFn: ({ id, enabled }: { id: string; enabled: boolean }) => setSecurityRuleEnabled(id, enabled), onSuccess: done });
}

export const useSuppressions = () => useQuery({ queryKey: [...keys.all, 'suppressions'], queryFn: listSuppressions });

export function useSaveSuppression(id?: string) {
  const done = useInvalidate();
  return useMutation({ mutationFn: (input: SuppressionInput) => (id ? updateSuppression(id, input) : createSuppression(input)), onSuccess: done });
}

export function useDeleteSuppression() {
  const done = useInvalidate();
  return useMutation({ mutationFn: deleteSuppression, onSuccess: done });
}

export const useSignals = (req: SignalSearchRequest) =>
  useQuery({ queryKey: [...keys.all, 'signals', req], queryFn: ({ signal }) => searchSignals(req, signal), placeholderData: keepPreviousData, refetchInterval: 60_000 });

export const useSignal = (id: string | null) => useQuery({ queryKey: [...keys.all, 'signal', id], queryFn: () => getSignal(id!), enabled: !!id });

export function useSignalState() {
  const done = useInvalidate();
  return useMutation({ mutationFn: ({ id, input }: { id: string; input: SignalStateInput }) => setSignalState(id, input), onSuccess: done });
}

export function useSignalAssignee() {
  const done = useInvalidate();
  return useMutation({ mutationFn: ({ id, assigneeId }: { id: string; assigneeId: string | null }) => setSignalAssignee(id, assigneeId), onSuccess: done });
}

export function useBulkSignals() {
  const done = useInvalidate();
  return useMutation({ mutationFn: (input: SignalBulkInput) => bulkUpdateSignals(input), onSuccess: done });
}

export const useContentPacks = () => useQuery({ queryKey: [...keys.all, 'packs'], queryFn: listContentPacks, refetchInterval: 120_000 });

export function useSetPack() {
  const done = useInvalidate();
  return useMutation({ mutationFn: ({ id, enabled }: { id: string; enabled: boolean }) => setContentPack(id, enabled), onSuccess: done });
}

export const useSiemOverview = (range: { from: number; to: number }) =>
  useQuery({ queryKey: [...keys.all, 'siem-overview', range], queryFn: ({ signal }) => getSiemOverview(range, signal), placeholderData: keepPreviousData, refetchInterval: 60_000 });

/** The logs a signal is about (with the reader's log access): counts over time and the newest. */
export const useSignalLogs = (query: string | undefined, range: { from: number; to: number }) => ({
  histogram: useQuery({ queryKey: ['logs', 'signal-histogram', query, range], queryFn: () => logHistogram({ query: query!, range }), enabled: !!query }),
  samples: useQuery({ queryKey: ['logs', 'signal-samples', query, range], queryFn: () => searchLogPage({ query: query!, range }, 20), enabled: !!query }),
});
