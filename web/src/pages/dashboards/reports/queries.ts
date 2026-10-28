// TanStack Query bindings for dashboard reports.
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import {
  createReport,
  deleteReport,
  listReports,
  previewReport,
  reportOptions,
  sendReport,
  setReportSubscription,
  updateReport,
  type DashboardReportInput,
} from '../../../api/dashboardReports/client';
import { getProfile } from '../../../api/identity/client';

const keys = { all: ['dashboard-reports'] as const };

export const useReports = () => useQuery({ queryKey: [...keys.all, 'list'], queryFn: listReports });
export const useReportOptions = () => useQuery({ queryKey: [...keys.all, 'options'], queryFn: reportOptions, staleTime: 60_000 });
export const useReportPreview = (id: string) => useQuery({ queryKey: [...keys.all, 'preview', id], queryFn: () => previewReport(id), staleTime: 0 });
export const useMe = () => useQuery({ queryKey: ['identity', 'profile'], queryFn: getProfile, staleTime: 60_000, retry: false });

function useInvalidate() {
  const qc = useQueryClient();
  return () => qc.invalidateQueries({ queryKey: [...keys.all] });
}

export function useSaveReport() {
  const invalidate = useInvalidate();
  return useMutation({ mutationFn: ({ id, input }: { id?: string; input: DashboardReportInput }) => (id ? updateReport(id, input) : createReport(input)), onSuccess: invalidate });
}

export function useDeleteReport() {
  const invalidate = useInvalidate();
  return useMutation({ mutationFn: (id: string) => deleteReport(id), onSuccess: invalidate });
}

export function useSubscribeReport() {
  const invalidate = useInvalidate();
  return useMutation({ mutationFn: ({ id, subscribed }: { id: string; subscribed: boolean }) => setReportSubscription(id, subscribed), onSuccess: invalidate });
}

export function useSendReport() {
  const invalidate = useInvalidate();
  return useMutation({ mutationFn: (id: string) => sendReport(id), onSuccess: invalidate });
}
