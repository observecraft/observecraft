// LLM Observability. In "live" mode the Rust API reads LLM spans from the stored traces
// (routes/llm.rs: OpenTelemetry GenAI, OpenLLMetry, OpenInference and dd-trace LLM Observability
// attributes); otherwise the in-browser mock answers from synthetic conversations (mock/llm.ts).
//   POST /api/v1/llm/overview         llmOverview
//   POST /api/v1/llm/facets           llmFacets
//   POST /api/v1/llm/traces/search    searchLlmTraces
//   GET  /api/v1/llm/traces/:id       getLlmTrace
//   POST /api/v1/llm/evaluations      llmEvaluations
import type { components } from '../generated/schema';
import { API_MODE, get, post } from '../http';
import * as mock from '../mock/llm';

type S = components['schemas'];
export type LlmQuery = S['LlmQuery'];
export type LlmOverview = S['LlmOverview'];
export type LlmSeries = S['LlmSeries'];
export type LlmModelUsage = S['LlmModelUsage'];
export type LlmAppUsage = S['LlmAppUsage'];
export type LlmFacets = S['LlmFacets'];
export type LlmTraceRow = S['LlmTraceRow'];
export type LlmTracePage = S['LlmTracePage'];
export type LlmTraceDetail = S['LlmTraceDetail'];
export type LlmSpanDetail = S['LlmSpanDetail'];
export type LlmMessage = S['LlmMessage'];
export type LlmEvaluations = S['LlmEvaluations'];
export type LlmCheckSummary = S['LlmCheckSummary'];
export type LlmEvaluationResult = S['LlmEvaluationResult'];
export type CheckOutcome = S['CheckOutcome'];

/** Facet dimensions the API accepts. */
export const LLM_DIMENSIONS = ['ml_app', 'service', 'model', 'provider', 'kind', 'operation', 'env', 'status'] as const;

const live = API_MODE === 'live';

export async function llmOverview(q: LlmQuery, groupBy: 'model' | 'ml_app', signal?: AbortSignal): Promise<LlmOverview> {
  if (!live) return mock.llmOverview(q, groupBy);
  return post('/api/v1/llm/overview', { ...q, groupBy }, signal);
}

export async function llmFacets(q: LlmQuery, dimensions: string[], signal?: AbortSignal): Promise<LlmFacets> {
  if (!live) return mock.llmFacets(q, dimensions);
  return post('/api/v1/llm/facets', { ...q, dimensions }, signal);
}

export async function searchLlmTraces(q: LlmQuery, offset: number, limit: number, signal?: AbortSignal): Promise<LlmTracePage> {
  if (!live) return mock.searchLlmTraces(q, offset, limit);
  return post('/api/v1/llm/traces/search', { ...q, offset, limit }, signal);
}

export async function getLlmTrace(traceId: string, signal?: AbortSignal): Promise<LlmTraceDetail> {
  if (!live) return mock.getLlmTrace(traceId);
  return get('/api/v1/llm/traces/{trace_id}', { trace_id: traceId }, signal);
}

export async function llmEvaluations(q: LlmQuery, check: string | null, outcome: 'fail' | 'pass' | 'all', signal?: AbortSignal): Promise<LlmEvaluations> {
  if (!live) return mock.llmEvaluations(q, check, outcome);
  return post('/api/v1/llm/evaluations', { ...q, check, outcome }, signal);
}
