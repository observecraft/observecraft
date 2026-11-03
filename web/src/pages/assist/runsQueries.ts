// TanStack Query bindings for Assist's runs and the products built on them.
import { keepPreviousData, useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import * as api from '../../api/assist/runsClient';
import type { AgentInput, CodeSettings, CreditSettings, NewMemory, NewRepo, RunReportRequest, RunSearch, RunSettings, StartRun } from '../../api/assist/client';
import type { TimeRange } from '../../api/types';

const K = ['assist-runs'] as const;
const active = (s?: string) => s === 'queued' || s === 'running';

export const useRuns = (q: RunSearch) =>
  useQuery({ queryKey: [...K, 'list', q], queryFn: ({ signal }) => api.searchRuns(q, signal), refetchInterval: (query) => (query.state.data?.some((r) => active(r.status)) ? 2000 : false) });
/** Polls while the run is queued or running. */
export const useRun = (id: string | undefined) =>
  useQuery({ queryKey: [...K, 'run', id], queryFn: ({ signal }) => api.getRun(id!, signal), enabled: !!id, refetchInterval: (query) => (active(query.state.data?.status) ? 1500 : false) });
export const useRunReport = (r: RunReportRequest) => useQuery({ queryKey: [...K, 'report', r], queryFn: ({ signal }) => api.runReport(r, signal), placeholderData: keepPreviousData });
export const useRunSettings = () => useQuery({ queryKey: [...K, 'settings'], queryFn: api.getRunSettings });
export const useMemories = () => useQuery({ queryKey: [...K, 'memories'], queryFn: api.listMemories });
export const useInvestigationMonitors = () => useQuery({ queryKey: [...K, 'monitors'], queryFn: api.investigationMonitors });
export const useAnalyzedSignals = (range: TimeRange) =>
  useQuery({
    queryKey: [...K, 'signals', range],
    queryFn: ({ signal }) => api.analyzedSignals(range, signal),
    placeholderData: keepPreviousData,
    refetchInterval: (q) => (q.state.data?.some((s) => active(s.run?.status)) ? 2000 : false),
  });
export const useAgents = () => useQuery({ queryKey: [...K, 'agents'], queryFn: api.listAgents });
export const useAgent = (id: string | undefined) => useQuery({ queryKey: [...K, 'agent', id], queryFn: () => api.getAgent(id!), enabled: !!id });
export const useCredits = () => useQuery({ queryKey: [...K, 'credits'], queryFn: api.getCredits });
export const useUsage = (range: TimeRange) => useQuery({ queryKey: [...K, 'usage', range], queryFn: ({ signal }) => api.assistUsage(range, signal), placeholderData: keepPreviousData });
export const useCodeConnections = () => useQuery({ queryKey: [...K, 'code-connections'], queryFn: api.codeConnections });
export const useRepos = () => useQuery({ queryKey: [...K, 'repos'], queryFn: api.listRepos });
export const useCodeSettings = () => useQuery({ queryKey: [...K, 'code-settings'], queryFn: api.getCodeSettings });
export const useCodeUsage = (range: TimeRange) => useQuery({ queryKey: [...K, 'code-usage', range], queryFn: ({ signal }) => api.codeUsage(range, signal), placeholderData: keepPreviousData });

/** A mutation that refreshes every Assist runs query when it succeeds. */
function useAssistMutation<A, R>(fn: (a: A) => Promise<R>) {
  const qc = useQueryClient();
  return useMutation({ mutationFn: fn, onSuccess: () => qc.invalidateQueries({ queryKey: K }) });
}

export const useStartRun = () => useAssistMutation((r: StartRun) => api.startRun(r));
export const useCancelRun = () => useAssistMutation((id: string) => api.cancelRun(id));
export const useRunFeedback = () => useAssistMutation((a: { id: string; feedback: 'helpful' | 'not_helpful'; remember?: string }) => api.runFeedback(a.id, a.feedback, a.remember));
export const useSaveRunSettings = () => useAssistMutation((s: RunSettings) => api.putRunSettings(s));
export const useAddMemory = () => useAssistMutation((m: NewMemory) => api.addMemory(m));
export const useDeleteMemory = () => useAssistMutation((id: string) => api.deleteMemory(id));
export const useSetMonitorAuto = () => useAssistMutation((a: { id: number; on: boolean }) => api.setMonitorAuto(a.id, a.on));
export const useSaveAgent = () => useAssistMutation((a: { id?: string; input: AgentInput }) => (a.id ? api.updateAgent(a.id, a.input) : api.createAgent(a.input)));
export const useDeleteAgent = () => useAssistMutation((id: string) => api.deleteAgent(id));
export const useRunAgent = () => useAssistMutation((a: { id: string; input?: string }) => api.runAgent(a.id, a.input));
export const useDraftAgent = () => useMutation({ mutationFn: (description: string) => api.draftAgent(description) });
export const useSaveCredits = () => useAssistMutation((c: CreditSettings) => api.putCredits(c));
export const useAddRepo = () => useAssistMutation((r: NewRepo) => api.addRepo(r));
export const useRemoveRepo = () => useAssistMutation((repo: string) => api.removeRepo(repo));
export const useSaveCodeSettings = () => useAssistMutation((s: CodeSettings) => api.putCodeSettings(s));
export const useStartCodeSession = () => useAssistMutation((a: { repo: string; prompt: string }) => api.startCodeSession(a.repo, a.prompt));
export const useOpenPullRequest = () => useAssistMutation((id: string) => api.openPullRequest(id));
