// Log pipelines (Logs → Configuration → Pipelines), the integration pipeline library and the
// processor test endpoints. Outside live mode the in-browser mock (mock/pipelines.ts) answers.
import { API_MODE, del, get, patch, post, postTo, put } from '../http';
import * as mock from '../mock/pipelines';
import type { GrokTestRequest, GrokTestResult, LibraryPipeline, LogPipeline, LogPipelineInput, PipelineTestRequest, PipelineTestResult } from './types';

export * from './types';

const live = API_MODE === 'live';

/** The generated schema types processors and attributes as opaque objects; ./types has their shape. */
const wire = (body: object) => body as never;

export const listPipelines = (): Promise<LogPipeline[]> => (live ? get('/api/v1/logs/pipelines', {}).then((r) => r as unknown as LogPipeline[]) : mock.listPipelines());
export const createPipeline = (i: LogPipelineInput): Promise<LogPipeline> => (live ? post('/api/v1/logs/pipelines', wire(i)).then((r) => r as unknown as LogPipeline) : mock.createPipeline(i));
export const updatePipeline = (id: string, i: LogPipelineInput): Promise<LogPipeline> =>
  live ? put('/api/v1/logs/pipelines/{id}', { id }, wire(i)).then((r) => r as unknown as LogPipeline) : mock.updatePipeline(id, i);
export const setPipelineEnabled = (id: string, enabled: boolean): Promise<LogPipeline> =>
  live ? patch('/api/v1/logs/pipelines/{id}', { id }, { enabled }).then((r) => r as unknown as LogPipeline) : mock.setPipelineEnabled(id, enabled);
export const deletePipeline = (id: string): Promise<void> => (live ? del('/api/v1/logs/pipelines/{id}', { id }) : mock.deletePipeline(id));
export const reorderPipelines = (ids: string[]): Promise<void> => (live ? post('/api/v1/logs/pipelines/reorder', { ids }) : mock.reorderPipelines(ids));
export const listLibrary = (): Promise<LibraryPipeline[]> => (live ? get('/api/v1/logs/pipeline-library', {}).then((r) => r as unknown as LibraryPipeline[]) : mock.listLibrary());
export const installLibraryPipeline = (id: string): Promise<LogPipeline> =>
  live ? postTo('/api/v1/logs/pipeline-library/{id}/install', { id }, undefined).then((r) => r as unknown as LogPipeline) : mock.installLibraryPipeline(id);
export const testPipeline = (req: PipelineTestRequest, signal?: AbortSignal): Promise<PipelineTestResult> =>
  live ? post('/api/v1/logs/pipelines/test', wire(req), signal).then((r) => r as unknown as PipelineTestResult) : Promise.resolve(mock.testPipeline(req));
export const parseGrok = (req: GrokTestRequest, signal?: AbortSignal): Promise<GrokTestResult> =>
  live ? post('/api/v1/logs/pipelines/grok', req, signal).then((r) => r as unknown as GrokTestResult) : Promise.resolve(mock.parseGrok(req));
