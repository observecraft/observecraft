// TanStack Query bindings for Workload Protection → Overview.
import { keepPreviousData, useQuery } from '@tanstack/react-query';
import * as api from '../../../api/workloadProtection/client';
import type { TimeRange } from '../../../api/workloadProtection/client';

const keys = { all: ['workload-protection'] as const };

export const useWpSummary = (range: TimeRange) => useQuery({ queryKey: [...keys.all, 'summary', range], queryFn: () => api.getWpSummary(range), placeholderData: keepPreviousData });
export const useWpAgentsList = () => useQuery({ queryKey: [...keys.all, 'agents'], queryFn: api.listWpAgents });
