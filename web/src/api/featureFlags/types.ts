import type { components } from '../generated/schema';

type S = components['schemas'];
export type FeatureFlag = S['FeatureFlag'];
export type FeatureFlagInput = S['FeatureFlagInput'];
export type FeatureFlagsPage = S['FeatureFlagsPage'];
export type FeatureFlagEnvState = S['FeatureFlagEnvState'];
export type FeatureFlagEnvUpdate = S['FeatureFlagEnvUpdate'];
export type FeatureFlagEnvironment = S['FeatureFlagEnvironment'];
export type FeatureFlagEnvironmentInput = S['FeatureFlagEnvironmentInput'];
export type FeatureFlagChange = S['FeatureFlagChange'];
export type FeatureFlagApiKey = S['FeatureFlagApiKey'];
export type FlagType = S['FlagType'];
export type FlagVariant = S['FlagVariant'];
export type FlagEnvConfig = S['FlagEnvConfig'];
export type FlagRule = S['FlagRule'];
export type FlagCondition = S['FlagCondition'];
export type FlagOperator = S['FlagOperator'];
export type FlagServe = S['FlagServe'];
export type FlagSplit = S['FlagSplit'];
export type FlagRollout = S['FlagRollout'];
export type FlagRolloutStep = S['FlagRolloutStep'];
export type FlagSdkAccess = S['FlagSdkAccess'];
export type FlagLifecycle = S['FlagLifecycle'];

/** Evaluations of one flag over a time range (from the `feature_flag.evaluations` metric). */
export interface FlagEvaluations {
  flag: string;
  total: number;
}

/** Evaluations of one flag per variant over time. */
export interface FlagEvaluationSeries {
  variant: string;
  /** `[timestamp ms, evaluations in the interval]`. */
  points: [number, number][];
}
