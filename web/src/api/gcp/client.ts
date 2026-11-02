// Integrations → Google Cloud. Outside live mode the in-browser mock (mock/gcp.ts) answers.
import { API_MODE, del, get, patch, post } from '../http';
import * as mock from '../mock/gcp';
import type { GcpAccount, GcpAccountInput, GcpAccountStatus, GcpSetup } from './types';

export * from './types';

const live = API_MODE === 'live';

export const getGcpSetup = (): Promise<GcpSetup> => (live ? get('/api/v1/integrations/gcp/setup', {}) : mock.getGcpSetup());
export const listGcpAccounts = (): Promise<GcpAccount[]> => (live ? (get('/api/v1/integrations/gcp/accounts', {}) as Promise<unknown> as Promise<GcpAccount[]>) : mock.listGcpAccounts());
export const createGcpAccount = (input: GcpAccountInput): Promise<GcpAccount> =>
  live ? (post('/api/v1/integrations/gcp/accounts', input as never) as Promise<unknown> as Promise<GcpAccount>) : mock.createGcpAccount(input);
export const updateGcpAccount = (id: string, input: GcpAccountInput): Promise<GcpAccount> =>
  live ? (patch('/api/v1/integrations/gcp/accounts/{id}', { id }, input as never) as Promise<unknown> as Promise<GcpAccount>) : mock.updateGcpAccount(id, input);
export const deleteGcpAccount = (id: string): Promise<void> => (live ? del('/api/v1/integrations/gcp/accounts/{id}', { id }) : mock.deleteGcpAccount(id));
export const getGcpAccountStatus = (id: string): Promise<GcpAccountStatus> => (live ? get('/api/v1/integrations/gcp/accounts/{id}/status', { id }) : mock.getGcpAccountStatus(id));
