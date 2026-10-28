// Synthetic distributed traces for the in-browser APM mock: the demo shop's request flows (the
// services, hosts and languages oc-tracegen and the network mock use), generated minute by minute
// from a seed so any time range can be rebuilt and a trace id alone finds its trace again.
import { rng, type Rand } from './generator';

export type SpanKind = 'Server' | 'Client' | 'Producer' | 'Consumer';

export interface SpanError {
  type: string;
  message: string;
  /** HTTP status the failing server answered with. */
  status: number;
}

export interface GenTrace {
  traceId: string;
  env: 'prod' | 'staging';
  spans: GenSpan[];
}

export interface GenSpan {
  trace: GenTrace;
  spanId: string;
  /** '' for the root span. */
  parentId: string;
  service: string;
  name: string;
  kind: SpanKind;
  /** Epoch ms (fractional). */
  start: number;
  durationMs: number;
  error: SpanError | null;
  attrs: Record<string, string>;
  version: string;
}

export const SERVICES: Record<string, { lang: string; host: string; namespace: string }> = {
  'api-gateway': { lang: 'go', host: 'i-0a1b2c3d4e5f60001', namespace: 'edge' },
  'storefront-api': { lang: 'java', host: 'i-0a1b2c3d4e5f60002', namespace: 'shop' },
  'order-service': { lang: 'go', host: 'i-0a1b2c3d4e5f60001', namespace: 'shop' },
  billing: { lang: 'go', host: 'i-0a1b2c3d4e5f60002', namespace: 'payments' },
  'inventory-service': { lang: 'nodejs', host: 'i-0a1b2c3d4e5f60003', namespace: 'shop' },
  'catalog-service': { lang: 'dotnet', host: 'i-0a1b2c3d4e5f60004', namespace: 'shop' },
  'workflow-engine': { lang: 'java', host: 'i-0a1b2c3d4e5f60004', namespace: 'payments' },
  'notification-worker': { lang: 'python', host: 'i-0a1b2c3d4e5f60005', namespace: 'shop' },
};

const MIN = 60_000;
/** Times below are relative to when the mock first loads, so recent ranges always show the same story. */
export const ANCHOR = Date.now();
/** catalog-service 7.1.0 ships a slower product query: latency and timeouts rise from here on. */
const CATALOG_RELEASE = ANCHOR - 25 * MIN;

// Versions per service, newest first, with the offset from ANCHOR at which each rolled out.
const VERSIONS: Record<string, [version: string, since: number][]> = {
  'api-gateway': [['1.12.0', -Infinity]],
  'storefront-api': [
    ['2.14.3', -2 * 24 * 60 * MIN],
    ['2.14.2', -Infinity],
  ],
  'order-service': [
    ['3.2.1', -95 * MIN],
    ['3.2.0', -3 * 24 * 60 * MIN],
    ['3.1.4', -Infinity],
  ],
  billing: [['1.8.0', -Infinity]],
  'inventory-service': [['4.1.2', -Infinity]],
  'catalog-service': [
    ['7.1.0', CATALOG_RELEASE - ANCHOR],
    ['7.0.1', -Infinity],
  ],
  'workflow-engine': [['3.3.0', -Infinity]],
  'notification-worker': [['0.9.4', -Infinity]],
};

export function versionAt(service: string, t: number): string {
  return VERSIONS[service].find(([, since]) => t >= ANCHOR + since)![0];
}

/** When the version running now was first deployed (epoch ms), or null when it predates `since`. */
export function deployedAt(service: string, since: number): number | null {
  const start = ANCHOR + VERSIONS[service][0][1];
  return start >= since ? start : null;
}

function hex(r: Rand, len: number): string {
  let s = '';
  for (let i = 0; i < len; i++) s += Math.floor(r() * 16).toString(16);
  return s;
}

/** Log-normal-ish latency around `median` ms with a long tail. */
function latency(r: Rand, median: number): number {
  const z = r() + r() + r() + r() - 2;
  return median * Math.exp(z * 0.9) * (r() < 0.01 ? 8 : 1);
}

interface Step {
  end: number;
  /** The failure the calling server reports, if any. */
  err: SpanError | null;
}

class TraceBuilder {
  readonly spans: GenSpan[] = [];
  readonly r: Rand;
  readonly trace: GenTrace;
  constructor(r: Rand, trace: GenTrace) {
    this.r = r;
    this.trace = trace;
  }

  push(service: string, parentId: string, name: string, kind: SpanKind, start: number, end: number, attrs: Record<string, string>, error: SpanError | null): string {
    const spanId = hex(this.r, 16);
    this.spans.push({ trace: this.trace, spanId, parentId, service, name, kind, start, durationMs: end - start, error, attrs, version: versionAt(service, start) });
    return spanId;
  }

  /** A server span whose children `body` creates; it fails with `own` or with what its body reports. */
  server(service: string, parentId: string, start: number, method: string, route: string, selfMs: number, body: (id: string, start: number) => Step, own: SpanError | null = null): Step {
    // The span id is needed by the children before the span itself is known: reserve its slot.
    const index = this.spans.length;
    const spanId = hex(this.r, 16);
    this.spans.push(null as unknown as GenSpan);
    const child = body(spanId, start + latency(this.r, selfMs / 2));
    const end = child.end + latency(this.r, selfMs / 2);
    const err = own ?? child.err;
    const attrs = { 'http.request.method': method, 'http.route': route, 'http.response.status_code': String(err?.status ?? 200) };
    this.spans[index] = {
      trace: this.trace,
      spanId,
      parentId,
      service,
      name: `${method} ${route}`,
      kind: 'Server',
      start,
      durationMs: end - start,
      error: err,
      attrs,
      version: versionAt(service, start),
    };
    return { end, err };
  }

  /** An HTTP call from `from` to `to`'s server span; a failure comes back as an upstream error. */
  call(from: string, to: string, parentId: string, start: number, method: string, route: string, selfMs: number, body: (id: string, start: number) => Step, own: SpanError | null = null): Step {
    const index = this.spans.length;
    const spanId = hex(this.r, 16);
    this.spans.push(null as unknown as GenSpan);
    const served = this.server(to, spanId, start + latency(this.r, 0.6), method, route, selfMs, body, own);
    const end = served.end + latency(this.r, 0.6);
    const status = String(served.err?.status ?? 200);
    const attrs = { 'peer.service': to, 'http.request.method': method, 'url.path': route, 'http.response.status_code': status };
    this.spans[index] = {
      trace: this.trace,
      spanId,
      parentId,
      service: from,
      name: `${method} ${route}`,
      kind: 'Client',
      start,
      durationMs: end - start,
      error: served.err,
      attrs,
      version: versionAt(from, start),
    };
    const err = served.err && { type: 'UpstreamServiceError', message: `${to}: ${method} ${route} returned ${served.err.status}`, status: 502 };
    return { end, err };
  }

  /** A client call to a database or cache (an inferred service); its failure fails the caller as is. */
  db(service: string, parentId: string, start: number, peer: string, system: string, dbName: string, name: string, statement: string, median: number, err: SpanError | null = null): Step {
    const end = start + latency(this.r, median) * (err ? 6 : 1);
    this.push(service, parentId, name, 'Client', start, end, { 'peer.service': peer, 'db.system': system, 'db.name': dbName, 'db.statement': statement }, err);
    return { end, err };
  }
}

const fail = (type: string, message: string, status = 500): SpanError => ({ type, message, status });

/** Browsing: product listing from the catalog, sometimes with an N+1 image query. */
function browse(b: TraceBuilder, start: number) {
  const r = b.r;
  const degraded = start >= CATALOG_RELEASE;
  const timeout = r() < (degraded ? 0.05 : 0.002) ? fail('QueryCanceled', 'canceling statement due to statement timeout') : null;
  const nPlusOne = r() < 0.2;
  b.server('api-gateway', '', start, 'GET', '/api/products', 1, (p, s) =>
    b.call('api-gateway', 'storefront-api', p, s, 'GET', '/api/products', 4, (p, s) =>
      b.call('storefront-api', 'catalog-service', p, s, 'GET', '/products', 3, (p, s) => {
        const q = 'SELECT id, sku, name, price FROM products WHERE category_id = ? ORDER BY rank LIMIT ?';
        let step = b.db('catalog-service', p, s, 'postgres', 'postgresql', 'catalog', 'SELECT products', q, degraded ? 45 : 5, timeout);
        if (step.err) return step;
        for (let i = 0; i < (nPlusOne ? 6 : 1); i++) {
          step = b.db('catalog-service', p, step.end + 0.1, 'postgres', 'postgresql', 'catalog', 'SELECT product_images', 'SELECT url, alt FROM product_images WHERE product_id = ?', 1.5);
        }
        return step;
      }),
    ),
  );
}

/** Checkout: order, stock reservation, payment, then fulfilment and an email through Kafka. */
function checkout(b: TraceBuilder, start: number) {
  const r = b.r;
  const stockErr = r() < 0.015 ? fail('RedisTimeoutError', 'Command timed out after 500ms: DECRBY stock') : null;
  const payErr = r() < 0.03 ? fail('PaymentProviderTimeout', 'payments provider did not answer within 5s', 504) : null;
  const mailErr = r() < 0.02 ? fail('EmailDeliveryError', 'email provider answered 429 Too Many Requests', 429) : null;
  b.server('api-gateway', '', start, 'POST', '/api/orders', 1, (p, s) =>
    b.call('api-gateway', 'storefront-api', p, s, 'POST', '/api/orders', 5, (p, s) =>
      b.call('storefront-api', 'order-service', p, s, 'POST', '/orders', 6, (p, s) => {
        const insert = 'INSERT INTO orders (customer_id, total, currency) VALUES (?, ?, ?)';
        let step = b.db('order-service', p, s, 'postgres', 'postgresql', 'orders', 'INSERT orders', insert, 3);
        step = b.call('order-service', 'inventory-service', p, step.end + 0.2, 'POST', '/inventory/reserve', 4, (p, s) => {
          const got = b.db('inventory-service', p, s, 'redis', 'redis', '0', 'GET', 'GET stock:?', 0.4);
          return b.db('inventory-service', p, got.end + 0.05, 'redis', 'redis', '0', 'DECRBY', 'DECRBY stock:? ?', 0.5, stockErr);
        });
        if (step.err) return step;
        step = b.call('order-service', 'billing', p, step.end + 0.2, 'POST', '/payments/charge', 3, (p, s) => {
          const end = s + (payErr ? 5000 : latency(r, 140));
          b.push(
            'billing',
            p,
            'POST /v1/charges',
            'Client',
            s,
            end,
            { 'peer.service': 'payments-provider', 'http.request.method': 'POST', 'url.path': '/v1/charges', 'http.response.status_code': payErr ? '504' : '200' },
            payErr,
          );
          return { end, err: payErr };
        });
        if (step.err) return step;
        return b.call('order-service', 'workflow-engine', p, step.end + 0.2, 'POST', '/workflows/fulfillment', 6, (p, s) => {
          const task = b.db('workflow-engine', p, s, 'postgres', 'postgresql', 'workflow', 'INSERT workflow_tasks', 'INSERT INTO workflow_tasks (order_id, kind, state) VALUES (?, ?, ?)', 2.5);
          const queue = { 'peer.service': 'kafka', 'messaging.system': 'kafka', 'messaging.destination.name': 'order-events' };
          const pubEnd = task.end + latency(r, 1.5);
          const producer = b.push('workflow-engine', p, 'order-events publish', 'Producer', task.end, pubEnd, queue, null);
          // The consumer runs asynchronously, after the producer returned.
          const consStart = pubEnd + latency(r, 4);
          const mailStart = consStart + latency(r, 1);
          const mailEnd = mailStart + latency(r, 80);
          const consumer = b.push(
            'notification-worker',
            producer,
            'order-events process',
            'Consumer',
            consStart,
            mailEnd + latency(r, 0.8),
            { 'messaging.system': 'kafka', 'messaging.destination.name': 'order-events' },
            mailErr,
          );
          const mail = { 'peer.service': 'email-provider', 'http.request.method': 'POST', 'url.path': '/v3/mail/send', 'http.response.status_code': mailErr ? '429' : '202' };
          b.push('notification-worker', consumer, 'POST /v3/mail/send', 'Client', mailStart, mailEnd, mail, mailErr);
          return { end: pubEnd, err: null };
        });
      }),
    ),
  );
}

/** Order status page. */
function orderStatus(b: TraceBuilder, start: number) {
  b.server('api-gateway', '', start, 'GET', '/api/orders/{id}', 1, (p, s) =>
    b.call('api-gateway', 'storefront-api', p, s, 'GET', '/api/orders/{id}', 3, (p, s) =>
      b.call('storefront-api', 'order-service', p, s, 'GET', '/orders/{id}', 2, (p, s) => {
        const order = b.db('order-service', p, s, 'postgres', 'postgresql', 'orders', 'SELECT orders', 'SELECT * FROM orders WHERE id = ?', 2);
        return b.db('order-service', p, order.end + 0.1, 'postgres', 'postgresql', 'orders', 'SELECT order_items', 'SELECT sku, quantity, price FROM order_items WHERE order_id = ?', 1.8);
      }),
    ),
  );
}

/** Invoice download, straight from the gateway to billing. */
function invoice(b: TraceBuilder, start: number) {
  const err = b.r() < 0.005 ? fail('InvoiceRenderError', 'line item total does not match invoice total') : null;
  b.server('api-gateway', '', start, 'GET', '/api/billing/v1/invoices/{id}', 1, (p, s) =>
    b.call(
      'api-gateway',
      'billing',
      p,
      s,
      'GET',
      '/api/billing/v1/invoices/{id}',
      4,
      (p, s) => {
        const inv = b.db('billing', p, s, 'postgres', 'postgresql', 'billing', 'SELECT invoices', 'SELECT * FROM invoices WHERE id = ?', 4);
        return b.db('billing', p, inv.end + 0.1, 'postgres', 'postgresql', 'billing', 'SELECT invoice_lines', 'SELECT * FROM invoice_lines WHERE invoice_id = ?', 2.5);
      },
      err,
    ),
  );
}

const FLOWS: [weight: number, build: (b: TraceBuilder, start: number) => void][] = [
  [0.4, browse],
  [0.25, checkout],
  [0.2, orderStatus],
  [0.15, invoice],
];

// Trace ids carry their minute and position (scrambled) so getTrace can rebuild the minute.
const MINUTE_MASK = 0x5bd1e995;
const ORDINAL_MASK = 0xa7;

function traceId(minute: number, ordinal: number, r: Rand): string {
  return (((minute ^ MINUTE_MASK) >>> 0).toString(16).padStart(8, '0') + (ordinal ^ ORDINAL_MASK).toString(16).padStart(2, '0') + hex(r, 22)).slice(0, 32);
}

function generate(minute: number): GenTrace[] {
  const r = rng(Math.imul(minute, 2654435761) ^ 0x2a);
  const from = minute * MIN;
  // About 20 traces a minute, busier in the European afternoon than at night.
  const hour = new Date(from).getUTCHours();
  const count = Math.round(15 + 5 * Math.sin(((hour - 8) / 24) * 2 * Math.PI) + r() * 4);
  const out: GenTrace[] = [];
  for (let i = 0; i < count; i++) {
    const trace: GenTrace = { traceId: traceId(minute, i, r), env: r() < 0.1 ? 'staging' : 'prod', spans: [] };
    const b = new TraceBuilder(r, trace);
    let x = r();
    const flow = FLOWS.find(([w]) => (x -= w) < 0) ?? FLOWS[0];
    flow[1](b, from + r() * MIN);
    trace.spans = b.spans;
    out.push(trace);
  }
  return out;
}

export const CHUNK_MS = MIN;
const MAX_CACHED = 1200;
const cache = new Map<number, GenTrace[]>();

/** The traces started in one minute (epoch minute index). */
export function tracesOfMinute(minute: number): GenTrace[] {
  let traces = cache.get(minute);
  if (!traces) {
    traces = generate(minute);
    if (cache.size >= MAX_CACHED) cache.delete(cache.keys().next().value!);
    cache.set(minute, traces);
  }
  return traces;
}

export function findTrace(id: string): GenTrace | undefined {
  if (!/^[0-9a-f]{32}$/.test(id)) return undefined;
  const minute = (parseInt(id.slice(0, 8), 16) ^ MINUTE_MASK) >>> 0;
  const ordinal = parseInt(id.slice(8, 10), 16) ^ ORDINAL_MASK;
  if (minute * MIN > Date.now()) return undefined;
  const trace = tracesOfMinute(minute)[ordinal];
  return trace?.traceId === id ? trace : undefined;
}
