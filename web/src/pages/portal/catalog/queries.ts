// TanStack Query bindings for the Software Catalog and Reports.
import { keepPreviousData, useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { getDependencies, listDeployments } from '../../../api/apm/client';
import { searchCatalog } from '../../../api/catalog/client';
import { searchDashboards } from '../../../api/dashboards/client';
import { searchMonitors } from '../../../api/monitors/client';
import { searchSlos } from '../../../api/slos/client';
import { listOnCallTeams } from '../../../api/onCall/client';
import {
  deleteEntityDefinition,
  getEntityDefinition,
  getPortalCatalog,
  getScorecardsReport,
  getTeamsReport,
  importEntityDefinitions,
  listReportDashboards,
  setReportDashboards,
  updateEntityDefinition,
  upsertEntityDefinition,
  type EntityDefinition,
  type PortalReportRequest,
} from '../../../api/portalCatalog/client';
import type { TimeRange } from '../../../api/types';
import { scorecardKeys } from '../queries';

export const portalKeys = {
  catalog: ['portal', 'catalog'] as const,
  definitions: ['portal', 'definitions'] as const,
  reports: ['portal', 'reports'] as const,
  dashboards: ['portal', 'report-dashboards'] as const,
};

export const usePortalCatalog = (range: TimeRange, env: string | null) =>
  useQuery({
    queryKey: [...portalKeys.catalog, range.from, range.to, env],
    queryFn: ({ signal }) => getPortalCatalog({ range, env }, signal),
    placeholderData: keepPreviousData,
    retry: 1,
  });

export const useEntityDefinition = (id: string | null) => useQuery({ queryKey: [...portalKeys.definitions, id], queryFn: ({ signal }) => getEntityDefinition(id!, signal), enabled: !!id, retry: 1 });

/** Infrastructure resources, for the left navigation's count. */
export const useInfrastructureCount = (range: TimeRange) =>
  useQuery({
    queryKey: ['portal', 'infrastructure-count', Math.floor(range.to / 60_000)],
    queryFn: () => searchCatalog({ range, query: '', offset: 0, limit: 1 }),
    select: (p) => p.total,
    retry: 1,
    staleTime: 60_000,
  });

/** Calls to and from an instrumented service, and its versions (APM). */
export const useEntityDependencies = (service: string | null, range: TimeRange) =>
  useQuery({ queryKey: ['portal', 'dependencies', service, range.from, range.to], queryFn: ({ signal }) => getDependencies({ service: service!, range }, signal), enabled: !!service, retry: 1 });
export const useEntityDeployments = (service: string | null, range: TimeRange) =>
  useQuery({ queryKey: ['portal', 'deployments', service, range.from, range.to], queryFn: ({ signal }) => listDeployments({ service: service!, range }, signal), enabled: !!service, retry: 1 });

/** On-Call teams (schedules and who is on call now), for ownership columns and reports. */
export const useOnCallTeams = () => useQuery({ queryKey: ['portal', 'on-call-teams'], queryFn: listOnCallTeams, retry: 1, staleTime: 60_000 });

function useInvalidating<V, R>(fn: (v: V) => Promise<R>) {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: fn,
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ['portal'] });
      // Service definitions write the service metadata Scorecards reads.
      qc.invalidateQueries({ queryKey: scorecardKeys.all });
    },
  });
}

export const useUpsertDefinition = () => useInvalidating((def: EntityDefinition) => upsertEntityDefinition(def));
export const useUpdateDefinition = () => useInvalidating(({ id, def }: { id: string; def: EntityDefinition }) => updateEntityDefinition(id, def));
export const useDeleteDefinition = () => useInvalidating((id: string) => deleteEntityDefinition(id));
export const useImportDefinitions = () => useInvalidating((content: string) => importEntityDefinitions(content));

export const useTeamsReport = (req: PortalReportRequest) =>
  useQuery({ queryKey: [...portalKeys.reports, 'teams', req], queryFn: ({ signal }) => getTeamsReport(req, signal), placeholderData: keepPreviousData, retry: 1 });
export const useScorecardsReport = (req: PortalReportRequest) =>
  useQuery({ queryKey: [...portalKeys.reports, 'scorecards', req], queryFn: ({ signal }) => getScorecardsReport(req, signal), placeholderData: keepPreviousData, retry: 1 });
/** Every SLO (status over its primary window) and monitor, for Reliability Overview. */
export const useReportSlos = () =>
  useQuery({
    queryKey: [...portalKeys.reports, 'slos'],
    queryFn: () => searchSlos({ query: '', offset: 0, limit: 500, burnRates: [], creators: [], envs: [], services: [], states: [], tags: [], teams: [], types: [], sort: null, window: null }),
    retry: 1,
  });
export const useReportMonitors = () =>
  useQuery({
    queryKey: [...portalKeys.reports, 'monitors'],
    queryFn: () => searchMonitors({ query: '', statuses: [], muted: [], priorities: [], types: [], drafts: [false], recipients: [], creators: [], teams: [], sort: 'name', offset: 0, limit: 500 }),
    retry: 1,
  });
/** Dashboards to pick for Custom Reports. */
export const useDashboardChoices = (query: string) =>
  useQuery({
    queryKey: ['portal', 'dashboard-choices', query],
    queryFn: () => searchDashboards({ query, preset: 'all', listId: null, team: null, sort: { field: 'title', dir: 'asc' }, offset: 0, limit: 100 }),
    placeholderData: keepPreviousData,
    retry: 1,
  });

export const useReportDashboards = () => useQuery({ queryKey: portalKeys.dashboards, queryFn: ({ signal }) => listReportDashboards(signal), retry: 1 });
export function useSetReportDashboards() {
  const qc = useQueryClient();
  return useMutation({ mutationFn: setReportDashboards, onSuccess: (data) => qc.setQueryData(portalKeys.dashboards, data) });
}
