// Metrics → Catalog and Overview: per-metric volume and one metric's details (tags, volume over time,
// related assets). Outside live mode the in-browser mock (mock/metricCatalog.ts) answers.
import type { components } from '../generated/schema';
import { API_MODE, post } from '../http';
import type { TimeRange } from '../types';
import * as mock from '../mock/metricCatalog';

const live = API_MODE === 'live';

export type CatalogStats = components['schemas']['CatalogStats'];
export type MetricStats = components['schemas']['MetricStats'];
export type MetricDetail = components['schemas']['MetricDetail'];
export type VolumePoint = components['schemas']['VolumePoint'];

export const catalogStats = (range: TimeRange, signal?: AbortSignal): Promise<CatalogStats> => (live ? post('/api/v1/metrics/catalog/stats', { range }, signal) : mock.catalogStats(range));
export const metricDetail = (name: string, range: TimeRange, signal?: AbortSignal): Promise<MetricDetail> =>
  live ? post('/api/v1/metrics/detail', { name, range }, signal) : mock.metricDetail(name, range);
