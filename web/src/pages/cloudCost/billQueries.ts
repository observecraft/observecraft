// TanStack Query bindings for Cloud Cost → Settings: the providers' bills ObserveCraft reads.
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { createBillConfig, deleteBillConfig, listBillConfigs, setBillConfigEnabled, type BillInput, type BillProvider } from '../../api/cloudCost/client';

const billKeys = { all: ['cloud-cost', 'bill-configs'] as const };

export function useBillConfigs() {
  return useQuery({ queryKey: billKeys.all, queryFn: ({ signal }) => listBillConfigs(signal) });
}

/** Create, enable / disable and remove; the list is refetched afterwards. */
export function useBillConfigMutations() {
  const qc = useQueryClient();
  const onSuccess = () => qc.invalidateQueries({ queryKey: billKeys.all });
  return {
    create: useMutation({ mutationFn: ({ provider, attributes }: { provider: BillProvider; attributes: BillInput }) => createBillConfig(provider, attributes), onSuccess }),
    setEnabled: useMutation({ mutationFn: ({ provider, id, enabled }: { provider: BillProvider; id: string; enabled: boolean }) => setBillConfigEnabled(provider, id, enabled), onSuccess }),
    remove: useMutation({ mutationFn: ({ provider, id }: { provider: BillProvider; id: string }) => deleteBillConfig(provider, id), onSuccess }),
  };
}
