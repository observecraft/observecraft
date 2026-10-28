// Metrics → Settings → Rules: ingestion controls (drop metrics or strip tags at the intake) and indexing
// controls (the tags queries may use). Outside live mode the in-browser mock (mock/metricRules.ts) answers.
import type { components } from '../generated/schema';
import { API_MODE, del, get, post, put } from '../http';
import type { TimeRange } from '../types';
import * as mock from '../mock/metricRules';

const live = API_MODE === 'live';

export type MetricRule = components['schemas']['MetricRule'];
export type MetricRuleInput = components['schemas']['MetricRuleInput'];
export type MetricRuleKind = components['schemas']['MetricRuleKind'];
export type MetricRuleAction = components['schemas']['MetricRuleAction'];
export type MetricRulesOverview = components['schemas']['MetricRulesOverview'];
export type MetricCardinality = components['schemas']['MetricCardinality'];

export const listMetricRules = (): Promise<MetricRule[]> => (live ? get('/api/v1/metrics/rules', {}) : mock.listMetricRules());
export const createMetricRule = (i: MetricRuleInput): Promise<MetricRule> => (live ? post('/api/v1/metrics/rules', i) : mock.createMetricRule(i));
export const updateMetricRule = (id: string, i: MetricRuleInput): Promise<MetricRule> => (live ? put('/api/v1/metrics/rules/{id}', { id }, i) : mock.updateMetricRule(id, i));
export const deleteMetricRule = (id: string): Promise<void> => (live ? del('/api/v1/metrics/rules/{id}', { id }) : mock.deleteMetricRule(id));
export const metricRulesOverview = (range: TimeRange, signal?: AbortSignal): Promise<MetricRulesOverview> =>
  live ? post('/api/v1/metrics/rules/overview', { range }, signal) : mock.metricRulesOverview();
