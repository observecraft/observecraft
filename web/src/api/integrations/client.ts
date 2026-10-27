// Live state is derived from organization-scoped telemetry. Catalog templates do not install collectors.
import * as mock from '../mock/integrations';
import { API_MODE, get } from '../http';
import { hasReportedMetrics, reportedMetricNames } from './detection';
import type { CatalogRequest, CatalogResponse, IntegrationDetail, MarketplaceApp, MarketplaceRequest, IntegrationSummary } from './types';

async function reportedMetrics(): Promise<Set<string>> {
  const metrics = await get('/api/v1/metrics', {});
  return reportedMetricNames(metrics);
}

export async function searchCatalog(req: CatalogRequest): Promise<CatalogResponse> {
  if (API_MODE === 'mock') return mock.searchCatalog(req);
  const names = await reportedMetrics();
  const q = req.query.trim().toLowerCase();
  const integrations = mock
    .catalogMetadata()
    .map((entry): IntegrationSummary => ({
      ...entry,
      status: hasReportedMetrics(entry.id, mock.integrationMetadata(entry.id).metrics, names) ? 'detected' : 'available',
    }))
    .filter((s) => req.status === 'all' || s.status === req.status)
    .filter((s) => !req.category || s.categories.includes(req.category))
    .filter((s) => !q || s.name.toLowerCase().includes(q) || s.categories.some((c) => c.toLowerCase().includes(q)))
    .sort((a, b) => a.name.localeCompare(b.name));
  return { integrations };
}

export const listCategories = mock.listCategories;

export async function getIntegration(id: string): Promise<IntegrationDetail> {
  if (API_MODE === 'mock') return mock.getIntegration(id);
  const detail = mock.integrationMetadata(id);
  return { ...detail, status: hasReportedMetrics(id, detail.metrics, await reportedMetrics()) ? 'detected' : 'available' };
}

export function installIntegration(id: string): Promise<IntegrationDetail> {
  if (API_MODE === 'mock') return mock.installIntegration(id);
  return Promise.reject(new Error('Apply the collector configuration manually. Remote installation is not available.'));
}

export function uninstallIntegration(id: string): Promise<IntegrationDetail> {
  if (API_MODE === 'mock') return mock.uninstallIntegration(id);
  return Promise.reject(new Error('Remove the configuration from your collector to stop collection.'));
}

export function searchMarketplace(req: MarketplaceRequest): Promise<MarketplaceApp[]> {
  return API_MODE === 'mock' ? mock.searchMarketplace(req) : Promise.resolve([]);
}
