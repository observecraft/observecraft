// Security (Security → Overview, Inbox, Signals, Research Feed; Cloud SIEM). In "live" mode from the
// Rust API, otherwise the in-browser mock (mock/security.ts).
//   GET    /api/v1/security/overview               getSecurityOverview
//   GET    /api/v1/security/inbox                  getSecurityInbox
//   GET    /api/v1/security/feed                   getResearchFeed
//   …      /api/v1/security/rules[/:id[/enabled]]  detection rules
//   …      /api/v1/security/suppressions[/:id]     suppressions
//   POST   /api/v1/security/signals/search         searchSignals
//   GET    /api/v1/security/signals/:id            getSignal
//   PUT    /api/v1/security/signals/:id/state      setSignalState
//   PUT    /api/v1/security/signals/:id/assignee   setSignalAssignee
//   POST   /api/v1/security/signals/bulk           bulkUpdateSignals
//   GET    /api/v1/security/siem/packs             listContentPacks
//   PUT    /api/v1/security/siem/packs/:id         setContentPack
//   POST   /api/v1/security/siem/overview          getSiemOverview
import { API_MODE, del, get, post, put } from '../http';
import * as mock from '../mock/security';
import type {
  ContentPackStatus,
  FeedItem,
  SecurityInbox,
  SecurityOverview,
  SecurityRule,
  SecurityRuleInput,
  SecurityRuleSummary,
  SiemOverview,
  SignalBulkInput,
  SignalDetail,
  SignalSearchPage,
  SignalSearchRequest,
  SignalStateInput,
  Suppression,
  SuppressionInput,
} from './types';

export * from './types';

const live = API_MODE === 'live';

export const getSecurityOverview = (): Promise<SecurityOverview> => (live ? get('/api/v1/security/overview', {}) : mock.securityOverview());
export const getSecurityInbox = (): Promise<SecurityInbox> => (live ? get('/api/v1/security/inbox', {}) : mock.securityInbox());
export const getResearchFeed = (): Promise<FeedItem[]> => (live ? get('/api/v1/security/feed', {}) : mock.researchFeed());

export const listSecurityRules = (): Promise<SecurityRuleSummary[]> => (live ? get('/api/v1/security/rules', {}) : mock.listRules());
export const getSecurityRule = (id: string): Promise<SecurityRule> => (live ? get('/api/v1/security/rules/{id}', { id }) : mock.getRule(id));
export const createSecurityRule = (input: SecurityRuleInput): Promise<SecurityRule> => (live ? post('/api/v1/security/rules', input) : mock.createRule(input));
export const updateSecurityRule = (id: string, input: SecurityRuleInput): Promise<SecurityRule> => (live ? put('/api/v1/security/rules/{id}', { id }, input) : mock.updateRule(id, input));
export const deleteSecurityRule = (id: string): Promise<void> => (live ? del('/api/v1/security/rules/{id}', { id }) : mock.deleteRule(id));
export const setSecurityRuleEnabled = (id: string, enabled: boolean): Promise<SecurityRule> =>
  live ? put('/api/v1/security/rules/{id}/enabled', { id }, { enabled }) : mock.setRuleEnabled(id, enabled);

export const listSuppressions = (): Promise<Suppression[]> => (live ? get('/api/v1/security/suppressions', {}) : mock.listSuppressions());
export const createSuppression = (input: SuppressionInput): Promise<Suppression> => (live ? post('/api/v1/security/suppressions', input) : mock.createSuppression(input));
export const updateSuppression = (id: string, input: SuppressionInput): Promise<Suppression> => (live ? put('/api/v1/security/suppressions/{id}', { id }, input) : mock.updateSuppression(id, input));
export const deleteSuppression = (id: string): Promise<void> => (live ? del('/api/v1/security/suppressions/{id}', { id }) : mock.deleteSuppression(id));

export const searchSignals = (req: SignalSearchRequest, signal?: AbortSignal): Promise<SignalSearchPage> => (live ? post('/api/v1/security/signals/search', req, signal) : mock.searchSignals(req));
export const getSignal = (id: string): Promise<SignalDetail> => (live ? get('/api/v1/security/signals/{id}', { id }) : mock.getSignal(id));
export const setSignalState = (id: string, input: SignalStateInput): Promise<SignalDetail> => (live ? put('/api/v1/security/signals/{id}/state', { id }, input) : mock.setSignalState(id, input));
export const setSignalAssignee = (id: string, assigneeId: string | null): Promise<SignalDetail> =>
  live ? put('/api/v1/security/signals/{id}/assignee', { id }, { assigneeId }) : mock.setSignalAssignee(id, assigneeId);
export const bulkUpdateSignals = (input: SignalBulkInput): Promise<{ updated: number }> => (live ? post('/api/v1/security/signals/bulk', input) : mock.bulkUpdateSignals(input));

export const listContentPacks = (): Promise<ContentPackStatus[]> => (live ? get('/api/v1/security/siem/packs', {}) : mock.listPacks());
export const setContentPack = (id: string, enabled: boolean): Promise<void> => (live ? put('/api/v1/security/siem/packs/{id}', { id }, { enabled }).then(() => undefined) : mock.setPack(id, enabled));
export const getSiemOverview = (range: { from: number; to: number }, signal?: AbortSignal): Promise<SiemOverview> =>
  live ? post('/api/v1/security/siem/overview', { range }, signal) : mock.siemOverview(range);
