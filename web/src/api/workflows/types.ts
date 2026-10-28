import type { components } from '../generated/schema';
import type { JsonValue } from '../types';
import type { ActionHttpRequest, ActionInput } from '../portalActions/types';

type S = components['schemas'];

export type WorkflowStepKind = S['WorkflowStepKind'];
export type WorkflowTriggerKind = S['WorkflowTriggerKind'];

/** A trigger with every field present (the server fills the defaults). */
export interface WorkflowTrigger {
  kind: WorkflowTriggerKind;
  /** Schedule: `minute hour day-of-month month day-of-week`. */
  cron: string;
  /** Schedule: IANA zone, UTC when empty. */
  timezone: string;
  /** Monitor: alert, warn, nodata, ok (alert when empty). */
  transitions: string[];
  /** Incident: declared, updated, resolved (all when empty). */
  incidentEvents: string[];
  /** Incident: SEV-1 … (any when empty). */
  severities: string[];
}

/** A step of a workflow (see routes/workflow_engine.rs). */
export interface WorkflowStep {
  id: string;
  kind: WorkflowStepKind;
  name: string;
  requests: ActionHttpRequest[];
  fields: Record<string, string>;
  /** If: the condition. For each: the list. */
  expression: string;
  /** If: the "true" branch. For each: the body. */
  steps: WorkflowStep[];
  /** If: the "false" branch. */
  elseSteps: WorkflowStep[];
  retries: number;
  continueOnError: boolean;
}

export interface WorkflowSpec {
  inputs: ActionInput[];
  triggers: WorkflowTrigger[];
  steps: WorkflowStep[];
}

export type Workflow = Omit<S['Workflow'], 'spec' | 'publishedSpec'> & { spec: WorkflowSpec; publishedSpec: WorkflowSpec | null };

/** What the editor saves: secrets by name, a value sets one, null removes it, names left out keep theirs. */
export type WorkflowInput = Omit<S['WorkflowInput'], 'spec' | 'secrets'> & { spec: WorkflowSpec; secrets?: Record<string, string | null> };

export type WorkflowRunStatus = 'queued' | 'running' | 'waiting' | 'succeeded' | 'failed' | 'canceled';

export type WorkflowRunStep = Omit<S['WorkflowRunStep'], 'inputs' | 'output' | 'status' | 'kind'> & {
  kind: WorkflowStepKind;
  status: 'succeeded' | 'failed';
  inputs: Record<string, JsonValue>;
  output: JsonValue;
};

export type WorkflowRun = Omit<S['WorkflowRun'], 'trigger' | 'inputs' | 'steps' | 'spec' | 'status' | 'triggerKind'> & {
  status: WorkflowRunStatus;
  triggerKind: WorkflowTriggerKind;
  trigger: Record<string, JsonValue>;
  inputs: Record<string, JsonValue>;
  steps: WorkflowRunStep[];
  spec?: WorkflowSpec | null;
};

/** Input values of a run, by input key. */
export type WorkflowValues = Record<string, string | number | boolean>;
