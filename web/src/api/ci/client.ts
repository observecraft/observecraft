// Software Delivery: CI pipeline executions, flaky tests and DORA metrics (ClickHouse CI/CD traces).
import type { components } from '../generated/schema';
import { get, post, put } from '../http';
import { requireLive } from '../identity/client';

type S = components['schemas'];
export type CiLevel = S['CiLevel'];
export type CiQuery = S['CiQuery'];
export type CiFilters = S['CiFilters'];
export type CiExecution = S['CiExecution'];
export type CiFacets = S['CiFacets'];
export type CiCharts = S['CiCharts'];
export type CiSpan = S['CiSpan'];
export type CiPipelineDetail = S['CiPipelineDetail'];
export type CiAggregate = S['CiAggregate'];
export type CiOverview = S['CiOverview'];
export type FlakyTest = S['FlakyTest'];
export type FlakyState = S['FlakyState'];
export type DoraResponse = S['DoraResponse'];
export type DoraSummary = S['DoraSummary'];
export type CiRepository = S['CiRepository'];
export type TimeRange = S['TimeRange'];

export async function searchExecutions(q: CiQuery, sort: string, offset: number, limit: number) {
  requireLive();
  return post('/api/v1/ci/executions', { ...q, sort, offset, limit });
}

export async function executionFacets(q: CiQuery) {
  requireLive();
  return post('/api/v1/ci/executions/facets', q);
}

export async function executionCharts(q: CiQuery) {
  requireLive();
  return post('/api/v1/ci/executions/charts', q);
}

export async function pipelineDetail(traceId: string) {
  requireLive();
  return get('/api/v1/ci/pipelines/{trace_id}', { trace_id: traceId });
}

export async function aggregations(q: CiQuery) {
  requireLive();
  return post('/api/v1/ci/aggregations', q);
}

export async function ciOverview(range: TimeRange) {
  requireLive();
  return post('/api/v1/ci/overview', { range });
}

export async function flakyTests(range: TimeRange, query: string, states: FlakyState[]) {
  requireLive();
  return post('/api/v1/ci/flaky', { range, query, states });
}

export async function setFlakyState(repository: string, suite: string, name: string, state: FlakyState) {
  requireLive();
  await put('/api/v1/ci/flaky/state', {}, { repository, suite, name, state });
}

export async function doraMetrics(range: TimeRange, env: string | null, services: string[]) {
  requireLive();
  return post('/api/v1/ci/dora', { range, env, services });
}

export async function ciRepositories() {
  requireLive();
  return get('/api/v1/ci/repositories', {});
}
