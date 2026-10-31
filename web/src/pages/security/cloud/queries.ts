// TanStack Query bindings for Cloud Security.
import { keepPreviousData, useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import {
  getCloudSecuritySummary,
  getCve,
  listInfraPackages,
  listInfraVulnerabilities,
  listMisconfigurations,
  listResourceCategories,
  searchCves,
  triageFinding,
  type CveQuery,
  type FindingStatus,
} from '../../../api/cloudSecurity/client';

const keys = { all: ['cloud-security'] as const };

export const useCloudSummary = () => useQuery({ queryKey: [...keys.all, 'summary'], queryFn: getCloudSecuritySummary, refetchInterval: 120_000 });
export const useMisconfigurations = (status: FindingStatus) =>
  useQuery({ queryKey: [...keys.all, 'misconfigs', status], queryFn: () => listMisconfigurations(status), placeholderData: keepPreviousData });
export const useInfraVulnerabilities = (status: FindingStatus) =>
  useQuery({ queryKey: [...keys.all, 'vulns', status], queryFn: () => listInfraVulnerabilities(status), placeholderData: keepPreviousData });
export const useResourceCategories = () => useQuery({ queryKey: [...keys.all, 'resources'], queryFn: listResourceCategories });
export const useInfraPackages = () => useQuery({ queryKey: [...keys.all, 'packages'], queryFn: listInfraPackages });
export const useCves = (q: CveQuery) => useQuery({ queryKey: [...keys.all, 'cves', q], queryFn: () => searchCves(q), placeholderData: keepPreviousData });
export const useCve = (id: string | null) => useQuery({ queryKey: [...keys.all, 'cve', id], queryFn: () => getCve(id!), enabled: !!id });

export function useTriage() {
  const qc = useQueryClient();
  return useMutation({ mutationFn: ({ key, status }: { key: string; status: 'open' | 'muted' }) => triageFinding(key, status), onSuccess: () => qc.invalidateQueries({ queryKey: keys.all }) });
}
