// APM → Settings. In "live" mode from the Rust API (routes/apm_settings.rs), otherwise the in-browser
// mock (mock/apmSettings.ts).
//   GET  /api/v1/apm/settings/overview?from&to               how spans flow: sources, generated / ingested / indexed
//   GET  /api/v1/apm/settings/volume?from&to                 spans and bytes ingested, live and indexed spans
//   GET  /api/v1/apm/settings/ingestion?from&to              ingestion per service and over time
//   …    /api/v1/apm/settings/defaults                       default environment and primary tags
//   …    /api/v1/apm/settings/retention-filters[/:id]        retention filters (the Datadog API's records) with what they keep
//   PUT  /api/v1/apm/settings/retention-filters-order        execution order
//   …    /api/v1/apm/settings/metrics[/:id]                  span-based metrics
//   …    /api/v1/apm/settings/pipelines[/:id]                span pipelines
import { API_MODE, del, get, patch, post, put } from '../http';
import * as mock from '../mock/apmSettings';
import type {
  ApmDefaults,
  ApmDefaultsInput,
  ApmFlow,
  Ingestion,
  RetentionFilter,
  RetentionFilterInput,
  RetentionFilters,
  SpanPipeline,
  SpanPipelineInput,
  SpansMetric,
  SpansMetricInput,
  SpanVolume,
  TimeRange,
} from './types';

export * from './types';

const live = API_MODE === 'live';
const q = (r: TimeRange) => ({ from: r.from, to: r.to });

export const getApmFlow = (range: TimeRange): Promise<ApmFlow> => (live ? get('/api/v1/apm/settings/overview', {}, undefined, q(range)) : mock.overview(range));
export const getSpanVolume = (range: TimeRange): Promise<SpanVolume> => (live ? get('/api/v1/apm/settings/volume', {}, undefined, q(range)) : mock.volume(range));
export const getIngestion = (range: TimeRange): Promise<Ingestion> => (live ? get('/api/v1/apm/settings/ingestion', {}, undefined, q(range)) : mock.ingestion(range));
export const getApmDefaults = (): Promise<ApmDefaults> => (live ? get('/api/v1/apm/settings/defaults', {}) : mock.defaults());
export const updateApmDefaults = (input: ApmDefaultsInput): Promise<ApmDefaults> => (live ? put('/api/v1/apm/settings/defaults', {}, input) : mock.updateDefaults(input));
export const listRetentionFilters = (range: TimeRange): Promise<RetentionFilters> => (live ? get('/api/v1/apm/settings/retention-filters', {}, undefined, q(range)) : mock.retentionFilters(range));
export const createRetentionFilter = (input: RetentionFilterInput): Promise<RetentionFilter> => (live ? post('/api/v1/apm/settings/retention-filters', input) : mock.createFilter(input));
export const updateRetentionFilter = (id: string, input: RetentionFilterInput): Promise<RetentionFilter> =>
  live ? put('/api/v1/apm/settings/retention-filters/{id}', { id }, input) : mock.updateFilter(id, input);
export const deleteRetentionFilter = (id: string): Promise<void> => (live ? del('/api/v1/apm/settings/retention-filters/{id}', { id }) : mock.deleteFilter(id));
export const reorderRetentionFilters = (ids: string[]): Promise<void> => (live ? put('/api/v1/apm/settings/retention-filters-order', {}, { ids }).then(() => undefined) : mock.reorderFilters(ids));
export const listSpansMetrics = (): Promise<SpansMetric[]> => (live ? get('/api/v1/apm/settings/metrics', {}) : mock.metrics());
export const createSpansMetric = (input: SpansMetricInput): Promise<SpansMetric> => (live ? post('/api/v1/apm/settings/metrics', input) : mock.createMetric(input));
export const updateSpansMetric = (id: string, input: SpansMetricInput): Promise<SpansMetric> => (live ? patch('/api/v1/apm/settings/metrics/{id}', { id }, input) : mock.updateMetric(id, input));
export const deleteSpansMetric = (id: string): Promise<void> => (live ? del('/api/v1/apm/settings/metrics/{id}', { id }) : mock.deleteMetric(id));
export const listSpanPipelines = (): Promise<SpanPipeline[]> => (live ? get('/api/v1/apm/settings/pipelines', {}) : mock.pipelines());
export const createSpanPipeline = (input: SpanPipelineInput): Promise<SpanPipeline> => (live ? post('/api/v1/apm/settings/pipelines', input) : mock.createPipeline(input));
export const updateSpanPipeline = (id: string, input: SpanPipelineInput): Promise<SpanPipeline> => (live ? put('/api/v1/apm/settings/pipelines/{id}', { id }, input) : mock.updatePipeline(id, input));
export const deleteSpanPipeline = (id: string): Promise<void> => (live ? del('/api/v1/apm/settings/pipelines/{id}', { id }) : mock.deletePipeline(id));
