// In-browser Cloud Network Monitoring for mock mode: the demo services' traffic (the same topology
// oc-tracegen sends), oriented and grouped like the API does.
import type { DnsFlow, DnsPage, DnsPoint, DnsRequest, Flow, FlowPage, FlowRequest, NetworkPoint } from '../network/client';
import { parseQuery, type Term } from '../../lib/searchQuery';
import { rng } from './generator';

type Tags = Record<string, string>;

const NODES: Tags[] = [
  { service: 'api-gateway', ip: '10.0.1.11', host: 'i-0a1b2c3d4e5f60001', 'availability-zone': 'us-east-1a', kube_namespace: 'edge' },
  { service: 'storefront-api', ip: '10.0.1.12', host: 'i-0a1b2c3d4e5f60002', 'availability-zone': 'us-east-1b', kube_namespace: 'shop' },
  { service: 'order-service', ip: '10.0.1.13', host: 'i-0a1b2c3d4e5f60001', 'availability-zone': 'us-east-1a', kube_namespace: 'shop' },
  { service: 'billing', ip: '10.0.1.14', host: 'i-0a1b2c3d4e5f60002', 'availability-zone': 'us-east-1b', kube_namespace: 'payments' },
  { service: 'inventory-service', ip: '10.0.2.15', host: 'i-0a1b2c3d4e5f60003', 'availability-zone': 'us-east-1a', kube_namespace: 'shop' },
  { service: 'catalog-service', ip: '10.0.2.16', host: 'i-0a1b2c3d4e5f60004', 'availability-zone': 'us-east-1c', kube_namespace: 'shop' },
  { service: 'workflow-engine', ip: '10.0.2.17', host: 'i-0a1b2c3d4e5f60004', 'availability-zone': 'us-east-1c', kube_namespace: 'payments' },
  { service: 'notification-worker', ip: '10.0.3.18', host: 'i-0a1b2c3d4e5f60005', 'availability-zone': 'us-east-1b', kube_namespace: 'shop' },
].map((n) => ({ ...n, env: 'prod', pod_name: `${n.service}-7d9f8c`, container_name: n.service, short_image: n.service }));

// client, server (service or bare address), port, bytes sent / received per 30 s, connections per 30 s, RTT µs
const EDGES: [string, string, number, number, number, number, number][] = [
  ['api-gateway', 'storefront-api', 8080, 2.4e6, 9.6e6, 40, 380],
  ['storefront-api', 'catalog-service', 8080, 0.9e6, 6.1e6, 22, 920],
  ['storefront-api', 'order-service', 8080, 1.2e6, 0.8e6, 12, 410],
  ['order-service', 'billing', 8080, 0.4e6, 0.2e6, 8, 450],
  ['order-service', 'inventory-service', 8080, 0.3e6, 0.5e6, 9, 390],
  ['order-service', 'workflow-engine', 8080, 0.2e6, 0.1e6, 4, 1100],
  ['workflow-engine', 'notification-worker', 8080, 0.1e6, 0.05e6, 3, 1050],
  ['order-service', '10.0.5.20', 5432, 1.6e6, 3.9e6, 2, 310],
  ['catalog-service', '10.0.5.20', 5432, 0.7e6, 5.4e6, 2, 330],
  ['inventory-service', '10.0.5.21', 6379, 0.5e6, 0.9e6, 1, 120],
  ['billing', '203.0.113.10', 443, 0.15e6, 0.3e6, 6, 24_000],
  ['notification-worker', '198.51.100.25', 587, 0.6e6, 0.02e6, 3, 31_000],
];

const FACET_KEYS = ['service', 'env', 'availability-zone', 'kube_namespace', 'pod_name', 'host', 'ip', 'port', 'domain'];
const isPublic = (ip: string) => !/^(10\.|192\.168\.|172\.(1[6-9]|2\d|3[01])\.|127\.|100\.(6[4-9]|[7-9]\d|1[01]\d|12[0-7])\.)/.test(ip);

interface Row {
  t: number;
  client: Tags;
  server: Tags;
  sent: number;
  received: number;
  established: number;
  retransmits: number;
  rtt: number;
  external: boolean;
}

function rows(req: FlowRequest, stepMs: number): Row[] {
  const out: Row[] = [];
  for (let t = Math.ceil(req.range.from / stepMs) * stepMs; t <= req.range.to; t += stepMs) {
    const r = rng(Math.floor(t / 30_000));
    const scale = stepMs / 30_000;
    for (const [c, s, port, sent, received, conns, rtt] of EDGES) {
      const client = { ...NODES.find((n) => n.service === c)! };
      const node = NODES.find((n) => n.service === s);
      const server: Tags = node ? { ...node, port: String(port) } : { ip: s, port: String(port) };
      const j = 0.7 + r() * 0.6;
      out.push({
        t,
        client,
        server,
        sent: Math.round(sent * j * scale),
        received: Math.round(received * j * scale),
        established: Math.round(conns * j * scale),
        retransmits: r() < 0.25 ? Math.ceil(r() * 5 * scale) : 0,
        rtt: rtt * (0.8 + r() * 0.4),
        external: !node && isPublic(s),
      });
    }
  }
  return out;
}

const glob = (p: string, v: string) =>
  new RegExp(
    `^${p
      .toLowerCase()
      .replace(/[.+?^${}()|[\]\\]/g, '\\$&')
      .replace(/\*/g, '.*')}$`,
  ).test(v.toLowerCase());

function matches(terms: Term[], row: Row, labels: [string, string], skip?: string): boolean {
  return terms.every((t) => {
    if (t.kind === 'text') return (labels.some((l) => l.toLowerCase().includes(t.text.toLowerCase())) as boolean) !== t.negate;
    if (t.key === skip) return true;
    const m = /^(client|server)_(.+)$/.exec(t.key);
    const sides = m ? [m[1] === 'client' ? row.client : row.server] : [row.client, row.server];
    const key = m ? m[2] : t.key;
    const hit = sides.some((tags) => tags[key] != null && t.values.some((p) => glob(p, tags[key])));
    return hit !== t.negate;
  });
}

export async function networkFlows(req: FlowRequest): Promise<FlowPage> {
  const span = Math.max(1, req.range.to - req.range.from);
  const stepMs = Math.max(30_000, Math.floor(span / 60 / 30_000) * 30_000);
  const cg = req.clientGroupBy || 'service';
  const sg = req.serverGroupBy || 'service';
  const all = rows(req, stepMs);
  const visible = all.filter((r) => ((req.showExternal ?? true) ? true : !r.external) && (req.showUnresolved || (r.client[cg] != null && r.server[sg] != null)));
  const terms = parseQuery(req.query ?? '');
  const label = (tags: Tags, k: string) => tags[k] ?? 'N/A';
  const flows = new Map<string, Flow & { rttSum: number; n: number }>();
  const series = new Map<number, NetworkPoint & { rttSum: number; n: number }>();
  for (const r of visible) {
    const labels: [string, string] = [label(r.client, cg), label(r.server, sg)];
    if (!matches(terms, r, labels)) continue;
    const key = labels.join('\u0000');
    const f =
      flows.get(key) ??
      ({
        client: labels[0],
        server: labels[1],
        clientTags: Object.entries(r.client).map(([k, v]) => `${k}:${v}`),
        serverTags: Object.entries(r.server).map(([k, v]) => `${k}:${v}`),
        bytesSent: 0,
        bytesReceived: 0,
        throughputSent: 0,
        throughputReceived: 0,
        retransmits: 0,
        rttUs: null,
        rttVarUs: null,
        established: 0,
        closed: 0,
        external: r.external,
        rttSum: 0,
        n: 0,
      } as Flow & { rttSum: number; n: number });
    f.bytesSent += r.sent;
    f.bytesReceived += r.received;
    f.retransmits += r.retransmits;
    f.established += r.established;
    f.closed += r.established;
    f.rttSum += r.rtt;
    f.n += 1;
    flows.set(key, f);
    const p = series.get(r.t) ?? { t: r.t, bytesSent: 0, bytesReceived: 0, established: 0, closed: 0, retransmits: 0, rttUs: null, rttVarUs: null, rttSum: 0, n: 0 };
    p.bytesSent += r.sent;
    p.bytesReceived += r.received;
    p.established += r.established;
    p.closed += r.established;
    p.retransmits += r.retransmits;
    p.rttSum += r.rtt;
    p.n += 1;
    series.set(r.t, p);
  }
  const secs = span / 1000;
  const list = [...flows.values()]
    .map(({ rttSum, n, ...f }) => ({ ...f, rttUs: rttSum / n, rttVarUs: (rttSum / n) * 0.12, throughputSent: f.bytesSent / secs, throughputReceived: f.bytesReceived / secs }))
    .sort((a, b) => b.bytesSent + b.bytesReceived - (a.bytesSent + a.bytesReceived));
  const facets = (['client', 'server'] as const).flatMap((side) =>
    FACET_KEYS.map((k) => {
      const key = `${side}_${k}`;
      const counts = new Map<string, number>();
      for (const r of visible) {
        if (!matches(terms, r, [label(r.client, cg), label(r.server, sg)], key)) continue;
        const v = r[side][k];
        if (v != null) counts.set(v, (counts.get(v) ?? 0) + 1);
      }
      return { key, values: [...counts].map(([value, count]) => ({ value, count })).sort((a, b) => b.count - a.count) };
    }),
  );
  return {
    flows: list.slice(0, req.limit),
    total: list.length,
    totalBytes: list.reduce((a, f) => a + f.bytesSent + f.bytesReceived, 0),
    enabled: true,
    stepMs,
    series: [...series.values()].sort((a, b) => a.t - b.t).map(({ rttSum, n, ...p }) => ({ ...p, rttUs: n ? rttSum / n : null, rttVarUs: n ? (rttSum / n) * 0.12 : null })),
    facets,
  };
}

// client, domain, type, requests per 30 s, NXDOMAIN share, response time (µs) — as oc-tracegen sends.
const DNS: [string, string, string, number, number, number][] = [
  ['api-gateway', 'storefront-api.shop.svc.cluster.local', 'A', 120, 0, 450],
  ['storefront-api', 'catalog-service.shop.svc.cluster.local', 'A', 80, 0, 420],
  ['storefront-api', 'order-service.shop.svc.cluster.local', 'A', 60, 0, 430],
  ['order-service', 'billing.payments.svc.cluster.local', 'A', 40, 0, 460],
  ['order-service', 'postgres.internal.example.net', 'A', 12, 0, 1900],
  ['inventory-service', 'inventory-cache.shop.svc.cluster.local', 'A', 20, 0.08, 520],
  ['billing', 'payments.example.net', 'A', 18, 0, 18_000],
  ['workflow-engine', 'legacy-erp.internal.example.net', 'A', 6, 1, 2400],
  ['notification-worker', 'smtp.example.net', 'MX', 3, 0, 23_000],
];
const RESOLVER: Tags = { service: 'coredns', ip: '10.0.4.10', host: 'i-0a1b2c3d4e5f60003', pod_name: 'coredns-7d9f8c', dns_server: 'coredns-7d9f8c', kube_namespace: 'kube-system', port: '53' };

export async function networkDns(req: DnsRequest): Promise<DnsPage> {
  const span = Math.max(1, req.range.to - req.range.from);
  const stepMs = Math.max(30_000, Math.floor(span / 60 / 30_000) * 30_000);
  const cg = req.clientGroupBy || 'service';
  const sg = req.serverGroupBy || 'dns_server';
  const terms = parseQuery(req.query ?? '');
  const flows = new Map<string, DnsFlow & { latency: number }>();
  const series = new Map<number, DnsPoint & { latency: number }>();
  for (let t = Math.ceil(req.range.from / stepMs) * stepMs; t <= req.range.to; t += stepMs) {
    const r = rng(Math.floor(t / 30_000) + 7);
    for (const [c, domain, type, per, nx, latency] of DNS) {
      const client = NODES.find((n) => n.service === c)!;
      const server = { ...RESOLVER, dns_query: domain, dns_query_type: type };
      const row: Row = { t, client, server, sent: 0, received: 0, established: 0, retransmits: 0, rtt: 0, external: false };
      const labels: [string, string] = [client[cg] ?? 'N/A', server[sg as keyof typeof server] ?? 'N/A'];
      if (!matches(terms, row, labels)) continue;
      const requests = Math.round(per * (stepMs / 30_000) * (0.7 + r() * 0.6));
      const nxdomain = Math.round(requests * nx);
      const timeouts = r() < 0.05 ? 1 : 0;
      const key = labels.join('\u0000');
      const f = flows.get(key) ?? {
        client: labels[0],
        server: labels[1],
        clientTags: Object.entries(client).map(([k, v]) => `${k}:${v}`),
        serverTags: Object.entries(server).map(([k, v]) => `${k}:${v}`),
        requests: 0,
        requestsPerSec: 0,
        noerror: 0,
        servfail: 0,
        nxdomain: 0,
        otherErrors: 0,
        timeouts: 0,
        responseTimeUs: null,
        latency: 0,
      };
      f.requests += requests;
      f.nxdomain += nxdomain;
      f.timeouts += timeouts;
      f.noerror += requests - nxdomain - timeouts;
      f.latency += (requests - timeouts) * latency;
      flows.set(key, f);
      const p = series.get(t) ?? { t, requests: 0, errors: 0, timeouts: 0, responseTimeUs: null, latency: 0 };
      p.requests += requests;
      p.errors += nxdomain;
      p.timeouts += timeouts;
      p.latency += (requests - timeouts) * latency;
      series.set(t, p);
    }
  }
  const list = [...flows.values()]
    .map(({ latency, ...f }) => ({ ...f, requestsPerSec: f.requests / (span / 1000), responseTimeUs: f.requests > f.timeouts ? latency / (f.requests - f.timeouts) : null }))
    .sort((a, b) => b.requests - a.requests);
  const facet = (key: string, values: string[]) => ({ key, values: [...new Set(values)].map((value) => ({ value, count: values.filter((v) => v === value).length })) });
  return {
    flows: list.slice(0, req.limit),
    total: list.length,
    enabled: true,
    stepMs,
    series: [...series.values()].map(({ latency, ...p }) => ({ ...p, responseTimeUs: p.requests > p.timeouts ? latency / (p.requests - p.timeouts) : null })),
    facets: [
      facet(
        'client_service',
        DNS.map((d) => d[0]),
      ),
      facet('server_dns_server', [RESOLVER.dns_server]),
      facet(
        'server_dns_query',
        DNS.map((d) => d[1]),
      ),
      facet(
        'server_dns_query_type',
        DNS.map((d) => d[2]),
      ),
    ],
  };
}
