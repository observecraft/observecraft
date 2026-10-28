// In-browser Developer Portal home and settings for mock mode. GitHub and Jira start connected with
// fictional pull requests and tickets; GitLab and Linear start disconnected, so their pages show the
// "connect" overlay. State lives in memory for the session; validation follows routes/portal_home.rs.
import { getScorecards } from './scorecards';
import { SERVICES, deployedAt } from './apmTraces';
import type {
  CodeHost,
  PortalApp,
  PortalAppInput,
  PortalConnectionTest,
  PortalConnector,
  PortalConnectorInput,
  PortalEntity,
  PortalIdentitiesInput,
  PortalIdentity,
  PortalProvider,
  PortalPullRequest,
  PortalPullRequests,
  PortalSources,
  PortalTicket,
  PortalTickets,
  TicketSource,
} from '../portal/types';

const MIN = 60_000;
const HOUR = 60 * MIN;
const DAY = 24 * HOUR;
const PROVIDERS: PortalProvider[] = ['github', 'gitlab', 'jira', 'linear'];
const DEFAULT_BASE: Record<PortalProvider, string> = { github: 'https://api.github.com', gitlab: 'https://gitlab.com', jira: '', linear: 'https://api.linear.app' };
const LABEL: Record<PortalProvider, string> = { github: 'GitHub', gitlab: 'GitLab', jira: 'Jira', linear: 'Linear' };
const ME_EMAIL = 'alex.morgan@example.com';

interface State {
  connectors: Map<PortalProvider, { baseUrl: string; account: string; checkedAt: number | null; ok: boolean | null; error: string; updatedAt: number }>;
  identities: Map<PortalProvider, string>;
  apps: PortalApp[];
  favorites: Set<string>;
}

let state: State | null = null;
let seq = 0;
const uuid = () => `00000000-0000-4000-9000-${String(++seq).padStart(12, '0')}`;
const fail = (message: string): never => {
  throw new Error(message);
};

function db(): State {
  if (state) return state;
  const now = Date.now();
  state = {
    connectors: new Map([
      ['github', { baseUrl: DEFAULT_BASE.github, account: '', checkedAt: now - 2 * HOUR, ok: true, error: '', updatedAt: now - 9 * DAY }],
      ['jira', { baseUrl: 'https://example-shop.atlassian.net', account: 'portal-bot@example.com', checkedAt: now - 5 * HOUR, ok: true, error: '', updatedAt: now - 9 * DAY }],
    ]),
    identities: new Map([['github', 'alex-morgan']]),
    apps: [],
    favorites: new Set(['order-service']),
  };
  return state;
}

/** Forgets every change (tests). */
export function resetPortalMock(): void {
  state = null;
}

function connectorOut(p: PortalProvider): PortalConnector {
  const c = db().connectors.get(p);
  return c
    ? { provider: p, configured: true, baseUrl: c.baseUrl, account: c.account, lastCheckedAt: c.checkedAt, lastCheckOk: c.ok, lastCheckError: c.error, updatedAt: c.updatedAt }
    : { provider: p, configured: false, baseUrl: DEFAULT_BASE[p], account: '', lastCheckedAt: null, lastCheckOk: null, lastCheckError: '', updatedAt: null };
}

export async function getConnectors(): Promise<PortalConnector[]> {
  return PROVIDERS.map(connectorOut);
}

export async function saveConnector(p: PortalProvider, input: PortalConnectorInput): Promise<PortalConnector> {
  const raw = (input.baseUrl ?? '').trim() || DEFAULT_BASE[p];
  const account = (input.account ?? '').trim();
  if (!raw) fail(`${LABEL[p]} needs a site URL (https://your-site.atlassian.net)`);
  if (!/^https:\/\/[^/@\s?#]+(\/[^\s?#]*)?$/.test(raw)) fail('The base URL must use https');
  if (p === 'linear' && raw.replace(/\/$/, '') !== DEFAULT_BASE.linear) fail("Linear's API URL cannot be changed");
  if (p === 'jira' && !/^[^@\s]+@[^@\s]+\.[^@\s]+$/.test(account)) fail('Jira needs the email the API token belongs to');
  const existing = db().connectors.get(p);
  if (!existing && !input.token?.trim()) fail(`${LABEL[p]} needs a token`);
  db().connectors.set(p, { baseUrl: raw.replace(/\/$/, ''), account: p === 'jira' ? account : '', checkedAt: null, ok: null, error: '', updatedAt: Date.now() });
  return connectorOut(p);
}

export async function deleteConnector(p: PortalProvider): Promise<void> {
  if (!db().connectors.delete(p)) fail(`${LABEL[p]} is not configured`);
}

export async function testConnector(p: PortalProvider): Promise<PortalConnectionTest> {
  const c = db().connectors.get(p) ?? fail(`${LABEL[p]} is not configured`);
  const now = Date.now();
  Object.assign(c, { checkedAt: now, ok: true, error: '' });
  return { ok: true, message: `Connected as ${p === 'jira' ? 'Portal Bot' : 'alex-morgan'}`, checkedAt: now };
}

export async function getIdentities(): Promise<PortalIdentity[]> {
  return PROVIDERS.map((p) => {
    const set = db().identities.get(p);
    if (set) return { provider: p, handle: set, isDefault: false };
    return { provider: p, handle: p === 'jira' || p === 'linear' ? ME_EMAIL : '', isDefault: true };
  });
}

export async function saveIdentities(input: PortalIdentitiesInput): Promise<PortalIdentity[]> {
  for (const i of input.identities) {
    const h = i.handle.trim().replace(/^@/, '');
    if (!h) {
      db().identities.delete(i.provider);
      continue;
    }
    const ok = i.provider === 'github' ? /^[A-Za-z0-9][A-Za-z0-9-]{0,38}$/.test(h) : i.provider === 'gitlab' ? /^[A-Za-z0-9_.-]{1,255}$/.test(h) : /^[^@\s"\\]+@[^@\s"\\]+\.[^@\s"\\]+$/.test(h);
    if (!ok) fail(i.provider === 'github' || i.provider === 'gitlab' ? `${h} is not a valid ${LABEL[i.provider]} username` : `${h} is not an email address`);
    db().identities.set(i.provider, h);
  }
  return getIdentities();
}

// ---------------------------------------------------------------- home lists

const person = (login: string) => ({ login, avatarUrl: '' });

function pullRequests(now: number): PortalPullRequest[] {
  const pr = (
    group: PortalPullRequest['group'],
    repo: string,
    number: number,
    title: string,
    author: string,
    reviewers: string[],
    comments: number,
    additions: number,
    deletions: number,
    agoMin: number,
  ): PortalPullRequest => ({
    group,
    title,
    repo,
    number,
    url: `https://github.com/${repo}/pull/${number}`,
    author: person(author),
    comments,
    reviewers: reviewers.map(person),
    additions,
    deletions,
    updatedAt: now - agoMin * MIN,
  });
  return [
    pr('needs_review', 'example-org/storefront', 482, 'Cache product listings per locale', 'sam-lee', ['alex-morgan'], 3, 214, 38, 25),
    pr('needs_review', 'example-org/order-service', 1290, 'Retry payment authorization on gateway timeouts', 'ana-ruiz', ['alex-morgan', 'kai-chen'], 7, 96, 12, 140),
    pr('returned', 'example-org/billing', 377, 'Split invoice rendering into a worker', 'alex-morgan', ['ana-ruiz'], 5, 402, 167, 300),
    pr('approved', 'example-org/catalog-service', 811, 'Index product attributes for faceted search', 'alex-morgan', ['sam-lee'], 2, 58, 9, 75),
    pr('waiting', 'example-org/inventory-service', 154, 'Reserve stock in one transaction', 'alex-morgan', ['kai-chen', 'mia-novak'], 0, 131, 44, 50),
    pr('waiting', 'example-org/api-gateway', 903, 'Rate limit per API key', 'alex-morgan', ['platform-team'], 1, 77, 20, 1440),
    pr('drafts', 'example-org/storefront', 489, 'Checkout: new address form', 'alex-morgan', [], 0, 520, 210, 180),
    pr('merged', 'example-org/order-service', 1284, 'Emit order.placed events with the cart id', 'alex-morgan', ['ana-ruiz'], 4, 63, 15, 2 * 1440),
    pr('merged', 'example-org/notification-worker', 66, 'Batch email sends', 'alex-morgan', ['sam-lee'], 1, 140, 98, 4 * 1440),
  ];
}

export async function getPullRequests(provider: CodeHost): Promise<PortalPullRequests> {
  const now = Date.now();
  const base = { provider, user: db().identities.get(provider) ?? '', fetchedAt: now };
  if (!db().connectors.has(provider)) return { ...base, status: 'not_connected', message: `Connect ${LABEL[provider]} to see your pull requests`, pullRequests: [] };
  if (!base.user) return { ...base, status: 'identity_required', message: `Set your ${LABEL[provider]} username to see your pull requests`, pullRequests: [] };
  return { ...base, status: 'ok', message: '', pullRequests: provider === 'github' ? pullRequests(now) : [] };
}

function jiraTickets(now: number): PortalTicket[] {
  const t = (key: string, title: string, status: string, category: string, priority: string, level: string, createdDays: number, updatedHours: number): PortalTicket => ({
    key,
    title,
    url: `https://example-shop.atlassian.net/browse/${key}`,
    status,
    statusCategory: category,
    createdAt: now - createdDays * DAY,
    updatedAt: now - updatedHours * HOUR,
    priority,
    priorityLevel: level,
  });
  return [
    t('SHOP-1432', 'Checkout fails for saved cards after 3DS challenge', 'In Progress', 'in_progress', 'Highest', 'urgent', 3, 2),
    t('SHOP-1418', 'Show delivery estimate on the cart page', 'In Review', 'in_progress', 'High', 'high', 8, 20),
    t('SHOP-1399', 'Migrate order-service to the new payments SDK', 'To Do', 'todo', 'Medium', 'medium', 14, 49),
    t('SHOP-1370', 'Remove legacy coupon endpoint', 'To Do', 'todo', 'Low', 'low', 30, 120),
  ];
}

export async function getTickets(provider: TicketSource): Promise<PortalTickets> {
  const now = Date.now();
  if (provider === 'work_items') {
    const items: PortalTicket[] = [
      {
        key: 'IR-14',
        title: 'Elevated checkout latency in storefront-api',
        url: '/incidents/14',
        status: 'Active',
        statusCategory: 'in_progress',
        createdAt: now - 3 * HOUR,
        updatedAt: now - 20 * MIN,
        priority: 'SEV-2',
        priorityLevel: 'high',
      },
      {
        key: 'IR-11',
        title: 'Billing exports delayed',
        url: '/incidents/11',
        status: 'Stable',
        statusCategory: 'in_progress',
        createdAt: now - 2 * DAY,
        updatedAt: now - 6 * HOUR,
        priority: 'SEV-3',
        priorityLevel: 'medium',
      },
    ];
    return { provider, status: 'ok', message: '', user: 'Alex Morgan', tickets: items, fetchedAt: now };
  }
  const user = db().identities.get(provider) ?? ME_EMAIL;
  if (!db().connectors.has(provider)) return { provider, status: 'not_connected', message: `See your ${LABEL[provider]} work items here`, user, tickets: [], fetchedAt: now };
  return { provider, status: 'ok', message: '', user, tickets: provider === 'jira' ? jiraTickets(now) : [], fetchedAt: now };
}

/** Fictional counts per service: what exists around each mock service. */
const AROUND: Record<string, { monitors: number; issues: number; dashboards: number }> = {
  'api-gateway': { monitors: 4, issues: 1, dashboards: 2 },
  'storefront-api': { monitors: 3, issues: 2, dashboards: 1 },
  'order-service': { monitors: 5, issues: 3, dashboards: 2 },
  billing: { monitors: 2, issues: 0, dashboards: 1 },
  'inventory-service': { monitors: 1, issues: 1, dashboards: 0 },
  'catalog-service': { monitors: 2, issues: 4, dashboards: 1 },
  'workflow-engine': { monitors: 0, issues: 0, dashboards: 0 },
  'notification-worker': { monitors: 1, issues: 0, dashboards: 0 },
};
const ON_CALL: Record<string, string> = { checkout: 'Checkout primary', platform: 'Platform on-call', storefront: 'Storefront rotation' };

export async function getEntities(): Promise<PortalEntity[]> {
  const now = Date.now();
  const sc = await getScorecards();
  const meta = new Map(sc.services.map((s) => [s.service, s]));
  const names = [...new Set([...Object.keys(SERVICES), ...sc.services.map((s) => s.service)])];
  return names
    .map((name, i) => {
      const s = meta.get(name);
      const team = s?.team ?? (name === 'api-gateway' ? 'platform' : '');
      return {
        name,
        kind: 'service',
        team,
        lastSeen: name in SERVICES ? now - (i + 1) * MIN : null,
        lastDeploy: deployedAt(name, now - 30 * DAY),
        monitors: AROUND[name]?.monitors ?? 0,
        issues: AROUND[name]?.issues ?? 0,
        dashboards: AROUND[name]?.dashboards ?? 0,
        scorecardLevel: sc.setUp ? (s?.level ?? 0) : null,
        onCall: ON_CALL[team] ?? null,
        favorite: db().favorites.has(name),
      };
    })
    .sort((a, b) => (b.lastSeen ?? 0) - (a.lastSeen ?? 0) || a.name.localeCompare(b.name));
}

export async function setFavorite(entity: string, favorite: boolean): Promise<void> {
  if (favorite) db().favorites.add(entity);
  else db().favorites.delete(entity);
}

// ---------------------------------------------------------------- apps and sources

function checkApp(input: PortalAppInput): PortalAppInput {
  const name = input.name.trim();
  if (!name || name.length > 100) fail('The app name must be 1–100 characters');
  const url = input.url.trim();
  if (!/^https?:\/\/[^\s/]+/i.test(url) || url.length > 2048) fail('The URL must be an http(s) URL');
  const logo = (input.logo ?? '').trim();
  if (!/^[a-z0-9:._-]{0,60}$/.test(logo)) fail('Unknown logo');
  return { name, url, logo };
}

export async function getApps(): Promise<PortalApp[]> {
  return db().apps.map((a) => ({ ...a }));
}

export async function createApp(input: PortalAppInput): Promise<PortalApp[]> {
  const a = checkApp(input);
  if (db().apps.length >= 50) fail('An organization can have at most 50 apps');
  db().apps.push({ id: uuid(), name: a.name, url: a.url, logo: a.logo ?? '' });
  return getApps();
}

export async function updateApp(id: string, input: PortalAppInput): Promise<PortalApp[]> {
  const a = checkApp(input);
  const app = db().apps.find((x) => x.id === id) ?? fail('No such app');
  Object.assign(app, { name: a.name, url: a.url, logo: a.logo ?? '' });
  return getApps();
}

export async function deleteApp(id: string): Promise<void> {
  const before = db().apps.length;
  db().apps = db().apps.filter((a) => a.id !== id);
  if (db().apps.length === before) fail('No such app');
}

export async function getSources(): Promise<PortalSources> {
  return {
    signals: [
      { key: 'apm', active: true },
      { key: 'logs', active: true },
      { key: 'network', active: false },
      { key: 'containers', active: true },
    ],
    connectors: await getConnectors(),
  };
}
