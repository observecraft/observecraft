import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { createSavedQuery, deleteSavedQuery, getSqlSchema, listSavedQueries, listSqlTemplates, updateSavedQuery, type SavedSqlQueryInput } from '../../api/sqlEditor/client';

const keys = {
  schema: ['sql', 'schema'] as const,
  templates: ['sql', 'templates'] as const,
  queries: ['sql', 'queries'] as const,
};

export const useSqlSchema = () => useQuery({ queryKey: keys.schema, queryFn: getSqlSchema, staleTime: 10 * 60_000 });
export const useSqlTemplates = () => useQuery({ queryKey: keys.templates, queryFn: listSqlTemplates, staleTime: 10 * 60_000 });
export const useSavedQueries = () => useQuery({ queryKey: keys.queries, queryFn: listSavedQueries });

/** Saved query writes refresh the list. */
export function useSavedQueryMutations() {
  const qc = useQueryClient();
  const onSuccess = () => qc.invalidateQueries({ queryKey: keys.queries });
  return {
    create: useMutation({ mutationFn: (i: SavedSqlQueryInput) => createSavedQuery(i), onSuccess }),
    update: useMutation({ mutationFn: ({ id, input }: { id: string; input: SavedSqlQueryInput }) => updateSavedQuery(id, input), onSuccess }),
    remove: useMutation({ mutationFn: (id: string) => deleteSavedQuery(id), onSuccess }),
  };
}
