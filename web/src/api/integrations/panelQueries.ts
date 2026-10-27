import type { QueryClient } from '@tanstack/react-query';
import type { MetricCatalogEntry, MetricSeries, TimeseriesRequest } from '../dashboards/types';

export interface PanelDataSource {
  catalog: (signal?: AbortSignal) => Promise<MetricCatalogEntry[]>;
  timeseries: (request: TimeseriesRequest, signal?: AbortSignal) => Promise<MetricSeries[]>;
}

/** Share work across panels using the session's QueryClient (cleared on sign-out/org switch). */
export function cachedPanelSource(client: QueryClient, source: PanelDataSource): PanelDataSource {
  const deadline = (signal: AbortSignal) => AbortSignal.any([signal, AbortSignal.timeout(30_000)]);
  return {
    catalog: () =>
      client.fetchQuery({
        queryKey: ['integration-metrics', 'catalog'],
        queryFn: ({ signal }) => source.catalog(deadline(signal)),
        staleTime: 60_000,
        gcTime: 60_000,
        retry: false,
      }),
    timeseries: (request) =>
      client.fetchQuery({
        queryKey: ['integration-metrics', 'timeseries', request],
        queryFn: ({ signal }) => source.timeseries(request, deadline(signal)),
        staleTime: 60_000,
        gcTime: 60_000,
        retry: false,
      }),
  };
}
