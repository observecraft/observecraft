// TanStack Query bindings for Workload Protection's Inventory and Policies pages.
import { keepPreviousData, useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import {
  createWpAgentRule,
  createWpPolicy,
  deleteWpAgentRule,
  deleteWpPolicy,
  listWpAgentRules,
  listWpAgents,
  listWpFindings,
  listWpPolicies,
  updateWpAgentRule,
  updateWpPolicy,
  type TimeRange,
  type WpAgentRuleInput,
  type WpPolicyInput,
} from '../../../api/workloadProtection/client';

const keys = { all: ['workload-protection'] as const };

function useInvalidate() {
  const qc = useQueryClient();
  return () => qc.invalidateQueries({ queryKey: keys.all });
}

export const useWpAgents = () => useQuery({ queryKey: [...keys.all, 'agents'], queryFn: listWpAgents, refetchInterval: 60_000 });
export const useWpPolicies = () => useQuery({ queryKey: [...keys.all, 'policies'], queryFn: listWpPolicies });
export const useWpAgentRules = (range: TimeRange) =>
  useQuery({ queryKey: [...keys.all, 'agent-rules', range.from, range.to], queryFn: () => listWpAgentRules(range), placeholderData: keepPreviousData });
export const useWpFindingRules = (range: TimeRange) =>
  useQuery({ queryKey: [...keys.all, 'findings', range.from, range.to], queryFn: () => listWpFindings(range), placeholderData: keepPreviousData, select: (d) => d.rules });

export function useSaveWpPolicy() {
  const invalidate = useInvalidate();
  return useMutation({ mutationFn: ({ id, input }: { id?: string; input: WpPolicyInput }) => (id ? updateWpPolicy(id, input) : createWpPolicy(input)), onSuccess: invalidate });
}
export function useDeleteWpPolicy() {
  const invalidate = useInvalidate();
  return useMutation({ mutationFn: deleteWpPolicy, onSuccess: invalidate });
}
export function useSaveWpAgentRule() {
  const invalidate = useInvalidate();
  return useMutation({ mutationFn: ({ id, input }: { id?: string; input: WpAgentRuleInput }) => (id ? updateWpAgentRule(id, input) : createWpAgentRule(input)), onSuccess: invalidate });
}
export function useDeleteWpAgentRule() {
  const invalidate = useInvalidate();
  return useMutation({ mutationFn: deleteWpAgentRule, onSuccess: invalidate });
}
