// On-Call (Incident Response → On-Call): schedules, escalation policies, team routing, pages, profiles
// and analytics. In "live" mode they come from the Rust API (routes/on_call.rs); otherwise the
// in-browser mock answers (mock/onCall.ts).
//   GET    /api/v1/on-call/schedules                         listOnCallSchedules
//   POST   /api/v1/on-call/schedules                         createOnCallSchedule
//   GET    /api/v1/on-call/schedules/:id                     getOnCallSchedule
//   PUT    /api/v1/on-call/schedules/:id                     updateOnCallSchedule
//   DELETE /api/v1/on-call/schedules/:id                     deleteOnCallSchedule
//   POST   /api/v1/on-call/schedules/:id/shifts              onCallScheduleShifts
//   POST   /api/v1/on-call/schedules/preview                 previewOnCallSchedule
//   POST   /api/v1/on-call/schedules/:id/overrides           createOnCallOverride
//   DELETE /api/v1/on-call/schedules/:id/overrides/:oid      deleteOnCallOverride
//   POST   /api/v1/on-call/my-shifts                         myOnCallShifts
//   GET    /api/v1/on-call/escalation-policies               listOnCallPolicies (+ POST, GET/PUT/DELETE :id)
//   GET    /api/v1/on-call/teams                             listOnCallTeams
//   PUT    /api/v1/on-call/teams/:handle/routing             setOnCallRouting
//   POST   /api/v1/on-call/pages/search                      searchOnCallPages
//   POST   /api/v1/on-call/pages                             createOnCallPage
//   GET    /api/v1/on-call/pages/:id                         getOnCallPage
//   POST   /api/v1/on-call/pages/:id/{acknowledge,resolve,reassign}
//   GET    /api/v1/on-call/profile  (+ PUT, POST /test)      getOnCallProfile
//   POST   /api/v1/on-call/analytics                         onCallAnalytics
import { API_MODE, del, get, post, postTo, put } from '../http';
import * as mock from '../mock/onCall';
import type {
  CreateOnCallOverride,
  CreateOnCallPage,
  OnCallAnalytics,
  OnCallAnalyticsRequest,
  OnCallMyShift,
  OnCallOverride,
  OnCallPage,
  OnCallPageDetail,
  OnCallPageSearch,
  OnCallPagesResult,
  OnCallPolicy,
  OnCallPolicyInput,
  OnCallPreviewRequest,
  OnCallProfile,
  OnCallProfileInput,
  OnCallRange,
  OnCallSchedule,
  OnCallScheduleInput,
  OnCallScheduleShifts,
  OnCallTeam,
  OnCallTestResult,
  ReassignOnCallPage,
} from './types';

export * from './types';

const live = API_MODE === 'live';

export const listOnCallSchedules = (): Promise<OnCallSchedule[]> => (live ? get('/api/v1/on-call/schedules', {}) : mock.listSchedules());
export const getOnCallSchedule = (id: string, signal?: AbortSignal): Promise<OnCallSchedule> => (live ? get('/api/v1/on-call/schedules/{id}', { id }, signal) : mock.getSchedule(id));
export const createOnCallSchedule = (input: OnCallScheduleInput): Promise<OnCallSchedule> => (live ? post('/api/v1/on-call/schedules', input) : mock.createSchedule(input));
export const updateOnCallSchedule = (id: string, input: OnCallScheduleInput): Promise<OnCallSchedule> => (live ? put('/api/v1/on-call/schedules/{id}', { id }, input) : mock.updateSchedule(id, input));
export const deleteOnCallSchedule = (id: string): Promise<void> => (live ? del('/api/v1/on-call/schedules/{id}', { id }) : mock.deleteSchedule(id));
export const onCallScheduleShifts = (id: string, range: OnCallRange, signal?: AbortSignal): Promise<OnCallScheduleShifts> =>
  live ? postTo('/api/v1/on-call/schedules/{id}/shifts', { id }, range, signal) : mock.scheduleShifts(id, range);
export const previewOnCallSchedule = (req: OnCallPreviewRequest, signal?: AbortSignal): Promise<OnCallScheduleShifts> =>
  live ? post('/api/v1/on-call/schedules/preview', req, signal) : mock.previewSchedule(req);
export const createOnCallOverride = (id: string, req: CreateOnCallOverride): Promise<OnCallOverride> =>
  live ? postTo('/api/v1/on-call/schedules/{id}/overrides', { id }, req) : mock.createOverride(id, req);
export const deleteOnCallOverride = (id: string, overrideId: string): Promise<void> =>
  live ? del('/api/v1/on-call/schedules/{id}/overrides/{override_id}', { id, override_id: overrideId }) : mock.deleteOverride(id, overrideId);
export const myOnCallShifts = (range: OnCallRange, signal?: AbortSignal): Promise<OnCallMyShift[]> => (live ? post('/api/v1/on-call/my-shifts', range, signal) : mock.myShifts(range));

export const listOnCallPolicies = (): Promise<OnCallPolicy[]> => (live ? get('/api/v1/on-call/escalation-policies', {}) : mock.listPolicies());
export const getOnCallPolicy = (id: string, signal?: AbortSignal): Promise<OnCallPolicy> => (live ? get('/api/v1/on-call/escalation-policies/{id}', { id }, signal) : mock.getPolicy(id));
export const createOnCallPolicy = (input: OnCallPolicyInput): Promise<OnCallPolicy> => (live ? post('/api/v1/on-call/escalation-policies', input) : mock.createPolicy(input));
export const updateOnCallPolicy = (id: string, input: OnCallPolicyInput): Promise<OnCallPolicy> =>
  live ? put('/api/v1/on-call/escalation-policies/{id}', { id }, input) : mock.updatePolicy(id, input);
export const deleteOnCallPolicy = (id: string): Promise<void> => (live ? del('/api/v1/on-call/escalation-policies/{id}', { id }) : mock.deletePolicy(id));

export const listOnCallTeams = (): Promise<OnCallTeam[]> => (live ? get('/api/v1/on-call/teams', {}) : mock.listTeams());
/** Which escalation policy a team's pages follow (null: every member is paged). */
export const setOnCallRouting = (handle: string, policyId: string | null): Promise<void> =>
  live ? put('/api/v1/on-call/teams/{handle}/routing', { handle }, { policyId }) : mock.setRouting(handle, policyId);

export const searchOnCallPages = (req: OnCallPageSearch, signal?: AbortSignal): Promise<OnCallPagesResult> => (live ? post('/api/v1/on-call/pages/search', req, signal) : mock.searchPages(req));
export const getOnCallPage = (id: number, signal?: AbortSignal): Promise<OnCallPageDetail> => (live ? get('/api/v1/on-call/pages/{id}', { id: String(id) }, signal) : mock.getPage(id));
export const createOnCallPage = (req: CreateOnCallPage): Promise<OnCallPage> => (live ? post('/api/v1/on-call/pages', req) : mock.createPage(req));
export const acknowledgeOnCallPage = (id: number): Promise<OnCallPage> => (live ? postTo('/api/v1/on-call/pages/{id}/acknowledge', { id: String(id) }, undefined) : mock.acknowledgePage(id));
export const resolveOnCallPage = (id: number): Promise<OnCallPage> => (live ? postTo('/api/v1/on-call/pages/{id}/resolve', { id: String(id) }, undefined) : mock.resolvePage(id));
export const reassignOnCallPage = (id: number, req: ReassignOnCallPage): Promise<OnCallPage> =>
  live ? postTo('/api/v1/on-call/pages/{id}/reassign', { id: String(id) }, req) : mock.reassignPage(id, req);

export const getOnCallProfile = (): Promise<OnCallProfile> => (live ? get('/api/v1/on-call/profile', {}) : mock.getProfile());
export const updateOnCallProfile = (input: OnCallProfileInput): Promise<OnCallProfile> => (live ? put('/api/v1/on-call/profile', {}, input) : mock.updateProfile(input));
/** Sends a test notification to the signed-in member's high-urgency channels. */
export const testOnCallProfile = (): Promise<OnCallTestResult[]> => (live ? postTo('/api/v1/on-call/profile/test', {}, undefined) : mock.testProfile());

export const onCallAnalytics = (req: OnCallAnalyticsRequest, signal?: AbortSignal): Promise<OnCallAnalytics> => (live ? post('/api/v1/on-call/analytics', req, signal) : mock.analytics(req));
