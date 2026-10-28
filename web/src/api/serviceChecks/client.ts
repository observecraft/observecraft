// Monitoring → Check Summary: service checks (the `service_check.status` gauge the intake writes from
// the Datadog Agent's check runs) and each check's status per host. Outside live mode the
// in-browser mock (mock/serviceChecks.ts) answers.
//   GET  /api/v1/service-checks          listServiceChecks
//   POST /api/v1/service-checks/detail   serviceCheckDetail
import type { components } from '../generated/schema';
import { API_MODE, get, post } from '../http';
import type { TimeRange } from '../types';
import * as mock from '../mock/serviceChecks';

const live = API_MODE === 'live';

export type ServiceCheckEntry = components['schemas']['ServiceCheckEntry'];
export type ServiceCheckDetail = components['schemas']['ServiceCheckDetail'];
export type ServiceCheckGroup = components['schemas']['ServiceCheckGroup'];

export const listServiceChecks = (signal?: AbortSignal): Promise<ServiceCheckEntry[]> => (live ? get('/api/v1/service-checks', {}, signal) : mock.listServiceChecks());

export const serviceCheckDetail = (check: string, range: TimeRange, signal?: AbortSignal): Promise<ServiceCheckDetail> =>
  live ? post('/api/v1/service-checks/detail', { check, range }, signal) : mock.serviceCheckDetail(check, range);
