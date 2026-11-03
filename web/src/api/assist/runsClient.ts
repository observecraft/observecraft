// Assist's background runs and the products built on them: Investigation, Security Analyst, Agent
// Builder, Code, AI Credits and Usage. In "live" mode the Rust API (routes/assist_runs.rs,
// assist_agents.rs, assist_code.rs, assist_usage.rs) answers; otherwise the in-browser mock
// (mock/assistRuns.ts) plays runs that progress over a few seconds.
import { API_MODE, del, get, post, postTo, put } from '../http';
import * as mock from '../mock/assistRuns';
import type {
  AgentDraft,
  AgentInput,
  AgentView,
  AnalyzedSignal,
  CodeConnections,
  CodeSettings,
  CodeUsage,
  CreditSettings,
  CreditsView,
  InvestigationMonitor,
  Memory,
  NewMemory,
  NewRepo,
  PullRequest,
  RepoView,
  RunDetail,
  RunReport,
  RunReportRequest,
  RunSearch,
  RunSettings,
  RunSummary,
  StartRun,
  UsageReport,
} from './types';
import type { TimeRange } from '../types';

const live = API_MODE === 'live';

export const startRun = (r: StartRun): Promise<RunSummary> => (live ? post('/api/v1/assist/runs', r) : mock.startRun(r));
export const searchRuns = (q: RunSearch, signal?: AbortSignal): Promise<RunSummary[]> => (live ? post('/api/v1/assist/runs/search', q, signal) : mock.searchRuns(q));
export const getRun = (id: string, signal?: AbortSignal): Promise<RunDetail> => (live ? get('/api/v1/assist/runs/{id}', { id }, signal) : mock.getRun(id));
export const cancelRun = (id: string): Promise<void> => (live ? postTo('/api/v1/assist/runs/{id}/cancel', { id }, undefined).then(() => undefined) : mock.cancelRun(id));
export const runFeedback = (id: string, feedback: 'helpful' | 'not_helpful', remember?: string): Promise<void> =>
  live ? postTo('/api/v1/assist/runs/{id}/feedback', { id }, { feedback, remember: remember ?? null }).then(() => undefined) : mock.runFeedback(id, feedback, remember);
export const runReport = (r: RunReportRequest, signal?: AbortSignal): Promise<RunReport> => (live ? post('/api/v1/assist/runs/report', r, signal) : mock.runReport(r));

export const getRunSettings = (): Promise<RunSettings> => (live ? get('/api/v1/assist/run-settings', {}) : mock.getRunSettings());
export const putRunSettings = (s: RunSettings): Promise<void> => (live ? put('/api/v1/assist/run-settings', {}, s) : mock.putRunSettings(s));
export const listMemories = (): Promise<Memory[]> => (live ? get('/api/v1/assist/memories', {}) : mock.listMemories());
export const addMemory = (m: NewMemory): Promise<void> => (live ? post('/api/v1/assist/memories', m).then(() => undefined) : mock.addMemory(m));
export const deleteMemory = (id: string): Promise<void> => (live ? del('/api/v1/assist/memories/{id}', { id }) : mock.deleteMemory(id));
export const investigationMonitors = (): Promise<InvestigationMonitor[]> => (live ? get('/api/v1/assist/investigations/monitors', {}) : mock.investigationMonitors());
export const setMonitorAuto = (id: number, autoInvestigate: boolean): Promise<void> =>
  live ? put('/api/v1/assist/investigations/monitors/{id}', { id: String(id) }, { autoInvestigate }) : mock.setMonitorAuto(id, autoInvestigate);
export const analyzedSignals = (range: TimeRange, signal?: AbortSignal): Promise<AnalyzedSignal[]> => (live ? post('/api/v1/assist/security/signals', { range }, signal) : mock.analyzedSignals(range));

export const listAgents = (): Promise<AgentView[]> => (live ? get('/api/v1/assist/agents', {}) : mock.listAgents());
export const getAgent = (id: string): Promise<AgentView> => (live ? get('/api/v1/assist/agents/{id}', { id }) : mock.getAgent(id));
export const createAgent = (a: AgentInput): Promise<AgentView> => (live ? post('/api/v1/assist/agents', a) : mock.createAgent(a));
export const updateAgent = (id: string, a: AgentInput): Promise<AgentView> => (live ? put('/api/v1/assist/agents/{id}', { id }, a) : mock.updateAgent(id, a));
export const deleteAgent = (id: string): Promise<void> => (live ? del('/api/v1/assist/agents/{id}', { id }) : mock.deleteAgent(id));
export const runAgent = (id: string, input?: string): Promise<RunSummary> => (live ? postTo('/api/v1/assist/agents/{id}/run', { id }, { input: input ?? null }) : mock.runAgent(id, input));
export const draftAgent = (description: string): Promise<AgentDraft> => (live ? post('/api/v1/assist/agents/draft', { description }) : mock.draftAgent(description));

export const getCredits = (): Promise<CreditsView> => (live ? get('/api/v1/assist/credits', {}) : mock.getCredits());
export const putCredits = (c: CreditSettings): Promise<void> => (live ? put('/api/v1/assist/credits', {}, c) : mock.putCredits(c));
export const assistUsage = (range: TimeRange, signal?: AbortSignal): Promise<UsageReport> => (live ? post('/api/v1/assist/usage', { range }, signal) : mock.usage(range));

export const codeConnections = (): Promise<CodeConnections> => (live ? get('/api/v1/assist/code/connections', {}) : mock.codeConnections());
export const listRepos = (): Promise<RepoView[]> => (live ? get('/api/v1/assist/code/repos', {}) : mock.listRepos());
export const addRepo = (r: NewRepo): Promise<void> => (live ? post('/api/v1/assist/code/repos', r).then(() => undefined) : mock.addRepo(r));
export const removeRepo = (repo: string): Promise<void> => (live ? post('/api/v1/assist/code/repos/remove', { repo }).then(() => undefined) : mock.removeRepo(repo));
export const getCodeSettings = (): Promise<CodeSettings> => (live ? get('/api/v1/assist/code/settings', {}) : mock.getCodeSettings());
export const putCodeSettings = (s: CodeSettings): Promise<void> => (live ? put('/api/v1/assist/code/settings', {}, s) : mock.putCodeSettings(s));
export const startCodeSession = (repo: string, prompt: string): Promise<RunSummary> => (live ? post('/api/v1/assist/code/sessions', { repo, prompt }) : mock.startCodeSession(repo, prompt));
export const openPullRequest = (id: string): Promise<PullRequest> => (live ? postTo('/api/v1/assist/code/sessions/{id}/pull-request', { id }, undefined) : mock.openPullRequest(id));
export const codeUsage = (range: TimeRange, signal?: AbortSignal): Promise<CodeUsage> => (live ? post('/api/v1/assist/code/usage', { range }, signal) : mock.codeUsage(range));
