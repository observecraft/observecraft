// Automation → Workflow Automation. In "live" mode workflows come from the Rust API
// (routes/workflows.rs); otherwise the in-browser mock answers (mock/workflows.ts).
//   GET    /api/v1/workflows                       listWorkflows
//   POST   /api/v1/workflows                       createWorkflow
//   GET    /api/v1/workflows/:id                   getWorkflow
//   PUT    /api/v1/workflows/:id                   updateWorkflow
//   DELETE /api/v1/workflows/:id                   deleteWorkflow
//   POST   /api/v1/workflows/:id/publish           publishWorkflow
//   POST   /api/v1/workflows/:id/unpublish         unpublishWorkflow
//   POST   /api/v1/workflows/:id/token             createWorkflowToken
//   GET    /api/v1/workflows/:id/runs              listWorkflowRuns
//   POST   /api/v1/workflows/:id/runs              runWorkflow
//   GET    /api/v1/workflows/:id/runs/:run         getWorkflowRun
//   POST   /api/v1/workflows/:id/runs/:run/cancel  cancelWorkflowRun
import type { components } from '../generated/schema';
import { API_MODE, del, get, post, postTo, put } from '../http';
import * as mock from '../mock/workflows';
import type { Workflow, WorkflowInput, WorkflowRun, WorkflowValues } from './types';

export * from './types';

type S = components['schemas'];
const live = API_MODE === 'live';
// The generated schema types free-form objects (secrets, input values) loosely.
const wire = (input: WorkflowInput) => input as unknown as S['WorkflowInput'];
const runBody = (inputs: WorkflowValues) => ({ inputs }) as unknown as S['WorkflowRunInput'];
const asWorkflow = (w: Promise<unknown>) => w as Promise<Workflow>;
const asRun = (r: Promise<unknown>) => r as Promise<WorkflowRun>;

export const listWorkflows = (signal?: AbortSignal): Promise<Workflow[]> => (live ? (get('/api/v1/workflows', {}, signal) as Promise<unknown> as Promise<Workflow[]>) : mock.listWorkflows());
export const getWorkflow = (id: string, signal?: AbortSignal): Promise<Workflow> => (live ? asWorkflow(get('/api/v1/workflows/{id}', { id }, signal)) : mock.getWorkflow(id));
export const createWorkflow = (input: WorkflowInput): Promise<Workflow> => (live ? asWorkflow(post('/api/v1/workflows', wire(input))) : mock.createWorkflow(input));
export const updateWorkflow = (id: string, input: WorkflowInput): Promise<Workflow> => (live ? asWorkflow(put('/api/v1/workflows/{id}', { id }, wire(input))) : mock.updateWorkflow(id, input));
export const deleteWorkflow = (id: string): Promise<void> => (live ? del('/api/v1/workflows/{id}', { id }) : mock.deleteWorkflow(id));
/** Publishes the saved draft: its triggers run it from now on, as the signed-in member. */
export const publishWorkflow = (id: string): Promise<Workflow> => (live ? asWorkflow(postTo('/api/v1/workflows/{id}/publish', { id }, undefined)) : mock.publishWorkflow(id));
export const unpublishWorkflow = (id: string): Promise<Workflow> => (live ? asWorkflow(postTo('/api/v1/workflows/{id}/unpublish', { id }, undefined)) : mock.unpublishWorkflow(id));
/** A new API trigger token, shown once (the previous one stops working). */
export const createWorkflowToken = (id: string): Promise<{ token: string }> => (live ? postTo('/api/v1/workflows/{id}/token', { id }, undefined) : mock.createToken(id));
export const listWorkflowRuns = (id: string, signal?: AbortSignal): Promise<WorkflowRun[]> =>
  live ? (get('/api/v1/workflows/{id}/runs', { id }, signal) as Promise<unknown> as Promise<WorkflowRun[]>) : mock.listRuns(id);
export const getWorkflowRun = (id: string, run: number, signal?: AbortSignal): Promise<WorkflowRun> =>
  live ? asRun(get('/api/v1/workflows/{id}/runs/{run}', { id, run: String(run) }, signal)) : mock.getRun(id, run);
/** Runs the saved draft now, as the signed-in member; the run goes on in the background. */
export const runWorkflow = (id: string, inputs: WorkflowValues): Promise<WorkflowRun> => (live ? asRun(postTo('/api/v1/workflows/{id}/runs', { id }, runBody(inputs))) : mock.runWorkflow(id, inputs));
export const cancelWorkflowRun = (id: string, run: number): Promise<WorkflowRun> =>
  live ? asRun(postTo('/api/v1/workflows/{id}/runs/{run}/cancel', { id, run: String(run) }, undefined)) : mock.cancelRun(id, run);
