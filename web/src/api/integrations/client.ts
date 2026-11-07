// Live state is derived from organization-scoped telemetry. Catalog templates do not install collectors.
import * as mock from '../mock/integrations';
import { API_MODE, get } from '../http';
import { hasReportedMetrics, reportedMetricNames, type ReportedMetrics } from './detection';
import type { IntegrationDetail, IntegrationSummary } from './types';

async function reportedMetrics(): Promise<ReportedMetrics> {
  const metrics = await get('/api/v1/metrics', {});
  return reportedMetricNames(metrics);
}

/**
 * The whole catalog with each integration's status. The catalog is bounded (hundreds of entries), so searches filter
 * this list in the browser (`filterCatalog`) instead of fetching and detecting again per keystroke.
 */
export async function listCatalog(): Promise<IntegrationSummary[]> {
  if (API_MODE === 'mock') return mock.listCatalog();
  const reported = await reportedMetrics();
  return mock.catalogMetadata().map((entry) => ({
    ...entry,
    status: hasReportedMetrics(entry.id, mock.declaredMetrics(entry.id), reported, mock.catalogPrefixes(entry.id)) ? 'detected' : 'available',
  }));
}

export const listCategories = mock.listCategories;

/** Static documentation bundled with the UI (integrations-core), the same in mock and live mode. */
export { loadIntegrationDocs } from './catalog';

export async function getIntegration(id: string): Promise<IntegrationDetail> {
  if (API_MODE === 'mock') return mock.getIntegration(id);
  const detail = mock.integrationMetadata(id);
  return { ...detail, status: hasReportedMetrics(id, detail.metrics, await reportedMetrics(), mock.catalogPrefixes(id)) ? 'detected' : 'available' };
}

export function installIntegration(id: string): Promise<IntegrationDetail> {
  if (API_MODE === 'mock') return mock.installIntegration(id);
  return Promise.reject(new Error('Apply the collector configuration manually. Remote installation is not available.'));
}

export function uninstallIntegration(id: string): Promise<IntegrationDetail> {
  if (API_MODE === 'mock') return mock.uninstallIntegration(id);
  return Promise.reject(new Error('Remove the configuration from your collector to stop collection.'));
}
