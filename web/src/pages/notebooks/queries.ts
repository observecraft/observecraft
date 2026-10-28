// TanStack Query bindings for notebooks.
import { keepPreviousData, useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { createDashboard, updateDashboard } from '../../api/dashboards/client';
import type { Widget } from '../../api/dashboards/types';
import {
  cloneNotebook,
  createNotebook,
  deleteNotebook,
  getNotebook,
  restoreNotebook,
  searchNotebooks,
  setNotebookFavorite,
  updateNotebook,
  type CreateNotebookRequest,
  type Notebook,
  type NotebookCell,
  type NotebookPatch,
  type NotebookSearchRequest,
} from '../../api/notebooks/client';

export const notebookKeys = {
  all: ['notebooks'] as const,
  search: (req: NotebookSearchRequest) => [...notebookKeys.all, 'search', req] as const,
  detail: (id: string) => [...notebookKeys.all, 'detail', id] as const,
};

export const useNotebookSearch = (req: NotebookSearchRequest) => useQuery({ queryKey: notebookKeys.search(req), queryFn: () => searchNotebooks(req), placeholderData: keepPreviousData });
/** The editor keeps its own copy of the cells, so the document is fetched once per visit. */
export const useNotebook = (id: string) => useQuery({ queryKey: notebookKeys.detail(id), queryFn: () => getNotebook(id), staleTime: Infinity, gcTime: 0 });

function useInvalidateLists() {
  const qc = useQueryClient();
  return () => qc.invalidateQueries({ queryKey: [...notebookKeys.all, 'search'] });
}

export function useCreateNotebook() {
  const invalidate = useInvalidateLists();
  return useMutation({ mutationFn: (req: CreateNotebookRequest) => createNotebook(req), onSuccess: invalidate });
}

/** Saves a patch and keeps the cached document in step (the editor autosaves through this). */
export function useUpdateNotebook(id: string) {
  const qc = useQueryClient();
  const invalidate = useInvalidateLists();
  return useMutation({
    mutationFn: (patch: NotebookPatch) => updateNotebook(id, patch),
    onSuccess: (nb) => {
      qc.setQueryData(notebookKeys.detail(id), nb);
      invalidate();
    },
  });
}

/** Appends cells to an existing notebook ("Add to notebook" from other pages). */
export function useAppendCells() {
  const invalidate = useInvalidateLists();
  return useMutation({
    mutationFn: async ({ id, cells }: { id: string; cells: NotebookCell[] }) => {
      const nb = await getNotebook(id);
      return updateNotebook(id, { cells: [...nb.cells, ...cells] });
    },
    onSuccess: invalidate,
  });
}

export function useNotebookFavorite() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({ id, favorite }: { id: string; favorite: boolean }) => setNotebookFavorite(id, favorite),
    onSuccess: (_, { id, favorite }) => {
      qc.setQueryData<Notebook>(notebookKeys.detail(id), (nb) => (nb ? { ...nb, favorite } : nb));
      qc.invalidateQueries({ queryKey: [...notebookKeys.all, 'search'] });
    },
  });
}

export function useCloneNotebook() {
  const invalidate = useInvalidateLists();
  return useMutation({ mutationFn: (id: string) => cloneNotebook(id), onSuccess: invalidate });
}

export function useDeleteNotebook() {
  const qc = useQueryClient();
  return useMutation({ mutationFn: (id: string) => deleteNotebook(id), onSuccess: () => qc.invalidateQueries({ queryKey: notebookKeys.all }) });
}

export function useRestoreNotebook() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (id: string) => restoreNotebook(id),
    onSuccess: (nb) => {
      qc.setQueryData(notebookKeys.detail(nb.id), nb);
      qc.invalidateQueries({ queryKey: [...notebookKeys.all, 'search'] });
    },
  });
}

/** A new dashboard holding the notebook's cells as widgets, one per row. */
export function useExportToDashboard() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: async ({ title, widgets }: { title: string; widgets: Widget[] }) => {
      const dashboard = await createDashboard({ title, teams: [], layoutType: 'grid' });
      return updateDashboard(dashboard.id, { widgets });
    },
    onSuccess: () => qc.invalidateQueries({ queryKey: ['dashboards'] }),
  });
}
