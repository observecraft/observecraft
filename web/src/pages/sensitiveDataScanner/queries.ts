import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import * as api from '../../api/sensitiveDataScanner/client';
import type { SdsGroup, SdsGroupInput, SdsRule, SdsRuleInput, SdsTestRequest } from '../../api/sensitiveDataScanner/client';

const KEY = ['sensitive-data-scanner'];

export const useSdsConfiguration = () => useQuery({ queryKey: KEY, queryFn: api.getConfiguration });
export const useStandardPatterns = () => useQuery({ queryKey: [...KEY, 'standard-patterns'], queryFn: api.listStandardPatterns, staleTime: Infinity });
export const useRuleTest = (req: SdsTestRequest | null) =>
  useQuery({ queryKey: [...KEY, 'test', req], queryFn: () => api.testRule(req!), enabled: !!req && req.sample.trim() !== '', retry: false, staleTime: 60_000 });

/** A mutation that reloads the configuration once it succeeds. */
function useChange<A, R>(fn: (a: A) => Promise<R>) {
  const qc = useQueryClient();
  return useMutation({ mutationFn: fn, onSettled: () => qc.invalidateQueries({ queryKey: KEY }) });
}

export const useSaveGroup = () => useChange(({ id, input }: { id?: string; input: SdsGroupInput }) => (id ? api.updateGroup(id, input) : api.createGroup(input)));
export const useDeleteGroup = () => useChange(api.deleteGroup);
export const useReorderGroups = () => useChange(api.reorderGroups);
export const useSaveRule = () => useChange(({ id, input }: { id?: string; input: SdsRuleInput }) => (id ? api.updateRule(id, input) : api.createRule(input)));
export const useDeleteRule = () => useChange(api.deleteRule);
export const useReorderRules = () => useChange(({ groupId, ids }: { groupId: string; ids: string[] }) => api.reorderRules(groupId, ids));

/** A group as the editor (and an update) sends it back. */
export const groupInput = (g: SdsGroup): SdsGroupInput => ({ name: g.name, description: g.description, query: g.query, enabled: g.enabled, products: g.products, samplingRate: g.samplingRate });

/** A rule as an update sends it back. */
export const ruleInput = (r: SdsRule): SdsRuleInput => ({
  groupId: r.groupId,
  name: r.name,
  description: r.description,
  pattern: r.pattern ?? null,
  standardPatternId: r.standardPatternId ?? null,
  namespaces: r.namespaces,
  excludedNamespaces: r.excludedNamespaces,
  tags: r.tags,
  enabled: r.enabled,
  priority: r.priority ?? null,
  replacement: r.replacement,
  keywords: r.keywords,
  keywordCharacters: r.keywordCharacters ?? null,
  useRecommendedKeywords: r.useRecommendedKeywords,
  suppressions: r.suppressions,
});

/** `list` with the item at `i` moved by `by`. */
export function moved<T>(list: T[], i: number, by: -1 | 1): T[] {
  const j = i + by;
  if (j < 0 || j >= list.length) return list;
  const out = [...list];
  [out[i], out[j]] = [out[j], out[i]];
  return out;
}
