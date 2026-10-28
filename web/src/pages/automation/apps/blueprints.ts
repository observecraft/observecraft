// App Builder blueprints: ready-made apps to start from (client-side templates).
import type { AppComponent, AppSpec } from '../../../api/apps/types';
import { blankComponent } from './appLogic';

export interface AppBlueprint {
  id: string;
  title: string;
  description: string;
  /** Short labels of what the app contains. */
  features: string[];
  /** Its buttons run a workflow with an app trigger. */
  needsWorkflow: boolean;
  tags: string[];
  /** The components; `workflowId` is filled when the app is created. */
  spec: (workflowId: string) => AppSpec;
}

const c = (id: string, kind: AppComponent['kind'], extra: Partial<AppComponent> = {}): AppComponent => ({ ...blankComponent(id, kind), ...extra });

export const APP_BLUEPRINTS: AppBlueprint[] = [
  {
    id: 'service_operations',
    title: 'Service operations',
    description: 'Scale a storefront service from a form, see its pods after the run and keep an eye on its traffic.',
    features: ['Form', 'Workflow button', 'Run results table', 'Metric values'],
    needsWorkflow: true,
    tags: ['team:platform'],
    spec: (workflowId) => ({
      components: [
        c('title', 'heading', { text: 'Service operations' }),
        c('intro', 'text', { text: 'Pick a service and an environment, then scale it.' }),
        c('service', 'select', { label: 'Service', options: ['order-service', 'catalog-service', 'inventory-service', 'storefront-api'], defaultValue: 'order-service', width: 4 }),
        c('environment', 'select', { label: 'Environment', options: ['staging', 'production'], defaultValue: 'staging', width: 4 }),
        c('replicas', 'number_input', { label: 'Replicas', defaultValue: '3', width: 4 }),
        c('scale', 'button', {
          label: 'Scale service',
          workflowId,
          inputs: { service: '{{ components.service.value }}', environment: '{{ components.environment.value }}', replicas: '{{ components.replicas.value }}' },
          width: 3,
        }),
        c('pods', 'table', { label: 'Running pods', source: 'scale', path: 'Steps.pods.output.groups', columns: ['group', 'value'] }),
        c('rps', 'metric', { label: 'storefront-api requests/s', query: 'sum:trace.http.request.hits{service:storefront-api}', aggregation: 'avg', unit: 'req/s', width: 6 }),
        c('errors', 'metric', { label: 'order-service errors/s', query: 'sum:trace.http.request.errors{service:order-service}', aggregation: 'avg', unit: 'err/s', width: 6 }),
      ],
    }),
  },
  {
    id: 'checkout_health',
    title: 'Checkout health board',
    description: 'The checkout numbers an on-call engineer looks at first: orders, latency and payment errors.',
    features: ['Metric values', 'Notes'],
    needsWorkflow: false,
    tags: ['team:checkout'],
    spec: () => ({
      components: [
        c('title', 'heading', { text: 'Checkout health' }),
        c('orders', 'metric', { label: 'Orders/min', query: 'sum:trace.http.request.hits{service:order-service}', aggregation: 'sum', width: 4 }),
        c('latency', 'metric', { label: 'p95 latency', query: 'p95:trace.http.request.duration{service:order-service}', aggregation: 'avg', unit: 's', width: 4 }),
        c('billing_errors', 'metric', { label: 'billing errors/s', query: 'sum:trace.http.request.errors{service:billing}', aggregation: 'avg', unit: 'err/s', width: 4 }),
        c('runbook', 'text', { text: 'If billing errors climb above 1/s, page @payments and check the latest billing deploy.' }),
      ],
    }),
  },
  {
    id: 'incident_quick_declare',
    title: 'Incident quick declare',
    description: 'A short form that runs an incident workflow: title, severity and the affected service.',
    features: ['Form', 'Workflow button', 'Run results table'],
    needsWorkflow: true,
    tags: ['incidents'],
    spec: (workflowId) => ({
      components: [
        c('title', 'heading', { text: 'Declare an incident' }),
        c('summary', 'text_input', { label: 'What is broken?', placeholder: 'Checkout payments fail for card users', width: 6 }),
        c('severity', 'select', { label: 'Severity', options: ['SEV-1', 'SEV-2', 'SEV-3', 'SEV-4'], defaultValue: 'SEV-3', width: 3 }),
        c('service', 'select', { label: 'Service', options: ['billing', 'order-service', 'storefront-api', 'catalog-service'], defaultValue: 'billing', width: 3 }),
        c('declare', 'button', {
          label: 'Declare',
          workflowId,
          inputs: { title: '{{ components.summary.value }}', severity: '{{ components.severity.value }}', service: '{{ components.service.value }}' },
          width: 3,
        }),
        c('result', 'table', { label: 'Result', source: 'declare', path: 'Steps.declare.output' }),
      ],
    }),
  },
  {
    id: 'error_triage',
    title: 'Error triage',
    description: 'Count error logs per service for a time window and list the noisiest ones.',
    features: ['Form', 'Workflow button', 'Run results table'],
    needsWorkflow: true,
    tags: ['team:checkout', 'logs'],
    spec: (workflowId) => ({
      components: [
        c('title', 'heading', { text: 'Error triage' }),
        c('minutes', 'number_input', { label: 'Look back (minutes)', defaultValue: '60', width: 4 }),
        c('count', 'button', { label: 'Count errors', workflowId, inputs: { minutes: '{{ components.minutes.value }}' }, width: 3 }),
        c('top', 'table', { label: 'Error logs by service', source: 'count', path: 'Steps.top_errors.output.groups', columns: ['group', 'count'] }),
      ],
    }),
  },
];

export const appBlueprint = (id: string) => APP_BLUEPRINTS.find((b) => b.id === id);
