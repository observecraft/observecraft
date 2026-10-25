// Realistic log event generator for the mock backend. Shapes mirror what Vector +
// the Kubernetes/Docker enrichment will produce: reserved fields, infra tags, parsed attributes.
import type { JsonValue, LogEvent, LogStatus } from '../types';

export type Rand = () => number;

export function rng(seed: number): Rand {
  seed = seed >>> 0; // callers may pass XOR-ed (signed 32-bit) seeds; keep the state unsigned
  return () => {
    seed = (seed * 1664525 + 1013904223) % 4294967296;
    return seed / 4294967296;
  };
}

const pick = <T,>(r: Rand, arr: readonly T[]): T => arr[Math.floor(r() * arr.length)];

function hex(r: Rand, len: number): string {
  let s = '';
  for (let i = 0; i < len; i++) s += Math.floor(r() * 16).toString(16);
  return s;
}

function uuid(r: Rand): string {
  return `${hex(r, 8)}-${hex(r, 4)}-4${hex(r, 3)}-${pick(r, ['8', '9', 'a', 'b'])}${hex(r, 3)}-${hex(r, 12)}`;
}

function alnum(r: Rand, len: number): string {
  const chars = 'bcdfghjklmnpqrstvwxz2456789';
  let s = '';
  for (let i = 0; i < len; i++) s += chars[Math.floor(r() * chars.length)];
  return s;
}

interface Emitted {
  message: string;
  status?: LogStatus;
  attributes?: Record<string, JsonValue>;
}

interface Workload {
  service: string;
  weight: number; // relative share of traffic
  namespace: 'ns-billing-prod' | 'ns-billing-staging';
  emit: (r: Rand, ts: number) => Emitted;
}

const PRODUCT_IDS = ['165daad8-0e7b-4bb7-ab2f-064b5ab021f4', '0c41d2f9-77aa-4e1b-9c3e-5a1f0b2e8d31', '9f3b6c2e-14d8-4a7f-b1e5-c0d2a9e47f18'];
const INVOICE_IDS = ['89feee05-4877-4862-adca-cad892229daf', '3b1e7c4a-92d0-4f65-8e1b-0a6d5c2f9e74'];

function goLogTime(ts: number): string {
  const d = new Date(ts);
  const p = (n: number) => String(n).padStart(2, '0');
  return `${d.getUTCFullYear()}/${p(d.getUTCMonth() + 1)}/${p(d.getUTCDate())} ${p(d.getUTCHours())}:${p(d.getUTCMinutes())}:${p(d.getUTCSeconds())}`;
}

const WORKLOADS: Workload[] = [
  {
    service: 'storefront-api',
    weight: 40,
    namespace: 'ns-billing-prod',
    emit: (r): Emitted => {
      const pid = pick(r, PRODUCT_IDS);
      const url = `http://pricing-service:8083/api/resources/v1/products/${pid}/${pick(r, ['resource', 'texts'])}`;
      const x = r();
      if (x < 0.35) return { message: `[RQ] [catalogue] GET ${url}`, attributes: { http: { method: 'GET', url } } };
      if (x < 0.65) {
        const ok = r() > 0.01;
        return {
          message: `[RS] [catalogue] GET ${url} ${ok ? '200 OK' : '502 Bad Gateway'}`,
          status: ok ? 'info' : 'error',
          attributes: { http: { method: 'GET', url, status_code: ok ? 200 : 502 }, duration_ms: Math.round(8 + r() * 120) },
        };
      }
      if (x < 0.85) return { message: '[IN] GET /api/images', attributes: { http: { method: 'GET', url_details: { path: '/api/images' } } } };
      const path = `/api/processes/${uuid(r)}/next`;
      return { message: `[IN] POST ${path}`, attributes: { http: { method: 'POST', url_details: { path } } } };
    },
  },
  {
    service: 'inventory-service',
    weight: 25,
    namespace: 'ns-billing-prod',
    emit: (r) => ({
      message: pick(r, [
        'updating state',
        'updating state',
        'page actions for ui',
        'page actions',
        '-- Response sent --',
        'response ready',
        'cycle page seq',
        'active page seq',
        'backup page seq',
        'add response for page',
        'identifying flow',
        'start of the dialogue',
        'next page per transitions',
        'action button visibility expression evaluated',
        'named expression definions in prod def',
      ]),
      attributes: { flow_id: `flow-${Math.floor(r() * 40)}` },
    }),
  },
  {
    service: 'order-service',
    weight: 10,
    namespace: 'ns-billing-prod',
    emit: (r): Emitted => {
      const x = r();
      const request_id = uuid(r);
      if (x < 0.06)
        return {
          message: pick(r, ['handleOrder', 'handleOrder.GetCustomer', 'GetCustomer bad response']),
          status: 'error',
          attributes: { request_id, error: { kind: 'UpstreamError', message: 'GetCustomer bad response' } },
        };
      return {
        message: pick(r, ['handleAgentCommission', 'handleAgentCommission: reply processed', 'handleOrder: request accepted']),
        attributes: { request_id },
      };
    },
  },
  {
    service: 'billing',
    weight: 8,
    namespace: 'ns-billing-prod',
    emit: (r, ts) => {
      const url = `/api/billing/v1/invoices/${pick(r, INVOICE_IDS)}`;
      return {
        message: `${goLogTime(ts)} INFO: GetInvoiceByIdHandler. got following request: method='GET' url='${url}'`,
        attributes: { http: { method: 'GET', url } },
      };
    },
  },
  {
    service: 'billing-watcher',
    weight: 5,
    namespace: 'ns-billing-prod',
    emit: (r) =>
      r() < 0.4
        ? { message: "'lineItems' missing in invoice", status: 'warn' }
        : { message: pick(r, ['sending event', 'event acknowledged', 'watch cycle complete']) },
  },
  {
    service: 'workflow-engine',
    weight: 7,
    namespace: 'ns-billing-prod',
    emit: (r) => ({
      message: pick(r, [`process instance started id=${hex(r, 12)}`, 'task completed', 'gateway evaluated: approve']),
      attributes: { process: { definition: pick(r, ['returns-intake', 'subscription-renewal']) } },
    }),
  },
  {
    service: 'pricing-service',
    weight: 4,
    namespace: 'ns-billing-prod',
    emit: (r) =>
      r() < 0.1
        ? { message: 'slow query on products table took 1243ms', status: 'warn', attributes: { duration_ms: 1243 } }
        : { message: `GET /api/resources/v1/products/${pick(r, PRODUCT_IDS)} 200`, attributes: { duration_ms: Math.round(4 + r() * 40) } },
  },
  {
    service: 'notificationsrv',
    weight: 3,
    namespace: 'ns-billing-staging',
    emit: (r) => ({ message: pick(r, ['email queued', 'sms delivered', 'push notification sent']) }),
  },
  {
    service: 'import-service',
    weight: 1,
    namespace: 'ns-billing-prod',
    emit: (r) => (r() < 0.2 ? { message: 'config reload failed: timeout', status: 'error' } : { message: 'config reloaded' }),
  },
];

const TOTAL_WEIGHT = WORKLOADS.reduce((a, w) => a + w.weight, 0);

// Stable per-workload infrastructure identity (host, pod, container, image).
const infraRand = rng(7);
const HOSTS = Array.from({ length: 5 }, () => `i-0${hex(infraRand, 16)}`);
/** Hosts emitting the mock logs — the infrastructure mock includes them so host → logs links have data. */
export const LOG_HOSTS: readonly string[] = HOSTS;
const INFRA = new Map(
  WORKLOADS.map((w, i) => {
    const rs = hex(infraRand, 10);
    const pod = `shop-${w.service}-${rs}-${alnum(infraRand, 5)}`;
    const env = w.namespace === 'ns-billing-prod' ? 'prod' : 'staging';
    const tags: Record<string, string> = {
      env,
      workloadname: 'billing',
      kube_namespace: w.namespace,
      kube_cluster_name: `billing${env}-core`,
      cluster_name: `billing${env}-core`,
      kube_deployment: `shop-${w.service}`,
      kube_replica_set: `shop-${w.service}-${rs}`,
      kube_container_name: w.service,
      pod_name: pod,
      container_name: w.service,
      display_container_name: `${w.service}_${pod}`,
      container_id: hex(infraRand, 64),
      docker_image: `123456789012.dkr.ecr.eu-central-1.amazonaws.com/${w.service}:2.${i + 3}.${Math.floor(infraRand() * 20)}`,
      image_name: `123456789012.dkr.ecr.eu-central-1.amazonaws.com/${w.service}`,
      image_tag: `2.${i + 3}`,
      region: 'eu-central-1',
      'availability-zone': pick(infraRand, ['eu-central-1a', 'eu-central-1b', 'eu-central-1c']),
      dirname: `/var/log/pods/${w.namespace}_${pod}_${uuid(infraRand)}/${w.service}`,
      filename: '0.log',
      service: w.service,
      source: w.service,
      host: HOSTS[i % HOSTS.length],
    };
    // One shared, frozen tag object per workload keeps long live-tail sessions cheap.
    return [w.service, { host: tags.host, tags: Object.freeze(tags) }];
  }),
);

export const SERVICE_NAMES = WORKLOADS.map((w) => w.service);

function pickWorkload(r: Rand): Workload {
  let x = r() * TOTAL_WEIGHT;
  for (const w of WORKLOADS) {
    x -= w.weight;
    if (x <= 0) return w;
  }
  return WORKLOADS[0];
}

let seq = 0;

export function generateLog(r: Rand, ts: number): LogEvent {
  const w = pickWorkload(r);
  const infra = INFRA.get(w.service)!;
  const e = w.emit(r, ts);
  const status = e.status ?? 'info';
  return {
    id: `log-${ts}-${seq++}`,
    timestamp: ts,
    status,
    service: w.service,
    host: infra.host,
    source: w.service,
    message: e.message,
    tags: infra.tags,
    attributes: {
      level: status === 'warn' ? 'warning' : status,
      timestamp: new Date(ts).toISOString(),
      ...e.attributes,
    },
  };
}
