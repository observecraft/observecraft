// TanStack Query bindings for Sheets: spreadsheets (documents) and the data of each sheet's source.
import { keepPreviousData, useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { createSpreadsheet, deleteSpreadsheet, getSpreadsheet, loadSheetSource, searchSpreadsheets, updateSpreadsheet } from '../../api/sheets/client';
import type { CreateSpreadsheetRequest, SheetSource, SheetTime, SpreadsheetPatch, SpreadsheetSearchRequest } from '../../api/sheets/types';
import { RELATIVE_RANGES } from '../../components/shell/TimeContext';

const keys = { all: ['sheets'] as const };

export const useSpreadsheetSearch = (req: SpreadsheetSearchRequest) => useQuery({ queryKey: [...keys.all, 'search', req], queryFn: () => searchSpreadsheets(req), placeholderData: keepPreviousData });
export const useSpreadsheet = (id: string) => useQuery({ queryKey: [...keys.all, 'detail', id], queryFn: () => getSpreadsheet(id), staleTime: Infinity });

/** The sheet's source over its time frame (ending now), refetched on demand. */
export function useSheetData(source: SheetSource, time: SheetTime, version: number) {
  return useQuery({
    queryKey: [...keys.all, 'data', source, time, version],
    queryFn: () => {
      const ms = RELATIVE_RANGES.find((r) => r.key === time)?.ms ?? 3_600_000;
      const now = Date.now();
      return loadSheetSource(source, { from: now - ms, to: now });
    },
    placeholderData: keepPreviousData,
    staleTime: 60_000,
    retry: false,
  });
}

function useInvalidate() {
  const qc = useQueryClient();
  return () => qc.invalidateQueries({ queryKey: [...keys.all, 'search'] });
}

export function useCreateSpreadsheet() {
  const invalidate = useInvalidate();
  return useMutation({ mutationFn: (req: CreateSpreadsheetRequest) => createSpreadsheet(req), onSuccess: invalidate });
}

export function useUpdateSpreadsheet() {
  const qc = useQueryClient();
  const invalidate = useInvalidate();
  return useMutation({
    mutationFn: ({ id, patch }: { id: string; patch: SpreadsheetPatch }) => updateSpreadsheet(id, patch),
    onSuccess: (s) => {
      qc.setQueryData([...keys.all, 'detail', s.id], s);
      invalidate();
    },
  });
}

export function useDeleteSpreadsheets() {
  const invalidate = useInvalidate();
  return useMutation({ mutationFn: async (ids: string[]) => void (await Promise.all(ids.map(deleteSpreadsheet))), onSuccess: invalidate });
}
