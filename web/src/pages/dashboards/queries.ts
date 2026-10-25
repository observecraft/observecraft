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

export const useDashboardSearch = (req: DashboardSearchRequest) =>
  useQuery({ queryKey: dashKeys.search(req), queryFn: () => searchDashboards(req), placeholderData: keepPreviousData });
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

export const useTimeseriesData = (queries: string[], range: TimeRange) =>
  useQuery({ queryKey: metricKey('ts', queries, range), queryFn: () => queryTimeseries({ queries, range, points: 120 }), placeholderData: keepPreviousData, enabled: queries.length > 0 });

export const useScalarData = (query: string, aggregator: TimeAggregator, range: TimeRange) =>
  useQuery({ queryKey: metricKey('scalar', query, aggregator, range), queryFn: () => queryScalar({ query, aggregator, range }), placeholderData: keepPreviousData, enabled: !!query });

export const useGroupedData = (query: string, aggregator: TimeAggregator, range: TimeRange) =>
  useQuery({ queryKey: metricKey('grouped', query, aggregator, range), queryFn: () => queryGrouped({ query, aggregator, range }), placeholderData: keepPreviousData, enabled: !!query });

export const useDistributionData = (query: string, range: TimeRange) =>
  useQuery({ queryKey: metricKey('dist', query, range), queryFn: () => queryDistribution({ query, range }), placeholderData: keepPreviousData, enabled: !!query });

export const useMonitorSummary = (query: string) => useQuery({ queryKey: ['monitors', 'summary', query], queryFn: () => monitorSummary(query), refetchInterval: 30_000 });

export const useMetricCatalog = () => useQuery({ queryKey: metricKey('catalog'), queryFn: metricCatalog, staleTime: Infinity });

export const useTagValues = (key: string) => useQuery({ queryKey: metricKey('tags', key), queryFn: () => tagValues(key), staleTime: 5 * 60_000 });
