// TanStack Query bindings for Developer Portal scorecards. Every write answers with the whole overview,
// which replaces the cached one.
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import {
  createScorecardRule,
  deleteScorecardRule,
  evaluateScorecards,
  getScorecards,
  setScorecardOutcomes,
  setupScorecards,
  updateScorecardRule,
  updateServiceMetadata,
  type ScorecardOutcomesInput,
  type ScorecardRuleInput,
  type ScorecardRuleUpdate,
  type ScorecardsOverview,
  type ServiceMetadataInput,
} from '../../api/scorecards/client';

export const scorecardKeys = { all: ['scorecards'] as const };

// The server evaluates hourly: refresh every few minutes to pick that up.
export const useScorecards = () => useQuery({ queryKey: scorecardKeys.all, queryFn: ({ signal }) => getScorecards(signal), retry: 1, refetchInterval: 300_000 });

function useOverviewMutation<V>(fn: (v: V) => Promise<ScorecardsOverview>) {
  const qc = useQueryClient();
  return useMutation({ mutationFn: fn, onSuccess: (data) => qc.setQueryData(scorecardKeys.all, data) });
}

export const useSetupScorecards = () => useOverviewMutation((rules: string[]) => setupScorecards({ rules }));
export const useEvaluateScorecards = () => useOverviewMutation((_: void) => evaluateScorecards());
export const useCreateScorecardRule = () => useOverviewMutation((input: ScorecardRuleInput) => createScorecardRule(input));
export const useUpdateScorecardRule = () => useOverviewMutation(({ id, input }: { id: string; input: ScorecardRuleUpdate }) => updateScorecardRule(id, input));
export const useSetScorecardOutcomes = () => useOverviewMutation(({ id, input }: { id: string; input: ScorecardOutcomesInput }) => setScorecardOutcomes(id, input));
export const useUpdateServiceMetadata = () => useOverviewMutation(({ service, input }: { service: string; input: ServiceMetadataInput }) => updateServiceMetadata(service, input));

export function useDeleteScorecardRule() {
  const qc = useQueryClient();
  return useMutation({ mutationFn: deleteScorecardRule, onSuccess: () => qc.invalidateQueries({ queryKey: scorecardKeys.all }) });
}
