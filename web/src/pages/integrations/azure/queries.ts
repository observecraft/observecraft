// TanStack Query bindings for Integrations → Microsoft Azure.
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import {
  createAzureAccount,
  deleteAzureAccount,
  getAzureAccountStatus,
  getEntraLogs,
  listAzureAccounts,
  saveEntraLogs,
  testEntraLogs,
  updateAzureAccount,
  type AzureAccountInput,
  type EntraLogsInput,
} from '../../../api/azure/client';
import { integrationKeys } from '../queries';

const azureKeys = { all: ['azure'] as const, accounts: () => [...azureKeys.all, 'accounts'] as const };

export function useAzureAccounts() {
  return useQuery({ queryKey: azureKeys.accounts(), queryFn: listAzureAccounts });
}

/** The crawl status per subscription, refreshed every minute. */
export function useAzureAccountStatus(id: string) {
  return useQuery({ queryKey: [...azureKeys.all, 'status', id], queryFn: () => getAzureAccountStatus(id), refetchInterval: 60_000 });
}

/** Create, update and delete; the app registrations and the catalog are refetched afterwards. */
export function useAzureAccountMutations() {
  const qc = useQueryClient();
  const onSuccess = () => Promise.all([qc.invalidateQueries({ queryKey: azureKeys.all }), qc.invalidateQueries({ queryKey: integrationKeys.all })]);
  return {
    create: useMutation({ mutationFn: createAzureAccount, onSuccess }),
    update: useMutation({ mutationFn: ({ id, input }: { id: string; input: AzureAccountInput }) => updateAzureAccount(id, input), onSuccess }),
    remove: useMutation({ mutationFn: deleteAzureAccount, onSuccess }),
  };
}

/** The app registration's Entra ID logs and their streams' polling, refreshed every minute. */
export function useEntraLogs(id: string) {
  return useQuery({ queryKey: [...azureKeys.all, 'entra', id], queryFn: () => getEntraLogs(id), refetchInterval: 60_000 });
}

/** Turn them on or off (the catalog's Microsoft Entra ID tile follows), and test what Graph lets the app registration read. */
export function useEntraLogsMutations(id: string) {
  const qc = useQueryClient();
  const onSuccess = () => Promise.all([qc.invalidateQueries({ queryKey: [...azureKeys.all, 'entra', id] }), qc.invalidateQueries({ queryKey: integrationKeys.all })]);
  return {
    save: useMutation({ mutationFn: (input: EntraLogsInput) => saveEntraLogs(id, input), onSuccess }),
    test: useMutation({ mutationFn: () => testEntraLogs(id) }),
  };
}
