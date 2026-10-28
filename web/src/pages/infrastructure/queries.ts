// TanStack Query bindings for Infrastructure.
import { keepPreviousData, useQuery } from '@tanstack/react-query';
import { searchCatalog, type CatalogSearchRequest } from '../../api/catalog/client';
import { getHost, getHostMetrics, listContainers, listPods, listProcesses, searchHosts } from '../../api/infrastructure/client';
import type { HostSearchRequest, PageRequest } from '../../api/infrastructure/types';
import type { TimeRange } from '../../api/types';

export const infraKeys = {
  all: ['infrastructure'] as const,
  hosts: (req: HostSearchRequest) => [...infraKeys.all, 'hosts', req] as const,
  host: (name: string) => [...infraKeys.all, 'host', name] as const,
  metrics: (name: string, range: TimeRange) => [...infraKeys.all, 'metrics', name, range] as const,
  pods: (req: PageRequest) => [...infraKeys.all, 'pods', req] as const,
  containers: (req: PageRequest) => [...infraKeys.all, 'containers', req] as const,
  processes: (req: PageRequest) => [...infraKeys.all, 'processes', req] as const,
  catalog: (req: CatalogSearchRequest) => [...infraKeys.all, 'catalog', req] as const,
};

export function useHosts(req: HostSearchRequest) {
  return useQuery({ queryKey: infraKeys.hosts(req), queryFn: () => searchHosts(req), placeholderData: keepPreviousData });
}

export function useHost(name: string) {
  return useQuery({ queryKey: infraKeys.host(name), queryFn: () => getHost(name) });
}

export function useHostMetrics(name: string, range: TimeRange) {
  return useQuery({ queryKey: infraKeys.metrics(name, range), queryFn: () => getHostMetrics(name, range), placeholderData: keepPreviousData });
}

export function usePods(req: PageRequest) {
  return useQuery({ queryKey: infraKeys.pods(req), queryFn: () => listPods(req), placeholderData: keepPreviousData });
}

export function useContainers(req: PageRequest) {
  return useQuery({ queryKey: infraKeys.containers(req), queryFn: () => listContainers(req), placeholderData: keepPreviousData });
}

export function useProcesses(req: PageRequest) {
  return useQuery({ queryKey: infraKeys.processes(req), queryFn: () => listProcesses(req), placeholderData: keepPreviousData });
}

export function useCatalog(req: CatalogSearchRequest) {
  return useQuery({ queryKey: infraKeys.catalog(req), queryFn: () => searchCatalog(req), placeholderData: keepPreviousData });
}
