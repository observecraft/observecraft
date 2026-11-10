// Integrations → Microsoft Azure. Outside live mode the in-browser mock (mock/azure.ts) answers.
import { API_MODE, del, get, post, postTo, put } from '../http';
import * as mock from '../mock/azure';
import * as entraMock from '../mock/entra';
import type { AzureAccount, AzureAccountInput, AzureAccountStatus, EntraLogs, EntraLogsInput, EntraLogsTest } from './types';

export * from './types';

const live = API_MODE === 'live';

export const listAzureAccounts = (): Promise<AzureAccount[]> => (live ? (get('/api/v1/integrations/azure/accounts', {}) as Promise<unknown> as Promise<AzureAccount[]>) : mock.listAzureAccounts());
export const createAzureAccount = (input: AzureAccountInput): Promise<AzureAccount> =>
  live ? (post('/api/v1/integrations/azure/accounts', input as never) as Promise<unknown> as Promise<AzureAccount>) : mock.createAzureAccount(input);
export const updateAzureAccount = (id: string, input: AzureAccountInput): Promise<AzureAccount> =>
  live ? (put('/api/v1/integrations/azure/accounts/{id}', { id }, input as never) as Promise<unknown> as Promise<AzureAccount>) : mock.updateAzureAccount(id, input);
export const deleteAzureAccount = (id: string): Promise<void> => (live ? del('/api/v1/integrations/azure/accounts/{id}', { id }) : mock.deleteAzureAccount(id));
export const getAzureAccountStatus = (id: string): Promise<AzureAccountStatus> => (live ? get('/api/v1/integrations/azure/accounts/{id}/status', { id }) : mock.getAzureAccountStatus(id));

// An app registration's Entra ID logs (routes/entra_logs.rs; mock/entra.ts).
export const getEntraLogs = (id: string): Promise<EntraLogs> => (live ? get('/api/v1/integrations/azure/accounts/{id}/entra-logs', { id }) : entraMock.getEntraLogs(id));
export const saveEntraLogs = (id: string, input: EntraLogsInput): Promise<EntraLogs> =>
  live ? put('/api/v1/integrations/azure/accounts/{id}/entra-logs', { id }, input) : entraMock.saveEntraLogs(id, input);
export const testEntraLogs = (id: string): Promise<EntraLogsTest> => (live ? postTo('/api/v1/integrations/azure/accounts/{id}/entra-logs/test', { id }, undefined) : entraMock.testEntraLogs());
