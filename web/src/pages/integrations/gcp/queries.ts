// TanStack Query bindings for Integrations → Google Cloud.
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { createGcpAccount, deleteGcpAccount, getGcpAccountStatus, getGcpSetup, listGcpAccounts, updateGcpAccount, type GcpAccountInput } from '../../../api/gcp/client';
import { integrationKeys } from '../queries';

const gcpKeys = { all: ['gcp'] as const, accounts: () => [...gcpKeys.all, 'accounts'] as const };

export function useGcpSetup() {
  return useQuery({ queryKey: [...gcpKeys.all, 'setup'], queryFn: getGcpSetup, staleTime: Infinity });
}

export function useGcpAccounts() {
  return useQuery({ queryKey: gcpKeys.accounts(), queryFn: listGcpAccounts });
}

/** The crawl status per project, refreshed every minute. */
export function useGcpAccountStatus(id: string) {
  return useQuery({ queryKey: [...gcpKeys.all, 'status', id], queryFn: () => getGcpAccountStatus(id), refetchInterval: 60_000 });
}

/** Create, update and delete; the service accounts and the catalog are refetched afterwards. */
export function useGcpAccountMutations() {
  const qc = useQueryClient();
  const onSuccess = () => Promise.all([qc.invalidateQueries({ queryKey: gcpKeys.all }), qc.invalidateQueries({ queryKey: integrationKeys.all })]);
  return {
    create: useMutation({ mutationFn: createGcpAccount, onSuccess }),
    update: useMutation({ mutationFn: ({ id, input }: { id: string; input: GcpAccountInput }) => updateGcpAccount(id, input), onSuccess }),
    remove: useMutation({ mutationFn: deleteGcpAccount, onSuccess }),
  };
}
