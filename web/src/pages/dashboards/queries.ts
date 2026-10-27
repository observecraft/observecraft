import { useContext } from 'react';
import { DashboardContext } from './widgets/context';
import { groupedMetricValues, scalarMetricValue } from '../../api/dashboards/reduce';
// TanStack Query bindings for dashboards (documents) and widget data (metric queries).
import { keepPreviousData, useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import {
  addToList,
  cloneDashboard,
  createDashboard,
  createList,
  deleteDashboards,
  getDashboard,
  listLists,
  metricCatalog,
  monitorSummary,
  queryDistribution,
  queryGrouped,
  queryScalar,
  queryTimeseries,
  recentlyViewed,
  restoreDashboard,
  searchDashboards,
  tagValues,
  updateDashboard,
  type DashboardPatch,
} from '../../api/dashboards/client';
import type { CreateDashboardRequest, Dashboard, DashboardSearchRequest, TimeAggregator } from '../../api/dashboards/types';
import type { TimeRange } from '../../api/types';

export const dashKeys = {
  all: ['dashboards'] as const,
  search: (req: DashboardSearchRequest) => [...dashKeys.all, 'search', req] as const,
  recent: () => [...dashKeys.all, 'recent'] as const,
  detail: (id: string) => [...dashKeys.all, 'detail', id] as const,
  lists: () => [...dashKeys.all, 'lists'] as const,
};

export const useDashboardSearch = (req: DashboardSearchRequest) => useQuery({ queryKey: dashKeys.search(req), queryFn: () => searchDashboards(req), placeholderData: keepPreviousData });
export const useRecentlyViewed = () => useQuery({ queryKey: dashKeys.recent(), queryFn: () => recentlyViewed(4) });
export const useDashboard = (id: string) => useQuery({ queryKey: dashKeys.detail(id), queryFn: () => getDashboard(id), staleTime: Infinity });
export const useLists = () => useQuery({ queryKey: dashKeys.lists(), queryFn: listLists });

function useInvalidateAll() {
  const qc = useQueryClient();
  return () => qc.invalidateQueries({ queryKey: [...dashKeys.all] });
}

export function useCreateDashboard() {
  const invalidate = useInvalidateAll();
  return useMutation({ mutationFn: (req: CreateDashboardRequest) => createDashboard(req), onSuccess: invalidate });
}

/** Saves a patch; the detail cache is updated optimistically so the canvas never flickers. */
export function useUpdateDashboard(id: string) {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (patch: DashboardPatch) => updateDashboard(id, patch),
    onMutate: (patch) => {
      qc.setQueryData<Dashboard>(dashKeys.detail(id), (d) => (d ? { ...d, ...patch } : d));
    },
    onSettled: () => {
      qc.invalidateQueries({ queryKey: [...dashKeys.all, 'search'] });
      qc.invalidateQueries({ queryKey: dashKeys.recent() });
    },
  });
}

/** Star toggle from lists (id varies per call, unlike useUpdateDashboard). */
export function useUpdateDashboardStar() {
  const invalidate = useInvalidateAll();
  return useMutation({ mutationFn: ({ id, starred }: { id: string; starred: boolean }) => updateDashboard(id, { starred }), onSuccess: invalidate });
}

export function useCloneDashboard() {
  const invalidate = useInvalidateAll();
  return useMutation({ mutationFn: (id: string) => cloneDashboard(id), onSuccess: invalidate });
}

export function useDeleteDashboards() {
  const invalidate = useInvalidateAll();
  return useMutation({ mutationFn: (ids: string[]) => deleteDashboards(ids), onSuccess: invalidate });
}

export function useRestoreDashboard() {
  const invalidate = useInvalidateAll();
  return useMutation({ mutationFn: (id: string) => restoreDashboard(id), onSuccess: invalidate });
}

export function useCreateList() {
  const invalidate = useInvalidateAll();
  return useMutation({ mutationFn: (name: string) => createList(name), onSuccess: invalidate });
}

export function useAddToList() {
  const invalidate = useInvalidateAll();
  return useMutation({ mutationFn: ({ listId, ids }: { listId: string; ids: string[] }) => addToList(listId, ids), onSuccess: invalidate });
}

// ---------------------------------------------------------------- widget data

const metricKey = (...parts: unknown[]) => ['metrics', ...parts];

function useExecution(queries: string[], points: number, range: TimeRange) {
  const context = useContext(DashboardContext);
  const execution = context?.metrics;
  return execution && context.range.from === range.from && context.range.to === range.to && queries.every((q) => execution.has(q, points)) ? execution : undefined;
}

export function useTimeseriesData(queries: string[], range: TimeRange) {
  const execution = useExecution(queries, 120, range);
  return useQuery({
    queryKey: metricKey('ts', queries, range, execution?.version),
    queryFn: () => {
      if (execution?.error) throw execution.error;
      return execution ? execution.source.timeseries({ queries, range, points: 120 }) : queryTimeseries({ queries, range, points: 120 });
    },
    placeholderData: keepPreviousData,
    gcTime: 30_000,
    enabled: queries.length > 0 && (!execution || execution.ready || !!execution.error),
    retry: execution ? false : 1,
  });
}

export function useScalarData(query: string, aggregator: TimeAggregator, range: TimeRange) {
  const execution = useExecution([query], 60, range);
  return useQuery({
    queryKey: metricKey('scalar', query, aggregator, range, execution?.version),
    queryFn: async () => {
      if (execution?.error) throw execution.error;
      return execution ? scalarMetricValue(await execution.source.timeseries({ queries: [query], range, points: 60 }), query, aggregator) : queryScalar({ query, aggregator, range });
    },
    placeholderData: keepPreviousData,
    gcTime: 30_000,
    enabled: !!query && (!execution || execution.ready || !!execution.error),
    retry: execution ? false : 1,
  });
}

export function useGroupedData(query: string, aggregator: TimeAggregator, range: TimeRange) {
  const execution = useExecution([query], 60, range);
  return useQuery({
    queryKey: metricKey('grouped', query, aggregator, range, execution?.version),
    queryFn: async () => {
      if (execution?.error) throw execution.error;
      return execution ? groupedMetricValues(await execution.source.timeseries({ queries: [query], range, points: 60 }), query, aggregator) : queryGrouped({ query, aggregator, range });
    },
    placeholderData: keepPreviousData,
    gcTime: 30_000,
    enabled: !!query && (!execution || execution.ready || !!execution.error),
    retry: execution ? false : 1,
  });
}

export const useDistributionData = (query: string, range: TimeRange) =>
  useQuery({ queryKey: metricKey('dist', query, range), queryFn: () => queryDistribution({ query, range }), placeholderData: keepPreviousData, enabled: !!query });

export const useMonitorSummary = (query: string) => useQuery({ queryKey: ['monitors', 'summary', query], queryFn: () => monitorSummary(query), refetchInterval: 30_000 });

export const useMetricCatalog = () => useQuery({ queryKey: metricKey('catalog'), queryFn: ({ signal }) => metricCatalog(signal), staleTime: Infinity });

export const useTagValues = (key: string) => useQuery({ queryKey: metricKey('tags', key), queryFn: () => tagValues(key), staleTime: 5 * 60_000 });
