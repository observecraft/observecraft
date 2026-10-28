// Automation → App Builder. In "live" mode apps come from the Rust API (routes/apps.rs); otherwise
// the in-browser mock answers (mock/apps.ts). A button's run is read with getWorkflowRun.
//   GET    /api/v1/apps          listApps
//   POST   /api/v1/apps          createApp
//   GET    /api/v1/apps/:id      getApp
//   PUT    /api/v1/apps/:id      updateApp
//   DELETE /api/v1/apps/:id      deleteApp
//   POST   /api/v1/apps/:id/run  runAppButton
import type { components } from '../generated/schema';
import { API_MODE, del, get, post, postTo, put } from '../http';
import * as mock from '../mock/apps';
import type { AppValues, AutomationApp, AutomationAppInput, AutomationAppRun } from './types';

export * from './types';

type S = components['schemas'];
const live = API_MODE === 'live';
const wire = (input: AutomationAppInput) => input as unknown as S['AutomationAppInput'];
const asApp = (a: Promise<unknown>) => a as Promise<AutomationApp>;

export const listApps = (signal?: AbortSignal): Promise<AutomationApp[]> => (live ? (get('/api/v1/apps', {}, signal) as Promise<unknown> as Promise<AutomationApp[]>) : mock.listApps());
export const getApp = (id: string, signal?: AbortSignal): Promise<AutomationApp> => (live ? asApp(get('/api/v1/apps/{id}', { id }, signal)) : mock.getApp(id));
export const createApp = (input: AutomationAppInput): Promise<AutomationApp> => (live ? asApp(post('/api/v1/apps', wire(input))) : mock.createApp(input));
export const updateApp = (id: string, input: AutomationAppInput): Promise<AutomationApp> => (live ? asApp(put('/api/v1/apps/{id}', { id }, wire(input))) : mock.updateApp(id, input));
export const deleteApp = (id: string): Promise<void> => (live ? del('/api/v1/apps/{id}', { id }) : mock.deleteApp(id));
/** Presses a button: runs its workflow with the app's values. */
export const runAppButton = (id: string, component: string, values: AppValues): Promise<AutomationAppRun> =>
  live ? postTo('/api/v1/apps/{id}/run', { id }, { component, values } as unknown as S['AutomationAppRunInput']) : mock.runApp(id, component, values);
