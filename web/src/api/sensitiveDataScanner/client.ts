// Sensitive Data Scanner (Organization Settings → Sensitive Data Scanner): scanning groups and rules, the standard
// pattern library and the rule tester. Outside live mode the in-browser mock (mock/sensitiveDataScanner.ts) answers.
import { API_MODE, del, get, post, put } from '../http';
import * as mock from '../mock/sensitiveDataScanner';
import type { SdsConfiguration, SdsGroup, SdsGroupInput, SdsRule, SdsRuleInput, SdsStandardPattern, SdsTestRequest, SdsTestResult } from './types';

export * from './types';

const live = API_MODE === 'live';

export const getConfiguration = (): Promise<SdsConfiguration> => (live ? get('/api/v1/sensitive-data-scanner', {}) : mock.getConfiguration());
export const listStandardPatterns = (): Promise<SdsStandardPattern[]> => (live ? get('/api/v1/sensitive-data-scanner/standard-patterns', {}) : mock.listStandardPatterns());
export const testRule = (req: SdsTestRequest): Promise<SdsTestResult> => (live ? post('/api/v1/sensitive-data-scanner/test', req) : mock.testRule(req));

export const createGroup = (i: SdsGroupInput): Promise<SdsGroup> => (live ? post('/api/v1/sensitive-data-scanner/groups', i) : mock.createGroup(i));
export const updateGroup = (id: string, i: SdsGroupInput): Promise<SdsGroup> => (live ? put('/api/v1/sensitive-data-scanner/groups/{id}', { id }, i) : mock.updateGroup(id, i));
export const deleteGroup = (id: string): Promise<void> => (live ? del('/api/v1/sensitive-data-scanner/groups/{id}', { id }) : mock.deleteGroup(id));
export const reorderGroups = async (ids: string[]): Promise<void> => {
  if (!live) return mock.reorderGroups(ids);
  await put('/api/v1/sensitive-data-scanner/group-order', {}, { ids });
};
export const reorderRules = async (groupId: string, ids: string[]): Promise<void> => {
  if (!live) return mock.reorderRules(groupId, ids);
  await put('/api/v1/sensitive-data-scanner/groups/{id}/rule-order', { id: groupId }, { ids });
};

export const createRule = (i: SdsRuleInput): Promise<SdsRule> => (live ? post('/api/v1/sensitive-data-scanner/rules', i) : mock.createRule(i));
export const updateRule = (id: string, i: SdsRuleInput): Promise<SdsRule> => (live ? put('/api/v1/sensitive-data-scanner/rules/{id}', { id }, i) : mock.updateRule(id, i));
export const deleteRule = (id: string): Promise<void> => (live ? del('/api/v1/sensitive-data-scanner/rules/{id}', { id }) : mock.deleteRule(id));
