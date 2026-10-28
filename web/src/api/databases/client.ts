// Data Observability data access: Database Monitoring and Data Jobs. In "live" mode they come from the
// Rust API (database, Spark and Airflow integration metrics in ClickHouse); otherwise the in-browser
// mock answers.
//   POST /api/v1/databases/search    searchDatabases   (instances with current values, facets, unconfigured hosts)
//   POST /api/v1/databases/instance  databaseInstance  (one instance's current values and series)
//   POST /api/v1/data-jobs/search    searchDataJobs    (Airflow DAGs and Spark applications with their runs)
import type { components } from '../generated/schema';
import { API_MODE, post } from '../http';
import * as mock from '../mock/databases';

const live = API_MODE === 'live';

export type DbEngine = components['schemas']['DbEngine'];
export type DbInstance = components['schemas']['DbInstance'];
export type DbFacet = components['schemas']['DbFacet'];
export type DbSearchPage = components['schemas']['DbSearchPage'];
export type DbSearchRequest = components['schemas']['DbSearchRequest'];
export type DbInstanceRequest = components['schemas']['DbInstanceRequest'];
export type DbInstanceDetail = components['schemas']['DbInstanceDetail'];
export type UnconfiguredDb = components['schemas']['UnconfiguredDb'];
export type DataJob = components['schemas']['DataJob'];
export type JobFacet = components['schemas']['JobFacet'];
export type JobStatus = components['schemas']['JobStatus'];
export type JobSearchPage = components['schemas']['JobSearchPage'];
export type JobSearchRequest = components['schemas']['JobSearchRequest'];

export async function searchDatabases(req: DbSearchRequest): Promise<DbSearchPage> {
  if (!live) return mock.searchDatabases(req);
  return post('/api/v1/databases/search', req);
}

export async function databaseInstance(req: DbInstanceRequest): Promise<DbInstanceDetail> {
  if (!live) return mock.databaseInstance(req);
  return post('/api/v1/databases/instance', req);
}

export async function searchDataJobs(req: JobSearchRequest): Promise<JobSearchPage> {
  if (!live) return mock.searchDataJobs(req);
  return post('/api/v1/data-jobs/search', req);
}
