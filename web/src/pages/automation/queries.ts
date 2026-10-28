// TanStack Query bindings for Workflow Automation.
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import {
  cancelWorkflowRun,
  createWorkflow,
  createWorkflowToken,
  deleteWorkflow,
  getWorkflow,
  getWorkflowRun,
  listWorkflowRuns,
  listWorkflows,
  publishWorkflow,
  runWorkflow,
  unpublishWorkflow,
  updateWorkflow,
  type Workflow,
  type WorkflowInput,
  type WorkflowRun,
  type WorkflowValues,
} from '../../api/workflows/client';

export const workflowKeys = {
  all: ['workflows'] as const,
  one: (id: string) => ['workflows', id] as const,
  runs: (id: string) => ['workflows', id, 'runs'] as const,
  run: (id: string, run: number) => ['workflows', id, 'runs', run] as const,
};

const active = (r: WorkflowRun | undefined) => !!r && (r.status === 'queued' || r.status === 'running' || r.status === 'waiting');

export const useWorkflows = () => useQuery({ queryKey: workflowKeys.all, queryFn: ({ signal }) => listWorkflows(signal), retry: 1 });
export const useWorkflow = (id: string | undefined) => useQuery({ queryKey: workflowKeys.one(id ?? ''), queryFn: ({ signal }) => getWorkflow(id!, signal), enabled: !!id, retry: 1 });
/** The runs of a workflow, refreshed while one is in progress. */
export const useWorkflowRuns = (id: string | undefined) =>
  useQuery({
    queryKey: workflowKeys.runs(id ?? ''),
    queryFn: ({ signal }) => listWorkflowRuns(id!, signal),
    enabled: !!id,
    retry: 1,
    refetchInterval: (q) => (q.state.data?.some(active) ? 1500 : 15_000),
  });
/** One run with its steps, polled until it ends. */
export const useWorkflowRun = (id: string | undefined, run: number | undefined) =>
  useQuery({
    queryKey: workflowKeys.run(id ?? '', run ?? 0),
    queryFn: ({ signal }) => getWorkflowRun(id!, run!, signal),
    enabled: !!id && !!run,
    retry: 1,
    refetchInterval: (q) => (active(q.state.data) ? 1000 : false),
  });

function useWorkflowMutation<V>(fn: (v: V) => Promise<Workflow>) {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: fn,
    onSuccess: (w) => {
      qc.setQueryData(workflowKeys.one(w.id), w);
      qc.invalidateQueries({ queryKey: workflowKeys.all, exact: true });
    },
  });
}

export const useSaveWorkflow = () => useWorkflowMutation(({ id, input }: { id?: string; input: WorkflowInput }) => (id ? updateWorkflow(id, input) : createWorkflow(input)));
export const usePublishWorkflow = () => useWorkflowMutation((id: string) => publishWorkflow(id));
export const useUnpublishWorkflow = () => useWorkflowMutation((id: string) => unpublishWorkflow(id));

export function useDeleteWorkflow() {
  const qc = useQueryClient();
  return useMutation({ mutationFn: deleteWorkflow, onSuccess: () => qc.invalidateQueries({ queryKey: workflowKeys.all }) });
}

export function useCreateWorkflowToken() {
  const qc = useQueryClient();
  return useMutation({ mutationFn: createWorkflowToken, onSuccess: (_, id) => qc.invalidateQueries({ queryKey: workflowKeys.one(id) }) });
}

export function useRunWorkflow(id: string) {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (inputs: WorkflowValues) => runWorkflow(id, inputs),
    onSuccess: (r) => {
      qc.setQueryData(workflowKeys.run(id, r.id), r);
      qc.invalidateQueries({ queryKey: workflowKeys.runs(id) });
      qc.invalidateQueries({ queryKey: workflowKeys.all, exact: true });
    },
  });
}

export function useCancelWorkflowRun(id: string) {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (run: number) => cancelWorkflowRun(id, run),
    onSuccess: (r) => {
      qc.setQueryData(workflowKeys.run(id, r.id), r);
      qc.invalidateQueries({ queryKey: workflowKeys.runs(id) });
    },
  });
}
