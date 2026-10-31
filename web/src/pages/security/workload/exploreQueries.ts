// TanStack Query bindings for Workload Protection's Signals, Investigate and Findings pages.
import { keepPreviousData, useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import * as api from '../../../api/workloadProtection/client';
import type { TimeRange } from '../../../api/workloadProtection/client';

const keys = { all: ['workload-protection'] as const };

export const useWpEvents = (range: TimeRange, query: string, limit = 200) =>
  useQuery({ queryKey: [...keys.all, 'events', range, query, limit], queryFn: () => api.searchWpEvents(range, query, limit), placeholderData: keepPreviousData });
export const useWpFindings = (range: TimeRange) => useQuery({ queryKey: [...keys.all, 'findings', range], queryFn: () => api.listWpFindings(range), placeholderData: keepPreviousData });

export function useTriageWpFinding() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({ key, status }: { key: string; status: 'open' | 'muted' }) => api.triageWpFinding(key, status),
    onSuccess: () => qc.invalidateQueries({ queryKey: [...keys.all, 'findings'] }),
  });
}
