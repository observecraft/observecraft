// In-browser App Builder for mock mode. State lives in memory for the session; a button runs its
// workflow through the workflows mock (mock/workflows.ts), like the server does.
import type { JsonValue } from '../types';
import type { AppComponent, AppValues, AutomationApp, AutomationAppInput, AutomationAppRun } from '../apps/types';
import { DEMO_IDS, startFromApp } from './workflows';

const ME = 'Alex Morgan';
const DAY = 24 * 3_600_000;

let apps: AutomationApp[] | null = null;

const delay = <T>(v: T): Promise<T> => new Promise((r) => setTimeout(() => r(structuredClone(v)), 120));
const fail = (msg: string): never => {
  throw new Error(msg);
};
const uuid = () => (globalThis.crypto?.randomUUID ? globalThis.crypto.randomUUID() : `00000000-0000-4000-8000-${String(Date.now()).padStart(12, '0').slice(-12)}`);

/** A component with the defaults filled in. */
export function component(id: string, kind: AppComponent['kind'], extra: Partial<AppComponent> = {}): AppComponent {
  return {
    id,
    kind,
    label: '',
    text: '',
    defaultValue: '',
    placeholder: '',
    options: [],
    workflowId: '',
    inputs: {},
    source: '',
    path: '',
    columns: [],
    query: '',
    aggregation: '',
    unit: '',
    width: 12,
    ...extra,
  };
}

function seed(): AutomationApp[] {
  if (apps) return apps;
  const now = Date.now();
  apps = [
    {
      id: '9b2d4f60-1a2b-4c3d-8e9f-0a1b2c3d4e01',
      name: 'Service operations',
      description: 'Scale storefront services and check their pods without leaving ObserveCraft.',
      tags: ['team:platform'],
      blueprint: 'service_operations',
      published: true,
      createdBy: 'Alex Morgan',
      createdAt: now - 6 * DAY,
      updatedAt: now - 2 * DAY,
      spec: {
        components: [
          component('title', 'heading', { text: 'Service operations' }),
          component('intro', 'text', { text: 'Pick a service and an environment, then scale it. The run acts as the workflow’s publisher.' }),
          component('service', 'select', { label: 'Service', options: ['order-service', 'catalog-service', 'inventory-service', 'storefront-api'], defaultValue: 'order-service', width: 4 }),
          component('environment', 'select', { label: 'Environment', options: ['staging', 'production'], defaultValue: 'staging', width: 4 }),
          component('replicas', 'number_input', { label: 'Replicas', defaultValue: '3', width: 4 }),
          component('scale', 'button', {
            label: 'Scale service',
            workflowId: DEMO_IDS.scale,
            inputs: { service: '{{ components.service.value }}', environment: '{{ components.environment.value }}', replicas: '{{ components.replicas.value }}' },
            width: 3,
          }),
          component('pods', 'table', { label: 'Running pods', source: 'scale', path: 'Steps.pods.output.groups', columns: ['group', 'value'], width: 12 }),
          component('rps', 'metric', { label: 'storefront-api requests/s', query: 'sum:trace.http.request.hits{service:storefront-api}', aggregation: 'avg', unit: 'req/s', width: 6 }),
          component('errors', 'metric', { label: 'order-service errors/s', query: 'sum:trace.http.request.errors{service:order-service}', aggregation: 'avg', unit: 'err/s', width: 6 }),
        ],
      },
    },
    {
      id: '9b2d4f60-1a2b-4c3d-8e9f-0a1b2c3d4e02',
      name: 'Checkout health board',
      description: 'Key checkout numbers for the on-call engineer.',
      tags: ['team:checkout'],
      blueprint: '',
      published: false,
      createdBy: 'Priya Natarajan',
      createdAt: now - 3 * DAY,
      updatedAt: now - 1 * DAY,
      spec: {
        components: [
          component('title', 'heading', { text: 'Checkout health' }),
          component('orders', 'metric', { label: 'Orders/min', query: 'sum:trace.http.request.hits{service:order-service}', aggregation: 'sum', unit: '', width: 4 }),
          component('latency', 'metric', { label: 'p95 latency', query: 'p95:trace.http.request.duration{service:order-service}', aggregation: 'avg', unit: 's', width: 4 }),
          component('billing_errors', 'metric', { label: 'billing errors/s', query: 'sum:trace.http.request.errors{service:billing}', aggregation: 'avg', unit: 'err/s', width: 4 }),
        ],
      },
    },
  ];
  return apps;
}

function find(id: string): AutomationApp {
  return seed().find((a) => a.id === id) ?? fail('App not found');
}

function check(input: AutomationAppInput, current: AutomationApp | null): AutomationApp {
  const name = input.name.trim();
  if (!name) fail('The name is required');
  if (seed().some((a) => a.name === name && a.id !== current?.id)) fail(`An app named ${name} exists already`);
  const ids = new Set<string>();
  for (const c of input.spec.components) {
    if (!/^[a-z0-9_]{1,40}$/.test(c.id)) fail(`${c.id} isn't a component id (a–z, 0–9 and _, up to 40)`);
    if (ids.has(c.id)) fail(`The component id ${c.id} is used twice`);
    ids.add(c.id);
  }
  for (const c of input.spec.components) {
    if (c.kind === 'button' && !c.workflowId) fail(`Component ${c.id}: choose the workflow the button runs`);
    if (c.kind === 'table' && !input.spec.components.some((b) => b.kind === 'button' && b.id === c.source)) fail(`Component ${c.id}: a table shows the latest run of one of the app's buttons`);
    if (c.kind === 'select' && c.options.filter(Boolean).length === 0) fail(`Component ${c.id}: a select needs 1–50 distinct options`);
    if ((c.kind === 'heading' || c.kind === 'text') && !c.text.trim()) fail(`Component ${c.id}: The text is required`);
    if (c.kind === 'metric' && !c.query.trim()) fail(`Component ${c.id}: The query is required`);
  }
  const now = Date.now();
  return {
    id: current?.id ?? uuid(),
    name,
    description: input.description?.trim() ?? '',
    tags: input.tags ?? [],
    blueprint: input.blueprint ?? '',
    published: input.published ?? false,
    spec: input.spec,
    createdBy: current?.createdBy ?? ME,
    createdAt: current?.createdAt ?? now,
    updatedAt: now,
  };
}

export function listApps(): Promise<AutomationApp[]> {
  return delay([...seed()].sort((a, b) => a.name.localeCompare(b.name)));
}

export function getApp(id: string): Promise<AutomationApp> {
  return delay(find(id));
}

export function createApp(input: AutomationAppInput): Promise<AutomationApp> {
  const a = check(input, null);
  seed().push(a);
  return delay(a);
}

export function updateApp(id: string, input: AutomationAppInput): Promise<AutomationApp> {
  const a = check(input, find(id));
  apps = seed().map((x) => (x.id === id ? a : x));
  return delay(a);
}

export function deleteApp(id: string): Promise<void> {
  find(id);
  apps = seed().filter((a) => a.id !== id);
  return delay(undefined);
}

/** The workflow inputs a button sends: its templates filled with the app's values. */
export function buttonInputs(button: AppComponent, values: AppValues): Record<string, JsonValue> {
  const out: Record<string, JsonValue> = {};
  for (const [key, tpl] of Object.entries(button.inputs)) {
    const single = /^\{\{\s*components\.([a-z0-9_]+)\.value\s*\}\}$/.exec(tpl.trim());
    const value: JsonValue = single ? (values[single[1]] ?? null) : tpl.replace(/\{\{\s*components\.([a-z0-9_]+)\.value\s*\}\}/g, (_, id: string) => String(values[id] ?? '')).trim();
    if (value !== null && value !== '') out[key] = value;
  }
  return out;
}

export function runApp(id: string, componentId: string, values: AppValues): Promise<AutomationAppRun> {
  const app = find(id);
  const button = app.spec.components.find((c) => c.id === componentId && c.kind === 'button') ?? fail(`The app has no button ${componentId}`);
  const runId = startFromApp(button.workflowId, app.name, buttonInputs(button, values));
  return delay({ workflowId: button.workflowId, runId });
}
