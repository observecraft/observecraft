// TanStack Query bindings for Software Delivery.
import { keepPreviousData, useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import {
  aggregations,
  ciOverview,
  ciRepositories,
  deleteGitlabCi,
  gitlabCiStatus,
  saveGitlabCi,
  syncGitlabCi,
  testGitlabCi,
  doraMetrics,
  executionCharts,
  executionFacets,
  flakyTests,
  pipelineDetail,
  searchExecutions,
  setFlakyState,
  type CiQuery,
  type FlakyState,
  type GitlabCiInput,
  type TimeRange,
} from '../../api/ci/client';

const keep = { placeholderData: keepPreviousData, retry: 1 } as const;

export const useExecutions = (q: CiQuery, sort: string, offset: number, limit: number) =>
  useQuery({
    queryKey: ['ci', 'executions', q, sort, offset, limit],
    queryFn: () => searchExecutions(q, sort, offset, limit),
    ...keep,
  });
export const useFacets = (q: CiQuery) =>
  useQuery({
    queryKey: ['ci', 'facets', q],
    queryFn: () => executionFacets(q),
    ...keep,
  });
export const useCharts = (q: CiQuery, enabled = true) =>
  useQuery({
    queryKey: ['ci', 'charts', q],
    queryFn: () => executionCharts(q),
    enabled,
    ...keep,
  });
export const useAggregations = (q: CiQuery, enabled = true) =>
  useQuery({
    queryKey: ['ci', 'aggregations', q],
    queryFn: () => aggregations(q),
    enabled,
    ...keep,
  });
export const usePipeline = (traceId: string | null) =>
  useQuery({
    queryKey: ['ci', 'pipeline', traceId],
    queryFn: () => pipelineDetail(traceId!),
    enabled: !!traceId,
    retry: 1,
    staleTime: 300_000,
  });
export const useCiOverview = (range: TimeRange) =>
  useQuery({
    queryKey: ['ci', 'overview', range],
    queryFn: () => ciOverview(range),
    ...keep,
  });
// No placeholder data: rows from another state tab must never be shown (and acted on) as this tab's.
export const useFlaky = (range: TimeRange, query: string, states: FlakyState[]) =>
  useQuery({
    queryKey: ['ci', 'flaky', range, query, states],
    queryFn: () => flakyTests(range, query, states),
    retry: 1,
  });
export const useDora = (range: TimeRange, env: string | null, services: string[]) =>
  useQuery({
    queryKey: ['ci', 'dora', range, env, services],
    queryFn: () => doraMetrics(range, env, services),
    ...keep,
  });
export const useRepositories = () =>
  useQuery({
    queryKey: ['ci', 'repositories'],
    queryFn: ciRepositories,
    retry: 1,
  });

export function useSetFlakyState() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (v: { repository: string; suite: string; name: string; state: FlakyState }) => setFlakyState(v.repository, v.suite, v.name, v.state),
    onSuccess: () => qc.invalidateQueries({ queryKey: ['ci', 'flaky'] }),
  });
}

/** The GitLab CI connection and its sync status; refreshed every 15 s while the first pipelines arrive. */
export const useGitlabCi = () =>
  useQuery({
    queryKey: ['ci', 'gitlab'],
    queryFn: gitlabCiStatus,
    refetchInterval: (q) => (q.state.data?.connection ? 15_000 : false),
  });

export function useGitlabCiMutations() {
  const qc = useQueryClient();
  const refresh = () => qc.invalidateQueries({ queryKey: ['ci'] });
  return {
    test: useMutation({ mutationFn: (input: GitlabCiInput) => testGitlabCi(input) }),
    save: useMutation({ mutationFn: (input: GitlabCiInput) => saveGitlabCi(input), onSuccess: refresh }),
    remove: useMutation({ mutationFn: deleteGitlabCi, onSuccess: refresh }),
    sync: useMutation({ mutationFn: syncGitlabCi, onSuccess: refresh }),
  };
}
