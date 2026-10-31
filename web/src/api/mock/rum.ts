// In-browser Real User Monitoring for mock mode: three fictional applications of the demo shop and
// their sessions, generated on the fly from time slots so any range has events, then searched,
// aggregated and summarized the way the server does (routes/rum.rs).
import { parseQuery } from '../../lib/searchQuery';
import type { AnalyticsItem, AnalyticsReport, AnalyticsRequest, RumApplication, RumApplicationInput, RumBucket, RumEventRow, RumFacet, RumSearchPage, RumSearchRequest, RumSummary, RumSummaryRequest } from '../rum/types';
import { rng } from './generator';

const MIN = 60_000;
const DAY = 24 * 60 * MIN;
const delay = <T>(v: T, ms = 140): Promise<T> => new Promise((r) => setTimeout(() => r(structuredClone(v)), ms));
const fail = (m: string): never => {
  throw new Error(m);
};

const now0 = Date.now();
const app = (id: string, name: string, type: string, tags: string[], ageDays: number, by: [string, string]): RumApplication => ({
  id,
  name,
  type,
  clientToken: `pub${id.replace(/-/g, '')}`.slice(0, 35),
  tags,
  rumEventProcessingState: 'ALL',
  rumEventProcessingModifiedAt: now0 - ageDays * DAY,
  productAnalyticsRetentionState: 'NONE',
  productAnalyticsModifiedAt: now0 - ageDays * DAY,
  createdBy: by[0],
  createdByHandle: by[1],
  updatedBy: by[0],
  updatedByHandle: by[1],
  createdAt: now0 - ageDays * DAY,
  updatedAt: now0 - ageDays * DAY,
});

let apps: RumApplication[] = [
  app('5e1f0000-0000-4000-8000-0000000000a1', 'storefront-web', 'browser', ['team:storefront'], 120, ['Priya Shah', 'priya.shah@example.com']),
  app('5e1f0000-0000-4000-8000-0000000000a2', 'shop-ios', 'ios', ['team:mobile'], 80, ['Daniel Kim', 'daniel.kim@example.com']),
  app('5e1f0000-0000-4000-8000-0000000000a3', 'shop-android', 'android', ['team:mobile'], 75, ['Daniel Kim', 'daniel.kim@example.com']),
  app('5e1f0000-0000-4000-8000-0000000000a4', 'back-office-web', 'browser', ['team:billing'], 20, ['Alex Morgan', 'alex.morgan@example.com']),
];

// ---------------------------------------------------------------- applications

export async function listRumApplications(): Promise<RumApplication[]> {
  return delay([...apps].sort((a, b) => a.name.localeCompare(b.name)));
}

export async function getRumApplication(id: string): Promise<RumApplication> {
  return delay(apps.find((a) => a.id === id) ?? fail('RUM application not found'));
}

const TYPES = ['browser', 'ios', 'android', 'react-native', 'flutter', 'roku', 'electron', 'unity', 'kotlin-multiplatform'];

function check(input: RumApplicationInput, creating: boolean) {
  if ((creating || input.name != null) && !input.name?.trim()) fail('The application name is required');
  if (creating && !input.type) fail('The application type is required');
  if (input.type && !TYPES.includes(input.type)) fail(`Unknown application type '${input.type}'`);
}

export async function createRumApplication(input: RumApplicationInput): Promise<RumApplication> {
  check(input, true);
  const id = crypto.randomUUID();
  const hex = Array.from(crypto.getRandomValues(new Uint8Array(16)), (b) => b.toString(16).padStart(2, '0')).join('');
  const a: RumApplication = { ...app(id, input.name!.trim(), input.type!, input.tags ?? [], 0, ['Alex Morgan', 'alex.morgan@example.com']), clientToken: `pub${hex}` };
  a.createdAt = a.updatedAt = Date.now();
  apps = [...apps, a];
  return delay(a);
}

export async function updateRumApplication(id: string, input: RumApplicationInput): Promise<RumApplication> {
  check(input, false);
  const before = apps.find((a) => a.id === id) ?? fail('RUM application not found');
  const t = Date.now();
  const after: RumApplication = {
    ...before,
    name: input.name?.trim() ?? before.name,
    type: input.type ?? before.type,
    tags: input.tags ?? before.tags,
    rumEventProcessingState: input.rumEventProcessingState ?? before.rumEventProcessingState,
    rumEventProcessingModifiedAt: input.rumEventProcessingState && input.rumEventProcessingState !== before.rumEventProcessingState ? t : before.rumEventProcessingModifiedAt,
    productAnalyticsRetentionState: input.productAnalyticsRetentionState ?? before.productAnalyticsRetentionState,
    updatedAt: t,
  };
  apps = apps.map((a) => (a.id === id ? after : a));
  return delay(after);
}

export async function deleteRumApplication(id: string): Promise<void> {
  if (!apps.some((a) => a.id === id)) fail('RUM application not found');
  apps = apps.filter((a) => a.id !== id);
  return delay(undefined);
}

// ---------------------------------------------------------------- events

interface MockEvent {
  t: number;
  type: string;
  service: string;
  attrs: Record<string, string>;
}

const WEB_PAGES: [string, number, string[]][] = [
  ['/', 620, ['/api/products/featured', '/api/cart']],
  ['/products', 780, ['/api/products?limit=24']],
  ['/products/?', 900, ['/api/products/sku', '/api/reviews']],
  ['/cart', 540, ['/api/cart', '/api/shipping/estimate']],
  ['/checkout', 1180, ['/api/payments/methods', '/api/orders']],
];
const MOBILE_PAGES: [string, number, string[]][] = [
  ['HomeScreen', 480, ['/api/products/featured']],
  ['CatalogScreen', 620, ['/api/products?limit=24']],
  ['ProductScreen', 700, ['/api/products/sku']],
  ['CartScreen', 420, ['/api/cart']],
  ['CheckoutScreen', 950, ['/api/orders']],
];
const BACK_OFFICE: [string, number, string[]][] = [
  ['/invoices', 840, ['/api/invoices']],
  ['/invoices/?', 690, ['/api/invoices/id']],
  ['/refunds', 720, ['/api/refunds']],
];
const BROWSERS: [string, string, string, string][] = [
  ['Chrome', '130.0.0.0', 'Mac OS X', 'Desktop'],
  ['Edge', '130.0.2849.68', 'Windows', 'Desktop'],
  ['Mobile Safari', '17.4', 'iOS', 'Mobile'],
  ['Chrome Mobile', '130.0.0.0', 'Android', 'Mobile'],
  ['Firefox', '131.0', 'Linux', 'Desktop'],
  ['Safari', '17.5', 'Mac OS X', 'Desktop'],
];
const COUNTRIES = ['United States', 'Germany', 'United Kingdom', 'France', 'Canada', 'Spain'];
const USERS: [string, string][] = [
  ['Maya Chen', 'maya.chen@example.com'],
  ['Jonas Weber', 'jonas.weber@example.com'],
  ['Lucía Torres', 'lucia.torres@example.com'],
  ['Arjun Patel', 'arjun.patel@example.com'],
];

const hex = (r: () => number, n: number) => Array.from({ length: n }, () => Math.floor(r() * 16).toString(16)).join('');

/** One session starting at `start`, from its slot's seed. */
function session(seed: number, start: number): MockEvent[] {
  const r = rng(seed);
  const pickApp = r();
  const a = pickApp < 0.62 ? apps[0] : pickApp < 0.78 ? apps[1] : pickApp < 0.92 ? apps[2] : apps[3];
  if (!a || !apps.includes(a) || a.rumEventProcessingState === 'NONE' || a.createdAt > start) return [];
  const mobile = a.type === 'ios' || a.type === 'android';
  const pages = a.name === 'back-office-web' ? BACK_OFFICE : mobile ? MOBILE_PAGES : WEB_PAGES;
  const service = a.name;
  const version = r() < 0.7 ? '2.14.1' : r() < 0.6 ? '2.15.0-beta' : '2.13.4';
  const browser = mobile ? (a.type === 'ios' ? BROWSERS[2] : BROWSERS[3]) : BROWSERS[Math.floor(r() * BROWSERS.length)];
  const user = r() < 0.45 ? USERS[Math.floor(r() * USERS.length)] : null;
  const synthetic = r() < 0.05;
  const sid = `${hex(r, 8)}-${hex(r, 4)}-${hex(r, 4)}-${hex(r, 4)}-${hex(r, 12)}`;
  const base: Record<string, string> = {
    'application.id': a.id,
    'session.id': sid,
    'session.type': synthetic ? 'synthetics' : 'user',
    'session.has_replay': String(!mobile && r() < 0.5),
    service,
    env: 'prod',
    version,
    'browser.name': mobile ? '' : browser[0],
    'browser.version': mobile ? '' : browser[1],
    'os.name': browser[2],
    'device.type': browser[3],
    'device.name': mobile ? (a.type === 'ios' ? 'iPhone' : 'Pixel 8') : '',
    'geo.country': COUNTRIES[Math.floor(r() * COUNTRIES.length)],
    ...(user ? { 'usr.id': `u-${1000 + USERS.indexOf(user)}`, 'usr.name': user[0], 'usr.email': user[1] } : {}),
  };
  for (const k of Object.keys(base)) if (!base[k]) delete base[k];
  const referrer = r() < 0.45 ? '' : (['https://search.example.com/', 'https://social.example.net/', 'https://news.example.org/'][Math.floor(r() * 3)] ?? '');
  const campaign = r() < 0.2 ? (['newsletter', 'spring-sale', 'partner'][Math.floor(r() * 3)] ?? '') : '';
  const out: MockEvent[] = [];
  let t = start;
  const views = 2 + Math.floor(r() * 6);
  let page = 0;
  for (let v = 0; v < views; v++) {
    const [name, loading, apis] = pages[page];
    const vid = hex(r, 16);
    const dur = 8_000 + Math.floor(r() * 35_000);
    const slow = (mobile ? 1.3 : 1) * (0.6 + r() * 0.9);
    const view: Record<string, string> = {
      ...base,
      'view.id': vid,
      'view.name': name,
      'view.url_path': name.replace('?', 'sku-1042'),
      'view.url': `https://shop.example.com${name.replace('?', 'sku-1042')}${v === 0 && campaign ? `?utm_source=${campaign}` : ''}`,
    };
    if (v === 0 && referrer) view['view.referrer'] = referrer;
    let errors = 0;
    let actions = 0;
    for (const api of apis) {
      if (r() < 0.7) {
        const failed = api === '/api/orders' && r() < 0.18;
        out.push({
          t: t + Math.floor(r() * dur * 0.4),
          type: 'resource',
          service,
          attrs: {
            ...view,
            type: 'resource',
            'resource.type': mobile ? 'native' : 'fetch',
            'resource.method': api === '/api/orders' ? 'POST' : 'GET',
            'resource.url': `https://api.example.com${api}`,
            'resource.url_host': 'api.example.com',
            'resource.status_code': failed ? '502' : '200',
            'resource.duration': String(Math.round((failed ? 900 : 120) * (0.5 + r()) * 1e6)),
          },
        });
        if (failed) {
          errors++;
          out.push({
            t: t + Math.floor(r() * dur * 0.5),
            type: 'error',
            service,
            attrs: { ...view, type: 'error', 'error.source': 'network', 'error.type': 'Error', 'error.handling': 'unhandled', 'error.message': `XHR error POST https://api.example.com${api}` },
          });
        }
      }
    }
    const nActions = Math.floor(r() * 3);
    for (let i = 0; i < nActions; i++) {
      actions++;
      const rage = name.includes('heckout') && r() < 0.2;
      out.push({
        t: t + Math.floor(r() * dur),
        type: 'action',
        service,
        attrs: {
          ...view,
          type: 'action',
          'action.type': mobile ? 'tap' : 'click',
          'action.target.name': name.includes('heckout') ? 'Place order' : name.includes('art') ? 'Proceed to checkout' : 'Add to cart',
          ...(rage ? { 'action.frustration.type': 'rage_click' } : {}),
        },
      });
    }
    if (name.includes('heckout') && r() < 0.08) {
      errors++;
      out.push({
        t: t + Math.floor(r() * dur),
        type: 'error',
        service,
        attrs: {
          ...view,
          type: 'error',
          'error.source': 'source',
          'error.type': 'TypeError',
          'error.handling': 'unhandled',
          'error.message': "Cannot read properties of undefined (reading 'shippingAddress')",
        },
      });
    }
    view['view.frustration.count'] = '0';
    if (r() < 0.15) out.push({ t: t + Math.floor(r() * dur), type: 'long_task', service, attrs: { ...view, type: 'long_task', 'long_task.duration': String(Math.round((60 + r() * 140) * 1e6)) } });
    const ms = loading * slow;
    out.push({
      t,
      type: 'view',
      service,
      attrs: {
        ...view,
        type: 'view',
        '_dd.document_version': String(3 + Math.floor(r() * 4)),
        'view.loading_time': String(Math.round(ms * 1e6)),
        'view.largest_contentful_paint': String(Math.round(ms * 0.82 * 1e6)),
        'view.interaction_to_next_paint': String(Math.round((40 + r() * (name.includes('heckout') ? 380 : 160)) * 1e6)),
        'view.cumulative_layout_shift': (r() * (name.includes('roduct') ? 0.22 : 0.06)).toFixed(3),
        'view.time_spent': String(dur * 1e6),
        'view.error.count': String(errors),
        'view.action.count': String(actions),
      },
    });
    t += dur;
    page = Math.min(page + 1, pages.length - 1);
    if (r() < 0.25) page = Math.floor(r() * 3) % pages.length;
  }
  return out;
}

// Sessions by start time, for the applications as they are (cleared when they change).
let cache = new Map<number, MockEvent[]>();
let cachedFor: RumApplication[] = apps;
function cachedSession(start: number): MockEvent[] {
  if (cachedFor !== apps) {
    cache = new Map();
    cachedFor = apps;
  }
  let evs = cache.get(start);
  if (!evs) {
    evs = session(Math.floor(start / 1000) ^ 0x5eed, start);
    if (cache.size > 20_000) cache.clear();
    cache.set(start, evs);
  }
  return evs;
}

/** The events of the sessions that started in the range (one slot per 2 minutes, wider for long ranges). */
function eventsIn(from: number, to: number): MockEvent[] {
  const slot = Math.max(2 * MIN, Math.ceil((to - from) / 720 / MIN) * MIN);
  const out: MockEvent[] = [];
  const end = Math.min(to, Date.now());
  for (let s = Math.ceil(from / slot) * slot; s <= end; s += slot) {
    const per = 1 + (Math.floor(s / slot) % 3);
    for (let i = 0; i < per; i++) {
      const start = s + i * Math.floor(slot / per);
      for (const e of cachedSession(start)) if (e.t >= from && e.t <= end) out.push(e);
    }
  }
  return out;
}

function like(actual: string, pattern: string): boolean {
  if (!pattern.includes('*')) return actual.toLowerCase() === pattern.toLowerCase();
  const re = new RegExp(
    `^${pattern
      .split('*')
      .map((p) => p.replace(/[.*+?^${}()|[\]\\]/g, '\\$&'))
      .join('.*')}$`,
    'i',
  );
  return re.test(actual);
}

/** The RUM search syntax over one event (the server's RumSchema). */
function matches(e: MockEvent, query: string): boolean {
  return parseQuery(query).every((t) => {
    let hit: boolean;
    if (t.kind === 'text') {
      const text = t.text.toLowerCase();
      hit = Object.values(e.attrs).some((v) => v.toLowerCase().includes(text));
    } else {
      const key = t.key.replace(/^@/, '');
      const actual = key === 'type' ? e.type : key === 'service' ? e.service : (e.attrs[key] ?? '');
      hit = t.values.some((v) => like(actual, v));
    }
    return t.negate ? !hit : hit;
  });
}

function scoped(req: RumSearchRequest): MockEvent[] {
  return eventsIn(req.range.from, req.range.to).filter((e) => (!req.applicationId || e.attrs['application.id'] === req.applicationId) && matches(e, req.query ?? ''));
}

function bucketMs(from: number, to: number): number {
  const target = (to - from) / 60;
  return [10_000, 30_000, 60_000, 120_000, 300_000, 600_000, 1_800_000, 3_600_000, 10_800_000, 21_600_000].find((v) => v >= target) ?? 43_200_000;
}

function buckets(times: number[], size: number): RumBucket[] {
  const m = new Map<number, number>();
  for (const t of times) {
    const b = Math.floor(t / size) * size;
    m.set(b, (m.get(b) ?? 0) + 1);
  }
  return [...m.entries()].sort((a, b) => a[0] - b[0]).map(([t, count]) => ({ t, count }));
}

const SESSION_FIELDS = [
  'application.id',
  'session.type',
  'session.has_replay',
  'usr.id',
  'usr.name',
  'usr.email',
  'device.type',
  'device.name',
  'browser.name',
  'browser.version',
  'os.name',
  'geo.country',
  'env',
  'version',
];

function sessions(events: MockEvent[]): RumEventRow[] {
  const by = new Map<string, MockEvent[]>();
  for (const e of events) {
    const id = e.attrs['session.id'];
    if (id) by.set(id, [...(by.get(id) ?? []), e]);
  }
  return [...by.entries()].map(([id, evs]) => {
    evs.sort((a, b) => a.t - b.t);
    const views = evs.filter((e) => e.type === 'view');
    const first = evs[0];
    const last = evs[evs.length - 1];
    const attributes: Record<string, string> = {};
    for (const f of SESSION_FIELDS) if (first.attrs[f]) attributes[f] = first.attrs[f];
    Object.assign(attributes, {
      'session.id': id,
      'session.time_spent': String(last.t - first.t),
      'session.view.count': String(new Set(views.map((v) => v.attrs['view.id'])).size),
      'session.error.count': String(evs.filter((e) => e.type === 'error').length),
      'session.action.count': String(evs.filter((e) => e.type === 'action').length),
      'session.frustration.count': String(evs.filter((e) => e.type === 'action' && e.attrs['action.frustration.type']).length),
      'session.initial_view.name': views[0]?.attrs['view.name'] ?? '',
      'session.last_view.name': views[views.length - 1]?.attrs['view.name'] ?? '',
      'session.last_event_at': String(last.t),
    });
    return { id, timestamp: first.t, type: 'session', service: first.service, attributes };
  });
}

export async function searchRumEvents(req: RumSearchRequest): Promise<RumSearchPage> {
  const kind = req.eventType;
  const limit = Math.min(req.limit ?? 100, 1000);
  const offset = req.offset ?? 0;
  const size = bucketMs(req.range.from, req.range.to);
  const all = scoped(req);
  let rows: RumEventRow[];
  if (kind === 'session') {
    // Sessions with a matching event, summed up from all their events (as the server does), then the session filters.
    const hit = new Set(all.map((e) => e.attrs['session.id']));
    const inApp = eventsIn(req.range.from, req.range.to).filter((e) => (!req.applicationId || e.attrs['application.id'] === req.applicationId) && hit.has(e.attrs['session.id']));
    const f = req.sessions;
    rows = sessions(inApp).filter((r) => {
      if (!f) return true;
      const a = r.attributes;
      if (f.withReplay && a['session.has_replay'] !== 'true') return false;
      if (f.minTimeSpentMs != null && Number(a['session.time_spent']) < f.minTimeSpentMs) return false;
      if (f.minActionCount != null && Number(a['session.action.count']) < f.minActionCount) return false;
      const active = Date.now() - Number(a['session.last_event_at']) < 15 * MIN;
      if (f.active != null && active !== f.active) return false;
      if (f.ids?.length && !f.ids.includes(r.id)) return false;
      return true;
    });
  } else {
    const evs = kind === 'all' ? all : all.filter((e) => e.type === kind);
    rows = evs.map((e, i) => ({ id: e.type === 'view' ? e.attrs['view.id'] : `${e.t}-${i}`, timestamp: e.t, type: e.type, service: e.service, attributes: { ...e.attrs } }));
  }
  rows.sort((a, b) => (req.ascending ? a.timestamp - b.timestamp : b.timestamp - a.timestamp));
  return delay({
    rows: rows.slice(offset, offset + limit),
    total: rows.length,
    buckets: buckets(
      rows.map((r) => r.timestamp),
      size,
    ),
    bucketMs: size,
  });
}

const COMMON_FACETS = [
  'application.id',
  'service',
  'env',
  'version',
  'session.type',
  'usr.name',
  'usr.id',
  'usr.email',
  'device.type',
  'device.name',
  'device.model',
  'browser.name',
  'os.name',
  'geo.country',
  'geo.city',
  'view.name',
  'view.url_path',
];
const TYPE_FACETS: Record<string, string[]> = {
  error: ['error.source', 'error.type', 'error.handling', 'error.is_crash'],
  action: ['action.type', 'action.target.name', 'action.frustration.type'],
  resource: ['resource.type', 'resource.method', 'resource.status_code', 'resource.url_host'],
  long_task: ['long_task.entry_type'],
  vital: ['vital.name', 'vital.type'],
};

export async function rumFacets(req: RumSearchRequest): Promise<RumFacet[]> {
  const kind = req.eventType;
  const evs = scoped(req).filter((e) => kind === 'all' || kind === 'session' || e.type === kind);
  const paths = [...COMMON_FACETS, ...(TYPE_FACETS[kind] ?? [])];
  return delay(
    paths.map((path) => {
      // Sessions count each session once.
      const seen = new Map<string, Set<string>>();
      const m = new Map<string, number>();
      for (const e of evs) {
        const v = path === 'service' ? e.service : e.attrs[path];
        if (!v) continue;
        if (kind === 'session') {
          const s = seen.get(v) ?? new Set<string>();
          s.add(e.attrs['session.id']);
          seen.set(v, s);
          m.set(v, s.size);
        } else m.set(v, (m.get(v) ?? 0) + 1);
      }
      return {
        path,
        values: [...m.entries()]
          .sort((a, b) => b[1] - a[1])
          .slice(0, 20)
          .map(([value, count]) => ({ value, count })),
      };
    }),
  );
}

const p75 = (xs: number[]) => {
  if (!xs.length) return null;
  const s = [...xs].sort((a, b) => a - b);
  return s[Math.min(s.length - 1, Math.floor(0.75 * s.length))];
};

export async function rumSummary(req: RumSummaryRequest): Promise<RumSummary> {
  const evs = eventsIn(req.range.from, req.range.to).filter((e) => e.attrs['application.id'] === req.applicationId);
  const views = evs.filter((e) => e.type === 'view');
  const errors = evs.filter((e) => e.type === 'error');
  const sessionIds = new Set(evs.map((e) => e.attrs['session.id']));
  const errorSessions = new Set(errors.map((e) => e.attrs['session.id']));
  const num = (k: string, scale = 1e6) => views.map((v) => Number(v.attrs[k]) / scale).filter((x) => Number.isFinite(x) && x > 0);
  const byView = new Map<string, { views: number; errors: number }>();
  for (const v of views) {
    const x = byView.get(v.attrs['view.name']) ?? { views: 0, errors: 0 };
    x.views++;
    byView.set(v.attrs['view.name'], x);
  }
  for (const e of errors) {
    const x = byView.get(e.attrs['view.name']) ?? { views: 0, errors: 0 };
    x.errors++;
    byView.set(e.attrs['view.name'], x);
  }
  const count = <T>(items: T[], key: (t: T) => string, distinct?: (t: T) => string) => {
    const m = new Map<string, Set<string> | number>();
    for (const it of items) {
      const k = key(it);
      if (!k) continue;
      if (distinct) (m.get(k) as Set<string> | undefined)?.add(distinct(it)) ?? m.set(k, new Set([distinct(it)]));
      else m.set(k, ((m.get(k) as number) ?? 0) + 1);
    }
    return [...m.entries()].map(([name, v]) => ({ name, count: typeof v === 'number' ? v : v.size })).sort((a, b) => b.count - a.count);
  };
  const size = bucketMs(req.range.from, req.range.to);
  const firsts = new Map<string, number>();
  for (const e of evs) firsts.set(e.attrs['session.id'], Math.min(firsts.get(e.attrs['session.id']) ?? Infinity, e.t));
  const minutes = Math.max(1, (req.range.to - req.range.from) / MIN);
  const cls = views.map((v) => Number(v.attrs['view.cumulative_layout_shift'])).filter((x) => Number.isFinite(x));
  return delay({
    sessions: sessionIds.size,
    views: views.length,
    errors: errors.length,
    errorRate: errors.length / minutes,
    crashFreeSessions: sessionIds.size ? (100 * (sessionIds.size - errorSessions.size)) / sessionIds.size : null,
    loadingTimeP75: p75(num('view.loading_time')),
    largestContentfulPaintP75: p75(num('view.largest_contentful_paint')),
    interactionToNextPaintP75: p75(num('view.interaction_to_next_paint')),
    cumulativeLayoutShiftP75: p75(cls),
    topErrorViews: [...byView.entries()]
      .filter(([, x]) => x.errors > 0)
      .map(([name, x]) => ({ name, count: x.errors, value: x.views ? (100 * x.errors) / x.views : 0 }))
      .sort((a, b) => b.value - a.value)
      .slice(0, 10),
    versions: count(
      evs,
      (e) => e.attrs.version,
      (e) => e.attrs['session.id'],
    )
      .slice(0, 10)
      .map((x) => ({ ...x, value: x.count })),
    topViews: count(views, (v) => v.attrs['view.name'])
      .slice(0, 10)
      .map((x) => ({ ...x, value: x.count })),
    sessionsOverTime: buckets([...firsts.values()], size),
    bucketMs: size,
  });
}

// ---------------------------------------------------------------- product analytics

const uid = (e: MockEvent) => e.attrs['usr.id'] ?? e.attrs['usr.anonymous_id'] ?? e.attrs['session.id'];

function kpis(evs: MockEvent[]) {
  const views = evs.filter((e) => e.type === 'view');
  const bySession = new Map<string, MockEvent[]>();
  for (const e of evs) bySession.set(e.attrs['session.id'], [...(bySession.get(e.attrs['session.id']) ?? []), e]);
  const sessions = [...bySession.values()];
  const viewCount = (ss: MockEvent[]) => new Set(ss.filter((e) => e.type === 'view').map((e) => e.attrs['view.id'])).size;
  const avg = (xs: number[]) => (xs.length ? xs.reduce((a, b) => a + b, 0) / xs.length : NaN);
  const loads = views.map((v) => Number(v.attrs['view.loading_time']) / 1e6).filter((x) => x > 0);
  const frustratedViews = new Set(evs.filter((e) => e.type === 'action' && e.attrs['action.frustration.type']).map((e) => e.attrs['view.id']));
  return {
    users: new Set(evs.map(uid)).size,
    sessions: sessions.length,
    views: views.length,
    load: p75(loads) ?? NaN,
    duration: avg(sessions.map((ss) => Math.max(...ss.map((e) => e.t)) - Math.min(...ss.map((e) => e.t)))),
    bounce: sessions.length ? (100 * sessions.filter((ss) => viewCount(ss) === 1).length) / Math.max(1, sessions.filter((ss) => viewCount(ss) >= 1).length) : NaN,
    withErrors: sessions.length ? (100 * sessions.filter((ss) => ss.some((e) => e.type === 'error')).length) / sessions.length : NaN,
    timeOnPage: avg(views.map((v) => Number(v.attrs['view.time_spent']) / 1e6).filter((x) => x > 0)),
    versions: new Set(evs.map((e) => e.attrs.version).filter(Boolean)).size,
    desktop: sessions.length ? (100 * sessions.filter((ss) => ss[0].attrs['device.type'] === 'Desktop').length) / sessions.length : NaN,
    issues: new Set(evs.filter((e) => e.type === 'error').map((e) => e.attrs['error.message'])).size,
    frustratedViews: views.length ? (100 * views.filter((v) => frustratedViews.has(v.attrs['view.id'])).length) / views.length : NaN,
  };
}

const val = (now: number, before?: number) => ({
  value: Number.isFinite(now) ? now : null,
  change: before !== undefined && before !== 0 && Number.isFinite(before) && Number.isFinite(now) ? (100 * (now - before)) / before : null,
});

function top(evs: MockEvent[], name: (e: MockEvent) => string | undefined, distinct: (e: MockEvent) => string, n = 10): AnalyticsItem[] {
  const m = new Map<string, Set<string>>();
  for (const e of evs) {
    const k = name(e);
    if (!k) continue;
    m.set(k, (m.get(k) ?? new Set()).add(distinct(e)));
  }
  return [...m.entries()]
    .map(([k, v]) => ({ name: k, value: v.size }))
    .sort((a, b) => b.value - a.value)
    .slice(0, n);
}

export async function productAnalyticsReport(req: AnalyticsRequest): Promise<AnalyticsReport> {
  const { from, to } = req.range;
  const span = to - from;
  const scoped = (a: number, b: number) => eventsIn(a, b).filter((e) => !req.applicationId || e.attrs['application.id'] === req.applicationId);
  const evs = scoped(from, to);
  const k = kpis(evs);
  const p = kpis(scoped(from - span, from));
  const ratio = (a: number, b: number) => (b ? a / b : NaN);
  const empty: AnalyticsReport = {
    activeUsers: val(k.users, p.users),
    sessions: val(k.sessions, p.sessions),
    loadTime: val(k.load, p.load),
    sessionDuration: val(k.duration, p.duration),
    bounceRate: val(k.bounce, p.bounce),
    sessionsWithErrors: val(k.withErrors, p.withErrors),
    viewsPerUser: val(ratio(k.views, k.users), ratio(p.views, p.users)),
    sessionsPerUser: val(ratio(k.sessions, k.users), ratio(p.sessions, p.users)),
    viewsPerSession: val(ratio(k.views, k.sessions), ratio(p.views, p.sessions)),
    timeOnPage: val(k.timeOnPage, p.timeOnPage),
    versionsInUse: val(k.versions),
    desktop: val(k.desktop),
    issueCount: val(k.issues, p.issues),
    viewsWithFrustration: val(k.frustratedViews),
    newUsers: val(NaN),
    returningUsers: val(NaN),
    usersOnline: val(NaN),
    overTime: [],
    bucketMs: 0,
    countries: [],
    powerUsers: [],
    versions: [],
    browsers: [],
    sources: [],
    campaigns: [],
    popularViews: [],
    frustrationByView: [],
    issues: [],
  };
  if (!k.users) return delay(empty);
  const r = empty;
  const size = span >= 2 * DAY ? DAY : bucketMs(from, to);
  r.bucketMs = size;
  const views = evs.filter((e) => e.type === 'view');
  if (req.page === 'home' || req.page === 'users') {
    const m = new Map<number, Set<string>>();
    for (const e of evs) {
      const b = Math.floor(e.t / size) * size;
      m.set(b, (m.get(b) ?? new Set()).add(uid(e)));
    }
    r.overTime = [...m.entries()].sort((a, b) => a[0] - b[0]).map(([t, v]) => ({ t, count: v.size }));
  }
  if (req.page === 'traffic') r.overTime = buckets(views.map((v) => v.t), size);
  if (req.page === 'users') {
    const before = new Set(scoped(from - 3 * DAY, from).map(uid));
    const now = new Set(evs.map(uid));
    r.returningUsers = val([...now].filter((u) => before.has(u)).length);
    r.newUsers = val([...now].filter((u) => !before.has(u)).length);
    r.countries = top(evs, (e) => e.attrs['geo.country'], uid);
    r.powerUsers = top(views, (e) => e.attrs['usr.id'] ?? e.attrs['session.id'], (e) => e.attrs['view.id']);
  }
  if (req.page === 'devices') {
    r.versions = top(evs, (e) => e.attrs.version, uid);
    r.browsers = top(evs, (e) => e.attrs['browser.name'] ?? e.attrs['os.name'], uid);
  }
  if (req.page === 'traffic') {
    r.sources = top(views, (e) => (e.attrs['view.referrer'] ? new URL(e.attrs['view.referrer']).hostname : 'Direct'), (e) => e.attrs['view.id']);
    r.campaigns = top(views, (e) => new URL(e.attrs['view.url']).searchParams.get('utm_source') ?? undefined, (e) => e.attrs['view.id']);
  }
  if (req.page === 'engagement') {
    const recent = Date.now() - 5 * MIN;
    r.usersOnline = val(new Set(scoped(recent, Date.now()).map(uid)).size);
    r.popularViews = top(views, (e) => e.attrs['view.name'], (e) => e.attrs['view.id']);
  }
  if (req.page === 'performance') {
    const byView = new Map<string, { views: Set<string>; frustrations: number }>();
    for (const e of evs) {
      const x = byView.get(e.attrs['view.name']) ?? { views: new Set(), frustrations: 0 };
      if (e.type === 'view') x.views.add(e.attrs['view.id']);
      if (e.type === 'action' && e.attrs['action.frustration.type']) x.frustrations++;
      byView.set(e.attrs['view.name'], x);
    }
    r.frustrationByView = [...byView.entries()]
      .map(([name, x]) => ({ name, value: x.frustrations / Math.max(1, x.views.size) }))
      .filter((x) => x.value > 0)
      .sort((a, b) => b.value - a.value)
      .slice(0, 12);
    const issues = new Map<string, number>();
    for (const e of evs.filter((e) => e.type === 'error')) issues.set(e.attrs['error.message'], (issues.get(e.attrs['error.message']) ?? 0) + 1);
    r.issues = [...issues.entries()].map(([name, value]) => ({ name, value })).sort((a, b) => b.value - a.value);
  }
  return delay(r);
}

/** A session's events around its start (sessions last well under half an hour), for Session Replay. */
export function sessionEvents(id: string, startedAt: number): { t: number; type: string; service: string; attrs: Record<string, string> }[] {
  return eventsIn(startedAt - 30 * MIN, startedAt + 30 * MIN)
    .filter((e) => e.attrs['session.id'] === id)
    .sort((a, b) => a.t - b.t);
}

export { sessions as sessionRows, eventsIn as mockEventsIn };
