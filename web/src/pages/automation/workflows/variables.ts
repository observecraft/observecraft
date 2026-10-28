// What a step's templates and expressions can read: the variable picker's suggestions, by context
// (the workflow's triggers and inputs, earlier steps, and the "for each" loops around the step).
import { allSteps } from '../../../api/workflows/logic';
import type { WorkflowSpec, WorkflowStep, WorkflowStepKind, WorkflowTriggerKind } from '../../../api/workflows/types';

export interface Variable {
  path: string;
  group: string;
  hint: string;
}

const TRIGGER_VARS: Record<WorkflowTriggerKind, [string, string][]> = {
  monitor: [
    ['Trigger.monitor.name', 'Monitor name'],
    ['Trigger.monitor.id', 'Monitor id'],
    ['Trigger.monitor.tags', 'Monitor tags (list)'],
    ['Trigger.monitor.url', 'Link to the monitor'],
    ['Trigger.transition.to', 'New state: alert, warn, nodata, ok'],
    ['Trigger.group', 'Group that changed'],
    ['Trigger.value', 'Value that crossed the threshold'],
    ['Trigger.message', 'Rendered monitor message'],
  ],
  incident: [
    ['Trigger.event', 'declared, updated or resolved'],
    ['Trigger.incident.id', 'Incident number'],
    ['Trigger.incident.title', 'Incident title'],
    ['Trigger.incident.severity', 'Severity'],
    ['Trigger.incident.services', 'Services (list)'],
    ['Trigger.url', 'Link to the incident'],
  ],
  schedule: [['Trigger.scheduledAt', 'Scheduled time (ms)']],
  manual: [['Trigger.user.name', 'Who ran it']],
  api: [['Trigger.user.name', 'Owner of the application key']],
  app: [
    ['Trigger.app', 'App name'],
    ['Trigger.user.name', 'Who pressed the button'],
  ],
};

/** The output fields each step kind produces. */
export const OUTPUT_FIELDS: Record<WorkflowStepKind, string[]> = {
  http: ['statusCode', 'body', 'link'],
  create_incident: ['incident', 'title', 'severity', 'link'],
  page_on_call: ['page', 'team', 'title', 'link'],
  incident_comment: ['incident', 'link'],
  synthetic_run: ['test', 'passed', 'link'],
  notify: ['sent', 'failed'],
  metric_query: ['value', 'groups'],
  log_count: ['count', 'groups'],
  wait: ['seconds', 'until'],
  if: ['result', 'branch'],
  for_each: ['items'],
};

/** The "for each" steps around `id`, outermost first. */
export function loopsAround(list: WorkflowStep[], id: string, around: WorkflowStep[] = []): WorkflowStep[] | null {
  for (const s of list) {
    if (s.id === id) return around;
    const inner = s.kind === 'for_each' ? [...around, s] : around;
    const found = loopsAround(s.steps, id, inner) ?? loopsAround(s.elseSteps, id, around);
    if (found) return found;
  }
  return null;
}

/** What the templates of step `id` (or of a new step when null) can read. */
export function variablesFor(spec: WorkflowSpec, id: string | null): Variable[] {
  const out: Variable[] = [];
  const kinds = new Set(spec.triggers.map((t) => t.kind));
  const seen = new Set<string>();
  for (const k of kinds)
    for (const [path, hint] of TRIGGER_VARS[k]) {
      if (seen.has(path)) continue;
      seen.add(path);
      out.push({ path, group: 'Trigger', hint });
    }
  for (const i of spec.inputs) out.push({ path: `inputs.${i.key}`, group: 'Inputs', hint: i.label || i.key });
  for (const s of allSteps(spec.steps)) {
    if (s.id === id) continue;
    out.push({ path: `Steps.${s.id}.status`, group: 'Steps', hint: `${s.name || s.id}: succeeded or failed` });
    for (const f of OUTPUT_FIELDS[s.kind]) out.push({ path: `Steps.${s.id}.output.${f}`, group: 'Steps', hint: s.name || s.id });
  }
  if (id && (loopsAround(spec.steps, id)?.length ?? 0) > 0) {
    out.push({ path: 'Loop.item', group: 'Loop', hint: 'Current item' }, { path: 'Loop.index', group: 'Loop', hint: 'Position of the item, from 0' });
  }
  out.push(
    { path: 'Workflow.name', group: 'Workflow', hint: 'This workflow’s name' },
    { path: 'Workflow.url', group: 'Workflow', hint: 'Link to this run' },
    { path: 'user.name', group: 'Workflow', hint: 'Member the run acts as' },
    { path: 'user.email', group: 'Workflow', hint: 'Their email' },
  );
  return out;
}
