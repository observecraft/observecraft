// Integrations data access. One function per query-service endpoint:
//   GET    /api/v1/integrations?query&status&category   searchCatalog
//   GET    /api/v1/integrations/categories               listCategories
//   GET    /api/v1/integrations/:id                      getIntegration
//   POST   /api/v1/integrations/:id/install              installIntegration
//   DELETE /api/v1/integrations/:id/install              uninstallIntegration
//   GET    /api/v1/marketplace/apps?query&category       searchMarketplace
// Today they resolve against the in-browser mock.
import * as mock from '../mock/integrations';
import type { CatalogRequest, CatalogResponse, IntegrationDetail, MarketplaceApp, MarketplaceRequest } from './types';

export function searchCatalog(req: CatalogRequest): Promise<CatalogResponse> {
  return mock.searchCatalog(req);
}

export function listCategories(): Promise<{ featured: string[]; all: string[] }> {
  return mock.listCategories();
}

export function getIntegration(id: string): Promise<IntegrationDetail> {
  return mock.getIntegration(id);
}

export function installIntegration(id: string): Promise<IntegrationDetail> {
  return mock.installIntegration(id);
}

export function uninstallIntegration(id: string): Promise<IntegrationDetail> {
  return mock.uninstallIntegration(id);
}

export function searchMarketplace(req: MarketplaceRequest): Promise<MarketplaceApp[]> {
  return mock.searchMarketplace(req);
}
