// Live state is derived from organization-scoped telemetry and from what the org installed (Integrations → Install
// installs an integration's log pipelines and dashboards). Catalog templates do not install collectors.
import * as mock from '../mock/integrations';
import { API_MODE, del, get, put } from '../http';
import { hasReportedMetrics, reportedMetricNames, waitingForData, type ReportedMetrics } from './detection';
import type { IntegrationDetail, IntegrationHost, IntegrationStatus, IntegrationSummary } from './types';

async function reportedMetrics(): Promise<ReportedMetrics> {
  const metrics = await get('/api/v1/metrics', {});
  return reportedMetricNames(metrics);
}

async function installedIds(): Promise<Set<string>> {
  return new Set(await get('/api/v1/integrations/installed', {}));
}

function liveStatus(id: string, installed: Set<string>, detected: boolean): IntegrationStatus {
  return installed.has(id) ? 'installed' : detected ? 'detected' : 'available';
}

/** The status and data state of an integration from what the org installed and the metrics it received. */
function liveState(id: string, declared: { name: string }[], reported: ReportedMetrics, installed: Set<string>): Pick<IntegrationSummary, 'status' | 'waitingForData'> {
  const prefixes = mock.catalogPrefixes(id);
  return {
    status: liveStatus(id, installed, hasReportedMetrics(id, declared, reported, prefixes)),
    waitingForData: waitingForData(id, installed.has(id), declared, reported, prefixes),
  };
}

/** The hosts whose Datadog Agent reported running the check in the last day, with its status there. */
async function checkHosts(check: string): Promise<IntegrationHost[]> {
  const hosts = await get('/api/v1/integrations/checks/{check}/hosts', { check });
  return hosts.map((h) => ({
    host: h.host,
    status: h.check.status === 'OK' ? 'ok' : h.check.status === 'WARNING' ? 'warning' : 'error',
    collectorVersion: h.agentVersion ? `Datadog Agent ${h.agentVersion}` : 'Datadog Agent',
    lastReported: h.lastReported,
    message: h.check.message ?? undefined,
  }));
}

/**
 * The whole catalog with each integration's status. The catalog is bounded (hundreds of entries), so searches filter
 * this list in the browser (`filterCatalog`) instead of fetching and detecting again per keystroke.
 */
export async function listCatalog(): Promise<IntegrationSummary[]> {
  if (API_MODE === 'mock') return mock.listCatalog();
  const [reported, installed] = await Promise.all([reportedMetrics(), installedIds()]);
  return mock.catalogMetadata().map((entry) => ({ ...entry, ...liveState(entry.id, mock.declaredMetrics(entry.id), reported, installed) }));
}

export const listCategories = mock.listCategories;

/** Static documentation bundled with the UI (integrations-core), the same in mock and live mode. */
export { loadIntegrationDocs } from './catalog';

export async function getIntegration(id: string): Promise<IntegrationDetail> {
  if (API_MODE === 'mock') return mock.getIntegration(id);
  const detail = mock.integrationMetadata(id);
  const [reported, installed, hosts] = await Promise.all([reportedMetrics(), installedIds(), detail.agent ? checkHosts(detail.agent.check) : []]);
  return { ...detail, ...liveState(id, detail.metrics, reported, installed), hosts };
}

/** Installs an integration's log pipelines and dashboards; collection itself is configured on the Agents. */
export async function installIntegration(id: string): Promise<IntegrationDetail> {
  if (API_MODE === 'mock') return mock.installIntegration(id);
  await put('/api/v1/integrations/{id}/install', { id }, undefined as never);
  return getIntegration(id);
}

/** Removes an integration's log pipelines; its dashboards stay. */
export async function uninstallIntegration(id: string): Promise<IntegrationDetail> {
  if (API_MODE === 'mock') return mock.uninstallIntegration(id);
  await del('/api/v1/integrations/{id}/install', { id });
  return getIntegration(id);
}
