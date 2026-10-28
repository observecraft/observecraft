// Workflow blueprints: ready-made workflows to start from. Fictional e-commerce names; the steps use
// only ObserveCraft's own step kinds and placeholder hosts (`*.example.com`).
import { emptyTrigger, normalizeStep } from '../../../api/workflows/logic';
import type { WorkflowInput, WorkflowStep, WorkflowTriggerKind } from '../../../api/workflows/types';

export type BlueprintCategory = 'Incident response' | 'Remediation' | 'Reporting' | 'Developer workflows';

export interface WorkflowBlueprint {
  key: string;
  title: string;
  description: string;
  category: BlueprintCategory;
  /** Logo registry keys, or `monitor` / `schedule` / `incident` / `internal` for our own icons. */
  logos: string[];
  trigger: WorkflowTriggerKind;
  /** Words the "What do you want to automate?" prompt matches. */
  keywords: string[];
  build: () => WorkflowInput;
}

const step = (id: string, kind: WorkflowStep['kind'], extra: Partial<WorkflowStep> = {}) => normalizeStep({ id, kind, ...extra });

function input(name: string, blueprint: string, trigger: WorkflowTriggerKind, steps: WorkflowStep[], extra: Partial<WorkflowInput> = {}): WorkflowInput {
  return { name, handle: '', description: '', tags: [], ownerTeam: '', service: '', blueprint, spec: { inputs: [], triggers: [emptyTrigger(trigger)], steps }, ...extra };
}

export const BLUEPRINTS: WorkflowBlueprint[] = [
  {
    key: 'restart_on_alert',
    title: 'Restart a service when its error rate alerts',
    description: 'Check the error rate again, restart the service through your deploy pipeline when it is still high, and tell the team on Slack.',
    category: 'Remediation',
    logos: ['monitor', 'kubernetes', 'slack'],
    trigger: 'monitor',
    keywords: ['restart', 'error', 'alert', 'monitor', 'remediate', 'pipeline', 'rollback'],
    build: () =>
      input('Restart a service when its error rate alerts', 'restart_on_alert', 'monitor', [
        step('check_errors', 'metric_query', {
          name: 'Check if monitor still in alert state',
          fields: { query: 'sum:trace.http.request.errors{service:order-service}.as_rate()', minutes: '5', aggregation: 'avg' },
        }),
        step('still_alerting', 'if', {
          name: 'Still above threshold?',
          expression: 'Steps.check_errors.output.value > 5',
          steps: [
            step('restart_service', 'http', {
              name: 'Run service restart pipeline',
              requests: [{ method: 'POST', url: 'https://deploy.example.com/api/v1/services/order-service/restart', headers: [], body: '{"reason": "{{ Trigger.monitor.name }}"}' }],
            }),
            step('tell_team', 'notify', { name: 'Tell the team', fields: { recipients: '@slack-checkout', message: '{{ Trigger.monitor.name }}: order-service was restarted.' } }),
          ],
        }),
      ]),
  },
  {
    key: 'daily_error_report',
    title: 'Daily report of the top services by errors',
    description: 'Every weekday morning, count the error logs of the last day per service and post the top three to a channel.',
    category: 'Reporting',
    logos: ['schedule', 'slack'],
    trigger: 'schedule',
    keywords: ['daily', 'report', 'top', 'errors', 'schedule', 'every day', 'morning', 'digest'],
    build: () =>
      input('Daily top errors report', 'daily_error_report', 'schedule', [
        step('top_errors', 'log_count', { name: 'Count error logs per service', fields: { query: 'status:error', minutes: '1440', group_by: 'service' } }),
        step('post_report', 'notify', {
          name: 'Post the report',
          fields: {
            recipients: '@slack-checkout',
            title: 'Daily top errors',
            message:
              '1. {{ Steps.top_errors.output.groups.0.group }} ({{ Steps.top_errors.output.groups.0.count }})\n2. {{ Steps.top_errors.output.groups.1.group }} ({{ Steps.top_errors.output.groups.1.count }})\n3. {{ Steps.top_errors.output.groups.2.group }} ({{ Steps.top_errors.output.groups.2.count }})',
          },
        }),
      ]),
  },
  {
    key: 'page_on_slo_burn',
    title: 'Page on-call when an SLO burns',
    description: 'When a burn-rate monitor warns or alerts, page the owning team, and declare an incident when it burns fast.',
    category: 'Incident response',
    logos: ['monitor', 'pagerduty', 'internal'],
    trigger: 'monitor',
    keywords: ['page', 'on-call', 'oncall', 'slo', 'burn', 'budget', 'pagerduty'],
    build: () => {
      const w = input('Page on-call when an SLO burns', 'page_on_slo_burn', 'monitor', [
        step('page_team', 'page_on_call', { name: 'Page the owning team', fields: { team: 'payments', title: 'SLO burning: {{ Trigger.monitor.name }}', urgency: 'high' } }),
        step('fast_burn', 'if', {
          name: 'Burning fast?',
          expression: "Trigger.transition.to == 'alert'",
          steps: [step('declare', 'create_incident', { name: 'Declare an incident', fields: { title: 'Error budget burning: {{ Trigger.monitor.name }}', severity: 'SEV-2', services: 'billing' } })],
        }),
      ]);
      w.spec.triggers[0].transitions = ['alert', 'warn'];
      return w;
    },
  },
  {
    key: 'scale_from_app',
    title: 'Scale a service from an app',
    description: 'A form for an app button or a manual run: pick a service, an environment and a number of replicas.',
    category: 'Remediation',
    logos: ['kubernetes', 'internal'],
    trigger: 'app',
    keywords: ['scale', 'replicas', 'app', 'button', 'form', 'deployment'],
    build: () => {
      const w = input('Scale a service', 'scale_from_app', 'app', [
        step('scale', 'http', {
          name: 'Scale the deployment',
          requests: [
            {
              method: 'POST',
              url: 'https://deploy.example.com/api/v1/services/{{inputs.service}}/scale',
              headers: [],
              body: '{"environment": "{{inputs.environment}}", "replicas": "{{inputs.replicas}}"}',
            },
          ],
        }),
      ]);
      w.spec.inputs = [
        { key: 'service', label: 'Service', kind: 'select', required: true, options: ['order-service', 'catalog-service', 'inventory-service'], default: 'order-service', description: '' },
        { key: 'environment', label: 'Environment', kind: 'select', required: true, options: ['staging', 'production'], default: 'staging', description: '' },
        { key: 'replicas', label: 'Replicas', kind: 'number', required: true, options: [], default: '3', description: '' },
      ];
      w.spec.triggers.push(emptyTrigger('manual'));
      return w;
    },
  },
  {
    key: 'incident_from_monitor',
    title: 'Declare an incident from a critical monitor',
    description: 'Open an incident with the monitor’s details, add the alert message to its timeline and notify the incident channel.',
    category: 'Incident response',
    logos: ['monitor', 'internal', 'teams'],
    trigger: 'monitor',
    keywords: ['incident', 'declare', 'critical', 'monitor', 'sev'],
    build: () =>
      input('Declare an incident from a critical monitor', 'incident_from_monitor', 'monitor', [
        step('declare', 'create_incident', { name: 'Declare the incident', fields: { title: '{{ Trigger.monitor.name }}', severity: 'SEV-2', summary: '{{ Trigger.message }}' } }),
        step('note', 'incident_comment', {
          name: 'Add the alert to the timeline',
          fields: { incident: '{{ Steps.declare.output.incident }}', body: 'Triggered on {{ Trigger.group }} with value {{ Trigger.value }}.' },
        }),
        step('announce', 'notify', { name: 'Announce it', fields: { recipients: '@slack-incidents', message: 'IR-{{ Steps.declare.output.incident }} declared: {{ Trigger.monitor.name }}' } }),
      ]),
  },
  {
    key: 'weekly_pr_digest',
    title: 'Weekly digest of stale pull requests',
    description: 'Every Monday, list the pull requests open for more than two weeks and post them to the team channel.',
    category: 'Developer workflows',
    logos: ['github', 'slack'],
    trigger: 'schedule',
    keywords: ['pull request', 'pr', 'prs', 'github', 'stale', 'weekly', 'digest', 'review'],
    build: () => {
      const w = input('Weekly stale pull request digest', 'weekly_pr_digest', 'schedule', [
        step('open_prs', 'http', {
          name: 'List open pull requests',
          requests: [{ method: 'GET', url: 'https://api.github.com/repos/example-org/storefront/pulls?state=open&sort=created', headers: [], body: '' }],
        }),
        step('post_digest', 'notify', {
          name: 'Post the digest',
          fields: { recipients: '@slack-storefront', title: 'Stale pull requests', message: 'Oldest open pull request: {{ Steps.open_prs.output.body.0.title }}' },
        }),
      ]);
      w.spec.triggers[0].cron = '0 9 * * 1';
      return w;
    },
  },
  {
    key: 'synthetic_after_deploy',
    title: 'Check the storefront after a deploy',
    description: 'Run the checkout synthetic test from a pipeline (API trigger), wait for caches to warm, and tell the release channel.',
    category: 'Developer workflows',
    logos: ['githubactions', 'internal', 'slack'],
    trigger: 'api',
    keywords: ['deploy', 'release', 'synthetic', 'test', 'api', 'pipeline', 'ci'],
    build: () =>
      input('Check the storefront after a deploy', 'synthetic_after_deploy', 'api', [
        step('warm_up', 'wait', { name: 'Let caches warm up', fields: { seconds: '60' } }),
        step('checkout_test', 'synthetic_run', { name: 'Run the checkout test', fields: { test: 'checkout-flow' } }),
        step('tell_release', 'notify', { name: 'Tell #releases', fields: { recipients: '@slack-releases', message: 'Checkout test passed after the deploy.' } }),
      ]),
  },
  {
    key: 'incident_followup',
    title: 'Notify stakeholders when an incident is resolved',
    description: 'When a SEV-1 or SEV-2 incident is resolved, send its summary to the stakeholders by email and Slack.',
    category: 'Incident response',
    logos: ['incident', 'slack'],
    trigger: 'incident',
    keywords: ['incident', 'resolved', 'stakeholders', 'email', 'postmortem', 'notify'],
    build: () => {
      const w = input('Notify stakeholders when an incident is resolved', 'incident_followup', 'incident', [
        step('tell_stakeholders', 'notify', {
          name: 'Tell the stakeholders',
          fields: { recipients: '@slack-incidents', title: 'Resolved: IR-{{ Trigger.incident.id }}', message: '{{ Trigger.incident.title }} is resolved. {{ Trigger.url }}' },
        }),
      ]);
      w.spec.triggers[0].incidentEvents = ['resolved'];
      w.spec.triggers[0].severities = ['SEV-1', 'SEV-2'];
      return w;
    },
  },
];

export const blueprintByKey = (key: string | null | undefined) => BLUEPRINTS.find((b) => b.key === key);

/** Blueprints whose keywords match a free-text request, best first. */
export function matchBlueprints(text: string): WorkflowBlueprint[] {
  const t = text.toLowerCase();
  if (!t.trim()) return [];
  return BLUEPRINTS.map((b) => ({ b, score: b.keywords.reduce((n, k) => n + (t.includes(k) ? k.length : 0), 0) }))
    .filter((x) => x.score > 0)
    .sort((a, c) => c.score - a.score)
    .map((x) => x.b);
}
