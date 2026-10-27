// TanStack Query bindings for Integrations / Marketplace.
import { keepPreviousData, useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { getIntegration, installIntegration, listCategories, searchCatalog, searchMarketplace, uninstallIntegration } from '../../api/integrations/client';
import type { CatalogRequest, MarketplaceRequest } from '../../api/integrations/types';
import { searchDashboards } from '../../api/dashboards/client';
import { integrationDashboards } from '../../api/integrations/dashboards';

export function useIntegrationDashboards(id: string) {
  const bundled = integrationDashboards.filter((d) => d.logo === id);
  return useQuery({
    queryKey: ['dashboards', 'integration', id],
    queryFn: async () => {
      const result = [];
      for (let offset = 0; ; offset += 200) {
        const page = await searchDashboards({ query: '', preset: 'integrations', listId: null, team: null, sort: { field: 'title', dir: 'asc' }, offset, limit: 200 });
        result.push(...page.dashboards.filter((d) => d.logo === id && bundled.some((b) => b.title === d.title)));
        if (offset + page.dashboards.length >= page.total || !page.dashboards.length) break;
      }
      return result;
    },
    enabled: bundled.length > 0,
  });
}

export const integrationKeys = {
  all: ['integrations'] as const,
  catalog: (req: CatalogRequest) => [...integrationKeys.all, 'catalog', req] as const,
  categories: () => [...integrationKeys.all, 'categories'] as const,
  detail: (id: string) => [...integrationKeys.all, 'detail', id] as const,
  marketplace: (req: MarketplaceRequest) => [...integrationKeys.all, 'marketplace', req] as const,
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

export function useMarketplace(req: MarketplaceRequest) {
  return useQuery({ queryKey: integrationKeys.marketplace(req), queryFn: () => searchMarketplace(req), placeholderData: keepPreviousData });
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
