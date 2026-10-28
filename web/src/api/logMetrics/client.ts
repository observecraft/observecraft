// Generated metrics (Logs → Configuration → Generate Metrics): logs-based metric definitions the
// indexer computes on every ingested log. Outside live mode the in-browser mock (mock/logMetrics.ts) answers.
import { API_MODE, del, get, post, put } from '../http';
import * as mock from '../mock/logMetrics';
import type { LogMetric, LogMetricInput } from './types';

export * from './types';

const live = API_MODE === 'live';

export const listLogMetrics = (): Promise<LogMetric[]> => (live ? get('/api/v1/logs/metrics', {}) : mock.listLogMetrics());
export const createLogMetric = (i: LogMetricInput): Promise<LogMetric> => (live ? post('/api/v1/logs/metrics', i) : mock.createLogMetric(i));
export const updateLogMetric = (id: string, i: LogMetricInput): Promise<LogMetric> => (live ? put('/api/v1/logs/metrics/{id}', { id }, i) : mock.updateLogMetric(id, i));
export const deleteLogMetric = (id: string): Promise<void> => (live ? del('/api/v1/logs/metrics/{id}', { id }) : mock.deleteLogMetric(id));
