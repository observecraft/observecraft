// App & API Protection (Security → App & API Protection). In "live" mode from the Rust API, otherwise
// the in-browser mock (mock/appsec.ts).
//   POST /api/v1/appsec/traces|attackers|users|overview   security traces and what they show
//   GET  /api/v1/appsec/findings|endpoints|services|business-logic|policies|denylist
//   POST|DELETE /api/v1/appsec/denylist[/:kind/:value]     block / unblock
//   …    /api/v1/appsec/waf/:kind[/:id]                   custom rules, exclusion filters, policies
//   GET|PUT /api/v1/appsec/settings/:key                   custom responses, scanners
//   POST /api/v1/code-security/triage                     mute / reopen a finding
import { API_MODE, del, get, post, postTo, put } from '../http';
import * as mock from '../mock/appsec';
import type {
  ApiFindingsPage,
  AppsecOverview,
  AppsecService,
  AppsecUser,
  AttackerPage,
  BusinessLogic,
  DenylistInput,
  DenylistView,
  EndpointPage,
  SecurityTracePage,
  TimeRange,
  WafDoc,
  WafKind,
  WafPolicyView,
} from './types';

export * from './types';

const live = API_MODE === 'live';

export const searchSecurityTraces = (range: TimeRange, query: string): Promise<SecurityTracePage> => (live ? post('/api/v1/appsec/traces', { range, query }) : mock.traces(range, query));
export const listAttackers = (range: TimeRange, query: string): Promise<AttackerPage> => (live ? post('/api/v1/appsec/attackers', { range, query }) : mock.attackers(range, query));
export const listAppsecUsers = (range: TimeRange, query: string): Promise<AppsecUser[]> => (live ? post('/api/v1/appsec/users', { range, query }) : mock.users(range, query));
export const getAppsecOverview = (range: TimeRange): Promise<AppsecOverview> => (live ? post('/api/v1/appsec/overview', { range }) : mock.overview(range));
export const listApiFindings = (): Promise<ApiFindingsPage> => (live ? get('/api/v1/appsec/findings', {}) : mock.findings());
export const listApiEndpoints = (): Promise<EndpointPage> => (live ? get('/api/v1/appsec/endpoints', {}) : mock.endpoints());
export const listAppsecServices = (): Promise<AppsecService[]> => (live ? get('/api/v1/appsec/services', {}) : mock.services());
export const getBusinessLogic = (): Promise<BusinessLogic> => (live ? get('/api/v1/appsec/business-logic', {}) : mock.businessLogic());
export const listWafPolicies = (): Promise<WafPolicyView[]> => (live ? get('/api/v1/appsec/policies', {}) : mock.policies());
export const listDenylist = (): Promise<DenylistView[]> => (live ? get('/api/v1/appsec/denylist', {}) : mock.denylistEntries());
export const blockEntities = (input: DenylistInput): Promise<void> => (live ? post('/api/v1/appsec/denylist', input).then(() => undefined) : mock.block(input));
export const unblockEntity = (kind: string, value: string): Promise<void> => (live ? del('/api/v1/appsec/denylist/{kind}/{value}', { kind, value }) : mock.unblock(kind, value));
export const listWaf = (kind: WafKind): Promise<WafDoc[]> => (live ? (get('/api/v1/appsec/waf/{kind}', { kind }) as Promise<unknown> as Promise<WafDoc[]>) : mock.wafList(kind));
export const createWaf = (kind: WafKind, attributes: Record<string, unknown>): Promise<WafDoc> =>
  live ? (postTo('/api/v1/appsec/waf/{kind}', { kind }, attributes as never) as Promise<unknown> as Promise<WafDoc>) : mock.wafCreate(kind, attributes);
export const updateWaf = (kind: WafKind, id: string, attributes: Record<string, unknown>): Promise<WafDoc> =>
  live ? (put('/api/v1/appsec/waf/{kind}/{id}', { kind, id }, attributes as never) as Promise<unknown> as Promise<WafDoc>) : mock.wafUpdate(kind, id, attributes);
export const deleteWaf = (kind: WafKind, id: string): Promise<void> => (live ? del('/api/v1/appsec/waf/{kind}/{id}', { kind, id }) : mock.wafDelete(kind, id));
export const getAppsecSetting = <T>(key: string): Promise<T> => (live ? (get('/api/v1/appsec/settings/{key}', { key }) as Promise<unknown> as Promise<T>) : mock.setting<T>(key));
export const setAppsecSetting = (key: string, value: unknown): Promise<void> =>
  live ? put('/api/v1/appsec/settings/{key}', { key }, value as never).then(() => undefined) : mock.setSetting(key, value);
export const triageApiFinding = (key: string, status: 'open' | 'muted'): Promise<void> =>
  live ? post('/api/v1/code-security/triage', { key, status, reason: '' }).then(() => undefined) : mock.triage(key, status);
