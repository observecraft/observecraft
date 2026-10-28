// TanStack Query bindings for Developer Portal self-service actions.
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import {
  createPortalAction,
  deletePortalAction,
  getPortalAction,
  listPortalActionRuns,
  listPortalActions,
  runPortalAction,
  updatePortalAction,
  type ActionValues,
  type PortalActionInput,
} from '../../../api/portalActions/client';

export const actionKeys = {
  all: ['portal-actions'] as const,
  one: (id: string) => ['portal-actions', id] as const,
  runs: (id: string) => ['portal-actions', id, 'runs'] as const,
};

export const usePortalActions = () => useQuery({ queryKey: actionKeys.all, queryFn: ({ signal }) => listPortalActions(signal), retry: 1 });
export const usePortalAction = (id: string | undefined) => useQuery({ queryKey: actionKeys.one(id ?? ''), queryFn: ({ signal }) => getPortalAction(id!, signal), enabled: !!id, retry: 1 });
export const usePortalActionRuns = (id: string | undefined) => useQuery({ queryKey: actionKeys.runs(id ?? ''), queryFn: ({ signal }) => listPortalActionRuns(id!, signal), enabled: !!id, retry: 1 });

export function useSavePortalAction() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({ id, input }: { id?: string; input: PortalActionInput }) => (id ? updatePortalAction(id, input) : createPortalAction(input)),
    onSuccess: (a) => {
      qc.setQueryData(actionKeys.one(a.id), a);
      qc.invalidateQueries({ queryKey: actionKeys.all, exact: true });
    },
  });
}

export function useDeletePortalAction() {
  const qc = useQueryClient();
  return useMutation({ mutationFn: deletePortalAction, onSuccess: () => qc.invalidateQueries({ queryKey: actionKeys.all }) });
}

export function useRunPortalAction(id: string) {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (inputs: ActionValues) => runPortalAction(id, inputs),
    onSuccess: () => qc.invalidateQueries({ queryKey: actionKeys.all }),
  });
}
