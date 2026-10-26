// In-browser continuous profiles for mock mode: the e-commerce demo services, with the same code
// paths the synthetic profiler in oc-tracegen uploads, so mock and live look alike.
import type {
  FlameGraph,
  FlameGraphRequest,
  FlameNode,
  InsightStatus,
  ProfileInsightList,
  ProfileInsightRequest,
  ProfileList,
  ProfileMetric,
  ProfileOverview,
  ProfileOverviewRequest,
  ProfileSearchRequest,
} from '../profiles/client';
import { parseQuery } from '../../lib/searchQuery';
import { rng } from './generator';

type Path = [frames: string[], endpoint: string, share: number];

interface Svc {
  service: string;
  language: string;
  version: string;
  host: string;
  cores: number;
  heapMiB: number;
  paths: Path[];
}

const JETTY = [
  'java.lang.Thread.run@Thread.java',
  'org.eclipse.jetty.util.thread.QueuedThreadPool$Runner.run@QueuedThreadPool.java',
  'org.springframework.web.servlet.DispatcherServlet.doDispatch@DispatcherServlet.java',
];
const KESTREL = ['System.Threading.ThreadPoolWorkQueue.Dispatch@', 'Microsoft.AspNetCore.Server.Kestrel.Core.Internal.Http.HttpProtocol.ProcessRequests@'];
const WORKER = ['<module>@/app/worker.py', 'run@/app/worker.py'];
const KOMBU = 'consume@/usr/local/lib/python3.12/site-packages/kombu/messaging.py';
const GOHTTP = ['runtime.goexit@runtime/asm_amd64.s', 'net/http.(*conn).serve@net/http/server.go', 'github.com/example-org/api-gateway/proxy.(*Router).ServeHTTP@proxy/router.go'];

const SERVICES: Svc[] = [
  {
    service: 'storefront-api',
    language: 'jvm',
    version: '2.14.0',
    host: 'i-0a1b2c3d4e5f60002',
    cores: 0.9,
    heapMiB: 610,
    paths: [
      [
        [
          ...JETTY,
          'com.example.storefront.cart.CartController.getCart@CartController.java',
          'com.example.storefront.cart.CartService.load@CartService.java',
          'com.example.storefront.cart.PriceCalculator.total@PriceCalculator.java',
        ],
        'GET /cart',
        0.24,
      ],
      [
        [
          ...JETTY,
          'com.example.storefront.cart.CartController.getCart@CartController.java',
          'com.example.storefront.cart.CartService.load@CartService.java',
          'com.fasterxml.jackson.databind.ObjectMapper.readValue@ObjectMapper.java',
        ],
        'GET /cart',
        0.12,
      ],
      [
        [
          ...JETTY,
          'com.example.storefront.checkout.CheckoutController.checkout@CheckoutController.java',
          'com.example.storefront.checkout.OrderClient.submit@OrderClient.java',
          'com.fasterxml.jackson.databind.ObjectMapper.writeValueAsBytes@ObjectMapper.java',
        ],
        'POST /checkout',
        0.16,
      ],
      [
        [
          ...JETTY,
          'com.example.storefront.checkout.CheckoutController.checkout@CheckoutController.java',
          'com.example.storefront.checkout.PromotionEngine.apply@PromotionEngine.java',
          'java.util.regex.Pattern$Start.match@Pattern.java',
        ],
        'POST /checkout',
        0.14,
      ],
      [[...JETTY, 'com.example.storefront.catalog.ProductController.search@ProductController.java', 'com.example.storefront.catalog.SearchIndex.query@SearchIndex.java'], 'GET /products', 0.18],
      [['java.lang.Thread.run@Thread.java', 'com.zaxxer.hikari.pool.HouseKeeper.run@HouseKeeper.java'], '', 0.04],
      [['java.lang.Thread.run@Thread.java', 'G1 Young Generation@'], '', 0.12],
    ],
  },
  {
    service: 'api-gateway',
    language: 'go',
    version: '3.2.0',
    host: 'i-0a1b2c3d4e5f60001',
    cores: 0.45,
    heapMiB: 70,
    paths: [
      [[...GOHTTP, 'github.com/example-org/api-gateway/auth.Verify@auth/jwt.go', 'crypto/ecdsa.Verify@crypto/ecdsa/ecdsa.go'], 'GET /api/*', 0.3],
      [[...GOHTTP, 'net/http/httputil.(*ReverseProxy).ServeHTTP@net/http/httputil/reverseproxy.go', 'io.copyBuffer@io/io.go'], 'GET /api/*', 0.32],
      [[...GOHTTP, 'github.com/example-org/api-gateway/ratelimit.(*Limiter).Allow@ratelimit/limiter.go'], 'POST /api/*', 0.14],
      [['runtime.goexit@runtime/asm_amd64.s', 'runtime.gcBgMarkWorker@runtime/mgc.go', 'runtime.scanobject@runtime/mgcmark.go'], '', 0.18],
      [['runtime.goexit@runtime/asm_amd64.s', 'runtime.bgsweep@runtime/mgcsweep.go'], '', 0.06],
    ],
  },
  {
    service: 'catalog-service',
    language: 'dotnet',
    version: '1.8.3',
    host: 'i-0a1b2c3d4e5f60004',
    cores: 0.35,
    heapMiB: 240,
    paths: [
      [
        [
          ...KESTREL,
          'Example.Catalog.Controllers.ProductsController.Get@ProductsController.cs',
          'Example.Catalog.Data.ProductRepository.FindAsync@ProductRepository.cs',
          'Npgsql.NpgsqlDataReader.Read@',
        ],
        'GET /api/products/{id}',
        0.34,
      ],
      [[...KESTREL, 'Example.Catalog.Controllers.ProductsController.List@ProductsController.cs', 'System.Text.Json.JsonSerializer.Serialize@'], 'GET /api/products', 0.38],
      [['System.Threading.ThreadPoolWorkQueue.Dispatch@', 'Example.Catalog.Indexing.IndexRefresher.Run@IndexRefresher.cs', 'Example.Catalog.Indexing.Tokenizer.Split@Tokenizer.cs'], '', 0.18],
      [['System.GC.Collect@'], '', 0.1],
    ],
  },
  {
    service: 'notification-worker',
    language: 'python',
    version: '0.9.1',
    host: 'i-0a1b2c3d4e5f60005',
    cores: 0.12,
    heapMiB: 95,
    paths: [
      [[...WORKER, KOMBU, 'handle_message@/app/handlers.py', 'render_email@/app/templates.py', 'render@/usr/local/lib/python3.12/site-packages/jinja2/environment.py'], '', 0.36],
      [[...WORKER, KOMBU, 'handle_message@/app/handlers.py', 'send@/app/smtp_client.py', 'sendall@/usr/local/lib/python3.12/ssl.py'], '', 0.22],
      [[...WORKER, KOMBU, 'decode@/usr/local/lib/python3.12/site-packages/kombu/serialization.py', 'loads@/usr/local/lib/python3.12/json/__init__.py'], '', 0.18],
      [[...WORKER, 'heartbeat@/usr/local/lib/python3.12/site-packages/kombu/connection.py'], '', 0.08],
      [['<module>@/app/worker.py', 'serve@/app/health.py', 'handle@/usr/local/lib/python3.12/http/server.py'], 'GET /healthz', 0.16],
    ],
  },
];

const METRICS: ProfileMetric[] = [
  { type: 'cpu-time', unit: 'nanoseconds', label: 'CPU Time', category: 'EXECUTION', gauge: false },
  { type: 'alloc-space', unit: 'bytes', label: 'Allocated Memory', category: 'MEMORY', gauge: false },
  { type: 'heap-space', unit: 'bytes', label: 'Heap Live Size', category: 'MEMORY', gauge: true },
  { type: 'wall-time', unit: 'nanoseconds', label: 'Wall Time', category: 'ELAPSED TIME', gauge: false },
];

const MINUTE = 60_000;

/** A service's value for one minute: CPU, wall and allocations per second (ns or bytes), heap in bytes. */
function perSecond(svc: Svc, type: string, minute: number): number {
  const r = rng(minute * 31 + svc.service.length);
  const burst = svc.service === 'storefront-api' && minute % 17 === 5 ? 4 : 1;
  const cores = svc.cores * burst * (0.85 + r() * 0.3);
  if (type === 'heap-space') return svc.heapMiB * 1048576 * (0.95 + r() * 0.1);
  if (type === 'wall-time') return cores * 2.6e9;
  if (type === 'alloc-space') return cores * 0.8e9;
  return cores * 1e9;
}

function matching(query: string): Svc[] {
  const terms = parseQuery(query);
  return SERVICES.filter((s) =>
    terms.every((t) => {
      if (t.kind !== 'tag') return true;
      const v = t.key === 'service' ? s.service : t.key === 'env' ? 'prod' : t.key === 'version' ? s.version : t.key === 'host' ? s.host : t.key === 'language' ? s.language : null;
      if (v == null) return true;
      return t.values.includes(v) !== t.negate;
    }),
  );
}

export async function profileOverview(req: ProfileOverviewRequest): Promise<ProfileOverview> {
  const svcs = matching(req.query ?? '');
  const metric = METRICS.find((m) => m.type === (req.sampleType || 'cpu-time')) ?? METRICS[0];
  const stepMs = Math.max(MINUTE, Math.floor((req.range.to - req.range.from) / 60 / MINUTE) * MINUTE);
  const start = Math.ceil(req.range.from / stepMs) * stepMs;
  const series = svcs.map((s) => {
    const points: [number, number][] = [];
    for (let t = start; t <= req.range.to; t += stepMs) points.push([t, perSecond(s, metric.type, Math.floor(t / MINUTE))]);
    return { service: s.service, points };
  });
  const services = series
    .map((s) => ({ service: s.service, language: SERVICES.find((x) => x.service === s.service)!.language, value: s.points.reduce((a, p) => a + p[1], 0) / Math.max(1, s.points.length) }))
    .sort((a, b) => b.value - a.value);
  const count = Math.round((req.range.to - req.range.from) / MINUTE);
  const facet = (key: string, values: [string, number][]) => ({ key, values: values.map(([value, c]) => ({ value, count: c })) });
  const all = matching((req.query ?? '').replace(/-?service:\S+/g, ''));
  return {
    metrics: METRICS,
    metric,
    stepMs,
    series,
    services,
    facets: [
      facet('env', [['prod', svcs.length * count]]),
      facet(
        'service',
        all.map((s) => [s.service, s.language === 'go' ? 2 * count : count]),
      ),
      facet(
        'version',
        svcs.map((s) => [s.version, count]),
      ),
      facet(
        'host',
        svcs.map((s) => [s.host, count]),
      ),
    ],
  };
}

function tree(paths: Path[], scale: number, endpoint: string | null | undefined): FlameNode {
  const root: FlameNode = { name: 'all', file: '', value: 0, children: [] };
  for (const [frames, ep, share] of paths) {
    if (endpoint != null && ep !== endpoint) continue;
    const v = Math.round(share * scale);
    root.value += v;
    let node = root;
    for (const f of frames) {
      const [name, file = ''] = f.split('@');
      let child = node.children.find((c) => c.name === name && c.file === file);
      if (!child) {
        child = { name, file, value: 0, children: [] };
        node.children.push(child);
      }
      child.value += v;
      node = child;
    }
  }
  const sort = (n: FlameNode) => {
    n.children.sort((a, b) => b.value - a.value);
    n.children.forEach(sort);
  };
  sort(root);
  return root;
}

function pkg(name: string, file: string): string {
  const m = /(?:site|dist)-packages\/([^/]+)/.exec(file);
  if (m) return m[1].replace(/\.py$/, '');
  if (file.includes('/lib/python')) return 'stdlib';
  if (/\.(py|rb|js|php)$/.test(file)) return '';
  if (name.includes('/')) {
    const i = name.lastIndexOf('/');
    return `${name.slice(0, i)}/${name.slice(i + 1).split('.')[0]}`;
  }
  const parts = name.split('(')[0].split('.');
  return parts.length > 2 ? parts.slice(0, -2).join('.') : '';
}

export async function flameGraph(req: FlameGraphRequest): Promise<FlameGraph> {
  const svc = matching(req.query ?? '')[0];
  const type = req.sampleType || 'cpu-time';
  const metric = METRICS.find((m) => m.type === type) ?? METRICS[0];
  const minutes = Math.max(1, Math.round((req.range.to - req.range.from) / MINUTE));
  const empty = { name: 'all', file: '', value: 0, children: [] };
  if (!svc) return { sampleTypes: [], sampleType: '', unit: '', metric: null, profiles: 0, scale: 1, cards: [], endpoints: [], functions: [], root: empty };
  const avg = (t: string) => perSecond(svc, t, Math.floor(req.range.to / MINUTE) - 3);
  // Totals over the range (per-minute value × minutes); the scale brings them back to per minute.
  const total = metric.gauge ? avg(type) * minutes : avg(type) * 60 * minutes;
  const root = tree(svc.paths, total, req.endpoint);
  const endpoints = new Map<string, number>();
  for (const [, ep, share] of svc.paths) endpoints.set(ep, (endpoints.get(ep) ?? 0) + Math.round(share * total));
  const functions = svc.paths
    .filter(([, ep]) => req.endpoint == null || ep === req.endpoint)
    .map(([frames, , share]) => {
      const [name, file = ''] = frames[frames.length - 1].split('@');
      return { name, file, package: pkg(name, file), value: Math.round(share * total) };
    })
    .sort((a, b) => b.value - a.value);
  return {
    sampleTypes: METRICS.map((m) => ({ type: m.type, unit: m.unit })),
    sampleType: metric.type,
    unit: metric.unit,
    metric,
    profiles: minutes,
    scale: 1 / minutes,
    cards: METRICS.map((m) => ({ metric: m, value: m.gauge ? avg(m.type) : avg(m.type) * 60 })),
    endpoints: [...endpoints].map(([name, value]) => ({ name, value })).sort((a, b) => b.value - a.value),
    functions,
    root,
  };
}

export async function searchProfiles(req: ProfileSearchRequest): Promise<ProfileList> {
  const svcs = matching(req.query ?? '');
  const items = [];
  for (let t = Math.floor(req.range.to / MINUTE) * MINUTE; t > req.range.from && items.length < req.limit; t -= MINUTE) {
    for (const s of svcs) {
      const minute = t / MINUTE;
      items.push({
        start: t - MINUTE,
        end: t,
        service: s.service,
        env: 'prod',
        version: s.version,
        host: s.host,
        language: s.language,
        runtimeId: `${s.service}-0001`,
        attachment: s.language === 'go' ? 'cpu.pprof' : 'auto.pprof',
        totals: Object.fromEntries(METRICS.map((m) => [m.type, Math.round(perSecond(s, m.type, minute) * (m.gauge ? 1 : 60))])),
      });
    }
  }
  return { items: items.slice(0, req.limit), services: SERVICES.map((s) => s.service) };
}

const states = new Map<string, InsightStatus>();

export async function profileInsights(req: ProfileInsightRequest): Promise<ProfileInsightList> {
  const key = 'cpu-burst:storefront-api:prod';
  const visible = (!req.service || req.service === 'storefront-api') && (!req.env || req.env === 'prod');
  const lastBurst = (Math.floor(req.range.to / MINUTE / 17) * 17 + 5) * MINUTE;
  const inRange = lastBurst >= req.range.from && lastBurst <= req.range.to;
  return {
    items:
      visible && (inRange || req.range.to - req.range.from >= 17 * MINUTE)
        ? [
            {
              key,
              title: 'Burst of High CPU Usage',
              severity: 'MEDIUM',
              category: 'CPU',
              service: 'storefront-api',
              env: 'prod',
              language: 'jvm',
              description: 'CPU usage rose to 3.71 cores against a usual 0.90 cores. Short bursts like this suggest the service could use more CPU, or that a hot code path is worth optimizing.',
              status: states.get(key) ?? 'for_review',
              detectedAt: lastBurst > req.range.to ? lastBurst - 17 * MINUTE : lastBurst,
            },
          ]
        : [],
  };
}

export async function setInsightState(key: string, status: InsightStatus): Promise<void> {
  states.set(key, status);
}
