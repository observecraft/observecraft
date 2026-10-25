// In-browser stand-in for the Dashboards endpoints (see api/dashboards/client.ts).
// The store persists to localStorage so dashboards created in the UI survive reloads —
// it plays the role of the dashboards table in the real backend.
import type {
  CreateDashboardRequest,
  Dashboard,
  DashboardList,
  DashboardSearchPage,
  DashboardSearchRequest,
  DashboardSummary,
  MonitorSummary,
  TemplateVariable,
  User,
  Widget,
  WidgetDefinition,
} from '../dashboards/types';
import { parseQuery } from '../../lib/searchQuery';
import { rng } from './generator';
import { matchesTerms } from './match';
import { monitorEntities } from './metricsEngine';

// Bump the version when seed definitions change so stale copies are replaced.
const STORAGE_KEY = 'oc.mock.dashboards.v5';
const DAY = 86400e3;
const delay = <T,>(v: T, ms = 80 + Math.random() * 140) => new Promise<T>((r) => setTimeout(() => r(structuredClone(v)), ms));

export const CURRENT_USER: User = { id: 'u-1', name: 'Alex Morgan' };
const AUTHORS: User[] = [CURRENT_USER, { id: 'u-2', name: 'Priya Shah' }, { id: 'u-3', name: 'Daniel Kim' }, { id: 'u-4', name: 'Sara Levi' }, { id: 'u-5', name: 'Omar Haddad' }];

// Seeds use a fixed PRNG so every reseed (another tab, hot reload) yields identical ids.
let idRand: () => number = Math.random;

function newId(): string {
  const c = 'abcdefghijkmnpqrstuvwxyz23456789';
  const part = (n: number) => Array.from({ length: n }, () => c[Math.floor(idRand() * c.length)]).join('');
  return `${part(3)}-${part(3)}-${part(3)}`;
}

let wseq = 0;
function W(definition: WidgetDefinition, x: number, y: number, w: number, h: number): Widget {
  return { id: `w${(++wseq).toString(36)}${Math.floor(idRand() * 1e6).toString(36)}`, definition, layout: { x, y, w, h } };
}

const ts = (title: string, queries: string[], display: 'line' | 'area' | 'bars' = 'line', extra: { stacked?: boolean } = {}): WidgetDefinition => ({
  type: 'timeseries',
  title,
  requests: queries.map((query) => ({ query })),
  display,
  ...extra,
});

const qv = (title: string, query: string, extra: Partial<Extract<WidgetDefinition, { type: 'query_value' }>> = {}): WidgetDefinition => ({
  type: 'query_value',
  title,
  request: { query },
  aggregator: 'last',
  precision: 0,
  formats: [],
  ...extra,
});

const top = (title: string, query: string, limit = 10, aggregator: 'avg' | 'sum' = 'avg'): WidgetDefinition => ({ type: 'toplist', title, request: { query }, aggregator, limit });

const note = (content: string, extra: Partial<Extract<WidgetDefinition, { type: 'note' }>> = {}): WidgetDefinition => ({
  type: 'note',
  content,
  background: 'white',
  align: 'left',
  fontSize: 'md',
  ...extra,
});

const tv = (name: string, prefix = name, def = '*'): TemplateVariable => ({ name, prefix, default: def });

// ---------------------------------------------------------------- seeded dashboards

function k8sDeployments(): Omit<Dashboard, keyof DashboardMeta> {
  const F = '{$kube_cluster_name,$kube_namespace,$kube_deployment}';
  const BY = `${F.slice(0, -1)}} by {kube_deployment}`;
  return {
    title: 'Kubernetes Deployments Overview',
    description: 'Scale, status and resource usage of deployment workloads.',
    kind: 'integration',
    logo: 'kubernetes',
    layoutType: 'grid',
    templateVariables: [tv('env'), tv('kube_cluster_name'), tv('kube_namespace'), tv('kube_deployment')],
    widgets: [
      W({ type: 'image', logo: 'kubernetes', background: 'linear-gradient(135deg,#1fb6ff,#0aa2e8 60%,#38c6ff)' }, 0, 0, 5, 7),
      W(
        note(
          '**Our Kubernetes Deployments dashboard gives you broad visibility into the scale, status, and resource usage of your deployment workloads.**\n\n---\n\nFurther reading for Kubernetes monitoring:\n- [Kubernetes deployments](https://kubernetes.io/docs/concepts/workloads/controllers/deployment/)\n- [kube-state-metrics](https://github.com/kubernetes/kube-state-metrics)\n- [OpenTelemetry k8s_cluster receiver](https://github.com/open-telemetry/opentelemetry-collector-contrib/tree/main/receiver/k8sclusterreceiver)\n- [Horizontal Pod Autoscaling](https://kubernetes.io/docs/tasks/run-application/horizontal-pod-autoscale/)',
          { fontSize: 'lg' },
        ),
        0,
        7,
        5,
        13,
      ),
      W(
        {
          type: 'group',
          title: 'Overview',
          color: 'blue',
          widgets: [
            W(qv('Deployments', `sum:kubernetes_state.deployment.count${F}`, { fill: true, formats: [{ comparator: '>=', value: 0, palette: 'green' }] }), 0, 0, 3, 4),
            W(qv('Replicas', `sum:kubernetes_state.deployment.replicas${F}`, { formats: [{ comparator: '>=', value: 0, palette: 'green' }] }), 3, 0, 3, 4),
            W(qv('Desired', `sum:kubernetes_state.deployment.replicas_desired${F}`, { formats: [{ comparator: '>=', value: 0, palette: 'green' }] }), 6, 0, 3, 4),
            W(qv('Available', `sum:kubernetes_state.deployment.replicas_available${F}`, { formats: [{ comparator: '>=', value: 0, palette: 'green' }] }), 9, 0, 3, 4),
            W(qv('Updated', `sum:kubernetes_state.deployment.replicas_updated${F}`, { formats: [{ comparator: '>=', value: 0, palette: 'green' }] }), 0, 4, 3, 4),
            W(qv('Unavailable', `sum:kubernetes_state.deployment.replicas_unavailable${F}`, { formats: [{ comparator: '>', value: 0, palette: 'red' }, { comparator: '<=', value: 0, palette: 'green' }] }), 3, 4, 3, 4),
            W(qv('Desired Not Available', `sum:kubernetes_state.deployment.replicas_desired_not_available${F}`, { formats: [{ comparator: '>', value: 0, palette: 'red' }, { comparator: '<=', value: 0, palette: 'green' }] }), 6, 4, 3, 4),
            W(qv('Outdated', `sum:kubernetes_state.deployment.outdated${F}`, { formats: [{ comparator: '>', value: 0, palette: 'yellow' }, { comparator: '<=', value: 0, palette: 'green' }] }), 9, 4, 3, 4),
            W(ts('Deployments', [`sum:kubernetes_state.deployment.count${F}`]), 0, 8, 6, 5),
            W(ts('Replicas per Deployments', [`sum:kubernetes_state.deployment.replicas${BY}`]), 6, 8, 6, 5),
            W({ type: 'change', title: 'Replicas per Deployments', request: { query: `sum:kubernetes_state.deployment.replicas{$kube_cluster_name,$kube_namespace} by {kube_cluster_name,kube_namespace,kube_deployment}` }, aggregator: 'avg', limit: 20 }, 0, 13, 6, 6),
            W(ts('Replicas Desired but Not Available', [`sum:kubernetes_state.deployment.replicas_desired_not_available${BY}`], 'area', { stacked: true }), 6, 13, 6, 6),
          ],
        },
        5,
        0,
        7,
        22,
      ),
      W(
        {
          type: 'group',
          title: 'Resource Usage',
          color: 'purple',
          widgets: [
            W(ts('CPU usage by deployment (millicores)', [`sum:kubernetes.cpu.usage.total${BY}`], 'area', { stacked: true }), 0, 0, 6, 6),
            W(ts('Memory usage by deployment (MiB)', [`sum:kubernetes.memory.usage${BY}`], 'line'), 6, 0, 6, 6),
            W(top('Top restarting deployments', `sum:kubernetes.containers.restarts${BY}`), 0, 6, 6, 6),
            W({ type: 'heatmap', title: 'CPU usage distribution', request: { query: `avg:kubernetes.cpu.usage.total${F}` } }, 6, 6, 6, 6),
          ],
        },
        0,
        22,
        12,
        14,
      ),
    ],
  };
}

function monitorNotifications(): Omit<Dashboard, keyof DashboardMeta> {
  const F = '{$env,$monitor_name,$service,$team,$monitor_id}';
  return {
    title: 'Monitor Notifications Overview',
    description: 'Live monitor status and notification trends.',
    kind: 'integration',
    logo: 'tile:monitors',
    layoutType: 'grid',
    templateVariables: [tv('env'), tv('monitor_name'), tv('service'), tv('team'), tv('monitor_id')],
    widgets: [
      W(note('### ⚡ LIVE Monitor Status', { align: 'center' }), 0, 0, 6, 2),
      W(note('This is a live status summary of your current monitors. This widget is always showing real-time data independent of the dashboard time frame.', { background: 'yellow', align: 'center', pointer: true }), 0, 2, 6, 3),
      W({ type: 'monitor_summary', query: '' }, 0, 5, 6, 12),
      W(
        {
          type: 'group',
          title: 'Notifications Overview',
          color: 'purple',
          widgets: [
            W(note('"Notifications" are created every time a Monitor changes status. These widgets follow the global timeframe, allowing you to compare trends over time.', { background: 'yellow', align: 'center', pointer: true }), 0, 0, 12, 3),
            W(qv('Alert Events', `sum:observecraft.monitor.notifications{status:alert,${F.slice(1)}`, { aggregator: 'sum', fill: true, formats: [{ comparator: '>=', value: 0, palette: 'red' }] }), 0, 3, 4, 3),
            W(qv('Warn Events', `sum:observecraft.monitor.notifications{status:warn,${F.slice(1)}`, { aggregator: 'sum', fill: true, formats: [{ comparator: '>=', value: 0, palette: 'yellow' }] }), 4, 3, 4, 3),
            W(qv('OK Events', `sum:observecraft.monitor.notifications{status:ok,${F.slice(1)}`, { aggregator: 'sum', fill: true, formats: [{ comparator: '>=', value: 0, palette: 'green' }] }), 8, 3, 4, 3),
            W(
              {
                type: 'timeseries',
                title: 'Alert and warn notifications timeline',
                requests: [
                  { query: `sum:observecraft.monitor.notifications{status:alert,${F.slice(1)}`, alias: 'Alerts' },
                  { query: `sum:observecraft.monitor.notifications{status:warn,${F.slice(1)}`, alias: 'Warns' },
                ],
                display: 'bars',
              },
              0,
              6,
              12,
              5,
            ),
            W(top('Top alert and warn notifications', `sum:observecraft.monitor.notifications{!status:ok,${F.slice(1)} by {monitor_name}`, 5, 'sum'), 0, 11, 12, 5),
          ],
        },
        6,
        0,
        6,
        17,
      ),
      W(
        {
          type: 'group',
          title: 'Notification Breakdown',
          color: 'purple',
          widgets: [
            W(top('Top notifications', `sum:observecraft.monitor.notifications${F} by {monitor_name}`, 10, 'sum'), 0, 0, 3, 7),
            W(top('Top notifying services', `sum:observecraft.monitor.notifications${F} by {service}`, 10, 'sum'), 3, 0, 3, 7),
            W(top('Top notifying monitors', `sum:observecraft.monitor.notifications{!status:ok,${F.slice(1)} by {monitor_name}`, 10, 'sum'), 6, 0, 3, 7),
            W(top('Top notifying hosts', `sum:observecraft.monitor.notifications${F} by {host}`, 10, 'sum'), 9, 0, 3, 7),
          ],
        },
        0,
        17,
        12,
        9,
      ),
    ],
  };
}

function billing(): Omit<Dashboard, keyof DashboardMeta> {
  return {
    title: 'Billing',
    description: 'Billing APIs and log activity.',
    kind: 'custom',
    layoutType: 'grid',
    templateVariables: [tv('env', 'env', 'prod'), tv('service')],
    widgets: [
      W(ts('API Gateway count metric', ['sum:trace.http.request.hits{$env,$service} by {service}']), 0, 0, 7, 7),
      W(ts('apigateway errors', ['sum:trace.http.request.errors{$env,$service} by {service}'], 'bars'), 7, 0, 5, 7),
      W(
        {
          type: 'table',
          title: 'Billing to Order APIs',
          requests: [
            { query: 'sum:trace.http.request.hits{$env,$service} by {service}', alias: 'REQUESTS COUNT' },
            { query: 'sum:trace.http.request.errors{$env,$service} by {service}', alias: 'ERROR' },
            { query: 'avg:trace.http.request.duration.p95{$env,$service} by {service}', alias: 'P95 (ms)' },
          ],
          aggregator: 'avg',
          limit: 20,
          searchable: true,
        },
        0,
        7,
        12,
        8,
      ),
      W({ type: 'log_stream', title: 'Billing logs', query: 'service:billing', limit: 15 }, 0, 15, 12, 9),
    ],
  };
}

function k8sPods(): Omit<Dashboard, keyof DashboardMeta> {
  const F = '{$kube_cluster_name,$kube_namespace}';
  return {
    title: 'Kubernetes Pods Overview',
    kind: 'integration',
    logo: 'kubernetes',
    layoutType: 'grid',
    templateVariables: [tv('kube_cluster_name'), tv('kube_namespace')],
    widgets: [
      W(qv('Running pods', `sum:kubernetes_state.pod.running${F}`, { formats: [{ comparator: '>=', value: 0, palette: 'green' }] }), 0, 0, 4, 4),
      W(qv('Pending pods', `sum:kubernetes_state.pod.pending${F}`, { formats: [{ comparator: '>', value: 0, palette: 'yellow' }, { comparator: '<=', value: 0, palette: 'green' }] }), 4, 0, 4, 4),
      W(qv('Failed pods', `sum:kubernetes_state.pod.failed${F}`, { formats: [{ comparator: '>', value: 0, palette: 'red' }, { comparator: '<=', value: 0, palette: 'green' }] }), 8, 0, 4, 4),
      W(ts('Running pods by namespace', [`sum:kubernetes_state.pod.running${F} by {kube_namespace}`], 'area', { stacked: true }), 0, 4, 8, 6),
      W({ type: 'pie', title: 'Pods by cluster', request: { query: `sum:kubernetes_state.pod.running${F} by {kube_cluster_name}` }, aggregator: 'last' }, 8, 4, 4, 6),
      W({ type: 'treemap', title: 'Memory by namespace', request: { query: `sum:kubernetes.memory.usage${F} by {kube_namespace}` }, aggregator: 'avg' }, 0, 10, 6, 6),
      W(top('Most restarts', `sum:kubernetes.containers.restarts${F} by {kube_deployment}`), 6, 10, 6, 6),
    ],
  };
}

function k8sNodes(): Omit<Dashboard, keyof DashboardMeta> {
  return {
    title: 'Kubernetes Nodes Overview',
    kind: 'integration',
    logo: 'kubernetes',
    layoutType: 'grid',
    templateVariables: [tv('kube_cluster_name'), tv('env')],
    widgets: [
      W(ts('CPU user % by node', ['avg:system.cpu.user{$kube_cluster_name,$env} by {host}']), 0, 0, 6, 6),
      W(ts('Load (1m) by node', ['avg:system.load.1{$kube_cluster_name,$env} by {host}']), 6, 0, 6, 6),
      W({ type: 'distribution', title: 'Memory used distribution (GiB)', request: { query: 'avg:system.mem.used{$kube_cluster_name,$env}' } }, 0, 6, 6, 6),
      W({ type: 'bar_chart', title: 'Disk in use by cluster', request: { query: 'avg:system.disk.in_use{$kube_cluster_name,$env} by {kube_cluster_name}' }, aggregator: 'avg', limit: 10 }, 6, 6, 6, 6),
    ],
  };
}

function serviceOverview(): Omit<Dashboard, keyof DashboardMeta> {
  return {
    title: 'Service Overview — Production',
    description: 'Golden signals for production services.',
    kind: 'custom',
    layoutType: 'grid',
    templateVariables: [tv('env', 'env', 'prod'), tv('service')],
    widgets: [
      W(qv('Requests / s', 'sum:trace.http.request.hits{$env,$service}', { aggregator: 'avg', unit: ' req/s', formats: [{ comparator: '>=', value: 0, palette: 'green' }] }), 0, 0, 3, 4),
      W(qv('Error rate', 'avg:trace.http.request.error_rate{$env,$service}', { aggregator: 'avg', precision: 2, unit: '%', fill: true, formats: [{ comparator: '>', value: 3, palette: 'red' }, { comparator: '>', value: 1, palette: 'yellow' }, { comparator: '<=', value: 1, palette: 'green' }] }), 3, 0, 3, 4),
      W(qv('p95 latency', 'avg:trace.http.request.duration.p95{$env,$service}', { aggregator: 'avg', unit: ' ms', formats: [{ comparator: '>', value: 300, palette: 'red' }, { comparator: '>', value: 150, palette: 'yellow' }, { comparator: '<=', value: 150, palette: 'green' }] }), 6, 0, 3, 4),
      W(top('Top hosts by CPU', 'avg:system.cpu.user{*} by {host}', 5), 9, 0, 3, 4),
      W(ts('Requests by service', ['sum:trace.http.request.hits{$env,$service} by {service}']), 0, 4, 6, 6),
      W(ts('p95 latency by service', ['avg:trace.http.request.duration.p95{$env,$service} by {service}'], 'area', { stacked: true }), 6, 4, 6, 6),
      W(ts('Errors by service', ['sum:trace.http.request.errors{$env,$service} by {service}'], 'bars', { stacked: true }), 0, 10, 6, 6),
      W({ type: 'heatmap', title: 'Request latency distribution', request: { query: 'avg:trace.http.request.duration.p95{$env,$service}' } }, 6, 10, 6, 6),
    ],
  };
}

type DashboardMeta = Pick<Dashboard, 'id' | 'author' | 'teams' | 'popularity' | 'createdAt' | 'modifiedAt' | 'viewedAt' | 'starred' | 'deletedAt'>;

const GENERIC_NAMES = ['Checkout', 'Business KPIs', 'pager-sync', 'Anomaly Overview - NOC', 'checkout-devops', 'Payments SLOs', 'Checkout funnel', 'Returns intake', 'Subscription renewal', 'Partner portal', 'Mobile app health', 'API Gateway', 'Batch jobs', 'Data pipelines', 'Kafka lag', 'Mongo clusters', 'Redis caches', 'NOC wallboard', 'On-call handover', 'Release health', 'Cost overview', 'Security posture', 'Billing reconciliation', 'Notifications service', 'Search latency', 'Auth service', 'Billing', 'Workflow engine', 'Order backend', 'Web vitals'];
const INTEGRATION_DASHES: [string, string][] = [
  ['Redis - Overview', 'redis'],
  ['NGINX - Overview', 'nginx'],
  ['PostgreSQL - Overview', 'postgresql'],
  ['Kafka - Overview', 'apachekafka'],
  ['AWS Lambda', 'awslambda'],
  ['Amazon RDS', 'amazonrds'],
  ['RabbitMQ - Overview', 'rabbitmq'],
  ['MongoDB - Overview', 'mongodb'],
  ['CoreDNS', 'coredns'],
  ['Istio - Overview', 'istio'],
];

function genericWidgets(r: () => number): Widget[] {
  const metric = ['trace.http.request.hits', 'system.cpu.user', 'kubernetes.cpu.usage.total', 'trace.http.request.duration.p95'][Math.floor(r() * 4)];
  const by = metric.startsWith('trace') ? 'service' : metric.startsWith('system') ? 'host' : 'kube_namespace';
  return [
    W(qv('Current value', `avg:${metric}{*}`, { aggregator: 'avg', formats: [{ comparator: '>=', value: 0, palette: 'blue' }] }), 0, 0, 4, 4),
    W(ts(`${metric} by ${by}`, [`avg:${metric}{*} by {${by}}`]), 4, 0, 8, 6),
    W(top(`Top ${by}`, `avg:${metric}{*} by {${by}}`, 8), 0, 4, 4, 6),
  ];
}

interface Store {
  dashboards: Dashboard[];
  lists: DashboardList[];
}

let store: Store | null = null;

function seed(): Store {
  const r = rng(77);
  idRand = rng(4242);
  wseq = 0;
  const now = Date.now();
  const meta = (i: number, author: User | null, teams: string[], viewedAgo: number | null, popularity: number): DashboardMeta => ({
    id: newId(),
    author,
    teams,
    popularity,
    createdAt: now - (30 + Math.floor(r() * 500)) * DAY,
    modifiedAt: now - Math.floor(r() * 120) * DAY - i * 3600e3,
    viewedAt: viewedAgo === null ? null : now - viewedAgo,
    starred: false,
    deletedAt: null,
  });
  const dashboards: Dashboard[] = [
    { ...meta(0, null, [], 48 * 60e3, 2), ...k8sDeployments() },
    { ...meta(1, null, [], 37 * 60e3, 4), ...monitorNotifications() },
    { ...meta(2, null, [], 60 * DAY, 3), ...k8sPods() },
    { ...meta(3, AUTHORS[3], ['billing'], 90 * DAY, 2), ...billing() },
    { ...meta(4, null, [], null, 0), ...k8sNodes() },
    { ...meta(5, CURRENT_USER, ['platform'], 2 * DAY, 3), ...serviceOverview() },
  ];
  INTEGRATION_DASHES.forEach(([title, logo], i) =>
    dashboards.push({ ...meta(6 + i, null, [], null, Math.floor(r() * 3)), title, kind: 'integration', logo, layoutType: 'grid', templateVariables: [tv('env')], widgets: genericWidgets(r) }),
  );
  for (let i = 0; i < 100; i++) {
    const base = GENERIC_NAMES[i % GENERIC_NAMES.length];
    const title = i < GENERIC_NAMES.length ? base : `${base} ${['(copy)', 'v2', '- staging', '- qa', '- EU'][i % 5]}`;
    const author = AUTHORS[Math.floor(r() * AUTHORS.length)];
    dashboards.push({
      ...meta(20 + i, author, r() < 0.4 ? [['billing', 'storefront', 'platform', 'checkout'][Math.floor(r() * 4)]] : [], null, Math.floor(r() * 5)),
      title,
      kind: 'custom',
      layoutType: r() < 0.85 ? 'grid' : 'auto',
      templateVariables: [tv('env')],
      widgets: genericWidgets(r),
    });
  }
  const ids = dashboards.map((d) => d.id);
  const lists: DashboardList[] = [
    { id: 'l-archived', name: 'Archived Dashboards', dashboardIds: [ids[30]] },
    { id: 'l-noc-approved', name: 'NOC - Approved', dashboardIds: ids.slice(0, 3) },
    { id: 'l-noc-work', name: 'NOC - At Work', dashboardIds: ids.slice(10, 19) },
    { id: 'l-platform', name: 'Platform', dashboardIds: [] },
    { id: 'l-billing', name: 'Billing team', dashboardIds: [ids[3], ids[5]] },
    { id: 'l-oncall', name: 'On-call', dashboardIds: ids.slice(1, 7) },
  ];
  idRand = Math.random;
  return { dashboards, lists };
}

let lastRaw: string | null = null;

/** localStorage is the source of truth (shared across tabs); the parsed copy is cached until it changes. */
function db(): Store {
  let raw: string | null = null;
  try {
    raw = localStorage.getItem(STORAGE_KEY);
  } catch {
    /* storage unavailable: in-memory only */
  }
  if (raw && raw !== lastRaw) {
    try {
      store = JSON.parse(raw) as Store;
      lastRaw = raw;
    } catch {
      store = null;
    }
  }
  if (!store) {
    store = seed();
    persist();
    // Drop copies left by older store versions.
    try {
      for (const k of Object.keys(localStorage)) if (k.startsWith('oc.mock.dashboards.') && k !== STORAGE_KEY) localStorage.removeItem(k);
    } catch {
      /* storage unavailable */
    }
  }
  return store;
}

function persist() {
  try {
    lastRaw = JSON.stringify(store);
    localStorage.setItem(STORAGE_KEY, lastRaw);
  } catch {
    /* storage full/unavailable: keep in memory only */
  }
}

function find(id: string): Dashboard {
  const d = db().dashboards.find((x) => x.id === id);
  if (!d) throw new Error('Dashboard not found');
  return d;
}

function summary(d: Dashboard): DashboardSummary {
  const { widgets: _w, templateVariables: _t, layoutType: _l, ...s } = d;
  return s;
}

// ---------------------------------------------------------------- endpoints

export function searchDashboards(req: DashboardSearchRequest): Promise<DashboardSearchPage> {
  const q = req.query.trim().toLowerCase();
  const s = db();
  const list = req.listId ? s.lists.find((l) => l.id === req.listId) : null;
  let items = s.dashboards.filter((d) => (req.preset === 'deleted' ? d.deletedAt !== null : d.deletedAt === null));
  if (req.preset === 'custom') items = items.filter((d) => d.kind === 'custom');
  if (req.preset === 'integrations') items = items.filter((d) => d.kind === 'integration');
  if (req.preset === 'mine') items = items.filter((d) => d.author?.id === CURRENT_USER.id);
  if (req.preset === 'starred') items = items.filter((d) => d.starred);
  if (list) items = items.filter((d) => list.dashboardIds.includes(d.id));
  if (req.team) items = items.filter((d) => d.teams.includes(req.team!));
  if (q) items = items.filter((d) => d.title.toLowerCase().includes(q) || d.author?.name.toLowerCase().includes(q));
  const dir = req.sort.dir === 'asc' ? 1 : -1;
  items.sort((a, b) => {
    // Starred first, like the reference list.
    if (a.starred !== b.starred) return a.starred ? -1 : 1;
    const f = req.sort.field;
    const x = f === 'title' ? a.title.toLowerCase() : f === 'popularity' ? a.popularity : a.modifiedAt;
    const y = f === 'title' ? b.title.toLowerCase() : f === 'popularity' ? b.popularity : b.modifiedAt;
    return (x < y ? -1 : x > y ? 1 : 0) * dir;
  });
  return delay({ dashboards: items.slice(req.offset, req.offset + req.limit).map(summary), total: items.length });
}

export function recentlyViewed(limit: number): Promise<DashboardSummary[]> {
  return delay(
    db()
      .dashboards.filter((d) => d.viewedAt !== null && d.deletedAt === null)
      .sort((a, b) => b.viewedAt! - a.viewedAt!)
      .slice(0, limit)
      .map(summary),
  );
}

export function getDashboard(id: string): Promise<Dashboard> {
  try {
    const d = find(id);
    d.viewedAt = Date.now();
    persist();
    return delay(d);
  } catch (e) {
    return Promise.reject(e);
  }
}

export function createDashboard(req: CreateDashboardRequest): Promise<Dashboard> {
  const now = Date.now();
  const d: Dashboard = {
    id: newId(),
    title: req.title,
    kind: 'custom',
    author: CURRENT_USER,
    teams: req.teams,
    popularity: 0,
    createdAt: now,
    modifiedAt: now,
    viewedAt: now,
    starred: false,
    deletedAt: null,
    layoutType: req.layoutType,
    templateVariables: [],
    widgets: [],
  };
  db().dashboards.unshift(d);
  persist();
  return delay(d);
}

export type DashboardPatch = Partial<Pick<Dashboard, 'title' | 'description' | 'widgets' | 'templateVariables' | 'layoutType' | 'teams' | 'starred'>>;

export function updateDashboard(id: string, patch: DashboardPatch): Promise<Dashboard> {
  try {
    const d = find(id);
    Object.assign(d, structuredClone(patch));
    if (!('starred' in patch && Object.keys(patch).length === 1)) d.modifiedAt = Date.now();
    persist();
    return delay(d, 120);
  } catch (e) {
    return Promise.reject(e);
  }
}

export function cloneDashboard(id: string): Promise<Dashboard> {
  const src = find(id);
  const now = Date.now();
  const copy: Dashboard = { ...structuredClone(src), id: newId(), title: `Clone of ${src.title}`, kind: 'custom', logo: src.logo, author: CURRENT_USER, createdAt: now, modifiedAt: now, viewedAt: now, starred: false, popularity: 0 };
  db().dashboards.unshift(copy);
  persist();
  return delay(copy);
}

export function deleteDashboards(ids: string[]): Promise<void> {
  for (const id of ids) find(id).deletedAt = Date.now();
  persist();
  return delay(undefined);
}

export function restoreDashboard(id: string): Promise<void> {
  find(id).deletedAt = null;
  persist();
  return delay(undefined);
}

export function listLists(): Promise<DashboardList[]> {
  return delay(db().lists);
}

export function createList(name: string): Promise<DashboardList> {
  const l: DashboardList = { id: `l-${newId()}`, name, dashboardIds: [] };
  db().lists.push(l);
  persist();
  return delay(l);
}

export function addToList(listId: string, dashboardIds: string[]): Promise<DashboardList> {
  const l = db().lists.find((x) => x.id === listId);
  if (!l) return Promise.reject(new Error('List not found'));
  l.dashboardIds = [...new Set([...l.dashboardIds, ...dashboardIds])];
  persist();
  return delay(l);
}

export function monitorSummary(query: string): Promise<MonitorSummary> {
  const terms = parseQuery(query);
  const ms = monitorEntities().filter((m) => matchesTerms((k) => m[k], m.monitor_name, terms));
  const counts = { alert: 0, warn: 0, ok: 0, nodata: 0 };
  for (const m of ms) counts[m.status as keyof typeof counts]++;
  const order = { alert: 0, warn: 1, nodata: 2, ok: 3 } as const;
  const monitors = ms
    .map((m) => ({ id: m.monitor_id, name: m.monitor_name, status: m.status as MonitorSummary['monitors'][number]['status'], muted: Number(m.monitor_id) % 17 === 0 }))
    .sort((a, b) => order[a.status] - order[b.status]);
  return delay({ counts, monitors });
}
