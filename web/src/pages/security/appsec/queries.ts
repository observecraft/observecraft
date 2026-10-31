// TanStack Query bindings for App & API Protection.
import { keepPreviousData, useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import * as api from '../../../api/appsec/client';
import type { DenylistInput, TimeRange, WafKind } from '../../../api/appsec/client';

const keys = { all: ['appsec'] as const };

function useDone() {
  const qc = useQueryClient();
  return () => qc.invalidateQueries({ queryKey: keys.all });
}

export const useAppsecOverview = (range: TimeRange) => useQuery({ queryKey: [...keys.all, 'overview', range], queryFn: () => api.getAppsecOverview(range), placeholderData: keepPreviousData });
export const useSecurityTraces = (range: TimeRange, query: string) =>
  useQuery({ queryKey: [...keys.all, 'traces', range, query], queryFn: () => api.searchSecurityTraces(range, query), placeholderData: keepPreviousData });
export const useAttackers = (range: TimeRange, query: string) =>
  useQuery({ queryKey: [...keys.all, 'attackers', range, query], queryFn: () => api.listAttackers(range, query), placeholderData: keepPreviousData });
export const useAppsecUsers = (range: TimeRange, query: string) =>
  useQuery({ queryKey: [...keys.all, 'users', range, query], queryFn: () => api.listAppsecUsers(range, query), placeholderData: keepPreviousData });
export const useApiFindings = () => useQuery({ queryKey: [...keys.all, 'findings'], queryFn: api.listApiFindings });
export const useApiEndpoints = () => useQuery({ queryKey: [...keys.all, 'endpoints'], queryFn: api.listApiEndpoints });
export const useAppsecServices = () => useQuery({ queryKey: [...keys.all, 'services'], queryFn: api.listAppsecServices });
export const useBusinessLogic = () => useQuery({ queryKey: [...keys.all, 'business-logic'], queryFn: api.getBusinessLogic });
export const useWafPolicies = () => useQuery({ queryKey: [...keys.all, 'policies'], queryFn: api.listWafPolicies });
export const useDenylist = () => useQuery({ queryKey: [...keys.all, 'denylist'], queryFn: api.listDenylist });
export const useWaf = (kind: WafKind) => useQuery({ queryKey: [...keys.all, 'waf', kind], queryFn: () => api.listWaf(kind) });
export const useAppsecSetting = <T>(key: string) => useQuery({ queryKey: [...keys.all, 'setting', key], queryFn: () => api.getAppsecSetting<T>(key) });

export function useBlock() {
  const done = useDone();
  return useMutation({ mutationFn: (input: DenylistInput) => api.blockEntities(input), onSuccess: done });
}
export function useUnblock() {
  const done = useDone();
  return useMutation({ mutationFn: ({ kind, value }: { kind: string; value: string }) => api.unblockEntity(kind, value), onSuccess: done });
}
export function useSaveWaf(kind: WafKind, id?: string) {
  const done = useDone();
  return useMutation({ mutationFn: (a: Record<string, unknown>) => (id ? api.updateWaf(kind, id, a) : api.createWaf(kind, a)), onSuccess: done });
}
/** Updates any object of a kind: `{id, attributes}`. */
export function useUpdateWaf(kind: WafKind) {
  const done = useDone();
  return useMutation({ mutationFn: ({ id, attributes }: { id: string; attributes: Record<string, unknown> }) => api.updateWaf(kind, id, attributes), onSuccess: done });
}
export function useDeleteWaf(kind: WafKind) {
  const done = useDone();
  return useMutation({ mutationFn: (id: string) => api.deleteWaf(kind, id), onSuccess: done });
}
export function useSaveSetting(key: string) {
  const done = useDone();
  return useMutation({ mutationFn: (v: unknown) => api.setAppsecSetting(key, v), onSuccess: done });
}
export function useTriageFinding() {
  const done = useDone();
  return useMutation({ mutationFn: ({ key, status }: { key: string; status: 'open' | 'muted' }) => api.triageApiFinding(key, status), onSuccess: done });
}
