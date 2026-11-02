// TanStack Query bindings for Integrations / Community / Reference Tables.
import { keepPreviousData, useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { getIntegration, installIntegration, loadIntegrationDocs, listCategories, searchCatalog, uninstallIntegration } from '../../api/integrations/client';
import type { CatalogRequest } from '../../api/integrations/types';
import { searchDashboards } from '../../api/dashboards/client';
import { loadAgentSpec } from '../../api/integrations/catalog';
import { createReferenceTable, deleteReferenceTable, getReferenceTable, listReferenceTables, replaceReferenceTable } from '../../api/referenceTables/client';
import type { ReferenceTableFile, ReferenceTableRowsQuery } from '../../api/referenceTables/types';

/** The org's dashboards of an integration (by logo, its catalog or integrations-core id): installed by the server once it sends data. */
export function useIntegrationDashboards(ids: string[]) {
  return useQuery({
    queryKey: ['dashboards', 'integration', ...ids],
    queryFn: async () => {
      const result = [];
      for (let offset = 0; ; offset += 200) {
        const page = await searchDashboards({ query: '', preset: 'integrations', listId: null, team: null, sort: { field: 'title', dir: 'asc' }, offset, limit: 200 });
        result.push(...page.dashboards.filter((d) => d.logo && ids.includes(d.logo)));
        if (offset + page.dashboards.length >= page.total || !page.dashboards.length) break;
      }
      return result;
    },
  });
}

/** The integration's integrations-core documentation (static, loaded once). */
export function useIntegrationDocs(coreId: string | undefined) {
  return useQuery({ queryKey: ['integrations', 'docs', coreId], queryFn: () => loadIntegrationDocs(coreId!), enabled: !!coreId, staleTime: Infinity });
}

/** The Agent check's configuration spec from integrations-core (static, loaded once). */
export function useAgentSpec(check: string | undefined) {
  return useQuery({ queryKey: ['integrations', 'agent-spec', check], queryFn: () => loadAgentSpec(check!), enabled: !!check, staleTime: Infinity });
}

export const integrationKeys = {
  all: ['integrations'] as const,
  catalog: (req: CatalogRequest) => [...integrationKeys.all, 'catalog', req] as const,
  categories: () => [...integrationKeys.all, 'categories'] as const,
  detail: (id: string) => [...integrationKeys.all, 'detail', id] as const,
};

export function useCatalog(req: CatalogRequest) {
  return useQuery({ queryKey: integrationKeys.catalog(req), queryFn: () => searchCatalog(req), placeholderData: keepPreviousData, refetchInterval: 60_000 });
}

export function useCategories() {
  return useQuery({ queryKey: integrationKeys.categories(), queryFn: listCategories, staleTime: Infinity });
}

export function useIntegration(id: string) {
  return useQuery({ queryKey: integrationKeys.detail(id), queryFn: () => getIntegration(id), refetchInterval: 60_000 });
}

/** Install / uninstall; the catalog and the detail are refetched afterwards. */
export function useInstallToggle(id: string) {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (install: boolean) => (install ? installIntegration(id) : uninstallIntegration(id)),
    onSuccess: (detail) => {
      qc.setQueryData(integrationKeys.detail(id), detail);
      qc.invalidateQueries({ queryKey: [...integrationKeys.all, 'catalog'] });
    },
  });
}

const referenceTableKeys = {
  all: ['reference-tables'] as const,
  list: () => [...referenceTableKeys.all, 'list'] as const,
  rows: (name: string, q: ReferenceTableRowsQuery) => [...referenceTableKeys.all, 'rows', name, q] as const,
};

export function useReferenceTables() {
  return useQuery({ queryKey: referenceTableKeys.list(), queryFn: listReferenceTables });
}

/** A table and a page of its rows. */
export function useReferenceTable(name: string, q: ReferenceTableRowsQuery) {
  return useQuery({ queryKey: referenceTableKeys.rows(name, q), queryFn: ({ signal }) => getReferenceTable(name, q, signal), placeholderData: keepPreviousData });
}

/** Create, replace and delete; every reference table query is refetched afterwards. */
export function useReferenceTableMutations() {
  const qc = useQueryClient();
  const onSuccess = () => qc.invalidateQueries({ queryKey: referenceTableKeys.all });
  return {
    create: useMutation({ mutationFn: createReferenceTable, onSuccess }),
    replace: useMutation({ mutationFn: ({ name, file }: { name: string; file: ReferenceTableFile }) => replaceReferenceTable(name, file), onSuccess }),
    // The deleted table's rows are dropped rather than refetched (they would answer 404).
    remove: useMutation({
      mutationFn: deleteReferenceTable,
      onSuccess: (_, name) => {
        qc.removeQueries({ queryKey: [...referenceTableKeys.all, 'rows', name] });
        return qc.invalidateQueries({ queryKey: referenceTableKeys.list() });
      },
    }),
  };
}
