// TanStack Query bindings for Infrastructure.
import { keepPreviousData, useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { searchCatalog, type CatalogSearchRequest } from '../../api/catalog/client';
import { searchChanges, type ChangeSearchRequest } from '../../api/changes/client';
import { getCloudMap, type CloudMapRequest } from '../../api/cloudMap/client';
import {
  createPolicy,
  deletePolicy,
  getPolicy,
  policyAttributes,
  previewPolicy,
  searchPolicies,
  updatePolicy,
  type ResourcePolicyInput,
  type ResourcePolicyPreviewRequest,
} from '../../api/policies/client';
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
  changes: (req: ChangeSearchRequest) => [...infraKeys.all, 'changes', req] as const,
  policies: ['infrastructure', 'policies'] as const,
  policyList: (query: string) => [...infraKeys.policies, 'list', query] as const,
  policy: (id: string, query: string) => [...infraKeys.policies, 'detail', id, query] as const,
  policyAttributes: () => [...infraKeys.policies, 'attributes'] as const,
  policyPreview: (req: ResourcePolicyPreviewRequest) => [...infraKeys.policies, 'preview', req] as const,
  cloudMap: (req: CloudMapRequest) => [...infraKeys.all, 'cloud-map', req] as const,
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

export function useChanges(req: ChangeSearchRequest) {
  return useQuery({ queryKey: infraKeys.changes(req), queryFn: () => searchChanges(req), placeholderData: keepPreviousData });
}

export function usePolicies(query: string) {
  return useQuery({ queryKey: infraKeys.policyList(query), queryFn: ({ signal }) => searchPolicies(query, signal), placeholderData: keepPreviousData });
}

export function usePolicy(id: string, query: string) {
  return useQuery({ queryKey: infraKeys.policy(id, query), queryFn: ({ signal }) => getPolicy(id, query, signal), enabled: !!id, placeholderData: keepPreviousData });
}

export function usePolicyAttributes() {
  return useQuery({ queryKey: infraKeys.policyAttributes(), queryFn: ({ signal }) => policyAttributes(signal), staleTime: 60_000 });
}

/** The editor's live score; `req` is null while the rule is incomplete. */
export function usePolicyPreview(req: ResourcePolicyPreviewRequest | null) {
  return useQuery({
    queryKey: infraKeys.policyPreview(req ?? { resourceType: 'host', conditions: [] }),
    queryFn: ({ signal }) => previewPolicy(req!, signal),
    enabled: !!req,
    placeholderData: keepPreviousData,
    retry: false,
  });
}

export function useSavePolicy(id?: string) {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (input: ResourcePolicyInput) => (id ? updatePolicy(id, input) : createPolicy(input)),
    onSuccess: () => qc.invalidateQueries({ queryKey: infraKeys.policies }),
  });
}

export function useDeletePolicy() {
  const qc = useQueryClient();
  return useMutation({ mutationFn: deletePolicy, onSuccess: () => qc.invalidateQueries({ queryKey: infraKeys.policies }) });
}

export function useCloudMap(req: CloudMapRequest) {
  return useQuery({ queryKey: infraKeys.cloudMap(req), queryFn: ({ signal }) => getCloudMap(req, signal), placeholderData: keepPreviousData });
}
