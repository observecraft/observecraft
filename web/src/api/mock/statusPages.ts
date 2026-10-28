// In-browser status pages for mock mode (Incident Response → Status Pages). State lives in memory for
// the session; validation and statuses follow the server (routes/status_pages.rs).
import { checkLogoUrl, checkSlug, currentComponents, overallStatus, UPTIME_DAYS } from '../statusPages/status';
import type {
  ComponentStatus,
  NoticeImpact,
  NoticeInput,
  NoticeStatus,
  NoticeUpdate,
  NoticeUpdateInput,
  PublicStatusPage,
  StatusPageDetail,
  StatusPageInput,
  StatusPageNotice,
  StatusPageSummary,
  StatusPageVisibility,
  StatusPageVisualization,
} from '../statusPages/types';
import { mockMe } from './identity';
import { getIncident } from './incidents';

const MAX_PAGES = 100;
const MAX_COMPONENTS = 100;
const MAX_NAME_CHARS = 100;
const MAX_DESCRIPTION_CHARS = 1000;
const MAX_TITLE_CHARS = 200;
const MAX_MESSAGE_CHARS = 10_000;
const MIN = 60_000;
const HOUR = 60 * MIN;
const DAY = 24 * HOUR;

const fail = (message: string): never => {
  throw new Error(message);
};

interface Component {
  id: string;
  name: string;
  group: string | null;
  status: ComponentStatus;
  createdAt: number | null;
}

interface Page {
  id: string;
  name: string;
  slug: string;
  visibility: StatusPageVisibility;
  description: string;
  logoUrl: string;
  emailSubscriptions: boolean;
  slackSubscriptions: boolean;
  visualization: StatusPageVisualization;
  components: Component[];
  notices: StatusPageNotice[];
  createdBy: string | null;
  updatedBy: string | null;
  createdAt: number;
  updatedAt: number;
}

let state: { pages: Page[]; seq: number } | null = null;

const uuid = () => crypto.randomUUID();
const componentId = () => `c_${Array.from(crypto.getRandomValues(new Uint8Array(12)), (b) => 'abcdefghijklmnopqrstuvwxyz0123456789'[b % 36]).join('')}`;

function db(): { pages: Page[]; seq: number } {
  if (state) return state;
  const now = Date.now();
  const s = { pages: [] as Page[], seq: 1 };
  const component = (name: string, group: string | null, status: ComponentStatus = 'operational'): Component => ({ id: componentId(), name, group, status, createdAt: null });
  const notice = (title: string, incidentId: number | null, steps: [number, NoticeStatus, string, NoticeImpact[]][]): StatusPageNotice => {
    const updates: NoticeUpdate[] = steps.map(([at, status, message, impacts]) => ({ id: s.seq++, status, message, impacts, createdBy: 'Priya Shah', createdAt: at })).reverse();
    const last = updates[0];
    return {
      id: uuid(),
      title,
      status: last.status,
      impacts: last.impacts,
      incidentId,
      createdBy: 'Priya Shah',
      startedAt: steps[0][0],
      resolvedAt: last.status === 'resolved' ? last.createdAt : null,
      updates,
    };
  };

  const storefront = component('Web storefront', 'Storefront');
  const checkout = component('Checkout', 'Storefront');
  const search = component('Product search', 'Storefront');
  const billing = component('Payments', 'Billing');
  const invoices = component('Invoices', 'Billing');
  const api = component('Public API', null);
  const inventory = component('Inventory sync', null);
  const hit = (c: Component, impact: ComponentStatus): NoticeImpact => ({ componentId: c.id, impact });
  const ago = (days: number, hours = 0) => now - days * DAY - hours * HOUR;
  s.pages.push({
    id: uuid(),
    name: 'Example Shop Status',
    slug: 'shop-status',
    visibility: 'public',
    description: 'Live status of the Example Shop storefront, checkout, payments and public API.',
    logoUrl: '',
    emailSubscriptions: true,
    slackSubscriptions: true,
    visualization: 'bars_and_uptime',
    components: [storefront, checkout, search, billing, invoices, api, inventory],
    notices: [
      notice('Inventory counts are updating slowly', null, [
        [ago(0, 3), 'investigating', 'Stock levels on product pages may lag behind by up to 15 minutes. We are looking into it.', [hit(inventory, 'degraded_performance')]],
        [ago(0, 1), 'monitoring', 'A backlog in the inventory sync is draining. Orders are not affected.', [hit(inventory, 'degraded_performance')]],
      ]),
      notice('Checkout errors for some customers', 80, [
        [ago(12, 5), 'investigating', 'Some customers see an error when placing an order. We are investigating.', [hit(checkout, 'major_outage'), hit(billing, 'partial_outage')]],
        [ago(12, 4), 'identified', 'A configuration change to the `order-service` caused the errors. We are rolling it back.', [hit(checkout, 'major_outage'), hit(billing, 'partial_outage')]],
        [ago(12, 3), 'monitoring', 'The rollback is complete and orders are going through. We are watching error rates.', [hit(checkout, 'degraded_performance')]],
        [ago(12, 2), 'resolved', 'Checkout has been working normally for an hour. Sorry for the trouble.', []],
      ]),
      notice('Search results missing products', null, [
        [ago(41, 9), 'identified', 'Search returns incomplete results while the catalog index is rebuilt.', [hit(search, 'partial_outage')]],
        [ago(41, 4), 'resolved', 'The index rebuild finished and search results are complete again.', []],
      ]),
      notice('Public API latency', null, [
        [ago(52, 6), 'investigating', 'API requests are slower than usual.', [hit(api, 'degraded_performance')]],
        [ago(52, 5), 'resolved', 'Latency is back to normal.', []],
      ]),
    ],
    createdBy: 'Priya Shah',
    updatedBy: 'Priya Shah',
    createdAt: ago(UPTIME_DAYS + 30),
    updatedAt: ago(0, 1),
  });

  const gateway = component('api-gateway', 'Edge');
  const orders = component('order-service', 'Services');
  const catalog = component('catalog-service', 'Services');
  const billingSvc = component('billing', 'Services');
  const queue = component('Order queue', 'Data', 'degraded_performance');
  s.pages.push({
    id: uuid(),
    name: 'Platform (internal)',
    slug: 'platform-internal',
    visibility: 'internal',
    description: 'Services behind the shop, for the engineering teams.',
    logoUrl: '',
    emailSubscriptions: false,
    slackSubscriptions: false,
    visualization: 'bars_and_uptime',
    components: [gateway, orders, catalog, billingSvc, queue],
    notices: [
      notice('Elevated latency on catalog-service', null, [
        [ago(5, 8), 'investigating', 'p95 latency of `catalog-service` doubled after the 14:00 deploy.', [hit(catalog, 'degraded_performance')]],
        [ago(5, 6), 'resolved', 'Rolled back to the previous version.', []],
      ]),
    ],
    createdBy: 'Daniel Kim',
    updatedBy: 'Daniel Kim',
    createdAt: ago(60),
    updatedAt: ago(2),
  });
  state = s;
  return s;
}

function find(id: string): Page {
  return db().pages.find((p) => p.id === id) ?? fail('Status page not found');
}

function required(value: string, what: string, max: number): string {
  const v = value.trim();
  if (!v) fail(`${what} is required`);
  if (v.length > max) fail(`${what} is at most ${max} characters`);
  return v;
}

/** Recent notices: open ones and those resolved in the last 60 days. */
const recent = (p: Page) => p.notices.filter((n) => n.resolvedAt == null || n.resolvedAt > Date.now() - UPTIME_DAYS * DAY).sort((a, b) => b.startedAt - a.startedAt);

function detail(p: Page): StatusPageDetail {
  const notices = recent(p).map((n) => ({ ...n, updates: [...n.updates] }));
  const components = currentComponents(p.components, notices);
  const { components: _, notices: __, ...fields } = p;
  return { ...fields, components, overallStatus: overallStatus(components), notices };
}

function checkPage(input: StatusPageInput, existing: Component[], self?: string): Omit<Page, 'id' | 'notices' | 'createdBy' | 'updatedBy' | 'createdAt' | 'updatedAt'> {
  const name = required(input.name, 'The name', MAX_NAME_CHARS);
  const slug = input.slug.trim();
  const slugError = checkSlug(slug);
  if (slugError) fail(slugError);
  if (db().pages.some((p) => p.slug === slug && p.id !== self)) fail(`The slug '${slug}' is already taken`);
  const description = (input.description ?? '').trim();
  if (description.length > MAX_DESCRIPTION_CHARS) fail(`The description is at most ${MAX_DESCRIPTION_CHARS} characters`);
  const logoUrl = (input.logoUrl ?? '').trim();
  const logoError = checkLogoUrl(logoUrl);
  if (logoError) fail(logoError);
  const list = input.components ?? [];
  if (list.length > MAX_COMPONENTS) fail(`A status page has at most ${MAX_COMPONENTS} components`);
  const names = new Set<string>();
  const ids = new Set<string>();
  const components = list.map((c): Component => {
    const n = required(c.name, 'The component name', MAX_NAME_CHARS);
    if (names.has(n.toLowerCase())) fail(`Component '${n.toLowerCase()}' appears twice`);
    names.add(n.toLowerCase());
    const id = c.id && existing.some((e) => e.id === c.id) && !ids.has(c.id) ? c.id : componentId();
    ids.add(id);
    return { id, name: n, group: c.group?.trim() || null, status: c.status ?? 'operational', createdAt: existing.find((e) => e.id === id)?.createdAt ?? Date.now() };
  });
  return {
    name,
    slug,
    visibility: input.visibility,
    description,
    logoUrl,
    emailSubscriptions: input.emailSubscriptions ?? false,
    slackSubscriptions: input.slackSubscriptions ?? false,
    visualization: input.visualization ?? 'bars_and_uptime',
    components,
  };
}

function checkImpacts(impacts: NoticeImpact[], components: Component[]) {
  const seen = new Set<string>();
  for (const i of impacts) {
    const c = components.find((x) => x.id === i.componentId) ?? fail(`Component '${i.componentId}' is not on this status page`);
    if (seen.has(c.id)) fail(`Component '${c.name}' is impacted twice`);
    seen.add(c.id);
    if (i.impact === 'operational') fail(`The impact on '${c.name}' must be degraded performance, partial outage or major outage`);
  }
}

function touch(p: Page) {
  p.updatedAt = Date.now();
  p.updatedBy = mockMe().name;
}

export async function listStatusPages(): Promise<StatusPageSummary[]> {
  return db()
    .pages.map((p) => {
      const d = detail(p);
      return {
        id: p.id,
        name: p.name,
        slug: p.slug,
        visibility: p.visibility,
        componentCount: p.components.length,
        overallStatus: d.overallStatus,
        openNotices: p.notices.filter((n) => n.resolvedAt == null).length,
        updatedBy: p.updatedBy,
        updatedAt: p.updatedAt,
      };
    })
    .sort((a, b) => a.name.toLowerCase().localeCompare(b.name.toLowerCase()) || a.slug.localeCompare(b.slug));
}

export async function createStatusPage(input: StatusPageInput): Promise<StatusPageDetail> {
  if (db().pages.length >= MAX_PAGES) fail(`An organization can have at most ${MAX_PAGES} status pages`);
  const fields = checkPage(input, []);
  const now = Date.now();
  const me = mockMe().name;
  const page: Page = { id: uuid(), ...fields, notices: [], createdBy: me, updatedBy: me, createdAt: now, updatedAt: now };
  db().pages.push(page);
  return detail(page);
}

export async function getStatusPage(id: string): Promise<StatusPageDetail> {
  return detail(find(id));
}

export async function updateStatusPage(id: string, input: StatusPageInput): Promise<StatusPageDetail> {
  const page = find(id);
  Object.assign(page, checkPage(input, page.components, id));
  touch(page);
  return detail(page);
}

export async function deleteStatusPage(id: string): Promise<void> {
  find(id);
  db().pages = db().pages.filter((p) => p.id !== id);
}

export async function createStatusPageNotice(id: string, input: NoticeInput): Promise<StatusPageDetail> {
  const page = find(id);
  const title = required(input.title, 'The title', MAX_TITLE_CHARS);
  const message = required(input.message, 'The message', MAX_MESSAGE_CHARS);
  const impacts = input.impacts ?? [];
  checkImpacts(impacts, page.components);
  if (input.incidentId != null) await getIncident(input.incidentId);
  const now = Date.now();
  const me = mockMe().name;
  page.notices.push({
    id: uuid(),
    title,
    status: input.status,
    impacts,
    incidentId: input.incidentId ?? null,
    createdBy: me,
    startedAt: now,
    resolvedAt: input.status === 'resolved' ? now : null,
    updates: [{ id: db().seq++, status: input.status, message, impacts, createdBy: me, createdAt: now }],
  });
  touch(page);
  return detail(page);
}

export async function updateStatusPageNotice(id: string, noticeId: string, input: NoticeUpdateInput): Promise<StatusPageDetail> {
  const page = find(id);
  const message = required(input.message, 'The message', MAX_MESSAGE_CHARS);
  const notice = page.notices.find((n) => n.id === noticeId) ?? fail('Notice not found');
  if (notice.resolvedAt != null) fail('The notice is resolved; open a new one instead');
  const impacts = input.impacts ?? notice.impacts;
  checkImpacts(impacts, page.components);
  const now = Date.now();
  notice.updates.unshift({ id: db().seq++, status: input.status, message, impacts, createdBy: mockMe().name, createdAt: now });
  notice.status = input.status;
  notice.impacts = impacts;
  notice.resolvedAt = input.status === 'resolved' ? now : null;
  touch(page);
  return detail(page);
}

export async function deleteStatusPageNotice(id: string, noticeId: string): Promise<void> {
  const page = find(id);
  if (!page.notices.some((n) => n.id === noticeId)) fail('Notice not found');
  page.notices = page.notices.filter((n) => n.id !== noticeId);
}

/** In mock mode the viewer is always a signed-in member, so internal pages render too. */
export async function getPublicStatusPage(slug: string): Promise<PublicStatusPage> {
  const page = db().pages.find((p) => p.slug === slug) ?? fail('Status page not found');
  const d = detail(page);
  return {
    name: d.name,
    slug: d.slug,
    visibility: d.visibility,
    description: d.description,
    logoUrl: d.logoUrl,
    emailSubscriptions: d.emailSubscriptions,
    slackSubscriptions: d.slackSubscriptions,
    visualization: d.visualization,
    components: d.components,
    overallStatus: d.overallStatus,
    notices: d.notices.map((n) => ({ ...n, incidentId: null, createdBy: null, updates: n.updates.map((u) => ({ ...u, createdBy: null })) })),
    updatedAt: d.updatedAt,
  };
}
