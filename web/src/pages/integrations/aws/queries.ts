// TanStack Query bindings for Integrations → Amazon Web Services.
import { useMutation, useQueries, useQuery, useQueryClient } from '@tanstack/react-query';
import { createAwsAccount, deleteAwsAccount, getAwsAccountStatus, getAwsSetup, listAwsAccounts, newExternalId, updateAwsAccount, type AwsAccountInput } from '../../../api/aws/client';
import { integrationKeys } from '../queries';

const awsKeys = {
  all: ['aws'] as const,
  setup: () => [...awsKeys.all, 'setup'] as const,
  accounts: () => [...awsKeys.all, 'accounts'] as const,
};

export function useAwsSetup() {
  return useQuery({ queryKey: awsKeys.setup(), queryFn: getAwsSetup, staleTime: Infinity });
}

export function useAwsAccounts() {
  return useQuery({ queryKey: awsKeys.accounts(), queryFn: listAwsAccounts });
}

/** Each account's crawl status (regions, issues), refreshed every minute. */
export function useAwsAccountStatuses(ids: string[]) {
  return useQueries({ queries: ids.map((id) => ({ queryKey: [...awsKeys.all, 'status', id], queryFn: () => getAwsAccountStatus(id), refetchInterval: 60_000 })) });
}

/** A new external id, generated once per add-account page. */
export function useExternalId() {
  return useQuery({ queryKey: [...awsKeys.all, 'external-id'], queryFn: newExternalId, staleTime: Infinity, gcTime: 0 });
}

/** Create, update and delete; the accounts and the catalog are refetched afterwards. */
export function useAwsAccountMutations() {
  const qc = useQueryClient();
  const onSuccess = () => Promise.all([qc.invalidateQueries({ queryKey: awsKeys.accounts() }), qc.invalidateQueries({ queryKey: integrationKeys.all })]);
  return {
    create: useMutation({ mutationFn: createAwsAccount, onSuccess }),
    update: useMutation({ mutationFn: ({ id, input }: { id: string; input: AwsAccountInput }) => updateAwsAccount(id, input), onSuccess }),
    remove: useMutation({ mutationFn: deleteAwsAccount, onSuccess }),
  };
}
