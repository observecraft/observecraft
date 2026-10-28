import { useNavigate } from 'react-router-dom';
import type { WorkflowInput } from '../../../api/workflows/types';
import { useSaveWorkflow, useWorkflows } from '../queries';
import type { WorkflowBlueprint } from './blueprints';

/** A name not used yet: `Name`, `Name (2)`, … */
export function freeName(name: string, taken: string[]): string {
  if (!taken.includes(name)) return name;
  for (let i = 2; ; i++) if (!taken.includes(`${name} (${i})`)) return `${name} (${i})`;
}

/** Creates a draft workflow (from a blueprint or a document) and opens it in the editor. */
export function useCreateDraft() {
  const navigate = useNavigate();
  const save = useSaveWorkflow();
  const list = useWorkflows();
  const create = (input: WorkflowInput, select?: string) => {
    const taken = (list.data ?? []).map((w) => w.name);
    save.mutate({ input: { ...input, name: freeName(input.name, taken) } }, { onSuccess: (w) => navigate(`/automation/workflows/${w.id}${select ? `?step=${select}` : ''}`) });
  };
  return Object.assign(create, { pending: save.isPending, error: save.error as Error | null });
}

export function useStartFromBlueprint() {
  const create = useCreateDraft();
  return Object.assign((b: WorkflowBlueprint) => create(b.build()), { pending: create.pending, error: create.error });
}
