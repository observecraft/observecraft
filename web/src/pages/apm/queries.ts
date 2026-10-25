// TanStack Query bindings for APM. Ranges are snapped to the minute so live time windows
// don't change the query key every 10 seconds.
import { keepPreviousData, useQuery } from '@tanstack/react-query';
import {
  getDependencies,
  getServiceSummary,
  getTrace,
  listAnomalies,
  listDatabases,
  listDeployments,
  listEnvs,
  listRecommendations,
  listResources,
  listServices,
  listSpanErrors,
  searchSpans,
  serviceLogs,
  exploreSpans,
  spanAnalytics,
  spanFacets,
  spanPercentile,
  spanSummary,
  type SpanMeasure,
  type SpanQuery,
  type ApmScope,
  type ServiceScope,
} from '../../api/apm/client';
import type { TimeRange } from '../../api/types';
import { snapRange } from '../../lib/time';

const scoped = <T extends { range: TimeRange }>(s: T): T => ({ ...s, range: snapRange(s.range) });
const opts = { placeholderData: keepPreviousData, retry: 1 } as const;

export const apmKeys = {
  all: ['apm'] as const,
  of: (name: string, scope: unknown) => [...apmKeys.all, name, scope] as const,
};

export function useServices(s: ApmScope) {
  const q = scoped(s);
  return useQuery({ queryKey: apmKeys.of('services', q), queryFn: ({ signal }) => listServices(q, signal), ...opts });
}

export function useEnvs(range: TimeRange) {
  const r = snapRange(range);
  return useQuery({ queryKey: apmKeys.of('envs', r), queryFn: ({ signal }) => listEnvs(r, signal), ...opts });
}

export function useRecommendations(s: ApmScope) {
  const q = scoped(s);
  return useQuery({ queryKey: apmKeys.of('recommendations', q), queryFn: ({ signal }) => listRecommendations(q, signal), ...opts });
}

export function useAnomalies(s: ApmScope) {
  const q = scoped(s);
  return useQuery({ queryKey: apmKeys.of('anomalies', q), queryFn: ({ signal }) => listAnomalies(q, signal), ...opts });
}

export function useResources(s: ApmScope & { service?: string | null }) {
  const q = scoped(s);
  return useQuery({ queryKey: apmKeys.of('resources', q), queryFn: ({ signal }) => listResources(q, signal), ...opts });
}

export function useSpanErrors(s: ApmScope & { service?: string | null }) {
  const q = scoped(s);
  return useQuery({ queryKey: apmKeys.of('errors', q), queryFn: ({ signal }) => listSpanErrors(q, signal), ...opts });
}

export function useServiceSummary(s: ServiceScope) {
  const q = scoped(s);
  return useQuery({ queryKey: apmKeys.of('service', q), queryFn: ({ signal }) => getServiceSummary(q, signal), ...opts });
}

export function useDeployments(s: ServiceScope) {
  const q = scoped(s);
  return useQuery({ queryKey: apmKeys.of('deployments', q), queryFn: ({ signal }) => listDeployments(q, signal), ...opts });
}

export function useDependencies(s: ServiceScope) {
  const q = scoped(s);
  return useQuery({ queryKey: apmKeys.of('dependencies', q), queryFn: ({ signal }) => getDependencies(q, signal), ...opts });
}

export function useDatabases(s: ServiceScope) {
  const q = scoped(s);
  return useQuery({ queryKey: apmKeys.of('databases', q), queryFn: ({ signal }) => listDatabases(q, signal), ...opts });
}

export function useSpanSearch(req: ApmScope & { query: string; cursor?: string | null; limit?: number }) {
  const q = scoped(req);
  return useQuery({ queryKey: apmKeys.of('spans', q), queryFn: ({ signal }) => searchSpans(q, signal), ...opts });
}

export function useTrace(traceId: string | null) {
  return useQuery({
    queryKey: apmKeys.of('trace', traceId),
    queryFn: ({ signal }) => getTrace(traceId!, signal),
    enabled: !!traceId,
    // A finished trace doesn't change.
    staleTime: Infinity,
    retry: false,
  });
}

export function useServiceLogs(range: TimeRange, service: string) {
  const r = snapRange(range);
  return useQuery({ queryKey: apmKeys.of('logs', [r, service]), queryFn: ({ signal }) => serviceLogs(r, service, signal), ...opts });
}

// ---------------------------------------------------------------- span explorer

const spanScoped = (q: SpanQuery): SpanQuery => ({ ...q, range: snapRange(q.range) });

export function useExploreSpans(q: SpanQuery, sort: string, offset: number, limit: number) {
  const s = spanScoped(q);
  return useQuery({ queryKey: apmKeys.of('explore', [s, sort, offset, limit]), queryFn: ({ signal }) => exploreSpans(s, sort, offset, limit, signal), ...opts });
}

export function useSpanFacets(q: SpanQuery, dimensions: string[]) {
  const s = spanScoped(q);
  return useQuery({ queryKey: apmKeys.of('span-facets', [s, dimensions]), queryFn: ({ signal }) => spanFacets(s, dimensions, signal), ...opts });
}

export function useSpanAnalytics(q: SpanQuery, groupBy: string | null, measure: SpanMeasure, top: number, enabled: boolean) {
  const s = spanScoped(q);
  return useQuery({ queryKey: apmKeys.of('span-analytics', [s, groupBy, measure, top]), queryFn: ({ signal }) => spanAnalytics(s, groupBy, measure, top, signal), enabled, ...opts });
}

export function useSpanSummary(q: SpanQuery, enabled: boolean) {
  const s = spanScoped(q);
  return useQuery({ queryKey: apmKeys.of('span-summary', s), queryFn: ({ signal }) => spanSummary(s, signal), enabled, ...opts });
}

export function useSpanPercentile(service: string, resource: string, durationMs: number, atMs: number) {
  const at = Math.round(atMs);
  return useQuery({
    queryKey: apmKeys.of('percentile', [service, resource, durationMs, at]),
    queryFn: ({ signal }) => spanPercentile(service, resource, durationMs, at, signal),
    staleTime: 300_000,
    retry: 1,
  });
}
