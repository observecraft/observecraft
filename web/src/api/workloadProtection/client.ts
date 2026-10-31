// Workload Protection (Security → Workload Protection). In "live" mode from the Rust API, otherwise
// the in-browser mock (mock/workloadProtection.ts).
//   GET  /api/v1/workload-protection/summary?from&to        the overview (agents, events, signals, findings, MITRE, packs)
//   POST /api/v1/workload-protection/events                 agent events explorer (list, series by category, facets)
//   GET  /api/v1/workload-protection/findings?from&to       finding rules and failing hosts / containers
//   GET  /api/v1/workload-protection/agents                 hosts & containers and serverless Agents
//   …    /api/v1/workload-protection/policies[/:id]         agent policies (the Datadog API's records)
//   …    /api/v1/workload-protection/agent-rules[/:id]      agent rules
//   GET  /api/v1/workload-protection/policies/download      the enabled policies as Agent policy files (zip)
//   POST /api/v1/code-security/triage                       mute / reopen a finding
// Detection rules are Cloud SIEM rules of type workload_security (api/security); signals likewise.
import { API_MODE, del, get, patch, post } from '../http';
import * as mock from '../mock/workloadProtection';
import type { WpAgentRule, WpAgentRuleInput, WpAgents, WpEventsPage, WpFindings, WpPolicy, WpPolicyInput, WpSummary, TimeRange } from './types';

export * from './types';

const live = API_MODE === 'live';
const q = (r: TimeRange) => ({ from: r.from, to: r.to });

export const getWpSummary = (range: TimeRange): Promise<WpSummary> => (live ? get('/api/v1/workload-protection/summary', {}, undefined, q(range)) : mock.summary(range));
export const searchWpEvents = (range: TimeRange, query: string, limit = 100): Promise<WpEventsPage> =>
  live ? post('/api/v1/workload-protection/events', { range, query, limit }) : mock.events(range, query, limit);
export const listWpFindings = (range: TimeRange): Promise<WpFindings> => (live ? get('/api/v1/workload-protection/findings', {}, undefined, q(range)) : mock.findings(range));
export const listWpAgents = (): Promise<WpAgents> => (live ? get('/api/v1/workload-protection/agents', {}) : mock.agents());
export const listWpPolicies = (): Promise<WpPolicy[]> => (live ? get('/api/v1/workload-protection/policies', {}) : mock.policies());
export const createWpPolicy = (input: WpPolicyInput): Promise<WpPolicy> => (live ? post('/api/v1/workload-protection/policies', input) : mock.createPolicy(input));
export const updateWpPolicy = (id: string, input: WpPolicyInput): Promise<WpPolicy> => (live ? patch('/api/v1/workload-protection/policies/{id}', { id }, input) : mock.updatePolicy(id, input));
export const deleteWpPolicy = (id: string): Promise<void> => (live ? del('/api/v1/workload-protection/policies/{id}', { id }) : mock.deletePolicy(id));
export const listWpAgentRules = (range: TimeRange, policyId?: string): Promise<WpAgentRule[]> =>
  live ? get('/api/v1/workload-protection/agent-rules', {}, undefined, { ...q(range), ...(policyId ? { policy_id: policyId } : {}) }) : mock.agentRules(range, policyId);
export const createWpAgentRule = (input: WpAgentRuleInput): Promise<WpAgentRule> => (live ? post('/api/v1/workload-protection/agent-rules', input) : mock.createAgentRule(input));
export const updateWpAgentRule = (id: string, input: WpAgentRuleInput): Promise<WpAgentRule> =>
  live ? patch('/api/v1/workload-protection/agent-rules/{id}', { id }, input) : mock.updateAgentRule(id, input);
export const deleteWpAgentRule = (id: string): Promise<void> => (live ? del('/api/v1/workload-protection/agent-rules/{id}', { id }) : mock.deleteAgentRule(id));
/** The enabled policies as Agent policy files: a zip (live) or the policy files as text (mock). */
export const downloadWpPolicies = async (): Promise<Blob> =>
  live ? (await fetch('/api/v1/workload-protection/policies/download', { credentials: 'include' })).blob() : new Blob([mock.policyFiles()], { type: 'text/yaml' });
export const triageWpFinding = (key: string, status: 'open' | 'muted'): Promise<void> =>
  live ? post('/api/v1/code-security/triage', { key, status, reason: '' }).then(() => undefined) : mock.triage(key, status);
