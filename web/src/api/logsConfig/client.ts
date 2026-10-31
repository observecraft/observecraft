// Logs → Configuration beyond the per-feature clients: custom destinations (log forwarding), Flex
// Logs controls and pipeline usage. Outside live mode the in-browser mock (mock/logsConfig.ts) answers.
import type { components } from '../generated/schema';
import { API_MODE, del, get, patch, post, postTo, put } from '../http';
import * as mock from '../mock/logsConfig';
import { listIndexes } from '../logIndexes/client';

type S = components['schemas'];
export type { CustomDestination, CustomDestinationInput } from '../mock/logsConfig';
import type { CustomDestination, CustomDestinationInput } from '../mock/logsConfig';
export type FlexSettings = S['FlexSettings'];
export type FlexSettingsInput = S['FlexSettingsInput'];
export type PipelineUsage = S['PipelineUsage'];

const live = API_MODE === 'live';

export const listDestinations = (): Promise<CustomDestination[]> => (live ? (get('/api/v1/logs/custom-destinations', {}) as Promise<CustomDestination[]>) : mock.listDestinations());
export const createDestination = (i: CustomDestinationInput): Promise<CustomDestination> =>
  live ? (post('/api/v1/logs/custom-destinations', i as S['CustomDestinationInput']) as Promise<CustomDestination>) : mock.createDestination(i);
export const updateDestination = (id: string, i: CustomDestinationInput): Promise<CustomDestination> =>
  live ? (put('/api/v1/logs/custom-destinations/{id}', { id }, i as S['CustomDestinationInput']) as Promise<CustomDestination>) : mock.updateDestination(id, i);
export const setDestinationEnabled = (id: string, enabled: boolean): Promise<CustomDestination> =>
  live ? (patch('/api/v1/logs/custom-destinations/{id}', { id }, { enabled }) as Promise<CustomDestination>) : mock.setDestinationEnabled(id, enabled);
export const deleteDestination = (id: string) => (live ? del('/api/v1/logs/custom-destinations/{id}', { id }) : mock.deleteDestination(id));
export const testDestination = (id: string) => (live ? postTo('/api/v1/logs/custom-destinations/{id}/test', { id }, undefined) : mock.testDestination(id));

const flexIndexes = async () => (await listIndexes()).indexes.filter((i) => i.flexRetentionDays != null).length;
export const getFlexSettings = async (): Promise<FlexSettings> => (live ? get('/api/v1/logs/flex-settings', {}) : mock.getFlex(await flexIndexes()));
export const updateFlexSettings = async (i: FlexSettingsInput): Promise<FlexSettings> => (live ? put('/api/v1/logs/flex-settings', {}, i) : mock.updateFlex(i, await flexIndexes()));

export const pipelinesUsage = (ids: string[]): Promise<PipelineUsage[]> => (live ? get('/api/v1/logs/pipelines/usage', {}) : mock.pipelinesUsage(ids));
