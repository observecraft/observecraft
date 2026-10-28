import { emptyTrigger, normalizeStep } from '../../../api/workflows/logic';
import type { Workflow, WorkflowInput, WorkflowSpec, WorkflowTrigger } from '../../../api/workflows/types';

/** What the editor changes before saving. */
export interface Draft {
  name: string;
  handle: string;
  description: string;
  tags: string[];
  ownerTeam: string;
  service: string;
  blueprint: string;
  spec: WorkflowSpec;
  /** Secret changes: a value sets one, null removes it. */
  secrets: Record<string, string | null>;
}

const trigger = (t: Partial<WorkflowTrigger> & { kind: WorkflowTrigger['kind'] }): WorkflowTrigger => ({ ...emptyTrigger(t.kind), transitions: [], ...t });

export function fromWorkflow(w: Workflow): Draft {
  return {
    name: w.name,
    handle: w.handle,
    description: w.description,
    tags: w.tags,
    ownerTeam: w.ownerTeam,
    service: w.service,
    blueprint: w.blueprint,
    spec: {
      inputs: w.spec.inputs ?? [],
      triggers: (w.spec.triggers ?? []).map(trigger),
      steps: (w.spec.steps ?? []).map(normalizeStep),
    },
    secrets: {},
  };
}

export function toInput(d: Draft): WorkflowInput {
  return { ...d, secrets: Object.keys(d.secrets).length ? d.secrets : undefined };
}

/** The draft differs from what was saved. */
export const changed = (a: Draft, saved: Draft) => JSON.stringify(a) !== JSON.stringify(saved);
