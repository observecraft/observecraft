// TanStack Query bindings for feature flags.
import { keepPreviousData, useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import {
  createFeatureFlag,
  createFeatureFlagEnvironment,
  deleteFeatureFlag,
  deleteFeatureFlagEnvironment,
  featureFlagEvaluations,
  featureFlagEvaluationSeries,
  getFeatureFlag,
  listFeatureFlagEnvironments,
  listFeatureFlagHistory,
  listFeatureFlags,
  updateFeatureFlag,
  updateFeatureFlagEnvironment,
  updateFeatureFlagEnvironmentSettings,
  type FeatureFlagEnvironmentInput,
  type FeatureFlagEnvUpdate,
  type FeatureFlagInput,
} from '../../api/featureFlags/client';
import type { TimeRange } from '../../api/types';

export const flagKeys = { all: ['feature-flags'] as const };

export const useFeatureFlags = () => useQuery({ queryKey: [...flagKeys.all, 'list'], queryFn: listFeatureFlags, retry: 1 });

export const useFeatureFlag = (id: string | undefined) => useQuery({ queryKey: [...flagKeys.all, 'one', id], queryFn: ({ signal }) => getFeatureFlag(id!, signal), enabled: !!id, retry: 1 });

export const useFlagHistory = (id?: string) => useQuery({ queryKey: [...flagKeys.all, 'history', id ?? '*'], queryFn: () => listFeatureFlagHistory(id), retry: 1 });

export const useFlagEnvironments = () => useQuery({ queryKey: [...flagKeys.all, 'environments'], queryFn: listFeatureFlagEnvironments, retry: 1 });

/** Rounded to the minute so live time ranges don't refetch on every render. */
const minute = (r: TimeRange): TimeRange => ({ from: Math.floor(r.from / 60_000) * 60_000, to: Math.floor(r.to / 60_000) * 60_000 });

export const useFlagEvaluations = (range: TimeRange) => {
  const r = minute(range);
  return useQuery({ queryKey: [...flagKeys.all, 'evaluations', r], queryFn: ({ signal }) => featureFlagEvaluations(r, signal), placeholderData: keepPreviousData, retry: 1, refetchInterval: 60_000 });
};

export const useFlagEvaluationSeries = (flagKey: string | undefined, range: TimeRange) => {
  const r = minute(range);
  return useQuery({
    queryKey: [...flagKeys.all, 'series', flagKey, r],
    queryFn: ({ signal }) => featureFlagEvaluationSeries(flagKey!, r, signal),
    enabled: !!flagKey,
    placeholderData: keepPreviousData,
    retry: 1,
    refetchInterval: 60_000,
  });
};

function useInvalidate() {
  const qc = useQueryClient();
  return () => qc.invalidateQueries({ queryKey: flagKeys.all });
}

export function useSaveFlag(id?: string) {
  const done = useInvalidate();
  return useMutation({ mutationFn: (input: FeatureFlagInput) => (id ? updateFeatureFlag(id, input) : createFeatureFlag(input)), onSuccess: done });
}

export function useDeleteFlag() {
  const qc = useQueryClient();
  const done = useInvalidate();
  return useMutation({
    mutationFn: deleteFeatureFlag,
    onSuccess: (_, id) => {
      qc.removeQueries({ queryKey: [...flagKeys.all, 'one', id] });
      return done();
    },
  });
}

export function useSaveFlagEnvironment(id: string) {
  const done = useInvalidate();
  return useMutation({ mutationFn: ({ environmentId, update }: { environmentId: string; update: FeatureFlagEnvUpdate }) => updateFeatureFlagEnvironment(id, environmentId, update), onSuccess: done });
}

export function useSaveEnvironment(id?: string) {
  const done = useInvalidate();
  return useMutation({ mutationFn: (input: FeatureFlagEnvironmentInput) => (id ? updateFeatureFlagEnvironmentSettings(id, input) : createFeatureFlagEnvironment(input)), onSuccess: done });
}

export function useDeleteEnvironment() {
  const done = useInvalidate();
  return useMutation({ mutationFn: deleteFeatureFlagEnvironment, onSuccess: done });
}
