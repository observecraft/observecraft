// The action catalog: every step kind a workflow can use, by category.
import { emptyStep } from '../../../api/workflows/logic';
import type { WorkflowStep, WorkflowStepKind } from '../../../api/workflows/types';

export interface CatalogEntry {
  kind: WorkflowStepKind;
  description: string;
  /** Logos of tools this step commonly reaches (registry keys). */
  logos?: string[];
}

export const CATALOG: { category: string; entries: CatalogEntry[] }[] = [
  {
    category: 'Control flow',
    entries: [
      { kind: 'if', description: 'Run one branch or the other, depending on a condition over the run’s data.' },
      { kind: 'for_each', description: 'Run the steps inside once per item of a list (up to 100).' },
      { kind: 'wait', description: 'Pause the run for up to an hour; it resumes where it stopped.' },
    ],
  },
  {
    category: 'Notifications',
    entries: [{ kind: 'notify', description: 'Send a message to Slack or webhook channels and to members by email.', logos: ['slack', 'webhook'] }],
  },
  {
    category: 'Observability',
    entries: [
      { kind: 'metric_query', description: 'Read a metric query or formula over the last minutes, per group.' },
      { kind: 'log_count', description: 'Count the logs matching a search, optionally per facet value.' },
      { kind: 'synthetic_run', description: 'Run a synthetic test now and fail the step when it fails.' },
    ],
  },
  {
    category: 'Incident response',
    entries: [
      { kind: 'create_incident', description: 'Declare an incident with a title, severity, services and teams.' },
      { kind: 'page_on_call', description: 'Page an on-call team through its escalation policy.' },
      { kind: 'incident_comment', description: 'Add a note to an incident’s timeline.' },
    ],
  },
  {
    category: 'HTTP',
    entries: [{ kind: 'http', description: 'Call any HTTP API (1–5 requests in order), with stored secrets for credentials.', logos: ['github', 'jira', 'pagerduty'] }],
  },
];

/** A base for the id of a new step of `kind`. */
export const ID_BASE: Record<WorkflowStepKind, string> = {
  http: 'request',
  create_incident: 'declare_incident',
  page_on_call: 'page',
  incident_comment: 'incident_note',
  synthetic_run: 'synthetic_test',
  notify: 'notify',
  metric_query: 'metric',
  log_count: 'logs',
  wait: 'wait',
  if: 'condition',
  for_each: 'for_each',
};

const STARTER_FIELDS: Partial<Record<WorkflowStepKind, Record<string, string>>> = {
  create_incident: { title: '{{ Workflow.name }}', severity: 'SEV-3' },
  page_on_call: { team: 'checkout', title: '{{ Workflow.name }} needs you', urgency: 'high' },
  incident_comment: { incident: '1', body: 'Note from {{ Workflow.name }}' },
  synthetic_run: { test: 'checkout-flow' },
  notify: { recipients: '@slack-checkout', message: 'Hello from {{ Workflow.name }}' },
  metric_query: { query: 'avg:system.cpu.user{*}', minutes: '15', aggregation: 'avg' },
  log_count: { query: 'status:error', minutes: '15' },
  wait: { seconds: '60' },
};

/** A new step of `kind` with values that pass the checks, so a draft can be saved right away. */
export function starterStep(id: string, kind: WorkflowStepKind): WorkflowStep {
  const s = emptyStep(id, kind);
  s.fields = { ...s.fields, ...(STARTER_FIELDS[kind] ?? {}) };
  if (kind === 'http') s.requests = [{ method: 'GET', url: 'https://api.example.com/health', headers: [], body: '' }];
  if (kind === 'if') s.expression = 'true';
  return s;
}
