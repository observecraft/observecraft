// TanStack Query bindings for On-Call.
import { keepPreviousData, useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import {
  acknowledgeOnCallPage,
  createOnCallOverride,
  createOnCallPage,
  createOnCallPolicy,
  createOnCallSchedule,
  deleteOnCallOverride,
  deleteOnCallPolicy,
  deleteOnCallSchedule,
  getOnCallPage,
  getOnCallPolicy,
  getOnCallProfile,
  getOnCallSchedule,
  listOnCallPolicies,
  listOnCallSchedules,
  listOnCallTeams,
  myOnCallShifts,
  onCallAnalytics,
  onCallScheduleShifts,
  previewOnCallSchedule,
  reassignOnCallPage,
  resolveOnCallPage,
  searchOnCallPages,
  setOnCallRouting,
  testOnCallProfile,
  updateOnCallPolicy,
  updateOnCallProfile,
  updateOnCallSchedule,
  type CreateOnCallOverride,
  type OnCallAnalyticsRequest,
  type OnCallPageSearch,
  type OnCallPolicyInput,
  type OnCallPreviewRequest,
  type OnCallRange,
  type OnCallScheduleInput,
  type ReassignOnCallPage,
} from '../../api/onCall/client';

export const onCallKeys = { all: ['on-call'] as const };

function useInvalidate() {
  const qc = useQueryClient();
  return () => qc.invalidateQueries({ queryKey: onCallKeys.all });
}

// Pages escalate on the server every few seconds: refresh lists on a short cadence.
export const useOnCallPages = (req: OnCallPageSearch) =>
  useQuery({ queryKey: [...onCallKeys.all, 'pages', req], queryFn: ({ signal }) => searchOnCallPages(req, signal), placeholderData: keepPreviousData, retry: 1, refetchInterval: 15_000 });

export const useOnCallPage = (id: number | null) =>
  useQuery({ queryKey: [...onCallKeys.all, 'page', id], queryFn: ({ signal }) => getOnCallPage(id!, signal), enabled: id != null, retry: 1, refetchInterval: 10_000 });

export const useOnCallSchedules = () => useQuery({ queryKey: [...onCallKeys.all, 'schedules'], queryFn: listOnCallSchedules, retry: 1, refetchInterval: 60_000 });

export const useOnCallSchedule = (id: string | undefined) =>
  useQuery({ queryKey: [...onCallKeys.all, 'schedule', id], queryFn: ({ signal }) => getOnCallSchedule(id!, signal), enabled: !!id, retry: 1 });

export const useOnCallShifts = (id: string | undefined, range: OnCallRange) =>
  useQuery({ queryKey: [...onCallKeys.all, 'shifts', id, range], queryFn: ({ signal }) => onCallScheduleShifts(id!, range, signal), enabled: !!id, placeholderData: keepPreviousData, retry: 1 });

export const useOnCallPreview = (req: OnCallPreviewRequest) =>
  useQuery({ queryKey: [...onCallKeys.all, 'preview', req], queryFn: ({ signal }) => previewOnCallSchedule(req, signal), placeholderData: keepPreviousData, retry: 1 });

export const useMyOnCallShifts = (range: OnCallRange) =>
  useQuery({ queryKey: [...onCallKeys.all, 'my-shifts', range], queryFn: ({ signal }) => myOnCallShifts(range, signal), placeholderData: keepPreviousData, retry: 1 });

export const useOnCallPolicies = () => useQuery({ queryKey: [...onCallKeys.all, 'policies'], queryFn: listOnCallPolicies, retry: 1 });

export const useOnCallPolicy = (id: string | undefined) => useQuery({ queryKey: [...onCallKeys.all, 'policy', id], queryFn: ({ signal }) => getOnCallPolicy(id!, signal), enabled: !!id, retry: 1 });

export const useOnCallTeams = () => useQuery({ queryKey: [...onCallKeys.all, 'teams'], queryFn: listOnCallTeams, retry: 1, refetchInterval: 60_000 });

export const useOnCallProfile = () => useQuery({ queryKey: [...onCallKeys.all, 'profile'], queryFn: getOnCallProfile, retry: 1 });

export const useOnCallAnalytics = (req: OnCallAnalyticsRequest) =>
  useQuery({ queryKey: [...onCallKeys.all, 'analytics', req], queryFn: ({ signal }) => onCallAnalytics(req, signal), placeholderData: keepPreviousData, retry: 1 });

export function useSaveOnCallSchedule(id?: string) {
  const done = useInvalidate();
  return useMutation({ mutationFn: (input: OnCallScheduleInput) => (id ? updateOnCallSchedule(id, input) : createOnCallSchedule(input)), onSuccess: done });
}

export function useDeleteOnCallSchedule() {
  const done = useInvalidate();
  return useMutation({ mutationFn: deleteOnCallSchedule, onSuccess: done });
}

export function useCreateOnCallOverride(scheduleId: string) {
  const done = useInvalidate();
  return useMutation({ mutationFn: (req: CreateOnCallOverride) => createOnCallOverride(scheduleId, req), onSuccess: done });
}

export function useDeleteOnCallOverride(scheduleId: string) {
  const done = useInvalidate();
  return useMutation({ mutationFn: (overrideId: string) => deleteOnCallOverride(scheduleId, overrideId), onSuccess: done });
}

export function useSaveOnCallPolicy(id?: string) {
  const done = useInvalidate();
  return useMutation({ mutationFn: (input: OnCallPolicyInput) => (id ? updateOnCallPolicy(id, input) : createOnCallPolicy(input)), onSuccess: done });
}

export function useDeleteOnCallPolicy() {
  const done = useInvalidate();
  return useMutation({ mutationFn: deleteOnCallPolicy, onSuccess: done });
}

export function useSetOnCallRouting() {
  const done = useInvalidate();
  return useMutation({ mutationFn: ({ handle, policyId }: { handle: string; policyId: string | null }) => setOnCallRouting(handle, policyId), onSuccess: done });
}

export function useCreateOnCallPage() {
  const done = useInvalidate();
  return useMutation({ mutationFn: createOnCallPage, onSuccess: done });
}

export function usePageAction() {
  const done = useInvalidate();
  return useMutation({
    mutationFn: ({ id, action, reassign }: { id: number; action: 'acknowledge' | 'resolve' | 'reassign'; reassign?: ReassignOnCallPage }) =>
      action === 'acknowledge' ? acknowledgeOnCallPage(id) : action === 'resolve' ? resolveOnCallPage(id) : reassignOnCallPage(id, reassign ?? {}),
    onSuccess: done,
  });
}

export function useUpdateOnCallProfile() {
  const done = useInvalidate();
  return useMutation({ mutationFn: updateOnCallProfile, onSuccess: done });
}

export const useTestOnCallProfile = () => useMutation({ mutationFn: testOnCallProfile });
