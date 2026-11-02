// Integrations → Amazon Web Services. Outside live mode the in-browser mock (mock/aws.ts) answers.
import { API_MODE, del, get, patch, post, postTo } from '../http';
import * as mock from '../mock/aws';
import type { AwsAccount, AwsAccountInput, AwsAccountStatus, AwsSetup } from './types';

export * from './types';

const live = API_MODE === 'live';

export const getAwsSetup = (): Promise<AwsSetup> => (live ? (get('/api/v1/integrations/aws/setup', {}) as Promise<unknown> as Promise<AwsSetup>) : mock.getAwsSetup());
export const newExternalId = (): Promise<string> => (live ? postTo('/api/v1/integrations/aws/external-id', {}, undefined).then((r) => r.external_id) : mock.newExternalId());
export const listAwsAccounts = (): Promise<AwsAccount[]> => (live ? (get('/api/v1/integrations/aws/accounts', {}) as Promise<unknown> as Promise<AwsAccount[]>) : mock.listAwsAccounts());
export const createAwsAccount = (input: AwsAccountInput): Promise<AwsAccount> =>
  live ? (post('/api/v1/integrations/aws/accounts', input as never) as Promise<unknown> as Promise<AwsAccount>) : mock.createAwsAccount(input);
export const updateAwsAccount = (id: string, input: AwsAccountInput): Promise<AwsAccount> =>
  live ? (patch('/api/v1/integrations/aws/accounts/{id}', { id }, input as never) as Promise<unknown> as Promise<AwsAccount>) : mock.updateAwsAccount(id, input);
export const deleteAwsAccount = (id: string): Promise<void> => (live ? del('/api/v1/integrations/aws/accounts/{id}', { id }) : mock.deleteAwsAccount(id));
export const getAwsAccountStatus = (id: string): Promise<AwsAccountStatus> => (live ? get('/api/v1/integrations/aws/accounts/{id}/status', { id }) : mock.getAwsAccountStatus(id));
