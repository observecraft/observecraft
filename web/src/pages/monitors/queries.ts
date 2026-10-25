// TanStack Query bindings for monitors and downtimes.
import { keepPreviousData, useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import {
  bulkMonitors,
  cancelDowntime,
  cloneMonitor,
  createDowntime,
  createMonitor,
  getMonitor,
  monitorHistory,
  monitorQuality,
  searchDowntimes,
  searchMonitors,
  searchTriggered,
  updateMonitor,
  type BulkAction,
  type DowntimeInput,
  type DowntimeSearchRequest,
  type MonitorInput,
  type MonitorQualityRequest,
  type MonitorSearchRequest,
  type TimeRange,
  type TriggeredSearchRequest,
} from '../../api/monitors/client';

export const monKeys = {
  all: ['monitors'] as const,
  downtimes: ['downtimes'] as const,
};

// Monitors re-evaluate every minute: lists refresh on the same cadence.
const LIVE = {
  placeholderData: keepPreviousData,
  retry: 1,
  refetchInterval: 60_000,
} as const;

export const useMonitors = (req: MonitorSearchRequest) =>
  useQuery({
    queryKey: [...monKeys.all, 'search', req],
    queryFn: () => searchMonitors(req),
    ...LIVE,
  });
export const useTriggered = (req: TriggeredSearchRequest) =>
  useQuery({
    queryKey: [...monKeys.all, 'triggered', req],
    queryFn: () => searchTriggered(req),
    ...LIVE,
  });
export const useMonitor = (id: number, enabled = true) =>
  useQuery({
    queryKey: [...monKeys.all, 'one', id],
    queryFn: () => getMonitor(id),
    retry: 1,
    refetchInterval: 60_000,
    enabled,
  });
export const useMonitorHistory = (id: number, range: TimeRange) =>
  useQuery({
    queryKey: [...monKeys.all, 'history', id, range],
    queryFn: () => monitorHistory(id, range),
    placeholderData: keepPreviousData,
    retry: 1,
  });
export const useQuality = (req: MonitorQualityRequest) =>
  useQuery({
    queryKey: [...monKeys.all, 'quality', req],
    queryFn: () => monitorQuality(req),
    placeholderData: keepPreviousData,
    retry: 1,
  });
export const useDowntimes = (req: DowntimeSearchRequest) =>
  useQuery({
    queryKey: [...monKeys.downtimes, req],
    queryFn: () => searchDowntimes(req),
    ...LIVE,
  });

function useInvalidate() {
  const qc = useQueryClient();
  return () => Promise.all([qc.invalidateQueries({ queryKey: monKeys.all }), qc.invalidateQueries({ queryKey: monKeys.downtimes })]);
}

export function useSaveMonitor(id?: number) {
  const done = useInvalidate();
  return useMutation({
    mutationFn: (input: MonitorInput) => (id ? updateMonitor(id, input) : createMonitor(input)),
    onSuccess: done,
  });
}

export function useCloneMonitor() {
  const done = useInvalidate();
  return useMutation({ mutationFn: cloneMonitor, onSuccess: done });
}

export function useBulk() {
  const qc = useQueryClient();
  const done = useInvalidate();
  return useMutation({
    mutationFn: (v: { ids: number[]; action: BulkAction; muteMinutes?: number | null }) => bulkMonitors(v.ids, v.action, v.muteMinutes),
    onSuccess: (_, v) => {
      // Deleted monitors would 404 if their open detail/history queries were refetched.
      if (v.action === 'delete') v.ids.forEach((id) => ['one', 'history'].forEach((k) => qc.removeQueries({ queryKey: [...monKeys.all, k, id] })));
      return done();
    },
  });
}

export function useCreateDowntime() {
  const done = useInvalidate();
  return useMutation({
    mutationFn: (input: DowntimeInput) => createDowntime(input),
    onSuccess: done,
  });
}

export function useCancelDowntime() {
  const done = useInvalidate();
  return useMutation({ mutationFn: cancelDowntime, onSuccess: done });
}
