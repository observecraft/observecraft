// In-browser Software Catalog and Reports for mock mode (Developer Portal). Services, endpoints and
// error types come from the mock APM; datastores, queues and external providers from fixed
// fictional span facts run through the same inference as the server; definitions live in memory;
// service metadata is the scorecards mock's (like the server's shared table). Merge and report
// aggregations follow routes/portal_model.rs and routes/portal_reports.rs.
import { assembleCatalog, entitiesPerTeam, inferEntities, metadataOf, parseJsonDocuments, validateDefinition, type PeerCall } from '../portalCatalog/logic';
import { DAY, scorecardTrend, servicesNamed, teamsReport } from '../portalCatalog/reports';
import type {
  EntityDefinition,
  PortalCatalog,
  PortalCatalogRequest,
  PortalDefinitionItem,
  PortalImportResult,
  PortalReportDashboard,
  PortalReportRequest,
  PortalScorecardsReport,
  PortalTeamsReport,
} from '../portalCatalog/types';
import { referencesService } from '../scorecards/logic';
import { listEnvs, listResources, listServices, listSpanErrors } from './apm';
import { ciRepositories } from './ci';
import { getDashboard } from './dashboards';
import { mockMyTeams, mockTeamRoster } from './identity';
import { mockIncidentFacts } from './incidents';
import { searchMonitors } from './monitors';
import { listSchedules } from './onCall';
import { getScorecards, mockServiceMetadata, setMockServiceMetadata } from './scorecards';
import { searchSlos } from './slos';

// Calls per second of the fictional peers (scaled by the range).
const PEERS: (Omit<PeerCall, 'hits' | 'errors' | 'p95Ms'> & { rps: number; errorRate: number; p95Ms: number })[] = [
  { service: 'order-service', dbSystem: 'postgresql', dbName: 'orders-db', peerService: 'postgres', rps: 6.2, errorRate: 0.002, p95Ms: 8.4 },
  { service: 'billing', dbSystem: 'postgresql', dbName: 'orders-db', peerService: 'postgres', rps: 1.1, errorRate: 0, p95Ms: 11.2 },
  { service: 'billing', dbSystem: 'postgresql', dbName: 'billing-db', peerService: 'postgres', rps: 2.4, errorRate: 0.001, p95Ms: 6.9 },
  { service: 'catalog-service', dbSystem: 'postgresql', dbName: 'catalog-db', peerService: 'postgres', rps: 9.8, errorRate: 0.004, p95Ms: 14.5 },
  { service: 'storefront-api', dbSystem: 'redis', dbName: '0', peerService: 'cart-cache', rps: 18.3, errorRate: 0, p95Ms: 0.9 },
  { service: 'inventory-service', dbSystem: 'redis', dbName: '0', peerService: 'cart-cache', rps: 7.5, errorRate: 0.01, p95Ms: 1.2 },
  { service: 'workflow-engine', messagingSystem: 'kafka', destination: 'order-events', peerService: 'kafka', rps: 3.1, errorRate: 0, p95Ms: 4.1 },
  { service: 'order-service', messagingSystem: 'kafka', destination: 'order-events', peerService: 'kafka', rps: 2.9, errorRate: 0, p95Ms: 3.6 },
  { service: 'notification-worker', messagingSystem: 'kafka', destination: 'order-events', rps: 6.0, errorRate: 0, p95Ms: 2.2 },
  { service: 'billing', peerService: 'payments-provider', rps: 1.3, errorRate: 0.03, p95Ms: 420 },
  { service: 'notification-worker', peerService: 'email-provider', rps: 0.8, errorRate: 0.02, p95Ms: 310 },
];
// Services named only on logs or metrics (not instrumented).
const LOG_ONLY = ['cart-sync-job'];
const METRIC_ONLY = ['image-resizer'];

const SEED: EntityDefinition[] = [
  {
    apiVersion: 'v3',
    kind: 'system',
    metadata: { name: 'checkout', displayName: 'Checkout', owner: 'checkout', description: 'Everything between the cart and a paid order.' },
    spec: { components: ['service:order-service', 'service:inventory-service', 'datastore:orders-db', 'queue:order-events'] },
  },
  {
    apiVersion: 'v3',
    kind: 'datastore',
    metadata: { name: 'orders-db', owner: 'checkout', description: 'Orders and order items (PostgreSQL).', tags: ['tier:1'] },
    spec: { lifecycle: 'production', tier: '1', type: 'postgres', componentOf: ['system:checkout'] },
  },
  {
    apiVersion: 'v3',
    kind: 'queue',
    metadata: { name: 'order-events', owner: 'platform', description: 'Order lifecycle events for fulfillment and notifications.' },
    spec: { lifecycle: 'production', type: 'kafka', componentOf: ['system:checkout'] },
  },
  {
    apiVersion: 'v3',
    kind: 'datastore',
    metadata: { name: 'cart-cache', owner: 'storefront', description: 'Shopping carts and stock counters (Redis).' },
    spec: { lifecycle: 'production', type: 'redis' },
  },
  {
    apiVersion: 'v3',
    kind: 'api',
    metadata: {
      name: 'orders-api',
      displayName: 'Orders API',
      owner: 'checkout',
      description: 'Public REST API to place and read orders.',
      links: [{ name: 'OpenAPI spec', type: 'doc', url: 'https://docs.example.com/apis/orders' }],
    },
    spec: { lifecycle: 'production', type: 'openapi', implementedBy: ['service:order-service'] },
  },
  {
    apiVersion: 'v3',
    kind: 'frontend',
    metadata: {
      name: 'storefront-web',
      displayName: 'Storefront web',
      owner: 'storefront',
      description: 'The customer-facing shop.',
      links: [{ name: 'Source', type: 'repo', url: 'https://git.example.com/example-org/storefront-web' }],
      tags: ['tier:1'],
    },
    spec: { lifecycle: 'production', tier: '1', languages: ['javascript'], dependsOn: ['service:storefront-api'] },
  },
];

interface Stored extends PortalDefinitionItem {}

let defs: Stored[] | null = null;
let seq = 0;
let reportDashboards: string[] = [];

function db(): Stored[] {
  defs ??= SEED.map((d) => validateDefinition(d)).map((definition) => ({
    id: uuid(),
    kind: definition.kind,
    name: definition.metadata.name,
    source: 'ui',
    definition,
    updatedAt: Date.now() - 3 * DAY,
  }));
  return defs;
}

const uuid = () => `00000000-0000-4000-9000-${String(++seq).padStart(12, '0')}`;
const fail = (message: string): never => {
  throw new Error(message);
};

/** Starts the mock over (tests). */
export function resetPortalCatalogMock(): void {
  defs = null;
  reportDashboards = [];
}

async function allMonitors() {
  return searchMonitors({ query: '', statuses: [], muted: [], priorities: [], types: [], drafts: [false], recipients: [], creators: [], teams: [], sort: 'name', offset: 0, limit: 500 }).monitors;
}

async function allSlos() {
  return (await searchSlos({ query: '', offset: 0, limit: 500, burnRates: [], creators: [], envs: [], services: [], states: [], tags: [], teams: [], types: [], sort: null, window: null })).slos;
}

export async function getPortalCatalog(req: PortalCatalogRequest): Promise<PortalCatalog> {
  const scope = { range: req.range, env: req.env ?? null };
  const services = listServices(scope).filter((s) => s.instrumented);
  const instrumented = new Set(services.map((s) => s.service));
  const secs = Math.max(1, (req.range.to - req.range.from) / 1000);
  const peers = inferEntities(
    PEERS.map(({ rps, errorRate, ...p }) => {
      const hits = Math.round(rps * secs);
      return { ...p, hits, errors: Math.round(hits * errorRate) };
    }),
    instrumented,
  );
  const errorTypes: Record<string, number> = {};
  for (const [service, types] of Object.entries(listSpanErrors(scope).reduce<Record<string, Set<string>>>((acc, g) => ((acc[g.service] ??= new Set()).add(g.errorType), acc), {})))
    errorTypes[service] = types.size;
  const [monitors, slos] = await Promise.all([allMonitors(), allSlos()]);
  const meta = mockServiceMetadata();
  const entities = assembleCatalog({
    services: services.map((s) => ({
      name: s.service,
      web: s.kind === 'web',
      language: s.language,
      version: s.version,
      requests: s.requests,
      reqPerSec: s.reqPerSec,
      errorRate: s.errorRate,
      p95Ms: s.p95Ms,
      lastDeploy: s.lastDeploy,
    })),
    logServices: [...instrumented, ...LOG_ONLY],
    metricServices: METRIC_ONLY,
    peers,
    endpoints: listResources(scope)
      .slice(0, 1000)
      .map((r) => ({ service: r.service, resource: r.resource, requests: r.requests, reqPerSec: r.reqPerSec, errorRate: r.errorRate, p95Ms: r.p95Ms })),
    errorTypes,
    definitions: db().map((d) => ({ id: d.id, definition: d.definition })),
    metadata: new Map(meta),
    slos: slos.map((s) => ({
      name: s.name,
      texts: [s.goodQuery, s.totalQuery, s.sliceQuery, ...s.tags].filter((t): t is string => !!t),
      monitors: monitors.filter((m) => s.monitorIds.includes(m.id)).map((m) => m.name),
    })),
    monitors: monitors.map((m) => ({ name: m.name, texts: [m.query, ...m.tags], alerting: m.status === 'alert' })),
    incidents: mockIncidentFacts()
      .filter((i) => i.services.length && (i.open || i.declaredAt >= req.range.from))
      .map((i) => ({ services: i.services, open: i.open, inRange: i.declaredAt >= req.range.from && i.declaredAt <= req.range.to })),
    repos: ciRepositories().map((r) => ({ name: r.repository, url: r.url, runs: r.runs, failures: r.failures, lastRun: r.lastRun })),
    references: referencesService,
  });
  const perTeam = entitiesPerTeam(entities);
  return {
    entities,
    teams: mockTeamRoster().map((t) => ({ handle: t.handle, name: t.name, members: t.members.length, entities: perTeam.get(t.handle) ?? 0 })),
    myTeams: mockMyTeams(),
    envs: listEnvs(req.range),
  };
}

export async function listEntityDefinitions(): Promise<PortalDefinitionItem[]> {
  return db().map((d) => structuredClone(d));
}

export async function getEntityDefinition(id: string): Promise<PortalDefinitionItem> {
  return structuredClone(db().find((d) => d.id === id) ?? fail('No such entity definition'));
}

async function syncMetadata(def: EntityDefinition): Promise<void> {
  if (def.kind !== 'service') return;
  const next = metadataOf(def);
  const current = mockServiceMetadata().get(def.metadata.name);
  await setMockServiceMetadata(def.metadata.name, {
    team: next.team || current?.team || '',
    contacts: next.contacts.length ? next.contacts : (current?.contacts ?? []),
    repoUrl: next.repoUrl || current?.repoUrl || '',
    docsUrl: next.docsUrl || current?.docsUrl || '',
  });
}

async function upsert(def: EntityDefinition, source: string): Promise<{ item: PortalDefinitionItem; created: boolean }> {
  const all = db();
  const existing = all.find((d) => d.kind === def.kind && d.name === def.metadata.name);
  const item: Stored = { id: existing?.id ?? uuid(), kind: def.kind, name: def.metadata.name, source, definition: def, updatedAt: Date.now() };
  if (existing) Object.assign(existing, item);
  else all.push(item);
  await syncMetadata(def);
  return { item: structuredClone(item), created: !existing };
}

export async function upsertEntityDefinition(input: EntityDefinition): Promise<PortalDefinitionItem> {
  return (await upsert(validateDefinition(input), 'ui')).item;
}

export async function updateEntityDefinition(id: string, input: EntityDefinition): Promise<PortalDefinitionItem> {
  const all = db();
  const current = all.find((d) => d.id === id) ?? fail('No such entity definition');
  const def = validateDefinition(input);
  if (all.some((d) => d.id !== id && d.kind === def.kind && d.name === def.metadata.name)) fail(`${def.kind}:${def.metadata.name} is already defined`);
  Object.assign(current, { kind: def.kind, name: def.metadata.name, definition: def, source: 'ui', updatedAt: Date.now() });
  await syncMetadata(def);
  return structuredClone(current);
}

export async function deleteEntityDefinition(id: string): Promise<void> {
  const all = db();
  const i = all.findIndex((d) => d.id === id);
  if (i < 0) fail('No such entity definition');
  all.splice(i, 1);
}

export async function importEntityDefinitions(content: string): Promise<PortalImportResult> {
  const parsed = parseJsonDocuments(content);
  const out: PortalImportResult = { created: 0, updated: 0, entities: [] };
  for (const def of parsed) {
    const { item, created } = await upsert(def, 'import');
    if (created) out.created++;
    else out.updated++;
    out.entities.push(item);
  }
  return out;
}

// ================================================================ reports

export async function getTeamsReport(req: PortalReportRequest): Promise<PortalTeamsReport> {
  const roster = mockTeamRoster();
  const teams = roster.filter((t) => !req.teams?.length || req.teams.includes(t.handle)).map((t) => ({ handle: t.handle, name: t.name, members: t.members.length }));
  const owners = new Map([...mockServiceMetadata()].filter(([, m]) => m.team).map(([s, m]) => [s, m.team]));
  const teamsOf = (own: string[], tags: string[]) =>
    own.length
      ? own.map((t) => t.replace(/^@/, ''))
      : servicesNamed(tags)
          .map((s) => owners.get(s) ?? '')
          .filter(Boolean);
  const [monitors, slos, schedules] = await Promise.all([allMonitors(), allSlos(), listSchedules()]);
  const report = teamsReport(req.range, teams, mockIncidentFacts(), {
    owners,
    sloTeams: slos.map((s) => teamsOf(s.teams, s.tags)),
    monitorTeams: monitors.map((m) => teamsOf(m.teams, m.tags)),
    scheduleTeams: schedules.map((s) => s.team ?? '').filter(Boolean),
  });
  return { ...report, totalTeams: roster.length };
}

/** A fictional history: each service climbed to today's level over the past weeks. */
export async function getScorecardsReport(req: PortalReportRequest): Promise<PortalScorecardsReport> {
  const overview = await getScorecards();
  if (!overview.setUp) return { days: [], teams: [] };
  const today = Math.floor(Date.now() / DAY) * DAY;
  const rows: [number, string, string, number][] = [];
  for (const s of overview.services) {
    const hash = [...s.service].reduce((h, c) => (h * 31 + c.charCodeAt(0)) >>> 0, 7);
    const since = 10 + (hash % 50);
    for (let d = Math.floor(req.range.from / DAY) * DAY; d <= Math.min(today, req.range.to); d += DAY) {
      const age = (today - d) / DAY;
      const level = Math.max(0, s.level - (age > since ? 1 : 0) - (age > since * 2 ? 1 : 0));
      rows.push([d, s.service, s.team, level]);
    }
  }
  return scorecardTrend(req.range, rows, req.teams ?? []);
}

export async function listReportDashboards(): Promise<PortalReportDashboard[]> {
  const out: PortalReportDashboard[] = [];
  for (const id of reportDashboards) {
    try {
      const d = await getDashboard(id);
      out.push({ id, title: d.title, description: d.description ?? null });
    } catch {
      /* deleted since */
    }
  }
  return out;
}

export async function setReportDashboards(ids: string[]): Promise<PortalReportDashboard[]> {
  const unique = [...new Set(ids.map((i) => i.trim()).filter(Boolean))];
  if (unique.length > 50) fail('Add at most 50 dashboards');
  for (const id of unique) await getDashboard(id).catch(() => fail("Some dashboards don't exist"));
  reportDashboards = unique;
  return listReportDashboards();
}
