// Integrations → Microsoft Azure. Outside live mode the in-browser mock (mock/azure.ts) answers.
import { API_MODE, del, get, post, put } from '../http';
import * as mock from '../mock/azure';
import type { AzureAccount, AzureAccountInput, AzureAccountStatus } from './types';

export * from './types';

const live = API_MODE === 'live';

export const listAzureAccounts = (): Promise<AzureAccount[]> => (live ? (get('/api/v1/integrations/azure/accounts', {}) as Promise<unknown> as Promise<AzureAccount[]>) : mock.listAzureAccounts());
export const createAzureAccount = (input: AzureAccountInput): Promise<AzureAccount> =>
  live ? (post('/api/v1/integrations/azure/accounts', input as never) as Promise<unknown> as Promise<AzureAccount>) : mock.createAzureAccount(input);
export const updateAzureAccount = (id: string, input: AzureAccountInput): Promise<AzureAccount> =>
  live ? (put('/api/v1/integrations/azure/accounts/{id}', { id }, input as never) as Promise<unknown> as Promise<AzureAccount>) : mock.updateAzureAccount(id, input);
export const deleteAzureAccount = (id: string): Promise<void> => (live ? del('/api/v1/integrations/azure/accounts/{id}', { id }) : mock.deleteAzureAccount(id));
export const getAzureAccountStatus = (id: string): Promise<AzureAccountStatus> => (live ? get('/api/v1/integrations/azure/accounts/{id}/status', { id }) : mock.getAzureAccountStatus(id));
