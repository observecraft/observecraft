// TanStack Query bindings for synthetic tests.
import { keepPreviousData, useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import {
  createSyntheticTest,
  deleteSyntheticTest,
  getSyntheticResult,
  getSyntheticTest,
  listSyntheticLocations,
  listSyntheticResults,
  runSyntheticTest,
  searchSyntheticTests,
  trySyntheticRequest,
  updateSyntheticTest,
  type SyntheticTestInput,
  type SyntheticTestSearchRequest,
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
