// Feature flags (Software Delivery → Feature Flags). In "live" mode they come from the Rust API;
// otherwise the in-browser mock answers (mock/featureFlags.ts). SDKs evaluate flags at the intake
// over OFREP; the counts they produce are read back from the `feature_flag.evaluations` metric.
//   GET    /api/v1/feature-flags                                  listFeatureFlags
//   POST   /api/v1/feature-flags                                  createFeatureFlag
//   GET    /api/v1/feature-flags/:id                              getFeatureFlag
//   PUT    /api/v1/feature-flags/:id                              updateFeatureFlag
//   DELETE /api/v1/feature-flags/:id                              deleteFeatureFlag
//   PUT    /api/v1/feature-flags/:id/environments/:environment    updateFeatureFlagEnvironment
//   GET    /api/v1/feature-flags/:id/history                      listFeatureFlagHistory(id)
//   GET    /api/v1/feature-flags/history                          listFeatureFlagHistory()
//   GET    /api/v1/feature-flags/environments                     listFeatureFlagEnvironments
//   POST   /api/v1/feature-flags/environments                     createFeatureFlagEnvironment
//   PUT    /api/v1/feature-flags/environments/:id                 updateFeatureFlagEnvironmentSettings
//   DELETE /api/v1/feature-flags/environments/:id                 deleteFeatureFlagEnvironment
//   POST   /api/v1/query/timeseries                               featureFlagEvaluations, featureFlagEvaluationSeries
import { API_MODE, ApiError, del, get, post, put } from '../http';
import type { TimeRange } from '../types';
import * as mock from '../mock/featureFlags';
import { metricStepMs, totalFromRates } from './logic';
import type {
  FeatureFlag,
  FeatureFlagChange,
  FeatureFlagEnvironment,
  FeatureFlagEnvironmentInput,
  FeatureFlagEnvUpdate,
  FeatureFlagInput,
  FeatureFlagsPage,
  FlagEvaluations,
  FlagEvaluationSeries,
} from './types';

export * from './types';

const live = API_MODE === 'live';
const METRIC = 'feature_flag.evaluations';

export const listFeatureFlags = (): Promise<FeatureFlagsPage> => (live ? get('/api/v1/feature-flags', {}) : mock.listFeatureFlags());
export const getFeatureFlag = (id: string, signal?: AbortSignal): Promise<FeatureFlag> => (live ? get('/api/v1/feature-flags/{id}', { id }, signal) : mock.getFeatureFlag(id));
export const createFeatureFlag = (input: FeatureFlagInput): Promise<FeatureFlag> => (live ? post('/api/v1/feature-flags', input) : mock.createFeatureFlag(input));
export const updateFeatureFlag = (id: string, input: FeatureFlagInput): Promise<FeatureFlag> => (live ? put('/api/v1/feature-flags/{id}', { id }, input) : mock.updateFeatureFlag(id, input));
export const deleteFeatureFlag = (id: string): Promise<void> => (live ? del('/api/v1/feature-flags/{id}', { id }) : mock.deleteFeatureFlag(id));
export const updateFeatureFlagEnvironment = (id: string, environmentId: string, update: FeatureFlagEnvUpdate): Promise<FeatureFlag> =>
  live ? put('/api/v1/feature-flags/{id}/environments/{environment_id}', { id, environment_id: environmentId }, update) : mock.updateFeatureFlagEnvironment(id, environmentId, update);
/** One flag's changes, or every flag's (Global Flag History) without an id. */
export const listFeatureFlagHistory = (id?: string): Promise<FeatureFlagChange[]> =>
  live ? (id ? get('/api/v1/feature-flags/{id}/history', { id }) : get('/api/v1/feature-flags/history', {})) : mock.listFeatureFlagHistory(id);
export const listFeatureFlagEnvironments = (): Promise<FeatureFlagEnvironment[]> => (live ? get('/api/v1/feature-flags/environments', {}) : mock.listFeatureFlagEnvironments());
export const createFeatureFlagEnvironment = (input: FeatureFlagEnvironmentInput): Promise<FeatureFlagEnvironment> =>
  live ? post('/api/v1/feature-flags/environments', input) : mock.createFeatureFlagEnvironment(input);
export const updateFeatureFlagEnvironmentSettings = (id: string, input: FeatureFlagEnvironmentInput): Promise<FeatureFlagEnvironment> =>
  live ? put('/api/v1/feature-flags/environments/{id}', { id }, input) : mock.updateFeatureFlagEnvironmentSettings(id, input);
export const deleteFeatureFlagEnvironment = (id: string): Promise<void> => (live ? del('/api/v1/feature-flags/environments/{id}', { id }) : mock.deleteFeatureFlagEnvironment(id));

/** Series of the evaluation metric; none while no SDK has evaluated anything yet. */
async function evaluationSeries(query: string, range: TimeRange, points: number, signal?: AbortSignal) {
  try {
    return await post('/api/v1/query/timeseries', { queries: [query], range, points }, signal);
  } catch (e) {
    if (e instanceof ApiError && e.status === 400 && e.message.startsWith('Unknown metric')) return [];
    throw e;
  }
}

/** `flag:checkout-new-payment-flow` → the tag's value. */
const tagValue = (name: string) => name.slice(name.indexOf(':') + 1);

/** Evaluations per flag over the range. */
export async function featureFlagEvaluations(range: TimeRange, signal?: AbortSignal): Promise<FlagEvaluations[]> {
  if (!live) return mock.featureFlagEvaluations(range);
  const series = await evaluationSeries(`sum:${METRIC}{*} by {flag}`, range, 48, signal);
  return series.map((s) => ({ flag: tagValue(s.name), total: totalFromRates(s.points, metricStepMs(range, 48)) }));
}

/** One flag's evaluations per variant over the range (counts per interval). */
export async function featureFlagEvaluationSeries(flagKey: string, range: TimeRange, signal?: AbortSignal): Promise<FlagEvaluationSeries[]> {
  if (!live) return mock.featureFlagEvaluationSeries(flagKey, range);
  const series = await evaluationSeries(`sum:${METRIC}{flag:${flagKey}} by {variant}`, range, 48, signal);
  const step = metricStepMs(range, 48) / 1000;
  return series.map((s) => ({ variant: tagValue(s.name), points: s.points.map(([t, v]) => [t, Math.round(v * step)] as [number, number]) }));
}
