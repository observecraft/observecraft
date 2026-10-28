// TanStack Query bindings for synthetic tests.
import { keepPreviousData, useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import {
  createSyntheticSuite,
  createSyntheticTest,
  createSyntheticVariable,
  deleteSyntheticSuite,
  deleteSyntheticTest,
  deleteSyntheticVariable,
  getSyntheticCoverage,
  getSyntheticResult,
  getSyntheticSettings,
  getSyntheticSuite,
  getSyntheticTest,
  listSyntheticLocations,
  listSyntheticResults,
  listSyntheticVariables,
  runSyntheticTest,
  searchSyntheticResultsAll,
  searchSyntheticSuites,
  searchSyntheticTests,
  trySyntheticRequest,
  updateSyntheticSettings,
  updateSyntheticSuite,
  updateSyntheticTest,
  updateSyntheticVariable,
  type SyntheticCoverageRequest,
  type SyntheticResultSearchRequest,
  type SyntheticSettings,
  type SyntheticSuiteInput,
  type SyntheticSuiteSearchRequest,
  type SyntheticTestInput,
  type SyntheticTestSearchRequest,
  type SyntheticVariableInput,
} from '../../api/synthetics/client';
import type { TimeRange } from '../../api/types';

export const syntheticKeys = { all: ['synthetics'] as const };

// Scheduled runs land every minute at most: refresh on that cadence.
export const useSyntheticTests = (req: SyntheticTestSearchRequest) =>
  useQuery({ queryKey: [...syntheticKeys.all, 'search', req], queryFn: () => searchSyntheticTests(req), placeholderData: keepPreviousData, retry: 1, refetchInterval: 60_000 });

export const useSyntheticTest = (id: string | undefined) =>
  useQuery({ queryKey: [...syntheticKeys.all, 'one', id], queryFn: ({ signal }) => getSyntheticTest(id!, signal), enabled: !!id, retry: 1, refetchInterval: 60_000 });

export const useSyntheticResults = (id: string, range: TimeRange) =>
  useQuery({
    queryKey: [...syntheticKeys.all, 'results', id, range],
    queryFn: ({ signal }) => listSyntheticResults(id, range, signal),
    placeholderData: keepPreviousData,
    retry: 1,
    refetchInterval: 60_000,
  });

export const useSyntheticResult = (id: string, resultId: number | null) =>
  useQuery({ queryKey: [...syntheticKeys.all, 'result', id, resultId], queryFn: ({ signal }) => getSyntheticResult(id, resultId!, signal), enabled: resultId !== null, retry: 1 });

export const useSyntheticLocations = () => useQuery({ queryKey: [...syntheticKeys.all, 'locations'], queryFn: listSyntheticLocations, staleTime: 300_000 });

function useInvalidate() {
  const qc = useQueryClient();
  return () => qc.invalidateQueries({ queryKey: syntheticKeys.all });
}

export function useSaveSyntheticTest(id?: string) {
  const done = useInvalidate();
  return useMutation({ mutationFn: (input: SyntheticTestInput) => (id ? updateSyntheticTest(id, input) : createSyntheticTest(input)), onSuccess: done });
}

export function useDeleteSyntheticTest() {
  const qc = useQueryClient();
  const done = useInvalidate();
  return useMutation({
    mutationFn: deleteSyntheticTest,
    onSuccess: (_, id) => {
      qc.removeQueries({ queryKey: [...syntheticKeys.all, 'one', id] });
      return done();
    },
  });
}

export function useRunSyntheticTest() {
  const done = useInvalidate();
  return useMutation({ mutationFn: runSyntheticTest, onSuccess: done });
}

export const useTrySyntheticRequest = () => useMutation({ mutationFn: trySyntheticRequest });

// ---------------------------------------------------------------- suites

export const useSyntheticSuites = (req: SyntheticSuiteSearchRequest) =>
  useQuery({ queryKey: [...syntheticKeys.all, 'suites', req], queryFn: () => searchSyntheticSuites(req), placeholderData: keepPreviousData, retry: 1, refetchInterval: 60_000 });

export const useSyntheticSuite = (id: string | undefined) =>
  useQuery({ queryKey: [...syntheticKeys.all, 'suite', id], queryFn: ({ signal }) => getSyntheticSuite(id!, signal), enabled: !!id, retry: 1 });

export function useSaveSyntheticSuite(id?: string) {
  const done = useInvalidate();
  return useMutation({ mutationFn: (input: SyntheticSuiteInput) => (id ? updateSyntheticSuite(id, input) : createSyntheticSuite(input)), onSuccess: done });
}

/** Bulk edits of the suites list: one update per suite. */
export function useUpdateSyntheticSuites() {
  const done = useInvalidate();
  return useMutation({ mutationFn: (changes: { id: string; input: SyntheticSuiteInput }[]) => Promise.all(changes.map((c) => updateSyntheticSuite(c.id, c.input))), onSuccess: done });
}

export function useDeleteSyntheticSuites() {
  const done = useInvalidate();
  return useMutation({ mutationFn: (ids: string[]) => Promise.all(ids.map((id) => deleteSyntheticSuite(id))), onSuccess: done });
}

// ---------------------------------------------------------------- explorer and coverage

export const useSyntheticResultSearch = (req: SyntheticResultSearchRequest) =>
  useQuery({
    queryKey: [...syntheticKeys.all, 'explorer', req],
    queryFn: ({ signal }) => searchSyntheticResultsAll(req, signal),
    placeholderData: keepPreviousData,
    retry: false,
    refetchInterval: 60_000,
  });

export const useSyntheticCoverage = (req: SyntheticCoverageRequest) =>
  useQuery({ queryKey: [...syntheticKeys.all, 'coverage', req], queryFn: ({ signal }) => getSyntheticCoverage(req, signal), placeholderData: keepPreviousData, retry: 1 });

// ---------------------------------------------------------------- settings and variables

export const useSyntheticSettings = () => useQuery({ queryKey: [...syntheticKeys.all, 'settings'], queryFn: getSyntheticSettings, staleTime: 60_000 });

export function useSaveSyntheticSettings() {
  const done = useInvalidate();
  return useMutation({ mutationFn: (s: SyntheticSettings) => updateSyntheticSettings(s), onSuccess: done });
}

export const useSyntheticVariables = () => useQuery({ queryKey: [...syntheticKeys.all, 'variables'], queryFn: listSyntheticVariables });

export function useSaveSyntheticVariable(id?: string) {
  const done = useInvalidate();
  return useMutation({ mutationFn: (input: SyntheticVariableInput) => (id ? updateSyntheticVariable(id, input) : createSyntheticVariable(input)), onSuccess: done });
}

export function useDeleteSyntheticVariable() {
  const done = useInvalidate();
  return useMutation({ mutationFn: deleteSyntheticVariable, onSuccess: done });
}
