// TanStack Query bindings for tagging policies, their coverage and resource provenance.
import { keepPreviousData, useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import {
  createTaggingPolicy,
  deleteTaggingPolicy,
  getTagCoverage,
  getTaggingPolicy,
  listResourceProvenance,
  listTaggingPolicies,
  updateTaggingPolicy,
  type TaggingPolicyInput,
  type TagTarget,
} from '../../api/tagging/client';

const keys = { all: ['tagging'] as const };

export const useTaggingPolicies = () => useQuery({ queryKey: [...keys.all, 'policies'], queryFn: listTaggingPolicies });
export const useTagCoverage = (policy?: string) => useQuery({ queryKey: [...keys.all, 'coverage', policy ?? ''], queryFn: () => getTagCoverage(policy), placeholderData: keepPreviousData });
export const useTaggingPolicy = (id: string | undefined) => useQuery({ queryKey: [...keys.all, 'policy', id], queryFn: () => getTaggingPolicy(id!), enabled: !!id });
export const useResourceProvenance = (target?: TagTarget) =>
  useQuery({ queryKey: [...keys.all, 'provenance', target ?? ''], queryFn: () => listResourceProvenance(target), placeholderData: keepPreviousData });

export function useSaveTaggingPolicy() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({ id, input }: { id?: string; input: TaggingPolicyInput }) => (id ? updateTaggingPolicy(id, input) : createTaggingPolicy(input)),
    onSuccess: () => qc.invalidateQueries({ queryKey: keys.all }),
  });
}

export function useDeleteTaggingPolicy() {
  const qc = useQueryClient();
  return useMutation({ mutationFn: deleteTaggingPolicy, onSuccess: () => qc.invalidateQueries({ queryKey: keys.all }) });
}
