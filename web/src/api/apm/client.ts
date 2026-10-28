// APM data access. In "live" mode spans come from ClickHouse through the Rust API (routes/apm.rs,
// routes/spans.rs); otherwise the in-browser mock answers from synthetic traces (mock/apm.ts).
import type { components } from '../generated/schema';
import { API_MODE, get, post } from '../http';
import * as mock from '../mock/apm';
import type { TimeRange } from '../types';

type S = components['schemas'];
export type ServiceKind = S['ServiceKind'];
export type ServiceStats = S['ServiceStats'];
export type ServiceSummary = S['ServiceSummary'];
export type SeriesPoint = S['SeriesPoint'];
export type ResourceStats = S['ResourceStats'];
export type Deployment = S['Deployment'];
export type Dependency = S['Dependency'];
export type Dependencies = S['Dependencies'];
export type DatabaseStats = S['DatabaseStats'];
export type SpanErrorGroup = S['SpanErrorGroup'];
export type SpanRow = S['SpanRow'];
export type TraceSearchPage = S['TraceSearchPage'];
export type TraceDetail = S['TraceDetail'];
export type TraceSpan = S['TraceSpan'];
export type Recommendation = S['Recommendation'];
export type Anomaly = S['Anomaly'];
export type Health = S['Health'];
export type LogEvent = S['LogEvent'];

export interface ApmScope {
  range: TimeRange;
  env?: string | null;
}

export interface ServiceScope extends ApmScope {
  service: string;
  resource?: string | null;
  version?: string | null;
}

const live = API_MODE === 'live';

export async function listServices(s: ApmScope, signal?: AbortSignal) {
  if (!live) return mock.listServices(s);
  return post('/api/v1/apm/services', { range: s.range, env: s.env }, signal);
}

export async function listEnvs(range: TimeRange, signal?: AbortSignal) {
  if (!live) return mock.listEnvs(range);
  return post('/api/v1/apm/envs', { range }, signal);
}

export async function listRecommendations(s: ApmScope, signal?: AbortSignal) {
  if (!live) return mock.listRecommendations(s);
  return post('/api/v1/apm/recommendations', { range: s.range, env: s.env }, signal);
}

export async function listAnomalies(s: ApmScope, signal?: AbortSignal) {
  if (!live) return mock.listAnomalies(s);
  return post('/api/v1/apm/anomalies', { range: s.range, env: s.env }, signal);
}

/** Resources (endpoints) of one service, or of every instrumented service when `service` is unset. */
export async function listResources(s: ApmScope & { service?: string | null }, signal?: AbortSignal) {
  if (!live) return mock.listResources(s);
  return post('/api/v1/apm/resources', { range: s.range, env: s.env, service: s.service }, signal);
}

export async function getServiceSummary(s: ServiceScope, signal?: AbortSignal) {
  if (!live) return mock.getServiceSummary(s);
  return post('/api/v1/apm/service', { range: s.range, env: s.env, service: s.service, resource: s.resource, version: s.version }, signal);
}

export async function listDeployments(s: ServiceScope, signal?: AbortSignal) {
  if (!live) return mock.listDeployments(s);
  return post('/api/v1/apm/deployments', { range: s.range, env: s.env, service: s.service }, signal);
}

export async function getDependencies(s: ServiceScope, signal?: AbortSignal) {
  if (!live) return mock.getDependencies(s);
  return post('/api/v1/apm/dependencies', { range: s.range, env: s.env, service: s.service }, signal);
}

export async function listDatabases(s: ServiceScope, signal?: AbortSignal) {
  if (!live) return mock.listDatabases(s);
  return post('/api/v1/apm/databases', { range: s.range, env: s.env, service: s.service }, signal);
}

export async function listSpanErrors(s: ApmScope & { service?: string | null }, signal?: AbortSignal) {
  if (!live) return mock.listSpanErrors(s);
  return post('/api/v1/apm/errors', { range: s.range, env: s.env, service: s.service }, signal);
}

export async function searchSpans(req: ApmScope & { query: string; cursor?: string | null; limit?: number }, signal?: AbortSignal) {
  if (!live) return mock.searchSpans(req);
  return post('/api/v1/apm/traces/search', { range: req.range, env: req.env, query: req.query, cursor: req.cursor, limit: req.limit }, signal);
}

export async function getTrace(traceId: string, signal?: AbortSignal) {
  if (!live) return mock.getTrace(traceId);
  return get('/api/v1/apm/traces/{trace_id}', { trace_id: traceId }, signal);
}

/** A page of the service's logs — just the rows, without the explorer's facets and histogram. */
export async function serviceLogs(range: TimeRange, service: string, signal?: AbortSignal) {
  if (!live) return mock.serviceLogs(range, service);
  return post('/api/v1/logs/search', { range, query: `service:${service}`, limit: 25 }, signal);
}

/** Logs written inside one trace (OTLP logs carry trace_id). */
export async function traceLogs(range: TimeRange, traceId: string, signal?: AbortSignal) {
  if (!live) return mock.traceLogs(range, traceId);
  return post('/api/v1/logs/search', { range, query: `trace_id:${traceId}`, limit: 200 }, signal);
}

// ---------------------------------------------------------------- span explorer

export type SpanScope = S['SpanScope'];
export type SpanQuery = S['SpanQuery'];
export type ExplorerSpan = S['ExplorerSpan'];
export type SpanSearchResult = S['SpanSearchResult'];
export type SpanFacets = S['SpanFacets'];
export type SpanMeasure = S['SpanMeasure'];
export type SpanAnalytics = S['SpanAnalytics'];
export type SpanSummary = S['SpanSummary'];

export async function exploreSpans(q: SpanQuery, sort: string, offset: number, limit: number, signal?: AbortSignal) {
  if (!live) return mock.exploreSpans(q, sort, offset, limit);
  return post('/api/v1/apm/spans/search', { ...q, sort, offset, limit }, signal);
}

export async function spanFacets(q: SpanQuery, dimensions: string[], signal?: AbortSignal) {
  if (!live) return mock.spanFacets(q, dimensions);
  return post('/api/v1/apm/spans/facets', { ...q, dimensions }, signal);
}

export async function spanAnalytics(q: SpanQuery, groupBy: string | null, measure: SpanMeasure, top: number, signal?: AbortSignal) {
  if (!live) return mock.spanAnalytics(q, groupBy, measure, top);
  return post('/api/v1/apm/spans/analytics', { ...q, groupBy, measure, top }, signal);
}

export async function spanSummary(q: SpanQuery, signal?: AbortSignal) {
  if (!live) return mock.spanSummary(q);
  return post('/api/v1/apm/spans/summary', q, signal);
}

export async function spanPercentile(service: string, resource: string, durationMs: number, at: number, signal?: AbortSignal) {
  if (!live) return mock.spanPercentile(service, resource, durationMs, at);
  return post('/api/v1/apm/spans/percentile', { service, resource, durationMs, at }, signal);
}
