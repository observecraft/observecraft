// TanStack Query bindings for Incident Response → Status Pages and the public rendering.
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import {
  createStatusPage,
  createStatusPageNotice,
  deleteStatusPage,
  deleteStatusPageNotice,
  getPublicStatusPage,
  getStatusPage,
  listStatusPages,
  updateStatusPage,
  updateStatusPageNotice,
} from '../../api/statusPages/client';
import type { NoticeInput, NoticeUpdateInput, StatusPageDetail, StatusPageInput } from '../../api/statusPages/types';

const keys = {
  all: ['status-pages'] as const,
  list: () => [...keys.all, 'list'] as const,
  detail: (id: string) => [...keys.all, 'detail', id] as const,
  public: (slug: string) => [...keys.all, 'public', slug] as const,
};

export function useStatusPages() {
  return useQuery({ queryKey: keys.list(), queryFn: listStatusPages, refetchInterval: 60_000 });
}

export function useStatusPage(id: string, enabled = true) {
  return useQuery({ queryKey: keys.detail(id), queryFn: ({ signal }) => getStatusPage(id, signal), refetchInterval: 60_000, enabled });
}

export function usePublicStatusPage(slug: string) {
  return useQuery({ queryKey: keys.public(slug), queryFn: ({ signal }) => getPublicStatusPage(slug, signal), refetchInterval: 60_000, retry: false });
}

/** Page and notice writes; the answer (the page's new detail) replaces the cached one and the list is refetched. */
export function useStatusPageMutations() {
  const qc = useQueryClient();
  const onSuccess = (d: StatusPageDetail) => {
    qc.setQueryData(keys.detail(d.id), d);
    return qc.invalidateQueries({ queryKey: keys.list() });
  };
  const refetch = () => qc.invalidateQueries({ queryKey: keys.all });
  return {
    create: useMutation({ mutationFn: createStatusPage, onSuccess }),
    update: useMutation({ mutationFn: ({ id, input }: { id: string; input: StatusPageInput }) => updateStatusPage(id, input), onSuccess }),
    // The deleted page's detail is dropped rather than refetched (it would answer 404).
    remove: useMutation({
      mutationFn: deleteStatusPage,
      onSuccess: (_, id) => {
        qc.removeQueries({ queryKey: keys.detail(id) });
        return qc.invalidateQueries({ queryKey: keys.list() });
      },
    }),
    createNotice: useMutation({ mutationFn: ({ id, input }: { id: string; input: NoticeInput }) => createStatusPageNotice(id, input), onSuccess }),
    updateNotice: useMutation({ mutationFn: ({ id, noticeId, input }: { id: string; noticeId: string; input: NoticeUpdateInput }) => updateStatusPageNotice(id, noticeId, input), onSuccess }),
    removeNotice: useMutation({ mutationFn: ({ id, noticeId }: { id: string; noticeId: string }) => deleteStatusPageNotice(id, noticeId), onSuccess: refetch }),
  };
}
