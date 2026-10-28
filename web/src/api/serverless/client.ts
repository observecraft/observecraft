// Serverless (AWS Lambda) data access. In "live" mode functions come from the Rust API (the Datadog Lambda
// extension's enhanced metrics, the AWS integration's aws.lambda.* metrics, OpenTelemetry faas.* metrics and
// Lambda spans, in ClickHouse); otherwise the in-browser mock answers.
//   POST /api/v1/serverless/functions     searchFunctions       (functions with stats, insights, upgrades, facets)
//   POST /api/v1/serverless/timeseries    serverlessTimeseries  (invocations, errors, duration, cold starts per function)
//   POST /api/v1/serverless/invocations   functionInvocations   (latest Lambda spans of one function)
import type { components } from '../generated/schema';
import { API_MODE, post } from '../http';
import * as mock from '../mock/serverless';

const live = API_MODE === 'live';

export type LambdaFunction = components['schemas']['LambdaFunction'];
export type LambdaInsight = components['schemas']['LambdaInsight'];
export type LambdaInsightKind = components['schemas']['LambdaInsightKind'];
export type LambdaUpgrade = components['schemas']['LambdaUpgrade'];
export type LambdaSource = components['schemas']['LambdaSource'];
export type LambdaFacet = components['schemas']['LambdaFacet'];
export type LambdaInvocation = components['schemas']['LambdaInvocation'];
export type ServerlessSummary = components['schemas']['ServerlessSummary'];
export type ServerlessSearchPage = components['schemas']['ServerlessSearchPage'];
export type ServerlessSearchRequest = components['schemas']['ServerlessSearchRequest'];
export type ServerlessSeries = components['schemas']['ServerlessSeries'];
export type ServerlessTimeseriesRequest = components['schemas']['ServerlessTimeseriesRequest'];
export type ServerlessInvocationsRequest = components['schemas']['ServerlessInvocationsRequest'];

export async function searchFunctions(req: ServerlessSearchRequest): Promise<ServerlessSearchPage> {
  if (!live) return mock.searchFunctions(req);
  return post('/api/v1/serverless/functions', req);
}

export async function serverlessTimeseries(req: ServerlessTimeseriesRequest): Promise<ServerlessSeries[]> {
  if (!live) return mock.serverlessTimeseries(req);
  return post('/api/v1/serverless/timeseries', req);
}

export async function functionInvocations(req: ServerlessInvocationsRequest): Promise<LambdaInvocation[]> {
  if (!live) return mock.functionInvocations(req);
  return post('/api/v1/serverless/invocations', req);
}
