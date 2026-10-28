// TanStack Query bindings for service level objectives and corrections.
import { keepPreviousData, useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import {
  createCorrection,
  createSlo,
  deleteCorrection,
  deleteSlo,
  getSlo,
  listCorrections,
  searchSlos,
  updateCorrection,
  updateSlo,
  type SloCorrectionInput,
  type SloInput,
  type SloSearchRequest,
  type SloWindow,
} from '../../api/slos/client';

export const sloKeys = {
  all: ['slos'] as const,
  corrections: ['slo-corrections'] as const,
};

// The server caches measured SLOs for a minute: refresh on the same cadence.
export const useSlos = (req: SloSearchRequest) =>
  useQuery({
    queryKey: [...sloKeys.all, 'search', req],
    queryFn: () => searchSlos(req),
    placeholderData: keepPreviousData,
    retry: 1,
    refetchInterval: 60_000,
  });

export const useSlo = (id: string | null, window?: SloWindow) =>
  useQuery({
    queryKey: [...sloKeys.all, 'one', id, window ?? 'primary'],
    queryFn: () => getSlo(id!, window),
    enabled: !!id,
    placeholderData: keepPreviousData,
    retry: 1,
    refetchInterval: 60_000,
  });

export const useCorrections = (sloId?: string) =>
  useQuery({
    queryKey: [...sloKeys.corrections, sloId ?? 'all'],
    queryFn: () => listCorrections(sloId),
    retry: 1,
  });

function useInvalidate() {
  const qc = useQueryClient();
  return () => Promise.all([qc.invalidateQueries({ queryKey: sloKeys.all }), qc.invalidateQueries({ queryKey: sloKeys.corrections })]);
}

export function useSaveSlo(id?: string) {
  const done = useInvalidate();
  return useMutation({ mutationFn: (input: SloInput) => (id ? updateSlo(id, input) : createSlo(input)), onSuccess: done });
}

export function useDeleteSlo() {
  const qc = useQueryClient();
  const done = useInvalidate();
  return useMutation({
    mutationFn: deleteSlo,
    onSuccess: (_, id) => {
      // A deleted SLO's open detail would 404 if refetched.
      qc.removeQueries({ queryKey: [...sloKeys.all, 'one', id] });
      return done();
    },
  });
}

export function useSaveCorrection(id?: string) {
  const done = useInvalidate();
  return useMutation({ mutationFn: (input: SloCorrectionInput) => (id ? updateCorrection(id, input) : createCorrection(input)), onSuccess: done });
}

export function useDeleteCorrection() {
  const done = useInvalidate();
  return useMutation({ mutationFn: deleteCorrection, onSuccess: done });
}
