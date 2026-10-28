// In-browser Workflow Automation for mock mode. State lives in memory for the session. Runs follow
// the server's state machine (workflows/logic.ts `simulate`) and answer each action step with a
// fictional result instead of calling HTTP endpoints, notification channels or the metric store; a
// run shows as running for a moment, its steps appearing one after the other.
import type { JsonValue } from '../types';
import { checkRunInputs } from '../portalActions/logic';
import { normalizeStep, renderTemplate, simulate, specError, type SimStep } from '../workflows/logic';
import type { Workflow, WorkflowInput, WorkflowRun, WorkflowRunStep, WorkflowSpec, WorkflowStep, WorkflowTrigger, WorkflowTriggerKind, WorkflowValues } from '../workflows/types';

const ME = { name: 'Alex Morgan', email: 'alex.morgan@example.com' };
const HOUR = 3_600_000;
const DAY = 24 * HOUR;
const STEP_MS = 450;

type Stored = Omit<Workflow, 'runs' | 'lastRun' | 'secretNames' | 'published' | 'unpublishedChanges'> & { secrets: Record<string, string> };
type StoredRun = WorkflowRun & { revealAt: number[] };

let workflows: Stored[] | null = null;
let runs: StoredRun[] = [];
let nextRun = 1;
let nextIncident = 57;

const delay = <T>(v: T): Promise<T> => new Promise((r) => setTimeout(() => r(structuredClone(v)), 120));
const fail = (msg: string): never => {
  throw new Error(msg);
};
const uuid = () => (globalThis.crypto?.randomUUID ? globalThis.crypto.randomUUID() : `00000000-0000-4000-8000-${String(Date.now()).padStart(12, '0').slice(-12)}`);

const trigger = (kind: WorkflowTriggerKind, extra: Partial<WorkflowTrigger> = {}): WorkflowTrigger => ({ kind, cron: '', timezone: '', transitions: [], incidentEvents: [], severities: [], ...extra });
const step = (id: string, kind: WorkflowStep['kind'], extra: Partial<WorkflowStep> = {}): WorkflowStep => normalizeStep({ id, kind, ...extra });

export const DEMO_IDS = {
  restart: '7a3e1c20-5b6d-4e7f-9a1b-2c3d4e5f6a01',
  report: '7a3e1c20-5b6d-4e7f-9a1b-2c3d4e5f6a02',
  burn: '7a3e1c20-5b6d-4e7f-9a1b-2c3d4e5f6a03',
  scale: '7a3e1c20-5b6d-4e7f-9a1b-2c3d4e5f6a04',
  digest: '7a3e1c20-5b6d-4e7f-9a1b-2c3d4e5f6a05',
};

function seed(): Stored[] {
  if (workflows) return workflows;
  const now = Date.now();
  const restart: WorkflowSpec = {
    inputs: [],
    triggers: [trigger('monitor', { transitions: ['alert'] })],
    steps: [
      step('check_errors', 'metric_query', {
        name: 'Check if monitor still in alert state',
        fields: { query: 'sum:trace.http.request.errors{service:order-service,env:prod}.as_rate()', minutes: '5', aggregation: 'avg' },
      }),
      step('still_alerting', 'if', {
        name: 'Still above threshold?',
        expression: 'Steps.check_errors.output.value > 5',
        steps: [
          step('restart_service', 'http', {
            name: 'Run service restart pipeline',
            requests: [
              {
                method: 'POST',
                url: 'https://deploy.example.com/api/v1/services/order-service/restart',
                headers: [{ name: 'Authorization', value: 'Bearer {{secrets.deploy_token}}' }],
                body: '{"reason": "{{ Trigger.monitor.name }}"}',
              },
            ],
            retries: 1,
          }),
          step('wait_recovery', 'wait', { name: 'Wait for pods to settle', fields: { seconds: '120' } }),
          step('tell_checkout', 'notify', {
            name: 'Tell #checkout',
            fields: {
              recipients: '@slack-checkout',
              title: 'order-service restarted',
              message: '{{ Trigger.monitor.name }} fired; error rate was {{ Steps.check_errors.output.value }}/s, so order-service was restarted.',
            },
          }),
        ],
        elseSteps: [
          step('note_recovered', 'notify', {
            name: 'Note: recovered on its own',
            fields: { recipients: '@slack-checkout', message: '{{ Trigger.monitor.name }} recovered before a restart was needed.' },
          }),
        ],
      }),
    ],
  };
  const report: WorkflowSpec = {
    inputs: [],
    triggers: [trigger('schedule', { cron: '0 9 * * 1-5', timezone: 'Europe/Paris' })],
    steps: [
      step('top_errors', 'log_count', { name: 'Top 3 services by error logs', fields: { query: 'status:error env:prod', minutes: '1440', group_by: 'service' } }),
      step('post_report', 'notify', {
        name: 'Post the daily report',
        fields: {
          recipients: '@slack-checkout',
          title: 'Daily top errors',
          message:
            '1. {{ Steps.top_errors.output.groups.0.group }}: {{ Steps.top_errors.output.groups.0.count }}\n2. {{ Steps.top_errors.output.groups.1.group }}: {{ Steps.top_errors.output.groups.1.count }}\n3. {{ Steps.top_errors.output.groups.2.group }}: {{ Steps.top_errors.output.groups.2.count }}',
        },
      }),
    ],
  };
  const burn: WorkflowSpec = {
    inputs: [],
    triggers: [trigger('monitor', { transitions: ['alert', 'warn'] }), trigger('manual')],
    steps: [
      step('page_payments', 'page_on_call', { name: 'Page payments on-call', fields: { team: 'payments', title: 'billing SLO burn: {{ Trigger.monitor.name }}', urgency: 'high' } }),
      step('is_alert', 'if', {
        name: 'Burning fast?',
        expression: "Trigger.transition.to == 'alert'",
        steps: [step('declare', 'create_incident', { name: 'Declare SEV-2', fields: { title: 'billing error budget burning', severity: 'SEV-2', services: 'billing', teams: 'payments' } })],
      }),
    ],
  };
  const scale: WorkflowSpec = {
    inputs: [
      {
        key: 'service',
        label: 'Service',
        kind: 'select',
        required: true,
        options: ['order-service', 'catalog-service', 'inventory-service', 'storefront-api'],
        default: 'order-service',
        description: '',
      },
      { key: 'environment', label: 'Environment', kind: 'select', required: true, options: ['staging', 'production'], default: 'staging', description: '' },
      { key: 'replicas', label: 'Replicas', kind: 'number', required: true, options: [], default: '3', description: '' },
    ],
    triggers: [trigger('app'), trigger('manual'), trigger('api')],
    steps: [
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
      step('pods', 'metric_query', {
        name: 'Read running pods',
        fields: { query: 'sum:kubernetes.pods.running{kube_deployment:{{inputs.service}}} by {kube_namespace}', minutes: '5', aggregation: 'last' },
      }),
    ],
  };
  const digest: WorkflowSpec = {
    inputs: [],
    triggers: [trigger('schedule', { cron: '0 8 * * 1', timezone: 'UTC' })],
    steps: [
      step('open_prs', 'http', { name: 'List open pull requests', requests: [{ method: 'GET', url: 'https://api.github.com/repos/example-org/storefront/pulls?state=open', headers: [], body: '' }] }),
    ],
  };
  const base = { handle: '', description: '', tags: [] as string[], ownerTeam: '', service: '', blueprint: '', hasApiToken: false, nextScheduleAt: null, secrets: {} as Record<string, string> };
  workflows = [
    {
      ...base,
      id: DEMO_IDS.restart,
      name: 'Restart order-service when error rate alerts',
      handle: 'restart-order-service',
      description: 'Checks the error rate again, restarts order-service through the deploy pipeline when it is still high, and tells #checkout.',
      tags: ['team:checkout', 'service:order-service'],
      ownerTeam: 'checkout',
      service: 'order-service',
      blueprint: 'restart_on_alert',
      spec: restart,
      publishedSpec: restart,
      publishedAt: now - 2 * DAY,
      publishedBy: 'Priya Natarajan',
      secrets: { deploy_token: 'mock-deploy-token-0001' },
      createdBy: 'Priya Natarajan',
      createdAt: now - 21 * DAY,
      updatedAt: now - 2 * DAY,
    },
    {
      ...base,
      id: DEMO_IDS.report,
      name: 'Daily top errors report to #checkout',
      handle: 'daily-top-errors',
      description: 'Every weekday at 09:00 Paris time, posts the three services with the most error logs.',
      tags: ['team:checkout', 'report'],
      ownerTeam: 'checkout',
      blueprint: 'daily_error_report',
      spec: report,
      publishedSpec: report,
      publishedAt: now - 9 * DAY,
      publishedBy: 'Alex Morgan',
      nextScheduleAt: now + 14 * HOUR,
      createdBy: 'Alex Morgan',
      createdAt: now - 30 * DAY,
      updatedAt: now - 9 * DAY,
    },
    {
      ...base,
      id: DEMO_IDS.burn,
      name: 'Page payments on-call when billing SLO burns',
      handle: 'billing-slo-burn',
      description: 'Pages @payments when the billing burn-rate monitor warns or alerts, and declares a SEV-2 on a fast burn.',
      tags: ['team:payments', 'service:billing', 'slo'],
      ownerTeam: 'payments',
      service: 'billing',
      spec: burn,
      publishedSpec: burn,
      publishedAt: now - 5 * DAY,
      publishedBy: 'Sam Rivera',
      createdBy: 'Sam Rivera',
      createdAt: now - 12 * DAY,
      updatedAt: now - 5 * DAY,
    },
    {
      ...base,
      id: DEMO_IDS.scale,
      name: 'Scale a storefront service',
      handle: 'scale-storefront-service',
      description: 'Scales a deployment of the storefront to a number of replicas; used by the Service operations app.',
      tags: ['team:platform'],
      ownerTeam: 'platform',
      spec: scale,
      publishedSpec: scale,
      publishedAt: now - 3 * DAY,
      publishedBy: 'Alex Morgan',
      hasApiToken: true,
      createdBy: 'Alex Morgan',
      createdAt: now - 8 * DAY,
      updatedAt: now - 3 * DAY,
    },
    {
      ...base,
      id: DEMO_IDS.digest,
      name: 'Weekly stale pull request digest',
      handle: 'weekly-stale-prs',
      description: 'Draft: lists pull requests open for more than two weeks.',
      tags: ['team:platform'],
      ownerTeam: 'platform',
      spec: digest,
      publishedSpec: null,
      publishedAt: null,
      publishedBy: null,
      createdBy: 'Alex Morgan',
      createdAt: now - 1 * DAY,
      updatedAt: now - 1 * DAY,
    },
  ];
  // A little history.
  const past: [string, WorkflowTriggerKind, string, number, Record<string, JsonValue>][] = [
    [DEMO_IDS.restart, 'monitor', 'Monitor #12', 3 * HOUR, monitorTrigger('alert', 9.4)],
    [DEMO_IDS.restart, 'monitor', 'Monitor #12', 2 * DAY, monitorTrigger('alert', 3.1)],
    [DEMO_IDS.report, 'schedule', 'Schedule', 10 * HOUR, { kind: 'schedule' }],
    [DEMO_IDS.report, 'schedule', 'Schedule', 34 * HOUR, { kind: 'schedule' }],
    [DEMO_IDS.burn, 'monitor', 'Monitor #31', 26 * HOUR, monitorTrigger('warn', 2.2, 'Billing SLO burn rate')],
    [DEMO_IDS.scale, 'app', 'Alex Morgan (app)', 5 * HOUR, { kind: 'app', app: 'Service operations' }],
  ];
  for (const [id, kind, by, ago, t] of past) {
    const w = workflows.find((x) => x.id === id)!;
    const inputs: Record<string, JsonValue> = id === DEMO_IDS.scale ? { service: 'order-service', environment: 'staging', replicas: 4 } : {};
    startRun(w, w.publishedSpec ?? w.spec, kind, by, t, inputs, now - ago, true);
  }
  return workflows;
}

function monitorTrigger(to: string, value: number, name = 'High error rate on order-service'): Record<string, JsonValue> {
  return {
    kind: 'monitor',
    monitor: { id: 12, name, priority: 2, tags: ['service:order-service', 'env:prod'], url: '/monitors/12' },
    group: 'service:order-service',
    transition: { from: 'ok', to },
    value,
    message: '@workflow-restart-order-service',
  };
}

function view(w: Stored): Workflow {
  const mine = runs.filter((r) => r.workflowId === w.id).sort((a, b) => b.createdAt - a.createdAt);
  const last = mine[0];
  const { secrets, ...rest } = w;
  return {
    ...rest,
    secretNames: Object.keys(secrets).sort(),
    published: w.publishedSpec !== null,
    unpublishedChanges: w.publishedSpec !== null && JSON.stringify(w.publishedSpec) !== JSON.stringify(w.spec),
    runs: mine.length,
    lastRun: last ? { id: last.id, status: statusOf(last), triggerKind: last.triggerKind, createdAt: last.createdAt } : null,
  };
}

function find(id: string): Stored {
  return seed().find((w) => w.id === id) ?? fail('Workflow not found');
}

// ---------------------------------------------------------------- fictional step results

type Performed = { ok: boolean; inputs: Record<string, JsonValue>; output: JsonValue; error?: string };

function perform(s: WorkflowStep, vars: JsonValue, inputs: Record<string, JsonValue>): Performed {
  const f = (name: string) => renderTemplate(s.fields[name] ?? '', vars).trim();
  const rendered: Record<string, JsonValue> = Object.fromEntries(Object.keys(s.fields).map((k) => [k, f(k)]));
  switch (s.kind) {
    case 'http': {
      const reqs = s.requests.map((r) => ({ method: r.method, url: renderTemplate(r.url, vars).replace(/\{\{[^}]*\}\}/g, '') }));
      const service = (inputs.service as string) || 'order-service';
      return {
        ok: true,
        inputs: { requests: reqs },
        output: { statusCode: 202, body: { status: 'accepted', service, pipeline: `run-${4800 + nextRun}`, replicas: inputs.replicas ?? null }, link: '' },
      };
    }
    case 'metric_query': {
      const q = f('query');
      const value = q.includes('errors') ? ((((vars as Record<string, JsonValue>).Trigger as Record<string, JsonValue>)?.value as number) ?? 7.2) : 4;
      const groups = q.includes(' by ')
        ? [
            { group: 'kube_namespace:storefront', value: Number(inputs.replicas ?? 3) },
            { group: 'kube_namespace:storefront-canary', value: 1 },
          ]
        : [{ group: '*', value }];
      return { ok: true, inputs: { query: q, minutes: Number(f('minutes') || 15), aggregation: f('aggregation') || 'avg' }, output: { value: groups[0].value, groups } };
    }
    case 'log_count': {
      const groups = [
        { group: 'order-service', count: 1284 },
        { group: 'billing', count: 731 },
        { group: 'catalog-service', count: 402 },
        { group: 'inventory-service', count: 118 },
      ];
      return {
        ok: true,
        inputs: { query: f('query'), minutes: Number(f('minutes') || 15), groupBy: f('group_by') },
        output: { count: groups.reduce((a, g) => a + g.count, 0), groups: f('group_by') ? groups : [] },
      };
    }
    case 'notify': {
      const recipients = f('recipients')
        .split(/[\s,]+/)
        .map((r) => r.replace(/^@/, ''))
        .filter(Boolean);
      return {
        ok: recipients.length > 0,
        inputs: { recipients, title: f('title'), message: f('message') },
        output: { sent: recipients, failed: [] },
        error: recipients.length ? undefined : 'Name 1–20 recipients',
      };
    }
    case 'create_incident': {
      const id = nextIncident++;
      return { ok: true, inputs: rendered, output: { incident: id, title: f('title'), severity: f('severity') || 'SEV-3', link: `/incidents/${id}` } };
    }
    case 'page_on_call':
      return { ok: true, inputs: rendered, output: { page: 300 + nextRun, team: f('team'), title: f('title'), link: '/on-call' } };
    case 'incident_comment':
      return { ok: true, inputs: rendered, output: { incident: Number(f('incident').replace(/\D/g, '')) || 1, link: '/incidents' } };
    case 'synthetic_run':
      return { ok: true, inputs: rendered, output: { test: f('test'), passed: true, link: '/synthetics' } };
    case 'wait':
      return { ok: true, inputs: { seconds: Number(f('seconds')) || 1 }, output: { seconds: Number(f('seconds')) || 1, until: Date.now() } };
    default:
      return { ok: true, inputs: {}, output: {} };
  }
}

function startRun(w: Stored, spec: WorkflowSpec, kind: WorkflowTriggerKind, by: string, t: Record<string, JsonValue>, inputs: Record<string, JsonValue>, at = Date.now(), done = false): StoredRun {
  const id = nextRun++;
  const base: Record<string, JsonValue> = { Trigger: t, inputs, Workflow: { id: w.id, name: w.name, runId: id }, user: { name: ME.name, email: ME.email } };
  const sim = simulate(spec, base, (s, vars) => perform(s, vars, inputs));
  let clock = at + 300;
  const revealAt: number[] = [];
  const steps: WorkflowRunStep[] = sim.steps.map((s: SimStep, i) => {
    const duration = s.kind === 'if' || s.kind === 'for_each' ? 0 : s.kind === 'wait' ? 1200 : 180 + ((i * 97) % 600);
    const started = clock;
    clock += (done ? duration : Math.min(duration, STEP_MS)) + 40;
    revealAt.push(clock);
    return {
      id: id * 1000 + i,
      stepId: s.stepId,
      kind: s.kind,
      name: s.name,
      iteration: s.iteration,
      status: s.ok ? 'succeeded' : 'failed',
      inputs: s.inputs,
      output: s.output,
      error: s.error,
      attempts: 1,
      startedAt: started,
      durationMs: duration,
    };
  });
  const run: StoredRun = {
    id,
    workflowId: w.id,
    workflowName: w.name,
    triggerKind: kind,
    triggeredBy: by,
    status: sim.ok ? 'succeeded' : 'failed',
    error: sim.error,
    trigger: t,
    inputs,
    createdAt: at,
    startedAt: at + 200,
    finishedAt: clock,
    nextAt: null,
    steps,
    spec,
    revealAt,
  };
  runs.push(run);
  return run;
}

function statusOf(r: StoredRun): string {
  if (r.status === 'canceled') return 'canceled';
  return Date.now() < (r.finishedAt ?? 0) ? 'running' : r.status;
}

/** The run as of now: running until its last step's time, steps appearing as they "finish". */
function snapshot(r: StoredRun, withSteps: boolean): WorkflowRun {
  const now = Date.now();
  const { revealAt, ...rest } = r;
  const running = statusOf(r) === 'running';
  const steps = withSteps ? r.steps.filter((_, i) => r.status === 'canceled' || revealAt[i] <= now) : [];
  return { ...rest, status: statusOf(r) as WorkflowRun['status'], error: running ? null : r.error, finishedAt: running ? null : r.finishedAt, steps, spec: withSteps ? r.spec : undefined };
}

// ---------------------------------------------------------------- API

export function listWorkflows(): Promise<Workflow[]> {
  return delay(
    seed()
      .map(view)
      .sort((a, b) => a.name.localeCompare(b.name)),
  );
}

export function getWorkflow(id: string): Promise<Workflow> {
  return delay(view(find(id)));
}

function check(input: WorkflowInput, current: Stored | null): Stored {
  const name = input.name.trim();
  if (!name) fail('The name is required');
  if (seed().some((w) => w.name === name && w.id !== current?.id)) fail(`A workflow named ${name} exists already`);
  const spec: WorkflowSpec = { inputs: input.spec.inputs, triggers: input.spec.triggers, steps: input.spec.steps.map(normalizeStep) };
  const problem = specError(spec);
  if (problem) fail(problem);
  const secrets = { ...(current?.secrets ?? {}) };
  for (const [k, v] of Object.entries(input.secrets ?? {})) {
    if (v === null) delete secrets[k];
    else secrets[k] = v;
  }
  const handle =
    input.handle?.trim() ||
    current?.handle ||
    name
      .toLowerCase()
      .replace(/[^a-z0-9]+/g, '-')
      .replace(/^-|-$/g, '')
      .slice(0, 60);
  const now = Date.now();
  return {
    id: current?.id ?? uuid(),
    name,
    handle,
    description: input.description?.trim() ?? '',
    tags: [...new Set((input.tags ?? []).map((t) => t.trim()).filter(Boolean))],
    ownerTeam: (input.ownerTeam ?? '').replace(/^@/, ''),
    service: input.service ?? '',
    blueprint: input.blueprint ?? '',
    spec,
    publishedSpec: current?.publishedSpec ?? null,
    publishedAt: current?.publishedAt ?? null,
    publishedBy: current?.publishedBy ?? null,
    secrets,
    hasApiToken: current?.hasApiToken ?? false,
    nextScheduleAt: current?.nextScheduleAt ?? null,
    createdBy: current?.createdBy ?? ME.name,
    createdAt: current?.createdAt ?? now,
    updatedAt: now,
  };
}

export function createWorkflow(input: WorkflowInput): Promise<Workflow> {
  const w = check(input, null);
  seed().push(w);
  return delay(view(w));
}

export function updateWorkflow(id: string, input: WorkflowInput): Promise<Workflow> {
  const current = find(id);
  const w = check(input, current);
  workflows = seed().map((x) => (x.id === id ? w : x));
  return delay(view(w));
}

export function deleteWorkflow(id: string): Promise<void> {
  find(id);
  workflows = seed().filter((w) => w.id !== id);
  runs = runs.filter((r) => r.workflowId !== id);
  return delay(undefined);
}

export function publishWorkflow(id: string): Promise<Workflow> {
  const w = find(id);
  if (w.spec.steps.length === 0) fail('Add a step before publishing');
  w.publishedSpec = structuredClone(w.spec);
  w.publishedAt = Date.now();
  w.publishedBy = ME.name;
  w.nextScheduleAt = w.spec.triggers.some((t) => t.kind === 'schedule') ? Date.now() + 6 * HOUR : null;
  return delay(view(w));
}

export function unpublishWorkflow(id: string): Promise<Workflow> {
  const w = find(id);
  w.publishedSpec = null;
  w.publishedAt = null;
  w.publishedBy = null;
  w.nextScheduleAt = null;
  return delay(view(w));
}

export function createToken(id: string): Promise<{ token: string }> {
  const w = find(id);
  w.hasApiToken = true;
  return delay({ token: `ocwt_mock${uuid().replace(/-/g, '').slice(0, 24)}` });
}

export function listRuns(id: string): Promise<WorkflowRun[]> {
  find(id);
  return delay(
    runs
      .filter((r) => r.workflowId === id)
      .sort((a, b) => b.createdAt - a.createdAt)
      .map((r) => snapshot(r, false)),
  );
}

export function getRun(id: string, run: number): Promise<WorkflowRun> {
  const r = runs.find((x) => x.workflowId === id && x.id === run) ?? fail('Run not found');
  return delay(snapshot(r, true));
}

export function runWorkflow(id: string, values: WorkflowValues): Promise<WorkflowRun> {
  const w = find(id);
  if (w.spec.steps.length === 0) fail('Add a step before running the workflow');
  const inputs = checkRunInputs(w.spec.inputs, values) as Record<string, JsonValue>;
  const r = startRun(w, w.spec, 'manual', ME.name, { kind: 'manual', user: { name: ME.name } }, inputs);
  return delay(snapshot(r, true));
}

/** An app's button (mock/apps.ts): runs the published workflow. */
export function startFromApp(id: string, app: string, values: Record<string, JsonValue>): number {
  const w = find(id);
  const spec = w.publishedSpec ?? fail('The workflow this button runs isn’t published');
  if (!spec.triggers.some((t) => t.kind === 'app')) fail('The workflow this button runs has no app trigger');
  const known = Object.fromEntries(Object.entries(values).filter(([k]) => spec.inputs.some((i) => i.key === k))) as WorkflowValues;
  const inputs = checkRunInputs(spec.inputs, known) as Record<string, JsonValue>;
  return startRun(w, spec, 'app', `${ME.name} (app)`, { kind: 'app', app, user: { name: ME.name } }, inputs).id;
}

export function cancelRun(id: string, run: number): Promise<WorkflowRun> {
  const r = runs.find((x) => x.workflowId === id && x.id === run) ?? fail('Run not found');
  if (statusOf(r) === 'running') {
    r.status = 'canceled';
    r.error = `Canceled by ${ME.name}`;
    const now = Date.now();
    r.steps = r.steps.filter((_, i) => r.revealAt[i] <= now);
    r.revealAt = r.revealAt.filter((t) => t <= now);
    r.finishedAt = now;
  }
  return delay(snapshot(r, true));
}
