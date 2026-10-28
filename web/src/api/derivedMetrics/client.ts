// Metrics → Derived Metrics: metric names that queries resolve to a formula over other metric queries,
// evaluated at query time. Outside live mode the in-browser mock (mock/derivedMetrics.ts) answers.
import type { components } from '../generated/schema';
import { API_MODE, del, get, post, put } from '../http';
import * as mock from '../mock/derivedMetrics';

const live = API_MODE === 'live';

export type DerivedMetric = components['schemas']['DerivedMetric'];
export type DerivedMetricInput = components['schemas']['DerivedMetricInput'];

export const listDerivedMetrics = (): Promise<DerivedMetric[]> => (live ? get('/api/v1/metrics/derived', {}) : mock.listDerivedMetrics());
export const createDerivedMetric = (i: DerivedMetricInput): Promise<DerivedMetric> => (live ? post('/api/v1/metrics/derived', i) : mock.createDerivedMetric(i));
export const updateDerivedMetric = (id: string, i: DerivedMetricInput): Promise<DerivedMetric> => (live ? put('/api/v1/metrics/derived/{id}', { id }, i) : mock.updateDerivedMetric(id, i));
export const deleteDerivedMetric = (id: string): Promise<void> => (live ? del('/api/v1/metrics/derived/{id}', { id }) : mock.deleteDerivedMetric(id));
