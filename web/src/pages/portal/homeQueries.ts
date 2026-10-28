// TanStack Query bindings for the Developer Portal home page and its settings.
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import {
  createPortalApp,
  deletePortalApp,
  deletePortalConnector,
  getPortalApps,
  getPortalConnectors,
  getPortalEntities,
  getPortalIdentities,
  getPortalPullRequests,
  getPortalSources,
  getPortalTickets,
  savePortalConnector,
  savePortalIdentities,
  setPortalFavorite,
  testPortalConnector,
  updatePortalApp,
  type CodeHost,
  type PortalAppInput,
  type PortalConnectorInput,
  type PortalIdentitiesInput,
  type PortalProvider,
  type TicketSource,
} from '../../api/portal/client';

export const portalKeys = {
  all: ['portal'] as const,
  prs: (p: CodeHost) => [...portalKeys.all, 'prs', p] as const,
  tickets: (p: TicketSource) => [...portalKeys.all, 'tickets', p] as const,
  entities: () => [...portalKeys.all, 'entities'] as const,
  apps: () => [...portalKeys.all, 'apps'] as const,
  connectors: () => [...portalKeys.all, 'connectors'] as const,
  identities: () => [...portalKeys.all, 'identities'] as const,
  sources: () => [...portalKeys.all, 'sources'] as const,
};

export const usePullRequests = (p: CodeHost) => useQuery({ queryKey: portalKeys.prs(p), queryFn: ({ signal }) => getPortalPullRequests(p, signal), staleTime: 60_000, retry: 1 });
export const useTickets = (p: TicketSource) => useQuery({ queryKey: portalKeys.tickets(p), queryFn: ({ signal }) => getPortalTickets(p, signal), staleTime: 60_000, retry: 1 });
export const useEntities = () => useQuery({ queryKey: portalKeys.entities(), queryFn: ({ signal }) => getPortalEntities(signal), staleTime: 60_000, retry: 1 });
export const useApps = () => useQuery({ queryKey: portalKeys.apps(), queryFn: getPortalApps, retry: 1 });
export const useConnectors = () => useQuery({ queryKey: portalKeys.connectors(), queryFn: getPortalConnectors, retry: 1 });
export const useIdentities = () => useQuery({ queryKey: portalKeys.identities(), queryFn: getPortalIdentities, retry: 1 });
export const useSources = () => useQuery({ queryKey: portalKeys.sources(), queryFn: getPortalSources, staleTime: 60_000, retry: 1 });

/** Every portal list depends on connectors and identities, so writes refresh the whole portal cache. */
function usePortalMutation<V, R>(fn: (v: V) => Promise<R>) {
  const qc = useQueryClient();
  return useMutation({ mutationFn: fn, onSuccess: () => qc.invalidateQueries({ queryKey: portalKeys.all }) });
}

export const useSaveConnector = () => usePortalMutation(({ provider, input }: { provider: PortalProvider; input: PortalConnectorInput }) => savePortalConnector(provider, input));
export const useDeleteConnector = () => usePortalMutation(deletePortalConnector);
export const useTestConnector = () => usePortalMutation(testPortalConnector);
export const useSaveIdentities = () => usePortalMutation((input: PortalIdentitiesInput) => savePortalIdentities(input));
export const useSetFavorite = () => usePortalMutation(({ entity, favorite }: { entity: string; favorite: boolean }) => setPortalFavorite(entity, favorite));
export const useCreateApp = () => usePortalMutation(createPortalApp);
export const useUpdateApp = () => usePortalMutation(({ id, input }: { id: string; input: PortalAppInput }) => updatePortalApp(id, input));
export const useDeleteApp = () => usePortalMutation(deletePortalApp);
