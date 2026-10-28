// In-browser stand-in for the Error Tracking endpoints (see api/errorTracking/client.ts).
// Issues are fixed templates; each gets a deterministic stream of occurrences over the last
// week. Mutations (status, assignee, comments) persist in memory for the session.
import type {
  ActivityEntry,
  AttributeGroup,
  DataSource,
  ErrorSample,
  ErrorSamplePage,
  ErrorSampleRequest,
  Issue,
  IssueDetail,
  IssueFacets,
  IssueFacetsRequest,
  IssuePatch,
  IssueSearchPage,
  IssueSearchRequest,
  IssueStateTab,
  IssueStatus,
  Persona,
  Platform,
  StackFrame,
  SuspectedCause,
  TagDistribution,
  TimelineEvent,
  User,
} from '../errorTracking/types';
import type { Facet, JsonValue, QuerySuggestion, TimeRange } from '../types';
import { parseQuery, type Term } from '../../lib/searchQuery';
import { rng, type Rand } from './generator';
import { matchesTerms } from './match';

const DAY = 24 * 3600_000;
const delay = <T>(value: T, ms = 120 + Math.random() * 180) => new Promise<T>((r) => setTimeout(() => r(value), ms));

// ---------------------------------------------------------------- users

export const USERS: User[] = [
  { id: 'u-1', name: 'Alex Morgan', email: 'alex.morgan@example.com' },
  { id: 'u-2', name: 'Priya Shah', email: 'priya.shah@example.com' },
  { id: 'u-3', name: 'Daniel Kim', email: 'daniel.kim@example.com' },
  { id: 'u-4', name: 'Sara Levi', email: 'sara.levi@example.com' },
  { id: 'u-5', name: 'Omar Haddad', email: 'omar.haddad@example.com' },
];
const CURRENT_USER = USERS[0];

// ---------------------------------------------------------------- issue templates

interface Template {
  errorType: string;
  message: string;
  platform: Platform;
  persona: Exclude<Persona, 'all'>;
  source: DataSource;
  service: string;
  application?: { name: string; packageId?: string };
  location?: { function: string; file: string };
  perDay: number;
  ageDays: number;
  isCrash?: boolean;
  isRegression?: boolean;
  cause: SuspectedCause | null;
  status?: IssueStatus;
  versions: [string, string];
  env?: string;
  team?: string;
}

const SHOP_APP = { name: 'shop-mobile-app', packageId: 'com.example.shop' };

const TEMPLATES: Template[] = [
  {
    errorType: 'SIGSEGV',
    message: 'NDK crash detected with signal: SIGSEGV',
    platform: 'android',
    persona: 'mobile',
    source: 'rum',
    service: 'shop-mobile-app',
    application: SHOP_APP,
    perDay: 38,
    ageDays: 27,
    isCrash: true,
    cause: null,
    versions: ['3.9.0', '3.9.0'],
  },
  {
    errorType: 'io.observecraft.android.rum.anr.ANRException',
    message: 'Application Not Responding',
    platform: 'android',
    persona: 'mobile',
    source: 'rum',
    service: 'shop-mobile-app',
    application: SHOP_APP,
    perDay: 2,
    ageDays: 120,
    isCrash: true,
    isRegression: true,
    cause: null,
    versions: ['3.4.2', '3.9.0'],
  },
  {
    errorType: 'java.lang.IndexOutOfBoundsException',
    message: 'Index X out of bounds for length X',
    platform: 'java',
    persona: 'backend',
    source: 'apm',
    service: 'billing',
    location: { function: 'outOfBounds', file: 'util.Preconditions.java' },
    perDay: 21,
    ageDays: 380,
    isRegression: true,
    cause: 'code_exception',
    versions: ['1.12.0', '1.31.4'],
  },
  {
    errorType: 'java.lang.NullPointerException',
    message: 'Cannot invoke "java.util.Collection.toArray()" because "c" is null',
    platform: 'java',
    persona: 'backend',
    source: 'apm',
    service: 'support-portal',
    location: { function: 'addAll', file: 'base/java.util.ArrayList.java' },
    perDay: 1,
    ageDays: 4,
    cause: 'code_exception',
    versions: ['2.8.1', '2.8.1'],
  },
  {
    errorType: 'Oracle.DataAccess.Client.OracleException',
    message: 'ORA-01422: exact fetch returns more than requested number of rows ORA-06512: at "INVENTORY.UPDATE_STOCK_PKG", line 1538 ORA-06512: at line 1',
    platform: 'dotnet',
    persona: 'backend',
    source: 'logs',
    service: 'catalog-service-oracle',
    location: { function: '…ess.Client.OracleCommand.ExecuteNonQuery', file: '' },
    perDay: 780,
    ageDays: 365,
    cause: null,
    versions: ['4.2.0', '4.6.3'],
  },
  {
    errorType: 'TypeError',
    message: "Cannot read properties of undefined (reading 'map')",
    platform: 'browser',
    persona: 'browser',
    source: 'rum',
    service: 'storefront-web-rum',
    application: { name: 'storefront-web-rum' },
    location: { function: 'renderQuotes', file: 'src/components/CartList.tsx' },
    perDay: 10,
    ageDays: 18,
    cause: 'code_exception',
    versions: ['5.3.0', '5.4.1'],
  },
  {
    errorType: 'ChunkLoadError',
    message: 'Loading chunk 412 failed. (error: https://account.example.com/static/js/412.8c1f.chunk.js)',
    platform: 'browser',
    persona: 'browser',
    source: 'rum',
    service: 'account-web-rum',
    application: { name: 'account-web-rum' },
    perDay: 8,
    ageDays: 9,
    cause: 'lost_connection',
    versions: ['2.1.0', '2.2.0'],
  },
  {
    errorType: 'AxiosError',
    message: 'Network Error',
    platform: 'browser',
    persona: 'browser',
    source: 'rum',
    service: 'gift-cards',
    application: { name: 'gift-cards' },
    location: { function: 'fetchOffers', file: 'src/api/offers.ts' },
    perDay: 7,
    ageDays: 60,
    cause: 'failed_request',
    versions: ['1.0.9', '1.1.2'],
  },
  {
    errorType: 'Error',
    message: 'Request failed with status code 502',
    platform: 'browser',
    persona: 'browser',
    source: 'rum',
    service: 'store-locator-web-rum',
    application: { name: 'store-locator-web-rum' },
    perDay: 2,
    ageDays: 33,
    cause: 'failed_request',
    versions: ['0.9.0', '0.9.4'],
  },
  {
    errorType: 'TypeError',
    message: 'Failed to fetch',
    platform: 'browser',
    persona: 'browser',
    source: 'rum',
    service: 'checkout-web-rum',
    application: { name: 'checkout-web-rum' },
    perDay: 3,
    ageDays: 140,
    cause: 'lost_connection',
    versions: ['3.0.0', '3.2.7'],
  },
  {
    errorType: 'Error',
    message: 'Route not found: /orders/undefined',
    platform: 'browser',
    persona: 'browser',
    source: 'rum',
    service: 'order-tracking-web-rum',
    application: { name: 'order-tracking-web-rum' },
    location: { function: 'resolveRoute', file: 'src/router/index.ts' },
    perDay: 4,
    ageDays: 6,
    cause: 'invalid_argument',
    versions: ['1.4.0', '1.4.0'],
  },
  {
    errorType: 'TypeError',
    message: 'e.data is undefined',
    platform: 'browser',
    persona: 'browser',
    source: 'rum',
    service: 'identity',
    application: { name: 'identity' },
    location: { function: 'onMessage', file: 'src/workers/sync.ts' },
    perDay: 13,
    ageDays: 45,
    cause: 'code_exception',
    versions: ['7.2.0', '7.3.1'],
  },
  {
    errorType: 'Invariant Violation',
    message: 'Text strings must be rendered within a <Text> component.',
    platform: 'react-native',
    persona: 'mobile',
    source: 'rum',
    service: 'shop-mobile-app',
    application: SHOP_APP,
    perDay: 6,
    ageDays: 2,
    isCrash: true,
    cause: 'code_exception',
    versions: ['3.9.0', '3.9.0'],
  },
  {
    errorType: 'MongoServerSelectionError',
    message: 'Server selection timed out after 30000 ms',
    platform: 'node',
    persona: 'backend',
    source: 'logs',
    service: 'checkout-worker',
    perDay: 17,
    ageDays: 75,
    cause: 'lost_connection',
    versions: ['2.0.3', '2.3.0'],
  },
  {
    errorType: 'UnhandledPromiseRejection',
    message: "Cannot read properties of null (reading 'orderId')",
    platform: 'node',
    persona: 'backend',
    source: 'apm',
    service: 'checkout-backend',
    location: { function: 'loadPolicy', file: 'src/orders/service.ts' },
    perDay: 8,
    ageDays: 12,
    cause: 'code_exception',
    versions: ['2.3.0', '2.3.0'],
  },
  {
    errorType: 'MongoServerError',
    message: 'E11000 duplicate key error collection: identity.users index: email_1 dup key',
    platform: 'node',
    persona: 'backend',
    source: 'logs',
    service: 'identity',
    perDay: 5,
    ageDays: 210,
    cause: 'invalid_argument',
    versions: ['7.0.0', '7.3.1'],
  },
  {
    errorType: 'System.ServiceModel.FaultException',
    message: 'The creator of this fault did not specify a Reason.',
    platform: 'dotnet',
    persona: 'backend',
    source: 'logs',
    service: 'crm-sync',
    perDay: 9,
    ageDays: 300,
    cause: 'failed_request',
    versions: ['9.1.0', '9.2.4'],
  },
  {
    errorType: 'KeyError',
    message: "'order_number'",
    platform: 'python',
    persona: 'backend',
    source: 'apm',
    service: 'etl-pipeline',
    location: { function: 'transform_row', file: 'etl/pipeline/transform.py' },
    perDay: 9,
    ageDays: 21,
    cause: 'code_exception',
    versions: ['0.14.2', '0.15.0'],
  },
  {
    errorType: 'pymongo.errors.AutoReconnect',
    message: 'connection closed',
    platform: 'python',
    persona: 'backend',
    source: 'logs',
    service: 'mongo',
    perDay: 8,
    ageDays: 95,
    cause: 'lost_connection',
    versions: ['1.2.0', '1.2.0'],
  },
  {
    errorType: 'context deadline exceeded',
    message: 'rpc error: code = DeadlineExceeded desc = context deadline exceeded',
    platform: 'go',
    persona: 'backend',
    source: 'apm',
    service: 'import-service',
    location: { function: 'ConfigClient.Reload', file: 'internal/config/client.go' },
    perDay: 5,
    ageDays: 14,
    cause: 'lost_connection',
    versions: ['0.8.0', '0.8.2'],
  },
  {
    errorType: 'UpstreamError',
    message: 'GetCustomer bad response',
    platform: 'go',
    persona: 'backend',
    source: 'logs',
    service: 'order-service',
    location: { function: 'handleOrder.GetCustomer', file: 'internal/orders/submitr.go' },
    perDay: 12,
    ageDays: 3,
    isRegression: true,
    cause: 'failed_request',
    versions: ['3.1.0', '3.2.0'],
  },
  {
    errorType: 'java.net.SocketTimeoutException',
    message: 'Read timed out',
    platform: 'java',
    persona: 'backend',
    source: 'apm',
    service: 'pricing-service',
    location: { function: 'socketRead0', file: 'java.net.SocketInputStream.java' },
    perDay: 4,
    ageDays: 50,
    cause: 'lost_connection',
    versions: ['6.0.1', '6.2.0'],
  },
  {
    errorType: 'NSInvalidArgumentException',
    message: '-[__NSCFNumber length]: unrecognized selector sent to instance 0x8000000000000000',
    platform: 'ios',
    persona: 'mobile',
    source: 'rum',
    service: 'shop-mobile-app',
    application: { name: 'shop-mobile-app', packageId: 'com.example.shop.ios' },
    perDay: 1,
    ageDays: 8,
    isCrash: true,
    cause: 'invalid_argument',
    versions: ['3.9.0', '3.9.0'],
  },
  {
    errorType: 'java.lang.IllegalStateException',
    message: 'Cannot call commit after onSaveInstanceState',
    platform: 'android',
    persona: 'mobile',
    source: 'rum',
    service: 'shop-mobile-app',
    application: SHOP_APP,
    perDay: 3,
    ageDays: 70,
    isCrash: true,
    cause: 'illegal_object_access',
    status: 'resolved',
    versions: ['3.6.0', '3.8.2'],
  },
  {
    errorType: 'TypeError',
    message: "Cannot destructure property 'id' of 'undefined'",
    platform: 'browser',
    persona: 'browser',
    source: 'rum',
    service: 'gift-cards',
    application: { name: 'gift-cards' },
    perDay: 2,
    ageDays: 30,
    cause: 'code_exception',
    status: 'resolved',
    versions: ['1.0.4', '1.0.9'],
  },
  {
    errorType: 'redis.exceptions.ConnectionError',
    message: 'Error 111 connecting to redis:6379. Connection refused.',
    platform: 'python',
    persona: 'backend',
    source: 'logs',
    service: 'etl-pipeline',
    perDay: 4,
    ageDays: 40,
    cause: 'lost_connection',
    status: 'resolved',
    versions: ['0.13.0', '0.14.2'],
  },
  {
    errorType: 'System.TimeoutException',
    message: 'The request channel timed out while waiting for a reply after 00:01:00.',
    platform: 'dotnet',
    persona: 'backend',
    source: 'logs',
    service: 'crm-sync',
    perDay: 3,
    ageDays: 160,
    cause: 'lost_connection',
    status: 'reviewed',
    versions: ['9.0.0', '9.2.4'],
  },
  {
    errorType: 'ResizeObserver loop limit exceeded',
    message: 'ResizeObserver loop completed with undelivered notifications.',
    platform: 'browser',
    persona: 'browser',
    source: 'rum',
    service: 'store-locator-web-rum',
    application: { name: 'store-locator-web-rum' },
    perDay: 20,
    ageDays: 200,
    cause: null,
    status: 'ignored',
    versions: ['0.1.0', '0.9.4'],
  },
];

// ---------------------------------------------------------------- store

interface StoredIssue {
  base: Omit<Issue, 'count' | 'sparkline' | 'lastSeen'>;
  env: string;
  occurrences: number[]; // ascending timestamps
  activity: ActivityEntry[];
  seed: number;
}

function hashSeed(s: string): number {
  let h = 2166136261;
  for (let i = 0; i < s.length; i++) h = Math.imul(h ^ s.charCodeAt(i), 16777619);
  return h >>> 0;
}

let store: StoredIssue[] | null = null;

function issues(): StoredIssue[] {
  if (store) return store;
  const now = Date.now();
  store = TEMPLATES.map((t, i) => {
    const id = `iss-${(hashSeed(t.errorType + t.service + i) % 0xffffff).toString(16).padStart(6, '0')}${i}`;
    const seed = hashSeed(id);
    const r = rng(seed);
    const span = Math.min(t.ageDays, 7) * DAY;
    // Resolved issues stopped occurring a few hours ago.
    const quietFor = t.status === 'resolved' ? 6 * 3600_000 : 0;
    const n = Math.max(1, Math.round((t.perDay * span) / DAY));
    const occurrences: number[] = [];
    for (let k = 0; k < n; k++) {
      // Bursty: square the uniform draw toward "recent" for half the occurrences.
      const u = k % 2 ? r() : Math.pow(r(), 2);
      occurrences.push(Math.round(now - quietFor - u * (span - quietFor)));
    }
    occurrences.push(now - quietFor - Math.round(r() * 5 * 60e3)); // seen recently
    occurrences.sort((a, b) => a - b);
    const firstSeen = now - t.ageDays * DAY;
    const status = t.status ?? 'for_review';
    const activity: ActivityEntry[] = [{ kind: 'created', at: firstSeen }];
    if (t.isRegression) activity.push({ kind: 'regression', at: now - Math.round(r() * 2 * DAY), version: t.versions[1] });
    if (status !== 'for_review') activity.push({ kind: 'status', at: now - 2 * DAY, by: USERS[2], from: 'for_review', to: status });
    return {
      seed,
      env: r() < 0.85 ? 'prod' : 'staging',
      occurrences,
      activity,
      base: {
        id,
        errorType: t.errorType,
        message: t.message,
        platform: t.platform,
        persona: t.persona,
        source: t.source,
        service: t.service,
        application: t.application,
        location: t.location,
        firstSeen,
        firstSeenVersion: t.versions[0],
        lastSeenVersion: t.versions[1],
        status,
        assignee: null,
        team: t.team ?? (t.source === 'apm' ? 'platform-team' : t.persona === 'browser' ? 'web-squad' : null),
        isCrash: !!t.isCrash,
        isRegression: !!t.isRegression,
        suspectedCause: t.cause,
      },
    };
  });
  return store;
}

function find(id: string): StoredIssue {
  const s = issues().find((x) => x.base.id === id);
  if (!s) throw new Error(`Issue ${id} not found`);
  return s;
}

function countIn(s: StoredIssue, range: TimeRange): number {
  let n = 0;
  for (const t of s.occurrences) if (t >= range.from && t <= range.to) n++;
  return n;
}

function buckets(s: StoredIssue, range: TimeRange, count: number): number[] {
  const size = (range.to - range.from) / count;
  const out = new Array<number>(count).fill(0);
  for (const t of s.occurrences) {
    if (t < range.from || t > range.to) continue;
    out[Math.min(count - 1, Math.floor((t - range.from) / size))]++;
  }
  return out;
}

function toIssue(s: StoredIssue, range: TimeRange): Issue {
  return {
    ...s.base,
    lastSeen: s.occurrences[s.occurrences.length - 1],
    count: countIn(s, range),
    sparkline: buckets(s, range, 12),
  };
}

// ---------------------------------------------------------------- search

function issueField(s: StoredIssue, key: string): string | undefined {
  const b = s.base;
  switch (key) {
    case 'service':
      return b.service;
    case 'application':
      return b.application?.name;
    case 'env':
      return s.env;
    case 'version':
      return b.lastSeenVersion;
    case 'platform':
      return b.platform;
    case 'source':
      return b.source;
    case 'error.type':
    case '@error.type':
      return b.errorType;
    case 'status':
      return b.status;
  }
  return undefined;
}

function matchIssue(s: StoredIssue, terms: Term[], skipKey?: string): boolean {
  return matchesTerms((k) => issueField(s, k), `${s.base.errorType} ${s.base.message}`, terms, skipKey);
}

/** Every filter except the query text and the state tab. */
function passesControls(s: StoredIssue, req: IssueFacetsRequest): boolean {
  const b = s.base;
  if (req.persona !== 'all' && b.persona !== req.persona) return false;
  if (req.source !== 'all' && b.source !== req.source) return false;
  if (req.suspectedCause !== 'all' && b.suspectedCause !== req.suspectedCause) return false;
  if (req.team !== 'all' && b.team !== req.team) return false;
  if (req.investigationReady && !b.location) return false;
  if (req.assignee === 'unassigned' && b.assignee) return false;
  if (req.assignee !== 'all' && req.assignee !== 'unassigned' && b.assignee?.id !== req.assignee) return false;
  return true;
}

function relevance(i: Issue): number {
  const ageH = (Date.now() - i.lastSeen) / 3600_000;
  return (i.count * (i.isCrash ? 3 : 1) * (i.isRegression ? 2 : 1)) / (1 + ageH / 6);
}

export function searchIssues(req: IssueSearchRequest): Promise<IssueSearchPage> {
  const terms = parseQuery(req.query);
  const matched = issues()
    .filter((s) => passesControls(s, req) && matchIssue(s, terms))
    .map((s) => toIssue(s, req.range))
    .filter((i) => i.count > 0);

  const totals: Record<IssueStateTab, number> = { for_review: 0, reviewed: 0, ignored: 0, resolved: 0, all: matched.length };
  for (const i of matched) totals[i.status]++;

  const inState = req.state === 'all' ? matched : matched.filter((i) => i.status === req.state);
  const sorters: Record<IssueSearchRequest['sort'], (a: Issue, b: Issue) => number> = {
    relevance: (a, b) => relevance(b) - relevance(a),
    count: (a, b) => b.count - a.count,
    last_seen: (a, b) => b.lastSeen - a.lastSeen,
    first_seen: (a, b) => b.firstSeen - a.firstSeen,
  };
  inState.sort(sorters[req.sort]);

  const offset = req.cursor ? Number(atob(req.cursor)) : 0;
  const page = inState.slice(offset, offset + req.limit);
  const next = offset + req.limit < inState.length ? btoa(String(offset + req.limit)) : null;
  return delay({ issues: page, nextCursor: next, totals });
}

const FACETS: { key: string; label: string }[] = [
  { key: 'service', label: 'Service' },
  { key: 'application', label: 'Application' },
  { key: 'env', label: 'Env' },
  { key: 'version', label: 'Version' },
  { key: 'platform', label: 'Platform' },
  { key: 'source', label: 'Source' },
];

export function issueFacets(req: IssueFacetsRequest): Promise<IssueFacets> {
  const terms = parseQuery(req.query);
  const scope = issues().filter((s) => passesControls(s, req) && countIn(s, req.range) > 0);
  const facets: Facet[] = FACETS.map(({ key, label }) => {
    const counts = new Map<string, number>();
    for (const s of scope) {
      if (!matchIssue(s, terms, key)) continue;
      const v = issueField(s, key);
      if (v !== undefined) counts.set(v, (counts.get(v) ?? 0) + 1);
    }
    return {
      key,
      label,
      group: '',
      values: [...counts.entries()].map(([value, count]) => ({ value, count })).sort((a, b) => b.count - a.count),
    };
  });
  return delay({ facets, totalAvailable: 1251 });
}

export function suggest(range: TimeRange, partial: string): Promise<QuerySuggestion[]> {
  const scope = issues().filter((s) => countIn(s, range) > 0);
  const p = partial.toLowerCase();
  const colon = p.indexOf(':');
  const keys = [...FACETS.map((f) => f.key), 'error.type'];
  if (colon > 0) {
    const key = partial.slice(0, colon);
    const v = p.slice(colon + 1);
    const counts = new Map<string, number>();
    for (const s of scope) {
      const val = issueField(s, key);
      if (val !== undefined && val.toLowerCase().includes(v)) counts.set(val, (counts.get(val) ?? 0) + 1);
    }
    return delay(
      [...counts.entries()]
        .sort((a, b) => b[1] - a[1])
        .slice(0, 7)
        .map(([val, count]) => ({ text: `${key}:${/\s/.test(val) ? `"${val}"` : val}`, count })),
      40,
    );
  }
  return delay(
    keys.filter((k) => k.startsWith(p)).map((k) => ({ text: `${k}:`, count: scope.filter((s) => issueField(s, k) !== undefined).length })),
    40,
  );
}

// ---------------------------------------------------------------- detail

const VIEW_NAMES = ['PreLoginScreen', 'SplashScreen', 'ApplicationLaunch', 'OrderListScreen', 'OrderDetailsScreen'];
const URL_PATHS = ['/cart', '/orders/:id', '/offers', '/dashboard', '/stores/search'];

function tagDistributions(s: StoredIssue): TagDistribution[] {
  const r = rng(s.seed + 3);
  const b = s.base;
  const split = (n: number) => {
    const raw = Array.from({ length: n }, () => r() + 0.2);
    const sum = raw.reduce((a, x) => a + x, 0);
    return raw.map((x) => x / sum).sort((a, b) => b - a);
  };
  if (b.persona === 'mobile') {
    const [v] = split(3);
    return [
      { key: '@view.name', value: VIEW_NAMES[0], share: Math.max(v, 0.5) },
      { key: '@os.version', value: b.platform === 'ios' ? '17.5' : '16', share: 0.45 },
      { key: 'version', value: b.lastSeenVersion ?? '1.0.0', share: 1 },
    ];
  }
  if (b.persona === 'browser') {
    return [
      { key: '@view.url_path', value: URL_PATHS[Math.floor(r() * URL_PATHS.length)], share: 0.58 },
      { key: '@browser.name', value: 'Chrome', share: 0.71 },
      { key: 'version', value: b.lastSeenVersion ?? '1.0.0', share: 0.92 },
    ];
  }
  return [
    { key: 'env', value: s.env, share: 1 },
    { key: 'version', value: b.lastSeenVersion ?? '1.0.0', share: 0.88 },
    { key: 'kube_namespace', value: `ns-billing-${s.env}`, share: 1 },
  ];
}

function detail(s: StoredIssue, range: TimeRange): IssueDetail {
  const issue = toIssue(s, range);
  const n = 48;
  const size = (range.to - range.from) / n;
  const counts = buckets(s, range, n);
  const frontend = s.base.persona !== 'backend';
  return {
    ...issue,
    histogram: counts.map((count, i) => ({ t: Math.round(range.from + i * size), count })),
    impact: frontend ? { sessions: issue.count, sessionShare: issue.count / 12000, views: Math.max(1, Math.min(issue.count, 5)), users: Math.ceil(issue.count * 0.8) } : null,
    tags: tagDistributions(s),
    activity: [...s.activity].sort((a, b) => b.at - a.at),
  };
}

export function getIssue(id: string, range: TimeRange): Promise<IssueDetail> {
  return delay(detail(find(id), range));
}

export function updateIssue(id: string, patch: IssuePatch): Promise<IssueDetail> {
  const s = find(id);
  const now = Date.now();
  if (patch.status && patch.status !== s.base.status) {
    s.activity.push({ kind: 'status', at: now, by: CURRENT_USER, from: s.base.status, to: patch.status });
    s.base.status = patch.status;
  }
  if (patch.assigneeId !== undefined) {
    const to = USERS.find((u) => u.id === patch.assigneeId) ?? null;
    s.activity.push({ kind: 'assign', at: now, by: CURRENT_USER, to });
    s.base.assignee = to;
  }
  if (patch.suspectedCause !== undefined) s.base.suspectedCause = patch.suspectedCause;
  const last = s.occurrences[s.occurrences.length - 1];
  return delay(detail(s, { from: now - DAY, to: Math.max(now, last) }), 150);
}

export function addComment(id: string, text: string): Promise<ActivityEntry> {
  const entry: ActivityEntry = { kind: 'comment', at: Date.now(), by: CURRENT_USER, text };
  find(id).activity.push(entry);
  return delay(entry, 150);
}

export const TEAMS = ['platform-team', 'web-squad', 'mobile-squad'];

export function listTeams(): Promise<string[]> {
  return delay(TEAMS, 40);
}

export function listUsers(): Promise<User[]> {
  return delay(USERS, 60);
}

// ---------------------------------------------------------------- samples

const hexStr = (r: Rand, n: number) => Array.from({ length: n }, () => Math.floor(r() * 16).toString(16)).join('');
const uuid = (r: Rand) => `${hexStr(r, 8)}-${hexStr(r, 4)}-4${hexStr(r, 3)}-a${hexStr(r, 3)}-${hexStr(r, 12)}`;
const addr = (r: Rand) => `0x${hexStr(r, 16).replace(/^0+/, '').padStart(16, '0')}`;

function frames(s: StoredIssue, r: Rand): { title: string; frames: StackFrame[] } {
  const b = s.base;
  const f = (index: number, module: string, symbol: string, extra: Partial<StackFrame> = {}): StackFrame => ({ index, module, symbol, inApp: false, ...extra });
  const msg = b.message.replace(/\bX\b/g, () => String(Math.floor(r() * 9) + 1));
  switch (b.platform) {
    case 'unknown':
      return { title: msg, frames: [] };
    case 'android':
      if (b.errorType === 'SIGSEGV') {
        const list: StackFrame[] = [
          f(0, '[vdso]', '__kernel_rt_sigreturn', { address: '0x0000000000000848' }),
          f(1, 'libhermes.so', '', { address: '0x00000000000bd018' }),
          f(2, 'libhermes.so', '', { address: '0x00000000000bf7ac' }),
        ];
        for (let i = 3; i < 28; i++) list.push(f(i, i % 3 ? 'libhermes.so' : 'libreactnativejni.so', '', { address: addr(r) }));
        list.push(
          f(
            28,
            'libfbjni.so',
            '_ZN8facebook3jni6detail13MethodWrapperIMNS0_15JNativeRunnableEFvvEXadL_ZNS3_3runEvEES3_vJEE8dispatchENS0_9alias_refIPNS1_8JTypeForINS0_11HybridClassIS3_NS0_9JRunnableEE8JavaPartESA_vE11_javaobjectEEE',
            { address: '0x0000000000017dd4' },
          ),
          f(
            29,
            'libfbjni.so',
            '_ZN8facebook3jni6detail15FunctionWrapperIPFvNS0_9alias_refIPNS1_8JTypeForINS0_11HybridClassINS0_15JNativeRunnableENS0_9JRunnableEE8JavaPartES7_vE11_javaobjectEEESC_vJEE4callEP7_JNIEnvP8_jobjectSF_',
            { address: '0x0000000000017d14' },
          ),
        );
        return { title: `SIGSEGV: ${b.message}`, frames: list };
      }
      return {
        title: `${b.errorType}: ${b.message}`,
        frames: [
          f(0, 'android.os.MessageQueue', 'nativePollOnce', { file: 'Native Method' }),
          f(1, 'android.os.MessageQueue', 'next', { file: 'MessageQueue.java', line: 335 }),
          f(2, 'android.os.Looper', 'loopOnce', { file: 'Looper.java', line: 161 }),
          f(3, 'android.os.Looper', 'loop', { file: 'Looper.java', line: 288 }),
          f(4, 'com.example.shop.MainActivity', 'onResume', { file: 'MainActivity.kt', line: 88, inApp: true }),
          f(5, 'android.app.ActivityThread', 'main', { file: 'ActivityThread.java', line: 8177 }),
        ],
      };
    case 'java':
      return {
        title: `${b.errorType}: ${msg}`,
        frames: [
          f(0, 'jdk.internal.util.Preconditions', b.location?.function ?? 'outOfBounds', { file: 'Preconditions.java', line: 64 }),
          f(1, 'java.util.ArrayList', 'get', { file: 'ArrayList.java', line: 427 }),
          f(2, `com.example.${b.service.replace(/-/g, '')}.service.InvoiceService`, 'buildInvoice', { file: 'InvoiceService.java', line: 212, inApp: true }),
          f(3, `com.example.${b.service.replace(/-/g, '')}.web.InvoiceController`, 'create', { file: 'InvoiceController.java', line: 58, inApp: true }),
          f(4, 'org.springframework.web.servlet.FrameworkServlet', 'service', { file: 'FrameworkServlet.java', line: 885 }),
          f(5, 'jakarta.servlet.http.HttpServlet', 'service', { file: 'HttpServlet.java', line: 658 }),
        ],
      };
    case 'dotnet':
      return {
        title: `${b.errorType}: ${b.message}`,
        frames: [
          f(0, 'Oracle.DataAccess.Client.OracleException', 'HandleErrorHelper', { file: 'OracleException.cs', line: 0 }),
          f(1, 'Oracle.DataAccess.Client.OracleCommand', 'ExecuteNonQuery', { file: 'OracleCommand.cs', line: 0 }),
          f(2, 'Catalog.Api.Data.StockRepository', 'Manage', { file: 'StockRepository.cs', line: 1538, inApp: true }),
          f(3, 'Catalog.Api.Services.StockService', 'Update', { file: 'StockService.cs', line: 77, inApp: true }),
        ],
      };
    case 'browser':
    case 'react-native':
      return {
        title: `${b.errorType}: ${b.message}`,
        frames: [
          f(0, b.location?.file ?? 'src/index.ts', b.location?.function ?? 'anonymous', { line: 42, inApp: true }),
          f(1, 'src/hooks/useAsync.ts', 'run', { line: 18, inApp: true }),
          f(2, 'node_modules/react-dom/cjs/react-dom.production.min.js', 'commitHookEffectListMount', { line: 1 }),
          f(3, 'node_modules/react-dom/cjs/react-dom.production.min.js', 'flushPassiveEffects', { line: 1 }),
        ],
      };
    case 'node':
      return {
        title: `${b.errorType}: ${b.message}`,
        frames: [
          f(0, b.location?.file ?? 'node_modules/mongodb/lib/sdam/topology.js', b.location?.function ?? 'Timeout._onTimeout', { line: 278, inApp: !!b.location }),
          f(1, 'node:internal/timers', 'listOnTimeout', { line: 569 }),
          f(2, 'node:internal/timers', 'processTimers', { line: 512 }),
        ],
      };
    case 'python':
      return {
        title: `${b.errorType}: ${b.message}`,
        frames: [
          f(0, b.location?.file ?? 'pymongo/pool.py', b.location?.function ?? 'receive_message', { line: 131, inApp: !!b.location }),
          f(1, 'etl/pipeline/run.py', 'process_batch', { line: 57, inApp: true }),
          f(2, 'celery/app/trace.py', '__protected_call__', { line: 760 }),
        ],
      };
    case 'go':
      return {
        title: `${b.errorType}: ${b.message}`,
        frames: [
          f(0, b.location?.file ?? 'main.go', b.location?.function ?? 'main', { line: 214, inApp: true }),
          f(1, 'google.golang.org/grpc/clientconn.go', 'Invoke', { line: 1022 }),
          f(2, 'runtime/asm_amd64.s', 'goexit', { line: 1700 }),
        ],
      };
    case 'ios':
      return {
        title: `${b.errorType}: ${b.message}`,
        frames: [
          f(0, 'CoreFoundation', '__exceptionPreprocess', { address: addr(r) }),
          f(1, 'libobjc.A.dylib', 'objc_exception_throw', { address: addr(r) }),
          f(2, 'ShopApp', '-[OrderCell configureWith:]', { address: addr(r), inApp: true }),
          f(3, 'UIKitCore', '-[UITableView _createPreparedCellForGlobalRow:]', { address: addr(r) }),
        ],
      };
  }
}

function rawStack(title: string, list: StackFrame[]): string {
  return [
    title,
    ...list.map((fr) =>
      fr.address ? `${String(fr.index).padEnd(3)} ${fr.module.padEnd(22)} ${fr.address} ${fr.symbol}` : `    at ${fr.module}.${fr.symbol}(${fr.file ?? fr.module}${fr.line ? `:${fr.line}` : ''})`,
    ),
  ].join('\n');
}

function sampleTags(s: StoredIssue, r: Rand): Record<string, string> {
  const b = s.base;
  const tags: Record<string, string> = { env: s.env, service: b.service, version: b.lastSeenVersion ?? '1.0.0' };
  if (b.persona === 'mobile') {
    tags['@view.name'] = r() < 0.63 ? VIEW_NAMES[0] : VIEW_NAMES[1 + Math.floor(r() * 4)];
    tags['@os.version'] = b.platform === 'ios' ? '17.5' : r() < 0.45 ? '16' : '14';
  } else if (b.persona === 'browser') {
    tags['@view.url_path'] = URL_PATHS[Math.floor(r() * URL_PATHS.length)];
    tags['@browser.name'] = r() < 0.71 ? 'Chrome' : 'Safari';
  } else {
    tags.host = `i-0${hexStr(r, 16)}`;
  }
  return tags;
}

function buildSample(s: StoredIssue, ts: number, r: Rand, tags: Record<string, string>): ErrorSample {
  const b = s.base;
  const { title, frames: list } = frames(s, r);
  const mobile = b.persona === 'mobile';
  const frontend = b.persona !== 'backend';
  const view = tags['@view.name'] ?? tags['@view.url_path'] ?? '';
  const viewId = uuid(r);
  const sessionId = uuid(r);
  const country = ['Israel', 'Thailand', 'Germany', 'United States'][Math.floor(r() * 4)];

  const timeline: TimelineEvent[] = frontend
    ? [
        { at: ts, type: b.isCrash ? 'crash' : 'error', label: b.isCrash ? 'Crash' : 'Error', message: `source ${title}`, isCurrent: true },
        { at: ts - 6161, type: 'view', label: 'View Load', message: mobile ? 'Load Screen' : 'Load Page', emphasis: view || 'Home' },
        { at: ts - 8238, type: mobile ? 'view' : 'action', label: mobile ? 'View Load' : 'Action', message: mobile ? 'Load Screen' : 'Click on', emphasis: mobile ? 'SplashScreen' : 'Submit' },
        { at: ts - 8687, type: 'view', label: 'View Load', message: mobile ? 'Load Screen' : 'Load Page', emphasis: mobile ? 'ApplicationLaunch' : '/' },
      ]
    : [
        { at: ts + 4, type: 'log', label: 'Log', message: `request failed: ${b.errorType}` },
        { at: ts, type: 'error', label: 'Error', message: title, isCurrent: true },
        { at: ts - 12, type: 'log', label: 'Log', message: `calling downstream for ${b.location?.function ?? 'handler'}` },
        { at: ts - 40, type: 'log', label: 'Log', message: 'request received POST /api/v1/process' },
      ];

  // Backend errors happen in a trace; APM samples link to it.
  const traceId = frontend ? '' : hexStr(r, 32);
  const spanId = frontend ? '' : hexStr(r, 16);
  const attributeGroups: AttributeGroup[] = frontend
    ? [
        {
          name: 'View',
          attributes: [
            { label: 'View Id', value: viewId },
            { label: mobile ? 'View Path' : 'View Path', value: mobile ? `${view}/${hexStr(r, 20)}` : view },
            { label: 'View URL', value: mobile ? `${view}/${hexStr(r, 20)}` : `https://${b.service.toLowerCase()}.example.com${view}` },
          ],
        },
        {
          name: 'Session',
          attributes: [
            { label: 'Session Id', value: sessionId },
            { label: 'Device', value: mobile ? 'Mobile' : 'Desktop' },
            { label: 'Country', value: country },
            { label: 'OS', value: mobile ? (b.platform === 'ios' ? 'iOS' : 'Android') : 'Windows' },
            { label: 'OS Version', value: tags['@os.version'] ?? '11' },
          ],
        },
        ...(mobile
          ? [
              {
                name: 'Device',
                attributes: [
                  { label: 'Brand', value: 'OnePlus' },
                  { label: 'Model', value: 'PJZ110' },
                  { label: 'Architecture', value: 'aarch64' },
                ],
              },
            ]
          : [
              {
                name: 'Browser',
                attributes: [
                  { label: 'Browser', value: tags['@browser.name'] ?? 'Chrome' },
                  { label: 'Version', value: '129.0' },
                ],
              },
            ]),
      ]
    : [
        {
          name: 'Host',
          attributes: [
            { label: 'Host', value: tags.host },
            { label: 'Env', value: s.env },
            { label: 'Version', value: tags.version },
          ],
        },
        {
          name: 'Trace',
          attributes: [
            { label: 'Trace Id', value: traceId },
            { label: 'Span Id', value: spanId },
          ],
        },
      ];

  const json: Record<string, JsonValue> = {
    _oc: { error_tracking: { billed: false, fingerprints: [hexStr(r, 16), hexStr(r, 16)], origin: b.source, persona: b.persona } },
    ...(b.application ? { application: { id: uuid(r) } } : {}),
    attributes: {
      ...(b.application ? { application: { id: uuid(r), name: b.application.name, short_name: b.application.name.replace(/-/g, '_') } } : {}),
      ...(mobile
        ? {
            connectivity: { interfaces: ['wifi'], status: 'connected' },
            device: { architecture: 'aarch64', brand: 'OnePlus', is_low_ram: false, logical_cpu_count: 8, model: 'PJZ110', name: 'OnePlus PJZ110' },
          }
        : {}),
      error: { type: b.errorType, message: b.message, is_crash: b.isCrash, source: b.source, stack: rawStack(title, list.slice(0, 4)) },
      ...(frontend ? { view: { id: viewId, name: view }, session: { id: sessionId } } : { host: tags.host, trace_id: traceId }),
      env: s.env,
      service: b.service,
      version: tags.version,
    },
    date: new Date(ts).toISOString(),
  };

  const trace = b.source === 'apm' ? { traceId, spanId } : {};
  return { id: `${b.id}-${ts}`, timestamp: ts, tags, stack: { title, frames: list, raw: rawStack(title, list) }, timeline, attributeGroups, json, ...trace };
}

export function getErrorSample(req: ErrorSampleRequest): Promise<ErrorSamplePage> {
  const s = find(req.issueId);
  const terms = parseQuery(req.query);
  const candidates = s.occurrences
    .filter((t) => t >= req.range.from && t <= req.range.to)
    .sort((a, b) => (req.order === 'oldest' ? a - b : b - a))
    .map((ts) => {
      const r = rng(s.seed ^ ts);
      return { ts, r, tags: sampleTags(s, r) };
    })
    .filter((c) => matchesTerms((k) => c.tags[k], s.base.message, terms));
  const index = Math.max(0, Math.min(req.index, candidates.length - 1));
  const c = candidates[index];
  return delay({ sample: c ? buildSample(s, c.ts, c.r, c.tags) : null, index, total: candidates.length, available: candidates.length });
}
