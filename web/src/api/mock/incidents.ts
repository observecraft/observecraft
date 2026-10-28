// In-browser incident management for mock mode: the e-commerce demo's incidents (IR-80 onwards,
// like the server's demo seed) with their timelines, response teams, pages, notifications,
// documents and postmortems. State lives in memory for the session. Semantics follow the server
// (routes/incidents.rs, incident_views.rs): search filters and facet counts, per-org IR numbers,
// and the timeline entries written automatically on declare and on status, severity and
// commander changes. People and teams come from the identity mock, so names match Settings.
import type { components } from '../generated/schema';
import { mockMe, mockTeam, mockUser, mockUserIds } from './identity';

type S = components['schemas'];

const MIN = 60_000;
const HOUR = 60 * MIN;
const DAY = 24 * HOUR;
const SEVERITIES = ['SEV-1', 'SEV-2', 'SEV-3', 'SEV-4', 'SEV-5', 'UNKNOWN'];
const DETECTION = ['monitor', 'customer', 'employee', 'automation', 'unknown'];
const ROLES = ['responder', 'communications_lead', 'scribe'];
const SYSTEM = 'ObserveCraft Incident Management';
const LABEL: Record<S['IncidentStatus'], string> = { active: 'Active', stable: 'Stable', resolved: 'Resolved' };

const fail = (message: string): never => {
  throw new Error(message);
};

function rng(seed: number): () => number {
  let s = seed >>> 0;
  return () => {
    s = (s + 0x6d2b79f5) >>> 0;
    let t = s;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

interface IncidentRow {
  id: number;
  title: string;
  status: S['IncidentStatus'];
  severity: string;
  incidentType: string;
  detectionMethod: string;
  commanderId: string | null;
  customerImpact: boolean;
  test: boolean;
  services: string[];
  teams: string[];
  detectedAt: number;
  declaredAt: number;
  acknowledgedAt: number | null;
  stableAt: number | null;
  resolvedAt: number | null;
  declaredBy: string;
  summary: string;
  rootCause: string;
  events: S['TimelineEvent'][];
  responders: { userId: string; role: string; addedAt: number }[];
  impacts: S['Impact'][];
  followups: S['Followup'][];
  pages: S['Page'][];
  notifications: S['Notification'][];
  documents: S['Document'][];
  postmortem: S['Postmortem'] | null;
}

/** (title, services, teams, type, detection), oldest first: the server's demo incidents. */
const SEED: [string, string[], string[], string, string][] = [
  ['[Checkout] Elevated 5xx on POST /orders/submit', ['order-service', 'api-gateway'], ['checkout'], 'Production Incident', 'monitor'],
  ['[Billing] Invoice generation delayed for EU customers', ['billing'], ['billing'], 'Production Incident', 'customer'],
  ['[Catalog] Stock levels stale after UPDATE_STOCK_PKG timeouts', ['catalog-service'], ['catalog'], 'EventBased-Alerts', 'monitor'],
  ['[Storefront] p95 latency above SLO on storefront-api', ['storefront-api'], ['storefront'], 'EventBased-Alerts', 'monitor'],
  ['[Notifications] Order emails queued in order-events', ['notification-worker'], ['notifications'], 'Production Incident', 'employee'],
  ['[Platform] Collector disk buffer filling on shop-prod-eks', [], ['platform'], 'EventBased-Alerts', 'monitor'],
  ['[Payments] Card provider returning intermittent timeouts', ['api-gateway', 'billing'], ['payments'], 'Production Incident', 'automation'],
  ['[Identity] SSO login loop for some users', [], ['identity'], 'Production Incident', 'customer'],
  ['[Search] Empty results for multi-word queries', ['catalog-service'], ['search'], 'Production Incident', 'employee'],
  ['[Inventory] Reservation conflicts during flash sale', ['inventory-service'], ['checkout'], 'Production Incident', 'monitor'],
  ['[Data Platform] Nightly export to warehouse failed', [], ['data-platform'], 'EventBased-Alerts', 'automation'],
  ['[Platform] api-gateway pods restarting after node rotation', ['api-gateway'], ['platform'], 'EventBased-Alerts', 'monitor'],
  ['[Mobile] Crash on launch for app 5.12.0 on iOS 18', [], ['mobile'], 'Production Incident', 'customer'],
  ['[SRE] Nightly database backup failed', [], ['sre'], 'EventBased-Alerts', 'monitor'],
  ['[SRE] Elevated error budget burn on api-gateway', ['api-gateway'], ['sre', 'platform'], 'EventBased-Alerts', 'monitor'],
  ['[Storefront] Images not loading from CDN in eu-central-1', ['storefront-api'], ['storefront'], 'Production Incident', 'customer'],
  ['[Billing] Duplicate invoices for annual plans', ['billing'], ['billing'], 'Production Incident', 'employee'],
  ['[Checkout] Redis cache evictions slowing customer lookups', ['order-service'], ['checkout'], 'EventBased-Alerts', 'monitor'],
  ['[Security] Unusual login attempts from new ASN', [], ['security', 'identity'], 'Security Incident', 'automation'],
  ['[Catalog] Pricing returning stale prices in catalog-service', ['catalog-service', 'inventory-service'], ['catalog'], 'Production Incident', 'monitor'],
];
const NOTES = [
  'Looking at the error spike — started right after the 3.2.1 rollout.',
  'Rolled back to the previous version; error rate is dropping.',
  'Customer support confirmed reports from several merchants.',
  'Increased the connection pool size as a mitigation.',
  'Root cause looks like an N+1 query introduced last week.',
  'Monitoring for 30 minutes before marking stable.',
  'Paging the owning team for a second pair of eyes.',
  'Dashboards linked in the channel; latency back under SLO.',
];
const FOLLOWUPS = ['Add a monitor for queue depth', 'Write the post-incident review', 'Load-test the checkout path before the next sale', 'Add a runbook link to the alert'];

let store: { incidents: IncidentRow[]; seq: number } | null = null;

function db() {
  if (store) return store;
  const r = rng(0x0b5ec0de);
  const pick = <T>(items: readonly T[]): T => items[Math.floor(r() * items.length)];
  const now = Date.now();
  const me = mockMe();
  const people = mockUserIds();
  const others = people.filter((id) => id !== me.id);
  const s = { incidents: [] as IncidentRow[], seq: 1 };
  const n = SEED.length;
  SEED.forEach(([title, services, teams, incidentType, detectionMethod], i) => {
    const fromEnd = n - i;
    const status: S['IncidentStatus'] = fromEnd <= 3 ? 'active' : fromEnd === 4 ? 'stable' : 'resolved';
    const severity = pick(['SEV-1', 'SEV-2', 'SEV-2', 'SEV-3', 'SEV-3', 'SEV-3', 'SEV-4']);
    // The mock user commands a few incidents and responds to others, so "My Incidents" isn't empty.
    const commanderId = i % 6 === 1 ? me.id : pick(others);
    const detectedAt = Math.round(now - (fromEnd * 3 - r() * 2) * DAY);
    const declaredAt = detectedAt + Math.round((1 + r() * 10) * MIN);
    const acknowledgedAt = declaredAt + Math.round((1 + r() * 5) * MIN);
    const resolveAt = Math.min(now - 10 * MIN, declaredAt + Math.round(30 * MIN + r() * 2 * DAY));
    const stableAt = status === 'active' ? null : Math.min(now - 20 * MIN, declaredAt + Math.round((resolveAt - declaredAt) * 0.7));
    const automated = detectionMethod === 'monitor' || detectionMethod === 'automation';
    const inc: IncidentRow = {
      id: 80 + i,
      title,
      status,
      severity,
      incidentType,
      detectionMethod,
      commanderId,
      customerImpact: r() < 0.4,
      test: false,
      services,
      teams,
      detectedAt,
      declaredAt,
      acknowledgedAt: Math.min(acknowledgedAt, now),
      stableAt,
      resolvedAt: status === 'resolved' ? resolveAt : null,
      declaredBy: automated ? SYSTEM : me.name,
      summary: r() < 0.6 ? `${title}. Impact is being assessed; see the timeline for updates.` : '',
      rootCause: status === 'resolved' && r() < 0.7 ? pick(NOTES) : '',
      events: [],
      responders: [],
      impacts: [],
      followups: [],
      pages: [],
      notifications: [],
      documents: [],
      postmortem: null,
    };
    const ev = (kind: string, author: string, authorId: string | null, body: string, createdAt: number) =>
      inc.events.push({ id: s.seq++, kind, author, authorId, body, important: false, createdAt: Math.min(createdAt, now) });
    ev('declared', automated ? SYSTEM : me.name, automated ? null : me.id, `Incident declared with severity ${severity}`, declaredAt);
    const channels = teams.flatMap((t) => mockTeam(t)?.notifications ?? []);
    if (channels.length) {
      const subject = `Incident #${inc.id}: ${title}`;
      inc.notifications.push({
        id: s.seq++,
        recipients: channels,
        subject,
        body: `Status: Active | Severity: ${severity} | Commander: ${mockUser(commanderId)?.name}`,
        automated: true,
        author: SYSTEM,
        createdAt: declaredAt + 5000,
      });
      ev('notification', SYSTEM, null, `${subject} — sent to ${channels.join(', ')}`, declaredAt + 5000);
    }
    const responders = [...new Set([...(i % 4 === 2 ? [me.id] : []), ...Array.from({ length: Math.floor(r() * 3) }, () => pick(others))])].filter((id) => id !== commanderId);
    for (const userId of responders) inc.responders.push({ userId, role: pick(['responder', 'responder', 'scribe', 'communications_lead']), addedAt: acknowledgedAt });
    const writers = [commanderId, ...responders];
    const notes = 1 + Math.floor(r() * 4);
    for (let k = 0; k < notes; k++) {
      const who = mockUser(pick(writers))!;
      const at = acknowledgedAt + k * 15 * MIN;
      if (status === 'active' || at < (stableAt ?? now)) ev('note', who.name, who.id, NOTES[(i + k) % NOTES.length], at);
    }
    if (inc.customerImpact) {
      inc.impacts.push({ id: s.seq++, description: `Customers saw errors or delays on ${services[0] ?? teams[0]}`, startAt: detectedAt, endAt: inc.resolvedAt });
      ev('impact', mockUser(commanderId)!.name, null, `Impact added: Customers saw errors or delays on ${services[0] ?? teams[0]}`, acknowledgedAt + MIN);
    }
    if (status === 'active') {
      inc.pages.push({
        id: s.seq++,
        team: teams[0],
        title,
        status: 'acknowledged',
        createdBy: mockUser(commanderId)!.name,
        createdAt: acknowledgedAt,
        acknowledgedBy: mockUser(pick(others))!.name,
        acknowledgedAt: acknowledgedAt + 4 * MIN,
        resolvedAt: null,
      });
      ev('notification', mockUser(commanderId)!.name, commanderId, `Paged @${teams[0]}: ${title}`, acknowledgedAt);
    }
    const commander = mockUser(commanderId)!;
    if (stableAt) ev('status', commander.name, commander.id, 'Status changed from Active to Stable', stableAt);
    if (inc.resolvedAt) ev('status', commander.name, commander.id, 'Status changed from Stable to Resolved', inc.resolvedAt);
    if (status === 'resolved' && r() < 0.5) inc.followups.push({ id: s.seq++, title: pick(FOLLOWUPS), done: r() < 0.5 });
    if (status !== 'active' && r() < 0.4)
      inc.documents.push({ id: s.seq++, title: 'Incident channel notes', url: `https://wiki.example.com/incidents/ir-${inc.id}`, createdBy: commander.name, createdAt: acknowledgedAt + 10 * MIN });
    inc.events.sort((a, b) => a.createdAt - b.createdAt || a.id - b.id);
    s.incidents.push(inc);
  });
  store = s;
  return s;
}

function find(id: number): IncidentRow {
  return db().incidents.find((i) => i.id === id) ?? fail(`Incident IR-${id} not found`);
}

const userName = (id: string) => mockUser(id)?.name ?? 'Unknown user';

function event(inc: IncidentRow, kind: string, body: string, author = mockMe()): void {
  inc.events.push({ id: db().seq++, kind, author: author.name, authorId: author.id, body, important: false, createdAt: Date.now() });
}

function text(s: string, what: string, max: number): string {
  const t = s.trim();
  if (!t || [...t].length > max) fail(`${what} must be 1–${max} characters`);
  return t;
}

const checkTitle = (t: string) => text(t, 'title', 300);
const checkSeverity = (s: string) => SEVERITIES.includes(s) || fail(`severity must be one of ${SEVERITIES.join(', ')}`);

function summaryOf(i: IncidentRow): S['IncidentSummary'] {
  return {
    id: i.id,
    title: i.title,
    status: i.status,
    severity: i.severity,
    incidentType: i.incidentType,
    detectionMethod: i.detectionMethod,
    commander: i.commanderId ? { id: i.commanderId, name: userName(i.commanderId) } : null,
    customerImpact: i.customerImpact,
    test: i.test,
    services: [...i.services],
    teams: [...i.teams],
    detectedAt: i.detectedAt,
    declaredAt: i.declaredAt,
    resolvedAt: i.resolvedAt,
  };
}

function detailOf(i: IncidentRow): S['IncidentDetail'] {
  return {
    ...summaryOf(i),
    summary: i.summary,
    rootCause: i.rootCause,
    declaredBy: i.declaredBy,
    acknowledgedAt: i.acknowledgedAt,
    stableAt: i.stableAt,
    responders: i.responders.map((x) => ({ id: x.userId, name: userName(x.userId) })).sort((a, b) => a.name.localeCompare(b.name)),
    impacts: [...i.impacts].sort((a, b) => a.startAt - b.startAt).map((x) => ({ ...x })),
    followups: i.followups.map((x) => ({ ...x })),
    notifications: i.events
      .filter((e) => e.kind === 'notification')
      .sort((a, b) => b.createdAt - a.createdAt || b.id - a.id)
      .slice(0, 5)
      .map((e) => ({ ...e })),
  };
}

// ================================================================ search

type Facet = 'severity' | 'team' | 'service' | 'detection' | 'type' | 'impact';

function matches(i: IncidentRow, req: S['IncidentSearchRequest'], skip?: Facet): boolean {
  if (i.declaredAt < req.range.from || i.declaredAt > req.range.to) return false;
  const q = (req.query ?? '').trim();
  if (q) {
    const needle = q.toLowerCase();
    const num = q.replace(/^(IR-|ir-)/, '');
    const hit =
      i.title.toLowerCase().includes(needle) || i.services.join(' ').toLowerCase().includes(needle) || i.teams.join(' ').toLowerCase().includes(needle) || (/^\d+$/.test(num) && Number(num) === i.id);
    if (!hit) return false;
  }
  if (req.severities?.length && skip !== 'severity' && !req.severities.includes(i.severity)) return false;
  if (req.teams?.length && skip !== 'team' && !i.teams.some((t) => req.teams!.includes(t))) return false;
  if (req.services?.length && skip !== 'service' && !i.services.some((x) => req.services!.includes(x))) return false;
  if (req.detectionMethods?.length && skip !== 'detection' && !req.detectionMethods.includes(i.detectionMethod)) return false;
  if (req.incidentTypes?.length && skip !== 'type' && !req.incidentTypes.includes(i.incidentType)) return false;
  if (req.customerImpact != null && skip !== 'impact' && i.customerImpact !== req.customerImpact) return false;
  if (req.mine) {
    const me = mockMe().id;
    if (i.commanderId !== me && !i.responders.some((x) => x.userId === me)) return false;
  }
  return true;
}

const statusOk = (i: IncidentRow, req: S['IncidentSearchRequest']) => !req.statuses?.length || req.statuses.includes(i.status);

export async function searchIncidents(req: S['IncidentSearchRequest']): Promise<S['IncidentSearchPage']> {
  const all = db().incidents;
  const hits = all.filter((i) => matches(i, req) && statusOk(i, req)).sort((a, b) => Number(a.status === 'resolved') - Number(b.status === 'resolved') || b.declaredAt - a.declaredAt);
  const counts = { active: 0, stable: 0, resolved: 0 };
  for (const i of all) if (matches(i, req)) counts[i.status]++;
  // Each facet's counts honour every filter but its own.
  const facetCounts = (facet: Facet, values: (i: IncidentRow) => string[]) => {
    const out: Record<string, number> = {};
    for (const i of all) if (matches(i, req, facet) && statusOk(i, req)) for (const v of values(i)) out[v] = (out[v] ?? 0) + 1;
    return out;
  };
  const severityCounts = facetCounts('severity', (i) => [i.severity]);
  const teamCounts = facetCounts('team', (i) => i.teams);
  const offset = Math.max(0, req.offset);
  const limit = Math.min(200, Math.max(1, req.limit));
  return {
    incidents: hits.slice(offset, offset + limit).map(summaryOf),
    total: hits.length,
    counts,
    severityCounts,
    teamCounts,
    serviceCounts: facetCounts('service', (i) => i.services),
    detectionCounts: facetCounts('detection', (i) => [i.detectionMethod]),
    typeCounts: facetCounts('type', (i) => [i.incidentType]),
    impactCounts: facetCounts('impact', (i) => [i.customerImpact ? 'yes' : 'no']),
  };
}

// ================================================================ detail, declare, update

export async function getIncident(id: number): Promise<S['IncidentDetail']> {
  return detailOf(find(id));
}

export async function declareIncident(req: S['DeclareRequest']): Promise<S['IncidentDetail']> {
  const title = checkTitle(req.title);
  checkSeverity(req.severity);
  const detection = req.detectionMethod ?? 'employee';
  if (!DETECTION.includes(detection)) fail(`detectionMethod must be one of ${DETECTION.join(', ')}`);
  const s = db();
  const me = mockMe();
  const commander = req.commanderId ?? me.id;
  const now = Date.now();
  const inc: IncidentRow = {
    id: s.incidents.reduce((max, i) => Math.max(max, i.id), 0) + 1,
    title,
    status: 'active',
    severity: req.severity,
    incidentType: (req.incidentType ?? 'Production Incident').trim(),
    detectionMethod: detection,
    commanderId: mockUser(commander) ? commander : null,
    customerImpact: req.customerImpact ?? false,
    test: req.test ?? false,
    services: [...(req.services ?? [])],
    teams: [...(req.teams ?? [])],
    detectedAt: now,
    declaredAt: now,
    acknowledgedAt: null,
    stableAt: null,
    resolvedAt: null,
    declaredBy: me.name,
    summary: (req.summary ?? '').trim(),
    rootCause: '',
    events: [],
    responders: [],
    impacts: [],
    followups: [],
    pages: [],
    notifications: [],
    documents: [],
    postmortem: null,
  };
  s.incidents.push(inc);
  event(inc, 'declared', `Incident declared with severity ${req.severity}`);
  // Teams with notification channels hear about their incidents automatically.
  const recipients = inc.teams.flatMap((t) => mockTeam(t)?.notifications ?? []);
  if (recipients.length) {
    const subject = `Incident #${inc.id}: ${title}`;
    inc.notifications.push({ id: s.seq++, recipients, subject, body: `Status: Active | Severity: ${req.severity} | Commander: ${me.name}`, automated: true, author: SYSTEM, createdAt: now });
    inc.events.push({ id: s.seq++, kind: 'notification', author: SYSTEM, authorId: null, body: `${subject} — sent to ${recipients.join(', ')}`, important: false, createdAt: now });
  }
  return detailOf(inc);
}

export async function updateIncident(id: number, p: S['IncidentPatch']): Promise<S['IncidentDetail']> {
  const inc = find(id);
  const title = p.title != null ? checkTitle(p.title) : null;
  if (p.severity != null) checkSeverity(p.severity);
  const now = Date.now();
  if (p.status && p.status !== inc.status) {
    event(inc, 'status', `Status changed from ${LABEL[inc.status]} to ${LABEL[p.status]}`);
    inc.stableAt = p.status === 'active' ? null : (inc.stableAt ?? now);
    inc.resolvedAt = p.status === 'resolved' ? now : null;
    inc.status = p.status;
  }
  if (p.severity != null && p.severity !== inc.severity) event(inc, 'severity', `Severity changed from ${inc.severity} to ${p.severity}`);
  if (p.commanderId != null && p.commanderId !== inc.commanderId) event(inc, 'commander', `${userName(p.commanderId)} is now the incident commander`);
  if (title != null) inc.title = title;
  if (p.severity != null) inc.severity = p.severity;
  if (p.commanderId != null && mockUser(p.commanderId)) inc.commanderId = p.commanderId;
  if (p.summary != null) inc.summary = p.summary.trim();
  if (p.rootCause != null) inc.rootCause = p.rootCause.trim();
  if (p.services != null) inc.services = [...p.services];
  if (p.teams != null) inc.teams = [...p.teams];
  if (p.customerImpact != null) inc.customerImpact = p.customerImpact;
  inc.acknowledgedAt ??= now;
  return detailOf(inc);
}

// ================================================================ timeline, impacts, follow-ups

export async function getTimeline(id: number): Promise<S['TimelineEvent'][]> {
  return [...find(id).events].sort((a, b) => a.createdAt - b.createdAt || a.id - b.id).map((e) => ({ ...e }));
}

export async function addNote(id: number, body: string): Promise<void> {
  const b = body.trim();
  if (!b || [...b].length > 10_000) fail('note must be 1–10000 characters');
  const inc = find(id);
  event(inc, 'note', b);
  inc.acknowledgedAt ??= Date.now();
}

export async function setImportant(id: number, eventId: number, important: boolean): Promise<void> {
  const e = find(id).events.find((x) => x.id === eventId) ?? fail('Timeline entry not found');
  e.important = important;
}

export async function addImpact(id: number, description: string, startAt: number, endAt?: number | null): Promise<void> {
  const desc = checkTitle(description);
  if (endAt != null && endAt < startAt) fail('impact end must be after its start');
  const inc = find(id);
  inc.impacts.push({ id: db().seq++, description: desc, startAt, endAt: endAt ?? null });
  inc.customerImpact = true;
  inc.events.push({ id: db().seq++, kind: 'impact', author: mockMe().name, authorId: null, body: `Impact added: ${desc}`, important: false, createdAt: Date.now() });
}

export async function addFollowup(id: number, title: string): Promise<void> {
  const t = checkTitle(title);
  find(id).followups.push({ id: db().seq++, title: t, done: false });
}

export async function setFollowupDone(id: number, followupId: number, done: boolean): Promise<void> {
  const f = find(id).followups.find((x) => x.id === followupId) ?? fail(`Follow-up ${followupId} not found`);
  f.done = done;
}

// ================================================================ response team

export async function listResponders(id: number): Promise<S['TeamMember'][]> {
  const inc = find(id);
  const lastActive = (userId: string) => {
    const at = inc.events.filter((e) => e.authorId === userId).map((e) => e.createdAt);
    return at.length ? Math.max(...at) : null;
  };
  const team = [
    ...(inc.commanderId ? [{ userId: inc.commanderId, role: 'incident_commander', addedAt: inc.declaredAt }] : []),
    ...inc.responders.filter((x) => x.userId !== inc.commanderId).sort((a, b) => a.addedAt - b.addedAt),
  ];
  return team.filter((m) => mockUser(m.userId)).map((m) => ({ userId: m.userId, name: userName(m.userId), role: m.role, addedAt: m.addedAt, lastActiveAt: lastActive(m.userId) }));
}

export async function addResponder(id: number, userId: string, role: string): Promise<void> {
  if (!ROLES.includes(role)) fail(`role must be one of ${ROLES.join(', ')}`);
  const inc = find(id);
  const user = mockUser(userId);
  if (!user || user.status !== 'active') fail('unknown or inactive user');
  const existing = inc.responders.find((x) => x.userId === userId);
  if (existing) existing.role = role;
  else inc.responders.push({ userId, role, addedAt: Date.now() });
  event(inc, 'note', `Added ${user!.name} to the response team as ${role.replace(/_/g, ' ')}`);
}

export async function removeResponder(id: number, userId: string): Promise<void> {
  const inc = find(id);
  if (!inc.responders.some((x) => x.userId === userId)) fail('Not on the response team');
  inc.responders = inc.responders.filter((x) => x.userId !== userId);
  event(inc, 'note', `Removed ${userName(userId)} from the response team`);
}

// ================================================================ pages and notifications

export async function listPages(id: number): Promise<S['Page'][]> {
  return [...find(id).pages].sort((a, b) => b.createdAt - a.createdAt || b.id - a.id).map((x) => ({ ...x }));
}

export async function createPage(id: number, team: string, title: string): Promise<S['Page']> {
  const inc = find(id);
  const t = text(title, 'title', 300);
  if (!mockTeam(team)) fail(`unknown team '${team}'`);
  const page: S['Page'] = { id: db().seq++, team, title: t, status: 'triggered', createdBy: mockMe().name, createdAt: Date.now(), acknowledgedBy: null, acknowledgedAt: null, resolvedAt: null };
  inc.pages.push(page);
  event(inc, 'notification', `Paged @${team}: ${t}`);
  return { ...page };
}

export async function updatePage(id: number, pageId: number, status: 'acknowledged' | 'resolved'): Promise<void> {
  const inc = find(id);
  const page = inc.pages.find((x) => x.id === pageId);
  const now = Date.now();
  const me = mockMe().name;
  if (status === 'acknowledged' && page?.status === 'triggered') Object.assign(page, { status, acknowledgedBy: me, acknowledgedAt: now });
  else if (status === 'resolved' && page && page.status !== 'resolved')
    Object.assign(page, { status, resolvedAt: now, acknowledgedBy: page.acknowledgedBy ?? me, acknowledgedAt: page.acknowledgedAt ?? now });
  else if (status !== 'acknowledged' && status !== 'resolved') fail('status must be acknowledged or resolved');
  else fail('Page not found or already in that state');
  event(inc, 'note', `Page ${pageId} ${status}`);
}

export async function listNotifications(id: number): Promise<S['Notification'][]> {
  return [...find(id).notifications].sort((a, b) => b.createdAt - a.createdAt || b.id - a.id).map((x) => ({ ...x, recipients: [...x.recipients] }));
}

export async function createNotification(id: number, recipients: string[], subject: string, body: string): Promise<void> {
  const inc = find(id);
  const to = recipients.map((x) => x.trim()).filter(Boolean);
  if (to.length === 0 || to.length > 50 || to.some((x) => x.length > 254 || /\s/.test(x))) fail('add 1–50 recipients (emails, #channels or @handles)');
  const subj = text(subject, 'subject', 300);
  inc.notifications.push({ id: db().seq++, recipients: to, subject: subj, body: [...body.trim()].slice(0, 5000).join(''), automated: false, author: mockMe().name, createdAt: Date.now() });
  event(inc, 'notification', `${subj} — sent to ${to.join(', ')}`);
}

// ================================================================ documents

export async function listDocuments(id: number): Promise<S['Document'][]> {
  return [...find(id).documents].sort((a, b) => a.createdAt - b.createdAt).map((x) => ({ ...x }));
}

export async function addDocument(id: number, title: string, url: string): Promise<void> {
  const inc = find(id);
  const t = text(title, 'title', 200);
  let link: URL | null = null;
  try {
    link = new URL(url.trim());
  } catch {
    link = null;
  }
  // Only web links: these render as anchors in the UI.
  if (!link || (link.protocol !== 'http:' && link.protocol !== 'https:')) fail('link must be an http(s) URL');
  inc.documents.push({ id: db().seq++, title: t, url: link!.href, createdBy: mockMe().name, createdAt: Date.now() });
  event(inc, 'note', `Linked document "${t}": ${link!.href}`);
}

export async function removeDocument(id: number, docId: number): Promise<void> {
  const inc = find(id);
  if (!inc.documents.some((d) => d.id === docId)) fail('Document not found');
  inc.documents = inc.documents.filter((d) => d.id !== docId);
}

// ================================================================ postmortem

const pad = (n: number) => String(n).padStart(2, '0');
function fmtTime(ms: number): string {
  const d = new Date(ms);
  return `${d.getUTCFullYear()}-${pad(d.getUTCMonth() + 1)}-${pad(d.getUTCDate())} ${pad(d.getUTCHours())}:${pad(d.getUTCMinutes())} UTC`;
}
function fmtDuration(ms: number): string {
  const m = Math.floor(ms / MIN);
  if (m < 60) return `${m} min`;
  if (m < 1440) return `${Math.floor(m / 60)}h ${m % 60}m`;
  return `${Math.floor(m / 1440)}d ${Math.floor((m % 1440) / 60)}h`;
}

export async function getPostmortem(id: number): Promise<S['Postmortem'] | null> {
  const pm = find(id).postmortem;
  return pm ? { ...pm } : null;
}

/** Fills a postmortem from the incident, like the server: properties, impact, root cause, timeline, follow-ups. */
export async function generatePostmortem(id: number, template: 'standard' | 'brief'): Promise<S['Postmortem']> {
  if (template !== 'standard' && template !== 'brief') fail('template must be standard or brief');
  const inc = find(id);
  const brief = template === 'brief';
  const todo = '_To be completed._';
  const or = (v: string) => v.trim() || todo;
  const end = inc.resolvedAt ?? Date.now();
  let md = `# Postmortem IR-${inc.id}: ${inc.title}\n\n## Overview\n\n| | |\n|---|---|\n`;
  const rows: [string, string][] = [
    ['Status', inc.status],
    ['Severity', inc.severity],
    ['Type', inc.incidentType],
    ['Commander', inc.commanderId ? userName(inc.commanderId) : 'Unassigned'],
    ['Detected', fmtTime(inc.detectedAt)],
    ['Declared', fmtTime(inc.declaredAt)],
    ['Resolved', inc.resolvedAt ? fmtTime(inc.resolvedAt) : 'Not resolved'],
    ['Duration', fmtDuration(end - inc.declaredAt)],
    ['Services', inc.services.length ? inc.services.join(', ') : '—'],
    ['Teams', inc.teams.length ? inc.teams.join(', ') : '—'],
    ['Customer impact', inc.customerImpact ? 'Yes' : 'No'],
  ];
  for (const [k, v] of rows) md += `| ${k} | ${v.replace(/\|/g, '\\|')} |\n`;
  md += `\n${or(inc.summary)}\n\n## What Happened?\n\n### Impact on Customers\n\n`;
  if (inc.impacts.length === 0) md += inc.customerImpact ? '_Customers were impacted; describe how._\n' : 'No customer impact recorded.\n';
  else for (const x of [...inc.impacts].sort((a, b) => a.startAt - b.startAt)) md += `- ${x.description}\n`;
  md += `\n## Why Did it Happen?\n\n### Root Cause\n\n${or(inc.rootCause)}\n`;
  if (!brief) {
    md += '\n## Timeline\n\n';
    for (const e of [...inc.events].sort((a, b) => a.createdAt - b.createdAt || a.id - b.id))
      md += `- **${fmtTime(e.createdAt)}** — ${e.author} (${e.kind})${e.important ? ' ⭐' : ''}: ${e.body.replace(/\n/g, ' ')}\n`;
  }
  md += '\n## How do we prevent it in the future?\n\n### Action Items\n\n';
  if (inc.followups.length === 0) md += '_No follow-ups yet._\n';
  else for (const f of inc.followups) md += `- [${f.done ? 'x' : ' '}] ${f.title}\n`;
  if (!brief) {
    md += `\n## Lessons Learned\n\n${todo}\n`;
    if (inc.documents.length) {
      md += '\n## Related Documents\n\n';
      for (const d of inc.documents) md += `- [${d.title}](${d.url})\n`;
    }
  }
  inc.postmortem = { template, content: md, createdBy: inc.postmortem?.createdBy ?? mockMe().name, updatedAt: Date.now() };
  event(inc, 'note', 'Generated the postmortem');
  return { ...inc.postmortem };
}

export async function savePostmortem(id: number, content: string): Promise<void> {
  if (content.length > 200_000) fail('postmortem is too long');
  const inc = find(id);
  if (!inc.postmortem) fail('Generate the postmortem first');
  inc.postmortem = { ...inc.postmortem!, content, updatedAt: Date.now() };
}
