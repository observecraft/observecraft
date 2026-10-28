// Software Delivery: CI pipeline executions, flaky tests and DORA metrics. In "live" mode they come from the
// Rust API (CI/CD traces in ClickHouse); otherwise the in-browser mock answers with pipeline runs generated
// like oc-tracegen's (see mock/ci.ts).
//   POST /api/v1/ci/executions          searchExecutions
//   POST /api/v1/ci/executions/facets   executionFacets
//   POST /api/v1/ci/executions/charts   executionCharts
//   GET  /api/v1/ci/pipelines/:trace_id pipelineDetail
//   POST /api/v1/ci/aggregations        aggregations
//   POST /api/v1/ci/overview            ciOverview
//   POST /api/v1/ci/flaky               flakyTests
//   PUT  /api/v1/ci/flaky/state         setFlakyState
//   POST /api/v1/ci/dora                doraMetrics
//   GET  /api/v1/ci/repositories        ciRepositories
import type { components } from '../generated/schema';
import { API_MODE, get, post, put } from '../http';
import * as mock from '../mock/ci';

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
  if (API_MODE !== 'live') return mock.searchExecutions(q, sort, offset, limit);
  return post('/api/v1/ci/executions', { ...q, sort, offset, limit });
}

export async function executionFacets(q: CiQuery) {
  if (API_MODE !== 'live') return mock.executionFacets(q);
  return post('/api/v1/ci/executions/facets', q);
}

export async function executionCharts(q: CiQuery) {
  if (API_MODE !== 'live') return mock.executionCharts(q);
  return post('/api/v1/ci/executions/charts', q);
}

export async function pipelineDetail(traceId: string) {
  if (API_MODE !== 'live') return mock.pipelineDetail(traceId);
  return get('/api/v1/ci/pipelines/{trace_id}', { trace_id: traceId });
}

export async function aggregations(q: CiQuery) {
  if (API_MODE !== 'live') return mock.aggregations(q);
  return post('/api/v1/ci/aggregations', q);
}

export async function ciOverview(range: TimeRange) {
  if (API_MODE !== 'live') return mock.ciOverview(range);
  return post('/api/v1/ci/overview', { range });
}

export async function flakyTests(range: TimeRange, query: string, states: FlakyState[]) {
  if (API_MODE !== 'live') return mock.flakyTests(range, query, states);
  return post('/api/v1/ci/flaky', { range, query, states });
}

export async function setFlakyState(repository: string, suite: string, name: string, state: FlakyState) {
  if (API_MODE !== 'live') return mock.setFlakyState(repository, suite, name, state);
  await put('/api/v1/ci/flaky/state', {}, { repository, suite, name, state });
}

export async function doraMetrics(range: TimeRange, env: string | null, services: string[]) {
  if (API_MODE !== 'live') return mock.doraMetrics(range, env, services);
  return post('/api/v1/ci/dora', { range, env, services });
}

export async function ciRepositories() {
  if (API_MODE !== 'live') return mock.ciRepositories();
  return get('/api/v1/ci/repositories', {});
}
