// Developer Portal → Self-Service Actions. In "live" mode they come from the Rust API; otherwise the
// in-browser mock answers (mock/portalActions.ts).
//   GET    /api/v1/portal/actions             listPortalActions
//   POST   /api/v1/portal/actions             createPortalAction
//   GET    /api/v1/portal/actions/:id         getPortalAction
//   PUT    /api/v1/portal/actions/:id         updatePortalAction
//   DELETE /api/v1/portal/actions/:id         deletePortalAction
//   GET    /api/v1/portal/actions/:id/runs    listPortalActionRuns
//   POST   /api/v1/portal/actions/:id/runs    runPortalAction
import type { components } from '../generated/schema';
import { API_MODE, del, get, post, postTo, put } from '../http';
import * as mock from '../mock/portalActions';
import type { ActionValues, PortalAction, PortalActionInput, PortalActionRun } from './types';

export * from './types';

type S = components['schemas'];
const live = API_MODE === 'live';
// The generated schema types free-form objects (secrets, input values) as empty records.
const wire = (input: PortalActionInput) => input as unknown as S['PortalActionInput'];
const runBody = (inputs: ActionValues) => ({ inputs }) as unknown as S['PortalActionRunInput'];

export const listPortalActions = (signal?: AbortSignal): Promise<PortalAction[]> => (live ? get('/api/v1/portal/actions', {}, signal) : mock.listActions());
export const getPortalAction = (id: string, signal?: AbortSignal): Promise<PortalAction> => (live ? get('/api/v1/portal/actions/{id}', { id }, signal) : mock.getAction(id));
export const createPortalAction = (input: PortalActionInput): Promise<PortalAction> => (live ? post('/api/v1/portal/actions', wire(input)) : mock.createAction(input));
export const updatePortalAction = (id: string, input: PortalActionInput): Promise<PortalAction> => (live ? put('/api/v1/portal/actions/{id}', { id }, wire(input)) : mock.updateAction(id, input));
export const deletePortalAction = (id: string): Promise<void> => (live ? del('/api/v1/portal/actions/{id}', { id }) : mock.deleteAction(id));
export const listPortalActionRuns = (id: string, signal?: AbortSignal): Promise<PortalActionRun[]> =>
  live ? (get('/api/v1/portal/actions/{id}/runs', { id }, signal) as Promise<PortalActionRun[]>) : mock.listRuns(id);
/** Runs the action now, as the signed-in user. */
export const runPortalAction = (id: string, inputs: ActionValues): Promise<PortalActionRun> =>
  live ? (postTo('/api/v1/portal/actions/{id}/runs', { id }, runBody(inputs)) as Promise<PortalActionRun>) : mock.runAction(id, inputs);
