// TanStack Query bindings for App Builder. A button's run is read with useWorkflowRun (../queries).
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { createApp, deleteApp, getApp, listApps, runAppButton, updateApp, type AppValues, type AutomationAppInput } from '../../../api/apps/client';
import { workflowKeys } from '../queries';

export const appKeys = {
  all: ['automation-apps'] as const,
  one: (id: string) => ['automation-apps', id] as const,
};

export const useApps = () => useQuery({ queryKey: appKeys.all, queryFn: ({ signal }) => listApps(signal), retry: 1 });
export const useApp = (id: string | undefined) => useQuery({ queryKey: appKeys.one(id ?? ''), queryFn: ({ signal }) => getApp(id!, signal), enabled: !!id, retry: 1 });

export function useSaveApp() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({ id, input }: { id?: string; input: AutomationAppInput }) => (id ? updateApp(id, input) : createApp(input)),
    onSuccess: (a) => {
      qc.setQueryData(appKeys.one(a.id), a);
      qc.invalidateQueries({ queryKey: appKeys.all, exact: true });
    },
  });
}

export function useDeleteApp() {
  const qc = useQueryClient();
  return useMutation({ mutationFn: deleteApp, onSuccess: () => qc.invalidateQueries({ queryKey: appKeys.all }) });
}

/** Presses a button of an app; the run is then read with useWorkflowRun(workflowId, runId). */
export function useRunAppButton(id: string) {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({ component, values }: { component: string; values: AppValues }) => runAppButton(id, component, values),
    onSuccess: (r) => qc.invalidateQueries({ queryKey: workflowKeys.runs(r.workflowId) }),
  });
}
